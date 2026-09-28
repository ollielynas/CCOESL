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
    // 0x0B is reserved for ProgressBar (#21).
    /// Ask the shell to open a folder picker (or accept drag-and-drop). The shell handles the
    /// actual file selection and upload; acting on the click is out of scope here (see #19).
    UploadFolder = 0x0C,
    /// Ask the shell to open `url` in a new browser tab, e.g. to stream a file out of the jail
    /// via `/files/{path}`. `label` is what the button shows — never the URL itself, which
    /// would be unreadable repeated next to every row of a file listing.
    OpenUrl = 0x0D,
    /// A line graph of `samples`, each already scaled by the guest to `0..=255`. The shell draws
    /// it, so a live chart costs one byte per point on the wire and no guest-side rendering.
    Plot = 0x0E,
    /// A button that has the shell upload a project folder from this computer into a new
    /// temporary server folder, leaving out what its `.gitignore`s exclude. The response's
    /// `aux` is that folder's id once the upload finishes.
    UploadProject = 0x0F,
    /// A multi-line text area. Same payload and the same delta protocol as `TextEditSingle`;
    /// only the widget the shell draws differs.
    TextEditMulti = 0x10,
    /// A label with typographic style (heading level, bold, italic, code, link, ...), so an app
    /// can render formatted text such as Markdown without the shell knowing any markup.
    Styled = 0x11,
    /// A clickable row that shows whether it is the selected one, like an entry in a file tree.
    Selectable = 0x12,
    /// A 3D view of a mesh the shell fetches by URL. The shell owns the camera, hover,
    /// snapping and drag preview; the guest gets finished gestures back as `VIEWPORT` events.
    /// See `view3d`.
    Viewport3d = 0x13,
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
            0x0C => Some(Self::UploadFolder),
            0x0D => Some(Self::OpenUrl),
            0x0E => Some(Self::Plot),
            0x0F => Some(Self::UploadProject),
            0x10 => Some(Self::TextEditMulti),
            0x11 => Some(Self::Styled),
            0x12 => Some(Self::Selectable),
            0x13 => Some(Self::Viewport3d),
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
