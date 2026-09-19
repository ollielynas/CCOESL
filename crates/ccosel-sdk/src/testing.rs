//! A native harness for testing apps, behind the `testing` feature.
//!
//! In production the shell supplies an app's recorder, frame context and RPC cache, and reaches
//! it through raw `u32` wasm pointers — which cannot be dereferenced on a 64-bit native target.
//! This wires those pieces together directly, so a test can drive an app the way a user and the
//! server would: run a frame, click a button by its label, answer an RPC, run the next frame,
//! and assert on what was drawn.
//!
//! ```ignore
//! let mut h = Harness::new(MyApp::default());
//! h.frame();                                   // the app asks the server for something
//! h.reply::<ListDir>(&DirListing { .. });      // the server answers
//! h.frame();
//! assert!(h.has_label("notes.md"));
//! h.click("Refresh");                          // observed by the app on the next frame
//! h.frame();
//! ```
//!
//! Off by default: this is test scaffolding and the SDK's budget is measured in kilobytes. Apps
//! enable it as a dev-dependency only, so it never reaches a shipped module.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use ccosel_abi::event::{Event, encode_error, event_kind};
use ccosel_abi::{Cmd, Decoder, RespRecord, ResponseFlags};

/// The codes [`Harness::fail`] takes, re-exported so an app's tests need no `ccosel-abi`
/// dependency of their own.
pub use ccosel_abi::event::rpc_error;
use ccosel_proto::Rpc;
use serde::Serialize;

use crate::rpc::OutCall;
use crate::{App, FrameCtx, Recorder, RpcCtx, Ui};

pub struct Harness<A: App> {
    /// The app under test. Public so a test can set up or inspect its state directly.
    pub app: A,
    /// Inputs (time, screen size, flags) the next frame will see.
    pub ctx: FrameCtx,
    rec: Recorder,
    rpc: RpcCtx,
    clicks: Vec<RespRecord>,
    calls: Vec<OutCall>,
    last: Vec<u8>,
}

impl<A: App> Harness<A> {
    pub fn new(app: A) -> Self {
        Self {
            app,
            ctx: FrameCtx::default(),
            rec: Recorder::new(),
            rpc: RpcCtx::new(),
            clicks: Vec::new(),
            calls: Vec::new(),
            last: Vec::new(),
        }
    }

    /// Runs one frame, as the runtime would: clicks queued since the last frame are reported to
    /// the app now (a response describes the *previous* frame), and any RPC the app issues is
    /// held until the test answers it.
    ///
    /// Panics if the app emits a command buffer the host would reject, so every frame a test
    /// runs is also checked for well-formedness.
    pub fn frame(&mut self) {
        self.rec.set_responses(core::mem::take(&mut self.clicks));
        self.rpc.begin_frame();
        {
            let mut ui = Ui::root(&mut self.rec, self.ctx, &self.rpc);
            self.app.update(&mut ui);
        }
        self.calls.extend(self.rpc.take_outbox());
        // The runtime forwards cancellations to the host; there is no host here to tell.
        let _ = self.rpc.take_cancels();
        self.last = self.rec.commands().to_vec();
        ccosel_abi::validate(&self.last).expect("the app emitted a malformed command buffer");
    }

    fn commands(&self) -> impl Iterator<Item = Cmd<'_>> {
        Decoder::new(&self.last).map(|c| c.expect("last frame decodes"))
    }

    /// Text of every label drawn in the last frame, in order.
    pub fn labels(&self) -> Vec<String> {
        self.commands()
            .filter_map(|c| match c {
                Cmd::Label { text, .. } => Some(text.to_string()),
                _ => None,
            })
            .collect()
    }

    /// Text of every button drawn in the last frame, in order.
    pub fn buttons(&self) -> Vec<String> {
        self.commands()
            .filter_map(|c| match c {
                Cmd::Button { text, .. } => Some(text.to_string()),
                _ => None,
            })
            .collect()
    }

    pub fn has_label(&self, text: &str) -> bool {
        self.labels().iter().any(|l| l == text)
    }

    pub fn has_button(&self, text: &str) -> bool {
        self.buttons().iter().any(|b| b == text)
    }

    /// Clicks the first button with this label in the last frame. The app observes it on the
    /// next [`frame`](Self::frame). Panics, listing what was drawn, if there is no such button.
    pub fn click(&mut self, text: &str) {
        let id = self
            .commands()
            .find_map(|c| match c {
                Cmd::Button { id, text: t } if t == text => Some(id),
                _ => None,
            })
            .unwrap_or_else(|| {
                panic!(
                    "no button labelled {text:?} in the last frame; buttons were {:?}",
                    self.buttons()
                )
            });
        self.clicks.push(RespRecord {
            local_id: id,
            flags: ResponseFlags::CLICKED | ResponseFlags::HOVERED | ResponseFlags::ENABLED,
            ..Default::default()
        });
    }

    /// How many calls to `M` the app has issued that the test has not yet answered.
    pub fn outstanding<M: Rpc>(&self) -> usize {
        self.calls
            .iter()
            .filter(|c| c.method == M::METHOD as u32)
            .count()
    }

    /// Answers the oldest outstanding call to `M` with `reply`. Panics if there is none.
    pub fn reply<M: Rpc>(&mut self, reply: &M::Reply)
    where
        M::Reply: Serialize,
    {
        let payload = postcard::to_allocvec(reply).expect("the reply serializes");
        self.answer::<M>(event_kind::RPC_OK, &payload);
    }

    /// Fails the oldest outstanding call to `M` with an `rpc_error` code. Panics if there is none.
    pub fn fail<M: Rpc>(&mut self, code: u32) {
        let payload = encode_error(code, "injected by the test harness");
        self.answer::<M>(event_kind::RPC_ERR, &payload);
    }

    fn answer<M: Rpc>(&mut self, kind: u32, payload: &[u8]) {
        let at = self
            .calls
            .iter()
            .position(|c| c.method == M::METHOD as u32)
            .expect("the app has no outstanding call to answer");
        let call = self.calls.remove(at);
        self.rpc.deliver(&Event {
            kind,
            call_id: call.call_id,
            payload,
        });
    }
}
