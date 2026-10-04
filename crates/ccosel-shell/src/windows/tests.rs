//! The window list and the window itself, driven with a fake app: no wasm module, no browser.
//! Run under node by `cargo xtask test-wasm`, since this crate only compiles for wasm32.

use std::cell::RefCell;
use std::rc::Rc;

use ccosel_abi::event::{decode_batch, event_kind};
use ccosel_abi::{Cmd, Encoder, RespRecord};
use ccosel_host::{AppInstance, FrameArgs, FrameResult, HostError, OutboundCall};
use ccosel_proto::Method;
use ccosel_transport::{EventSink, Incoming, Outgoing, Transport, Wire};
use wasm_bindgen_test::wasm_bindgen_test;

use super::*;
use crate::registry::find;

// --- A fake app, and a fake wire ----------------------------------------------------------

/// What a [`Fake`] app was given and will give back, shared with the test that made it.
#[derive(Default)]
struct Seen {
    /// The command stream every frame returns.
    commands: RefCell<Vec<u8>>,
    /// The responses handed to the latest frame: the previous frame's, as on a real guest.
    responses: RefCell<Vec<RespRecord>>,
    frames: RefCell<u64>,
    /// Event batches, in arrival order, and how many frames had run when each arrived.
    events: RefCell<Vec<(u64, Vec<u8>)>>,
    outbox: RefCell<Vec<OutboundCall>>,
    cancels: RefCell<Vec<u32>>,
}

struct Fake(Rc<Seen>);

impl AppInstance for Fake {
    fn frame(&mut self, args: &FrameArgs<'_>) -> Result<FrameResult, HostError> {
        *self.0.responses.borrow_mut() = args.responses.to_vec();
        *self.0.frames.borrow_mut() += 1;
        Ok(FrameResult {
            commands: self.0.commands.borrow().clone(),
            wants_repaint_after_ms: ccosel_abi::REPAINT_ON_INPUT_ONLY,
            status: 0,
        })
    }
    fn on_event(&mut self, bytes: &[u8]) -> Result<(), HostError> {
        let frames = *self.0.frames.borrow();
        self.0.events.borrow_mut().push((frames, bytes.to_vec()));
        Ok(())
    }
    fn save_state(&mut self) -> Result<Vec<u8>, HostError> {
        Ok(Vec::new())
    }
    fn take_outbox(&mut self) -> Vec<OutboundCall> {
        std::mem::take(&mut self.0.outbox.borrow_mut())
    }
    fn take_cancels(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.0.cancels.borrow_mut())
    }
}

fn encode(cmds: &[Cmd<'_>]) -> Vec<u8> {
    let mut e = Encoder::new();
    for c in cmds {
        e.push(c);
    }
    e.as_slice().to_vec()
}

/// A window of catalog app `app`, with instance id `id`, drawing `cmds` every frame.
fn window(app: &str, id: u64, cmds: &[Cmd<'_>]) -> (AppWindow<Fake>, Rc<Seen>) {
    let seen = Rc::new(Seen::default());
    *seen.commands.borrow_mut() = encode(cmds);
    let entry = find(app).expect("a catalog app");
    let w = AppWindow::new(Fake(seen.clone()), id, &entry, "", None);
    (w, seen)
}

fn input(events: Vec<egui::Event>) -> egui::RawInput {
    egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(800.0, 600.0),
        )),
        events,
        ..Default::default()
    }
}

/// One shell frame with `window` filling the page.
fn frame(ctx: &egui::Context, window: &mut AppWindow<Fake>, events: Vec<egui::Event>) {
    let mut out = ctx.run_ui(input(events), |ui| window.ui(ui));
    // Headless: nothing uploads these, and epaint panics if they are dropped unhandled.
    out.textures_delta.clear();
}

fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    }
}

/// Click widget `id`, found where the app was last told it is, then draw the frame that sees
/// the release. The click is reported by the window straight after that frame.
fn click(ctx: &egui::Context, window: &mut AppWindow<Fake>, seen: &Seen, id: u64) {
    // Two frames, so the app has been told where the widget is.
    frame(ctx, window, vec![]);
    frame(ctx, window, vec![]);
    let r = *seen
        .responses
        .borrow()
        .iter()
        .find(|r| r.local_id == id)
        .expect("the widget was drawn");
    let at = egui::pos2((r.rect[0] + r.rect[2]) / 2.0, (r.rect[1] + r.rect[3]) / 2.0);
    frame(
        ctx,
        window,
        vec![egui::Event::PointerMoved(at), button(at, true)],
    );
    frame(ctx, window, vec![button(at, false)]);
}

fn aux_of(seen: &Seen, id: u64) -> u32 {
    seen.responses
        .borrow()
        .iter()
        .find(|r| r.local_id == id)
        .map_or(0, |r| r.aux)
}

#[derive(Default)]
struct FakeWire {
    sent: Rc<RefCell<Vec<Vec<Outgoing>>>>,
}

impl Wire for FakeWire {
    fn send(&self, batch: Vec<Outgoing>) {
        self.sent.borrow_mut().push(batch);
    }
}

fn transport() -> (Transport, Rc<RefCell<Vec<Vec<Outgoing>>>>) {
    let wire = FakeWire::default();
    let sent = wire.sent.clone();
    (Transport::new(Box::new(wire)), sent)
}

fn call(call_id: u32) -> OutboundCall {
    OutboundCall {
        call_id,
        method: Method::ListDir as u32,
        args: vec![call_id as u8],
    }
}

/// Every RPC reply event the app received, as `(kind, call_id)`.
fn replies(seen: &Seen) -> Vec<(u32, u32)> {
    seen.events
        .borrow()
        .iter()
        .flat_map(|(_, batch)| decode_batch(batch).unwrap())
        .filter(|e| matches!(e.kind, event_kind::RPC_OK | event_kind::RPC_ERR))
        .map(|e| (e.kind, e.call_id))
        .collect()
}

const UPLOAD: Cmd<'static> = Cmd::UploadFolder {
    id: 7,
    dest: "/Docs",
};

// --- One window ---------------------------------------------------------------------------

#[wasm_bindgen_test]
fn the_app_draws_into_its_window_and_is_told_where_its_widgets_landed() {
    let ctx = egui::Context::default();
    let (mut w, seen) = window(
        "clock",
        1,
        &[
            Cmd::Label { id: 1, text: "hi" },
            Cmd::Button { id: 2, text: "Go" },
        ],
    );
    frame(&ctx, &mut w, vec![]);
    assert_eq!(*seen.frames.borrow(), 1);
    assert!(seen.responses.borrow().is_empty(), "nothing drawn yet");

    frame(&ctx, &mut w, vec![]);
    let ids: Vec<u64> = seen.responses.borrow().iter().map(|r| r.local_id).collect();
    assert_eq!(ids, [1, 2], "last frame's widgets, in order");
    assert!(w.command_bytes() > 0);
}

#[wasm_bindgen_test]
fn what_a_window_was_opened_on_reaches_the_app_before_its_first_frame() {
    let ctx = egui::Context::default();
    let seen = Rc::new(Seen::default());
    let entry = find("viewer").unwrap();
    let mut w = AppWindow::new(Fake(seen.clone()), 1, &entry, "", Some("/Docs/a.png"));
    assert_eq!(w.launch_arg.as_deref(), Some("/Docs/a.png"));

    frame(&ctx, &mut w, vec![]);
    let events = seen.events.borrow();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].0, 0, "delivered before any frame ran");
    let launch = decode_batch(&events[0].1).unwrap();
    assert_eq!(launch[0].kind, event_kind::LAUNCH);
    assert_eq!(launch[0].payload, b"/Docs/a.png");
}

#[wasm_bindgen_test]
fn an_upload_click_is_reported_in_the_frame_it_happens_and_only_then() {
    let ctx = egui::Context::default();
    let (mut w, seen) = window("file-browser", 1, &[UPLOAD]);
    assert_eq!(w.clicked_upload(), None);

    click(&ctx, &mut w, &seen, 7);
    assert_eq!(w.clicked_upload(), Some((7, "/Docs".to_owned())));
    assert_eq!(
        w.clicked_project_upload(),
        None,
        "not mixed up with a project"
    );
    assert_eq!(w.clicked_open_url(), None);

    frame(&ctx, &mut w, vec![]);
    assert_eq!(w.clicked_upload(), None, "one click opens one picker");
    assert_eq!(w.drop_target(), Some(DropTarget::Folder(7, "/Docs".into())));
}

#[wasm_bindgen_test]
fn a_finished_upload_reaches_the_app_in_its_buttons_response() {
    let ctx = egui::Context::default();
    let (mut w, seen) = window("file-browser", 1, &[UPLOAD, Cmd::UploadProject { id: 8 }]);
    frame(&ctx, &mut w, vec![]);
    frame(&ctx, &mut w, vec![]);
    assert_eq!((aux_of(&seen, 7), aux_of(&seen, 8)), (0, 0));

    w.upload_finished(7);
    w.upload_finished(7);
    w.project_uploaded(8, 42);
    frame(&ctx, &mut w, vec![]);
    frame(&ctx, &mut w, vec![]);
    assert_eq!(aux_of(&seen, 7), 2, "a count of finished uploads");
    assert_eq!(aux_of(&seen, 8), 42, "the project's temporary folder");
}

#[wasm_bindgen_test]
fn closing_a_window_tells_the_transport_it_is_gone() {
    let (mut w, _) = window("clock", 1, &[]);
    let sink = w.sink();
    assert!(w.open && sink.alive());
    w.close();
    assert!(!w.open);
    assert!(!sink.alive(), "a sink handed out earlier sees it too");
}

// --- Launching ----------------------------------------------------------------------------

fn ready(app: &str, id: u64, restored: Option<usize>) -> Launch<Fake> {
    Launch::Ready {
        window: Box::new(window(app, id, &[]).0),
        restored,
    }
}

fn failed(restored: Option<usize>) -> Launch<Fake> {
    Launch::Failed {
        name: "Clock".into(),
        error: "the module didn't compile".into(),
        restored,
    }
}

#[wasm_bindgen_test]
fn two_launches_of_one_app_are_two_windows_with_titles_to_tell_them_apart() {
    let (mut windows, mut errors, mut restoring) = (Vec::new(), Vec::new(), Restoring::default());
    for (app, id) in [
        ("clock", 1),
        ("file-browser", 2),
        ("clock", 3),
        ("clock", 4),
    ] {
        assert_eq!(
            accept(
                &mut windows,
                &mut errors,
                &mut restoring,
                ready(app, id, None)
            ),
            None
        );
    }
    let titles: Vec<(&str, u64)> = windows
        .iter()
        .map(|w| (w.title.as_str(), w.instance_id))
        .collect();
    assert_eq!(
        titles,
        [("Clock", 1), ("Files", 2), ("Clock 2", 3), ("Clock 3", 4)]
    );
    assert_ne!(window_id(1), window_id(3), "and separate egui state");
    assert!(errors.is_empty());
}

#[wasm_bindgen_test]
fn an_app_that_fails_to_load_is_an_error_not_an_empty_window() {
    let (mut windows, mut errors, mut restoring) = (Vec::new(), Vec::new(), Restoring::default());
    accept(&mut windows, &mut errors, &mut restoring, failed(None));
    assert!(windows.is_empty());
    assert_eq!(errors, ["Clock: the module didn't compile"]);
}

#[wasm_bindgen_test]
fn remembered_windows_are_stacked_as_they_were_once_the_last_one_opens() {
    let (mut windows, mut errors, mut restoring) = (Vec::new(), Vec::new(), Restoring::default());
    for _ in 0..3 {
        restoring.expect();
    }
    // They finish loading in any order; place 0 is the back.
    assert_eq!(
        accept(
            &mut windows,
            &mut errors,
            &mut restoring,
            ready("clock", 10, Some(2))
        ),
        None
    );
    // A window opened by hand meanwhile doesn't count toward the remembered ones.
    assert_eq!(
        accept(
            &mut windows,
            &mut errors,
            &mut restoring,
            ready("docs", 99, None)
        ),
        None
    );
    assert_eq!(
        accept(
            &mut windows,
            &mut errors,
            &mut restoring,
            ready("docs", 11, Some(0))
        ),
        None
    );
    assert!(restoring.in_progress());
    // One that fails still counts, or the rest would never be stacked.
    assert_eq!(
        accept(&mut windows, &mut errors, &mut restoring, failed(Some(1))),
        Some(vec![11, 10])
    );
    assert!(!restoring.in_progress());
    assert_eq!(windows.len(), 3);
    assert_eq!(errors.len(), 1);
}

// --- Drops and uploads --------------------------------------------------------------------

#[wasm_bindgen_test]
fn a_drop_goes_to_the_window_under_it_and_nowhere_else() {
    let ctx = egui::Context::default();
    let (mut takes, seen) = window("file-browser", 1, &[UPLOAD]);
    frame(&ctx, &mut takes, vec![]);
    drop(seen);
    let (refuses, _) = window("clock", 2, &[]);
    let windows = vec![takes, refuses];

    assert_eq!(
        drop_destination(&windows, false, Some(window_id(1))),
        Ok((1, DropTarget::Folder(7, "/Docs".into())))
    );
    assert_eq!(
        drop_destination(&windows, false, Some(window_id(2))),
        Err("upload: Clock doesn't take uploads".into())
    );
    let outside = Err("upload: drop files onto an app window to upload them".into());
    assert_eq!(drop_destination(&windows, false, None), outside);
    assert_eq!(
        drop_destination(&windows, false, Some(egui::Id::new("the dock"))),
        outside
    );
    // On an app's own page the app is the whole page.
    assert_eq!(drop_destination(&windows, true, None).map(|d| d.0), Ok(1));
}

#[wasm_bindgen_test]
fn a_minimised_window_takes_no_drops() {
    let ctx = egui::Context::default();
    let (mut w, _) = window("file-browser", 1, &[UPLOAD]);
    frame(&ctx, &mut w, vec![]);
    w.placement.minimized = true;
    let windows = vec![w];
    assert!(drop_destination(&windows, false, Some(window_id(1))).is_err());
    assert!(drop_destination(&windows, true, None).is_err());
}

#[wasm_bindgen_test]
fn a_finished_upload_is_told_to_its_own_window_only() {
    let ctx = egui::Context::default();
    let (a, seen_a) = window("file-browser", 1, &[UPLOAD]);
    let (b, seen_b) = window("file-browser", 2, &[UPLOAD]);
    let mut windows = vec![a, b];

    let done = |instance, project, scratch| Upload {
        instance,
        widget: 7,
        project,
        scratch,
        finished: true,
        ..Default::default()
    };
    upload_finished(&mut windows, &done(2, false, None));
    // A project upload that failed before it had a folder tells its app nothing.
    upload_finished(&mut windows, &done(1, true, None));
    // And a window closed since is simply not there to tell.
    upload_finished(&mut windows, &done(99, false, None));

    for w in &mut windows {
        frame(&ctx, w, vec![]);
        frame(&ctx, w, vec![]);
    }
    assert_eq!(aux_of(&seen_a, 7), 0);
    assert_eq!(aux_of(&seen_b, 7), 1);

    upload_finished(&mut windows, &done(1, true, Some(5)));
    frame(&ctx, &mut windows[0], vec![]);
    frame(&ctx, &mut windows[0], vec![]);
    assert_eq!(aux_of(&seen_a, 7), 5);
}

// --- Calls --------------------------------------------------------------------------------

#[wasm_bindgen_test]
fn an_apps_calls_go_out_and_their_replies_come_back_to_it() {
    let ctx = egui::Context::default();
    let (mut transport, sent) = transport();
    let (mut w, seen) = window("file-browser", 1, &[]);
    seen.outbox.borrow_mut().extend([call(1), call(2)]);

    forward_calls(&mut w, &mut transport, 0.0);
    assert!(seen.outbox.borrow().is_empty(), "drained");
    transport.flush();
    let batch = sent.borrow()[0].clone();
    assert_eq!(batch.len(), 2, "both calls in one request");

    transport.on_replies(
        batch
            .iter()
            .map(|o| Incoming {
                seq: o.seq,
                result: Ok(vec![]),
            })
            .collect(),
    );
    frame(&ctx, &mut w, vec![]);
    let mut got = replies(&seen);
    got.sort_unstable();
    assert_eq!(got, [(event_kind::RPC_OK, 1), (event_kind::RPC_OK, 2)]);
}

#[wasm_bindgen_test]
fn a_call_the_app_abandons_is_never_answered() {
    let ctx = egui::Context::default();
    let (mut transport, sent) = transport();
    let (mut w, seen) = window("file-browser", 1, &[]);
    seen.outbox.borrow_mut().push(call(1));
    forward_calls(&mut w, &mut transport, 0.0);
    transport.flush();

    seen.cancels.borrow_mut().push(1);
    forward_calls(&mut w, &mut transport, 0.0);
    assert_eq!(transport.in_flight(), 0);

    let seq = sent.borrow()[0][0].seq;
    transport.on_replies(vec![Incoming {
        seq,
        result: Ok(vec![]),
    }]);
    frame(&ctx, &mut w, vec![]);
    assert!(replies(&seen).is_empty());
}

#[wasm_bindgen_test]
fn closing_a_window_drops_it_and_a_late_reply_to_it_goes_nowhere() {
    let (mut transport, sent) = transport();
    let (mut a, seen_a) = window("file-browser", 1, &[]);
    let (b, _) = window("clock", 2, &[]);
    seen_a.outbox.borrow_mut().push(call(1));
    forward_calls(&mut a, &mut transport, 0.0);
    transport.flush();
    let sink = a.sink();

    a.close();
    let mut windows = vec![a, b];
    reap_closed(&mut windows, &mut transport);
    assert_eq!(
        windows.iter().map(|w| w.instance_id).collect::<Vec<_>>(),
        [2]
    );
    assert_eq!(transport.in_flight(), 0, "its calls are forgotten");

    // The reply arrives after all.
    let seq = sent.borrow()[0][0].seq;
    transport.on_replies(vec![Incoming {
        seq,
        result: Ok(vec![]),
    }]);
    transport.tick(1.0);
    assert!(!sink.alive());
    assert!(
        replies(&seen_a).is_empty(),
        "nothing reaches the closed app"
    );
}
