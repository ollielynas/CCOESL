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

#[wasm_bindgen_test]
async fn responses_cross_the_boundary_and_change_guest_state() {
    let host = WebHost::new();
    let module = host.compile(GUEST).await.expect("compile");
    let mut app = host.instantiate(&module).expect("instantiate");

    let f1 = app.frame(&FrameArgs::default()).expect("frame");
    let target = button_id(&f1.commands, "notes.md");

    // Synthesise what the replayer would have reported for a click.
    let responses = vec![RespRecord {
        local_id: target,
        flags: ResponseFlags::ENABLED | ResponseFlags::HOVERED | ResponseFlags::CLICKED,
        ..Default::default()
    }];

    let f2 = app
        .frame(&FrameArgs {
            frame_index: 1,
            responses: &responses,
            ..Default::default()
        })
        .expect("frame");

    let labels = labels(&f2.commands);
    assert!(labels.contains(&"notes.md".to_owned()), "got {labels:?}");
    assert!(!labels.contains(&"nothing selected".to_owned()));
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
