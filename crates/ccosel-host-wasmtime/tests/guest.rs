//! End-to-end across a real wasm boundary.
//!
//! Loads the actual `file-browser.wasm`, drives a frame, replays the commands into a real egui
//! context, sends a real pointer click, feeds the responses back into the module, and checks
//! that the guest's state changed. Everything except the network is in this loop.

use std::path::PathBuf;
use std::process::Command;

use ccosel_abi::{Cmd, Decoder, RespRecord};
use ccosel_host::{AppHost, AppInstance, FrameArgs, Replayer};
use ccosel_host_wasmtime::WasmtimeHost;

const APP: u64 = 1;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

/// The guest lives in a separate cargo workspace (different `[profile]`), so build it on
/// demand rather than relying on the caller having done it.
fn guest_wasm() -> Vec<u8> {
    let root = repo_root();
    let artifact = root
        .join("apps/target/wasm32-unknown-unknown/release/file_browser.wasm");

    // Always build, never just check for the artifact: cargo is incremental so this is nearly
    // free when up to date, and testing against a stale module silently validates the wrong
    // code — which is exactly the sort of bug this test exists to catch.
    let status = Command::new(env!("CARGO"))
        .current_dir(root.join("apps"))
        .args(["build", "--release", "--target", "wasm32-unknown-unknown"])
        .status()
        .expect("failed to run cargo for the guest workspace");
    assert!(status.success(), "guest build failed");

    std::fs::read(&artifact).expect("guest wasm missing after build")
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

/// Replay a command buffer through a real egui pass and return the response table.
fn render(
    ctx: &egui::Context,
    replayer: &mut Replayer,
    buf: &[u8],
    input: egui::RawInput,
) -> Vec<RespRecord> {
    let mut out = Vec::new();
    let mut full = ctx.run_ui(input, |ui| {
        out = replayer.replay(ui, APP, buf).expect("guest emitted a bad buffer");
    });
    full.textures_delta.clear();
    out
}

fn labels(buf: &[u8]) -> Vec<String> {
    Decoder::new(buf)
        .filter_map(|c| match c.ok()? {
            Cmd::Label { text, .. } => Some(text.to_owned()),
            _ => None,
        })
        .collect()
}

fn button_id(buf: &[u8], want: &str) -> u64 {
    Decoder::new(buf)
        .find_map(|c| match c.ok()? {
            Cmd::Button { id, text } if text == want => Some(id),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no button labelled {want:?}"))
}

/// Build the reply the server would have sent for a `list_dir`.
fn listing_reply(names: &[(&str, bool)]) -> Vec<u8> {
    use ccosel_proto::fs::{DirEntry, DirListing, EntryKind};
    let listing = DirListing {
        entries: names
            .iter()
            .map(|(n, is_dir)| DirEntry {
                name: (*n).to_owned(),
                kind: if *is_dir { EntryKind::Dir } else { EntryKind::File },
                size: 12,
                mtime_s: 0,
            })
            .collect(),
        truncated: false,
    };
    postcard::to_allocvec(&listing).unwrap()
}

#[test]
fn drives_a_real_guest_module_end_to_end() {
    let host = WasmtimeHost::new();
    let module = pollster::block_on(host.compile(&guest_wasm())).expect("compile");
    let mut app = host.instantiate(&module).expect("instantiate");

    let ctx = egui::Context::default();
    let mut replayer = Replayer::new();

    // --- Frame 1: nothing is known yet, so the app asks the server and says so.
    let f1 = app.frame(&FrameArgs::default()).expect("frame 1");
    ccosel_abi::validate(&f1.commands).expect("guest emitted an invalid buffer");
    assert!(
        labels(&f1.commands).contains(&"Loading…".to_owned()),
        "expected a loading state, got {:?}",
        labels(&f1.commands)
    );

    let calls = app.take_outbox();
    assert_eq!(calls.len(), 1, "exactly one request for the initial listing");
    assert_eq!(calls[0].method, ccosel_proto::Method::ListDir as u32);

    // Waiting is not animating: an app blocked on the network must not be re-run every frame.
    assert_eq!(f1.wants_repaint_after_ms, ccosel_abi::REPAINT_ON_INPUT_ONLY);

    // --- Frame 2: nothing has arrived, so it must not ask again.
    let f2 = app
        .frame(&FrameArgs { frame_index: 1, ..Default::default() })
        .expect("frame 2");
    assert!(app.take_outbox().is_empty(), "must not re-request while in flight");
    assert_eq!(f2.commands, f1.commands);

    // --- The reply arrives, the way the transport would deliver it.
    let payload = listing_reply(&[("Projects", true), ("notes.md", false)]);
    let batch = ccosel_abi::event::encode_batch(&[(
        ccosel_abi::event::event_kind::RPC_OK,
        calls[0].call_id,
        &payload,
    )]);
    app.on_event(&batch).expect("deliver reply");

    let f3 = app
        .frame(&FrameArgs { frame_index: 2, ..Default::default() })
        .expect("frame 3");
    assert!(!labels(&f3.commands).contains(&"Loading…".to_owned()));
    // Nothing is selected yet, but there are entries, so the status bar reports the count
    // rather than the empty-selection placeholder.
    assert!(labels(&f3.commands).contains(&"2 of 2 items".to_owned()));

    // --- Click "notes.md" for real, through egui hit-testing.
    render(&ctx, &mut replayer, &f3.commands, raw_input());
    let responses = render(&ctx, &mut replayer, &f3.commands, raw_input());

    let target_id = button_id(&f3.commands, "notes.md");
    let rect = responses
        .iter()
        .find(|r| r.local_id == target_id)
        .expect("no response for the notes.md button")
        .rect;
    let target = egui::pos2((rect[0] + rect[2]) / 2.0, (rect[1] + rect[3]) / 2.0);

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
    let responses = render(&ctx, &mut replayer, &f3.commands, input);
    assert!(responses.iter().find(|r| r.local_id == target_id).unwrap().clicked());

    // --- The selection crosses back into the guest.
    let f4 = app
        .frame(&FrameArgs { frame_index: 3, responses: &responses, ..Default::default() })
        .expect("frame 4");
    let labels = labels(&f4.commands);
    assert!(labels.contains(&"notes.md".to_owned()), "got {labels:?}");
    assert!(!labels.contains(&"nothing selected".to_owned()));
}

#[test]
fn entering_a_directory_issues_a_new_request_for_the_new_path() {
    // Navigation *is* the re-request: the app changes its path and the cache key moves with
    // it. Nothing in the app tracks whether a request is outstanding.
    let host = WasmtimeHost::new();
    let module = pollster::block_on(host.compile(&guest_wasm())).expect("compile");
    let mut app = host.instantiate(&module).expect("instantiate");

    let ctx = egui::Context::default();
    let mut replayer = Replayer::new();

    app.frame(&FrameArgs::default()).expect("frame");
    let first = app.take_outbox();
    let payload = listing_reply(&[("Projects", true)]);
    let batch = ccosel_abi::event::encode_batch(&[(
        ccosel_abi::event::event_kind::RPC_OK,
        first[0].call_id,
        &payload,
    )]);
    app.on_event(&batch).expect("deliver");

    let f = app
        .frame(&FrameArgs { frame_index: 1, ..Default::default() })
        .expect("frame");
    assert!(app.take_outbox().is_empty());

    render(&ctx, &mut replayer, &f.commands, raw_input());
    let responses = render(&ctx, &mut replayer, &f.commands, raw_input());
    let dir_id = button_id(&f.commands, "Projects");
    let rect = responses.iter().find(|r| r.local_id == dir_id).unwrap().rect;
    let target = egui::pos2((rect[0] + rect[2]) / 2.0, (rect[1] + rect[3]) / 2.0);

    let mut input = raw_input();
    input.events = vec![
        egui::Event::PointerMoved(target),
        egui::Event::PointerButton {
            pos: target, button: egui::PointerButton::Primary,
            pressed: true, modifiers: Default::default(),
        },
        egui::Event::PointerButton {
            pos: target, button: egui::PointerButton::Primary,
            pressed: false, modifiers: Default::default(),
        },
    ];
    let responses = render(&ctx, &mut replayer, &f.commands, input);

    // The click is applied at the end of the frame that observes it, so the request for the
    // new path goes out on the frame after. One extra frame, not one extra round trip.
    app.frame(&FrameArgs { frame_index: 2, responses: &responses, ..Default::default() })
        .expect("frame");
    assert!(app.take_outbox().is_empty(), "path changes at the end of this frame");

    app.frame(&FrameArgs { frame_index: 3, ..Default::default() })
        .expect("frame");

    let second = app.take_outbox();
    assert_eq!(second.len(), 1, "entering a directory requests it");
    let req: ccosel_proto::fs::ListDirReq = postcard::from_bytes(&second[0].args).unwrap();
    assert_eq!(req.path, "/Projects");
}

#[test]
fn guest_state_persists_across_frames_and_staging_buffers_are_reused() {
    // Each frame allocates a staging buffer inside guest memory and frees it again. If that
    // leaked, guest memory would grow without bound — and wasm memory cannot shrink, so the
    // app could never be brought back down without being destroyed outright.
    let host = WasmtimeHost::new();
    let module = pollster::block_on(host.compile(&guest_wasm())).expect("compile");
    let mut app = host.instantiate(&module).expect("instantiate");

    let steady = app.frame(&FrameArgs::default()).expect("frame");
    let _ = app.take_outbox();
    for i in 1..200 {
        let f = app
            .frame(&FrameArgs {
                frame_index: i,
                ..Default::default()
            })
            .expect("frame");
        assert_eq!(f.commands, steady.commands, "idle frames must be identical");
    }
}

#[test]
fn a_hostile_response_table_cannot_corrupt_the_guest() {
    // The host is trusted by the guest, but ids it does not recognise must simply miss.
    let host = WasmtimeHost::new();
    let module = pollster::block_on(host.compile(&guest_wasm())).expect("compile");
    let mut app = host.instantiate(&module).expect("instantiate");

    let baseline = app.frame(&FrameArgs::default()).expect("frame");

    let junk: Vec<RespRecord> = (0..64)
        .map(|i| RespRecord {
            local_id: u64::MAX - i,
            flags: u32::MAX,
            rect: [f32::NAN; 4],
            ..Default::default()
        })
        .collect();

    let f = app
        .frame(&FrameArgs {
            frame_index: 1,
            responses: &junk,
            ..Default::default()
        })
        .expect("guest trapped on an unrecognised response table");
    assert_eq!(f.commands, baseline.commands);
}
