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
