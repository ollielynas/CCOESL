//! The Viewer app: shows nearly any file, read-only.
//!
//! Pictures, audio, video and PDFs are shown by the *browser*: the shell decodes pictures with
//! the browser's own decoders and lays the browser's players over the window for the rest, so
//! this module links no decoder at all. Everything else is shown as text if it is text.
//!
//! It opens on a file when another app asks it to ("Open with Viewer" in Files) or someone
//! follows a shared link (`/app/viewer?open=<path>`). Opened from the app menu, it is a search
//! for a file to open.

use ccosel_proto::fs::{
    EntryKind, ImageInfo, ListDir, ListDirReq, MAX_TEXT_BYTES, PathReq, ReadFile, Search,
    SearchReq, WebCopy,
};
use ccosel_sdk::{App, MediaKind, Poll, Text, TextStyle, Ui, Vec2, icons, url};

/// The id this app has in the shell's catalog, which links to its own page use.
pub const APP_ID: &str = "viewer";

/// The fewest characters a search is run for: one letter matches nearly everything.
pub const MIN_QUERY: usize = 2;

/// How a file is shown, decided by its name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Image,
    Video,
    Audio,
    Pdf,
    /// Spreadsheet data (`.csv`, `.tsv`): text, shown as a table.
    Table,
    /// Anything else: shown if it turns out to be text.
    Other,
}

/// What kind of file `path` is, by its extension, whatever its case. These are the kinds the
/// server will send with their own type (`?inline=1`); anything else it only offers as a
/// download, so it can only be shown here as text.
pub fn kind_of(path: &str) -> Kind {
    let name = path.rsplit('/').next().unwrap_or(path);
    let Some((_, ext)) = name.rsplit_once('.') else {
        return Kind::Other;
    };
    match ext.to_ascii_lowercase().as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "avif" | "bmp" | "ico" | "heic" | "heif"
        | "tif" | "tiff" => Kind::Image,
        "mp4" | "m4v" | "webm" | "mov" | "ogv" | "mkv" => Kind::Video,
        "mp3" | "m4a" | "aac" | "wav" | "ogg" | "oga" | "opus" | "flac" | "caf" | "aif"
        | "aiff" => Kind::Audio,
        "pdf" => Kind::Pdf,
        "csv" | "tsv" => Kind::Table,
        _ => Kind::Other,
    }
}

/// Formats Apple devices make (and TIFF) that most browsers other than Safari can't show. The
/// server makes a copy of these that every browser can, and this shows that instead. An `.m4a`
/// may be Apple Lossless or plain AAC; the server tells which and sends AAC as it is.
pub fn needs_web_copy(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
    matches!(
        ext.as_deref(),
        Some("heic" | "heif" | "mov" | "m4a" | "caf" | "aif" | "aiff" | "tif" | "tiff")
    )
}

/// Where the page fetches `path` to show it: the file itself, or for a format only some
/// browsers show, the server's copy that all of them can.
pub fn shown_url(path: &str) -> String {
    let mut url = url::inline_file_url(path);
    if needs_web_copy(path) {
        url.push_str("&as=web");
    }
    url
}

/// How often a video being converted is asked about.
pub const POLL_MS: u32 = 500;

/// The most rows of a table drawn: each cell is a widget every frame, and nobody reads ten
/// thousand rows in a window. The rest are there in "Show as text", and in the download.
pub const MAX_ROWS: usize = 500;

/// The most characters of one cell drawn, so a cell holding a paragraph doesn't make its column
/// as wide as the paragraph.
pub const MAX_CELL: usize = 120;

/// What separates a table file's cells: a tab in `.tsv`, a comma otherwise.
pub fn separator(path: &str) -> char {
    if path.to_ascii_lowercase().ends_with(".tsv") {
        '\t'
    } else {
        ','
    }
}

/// The rows of a CSV or TSV file, each a list of cells. A cell in double quotes may hold the
/// separator, line breaks, and `""` for a quote, as spreadsheets write them. A blank last line
/// isn't a row.
pub fn parse_table(text: &str, sep: char) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted => {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    cell.push('"');
                } else {
                    quoted = false;
                }
            }
            '"' if cell.is_empty() => quoted = true,
            c if quoted => cell.push(c),
            c if c == sep => row.push(core::mem::take(&mut cell)),
            '\r' if chars.peek() == Some(&'\n') => {}
            '\n' => {
                row.push(core::mem::take(&mut cell));
                rows.push(core::mem::take(&mut row));
            }
            c => cell.push(c),
        }
    }
    if !cell.is_empty() || !row.is_empty() {
        row.push(cell);
        rows.push(row);
    }
    rows
}

/// `cell` cut to [`MAX_CELL`] characters, with an ellipsis when it was longer.
pub fn clip(cell: &str) -> String {
    match cell.char_indices().nth(MAX_CELL) {
        Some((at, _)) => format!("{}…", &cell[..at]),
        None => cell.to_owned(),
    }
}

/// The folder a path is in, and its name: `/a/b.txt` is `("/a", "b.txt")`.
pub fn split(path: &str) -> (&str, &str) {
    match path.rfind('/') {
        Some(0) => ("/", &path[1..]),
        Some(i) => (&path[..i], &path[i + 1..]),
        None => ("/", path),
    }
}

/// A size for people, with no float formatting: that would pull 20–40 KB of float-to-string
/// code into a module everyone downloads.
pub fn human_size(bytes: u64) -> String {
    let (n, unit) = if bytes >= 1 << 30 {
        (bytes >> 30, "GB")
    } else if bytes >= 1 << 20 {
        (bytes >> 20, "MB")
    } else if bytes >= 1 << 10 {
        (bytes >> 10, "KB")
    } else {
        (bytes, "bytes")
    };
    format!("{n} {unit}")
}

pub struct Viewer {
    /// The file on screen, or `None` for the search. Once set it never changes: a window is
    /// the file it was opened for, and another file opens in another window.
    pub file: Option<String>,
    pub query: Text,
    /// The text file's contents, set once when it arrives.
    text: Text,
    /// Which file `text` holds, so switching files reloads it.
    text_for: Option<String>,
    /// A table file's rows, parsed once when it arrives.
    rows: Vec<Vec<String>>,
    /// Whether a table file is shown as its text instead.
    pub as_text: bool,
    /// Whether a picture's details (size, camera, when and where it was taken) are shown.
    pub details: bool,
    /// Whether a video's web copy is being made, so the app polls until it is.
    pub converting: bool,
}

impl Default for Viewer {
    fn default() -> Self {
        Self {
            file: None,
            query: Text::new(""),
            text: Text::new(""),
            text_for: None,
            rows: Vec::new(),
            as_text: false,
            converting: false,
            details: false,
        }
    }
}

impl App for Viewer {
    fn wants_repaint_after_ms(&self) -> u32 {
        if self.converting {
            POLL_MS
        } else {
            ccosel_sdk::REPAINT_ON_INPUT_ONLY
        }
    }

    fn open(&mut self, arg: &str) {
        if arg.is_empty() || self.file.is_some() {
            return;
        }
        let path = if arg.starts_with('/') {
            arg.to_owned()
        } else {
            format!("/{arg}")
        };
        self.file = Some(path);
    }

    fn update(&mut self, ui: &mut Ui<'_>) {
        match self.file.clone() {
            Some(path) => self.file_view(ui, &path),
            None => {
                // Picked after drawing, so the drawing can borrow `self`.
                if let Some(path) = self.search_view(ui) {
                    self.file = Some(path);
                }
            }
        }
    }
}

impl Viewer {
    /// The search a window opened on no file shows. Returns the file picked from it, if any.
    fn search_view(&mut self, ui: &mut Ui<'_>) -> Option<String> {
        let mut picked = None;
        ui.styled("Find a file", TextStyle::heading(2));
        ui.horizontal(|ui| {
            ui.label(icons::MAGNIFYING_GLASS);
            ui.text_edit(&mut self.query);
        });
        ui.separator();
        let query = self.query.as_str().trim();
        if query.chars().count() < MIN_QUERY {
            ui.styled(
                "Type a name, or some words from inside a file, to find it anywhere you can see.",
                TextStyle::WEAK,
            );
            return None;
        }
        let req = SearchReq {
            path: "/",
            query,
            suffix: "",
        };
        match ui.rpc().get::<Search>(&req) {
            Poll::Pending => {
                ui.label("Searching…");
            }
            Poll::Failed(e) => {
                ui.label(e.message());
            }
            Poll::Ready(found) => {
                if found.hits.is_empty() {
                    ui.styled("Nothing matches.", TextStyle::WEAK | TextStyle::ITALIC);
                }
                ui.scroll(|ui| {
                    for hit in &found.hits {
                        let (dir, name) = split(&hit.path);
                        ui.push_id(&hit.path, |ui| {
                            ui.group(|ui| {
                                if ui.selectable(false, name).clicked() {
                                    picked = Some(hit.path.clone());
                                }
                                ui.tooltip(&hit.path);
                                ui.styled(dir, TextStyle::WEAK);
                                if !hit.line.is_empty() {
                                    ui.styled(&hit.line, TextStyle::ITALIC);
                                }
                            });
                        });
                    }
                    if found.truncated {
                        ui.styled(
                            "More files match than are shown: try a longer search.",
                            TextStyle::WEAK,
                        );
                    }
                });
            }
        }
        picked
    }

    fn file_view(&mut self, ui: &mut Ui<'_>, path: &str) {
        let (dir, name) = split(path);
        ui.horizontal(|ui| {
            ui.styled(name, TextStyle::STRONG);
            if kind_of(path) == Kind::Image {
                let label = if self.details {
                    format!("{}  Hide details", icons::INFO)
                } else {
                    format!("{}  Details", icons::INFO)
                };
                if ui.button(&label).clicked() {
                    self.details = !self.details;
                }
                ui.tooltip("Its size, and what the camera recorded: when, where and how");
            }
            ui.open_url(
                &format!("{}  Download", icons::DOWNLOAD_SIMPLE),
                &url::file_url(path),
            );
            ui.copy_link(
                &format!("{}  Share", icons::LINK),
                &url::app_link(APP_ID, path),
            );
            ui.tooltip(
                "Copy a link that opens this file here. Whoever follows it signs in first, \
                 and sees it only if they may.",
            );
        });
        ui.styled(dir, TextStyle::WEAK);
        ui.separator();

        // The folder's listing says whether the file is there and readable, and how big it is,
        // without reading it.
        let listing = ui.rpc().get::<ListDir>(&ListDirReq { path: dir });
        let entry = match &listing {
            Poll::Ready(l) => l.entries.iter().find(|e| e.name == name),
            _ => None,
        };
        match (&listing, entry) {
            (Poll::Pending, _) => {
                ui.label("Opening…");
                return;
            }
            (Poll::Failed(e), _) => {
                ui.label(e.message());
                return;
            }
            (Poll::Ready(_), Some(e)) if e.kind == EntryKind::Dir => {
                ui.label("That is a folder. Open it in Files to see what is in it.");
                return;
            }
            // A listing too long to hold everything may leave it out: try it anyway.
            (Poll::Ready(l), None) if !l.truncated => {
                ui.label("This file isn't there any more, or you aren't allowed to see it.");
                return;
            }
            _ => {}
        }
        let size = entry.map(|e| e.size);

        self.converting = false;
        match kind_of(path) {
            Kind::Image => {
                if self.details {
                    image_details(ui, path, size);
                    ui.separator();
                }
                ui.image(&shown_url(path), Vec2::new(0.0, 0.0));
            }
            Kind::Video => {
                if needs_web_copy(path) && !self.web_copy_ready(ui, path) {
                    return;
                }
                ui.media(&shown_url(path), MediaKind::Video, Vec2::new(0.0, 0.0));
            }
            Kind::Audio => {
                ui.media(&shown_url(path), MediaKind::Audio, Vec2::new(0.0, 0.0));
            }
            Kind::Pdf => {
                ui.media(
                    &url::inline_file_url(path),
                    MediaKind::Document,
                    Vec2::new(0.0, 0.0),
                );
            }
            Kind::Table => self.text_view(ui, path, size, Some(separator(path))),
            Kind::Other => self.text_view(ui, path, size, None),
        }
    }

    /// Whether the server's copy of the video `path` is ready to play. Until it is, says how
    /// far along it is, and keeps asking: a long video can take minutes to convert.
    fn web_copy_ready(&mut self, ui: &mut Ui<'_>, path: &str) -> bool {
        let req = PathReq { path };
        match ui.rpc().get::<WebCopy>(&req) {
            Poll::Ready(status) if status.finished => match &status.error {
                None => return true,
                Some(why) => {
                    ui.label(&format!(
                        "This video couldn't be converted to play here: {why}. Download it to \
                         watch it."
                    ));
                }
            },
            Poll::Ready(status) => {
                let mut text = String::from("Converting this video so it plays in this browser…");
                if let Some(p) = status.permille {
                    text.push(' ');
                    text.push_str(&(p / 10).to_string());
                    text.push('%');
                }
                ui.label(&text);
                ui.styled(
                    "Only the first time: once converted, it plays straight away.",
                    TextStyle::WEAK,
                );
                self.converting = true;
                // Ask again next time round; this reads the job's progress, never starts another.
                ui.rpc().invalidate::<WebCopy>(&req);
            }
            Poll::Pending => {
                ui.label("Converting this video so it plays in this browser…");
                self.converting = true;
            }
            Poll::Failed(e) => {
                ui.label(e.message());
            }
        }
        false
    }

    /// A text file, or with `table`, a table file split on that separator.
    fn text_view(&mut self, ui: &mut Ui<'_>, path: &str, size: Option<u64>, table: Option<char>) {
        if let Some(size) = size
            && size > MAX_TEXT_BYTES as u64
        {
            ui.label(&format!(
                "This file is too big to show here ({}). Download it to open it.",
                human_size(size)
            ));
            return;
        }
        match ui.rpc().get::<ReadFile>(&PathReq { path }) {
            Poll::Pending => {
                ui.label("Loading…");
            }
            // It is small enough, so it isn't text: there is nothing here to show it with.
            Poll::Failed(_) => {
                ui.label("This kind of file can't be shown here. Download it to open it.");
            }
            Poll::Ready(file) => {
                if self.text_for.as_deref() != Some(path) {
                    self.text.set(&file.text);
                    self.text_for = Some(path.to_owned());
                    self.rows = table.map_or_else(Vec::new, |sep| parse_table(&file.text, sep));
                    self.as_text = false;
                }
                if file.text.is_empty() {
                    ui.styled("This file is empty.", TextStyle::WEAK | TextStyle::ITALIC);
                } else if table.is_some() && !self.as_text {
                    self.table(ui);
                } else {
                    if table.is_some() && ui.button("Show as a table").clicked() {
                        self.as_text = false;
                    }
                    ui.scroll(|ui| {
                        ui.text_view(&mut self.text);
                    });
                }
            }
        }
    }
}

/// A picture's size and what its camera recorded, read by the server (the app never holds the
/// picture itself), as a two-column table. Asked for only once someone opens it.
fn image_details(ui: &mut Ui<'_>, path: &str, size: Option<u64>) {
    match ui.rpc().get::<ImageInfo>(&PathReq { path }) {
        Poll::Pending => {
            ui.label("Reading the details…");
        }
        Poll::Failed(e) => {
            ui.label(e.message());
        }
        Poll::Ready(info) => {
            let mut rows: Vec<(String, String)> = Vec::new();
            if let (Some(w), Some(h)) = (info.width, info.height) {
                rows.push(("Dimensions".to_owned(), format!("{w} × {h} pixels")));
            }
            if let Some(size) = size {
                rows.push(("File size".to_owned(), human_size(size)));
            }
            rows.extend(info.fields.iter().cloned());
            ui.table(|ui| {
                for (label, value) in &rows {
                    ui.row(|ui| {
                        ui.styled(label, TextStyle::STRONG);
                        ui.label(value);
                    });
                }
            });
            if info.fields.is_empty() {
                ui.styled(
                    "No camera details in this picture.",
                    TextStyle::WEAK | TextStyle::ITALIC,
                );
            }
        }
    }
}

impl Viewer {
    fn table(&mut self, ui: &mut Ui<'_>) {
        let total = self.rows.len();
        ui.horizontal(|ui| {
            if ui.button("Show as text").clicked() {
                self.as_text = true;
            }
            ui.tooltip("See the file exactly as it is written");
            // The first row is the column names, as spreadsheets save it.
            let records = total.saturating_sub(1);
            if records > MAX_ROWS {
                ui.styled(
                    &format!("The first {MAX_ROWS} of {records} rows"),
                    TextStyle::WEAK,
                );
            } else {
                ui.styled(
                    &format!("{records} row{}", if records == 1 { "" } else { "s" }),
                    TextStyle::WEAK,
                );
            }
        });
        let rows = &self.rows;
        ui.scroll(|ui| {
            ui.table(|ui| {
                for (i, record) in rows.iter().take(MAX_ROWS + 1).enumerate() {
                    ui.row(|ui| {
                        for cell in record {
                            if i == 0 {
                                ui.styled(&clip(cell), TextStyle::STRONG);
                            } else {
                                ui.label(&clip(cell));
                            }
                        }
                    });
                }
            });
        });
    }
}

#[cfg(test)]
mod tests;

ccosel_sdk::ccosel_app!(Viewer);
