//! Command stream opcodes.
//!
//! Numeric values are part of the ABI: a cached app module compiled against version N must
//! still decode correctly. Never renumber existing ones. New opcodes get a randomly
//! generated value outside the reserved ranges (see "Wire IDs" in CONTRIBUTING.md).

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
    /// A button that has the shell start another app, handing it `arg` (for the Viewer, a file
    /// path) as its launch argument. On the desktop that is a new window; on an app's own page,
    /// a new tab at that app's own page.
    OpenApp = 0x13,
    /// A button that copies a link to this server to the clipboard: `path` (such as
    /// `/app/viewer?open=...`) after the address the page was loaded from, which only the shell
    /// knows. The shell says so once it has.
    CopyLink = 0x14,
    /// Audio, video or a PDF, played or shown by the *browser* in an element the shell lays over
    /// the canvas: egui can draw neither, and the browser already has every decoder it ships.
    Media = 0x15,
    /// Read-only multi-line text, selectable and copyable. Same payload and protocol as
    /// `TextEditMulti`, so a long file costs its bytes once, not every frame.
    TextView = 0x16,
    /// Like `UploadFolder`, but the picker chooses one or more files rather than a folder. Same
    /// payload, and the same finished-count response.
    UploadFiles = 0xB0,
    /// The size the app would like its window's content area to be, in points, such as a
    /// picture's own size so its window takes the picture's shape. The shell sizes the window
    /// once, the first frame it's asked, within limits of its own (the desktop's size, a
    /// smallest window), and never again: after that the person sizes it. Ignored where there
    /// is no window to size, such as an app's own page.
    WindowSize = 0xB3,
    /// What the app's window is called while the app says so, such as the name of the file it
    /// shows; its own name otherwise. Ignored where there is no window, such as an app's own
    /// page.
    WindowTitle = 0xCB,
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
            0x13 => Some(Self::OpenApp),
            0x14 => Some(Self::CopyLink),
            0x15 => Some(Self::Media),
            0x16 => Some(Self::TextView),
            0xB0 => Some(Self::UploadFiles),
            0xB3 => Some(Self::WindowSize),
            0xCB => Some(Self::WindowTitle),
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
