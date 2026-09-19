//! Command stream opcodes.
//!
//! Numeric values are part of the ABI: a cached app module compiled against version N must
//! still decode correctly. Add new opcodes, never renumber existing ones.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum OpCode {
    Nop = 0x00,
    Label = 0x01,
    Button = 0x02,
    Separator = 0x03,
    BeginScope = 0x04,
    EndScope = 0x05,
    BeginWindow = 0x06,
    EndWindow = 0x07,
    TextEditSingle = 0x08,
    Image = 0x09,
    /// Emitted unconditionally alongside a widget. The *shell* decides hover and draws it, so
    /// there is no one-frame delay. Guests must never branch on `hovered()` in order to draw.
    Tooltip = 0x0A,
    // 0x40..0x4F reserved for subtree caching (BeginCached / EndCached / CachedRef).
    // Not implemented yet, but the space is reserved so adding it is not an ABI break.
}

impl OpCode {
    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            0x00 => Some(Self::Nop),
            0x01 => Some(Self::Label),
            0x02 => Some(Self::Button),
            0x03 => Some(Self::Separator),
            0x04 => Some(Self::BeginScope),
            0x05 => Some(Self::EndScope),
            0x06 => Some(Self::BeginWindow),
            0x07 => Some(Self::EndWindow),
            0x08 => Some(Self::TextEditSingle),
            0x09 => Some(Self::Image),
            0x0A => Some(Self::Tooltip),
            _ => None,
        }
    }

    /// Does this opcode push a scope onto the host's `Ui` stack?
    pub const fn opens_scope(self) -> bool {
        matches!(self, Self::BeginScope | Self::BeginWindow)
    }

    /// Does this opcode pop a scope? Returns the opcode that must have opened it.
    pub const fn closes(self) -> Option<Self> {
        match self {
            Self::EndScope => Some(Self::BeginScope),
            Self::EndWindow => Some(Self::BeginWindow),
            _ => None,
        }
    }
}
