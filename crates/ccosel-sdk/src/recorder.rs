//! Per-app recording state: the command buffer being built, and last frame's responses.

use alloc::vec::Vec;
use ccosel_abi::{Cmd, Encoder, RespRecord};

/// Owns the command buffer and the response table across frames.
///
/// An app keeps one of these for its lifetime. Steady-state frames reuse both allocations, so
/// rendering does not allocate.
pub struct Recorder {
    enc: Encoder,
    /// Last frame's responses, sorted by `local_id` for binary search.
    resp: Vec<RespRecord>,
}

impl Default for Recorder {
    fn default() -> Self {
        Self::new()
    }
}

impl Recorder {
    pub fn new() -> Self {
        Self {
            enc: Encoder::new(),
            resp: Vec::new(),
        }
    }

    /// Install the response table the host wrote for the previous frame.
    ///
    /// The host is expected to emit these already sorted; we sort defensively rather than
    /// trust it, because an unsorted table would make `lookup` silently return misses and
    /// produce a UI that just never responds to clicks — a miserable bug to chase.
    pub fn set_responses(&mut self, mut recs: Vec<RespRecord>) {
        recs.sort_unstable_by_key(|r| r.local_id);
        self.resp = recs;
    }

    pub(crate) fn lookup(&self, local_id: u64) -> RespRecord {
        match self.resp.binary_search_by_key(&local_id, |r| r.local_id) {
            Ok(i) => self.resp[i],
            Err(_) => RespRecord::default(),
        }
    }

    pub(crate) fn push(&mut self, cmd: &Cmd<'_>) {
        self.enc.push(cmd);
    }

    /// Start a new frame, keeping allocations.
    pub fn begin_frame(&mut self) {
        self.enc.clear();
    }

    /// The bytes to hand to the host.
    pub fn commands(&self) -> &[u8] {
        self.enc.as_slice()
    }
}
