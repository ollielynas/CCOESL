//! Guest-side command encoder.
//!
//! Wire format is deliberately dumb: `u8` opcode followed by a fixed payload, with strings as
//! LEB128 length + UTF-8 bytes. No framing, no self-description, no length prefix per command
//! — the decoder knows each opcode's shape. The buffer never leaves the machine (it crosses a
//! memory boundary, not a network), so compactness matters more than extensibility.

use alloc::vec::Vec;

use crate::decode::Cmd;
use crate::geom::Layout;
use crate::opcode::OpCode;

/// Appends commands to a reusable byte buffer.
///
/// Guests keep one of these for the lifetime of the app and [`clear`](Encoder::clear) it each
/// frame, so steady-state frames perform no allocation.
#[derive(Default)]
pub struct Encoder {
    buf: Vec<u8>,
}

impl Encoder {
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    pub fn with_capacity(cap: usize) -> Self {
        Self {
            buf: Vec::with_capacity(cap),
        }
    }

    /// Reset for a new frame, keeping the allocation.
    pub fn clear(&mut self) {
        self.buf.clear();
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.buf
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }

    fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    fn u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    fn f32(&mut self, v: f32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    fn varint(&mut self, mut v: u64) {
        loop {
            let byte = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                self.buf.push(byte);
                return;
            }
            self.buf.push(byte | 0x80);
        }
    }

    fn str(&mut self, s: &str) {
        self.varint(s.len() as u64);
        self.buf.extend_from_slice(s.as_bytes());
    }

    fn layout(&mut self, l: Layout) {
        self.u8(l.kind as u8);
        self.u8(l.cross_align as u8);
    }

    pub fn push(&mut self, cmd: &Cmd<'_>) {
        match *cmd {
            Cmd::Nop => self.u8(OpCode::Nop as u8),
            Cmd::Label { id, text } => {
                self.u8(OpCode::Label as u8);
                self.u64(id);
                self.str(text);
            }
            Cmd::Button { id, text } => {
                self.u8(OpCode::Button as u8);
                self.u64(id);
                self.str(text);
            }
            Cmd::Separator => self.u8(OpCode::Separator as u8),
            Cmd::BeginScope { id, layout } => {
                self.u8(OpCode::BeginScope as u8);
                self.u64(id);
                self.layout(layout);
            }
            Cmd::EndScope { id } => {
                self.u8(OpCode::EndScope as u8);
                self.u64(id);
            }
            Cmd::BeginWindow { id, title, flags } => {
                self.u8(OpCode::BeginWindow as u8);
                self.u64(id);
                self.str(title);
                self.u32(flags);
            }
            Cmd::EndWindow { id } => {
                self.u8(OpCode::EndWindow as u8);
                self.u64(id);
            }
            Cmd::TextEditSingle { id, version, set } => {
                self.u8(OpCode::TextEditSingle as u8);
                self.u64(id);
                self.u32(version);
                // `None` means "you already hold the buffer, shell" — the common case, and the
                // reason a text field costs 13 bytes per frame instead of the whole document.
                match set {
                    None => self.u8(0),
                    Some(s) => {
                        self.u8(1);
                        self.str(s);
                    }
                }
            }
            Cmd::Image { id, src, size } => {
                self.u8(OpCode::Image as u8);
                self.u64(id);
                self.str(src);
                self.f32(size.x);
                self.f32(size.y);
            }
            Cmd::Tooltip { id, text } => {
                self.u8(OpCode::Tooltip as u8);
                self.u64(id);
                self.str(text);
            }
        }
    }
}
