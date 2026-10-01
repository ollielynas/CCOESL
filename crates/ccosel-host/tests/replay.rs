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

#[test]
fn upload_project_is_drawn_and_tracked() {
    let buf = encode(&[Cmd::UploadProject { id: 50 }, Cmd::UploadProject { id: 51 }]);
    let ctx = egui::Context::default();
    let mut r = Replayer::new();
    let recs = frame(&ctx, &mut r, &buf, raw_input()).unwrap();
    assert_eq!(
        recs.iter().map(|r| r.local_id).collect::<Vec<_>>(),
        vec![50, 51]
    );
    assert_eq!(r.project_uploads(), &[50, 51]);
    assert!(r.uploads().is_empty(), "not mixed up with UploadFolder");
}

// --- Hard shadows behind buttons and text fields -------------------------------------------

/// The colour the shadow is drawn in: the widget outline's, so the host picks up the shell's
/// ink without naming a colour itself. Unusual enough that nothing else in a frame is it.
const INK: egui::Color32 = egui::Color32::from_rgb(0x11, 0x22, 0x33);

fn inked_context() -> egui::Context {
    let ctx = egui::Context::default();
    ctx.all_styles_mut(|s| {
        s.visuals.widgets.inactive.bg_stroke = egui::Stroke::new(2.0, INK);
    });
    ctx
}

/// Like `frame`, but returns what was painted instead of the response table.
fn frame_shapes(
    ctx: &egui::Context,
    replayer: &mut Replayer,
    buf: &[u8],
    input: egui::RawInput,
) -> (Vec<RespRecord>, Vec<egui::Shape>) {
    let mut recs = Vec::new();
    let mut full = ctx.run_ui(input, |ui| {
        recs = replayer.replay(ui, APP, buf).unwrap();
    });
    full.textures_delta.clear();
    (recs, full.shapes.into_iter().map(|c| c.shape).collect())
}

fn rect_of(rec: &RespRecord) -> egui::Rect {
    egui::Rect::from_min_max(
        egui::pos2(rec.rect[0], rec.rect[1]),
        egui::pos2(rec.rect[2], rec.rect[3]),
    )
}

/// Index of the solid ink block sitting 3px down and right of `widget`, if one was painted.
fn shadow_index(shapes: &[egui::Shape], widget: egui::Rect) -> Option<usize> {
    let want = widget.translate(egui::vec2(3.0, 3.0));
    shapes
        .iter()
        .position(|s| matches!(s, egui::Shape::Rect(r) if r.fill == INK && r.rect == want))
}

/// Index of the widget's own frame: a rect over exactly its area with a visible outline.
fn frame_index(shapes: &[egui::Shape], widget: egui::Rect) -> Option<usize> {
    shapes
        .iter()
        .position(|s| matches!(s, egui::Shape::Rect(r) if r.rect == widget && r.stroke.width > 0.0))
}

#[test]
fn every_kind_of_button_and_the_text_field_get_a_shadow_behind_them() {
    let buf = encode(&[
        Cmd::Button { id: 21, text: "Up" },
        Cmd::UploadFolder {
            id: 22,
            dest: "/Documents",
        },
        Cmd::UploadProject { id: 23 },
        Cmd::OpenUrl {
            id: 24,
            label: "Download",
            url: "/files/notes.md",
        },
        Cmd::TextEditSingle {
            id: 25,
            version: 0,
            set: None,
        },
    ]);
    let ctx = inked_context();
    let mut r = Replayer::new();
    let (recs, shapes) = frame_shapes(&ctx, &mut r, &buf, raw_input());

    for id in 21..=25 {
        let rect = rect_of(&find(&recs, id));
        let shadow = shadow_index(&shapes, rect).unwrap_or_else(|| panic!("{id}: no shadow"));
        let frame = frame_index(&shapes, rect).unwrap_or_else(|| panic!("{id}: no frame"));
        assert!(
            shadow < frame,
            "{id}: the shadow must be painted behind the widget"
        );
    }
}

#[test]
fn a_button_is_framed_at_rest() {
    // No hover, no press: the outline is there anyway (`frame_when_inactive` is not turned off).
    let buf = encode(&[Cmd::Button { id: 21, text: "Up" }]);
    let ctx = inked_context();
    let mut r = Replayer::new();
    let (recs, shapes) = frame_shapes(&ctx, &mut r, &buf, raw_input());
    let rect = rect_of(&find(&recs, 21));
    assert!(frame_index(&shapes, rect).is_some());
}

#[test]
fn a_pressed_button_loses_its_shadow() {
    let buf = encode(&[Cmd::Button { id: 21, text: "Up" }]);
    let ctx = inked_context();
    let mut r = Replayer::new();
    let (recs, _) = frame_shapes(&ctx, &mut r, &buf, raw_input());
    let rect = rect_of(&find(&recs, 21));

    // Held down, not yet released.
    let mut input = raw_input();
    input.events = vec![
        egui::Event::PointerMoved(rect.center()),
        egui::Event::PointerButton {
            pos: rect.center(),
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: Default::default(),
        },
    ];
    let (recs, shapes) = frame_shapes(&ctx, &mut r, &buf, input);
    assert!(find(&recs, 21).flags & ResponseFlags::IS_POINTER_BUTTON_DOWN_ON != 0);
    assert_eq!(shadow_index(&shapes, rect), None, "pressed: no shadow");

    // Released: the shadow comes back.
    let mut input = raw_input();
    input.events = vec![egui::Event::PointerButton {
        pos: rect.center(),
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Default::default(),
    }];
    frame_shapes(&ctx, &mut r, &buf, input);
    let (_, shapes) = frame_shapes(&ctx, &mut r, &buf, raw_input());
    assert!(
        shadow_index(&shapes, rect).is_some(),
        "released: shadow back"
    );
}

#[test]
fn a_disabled_button_has_no_shadow() {
    let buf = encode(&[Cmd::Button { id: 21, text: "Up" }]);
    let ctx = inked_context();
    let mut r = Replayer::new();
    let mut recs = Vec::new();
    let mut full = ctx.run_ui(raw_input(), |ui| {
        ui.disable();
        recs = r.replay(ui, APP, &buf).unwrap();
    });
    full.textures_delta.clear();
    let shapes: Vec<_> = full.shapes.into_iter().map(|c| c.shape).collect();
    let rect = rect_of(&find(&recs, 21));
    assert_eq!(shadow_index(&shapes, rect), None);
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

#[test]
fn a_sidebar_is_a_fixed_column_beside_top_aligned_content() {
    let ctx = egui::Context::default();
    let mut r = Replayer::new();
    let buf = encode(&[
        Cmd::BeginScope {
            id: 60,
            layout: Layout::new(ScopeKind::Horizontal, Align::Min),
        },
        Cmd::BeginScope {
            id: 61,
            layout: Layout::new(ScopeKind::Sidebar, Align::Min),
        },
        Cmd::Button { id: 62, text: "a" },
        Cmd::EndScope { id: 61 },
        Cmd::BeginScope {
            id: 63,
            layout: Layout::new(ScopeKind::Vertical, Align::Min),
        },
        Cmd::Label {
            id: 64,
            text: "one",
        },
        Cmd::Label {
            id: 65,
            text: "two",
        },
        Cmd::Label {
            id: 66,
            text: "three",
        },
        Cmd::EndScope { id: 63 },
        Cmd::EndScope { id: 60 },
    ]);
    let recs = frame(&ctx, &mut r, &buf, raw_input()).unwrap();
    let (side, main) = (find(&recs, 62), find(&recs, 64));
    // 30% of 800, clamped to at most 240: the main column starts past it, not beside "a".
    assert!(
        main.rect[0] >= side.rect[0] + 240.0,
        "{:?} {:?}",
        side.rect,
        main.rect
    );
    assert_eq!(side.rect[1], main.rect[1], "both start at the top");
}

#[test]
fn selectable_rows_click_and_indent_steps_in() {
    let ctx = egui::Context::default();
    let mut r = Replayer::new();
    let buf = encode(&[
        Cmd::Selectable {
            id: 70,
            text: "Folder",
            selected: false,
        },
        Cmd::BeginScope {
            id: 71,
            layout: Layout::new(ScopeKind::Indent, Align::Min),
        },
        Cmd::Selectable {
            id: 72,
            text: "doc",
            selected: true,
        },
        Cmd::EndScope { id: 71 },
    ]);
    let recs = frame(&ctx, &mut r, &buf, raw_input()).unwrap();
    let (outer, inner) = (find(&recs, 70), find(&recs, 72));
    assert!(inner.rect[0] > outer.rect[0] + 4.0, "the child is indented");
    let recs = frame(&ctx, &mut r, &buf, click_at(centre(&inner))).unwrap();
    assert!(find(&recs, 72).clicked());
}

#[test]
fn a_scroll_region_is_bounded_by_the_window_and_scrolls_on_its_own() {
    let ctx = egui::Context::default();
    let mut r = Replayer::new();
    let mut cmds = vec![
        Cmd::BeginScope {
            id: 80,
            layout: Layout::new(ScopeKind::Horizontal, Align::Min),
        },
        Cmd::BeginScope {
            id: 81,
            layout: Layout::new(ScopeKind::Scroll, Align::Min),
        },
    ];
    let labels: Vec<String> = (0..200).map(|i| format!("line {i}")).collect();
    for (i, text) in labels.iter().enumerate() {
        cmds.push(Cmd::Label {
            id: 1000 + i as u64,
            text,
        });
    }
    cmds.push(Cmd::EndScope { id: 81 });
    cmds.push(Cmd::EndScope { id: 80 });
    cmds.push(Cmd::Label {
        id: 90,
        text: "after",
    });
    let buf = encode(&cmds);

    // Inside a window-sized scroll area, as the shell draws every app.
    let mut out = Vec::new();
    let mut full = ctx.run_ui(raw_input(), |ui| {
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| out = r.replay(ui, APP, &buf).unwrap());
    });
    full.textures_delta.clear();

    // Two hundred lines would run far past a 600px window; the region stops at its bottom.
    let after = find(&out, 90);
    assert!(after.rect[1] <= 610.0, "bounded: {:?}", after.rect);
    let last = find(&out, 1199);
    assert!(
        last.rect[1] > 600.0,
        "its content is taller, so it scrolls: {:?} {:?}",
        last.rect,
        find(&out, 1000).rect
    );
}

#[test]
fn a_selectable_at_rest_has_an_invisible_outline_and_a_visible_one_when_hovered() {
    // Framed in every state so hovering can't change its size, but only visibly on hover.
    let buf = encode(&[Cmd::Selectable {
        id: 21,
        text: "notes.md",
        selected: false,
    }]);
    let ctx = inked_context();
    ctx.all_styles_mut(|s| {
        s.visuals.widgets.hovered.bg_stroke = egui::Stroke::new(2.0, INK);
    });
    let mut r = Replayer::new();
    let outline = |shapes: &[egui::Shape], rect: egui::Rect| {
        shapes.iter().find_map(|s| match s {
            egui::Shape::Rect(r) if r.rect == rect => Some(r.stroke),
            _ => None,
        })
    };

    let (recs, shapes) = frame_shapes(&ctx, &mut r, &buf, raw_input());
    let rect = rect_of(&find(&recs, 21));
    let rest = outline(&shapes, rect).expect("framed at rest");
    assert_eq!(rest.color, egui::Color32::TRANSPARENT);

    let mut hover = raw_input();
    hover.events = vec![egui::Event::PointerMoved(rect.center())];
    frame_shapes(&ctx, &mut r, &buf, hover.clone());
    let (recs, shapes) = frame_shapes(&ctx, &mut r, &buf, hover);
    assert_eq!(rect_of(&find(&recs, 21)), rect, "same size when hovered");
    assert_eq!(outline(&shapes, rect).expect("framed").color, INK);
}

/// A `ScrollFooter` holding `lines` labels, ids from 1000, over a prompt field, id 95.
fn console(lines: &[String]) -> Vec<u8> {
    let mut cmds = vec![
        Cmd::BeginScope {
            id: 90,
            layout: Layout::new(ScopeKind::ScrollFooter, Align::Min),
        },
        Cmd::BeginScope {
            id: 91,
            layout: Layout::new(ScopeKind::Vertical, Align::Min),
        },
    ];
    for (i, text) in lines.iter().enumerate() {
        cmds.push(Cmd::Label {
            id: 1000 + i as u64,
            text,
        });
    }
    cmds.extend([
        Cmd::EndScope { id: 91 },
        Cmd::BeginScope {
            id: 92,
            layout: Layout::new(ScopeKind::Horizontal, Align::Center),
        },
        Cmd::TextEditSingle {
            id: 95,
            version: 1,
            set: None,
        },
        Cmd::EndScope { id: 92 },
        Cmd::EndScope { id: 90 },
    ]);
    encode(&cmds)
}

#[test]
fn a_scroll_footer_pins_its_row_to_the_bottom_of_the_window() {
    let ctx = egui::Context::default();
    let mut r = Replayer::new();
    let mut run = |lines: &[String]| {
        let buf = console(lines);
        let mut out = Vec::new();
        // Inside a window-sized scroll area, as the shell draws every app.
        let mut full = ctx.run_ui(raw_input(), |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| out = r.replay(ui, APP, &buf).unwrap());
        });
        full.textures_delta.clear();
        out
    };

    // Two lines: the prompt still sits at the bottom of the window, not under them.
    let few: Vec<String> = (0..2).map(|i| format!("line {i}")).collect();
    let out = run(&few);
    let prompt = find(&out, 95);
    assert!(prompt.rect[3] > 550.0, "at the bottom: {:?}", prompt.rect);
    assert!(prompt.rect[3] <= 600.0, "on screen: {:?}", prompt.rect);

    // Two hundred lines run far past the window, and the prompt stays on screen below them.
    let many: Vec<String> = (0..200).map(|i| format!("line {i}")).collect();
    let out = run(&many);
    let prompt = find(&out, 95);
    assert!(
        prompt.rect[3] <= 600.0,
        "still on screen: {:?}",
        prompt.rect
    );
    assert!(
        find(&out, 1000).rect[1] < prompt.rect[1],
        "the lines are above it"
    );
}

#[test]
fn enter_in_a_footer_field_submits_it_and_keeps_the_focus() {
    let ctx = egui::Context::default();
    let mut r = Replayer::new();
    let buf = console(&[]);
    let recs = frame(&ctx, &mut r, &buf, raw_input()).unwrap();
    let at = centre(&find(&recs, 95));
    frame(&ctx, &mut r, &buf, click_at(at)).unwrap();
    let recs = frame(&ctx, &mut r, &buf, typing("x = 1")).unwrap();
    assert_eq!(find(&recs, 95).flags & ResponseFlags::SUBMITTED, 0);

    let enter = egui::RawInput {
        events: vec![egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
        ..raw_input()
    };
    let recs = frame(&ctx, &mut r, &buf, enter).unwrap();
    assert_ne!(find(&recs, 95).flags & ResponseFlags::SUBMITTED, 0);
    // Still the place to type the next command.
    let recs = frame(&ctx, &mut r, &buf, typing("y")).unwrap();
    assert_ne!(find(&recs, 95).flags & ResponseFlags::HAS_FOCUS, 0);
    assert_eq!(r.text(95).unwrap().0, "x = 1y");
}

#[test]
fn a_long_document_in_a_scroll_region_scrolls_there_not_the_window() {
    let ctx = egui::Context::default();
    let mut r = Replayer::new();
    let text: String = (0..200).map(|i| format!("line {i}\n")).collect();
    let buf = encode(&[
        Cmd::BeginScope {
            id: 85,
            layout: Layout::new(ScopeKind::Scroll, Align::Min),
        },
        Cmd::TextEditMulti {
            id: 86,
            version: 1,
            set: Some(&text),
        },
        Cmd::EndScope { id: 85 },
        Cmd::Label {
            id: 87,
            text: "after",
        },
    ]);
    // Inside a window-sized scroll area, as the shell draws every app.
    let mut out = Vec::new();
    let mut full = ctx.run_ui(raw_input(), |ui| {
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| out = r.replay(ui, APP, &buf).unwrap());
    });
    full.textures_delta.clear();
    let editor = find(&out, 86);
    assert!(
        editor.rect[3] > 600.0,
        "the document is taller: {:?}",
        editor.rect
    );
    assert!(
        find(&out, 87).rect[1] <= 610.0,
        "but the region stops at the window"
    );
}
