//! One running app, and the per-frame dance around it.

use std::cell::Cell;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;

use ccosel_abi::{REPAINT_ON_INPUT_ONLY, RespRecord};
use ccosel_host::{AppInstance, FrameArgs, Replayer};
use ccosel_transport::EventSink;

use crate::chrome::Placement;

/// The delivery target for one window's RPC replies.
///
/// Lives behind an `Rc` so the transport's pending table can hold a clone without owning the
/// window. `alive` is what lets the table reap entries for a window that has been closed —
/// otherwise abandoned calls would pile up against an app that no longer exists.
pub struct WindowSink {
    queue: Rc<RefCell<VecDeque<Vec<u8>>>>,
    alive: Rc<Cell<bool>>,
}

impl EventSink for WindowSink {
    fn deliver(&self, batch: Vec<u8>) {
        self.queue.borrow_mut().push_back(batch);
    }
    fn alive(&self) -> bool {
        self.alive.get()
    }
}

use std::cell::RefCell;

pub struct AppWindow<I: AppInstance> {
    pub title: String,
    pub icon: &'static str,
    pub color: egui::Color32,
    pub app_id: &'static str,
    /// Distinct per *instance*, not per app: two Files windows must not share egui state, or
    /// they would fight over scroll position and focus.
    pub instance_id: u64,
    pub open: bool,
    pub default_size: [f32; 2],
    /// Minimised or maximised, from the title bar.
    pub placement: Placement,
    /// What it was opened on, if anything, so a remembered desktop can open it on that again.
    pub launch_arg: Option<String>,
    instance: I,
    replayer: Replayer,
    /// Responses from the previous frame, handed back to the guest on the next one.
    responses: Vec<RespRecord>,
    frame_index: u64,
    /// Retained so a frame the guest failed to produce leaves the last good UI on screen
    /// instead of a blank window.
    last_commands: Vec<u8>,
    error: Option<String>,
    /// Events waiting to be handed to the guest. Owned by the window rather than the instance,
    /// so a suspended app's replies survive until it is resumed.
    events: Rc<RefCell<VecDeque<Vec<u8>>>>,
    alive: Rc<Cell<bool>>,
    /// Per `UploadFolder` and `UploadFiles` button, how many uploads from it have finished. Handed to the app in
    /// that button's response (`aux`), which is how it knows to re-list the folder.
    uploads_finished: HashMap<u64, u32>,
}

/// What the shell tells an app before its first frame, as one batch: facts about the page
/// (its `origin`, which only the browser knows), then what it was opened on, if anything. Page
/// facts come first so an app's `open` can already use them. Nothing at all if there is
/// nothing to say.
pub fn first_events(origin: &str, launch_arg: Option<&str>) -> VecDeque<Vec<u8>> {
    use ccosel_abi::event::{encode_batch, encode_page_info, event_kind};

    let page = encode_page_info(&[("origin", origin)]);
    let mut events: Vec<(u32, u32, &[u8])> = Vec::new();
    if !origin.is_empty() {
        events.push((event_kind::PAGE_INFO, 0, &page));
    }
    if let Some(arg) = launch_arg {
        events.push((event_kind::LAUNCH, 0, arg.as_bytes()));
    }
    let mut queue = VecDeque::new();
    if !events.is_empty() {
        queue.push_back(encode_batch(&events));
    }
    queue
}

impl<I: AppInstance> AppWindow<I> {
    /// A window of `app` in a page at `origin`, opened on `launch_arg` if there is one.
    pub fn new(
        instance: I,
        instance_id: u64,
        app: &crate::registry::AppEntry,
        origin: &str,
        launch_arg: Option<&str>,
    ) -> Self {
        // Reaches the app before its first frame, through the same queue as everything else
        // the shell tells it.
        let events = first_events(origin, launch_arg);
        Self {
            title: app.name.to_owned(),
            icon: app.icon,
            color: app.color,
            app_id: app.id,
            instance_id,
            open: true,
            default_size: app.default_size,
            placement: Placement::default(),
            launch_arg: launch_arg.map(str::to_owned),
            instance,
            replayer: Replayer::new(),
            responses: Vec::new(),
            frame_index: 0,
            last_commands: Vec::new(),
            error: None,
            events: Rc::new(RefCell::new(events)),
            alive: Rc::new(Cell::new(true)),
            uploads_finished: HashMap::new(),
        }
    }

    /// The `UploadFolder` or `UploadFiles` button clicked this frame, if any: its id, its
    /// destination folder, and whether it picks files rather than a folder. Called straight
    /// after [`Self::ui`], so the picker opens in the same frame as the click.
    pub fn clicked_upload(&self) -> Option<(u64, String, bool)> {
        self.replayer
            .uploads()
            .iter()
            .find(|(id, _)| self.clicked(*id))
            .map(|(id, dest)| (*id, dest.clone(), self.replayer.picks_files(*id)))
    }

    /// The `UploadProject` button clicked this frame, if any.
    pub fn clicked_project_upload(&self) -> Option<u64> {
        self.replayer
            .project_uploads()
            .iter()
            .copied()
            .find(|&id| self.clicked(id))
    }

    /// Where something dropped on this window goes: wherever its upload button would send it.
    pub fn drop_target(&self) -> Option<crate::upload::DropTarget> {
        crate::upload::drop_target(self.replayer.uploads(), self.replayer.project_uploads())
    }

    /// The target of the `OpenUrl` button clicked this frame, if any.
    pub fn clicked_open_url(&self) -> Option<String> {
        self.replayer
            .open_url_ids()
            .iter()
            .find(|(id, _)| self.clicked(*id))
            .map(|(_, url)| url.clone())
    }

    /// The app and argument of the `OpenApp` button clicked this frame, if any.
    pub fn clicked_open_app(&self) -> Option<(String, String)> {
        self.replayer
            .open_apps()
            .iter()
            .find(|(id, ..)| self.clicked(*id))
            .map(|(_, app, arg)| (app.clone(), arg.clone()))
    }

    /// The path of the `CopyLink` button clicked this frame, if any.
    pub fn clicked_copy_link(&self) -> Option<String> {
        self.replayer
            .copy_links()
            .iter()
            .find(|(id, _)| self.clicked(*id))
            .map(|(_, path)| path.clone())
    }

    /// Where this frame drew the app's audio, video and documents.
    pub fn media(&self) -> &[ccosel_host::MediaSlot] {
        self.replayer.media()
    }

    fn clicked(&self, local_id: u64) -> bool {
        self.responses
            .iter()
            .any(|r| r.local_id == local_id && r.clicked())
    }

    /// An upload started from this window's `widget` button has finished.
    pub fn upload_finished(&mut self, widget: u64) {
        *self.uploads_finished.entry(widget).or_default() += 1;
    }

    /// A project upload from this window's `widget` button has finished, into temporary
    /// folder `scratch`. Reported to the app in that button's response.
    pub fn project_uploaded(&mut self, widget: u64, scratch: u32) {
        self.uploads_finished.insert(widget, scratch);
    }

    /// A delivery handle for the transport's pending table.
    pub fn sink(&self) -> Rc<WindowSink> {
        Rc::new(WindowSink {
            queue: self.events.clone(),
            alive: self.alive.clone(),
        })
    }

    /// Mark the window gone so the transport stops trying to deliver to it.
    pub fn close(&mut self) {
        self.alive.set(false);
        self.open = false;
    }

    /// Bytes the guest emitted last frame — the number that would cross the network if this
    /// app were ever hosted remotely, and a useful health signal either way.
    pub fn command_bytes(&self) -> usize {
        self.last_commands.len()
    }

    /// Calls the guest issued during its last frame.
    pub fn take_outbox(&mut self) -> Vec<ccosel_host::OutboundCall> {
        self.instance.take_outbox()
    }

    /// Calls the guest abandoned during its last frame.
    pub fn take_cancels(&mut self) -> Vec<u32> {
        self.instance.take_cancels()
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        let screen = ui.ctx().viewport_rect();
        let args = FrameArgs {
            frame_index: self.frame_index,
            time_ms: ui.ctx().input(|i| i.time) * 1000.0,
            dt_ms: ui.ctx().input(|i| i.stable_dt) * 1000.0,
            pixels_per_point: ui.ctx().pixels_per_point(),
            screen_size: [screen.width(), screen.height()],
            flags: if ui.visuals().dark_mode {
                ccosel_abi::frame::input_flags::DARK_MODE
            } else {
                0
            },
            responses: &self.responses,
            events: &[],
        };
        self.frame_index += 1;

        // Events are handed over *before* the frame, in arrival order, so a reply is always
        // visible to the frame that renders because of it.
        {
            let mut queue = self.events.borrow_mut();
            while let Some(batch) = queue.pop_front() {
                if let Err(e) = self.instance.on_event(&batch) {
                    self.error = Some(e.to_string());
                }
            }
        }

        match self.instance.frame(&args) {
            Ok(result) => {
                self.error = None;
                self.last_commands = result.commands;
                if result.wants_repaint_after_ms != REPAINT_ON_INPUT_ONLY {
                    ui.ctx()
                        .request_repaint_after(std::time::Duration::from_millis(
                            result.wants_repaint_after_ms as u64,
                        ));
                }
            }
            Err(e) => self.error = Some(e.to_string()),
        }

        if let Some(err) = &self.error {
            ui.colored_label(egui::Color32::RED, err.as_str());
        }

        if !self.last_commands.is_empty() {
            match self
                .replayer
                .replay(ui, self.instance_id, &self.last_commands)
            {
                Ok(mut responses) => {
                    for r in &mut responses {
                        if let Some(&n) = self.uploads_finished.get(&r.local_id) {
                            r.aux = n;
                        }
                    }
                    self.responses = responses;
                    // What the user typed goes back to the guest before its next frame. That
                    // frame has to happen even if nothing else moves, or the app would only
                    // see a keystroke when the *next* one arrived.
                    if let Some(batch) = self.replayer.take_text_events() {
                        self.events.borrow_mut().push_back(batch);
                        ui.ctx().request_repaint();
                    }
                }
                Err(e) => {
                    // A malformed frame is dropped whole; the previous one stays up.
                    self.error = Some(e.to_string());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use ccosel_abi::event::{decode_batch, event_kind, page_info};
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::first_events;

    #[wasm_bindgen_test]
    fn an_app_is_told_its_page_origin_then_what_it_was_opened_on() {
        let queue = first_events("https://ccosel.example.com", Some("/Docs/a.md"));
        assert_eq!(
            queue.len(),
            1,
            "one batch, so both arrive before the first frame"
        );
        let events = decode_batch(&queue[0]).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].kind, event_kind::PAGE_INFO);
        assert_eq!(events[0].call_id, 0);
        assert_eq!(
            page_info(events[0].payload, "origin"),
            Some("https://ccosel.example.com")
        );
        assert_eq!(events[1].kind, event_kind::LAUNCH);
        assert_eq!(events[1].payload, b"/Docs/a.md");
    }

    #[wasm_bindgen_test]
    fn without_a_launch_argument_only_the_page_is_described() {
        let queue = first_events("http://192.168.1.20:8777", None);
        let events = decode_batch(&queue[0]).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, event_kind::PAGE_INFO);
    }

    #[wasm_bindgen_test]
    fn with_nothing_to_say_nothing_is_sent() {
        assert!(first_events("", None).is_empty());
        let queue = first_events("", Some("/x"));
        let events = decode_batch(&queue[0]).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, event_kind::LAUNCH);
    }
}
