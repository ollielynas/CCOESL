//! The CCOSEL app SDK.
//!
//! This is the only crate an app links against, and **it must stay small** — every byte here is
//! downloaded again for every app, over a LAN we assume is bad. The budget is 40–70 KB of wasm.
//! Concretely that means: no `egui`, no `serde_json`, no `regex`, no `chrono`, and no float
//! `Display` in hot paths.
//!
//! The API deliberately *looks* like egui but does something different: it records widget calls
//! into a flat command buffer that the shell replays into a real `egui::Ui`. Two consequences
//! an app author has to internalise:
//!
//! 1. **[`Response`] describes the previous frame.** The shell owns hover, focus, drag, scroll
//!    and text state, and it is the shell that draws the interaction — so nothing the *user*
//!    sees is stale. Your code merely learns the committed result one frame later.
//! 2. **Never branch on a response in order to draw.** `if r.hovered() { show tooltip }` would
//!    lag by a frame. Emit [`Ui::tooltip`] unconditionally and let the shell decide.

#![no_std]

extern crate alloc;

mod recorder;
pub mod rpc;
pub mod runtime;
mod response;
mod ui;

pub use ccosel_abi::{Align, Color32, Pos2, Rect, ScopeKind, Vec2};
pub use ccosel_abi::{ABI_VERSION, REPAINT_ON_INPUT_ONLY};
pub use recorder::Recorder;
pub use rpc::{Poll, RpcCtx, RpcError};
pub use response::Response;
pub use ui::{FrameCtx, Text, Ui};

/// Implemented by every CCOSEL app.
pub trait App {
    /// Build one frame of UI. Called by the shell when input targets this app or its repaint
    /// timer fires — *not* necessarily at display rate.
    fn update(&mut self, ui: &mut Ui<'_>);

    /// How long until this app wants to be re-run even with no input, in milliseconds.
    ///
    /// The default, [`REPAINT_ON_INPUT_ONLY`](ccosel_abi::REPAINT_ON_INPUT_ONLY), means "I am
    /// not animating" and costs zero CPU between interactions. Override it only while something
    /// is actually moving — an app that returns `0` here burns a core for no reason.
    fn wants_repaint_after_ms(&self) -> u32 {
        ccosel_abi::REPAINT_ON_INPUT_ONLY
    }
}
