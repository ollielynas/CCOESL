//! Runs the browser backend against a real `WebAssembly` engine, under node.
//!
//! This is not a simulation: `WebAssembly.compile`, `WebAssembly.Instance`, the imports object
//! and the `Uint8Array` views over guest memory are all the genuine article, driven by the
//! same `ccosel-host-web` code the browser will run. What it does not cover is the DOM and
//! egui rendering — for that the shell has to be loaded in an actual page.
//!
//! Run with: `cargo test -p ccosel-host-web --target wasm32-unknown-unknown`

#![cfg(target_arch = "wasm32")]

use ccosel_abi::{Cmd, Decoder, RespRecord, ResponseFlags};
use ccosel_host::{AppHost, AppInstance, FrameArgs};
use ccosel_host_web::WebHost;
use wasm_bindgen_test::*;

// node is wasm-bindgen-test's default target; `wasm_bindgen_test_configure!` is only needed to
// opt *into* a browser.

const GUEST: &[u8] = include_bytes!(env!("CCOSEL_GUEST_WASM"));

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
        .expect("button not found")
}

#[wasm_bindgen_test]
async fn compiles_and_instantiates_a_real_guest() {
    let host = WebHost::new();
    let module = host.compile(GUEST).await.expect("compile");
    let _instance = host.instantiate(&module).expect("instantiate");
}

#[wasm_bindgen_test]
async fn frames_round_trip_through_guest_memory() {
    let host = WebHost::new();
    let module = host.compile(GUEST).await.expect("compile");
    let mut app = host.instantiate(&module).expect("instantiate");

    let f1 = app.frame(&FrameArgs::default()).expect("frame");
    ccosel_abi::validate(&f1.commands).expect("invalid command buffer");
    assert!(labels(&f1.commands).contains(&"nothing selected".to_owned()));
    assert_eq!(f1.wants_repaint_after_ms, ccosel_abi::REPAINT_ON_INPUT_ONLY);

    // Idle frames must be byte-identical, which is also what proves the per-frame staging
    // allocation is being freed rather than leaked.
    for i in 1..50 {
        let f = app
            .frame(&FrameArgs {
                frame_index: i,
                ..Default::default()
            })
            .expect("frame");
        assert_eq!(f.commands, f1.commands);
    }
}

/// Build the reply the server would have sent for a `list_dir`.
fn listing_reply(names: &[(&str, bool)]) -> Vec<u8> {
    use ccosel_proto::fs::{DirEntry, DirListing, EntryKind};
    let listing = DirListing {
        entries: names
            .iter()
            .map(|(n, is_dir)| DirEntry {
                name: (*n).to_owned(),
                kind: if *is_dir {
                    EntryKind::Dir
                } else {
                    EntryKind::File
                },
                size: 3,
                mtime_s: 0,
            })
            .collect(),
        truncated: false,
    };
    postcard::to_allocvec(&listing).unwrap()
}

#[wasm_bindgen_test]
async fn rpc_round_trips_through_the_real_wasm_engine() {
    // The full guest-side RPC path under a genuine WebAssembly engine: the app asks, the
    // request crosses the import boundary, the reply crosses back through `on_event`, and the
    // next frame renders it. Only the network itself is simulated.
    let host = WebHost::new();
    let module = host.compile(GUEST).await.expect("compile");
    let mut app = host.instantiate(&module).expect("instantiate");

    let f1 = app.frame(&FrameArgs::default()).expect("frame");
    assert!(labels(&f1.commands).contains(&"Loading…".to_owned()));

    let calls = app.take_outbox();
    assert_eq!(calls.len(), 1, "one request for the initial listing");
    assert_eq!(calls[0].method, ccosel_proto::Method::ListDir as u32);
    let req: ccosel_proto::fs::ListDirReq = postcard::from_bytes(&calls[0].args).unwrap();
    assert_eq!(req.path, "/");

    let payload = listing_reply(&[("Projects", true), ("notes.md", false)]);
    let batch = ccosel_abi::event::encode_batch(&[(
        ccosel_abi::event::event_kind::RPC_OK,
        calls[0].call_id,
        &payload,
    )]);
    app.on_event(&batch).expect("deliver reply");

    let f2 = app
        .frame(&FrameArgs {
            frame_index: 1,
            ..Default::default()
        })
        .expect("frame");
    assert!(!labels(&f2.commands).contains(&"Loading…".to_owned()));
    let _ = button_id(&f2.commands, "notes.md");
    let _ = button_id(&f2.commands, "Projects");
}

#[wasm_bindgen_test]
async fn a_server_error_renders_without_a_decoder() {
    let host = WebHost::new();
    let module = host.compile(GUEST).await.expect("compile");
    let mut app = host.instantiate(&module).expect("instantiate");

    app.frame(&FrameArgs::default()).expect("frame");
    let calls = app.take_outbox();

    let payload =
        ccosel_abi::event::encode_error(ccosel_abi::event::rpc_error::DENIED, "outside the jail");
    let batch = ccosel_abi::event::encode_batch(&[(
        ccosel_abi::event::event_kind::RPC_ERR,
        calls[0].call_id,
        &payload,
    )]);
    app.on_event(&batch).expect("deliver error");

    let f = app
        .frame(&FrameArgs {
            frame_index: 1,
            ..Default::default()
        })
        .expect("frame");
    assert!(
        labels(&f.commands).contains(&"permission denied".to_owned()),
        "got {:?}",
        labels(&f.commands)
    );
}

#[wasm_bindgen_test]
async fn survives_a_large_response_table() {
    // Big staging buffers are the case most likely to make the guest grow its memory, which
    // detaches the ArrayBuffer and invalidates any view held across the call. If the
    // detachment rule were broken anywhere, this is where it would show up as corruption.
    let host = WebHost::new();
    let module = host.compile(GUEST).await.expect("compile");
    let mut app = host.instantiate(&module).expect("instantiate");

    let baseline = app.frame(&FrameArgs::default()).expect("frame");
    let _ = app.take_outbox();

    let junk: Vec<RespRecord> = (0..20_000)
        .map(|i| RespRecord {
            local_id: u64::MAX - i,
            flags: u32::MAX,
            ..Default::default()
        })
        .collect();

    let f = app
        .frame(&FrameArgs {
            frame_index: 1,
            responses: &junk,
            ..Default::default()
        })
        .expect("frame with a large response table");
    assert_eq!(f.commands, baseline.commands);

    // And the guest is still healthy afterwards.
    let after = app
        .frame(&FrameArgs {
            frame_index: 2,
            ..Default::default()
        })
        .expect("frame");
    assert_eq!(after.commands, baseline.commands);
}
