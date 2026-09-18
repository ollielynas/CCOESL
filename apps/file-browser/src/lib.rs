//! The File Browser app.
//!
//! Compiles to a standalone `.wasm` module with no `wasm-bindgen` and no egui — it records
//! commands, the shell draws them.
//!
//! Note what this app does *not* contain: no call ids, no "have I requested yet" flag, no
//! `on_event` handler, no cancellation, and no check for a stale reply arriving after the user
//! navigated away. Asking for a listing every frame is the whole of the data flow, because the
//! request cache keys on the request itself — so changing `self.path` *is* the re-request.

use ccosel_proto::fs::{EntryKind, ListDir, ListDirReq};
use ccosel_sdk::{App, Poll, Text, Ui};

pub struct FileBrowser {
    path: String,
    filter: Text,
    /// Keyed by name, not by index: the listing is replaced asynchronously, so an index would
    /// silently come to mean a different file.
    selected: Option<String>,
}

impl Default for FileBrowser {
    fn default() -> Self {
        Self {
            path: "/".to_owned(),
            filter: Text::new(""),
            selected: None,
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
    fn go_up(&mut self) {
        if self.path == "/" {
            return;
        }
        match self.path.rfind('/') {
            Some(0) | None => self.path = "/".to_owned(),
            Some(i) => self.path.truncate(i),
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

    /// The path as a chain of `(label, full path)` breadcrumbs, root first.
    fn crumbs(&self) -> Vec<(String, String)> {
        let mut out = vec![("🏠".to_owned(), "/".to_owned())];
        let mut acc = String::new();
        for seg in self.path.split('/').filter(|s| !s.is_empty()) {
            acc.push('/');
            acc.push_str(seg);
            out.push((seg.to_owned(), acc.clone()));
        }
        out
    }
}

impl App for FileBrowser {
    fn update(&mut self, ui: &mut Ui<'_>) {
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

        if go_up {
            self.go_up();
        }
        if let Some(path) = go_to {
            self.path = path;
            self.selected = None;
        }
        if refresh {
            ui.rpc().invalidate::<ListDir>(&ListDirReq { path: &self.path });
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
                total = listing.entries.len();
                let needle = self.filter.as_str().to_ascii_lowercase();

                if total > 0 {
                    ui.horizontal(|ui| {
                        ui.label("Name");
                        ui.label("Size");
                    });
                    ui.separator();
                }

                for entry in &listing.entries {
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
                                    ui.label(format!("· {}", human_size(entry.size)).as_str());
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
            ui.rpc().invalidate::<ListDir>(&ListDirReq { path: &self.path });
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

ccosel_sdk::ccosel_app!(FileBrowser);
