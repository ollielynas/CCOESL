//! Host-side command decoder.
//!
//! **This decodes untrusted input.** A guest can be buggy, mid-upgrade, or hostile. Every read
//! is bounds-checked, every enum is validated, and every string is checked for UTF-8. Nothing
//! here panics on bad input — it returns [`DecodeError`] and the shell re-renders the previous
//! frame instead.

use crate::MAX_SCOPE_DEPTH;
use crate::geom::{Align, Layout, ScopeKind, Vec2};
use crate::opcode::OpCode;

/// One decoded command, borrowing its strings from the command buffer.
///
/// The buffer is a host-owned copy of guest memory (never a live view into it), so these
/// borrows are sound for the duration of replay.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Cmd<'a> {
    Nop,
    Label {
        id: u64,
        text: &'a str,
    },
    Button {
        id: u64,
        text: &'a str,
    },
    Separator,
    BeginScope {
        id: u64,
        layout: Layout,
    },
    EndScope {
        id: u64,
    },
    BeginWindow {
        id: u64,
        title: &'a str,
        flags: u32,
    },
    EndWindow {
        id: u64,
    },
    TextEditSingle {
        id: u64,
        version: u32,
        /// `None` = the shell already holds the authoritative buffer. `Some` = the guest is
        /// overwriting it, valid only when `version` exceeds the shell's stored version.
        set: Option<&'a str>,
    },
    Image {
        id: u64,
        /// A URL the *shell* fetches, so it hits the browser HTTP cache and never occupies
        /// guest linear memory. Guests must never push pixel data through the ABI.
        src: &'a str,
        size: Vec2,
    },
    Tooltip {
        id: u64,
        text: &'a str,
    },
    /// Ask the shell to open a folder picker (or accept drag-and-drop). The shell drives the
    /// actual upload; this crate only carries the request and reports the click.
    UploadFolder {
        id: u64,
    },
    /// Ask the shell to open `url` in a new browser tab. `label` is the button text — kept
    /// separate from `url` so a listing of many rows (e.g. files) does not have to show the
    /// URL itself next to every one.
    OpenUrl {
        id: u64,
        label: &'a str,
        url: &'a str,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// Buffer ended in the middle of a command.
    Truncated,
    UnknownOpcode(u8),
    InvalidEnum,
    InvalidUtf8,
    /// A varint ran past 10 bytes or overflowed `u64`.
    BadVarint,
    /// More than `MAX_SCOPE_DEPTH` nested scopes.
    TooDeep,
    /// `EndScope`/`EndWindow` with no matching open, or closing the wrong kind.
    UnbalancedScope,
    /// An `EndScope` id did not match the `BeginScope` that opened it. Catches a guest whose
    /// own scope bookkeeping has drifted, before it corrupts the host's `Ui` stack.
    ScopeIdMismatch,
    /// Buffer ended with scopes still open.
    UnclosedScope,
}

/// A cursor over a command buffer, yielding [`Cmd`]s.
pub struct Decoder<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Decoder<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    pub fn is_done(&self) -> bool {
        self.pos >= self.buf.len()
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        let end = self.pos.checked_add(n).ok_or(DecodeError::Truncated)?;
        let slice = self.buf.get(self.pos..end).ok_or(DecodeError::Truncated)?;
        self.pos = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, DecodeError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn u64(&mut self) -> Result<u64, DecodeError> {
        let b = self.take(8)?;
        Ok(u64::from_le_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }

    fn f32(&mut self) -> Result<f32, DecodeError> {
        let b = self.take(4)?;
        Ok(f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn varint(&mut self) -> Result<u64, DecodeError> {
        let mut result: u64 = 0;
        let mut shift = 0u32;
        loop {
            if shift >= 64 {
                return Err(DecodeError::BadVarint);
            }
            let byte = self.u8()?;
            result |= u64::from(byte & 0x7f)
                .checked_shl(shift)
                .ok_or(DecodeError::BadVarint)?;
            if byte & 0x80 == 0 {
                return Ok(result);
            }
            shift += 7;
        }
    }

    fn str(&mut self) -> Result<&'a str, DecodeError> {
        let len = self.varint()?;
        let len = usize::try_from(len).map_err(|_| DecodeError::Truncated)?;
        let bytes = self.take(len)?;
        core::str::from_utf8(bytes).map_err(|_| DecodeError::InvalidUtf8)
    }

    fn layout(&mut self) -> Result<Layout, DecodeError> {
        let kind = ScopeKind::from_u8(self.u8()?).ok_or(DecodeError::InvalidEnum)?;
        let cross_align = Align::from_u8(self.u8()?).ok_or(DecodeError::InvalidEnum)?;
        Ok(Layout { kind, cross_align })
    }

    fn next_cmd(&mut self) -> Result<Cmd<'a>, DecodeError> {
        let raw = self.u8()?;
        let op = OpCode::from_u8(raw).ok_or(DecodeError::UnknownOpcode(raw))?;
        Ok(match op {
            OpCode::Nop => Cmd::Nop,
            OpCode::Label => Cmd::Label {
                id: self.u64()?,
                text: self.str()?,
            },
            OpCode::Button => Cmd::Button {
                id: self.u64()?,
                text: self.str()?,
            },
            OpCode::Separator => Cmd::Separator,
            OpCode::BeginScope => Cmd::BeginScope {
                id: self.u64()?,
                layout: self.layout()?,
            },
            OpCode::EndScope => Cmd::EndScope { id: self.u64()? },
            OpCode::BeginWindow => Cmd::BeginWindow {
                id: self.u64()?,
                title: self.str()?,
                flags: self.u32()?,
            },
            OpCode::EndWindow => Cmd::EndWindow { id: self.u64()? },
            OpCode::TextEditSingle => {
                let id = self.u64()?;
                let version = self.u32()?;
                let set = match self.u8()? {
                    0 => None,
                    1 => Some(self.str()?),
                    _ => return Err(DecodeError::InvalidEnum),
                };
                Cmd::TextEditSingle { id, version, set }
            }
            OpCode::Image => {
                let id = self.u64()?;
                let src = self.str()?;
                let size = Vec2::new(self.f32()?, self.f32()?);
                Cmd::Image { id, src, size }
            }
            OpCode::Tooltip => Cmd::Tooltip {
                id: self.u64()?,
                text: self.str()?,
            },
            OpCode::UploadFolder => Cmd::UploadFolder { id: self.u64()? },
            OpCode::OpenUrl => Cmd::OpenUrl {
                id: self.u64()?,
                label: self.str()?,
                url: self.str()?,
            },
        })
    }
}

impl<'a> Iterator for Decoder<'a> {
    type Item = Result<Cmd<'a>, DecodeError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.is_done() {
            return None;
        }
        match self.next_cmd() {
            Ok(cmd) => Some(Ok(cmd)),
            Err(e) => {
                // Stop the iteration; a malformed buffer has no recoverable tail.
                self.pos = self.buf.len();
                Some(Err(e))
            }
        }
    }
}

/// Check a whole buffer before replaying any of it.
///
/// Replay mutates the host's `Ui` stack, so a buffer that turns out to be unbalanced halfway
/// through would leave the shell in a broken state. Validating up front means a bad frame is
/// discarded whole and the previous frame is re-rendered instead.
pub fn validate(buf: &[u8]) -> Result<(), DecodeError> {
    // Stack of (opening opcode, id) — small and fixed, so no allocation.
    let mut stack: [(OpCode, u64); MAX_SCOPE_DEPTH as usize] =
        [(OpCode::Nop, 0); MAX_SCOPE_DEPTH as usize];
    let mut depth: usize = 0;

    for cmd in Decoder::new(buf) {
        let cmd = cmd?;
        match cmd {
            Cmd::BeginScope { id, .. } => {
                if depth >= MAX_SCOPE_DEPTH as usize {
                    return Err(DecodeError::TooDeep);
                }
                stack[depth] = (OpCode::BeginScope, id);
                depth += 1;
            }
            Cmd::BeginWindow { id, .. } => {
                if depth >= MAX_SCOPE_DEPTH as usize {
                    return Err(DecodeError::TooDeep);
                }
                stack[depth] = (OpCode::BeginWindow, id);
                depth += 1;
            }
            Cmd::EndScope { id } => {
                depth = depth.checked_sub(1).ok_or(DecodeError::UnbalancedScope)?;
                let (op, open_id) = stack[depth];
                if op != OpCode::BeginScope {
                    return Err(DecodeError::UnbalancedScope);
                }
                if open_id != id {
                    return Err(DecodeError::ScopeIdMismatch);
                }
            }
            Cmd::EndWindow { id } => {
                depth = depth.checked_sub(1).ok_or(DecodeError::UnbalancedScope)?;
                let (op, open_id) = stack[depth];
                if op != OpCode::BeginWindow {
                    return Err(DecodeError::UnbalancedScope);
                }
                if open_id != id {
                    return Err(DecodeError::ScopeIdMismatch);
                }
            }
            _ => {}
        }
    }

    if depth != 0 {
        return Err(DecodeError::UnclosedScope);
    }
    Ok(())
}
