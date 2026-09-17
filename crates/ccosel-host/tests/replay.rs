//! Replay tests against an offscreen `egui::Context`.
//!
//! No wasm, no window, no browser — the point of keeping `ccosel-host` free of any wasm
//! runtime is that the hardest code in the project is testable like ordinary Rust.

use ccosel_abi::{Cmd, Encoder, RespRecord, ResponseFlags};
use ccosel_host::{ReplayError, Replayer};

const APP: u64 = 1;

fn encode(cmds: &[Cmd<'_>]) -> Vec<u8> {
    let mut e = Encoder::new();
    for c in cmds {
        e.push(c);
    }
    e.as_slice().to_vec()
}

/// Drive one frame through a real egui context and return the response table.
fn frame(
    ctx: &egui::Context,
    replayer: &mut Replayer,
    buf: &[u8],
    input: egui::RawInput,
) -> Result<Vec<RespRecord>, ReplayError> {
    let mut out = Ok(Vec::new());
    // `run_ui` hands back the root `Ui` directly, which is how the shell will call replay:
    // into a `Ui` the shell already owns, not into a panel the app controls.
    let mut full = ctx.run_ui(input, |ui| {
        out = replayer.replay(ui, APP, buf);
    });
    // Headless: nothing is going to upload these, and epaint panics if they are dropped
    // unhandled.
    full.textures_delta.clear();
    out
}

fn raw_input() -> egui::RawInput {
    egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(800.0, 600.0),
        )),
        ..Default::default()
    }
}

fn find(recs: &[RespRecord], id: u64) -> RespRecord {
    *recs.iter().find(|r| r.local_id == id).expect("no record")
}

#[test]
fn renders_a_nested_tree_and_reports_every_widget() {
    let buf = encode(&[
        Cmd::Label { id: 10, text: "Files" },
        Cmd::Separator,
        Cmd::BeginScope {
            id: 20,
            layout: ccosel_abi::Layout::new(ccosel_abi::ScopeKind::Horizontal, ccosel_abi::Align::Center),
        },
        Cmd::Button { id: 21, text: "Up" },
        Cmd::Button { id: 22, text: "Home" },
        Cmd::EndScope { id: 20 },
    ]);

    let ctx = egui::Context::default();
    let mut r = Replayer::new();
    let recs = frame(&ctx, &mut r, &buf, raw_input()).unwrap();

    let ids: Vec<u64> = recs.iter().map(|r| r.local_id).collect();
    assert_eq!(ids, vec![10, 21, 22], "sorted by local_id, one per widget");

    // The two buttons are laid out side by side, so they share a row but not a column.
    let up = find(&recs, 21);
    let home = find(&recs, 22);
    assert!(up.rect[2] <= home.rect[0], "Up should be left of Home");
    assert_eq!(up.rect[1], home.rect[1], "same row");
}

#[test]
fn a_click_lands_on_the_right_widget() {
    let buf = encode(&[
        Cmd::Button { id: 21, text: "Up" },
        Cmd::Button { id: 22, text: "Home" },
    ]);

    let ctx = egui::Context::default();
    let mut r = Replayer::new();

    // Frame 1 establishes layout. Nothing is clicked.
    let recs = frame(&ctx, &mut r, &buf, raw_input()).unwrap();
    assert!(!find(&recs, 22).clicked());
    let home_rect = find(&recs, 22).rect;
    let target = egui::pos2(
        (home_rect[0] + home_rect[2]) / 2.0,
        (home_rect[1] + home_rect[3]) / 2.0,
    );

    // Frame 2 clicks the middle of the second button.
    let mut input = raw_input();
    input.events = vec![
        egui::Event::PointerMoved(target),
        egui::Event::PointerButton {
            pos: target,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: Default::default(),
        },
        egui::Event::PointerButton {
            pos: target,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Default::default(),
        },
    ];
    let recs = frame(&ctx, &mut r, &buf, input).unwrap();

    assert!(find(&recs, 22).clicked(), "Home should register the click");
    assert!(!find(&recs, 21).clicked(), "Up must not");
    assert!(find(&recs, 22).flags & ResponseFlags::HOVERED != 0);
}

#[test]
fn shell_retains_the_text_buffer_when_the_guest_omits_it() {
    // The cost model in one test: the guest sends its text once, then stops, and the field
    // keeps its contents anyway because the shell owns the buffer.
    let ctx = egui::Context::default();
    let mut r = Replayer::new();

    let first = encode(&[Cmd::TextEditSingle {
        id: 30,
        version: 1,
        set: Some("hello"),
    }]);
    frame(&ctx, &mut r, &first, raw_input()).unwrap();
    assert_eq!(r.text(30).unwrap().0, "hello");

    let steady = encode(&[Cmd::TextEditSingle {
        id: 30,
        version: 1,
        set: None,
    }]);
    frame(&ctx, &mut r, &steady, raw_input()).unwrap();
    assert_eq!(r.text(30).unwrap().0, "hello");

    // A stale echo at the same version must not clobber it.
    let stale = encode(&[Cmd::TextEditSingle {
        id: 30,
        version: 1,
        set: Some("stale"),
    }]);
    frame(&ctx, &mut r, &stale, raw_input()).unwrap();
    assert_eq!(r.text(30).unwrap().0, "hello");

    // A higher version wins.
    let push = encode(&[Cmd::TextEditSingle {
        id: 30,
        version: 2,
        set: Some("replaced"),
    }]);
    frame(&ctx, &mut r, &push, raw_input()).unwrap();
    assert_eq!(r.text(30).unwrap().0, "replaced");
}

#[test]
fn malformed_buffers_are_rejected_before_anything_is_drawn() {
    let ctx = egui::Context::default();
    let mut r = Replayer::new();

    for bad in [
        vec![0xEE],                                    // unknown opcode
        encode(&[Cmd::EndScope { id: 1 }]),            // close with no open
        encode(&[Cmd::BeginScope { id: 1, layout: Default::default() }]), // never closed
    ] {
        let out = frame(&ctx, &mut r, &bad, raw_input());
        assert!(out.is_err(), "expected rejection for {bad:?}");
    }
}

#[test]
fn arbitrary_bytes_never_panic_the_host() {
    // Guest output is untrusted. A hostile module must not be able to take down the desktop.
    let ctx = egui::Context::default();
    let mut r = Replayer::new();
    let mut state = 0xF00Du64;
    for _ in 0..400 {
        let mut buf = Vec::new();
        for _ in 0..(state % 48) {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            buf.push((state >> 33) as u8);
        }
        let _ = frame(&ctx, &mut r, &buf, raw_input());
        state = state.wrapping_add(1);
    }
}

#[test]
fn tooltips_do_not_need_a_hover_branch() {
    // The tooltip is emitted unconditionally alongside its widget and resolved by id, so the
    // guest never has to see `hovered()` to produce one.
    let buf = encode(&[
        Cmd::Button { id: 21, text: "Up" },
        Cmd::Tooltip {
            id: 21,
            text: "Go to parent directory",
        },
    ]);
    let ctx = egui::Context::default();
    let mut r = Replayer::new();
    let recs = frame(&ctx, &mut r, &buf, raw_input()).unwrap();
    assert_eq!(recs.len(), 1, "a tooltip is not itself a widget");
    assert_eq!(recs[0].local_id, 21);
}
