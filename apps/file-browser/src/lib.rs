//! The File Browser app.
//!
//! Compiles to a standalone `.wasm` module with no `wasm-bindgen` and no egui — it records
//! commands, the shell draws them.
//!
//! Note what this app does *not* contain: no call ids, no "have I requested yet" flag, no
//! `on_event` handler, no cancellation, and no check for a stale reply arriving after the user
//! navigated away. Asking for a listing every frame is the whole of the data flow, because the
//! request cache keys on the request itself — so changing `self.path` *is* the re-request.

use ccosel_proto::fs::{Access, EntryKind, ListDir, ListDirReq, PathReq};
use ccosel_sdk::{App, Poll, Text, TextStyle, Ui, url};

/// The two file systems a person has: what is shared on the server, and their own folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    Shared,
    Mine,
}

/// The tabs' labels. Also what a test clicks. The icons are `icons::USERS`, `icons::HOUSE`,
/// `icons::PENCIL_SIMPLE` and `icons::LOCK_SIMPLE`, written out because a `const` can't be
/// built with `format!`; `the_labels_use_the_sdk_icons` checks they stay the same.
pub const SHARED_TAB: &str = "\u{E4D6}  Shared";
pub const MINE_TAB: &str = "\u{E2C2}  My files";
/// Shown in the permissions column, and above the listing for the folder itself.
pub const CAN_CHANGE: &str = "\u{E3B4}  Can change";
pub const READ_ONLY: &str = "\u{E308}  Read-only";

/// The folder a place starts at. `None` for "My files" when nobody is signed in: there is no
/// private folder to show.
pub fn place_root(place: Place, user: Option<&str>) -> Option<String> {
    match place {
        Place::Shared => Some("/".to_owned()),
        Place::Mine => user.map(|u| format!("/home/{u}")),
    }
}

pub struct FileBrowser {
    place: Place,
    path: String,
    filter: Text,
    /// Keyed by name, not by index: the listing is replaced asynchronously, so an index would
    /// silently come to mean a different file.
    selected: Option<String>,
    /// The upload button's finished-upload count as of the last frame. When the shell's count
    /// moves past it, an upload has landed and the listing is re-asked.
    uploads_seen: u32,
}

impl Default for FileBrowser {
    fn default() -> Self {
        Self {
            place: Place::Shared,
            path: "/".to_owned(),
            filter: Text::new(""),
            selected: None,
            uploads_seen: 0,
        }
    }
}

/// A single glyph per kind, drawn from the same "stand-in until the icon pipeline exists"
/// convention the shell already uses for app icons (see `registry.rs`).
fn icon_for(kind: EntryKind) -> &'static str {
    match kind {
        EntryKind::Dir => "📁",
        EntryKind::Symlink => "🔗",
        EntryKind::File | EntryKind::Other => "📄",
    }
}

/// Human-readable size without touching float formatting, which would drag 20–40 KB of
/// float-to-string machinery into a module every user downloads.
fn human_size(bytes: u64) -> String {
    let (n, unit) = if bytes >= 1 << 20 {
        (bytes / (1 << 20), "M")
    } else if bytes >= 1 << 10 {
        (bytes / (1 << 10), "K")
    } else {
        (bytes, "B")
    };
    let mut s = String::new();
    s.push_str(itoa(n).as_str());
    s.push_str(unit);
    s
}

fn itoa(mut n: u64) -> String {
    if n == 0 {
        return "0".to_owned();
    }
    let mut buf = [0u8; 20];
    let mut i = buf.len();
    while n > 0 {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    String::from_utf8_lossy(&buf[i..]).into_owned()
}

impl FileBrowser {
    /// Where the current place starts. Up and the breadcrumbs never go above it.
    fn root(&self) -> String {
        match self.place {
            Place::Shared => "/".to_owned(),
            // `path` is inside the home folder whenever `place` is `Mine`: it is the first two
            // segments of it, `/home/<user>`.
            Place::Mine => {
                let mut segs = self.path.split('/').filter(|s| !s.is_empty());
                match (segs.next(), segs.next()) {
                    (Some(home), Some(user)) => format!("/{home}/{user}"),
                    _ => "/".to_owned(),
                }
            }
        }
    }

    fn switch_to(&mut self, place: Place, root: String) {
        self.place = place;
        self.path = root;
        self.selected = None;
    }

    fn go_up(&mut self) {
        let root = self.root();
        if self.path == root {
            return;
        }
        match self.path.rfind('/') {
            Some(0) | None => self.path = "/".to_owned(),
            Some(i) => self.path.truncate(i),
        }
        if self.path.len() < root.len() {
            self.path = root;
        }
        self.selected = None;
    }

    fn enter(&mut self, name: &str) {
        if !self.path.ends_with('/') {
            self.path.push('/');
        }
        self.path.push_str(name);
        self.selected = None;
    }

    /// The path as a chain of `(label, full path)` breadcrumbs, from the top of the current
    /// place down.
    fn crumbs(&self) -> Vec<(String, String)> {
        let root = self.root();
        let mut out = vec![("🏠".to_owned(), root.clone())];
        let mut acc = root.trim_end_matches('/').to_owned();
        let below = self.path.strip_prefix(root.as_str()).unwrap_or("");
        for seg in below.split('/').filter(|s| !s.is_empty()) {
            acc.push('/');
            acc.push_str(seg);
            out.push((seg.to_owned(), acc.clone()));
        }
        out
    }
}

impl App for FileBrowser {
    fn update(&mut self, ui: &mut Ui<'_>) {
        // Who the server thinks this is, which is where "My files" lives.
        let user = match ui.rpc().get::<Access>(&PathReq { path: "/" }) {
            Poll::Ready(a) => a.user.clone(),
            _ => None,
        };
        // Signed out while looking at "My files": there is no private folder any more.
        if self.place == Place::Mine && user.is_none() {
            self.switch_to(Place::Shared, "/".to_owned());
        }
        // What this caller may do in the folder on screen. Only a folder they may change gets
        // an upload button; the server refuses an upload anywhere else anyway.
        let here = match ui.rpc().get::<Access>(&PathReq { path: &self.path }) {
            Poll::Ready(a) => Some(a.write),
            _ => None,
        };

        let mut switch: Option<Place> = None;
        ui.horizontal(|ui| {
            ui.push_id("shared", |ui| {
                if ui
                    .selectable(self.place == Place::Shared, SHARED_TAB)
                    .clicked()
                {
                    switch = Some(Place::Shared);
                }
                ui.tooltip("What is shared on this server, as far as you are allowed to see it");
            });
            ui.push_id("mine", |ui| {
                if user.is_some() {
                    if ui.selectable(self.place == Place::Mine, MINE_TAB).clicked() {
                        switch = Some(Place::Mine);
                    }
                    ui.tooltip("Your private folder. Only you can see what is in it.");
                } else {
                    ui.styled("Sign in to have files of your own", TextStyle::WEAK);
                }
            });
        });
        if let Some(place) = switch
            && let Some(root) = place_root(place, user.as_deref())
        {
            self.switch_to(place, root);
        }

        // Button presses are recorded and acted on after the closure: a request borrows
        // `self.path`, and navigation mutates it.
        let mut go_up = false;
        let mut refresh = false;
        let mut go_to: Option<String> = None;

        ui.horizontal(|ui| {
            if ui.button("⬆ Up").clicked() {
                go_up = true;
            }
            ui.tooltip("Go to parent directory");
            if ui.button("⟳ Refresh").clicked() {
                refresh = true;
            }
            // Under its own id, since it comes and goes: an upload's finished count is kept
            // against the button's id.
            ui.push_id("upload", |ui| {
                if here != Some(true) {
                    return;
                }
                // Into the folder on screen. The shell opens the picker and does the upload.
                let finished = ui.upload_folder(&self.path).uploads_finished();
                ui.tooltip("Upload a folder from this computer into this one");
                if finished != self.uploads_seen {
                    self.uploads_seen = finished;
                    refresh = true;
                }
            });
        });

        // A clickable trail, not just a path label: jumping to an ancestor is one click instead
        // of several "Up"s.
        let crumbs = self.crumbs();
        ui.horizontal(|ui| {
            let last = crumbs.len() - 1;
            for (i, (label, path)) in crumbs.iter().enumerate() {
                ui.push_id(path.as_str(), |ui| {
                    if ui.button(label.as_str()).clicked() {
                        go_to = Some(path.clone());
                    }
                });
                if i != last {
                    ui.label("›");
                }
            }
        });

        ui.push_id("here", |ui| match here {
            Some(true) => ui.label(&format!("{CAN_CHANGE}: you can add and change files here")),
            Some(false) => ui.label(&format!(
                "{READ_ONLY}: you can open and download files here, but not change them"
            )),
            None => {}
        });

        if go_up {
            self.go_up();
        }
        if let Some(path) = go_to {
            self.path = path;
            self.selected = None;
        }
        if refresh {
            ui.rpc()
                .invalidate::<ListDir>(&ListDirReq { path: &self.path });
        }

        ui.horizontal(|ui| {
            ui.label("🔍 Filter");
            ui.text_edit(&mut self.filter);
        });
        ui.separator();

        // Deferred so the borrow of `self` inside the match does not collide with mutating it.
        let mut enter: Option<String> = None;
        let mut retry = false;
        let mut total = 0usize;
        let mut shown = 0usize;

        // Bound to a local so the borrow of `self.path` ends before the arms run.
        let listing = ui.rpc().get::<ListDir>(&ListDirReq { path: &self.path });

        match listing {
            // Waiting is not animating: `wants_repaint_after_ms` stays on input-only, and the
            // arriving reply is what wakes this app.
            Poll::Pending => {
                ui.label("Loading…");
            }
            Poll::Failed(err) => {
                ui.label(err.message());
                if ui.button("Retry").clicked() {
                    retry = true;
                }
            }
            Poll::Ready(listing) => {
                // The top of "Shared" leaves out `/home`: everyone's own folder is under "My
                // files", and the server only ever lists the caller's own there anyway.
                let hidden =
                    |name: &str| self.place == Place::Shared && self.path == "/" && name == "home";
                let entries: Vec<_> = listing
                    .entries
                    .iter()
                    .filter(|e| !hidden(&e.name))
                    .collect();
                total = entries.len();
                let needle = self.filter.as_str().to_ascii_lowercase();

                if total > 0 {
                    ui.horizontal(|ui| {
                        ui.label("Name");
                        ui.label("Size");
                        ui.label("Permissions");
                    });
                    ui.separator();
                }

                for entry in entries {
                    if !needle.is_empty() && !entry.name.to_ascii_lowercase().contains(&needle) {
                        continue;
                    }
                    shown += 1;
                    // Salt by name, not position: filtering changes which rows are emitted, and
                    // position-derived ids would land a click on whichever row moved into the
                    // slot.
                    ui.push_id(&entry.name, |ui| {
                        // A `group` per row, not just a `horizontal`: it's what gives the host
                        // something to key an alternating background off, so the list reads as
                        // a zebra-striped table instead of a wall of same-colored text.
                        ui.group(|ui| {
                            ui.horizontal(|ui| {
                                ui.label(icon_for(entry.kind));

                                let selected =
                                    self.selected.as_deref() == Some(entry.name.as_str());
                                let label = if selected {
                                    format!("▸ {}", entry.name)
                                } else {
                                    entry.name.clone()
                                };
                                if ui.button(&label).clicked() {
                                    if entry.is_dir() {
                                        enter = Some(entry.name.clone());
                                    } else {
                                        self.selected = Some(entry.name.clone());
                                    }
                                }
                                if !entry.is_dir() {
                                    ui.tooltip("Right-click to open, share or download it");
                                    let path = join(&self.path, &entry.name);
                                    // Attached to the name just drawn, so right-clicking the
                                    // file is what opens it.
                                    ui.context_menu(|ui| file_menu(ui, &path));
                                }
                                if !entry.is_dir() {
                                    ui.label(format!("· {}", human_size(entry.size)).as_str());
                                }
                                // Everything listed can be read, so the one thing to say is
                                // whether it can also be changed.
                                if entry.writable {
                                    ui.label(CAN_CHANGE);
                                } else {
                                    ui.label(READ_ONLY);
                                }
                            });
                        });
                    });
                }

                if total == 0 {
                    ui.label("(empty directory)");
                } else if shown == 0 {
                    ui.label("(nothing matches the filter)");
                }
                if listing.truncated {
                    ui.label("(listing truncated)");
                }
            }
        }

        if retry {
            ui.rpc()
                .invalidate::<ListDir>(&ListDirReq { path: &self.path });
        }
        if let Some(name) = enter {
            self.enter(&name);
        }

        ui.separator();
        match &self.selected {
            // Two labels, not one interpolated string: keeps the file's own name as its own
            // widget rather than burying it inside "Selected: notes.md" prose.
            Some(name) => {
                ui.horizontal(|ui| {
                    ui.label("Selected:");
                    ui.label(name.as_str());
                });
            }
            None if total > 0 => {
                ui.label(format!("{shown} of {total} items").as_str());
            }
            None => {
                ui.label("nothing selected");
            }
        }
    }
}

/// The labels of a file's right-click menu, and what a test clicks.
pub const OPEN_WITH_VIEWER: &str = "\u{E220}  Open with Viewer";
pub const SHARE_WITH_VIEWER: &str = "\u{E2E2}  Share with Viewer";
pub const DOWNLOAD: &str = "\u{E20C}  Download";

/// What can be done with a file: open it in the Viewer, copy a link that opens it there, or
/// download it. The shell does each, so this only says which.
fn file_menu(ui: &mut Ui<'_>, path: &str) {
    ui.open_app(OPEN_WITH_VIEWER, VIEWER, path);
    ui.copy_link(SHARE_WITH_VIEWER, &url::app_link(VIEWER, path));
    ui.tooltip(
        "Copy a link that opens this file in the Viewer. Whoever follows it signs in first, and \
         sees it only if they may.",
    );
    // Opened by the shell in a new tab.
    ui.open_url(DOWNLOAD, &url::file_url(path));
}

/// The Viewer's id in the shell's catalog.
const VIEWER: &str = "viewer";

/// `dir/name`, with exactly one `/` between them.
fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

#[cfg(test)]
mod tests;

ccosel_sdk::ccosel_app!(FileBrowser);
