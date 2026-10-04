//! The File Browser app.
//!
//! Compiles to a standalone `.wasm` module with no `wasm-bindgen` and no egui — it records
//! commands, the shell draws them.
//!
//! Note what this app does *not* contain: no call ids, no "have I requested yet" flag, no
//! `on_event` handler, no cancellation, and no check for a stale reply arriving after the user
//! navigated away. Asking for a listing every frame is the whole of the data flow, because the
//! request cache keys on the request itself — so changing `self.path` *is* the re-request.

use ccosel_proto::archive::{
    Archive, ArchiveAction, ArchiveReq, ArchiveResult, ArchiveStatus, archive_kind,
};
use ccosel_proto::fs::{Access, EntryKind, ListDir, ListDirReq, PathReq, Remove};
use ccosel_sdk::{App, CallId, Poll, Text, TextStyle, Ui, url};

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
    /// The upload buttons' finished-upload counts as of the last frame, folders then files.
    /// When the shell's count moves past one, an upload has landed and the listing is
    /// re-asked.
    uploads_seen: u32,
    file_uploads_seen: u32,
    /// What Delete was chosen for and is waiting to be confirmed: its name in the folder on
    /// screen, and whether it is a folder.
    confirm: Option<(String, bool)>,
    /// The deletion on its way to the server: what it deletes, and the call.
    deleting: Option<(String, CallId)>,
    /// How the last deletion or archive job went, until the next one or until the folder
    /// changes.
    status: Option<String>,
    /// The archive job running, if any: what it is of, and what it does. Polled every frame.
    archiving: Option<(String, ArchiveAction)>,
    /// Bumped once per Compress, Gzip or Extract: the job's idempotency key.
    generation: u32,
}

impl Default for FileBrowser {
    fn default() -> Self {
        Self {
            place: Place::Shared,
            path: "/".to_owned(),
            filter: Text::new(""),
            selected: None,
            uploads_seen: 0,
            file_uploads_seen: 0,
            confirm: None,
            deleting: None,
            status: None,
            archiving: None,
            generation: 0,
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
        self.left_folder();
    }

    /// Whatever was about the folder on screen no longer applies once it changes. A deletion
    /// already sent still finishes, and still says how it went.
    fn left_folder(&mut self) {
        self.selected = None;
        self.confirm = None;
        if self.deleting.is_none() && self.archiving.is_none() {
            self.status = None;
        }
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
        self.left_folder();
    }

    fn enter(&mut self, name: &str) {
        if !self.path.ends_with('/') {
            self.path.push('/');
        }
        self.path.push_str(name);
        self.left_folder();
    }

    /// Ask the server to delete the confirmed entry.
    fn delete_confirmed(&mut self, ui: &mut Ui<'_>) {
        let Some((name, _)) = self.confirm.take() else {
            return;
        };
        let path = join(&self.path, &name);
        let call = ui.rpc().send::<Remove>(&PathReq { path: &path });
        self.status = Some(format!("Deleting {name}…"));
        self.deleting = Some((path, call));
    }

    /// Start an archive job on `name` in the folder on screen.
    fn start_archive(&mut self, name: &str, action: ArchiveAction) {
        self.generation = self.generation.wrapping_add(1);
        self.archiving = Some((join(&self.path, name), action));
        self.status = Some(format!("{} {name}…", doing(action)));
    }

    /// Check on the archive job, if any: show how far it has got, and once it is done say
    /// what it made and list its folder again.
    fn poll_archive(&mut self, ui: &mut Ui<'_>) {
        let Some((path, action)) = &self.archiving else {
            return;
        };
        let rpc = ui.rpc();
        let req = ArchiveReq {
            path,
            action: *action,
            generation: self.generation,
        };
        let name = path.rsplit('/').next().unwrap_or(path).to_owned();
        let status = match rpc.get::<Archive>(&req) {
            // A poll on the wire: keep showing the last progress.
            Poll::Pending => return,
            Poll::Ready(s) if !s.finished => {
                self.status = Some(progress(*action, &name, &s));
                // Read again next frame; the job is keyed by its generation, so this never
                // starts a second one.
                rpc.invalidate::<Archive>(&req);
                return;
            }
            Poll::Ready(s) => match &s.result {
                Some(ArchiveResult::Made(made)) => {
                    let made = made.rsplit('/').next().unwrap_or(made);
                    format!("{} {name} into {made}", done(*action))
                }
                Some(ArchiveResult::Failed(why)) => {
                    format!("Couldn't {} {name}: {why}", verb(*action))
                }
                None => return,
            },
            Poll::Failed(err) => format!("Couldn't {} {name}: {}", verb(*action), err.message()),
        };
        let dir = match path.rfind('/') {
            Some(0) | None => "/",
            Some(i) => &path[..i],
        };
        rpc.invalidate::<ListDir>(&ListDirReq { path: dir });
        self.status = Some(status);
        self.archiving = None;
    }

    /// Check on the deletion in flight, if any. Once it is done the folder it was in is
    /// listed again, so it disappears from the list (or, if it failed, stays).
    fn poll_delete(&mut self, ui: &mut Ui<'_>) {
        let Some((path, call)) = &self.deleting else {
            return;
        };
        let rpc = ui.rpc();
        let name = path.rsplit('/').next().unwrap_or(path).to_owned();
        let status = match rpc.outcome::<Remove>(*call) {
            Poll::Pending => return,
            Poll::Ready(_) => format!("Deleted {name}"),
            Poll::Failed(err) => format!("Couldn't delete {name}: {}", err.message()),
        };
        let dir = match path.rfind('/') {
            Some(0) | None => "/",
            Some(i) => &path[..i],
        };
        rpc.invalidate::<ListDir>(&ListDirReq { path: dir });
        if self.selected.as_deref() == Some(name.as_str()) {
            self.selected = None;
        }
        self.status = Some(status);
        self.deleting = None;
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

/// 4 Hz while an archive job runs, as the Compiler polls a build.
const POLL_MS: u32 = 250;

impl App for FileBrowser {
    fn wants_repaint_after_ms(&self) -> u32 {
        if self.archiving.is_some() {
            POLL_MS
        } else {
            ccosel_sdk::REPAINT_ON_INPUT_ONLY
        }
    }

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
            ui.push_id("upload-files", |ui| {
                if here != Some(true) {
                    return;
                }
                let finished = ui.upload_files(&self.path).uploads_finished();
                ui.tooltip("Upload files from this computer into this folder");
                if finished != self.file_uploads_seen {
                    self.file_uploads_seen = finished;
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
            self.left_folder();
        }
        if refresh {
            ui.rpc()
                .invalidate::<ListDir>(&ListDirReq { path: &self.path });
        }

        self.poll_delete(ui);
        self.poll_archive(ui);
        if let Some((name, is_dir)) = &self.confirm {
            let question = if *is_dir {
                format!("Delete the folder {name} and everything in it? This can't be undone.")
            } else {
                format!("Delete {name}? This can't be undone.")
            };
            let mut answer = None;
            ui.push_id("confirm", |ui| {
                ui.horizontal(|ui| {
                    ui.label(&question);
                    if ui.button(CONFIRM_DELETE).clicked() {
                        answer = Some(true);
                    }
                    if ui.button(CANCEL).clicked() {
                        answer = Some(false);
                    }
                });
            });
            match answer {
                Some(true) => self.delete_confirmed(ui),
                Some(false) => self.confirm = None,
                None => {}
            }
        }
        if let Some(status) = &self.status {
            ui.push_id("status", |ui| ui.label(status));
        }

        ui.horizontal(|ui| {
            ui.label("🔍 Filter");
            ui.text_edit(&mut self.filter);
        });
        ui.separator();

        // Deferred so the borrow of `self` inside the match does not collide with mutating it.
        let mut enter: Option<String> = None;
        let mut delete: Option<(String, bool)> = None;
        let mut start: Option<(String, ArchiveAction)> = None;
        // Each archive action makes something in the folder on screen, so it is offered only
        // where the caller may write, and only while no other job is running.
        let can_archive = here == Some(true) && self.archiving.is_none();
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
                                let path = join(&self.path, &entry.name);
                                if !entry.is_dir() {
                                    ui.tooltip("Right-click to open, share or download it");
                                }
                                // Attached to the name just drawn, so right-clicking it is
                                // what opens it. A folder's menu only has something in it if
                                // it can be deleted, so a read-only folder has none.
                                if !entry.is_dir() || entry.writable || can_archive {
                                    ui.context_menu(|ui| {
                                        if !entry.is_dir() {
                                            file_menu(ui, &path);
                                        }
                                        if can_archive {
                                            for (label, action) in
                                                archive_actions(&entry.name, entry.is_dir())
                                            {
                                                if ui.button(label).clicked() {
                                                    start = Some((entry.name.clone(), action));
                                                }
                                            }
                                        }
                                        if entry.writable && ui.button(DELETE).clicked() {
                                            delete = Some((entry.name.clone(), entry.is_dir()));
                                        }
                                    });
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
        if delete.is_some() {
            self.confirm = delete;
        }
        if let Some((name, action)) = start {
            self.start_archive(&name, action);
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

/// The archive entries of a right-click menu.
pub const COMPRESS: &str = "\u{E09E}  Compress (.tar.gz)";
pub const GZIP: &str = "\u{E958}  Gzip (.gz)";
pub const EXTRACT: &str = "\u{E0A6}  Extract here";

/// What can be done to `name` with archives: extract an archive; compress anything; gzip a
/// file that isn't already compressed.
fn archive_actions(name: &str, is_dir: bool) -> Vec<(&'static str, ArchiveAction)> {
    let archive = !is_dir && archive_kind(name).is_some();
    let mut out = Vec::new();
    if archive {
        out.push((EXTRACT, ArchiveAction::Extract));
    }
    out.push((COMPRESS, ArchiveAction::Compress));
    if !is_dir && !archive {
        out.push((GZIP, ArchiveAction::Gzip));
    }
    out
}

/// "Compressing", for "Compressing photos… 40%".
fn doing(action: ArchiveAction) -> &'static str {
    match action {
        ArchiveAction::Compress => "Compressing",
        ArchiveAction::Gzip => "Gzipping",
        ArchiveAction::Extract => "Extracting",
    }
}

/// "Compressed", for "Compressed photos into photos.tar.gz".
fn done(action: ArchiveAction) -> &'static str {
    match action {
        ArchiveAction::Compress => "Compressed",
        ArchiveAction::Gzip => "Gzipped",
        ArchiveAction::Extract => "Extracted",
    }
}

/// "compress", for "Couldn't compress photos: …".
fn verb(action: ArchiveAction) -> &'static str {
    match action {
        ArchiveAction::Compress => "compress",
        ArchiveAction::Gzip => "gzip",
        ArchiveAction::Extract => "extract",
    }
}

/// "Compressing photos… 40%", or without a percentage until the size is known.
fn progress(action: ArchiveAction, name: &str, status: &ArchiveStatus) -> String {
    let mut s = format!("{} {name}…", doing(action));
    let done = status.done_bytes.min(status.total_bytes) * 100;
    if let Some(percent) = done.checked_div(status.total_bytes) {
        s.push(' ');
        s.push_str(&itoa(percent.min(99)));
        s.push('%');
    }
    s
}

/// A writable entry's right-click menu entry, then the two answers to "are you sure?".
pub const DELETE: &str = "\u{E4A6}  Delete";
pub const CONFIRM_DELETE: &str = "\u{E4A6}  Delete it";
pub const CANCEL: &str = "Cancel";

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
