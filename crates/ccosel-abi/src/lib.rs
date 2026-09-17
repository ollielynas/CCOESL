//! The CCOSEL app ABI: the contract between a guest app module and the shell.
//!
//! This crate is the *only* thing both sides are allowed to agree on. It is `no_std` and
//! depends on nothing but `bytemuck`, because every byte it pulls in is paid for in every
//! app module downloaded over a slow LAN.
//!
//! Three rules govern everything here:
//!
//! 1. **Guest output is untrusted input.** The decoder never panics on a malformed buffer;
//!    it returns [`DecodeError`].
//! 2. **Nothing borrows across a guest call.** The host copies command bytes out of guest
//!    memory at flush time (on web, `memory.buffer` detaches when the guest grows memory;
//!    on wasmtime, `Memory::data_mut` borrows the store).
//! 3. **The shell owns all egui state.** The guest owns only its model. See `frame::RespRecord`.

#![no_std]

extern crate alloc;

pub mod decode;
pub mod encode;
pub mod frame;
pub mod geom;
pub mod id;
pub mod opcode;

pub use decode::{Cmd, DecodeError, Decoder, validate};
pub use encode::Encoder;
pub use frame::{FrameInput, FrameOutput, RespRecord, ResponseFlags, Slice};
pub use geom::{Align, Color32, Layout, Pos2, Rect, ScopeKind, Vec2};
pub use id::{hash_bytes, hash_id, hash_str};
pub use opcode::OpCode;

/// Bumped on any incompatible change to the command stream, the frame structs, or the
/// guest export list. The shell refuses to instantiate a module whose
/// `ccosel_abi_version()` does not match.
pub const ABI_VERSION: u32 = 1;

/// Maximum scope nesting a guest may emit. Bounds the host's `Vec<egui::Ui>` stack so a
/// malicious or buggy guest cannot drive it into unbounded recursion.
pub const MAX_SCOPE_DEPTH: u32 = 64;

/// Sentinel for `FrameOutput::wants_repaint_after_ms`: repaint only when input arrives.
pub const REPAINT_ON_INPUT_ONLY: u32 = u32::MAX;
