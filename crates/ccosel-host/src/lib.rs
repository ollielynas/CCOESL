//! Replay an app's command stream into a real `egui::Ui`.
//!
//! This is the hardest and most security-sensitive code in the project, so it deliberately
//! depends on **neither wasm runtime** — not `wasmtime`, not `wasm-bindgen`. It takes a byte
//! slice and an `egui::Ui`, which means it can be tested against a hand-built buffer and an
//! offscreen `egui::Context` with no wasm, no browser and no window in the loop.
//!
//! The input is untrusted. A guest may be buggy, mid-upgrade, or hostile; the contract here is
//! that no byte sequence can panic, corrupt the `Ui` stack, or take down the desktop. Buffers
//! are validated whole *before* any of them is replayed, so a frame that turns out to be
//! malformed halfway through is discarded entirely and the previous frame stands.

mod convert;
mod host;
mod replay;

pub use host::{AppHost, AppInstance, FrameArgs, FrameResult, HostError};
pub use replay::{ReplayError, Replayer};
