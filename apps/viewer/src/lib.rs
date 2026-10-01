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
    EntryKind, ListDir, ListDirReq, MAX_TEXT_BYTES, PathReq, ReadFile, Search, SearchReq,
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

/// Formats Apple devices make that most browsers other than Safari can't show yet. The page
/// says so, rather than leaving someone wondering why a photo is blank.
pub fn apple_only(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
    matches!(
        ext.as_deref(),
        Some("heic" | "heif" | "mov" | "caf" | "aif" | "aiff" | "tif" | "tiff")
    )
}

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
    /// The file on screen, or `None` for the search.
    pub file: Option<String>,
    /// Whether `file` was picked from the search, so "Back" goes there.
    pub from_search: bool,
    pub query: Text,
    /// The text file's contents, set once when it arrives.
    text: Text,
    /// Which file `text` holds, so switching files reloads it.
    text_for: Option<String>,
    /// A table file's rows, parsed once when it arrives.
    rows: Vec<Vec<String>>,
    /// Whether a table file is shown as its text instead.
    pub as_text: bool,
}

impl Default for Viewer {
    fn default() -> Self {
        Self {
            file: None,
            from_search: false,
            query: Text::new(""),
            text: Text::new(""),
            text_for: None,
            rows: Vec::new(),
            as_text: false,
        }
    }
}

/// What the user asked for this frame, acted on after drawing so the drawing can borrow `self`.
#[derive(Default)]
struct Actions {
    open: Option<String>,
    back: bool,
    search: bool,
}

impl App for Viewer {
    fn open(&mut self, arg: &str) {
        if arg.is_empty() {
            return;
        }
        let path = if arg.starts_with('/') {
            arg.to_owned()
        } else {
            format!("/{arg}")
        };
        self.file = Some(path);
        self.from_search = false;
    }

    fn update(&mut self, ui: &mut Ui<'_>) {
        let mut act = Actions::default();
        match self.file.clone() {
            Some(path) => self.file_view(ui, &path, &mut act),
            None => self.search_view(ui, &mut act),
        }
        if let Some(path) = act.open {
            self.file = Some(path);
            self.from_search = true;
        }
        if act.back || act.search {
            self.file = None;
            if act.search {
                self.query.set("");
            }
        }
    }
}

impl Viewer {
    fn search_view(&mut self, ui: &mut Ui<'_>, act: &mut Actions) {
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
            return;
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
                                    act.open = Some(hit.path.clone());
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
    }

    fn file_view(&mut self, ui: &mut Ui<'_>, path: &str, act: &mut Actions) {
        let (dir, name) = split(path);
        ui.horizontal(|ui| {
            if self.from_search {
                if ui.button(&format!("{}  Back", icons::ARROW_LEFT)).clicked() {
                    act.back = true;
                }
                ui.tooltip("Back to the search results");
            } else {
                if ui
                    .button(&format!("{}  Search", icons::MAGNIFYING_GLASS))
                    .clicked()
                {
                    act.search = true;
                }
                ui.tooltip("Find another file to open");
            }
            ui.styled(name, TextStyle::STRONG);
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

        match kind_of(path) {
            Kind::Image => {
                ui.image(&url::inline_file_url(path), Vec2::new(0.0, 0.0));
            }
            Kind::Video => {
                ui.media(
                    &url::inline_file_url(path),
                    MediaKind::Video,
                    Vec2::new(0.0, 0.0),
                );
            }
            Kind::Audio => {
                ui.media(
                    &url::inline_file_url(path),
                    MediaKind::Audio,
                    Vec2::new(0.0, 0.0),
                );
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
        if apple_only(path) {
            ui.styled(
                "This is an Apple format. Safari shows it; other browsers may not yet, so \
                 download it if nothing appears.",
                TextStyle::WEAK,
            );
        }
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
