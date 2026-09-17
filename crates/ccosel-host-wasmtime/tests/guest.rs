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

#[test]
fn drives_a_real_guest_module_end_to_end() {
    let host = WasmtimeHost::new();
    let module = host.compile(&guest_wasm()).expect("compile");
    let mut app = host.instantiate(&module).expect("instantiate");

    let ctx = egui::Context::default();
    let mut replayer = Replayer::new();

    // --- Frame 1: the guest renders its initial state.
    let f1 = app.frame(&FrameArgs::default()).expect("frame 1");
    assert!(!f1.commands.is_empty(), "guest produced no commands");
    ccosel_abi::validate(&f1.commands).expect("guest emitted an invalid buffer");
    assert!(labels(&f1.commands).contains(&"nothing selected".to_owned()));

    // An idle app must not ask to be re-run; that is what keeps a dozen open windows from
    // costing a dozen wasm invocations per display frame.
    assert_eq!(f1.wants_repaint_after_ms, ccosel_abi::REPAINT_ON_INPUT_ONLY);

    // egui's first pass runs with a cold font/galley cache, so widget rects are not settled
    // until a second pass. Warm up before measuring, exactly as a continuously-repainting
    // shell would.
    render(&ctx, &mut replayer, &f1.commands, raw_input());
    let responses = render(&ctx, &mut replayer, &f1.commands, raw_input());

    // --- Frame 2: click "notes.md" for real, through egui hit-testing.
    let target_id = button_id(&f1.commands, "notes.md");
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
    let responses = render(&ctx, &mut replayer, &f1.commands, input);
    assert!(
        responses
            .iter()
            .find(|r| r.local_id == target_id)
            .unwrap()
            .clicked(),
        "egui did not register the click"
    );

    // --- Frame 3: hand the responses back across the boundary; the guest reacts.
    let f3 = app
        .frame(&FrameArgs {
            frame_index: 2,
            responses: &responses,
            ..Default::default()
        })
        .expect("frame 3");

    let labels = labels(&f3.commands);
    assert!(
        labels.contains(&"notes.md".to_owned()),
        "guest did not record the selection; labels were {labels:?}"
    );
    assert!(!labels.contains(&"nothing selected".to_owned()));
}

#[test]
fn guest_state_persists_across_frames_and_staging_buffers_are_reused() {
    // Each frame allocates a staging buffer inside guest memory and frees it again. If that
    // leaked, guest memory would grow without bound — and wasm memory cannot shrink, so the
    // app could never be brought back down without being destroyed outright.
    let host = WasmtimeHost::new();
    let module = host.compile(&guest_wasm()).expect("compile");
    let mut app = host.instantiate(&module).expect("instantiate");

    let steady = app.frame(&FrameArgs::default()).expect("frame");
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
    let module = host.compile(&guest_wasm()).expect("compile");
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
