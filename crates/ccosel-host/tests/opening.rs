//! Replay of what the Viewer and the Files right-click menu need: context menus, `OpenApp`,
//! `CopyLink`, `Media`, `TextView` and images, against an offscreen `egui::Context`.

use ccosel_abi::{Align, Cmd, Encoder, Layout, MediaKind, RespRecord, ScopeKind, Vec2};
use ccosel_host::{MediaSlot, Replayer};

const APP: u64 = 1;

fn encode(cmds: &[Cmd<'_>]) -> Vec<u8> {
    let mut e = Encoder::new();
    for c in cmds {
        e.push(c);
    }
    e.as_slice().to_vec()
}

fn screen() -> egui::RawInput {
    egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(800.0, 600.0),
        )),
        ..Default::default()
    }
}

fn frame(
    ctx: &egui::Context,
    r: &mut Replayer,
    buf: &[u8],
    input: egui::RawInput,
) -> Vec<RespRecord> {
    let mut out = Vec::new();
    let mut full = ctx.run_ui(input, |ui| {
        out = r.replay(ui, APP, buf).expect("replays");
    });
    full.textures_delta.clear();
    out
}

fn find(recs: &[RespRecord], id: u64) -> Option<RespRecord> {
    recs.iter().find(|r| r.local_id == id).copied()
}

fn centre(rec: &RespRecord) -> egui::Pos2 {
    egui::pos2(
        (rec.rect[0] + rec.rect[2]) / 2.0,
        (rec.rect[1] + rec.rect[3]) / 2.0,
    )
}

fn press(pos: egui::Pos2, button: egui::PointerButton) -> egui::RawInput {
    let ev = |pressed| egui::Event::PointerButton {
        pos,
        button,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    egui::RawInput {
        events: vec![egui::Event::PointerMoved(pos), ev(true), ev(false)],
        ..screen()
    }
}

/// A file row's button with a right-click menu holding the three things Files offers.
fn file_with_menu() -> Vec<u8> {
    encode(&[
        Cmd::Button {
            id: 10,
            text: "notes.md",
        },
        Cmd::BeginScope {
            id: 11,
            layout: Layout::new(ScopeKind::ContextMenu, Align::Min),
        },
        Cmd::OpenApp {
            id: 12,
            label: "Open with Viewer",
            app: "viewer",
            arg: "/notes.md",
        },
        Cmd::CopyLink {
            id: 13,
            label: "Share with Viewer",
            path: "/app/viewer?open=%2Fnotes.md",
        },
        Cmd::OpenUrl {
            id: 14,
            label: "Download",
            url: "/files/notes.md",
        },
        Cmd::EndScope { id: 11 },
    ])
}

#[test]
fn a_context_menu_opens_on_right_click_and_its_entries_click() {
    let ctx = egui::Context::default();
    let mut r = Replayer::new();
    let buf = file_with_menu();

    let recs = frame(&ctx, &mut r, &buf, screen());
    assert!(
        find(&recs, 12).is_none(),
        "closed: its entries aren't drawn"
    );
    assert_eq!(
        r.open_apps(),
        [(12, "viewer".to_owned(), "/notes.md".to_owned())],
        "but the shell knows them, to act on a click"
    );
    assert_eq!(
        r.copy_links(),
        [(13, "/app/viewer?open=%2Fnotes.md".to_owned())]
    );

    let file = find(&recs, 10).unwrap();
    frame(
        &ctx,
        &mut r,
        &buf,
        press(centre(&file), egui::PointerButton::Secondary),
    );
    let recs = frame(&ctx, &mut r, &buf, screen());
    let open = find(&recs, 12).expect("open after a right click");
    assert!(find(&recs, 13).is_some() && find(&recs, 14).is_some());
    assert!(
        !find(&recs, 10).unwrap().clicked(),
        "a right click is not a click"
    );

    let recs = frame(
        &ctx,
        &mut r,
        &buf,
        press(centre(&open), egui::PointerButton::Primary),
    );
    assert!(
        find(&recs, 12).unwrap().clicked(),
        "the entry reports the click"
    );
    // Settle the click, which closes the menu.
    frame(&ctx, &mut r, &buf, screen());
    let recs = frame(&ctx, &mut r, &buf, screen());
    assert!(find(&recs, 12).is_none(), "and the menu closes");
}

#[test]
fn a_context_menu_with_nothing_before_it_draws_nothing() {
    let ctx = egui::Context::default();
    let mut r = Replayer::new();
    let buf = encode(&[
        Cmd::BeginScope {
            id: 11,
            layout: Layout::new(ScopeKind::ContextMenu, Align::Min),
        },
        Cmd::Button { id: 12, text: "x" },
        Cmd::EndScope { id: 11 },
    ]);
    assert!(frame(&ctx, &mut r, &buf, screen()).is_empty());
}

#[test]
fn media_reserves_its_space_and_tells_the_shell_where() {
    let ctx = egui::Context::default();
    let mut r = Replayer::new();
    let buf = encode(&[
        Cmd::Media {
            id: 20,
            src: "/files/clip.mp4?inline=1",
            kind: MediaKind::Video,
            size: Vec2::new(0.0, 0.0),
        },
        Cmd::Media {
            id: 21,
            src: "/files/song.mp3?inline=1",
            kind: MediaKind::Audio,
            size: Vec2::new(300.0, 0.0),
        },
    ]);
    let recs = frame(&ctx, &mut r, &buf, screen());
    let slots: &[MediaSlot] = r.media();
    assert_eq!(slots.len(), 2);

    let video = &slots[0];
    assert_eq!(video.local_id, 20);
    assert_eq!(video.kind, MediaKind::Video);
    assert_eq!(video.src, "/files/clip.mp4?inline=1");
    let w = video.rect.width();
    assert!(w > 700.0, "fills the row: {w}");
    assert!((video.rect.height() - w * 9.0 / 16.0).abs() < 0.5, "16:9");
    assert_eq!(video.visible, video.rect, "all of it on screen");

    let audio = &slots[1];
    assert_eq!(audio.rect.width(), 300.0);
    assert!(audio.rect.min.y >= video.rect.max.y, "below the video");
    assert!(find(&recs, 21).is_some(), "reported like any widget");
}

#[test]
fn media_off_screen_is_not_handed_to_the_shell() {
    let ctx = egui::Context::default();
    let mut r = Replayer::new();
    let buf = encode(&[Cmd::Media {
        id: 20,
        src: "/files/clip.mp4",
        kind: MediaKind::Video,
        // Taller than the screen, after something that pushes it past the bottom.
        size: Vec2::new(100.0, 100.0),
    }]);
    let mut out = Vec::new();
    let mut full = ctx.run_ui(screen(), |ui| {
        ui.add_space(2000.0);
        out = r.replay(ui, APP, &buf).unwrap();
    });
    full.textures_delta.clear();
    assert!(r.media().is_empty(), "{:?}", r.media());
}

#[test]
fn a_text_view_shows_the_text_and_typing_cannot_change_it() {
    let ctx = egui::Context::default();
    let mut r = Replayer::new();
    let view = |set: Option<&'static str>| {
        encode(&[Cmd::TextView {
            id: 30,
            version: 2,
            set,
        }])
    };
    let recs = frame(&ctx, &mut r, &view(Some("line one\nline two")), screen());
    assert_eq!(r.text(30), Some(("line one\nline two", 2)));

    // Focus it and type: a read-only buffer stays as it was, and nothing goes back to the app.
    let rec = find(&recs, 30).unwrap();
    frame(
        &ctx,
        &mut r,
        &view(None),
        press(centre(&rec), egui::PointerButton::Primary),
    );
    let typing = egui::RawInput {
        events: vec![egui::Event::Text("zzz".to_owned())],
        ..screen()
    };
    frame(&ctx, &mut r, &view(None), typing);
    assert_eq!(r.text(30), Some(("line one\nline two", 2)));
    assert!(r.take_text_events().is_none());
}

#[test]
fn an_image_is_asked_of_the_loader_by_its_url() {
    let ctx = egui::Context::default();
    let mut r = Replayer::new();
    let buf = encode(&[
        Cmd::Image {
            id: 40,
            src: "/files/a.png?inline=1",
            size: Vec2::new(0.0, 0.0),
        },
        Cmd::Image {
            id: 41,
            src: "/files/b.png?inline=1",
            size: Vec2::new(64.0, 48.0),
        },
    ]);
    let recs = frame(&ctx, &mut r, &buf, screen());
    // No loader is installed here, so neither has loaded; both are still laid out and reported.
    assert!(find(&recs, 40).is_some());
    let fixed = find(&recs, 41).unwrap();
    assert_eq!(fixed.rect[2] - fixed.rect[0], 64.0);
    assert_eq!(fixed.rect[3] - fixed.rect[1], 48.0);
}

#[test]
fn a_tables_cells_line_up_in_columns() {
    let ctx = egui::Context::default();
    let mut r = Replayer::new();
    let row = |id: u64, a: &'static str, b: &'static str| {
        vec![
            Cmd::BeginScope {
                id,
                layout: Layout::new(ScopeKind::Group, Align::Min),
            },
            Cmd::Label {
                id: id + 1,
                text: a,
            },
            Cmd::Label {
                id: id + 2,
                text: b,
            },
            Cmd::EndScope { id },
        ]
    };
    let mut cmds = vec![Cmd::BeginScope {
        id: 1,
        layout: Layout::new(ScopeKind::Table, Align::Min),
    }];
    cmds.extend(row(10, "name", "age"));
    cmds.extend(row(20, "a much longer name", "7"));
    cmds.push(Cmd::EndScope { id: 1 });
    let buf = encode(&cmds);

    // Two frames: a grid sizes its columns from what the previous frame measured.
    frame(&ctx, &mut r, &buf, screen());
    let recs = frame(&ctx, &mut r, &buf, screen());
    let cell = |id| find(&recs, id).unwrap().rect;
    assert_eq!(
        cell(12)[0],
        cell(22)[0],
        "the second column starts in one place"
    );
    assert!(
        cell(12)[0] > cell(21)[2],
        "after the widest cell of the first column"
    );
    assert!(cell(21)[1] > cell(11)[1], "rows go down");
    assert_eq!(cell(11)[1], cell(12)[1], "a row's cells share a line");
}
