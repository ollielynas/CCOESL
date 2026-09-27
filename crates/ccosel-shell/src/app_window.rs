//! One running app, and the per-frame dance around it.

use std::cell::Cell;
use std::collections::VecDeque;
use std::rc::Rc;

use ccosel_abi::{REPAINT_ON_INPUT_ONLY, RespRecord};
use ccosel_host::{AppInstance, FrameArgs, Replayer};
use ccosel_transport::EventSink;

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
}

impl<I: AppInstance> AppWindow<I> {
    pub fn new(
        instance: I,
        instance_id: u64,
        app_id: &'static str,
        title: String,
        icon: &'static str,
        color: egui::Color32,
        default_size: [f32; 2],
    ) -> Self {
        Self {
            title,
            icon,
            color,
            app_id,
            instance_id,
            open: true,
            default_size,
            instance,
            replayer: Replayer::new(),
            responses: Vec::new(),
            frame_index: 0,
            last_commands: Vec::new(),
            error: None,
            events: Rc::new(RefCell::new(VecDeque::new())),
            alive: Rc::new(Cell::new(true)),
        }
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
                Ok(responses) => self.responses = responses,
                Err(e) => {
                    // A malformed frame is dropped whole; the previous one stays up.
                    self.error = Some(e.to_string());
                }
            }
        }
    }
}
