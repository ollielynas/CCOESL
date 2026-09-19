//! The per-frame structs passed across the boundary.
//!
//! All `#[repr(C)]` with explicit padding so the layout is identical under wasm32 and under
//! the host's native target — `ccosel-host` unit tests decode these on x86-64.

use bytemuck::{Pod, Zeroable};

/// Written by the host into guest memory immediately before `ccosel_frame`.
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
#[repr(C)]
pub struct FrameInput {
    pub frame_index: u64,
    pub time_ms: f64,
    pub dt_ms: f32,
    pub pixels_per_point: f32,
    pub screen_size: [f32; 2],
    pub abi_version: u32,
    /// `RespRecord[]` from the *previous* frame, sorted by `local_id`.
    pub resp_ptr: u32,
    pub resp_len: u32,
    /// Input events plus async deliveries (RPC results already went via `ccosel_on_event`).
    pub event_ptr: u32,
    pub event_len: u32,
    pub flags: u32,
    pub _pad: [u32; 2],
}

/// Environment flags in [`FrameInput::flags`]. Apps are expected to honour these; the shell
/// sets `LOW_BANDWIDTH` when measured throughput drops, and apps should degrade (icons instead
/// of thumbnails, summary instead of full diagnostics) rather than queue more requests.
pub mod input_flags {
    pub const DARK_MODE: u32 = 1 << 0;
    pub const REDUCED_MOTION: u32 = 1 << 1;
    pub const LOW_BANDWIDTH: u32 = 1 << 2;
    /// This app is running in a Worker; blocking calls into the shell are impossible.
    /// (Always the safe assumption — see `ccosel_frame` docs.)
    pub const OFF_MAIN_THREAD: u32 = 1 << 3;
}

/// Returned by `ccosel_frame` — a pointer to one of these lives in guest memory.
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
#[repr(C)]
pub struct FrameOutput {
    pub cmd_ptr: u32,
    pub cmd_len: u32,
    /// `REPAINT_ON_INPUT_ONLY` (`u32::MAX`) means "I am not animating".
    ///
    /// This is what decouples app tick rate from frame rate: the shell re-tessellates the
    /// *cached* command buffer at display rate and only re-runs a guest when input targets it
    /// or this timer expires.
    pub wants_repaint_after_ms: u32,
    pub status: u32,
}

/// A pointer/length pair in guest memory.
///
/// Used instead of packing both halves into a `u64` return value: wasm `i64` surfaces in JS as
/// a `BigInt`, which makes the browser backend's glue fiddly for no benefit. Returning a
/// pointer to one of these keeps every export's return type a plain `u32`.
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
#[repr(C)]
pub struct Slice {
    pub ptr: u32,
    pub len: u32,
}

/// `FrameOutput::status` values.
pub mod status {
    pub const OK: u32 = 0;
    /// The guest caught a panic and is in a degraded but renderable state.
    pub const RECOVERED: u32 = 1;
    /// The guest is unusable; the shell should show an error window and offer a restart.
    pub const FATAL: u32 = 2;
}

/// What the host tells a guest about one of its widgets — always describing the *previous*
/// frame. Exactly 48 bytes.
///
/// Staleness is invisible to the user because the shell, not the guest, draws the interaction:
/// it owns hover, focus, drag, scroll offsets, and text buffers. A guest storing a scroll
/// offset or a window rect is a bug.
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
#[repr(C, align(8))]
pub struct RespRecord {
    pub local_id: u64,
    /// See [`ResponseFlags`]. Mirrors `egui::response::Flags`.
    pub flags: u32,
    /// Opcode-specific: for `TextEditSingle`, the committed text version.
    pub aux: u32,
    pub rect: [f32; 4],
    pub drag_delta: [f32; 2],
    /// Committed value for sliders and drag-values. The shell owns the *live* value during a
    /// drag and paints it, so the guest seeing it a frame late has no visual consequence.
    pub value: f32,
    pub _pad: u32,
}

/// Bit positions in [`RespRecord::flags`], mirroring `egui::response::Flags` so the host
/// conversion is a straight copy rather than a translation table.
pub struct ResponseFlags;

impl ResponseFlags {
    pub const ENABLED: u32 = 1 << 0;
    pub const CONTAINS_POINTER: u32 = 1 << 1;
    pub const HOVERED: u32 = 1 << 2;
    pub const HIGHLIGHTED: u32 = 1 << 3;
    pub const CLICKED: u32 = 1 << 4;
    pub const FAKE_PRIMARY_CLICKED: u32 = 1 << 5;
    pub const LONG_TOUCHED: u32 = 1 << 6;
    pub const DRAG_STARTED: u32 = 1 << 7;
    pub const DRAGGED: u32 = 1 << 8;
    pub const DRAG_STOPPED: u32 = 1 << 9;
    pub const IS_POINTER_BUTTON_DOWN_ON: u32 = 1 << 10;
    pub const CHANGED: u32 = 1 << 11;
    pub const HAS_FOCUS: u32 = 1 << 12;
    pub const GAINED_FOCUS: u32 = 1 << 13;
    pub const LOST_FOCUS: u32 = 1 << 14;
}

impl RespRecord {
    pub fn clicked(&self) -> bool {
        self.flags & (ResponseFlags::CLICKED | ResponseFlags::FAKE_PRIMARY_CLICKED) != 0
    }
    pub fn hovered(&self) -> bool {
        self.flags & ResponseFlags::HOVERED != 0
    }
    pub fn changed(&self) -> bool {
        self.flags & ResponseFlags::CHANGED != 0
    }
    pub fn dragged(&self) -> bool {
        self.flags & ResponseFlags::DRAGGED != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::{align_of, size_of};

    /// These sizes are ABI. If one of these fails, `ABI_VERSION` must be bumped.
    #[test]
    fn layout_is_pinned() {
        assert_eq!(size_of::<RespRecord>(), 48);
        assert_eq!(align_of::<RespRecord>(), 8);
        assert_eq!(size_of::<FrameInput>(), 64);
        assert_eq!(size_of::<FrameOutput>(), 16);
    }
}
