//! The Rust Compiler app's method: build a jailed project directory, report what it produced.
//!
//! A build runs for minutes, so this is **not** a call that waits for one. `Compile` returns a
//! snapshot immediately: the first call for a given `(path, generation)` starts the job, every
//! later call reports how far it has got. The app polls, and `ARCHITECTURE.md`'s "anything over
//! ~2s is a Job" holds without needing the push channel that does not exist yet.

use alloc::string::String;
use alloc::vec::Vec;
use serde::{Deserialize, Serialize};

use crate::{Coalesce, Effect, Method, Query, Rpc};

#[derive(Debug, Serialize, Deserialize)]
pub struct CompileReq<'a> {
    /// A jail-relative directory containing a `Cargo.toml`, same convention as `ListDirReq`.
    #[serde(borrow)]
    pub path: &'a str,
    /// Bumped by the app once per Build press. Repeating a `(path, generation)` means "the job
    /// I already started", so polling can never kick off a second build — this is the
    /// idempotency key a job is supposed to carry, expressed through the request cache's key.
    pub generation: u32,
}

/// One executable `cargo build` produced. `path` is jail-relative, so the app can turn it
/// straight into a download link without ever seeing a filesystem-absolute path.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BuiltBinary {
    pub name: String,
    pub path: String,
    pub size: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompileResult {
    pub success: bool,
    /// Rendered compiler diagnostics. Tail-capped like a directory listing is entry-capped —
    /// see `output_truncated` — because a guest must never be handed an unbounded reply.
    pub output: String,
    pub output_truncated: bool,
    pub binaries: Vec<BuiltBinary>,
}

/// Where a build has got to. Cheap enough to re-send at a few hertz, which is the whole point.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompileStatus {
    pub finished: bool,
    pub units_done: u32,
    /// Cargo's resolved package count, or 0 while that is still unknown. An estimate on
    /// purpose: cargo does not publish its unit graph on stable, so this is the honest
    /// denominator available rather than a precise one that would need nightly.
    pub units_total: u32,
    /// What cargo is working on right now, e.g. `serde v1.0.229`.
    pub current: String,
    pub elapsed_ms: u64,
    /// Present exactly when `finished`.
    pub result: Option<CompileResult>,
}

impl CompileStatus {
    /// Progress in 0..=1, or negative when there is no denominator to divide by. Lives here so
    /// the app and any future progress UI agree on what the numbers mean.
    pub fn fraction(&self) -> f32 {
        if self.finished {
            return 1.0;
        }
        if self.units_total == 0 {
            return -1.0;
        }
        let f = self.units_done as f32 / self.units_total as f32;
        // Never show a full bar before the build says it is done, or the last stretch looks
        // like a hang instead of linking.
        if f > 0.99 { 0.99 } else { f }
    }
}

pub struct Compile;

impl Rpc for Compile {
    const METHOD: Method = Method::Compile;
    const COALESCE: Coalesce = Coalesce::ByArgs;
    // Honest, not a convenience: repeating a `(path, generation)` returns the same job's
    // snapshot and starts nothing, so this really is safe to retry and safe to cache.
    const EFFECT: Effect = Effect::Idempotent;
    // A poll, not a build: the server answers out of its job table without waiting.
    const DEADLINE_MS: u32 = 8_000;
    type Req<'a> = CompileReq<'a>;
    type Reply = CompileStatus;
}

impl Query for Compile {}
