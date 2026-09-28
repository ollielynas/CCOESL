//! Replay tests against an offscreen `egui::Context`.
//!
//! No wasm, no window, no browser — the point of keeping `ccosel-host` free of any wasm
//! runtime is that the hardest code in the project is testable like ordinary Rust.

use ccosel_abi::event::{decode_batch, decode_text_delta, event_kind};
use ccosel_abi::{Align, Cmd, Encoder, Layout, RespRecord, ResponseFlags, ScopeKind, TextStyle};
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
        Cmd::Label {
            id: 10,
            text: "Files",
        },
        Cmd::Separator,
        Cmd::BeginScope {
            id: 20,
            layout: ccosel_abi::Layout::new(
                ccosel_abi::ScopeKind::Horizontal,
                ccosel_abi::Align::Center,
            ),
        },
        Cmd::Button { id: 21, text: "Up" },
        Cmd::Button {
            id: 22,
            text: "Home",
        },
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
        Cmd::Button {
            id: 22,
            text: "Home",
        },
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
        vec![0xEE],                         // unknown opcode
        encode(&[Cmd::EndScope { id: 1 }]), // close with no open
        encode(&[Cmd::BeginScope {
            id: 1,
            layout: Default::default(),
        }]), // never closed
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
fn upload_folder_and_open_url_are_drawn_and_tracked() {
    let buf = encode(&[
        Cmd::UploadFolder {
            id: 30,
            dest: "/Documents",
        },
        Cmd::OpenUrl {
            id: 31,
            label: "Download",
            url: "/files/notes.md",
        },
    ]);

    let ctx = egui::Context::default();
    let mut r = Replayer::new();
    let recs = frame(&ctx, &mut r, &buf, raw_input()).unwrap();

    let ids: Vec<u64> = recs.iter().map(|r| r.local_id).collect();
    assert_eq!(ids, vec![30, 31], "both drawn as widgets and reported");

    assert_eq!(r.uploads(), &[(30, "/Documents".to_string())]);
    assert_eq!(r.open_url_ids(), &[(31, "/files/notes.md".to_string())]);
}

#[test]
fn a_click_on_open_url_lands_on_the_right_widget() {
    let buf = encode(&[
        Cmd::UploadFolder {
            id: 30,
            dest: "/Documents",
        },
        Cmd::OpenUrl {
            id: 31,
            label: "Download",
            url: "/files/notes.md",
        },
    ]);
    let ctx = egui::Context::default();
    let mut r = Replayer::new();

    let recs = frame(&ctx, &mut r, &buf, raw_input()).unwrap();
    let open_url_rect = find(&recs, 31).rect;
    let target = egui::pos2(
        (open_url_rect[0] + open_url_rect[2]) / 2.0,
        (open_url_rect[1] + open_url_rect[3]) / 2.0,
    );

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

    assert!(
        find(&recs, 31).clicked(),
        "the OpenUrl button should register the click"
    );
    assert!(
        !find(&recs, 30).clicked(),
        "the UploadFolder button must not"
    );
}

#[test]
fn upload_and_open_url_ids_do_not_survive_a_rejected_frame() {
    // Same contract as decoded state generally: an error leaves the previous good frame's
    // tracked ids standing rather than clearing them out from under the shell.
    let good = encode(&[Cmd::UploadFolder {
        id: 30,
        dest: "/Documents",
    }]);
    let ctx = egui::Context::default();
    let mut r = Replayer::new();
    frame(&ctx, &mut r, &good, raw_input()).unwrap();
    assert_eq!(r.uploads(), &[(30, "/Documents".to_string())]);

    let bad = vec![0xEE];
    assert!(frame(&ctx, &mut r, &bad, raw_input()).is_err());
    assert_eq!(
        r.uploads(),
        &[(30, "/Documents".to_string())],
        "unchanged by the rejected frame"
    );
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

#[test]
fn a_plot_is_drawn_at_its_size_and_reported() {
    let buf = encode(&[
        Cmd::Plot {
            id: 40,
            size: ccosel_abi::Vec2::new(120.0, 30.0),
            samples: &[0, 255, 10, 200],
        },
        // Zero width means "fill the row".
        Cmd::Plot {
            id: 41,
            size: ccosel_abi::Vec2::new(0.0, 20.0),
            samples: &[],
        },
    ]);
    let ctx = egui::Context::default();
    let mut r = Replayer::new();
    let recs = frame(&ctx, &mut r, &buf, raw_input()).unwrap();

    let fixed = find(&recs, 40).rect;
    assert_eq!(fixed[2] - fixed[0], 120.0);
    assert_eq!(fixed[3] - fixed[1], 30.0);
    let fill = find(&recs, 41).rect;
    assert!(fill[2] - fill[0] > 120.0, "a zero-width plot fills the row");
}

/// Type `text` into whatever widget has keyboard focus.
fn typing(text: &str) -> egui::RawInput {
    egui::RawInput {
        events: vec![egui::Event::Text(text.to_owned())],
        ..raw_input()
    }
}

/// Click at `pos`: press and release in one frame.
fn click_at(pos: egui::Pos2) -> egui::RawInput {
    let button = |pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    egui::RawInput {
        events: vec![egui::Event::PointerMoved(pos), button(true), button(false)],
        ..raw_input()
    }
}

fn centre(rec: &RespRecord) -> egui::Pos2 {
    egui::pos2(
        (rec.rect[0] + rec.rect[2]) / 2.0,
        (rec.rect[1] + rec.rect[3]) / 2.0,
    )
}

#[test]
fn typed_text_comes_back_to_the_guest_as_a_minimal_delta() {
    let ctx = egui::Context::default();
    let mut r = Replayer::new();
    let field = |set: Option<&'static str>, version| {
        encode(&[Cmd::TextEditMulti {
            id: 40,
            version,
            set,
        }])
    };

    // The guest loads a document. Nothing needs sending back: the guest wrote it.
    let recs = frame(&ctx, &mut r, &field(Some("hello world"), 1), raw_input()).unwrap();
    assert!(r.take_text_events().is_none());

    // Focus the field, then type. The cursor lands at the end of the text.
    let at = centre(&find(&recs, 40));
    frame(&ctx, &mut r, &field(None, 1), click_at(at)).unwrap();
    frame(&ctx, &mut r, &field(None, 1), typing("!")).unwrap();
    assert_eq!(r.text(40).unwrap().0, "hello world!");

    let batch = r.take_text_events().expect("the edit is reported");
    let events = decode_batch(&batch).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, event_kind::TEXT_DELTA);
    let d = decode_text_delta(events[0].payload).unwrap();
    assert_eq!((d.id, d.version), (40, 2));
    // Only the new character crosses, not the document.
    assert_eq!((d.start, d.end, d.inserted), (11, 11, "!"));

    // Reported once.
    assert!(r.take_text_events().is_none());
}

#[test]
fn a_new_empty_field_adopts_the_guests_version() {
    // `Text::new("")` starts at version 1 and sends nothing. If the shell kept its own 0, its
    // first edit would carry version 1 and the guest would drop it as stale.
    let ctx = egui::Context::default();
    let mut r = Replayer::new();
    let buf = encode(&[Cmd::TextEditSingle {
        id: 41,
        version: 1,
        set: None,
    }]);
    let recs = frame(&ctx, &mut r, &buf, raw_input()).unwrap();
    assert_eq!(find(&recs, 41).aux, 1);

    frame(&ctx, &mut r, &buf, click_at(centre(&find(&recs, 41)))).unwrap();
    frame(&ctx, &mut r, &buf, typing("ab")).unwrap();
    let batch = r.take_text_events().unwrap();
    let d = decode_text_delta(decode_batch(&batch).unwrap()[0].payload).unwrap();
    assert_eq!(d.version, 2);
    assert_eq!((d.start, d.end, d.inserted), (0, 0, "ab"));
}

#[test]
fn a_guest_set_is_applied_once_even_if_the_frame_is_replayed() {
    let ctx = egui::Context::default();
    let mut r = Replayer::new();
    let set = encode(&[Cmd::TextEditSingle {
        id: 42,
        version: 5,
        set: Some("loaded"),
    }]);
    let recs = frame(&ctx, &mut r, &set, raw_input()).unwrap();
    frame(&ctx, &mut r, &set, click_at(centre(&find(&recs, 42)))).unwrap();
    // The same buffer again, as the shell does when a guest's next frame fails, with the user
    // typing meanwhile: the old `set` must not wipe the keystroke.
    frame(&ctx, &mut r, &set, typing("!")).unwrap();
    assert_eq!(r.text(42).unwrap().0, "loaded!");

    // A genuinely new set, even at a lower version than the shell reached, wins.
    let again = encode(&[Cmd::TextEditSingle {
        id: 42,
        version: 6,
        set: Some("reloaded"),
    }]);
    frame(&ctx, &mut r, &again, raw_input()).unwrap();
    assert_eq!(r.text(42).unwrap(), ("reloaded", 6));
}

#[test]
fn styled_text_and_wrapped_rows_render_and_links_are_clickable() {
    let ctx = egui::Context::default();
    let mut r = Replayer::new();
    let buf = encode(&[
        Cmd::Styled {
            id: 50,
            text: "Title",
            style: TextStyle::heading(1),
        },
        Cmd::BeginScope {
            id: 51,
            layout: Layout::new(ScopeKind::Wrapped, Align::Min),
        },
        Cmd::Styled {
            id: 52,
            text: "plain, ",
            style: TextStyle::PLAIN,
        },
        Cmd::Styled {
            id: 53,
            text: "bold code",
            style: TextStyle::STRONG
                | TextStyle::ITALIC
                | TextStyle::CODE
                | TextStyle::STRIKE
                | TextStyle::WEAK,
        },
        Cmd::Styled {
            id: 54,
            text: "a link",
            style: TextStyle::LINK,
        },
        Cmd::EndScope { id: 51 },
    ]);
    let recs = frame(&ctx, &mut r, &buf, raw_input()).unwrap();
    let title = find(&recs, 50);
    let body = find(&recs, 52);
    assert!(
        title.rect[3] - title.rect[1] > body.rect[3] - body.rect[1],
        "a heading is taller than body text"
    );
    // Wrapped runs sit on one line, edge to edge.
    let (a, b) = (find(&recs, 52), find(&recs, 53));
    assert_eq!(a.rect[1], b.rect[1]);
    assert!((b.rect[0] - a.rect[2]).abs() < 0.5);

    let link = find(&recs, 54);
    let recs = frame(&ctx, &mut r, &buf, click_at(centre(&link))).unwrap();
    assert!(find(&recs, 54).clicked());
}
