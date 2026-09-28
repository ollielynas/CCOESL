//! Per-app recording state: the command buffer being built, and last frame's responses.

use alloc::string::String;
use alloc::vec::Vec;
use ccosel_abi::event::TextDelta;
use ccosel_abi::view3d::ViewEvent;
use ccosel_abi::{Cmd, Encoder, RespRecord};

/// A text edit the shell reported, waiting for the app to draw that field again.
pub(crate) struct PendingDelta {
    pub id: u64,
    pub version: u32,
    pub start: usize,
    pub end: usize,
    pub inserted: String,
}

/// Owns the command buffer and the response table across frames.
///
/// An app keeps one of these for its lifetime. Steady-state frames reuse both allocations, so
/// rendering does not allocate.
pub struct Recorder {
    enc: Encoder,
    /// Last frame's responses, sorted by `local_id` for binary search.
    resp: Vec<RespRecord>,
    /// Text edits from the shell, applied when the app next draws the field they belong to.
    /// The app owns its `Text`s, so there is nowhere to apply them before that.
    deltas: Vec<PendingDelta>,
    /// Finished viewport gestures, handed to the app when it next draws that viewport.
    views: Vec<ViewEvent>,
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
            deltas: Vec::new(),
            views: Vec::new(),
        }
    }

    /// Hold a text edit from the shell until its field is drawn.
    pub(crate) fn queue_text_delta(&mut self, d: &TextDelta<'_>) {
        self.deltas.push(PendingDelta {
            id: d.id,
            version: d.version,
            start: d.start as usize,
            end: d.end as usize,
            inserted: String::from(d.inserted),
        });
    }

    /// Remove and return the edits waiting for field `id`, oldest first.
    pub(crate) fn take_text_deltas(&mut self, id: u64) -> Vec<PendingDelta> {
        if self.deltas.is_empty() {
            return Vec::new();
        }
        let (mine, rest) = core::mem::take(&mut self.deltas)
            .into_iter()
            .partition(|d| d.id == id);
        self.deltas = rest;
        mine
    }

    pub(crate) fn queue_view_event(&mut self, e: ViewEvent) {
        self.views.push(e);
    }

    /// Remove and return the gestures waiting for viewport `id`, oldest first.
    pub(crate) fn take_view_events(&mut self, id: u64) -> Vec<ViewEvent> {
        if self.views.is_empty() {
            return Vec::new();
        }
        let (mine, rest) = core::mem::take(&mut self.views)
            .into_iter()
            .partition(|e| e.id == id);
        self.views = rest;
        mine
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
