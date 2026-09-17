//! Step-2 harness: an app's `update` is exercised natively, with no wasm anywhere, and the
//! bytes it produces are asserted directly.
//!
//! This is the loop app authors get: write UI, assert the command stream, no browser.

use ccosel_abi::{Cmd, Decoder, RespRecord, ResponseFlags};
use ccosel_sdk::{App, FrameCtx, Recorder, Text, Ui, Vec2};

struct Demo {
    clicks: u32,
    filter: Text,
}

impl App for Demo {
    fn update(&mut self, ui: &mut Ui<'_>) {
        ui.label("Files");
        ui.separator();
        ui.horizontal(|ui| {
            if ui.button("Up").clicked() {
                self.clicks += 1;
            }
            ui.tooltip("Go to parent directory");
            ui.text_edit(&mut self.filter);
        });
        ui.image("/cas/abc123", Vec2::new(64.0, 64.0));
    }
}

fn record(app: &mut Demo, rec: &mut Recorder) -> Vec<u8> {
    let mut ui = Ui::root(rec, FrameCtx::default());
    app.update(&mut ui);
    rec.commands().to_vec()
}

fn decode(buf: &[u8]) -> Vec<Cmd<'_>> {
    Decoder::new(buf).map(|c| c.unwrap()).collect()
}

fn demo() -> Demo {
    Demo {
        clicks: 0,
        filter: Text::new(""),
    }
}

#[test]
fn records_a_well_formed_frame() {
    let mut app = demo();
    let mut rec = Recorder::new();
    let buf = record(&mut app, &mut rec);

    ccosel_abi::validate(&buf).unwrap();
    let cmds = decode(&buf);

    assert!(matches!(cmds[0], Cmd::Label { text: "Files", .. }));
    assert!(matches!(cmds[1], Cmd::Separator));
    assert!(matches!(cmds[2], Cmd::BeginScope { .. }));
    assert!(matches!(cmds[3], Cmd::Button { text: "Up", .. }));
    assert!(matches!(cmds[4], Cmd::Tooltip { .. }));
    assert!(matches!(cmds[5], Cmd::TextEditSingle { .. }));
    assert!(matches!(cmds[6], Cmd::EndScope { .. }));
    assert!(matches!(cmds[7], Cmd::Image { src: "/cas/abc123", .. }));
}

#[test]
fn tooltip_attaches_to_the_preceding_widget() {
    let mut app = demo();
    let mut rec = Recorder::new();
    let buf = record(&mut app, &mut rec);
    let cmds = decode(&buf);

    let button_id = match cmds[3] {
        Cmd::Button { id, .. } => id,
        _ => panic!("expected a button"),
    };
    let tip_id = match cmds[4] {
        Cmd::Tooltip { id, .. } => id,
        _ => panic!("expected a tooltip"),
    };
    assert_eq!(button_id, tip_id);
}

fn widget_ids(buf: &[u8]) -> Vec<u64> {
    decode(buf)
        .into_iter()
        .filter_map(|c| match c {
            Cmd::Label { id, .. }
            | Cmd::Button { id, .. }
            | Cmd::BeginScope { id, .. }
            | Cmd::TextEditSingle { id, .. }
            | Cmd::Image { id, .. } => Some(id),
            _ => None,
        })
        .collect()
}

#[test]
fn ids_are_stable_across_frames() {
    // The whole response mechanism depends on this: an id that drifts frame to frame means
    // clicks are looked up against a table that never matches, and the app silently stops
    // responding.
    //
    // Note the buffers themselves are *not* identical — frame 1 pushes the initial text
    // contents and frame 2 omits them. Identity is what has to hold, not the bytes.
    let mut app = demo();
    let mut rec = Recorder::new();
    let first = record(&mut app, &mut rec);
    let second = record(&mut app, &mut rec);
    assert_eq!(widget_ids(&first), widget_ids(&second));
    assert!(!widget_ids(&first).is_empty());
}

#[test]
fn a_click_is_observed_one_frame_later() {
    let mut app = demo();
    let mut rec = Recorder::new();

    // Frame 1: nothing has happened yet.
    let buf = record(&mut app, &mut rec);
    assert_eq!(app.clicks, 0);

    let button_id = decode(&buf)
        .into_iter()
        .find_map(|c| match c {
            Cmd::Button { id, .. } => Some(id),
            _ => None,
        })
        .unwrap();

    // The host reports the click that happened during frame 1...
    rec.set_responses(vec![RespRecord {
        local_id: button_id,
        flags: ResponseFlags::CLICKED | ResponseFlags::HOVERED | ResponseFlags::ENABLED,
        ..Default::default()
    }]);

    // ...and the app sees it on frame 2.
    record(&mut app, &mut rec);
    assert_eq!(app.clicks, 1);

    // Responses are not sticky: without a fresh record the click does not repeat.
    rec.set_responses(vec![]);
    record(&mut app, &mut rec);
    assert_eq!(app.clicks, 1);
}

#[test]
fn unknown_widgets_return_a_blank_response() {
    let mut app = demo();
    let mut rec = Recorder::new();
    rec.set_responses(vec![RespRecord {
        local_id: 0xdead_beef,
        flags: ResponseFlags::CLICKED,
        ..Default::default()
    }]);
    record(&mut app, &mut rec);
    // A response for an id we never emitted must not leak onto some other widget.
    assert_eq!(app.clicks, 0);
}

#[test]
fn text_buffer_is_sent_once_then_omitted() {
    // The cost model that makes a code editor viable: after the initial push, a text field
    // costs a fixed 13 bytes per frame no matter how long the document is.
    let mut app = demo();
    app.filter = Text::new("initial contents");
    let mut rec = Recorder::new();

    let first = record(&mut app, &mut rec);
    assert!(matches!(
        decode(&first)[5],
        Cmd::TextEditSingle {
            set: Some("initial contents"),
            ..
        }
    ));

    let second = record(&mut app, &mut rec);
    assert!(matches!(
        decode(&second)[5],
        Cmd::TextEditSingle { set: None, .. }
    ));
    assert!(second.len() < first.len());

    // An explicit set pushes again, with a higher version so the shell accepts it.
    app.filter.set("replaced");
    let third = record(&mut app, &mut rec);
    match decode(&third)[5] {
        Cmd::TextEditSingle { set, version, .. } => {
            assert_eq!(set, Some("replaced"));
            assert!(version > 1);
        }
        _ => panic!("expected a text edit"),
    }
}

#[test]
fn deeply_nested_scopes_still_produce_a_valid_buffer() {
    // A guest that nests past the host's limit must not be able to emit a buffer the host
    // rejects wholesale — it would take the entire app off screen.
    struct Deep;
    impl App for Deep {
        fn update(&mut self, ui: &mut Ui<'_>) {
            fn recurse(ui: &mut Ui<'_>, n: u32) {
                if n == 0 {
                    ui.label("bottom");
                    return;
                }
                ui.horizontal(|ui| recurse(ui, n - 1));
            }
            recurse(ui, ccosel_abi::MAX_SCOPE_DEPTH + 20);
        }
    }

    let mut rec = Recorder::new();
    let mut ui = Ui::root(&mut rec, FrameCtx::default());
    Deep.update(&mut ui);
    ccosel_abi::validate(rec.commands()).unwrap();
}
