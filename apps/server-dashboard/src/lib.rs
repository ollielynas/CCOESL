//! The Server Dashboard app.
//!
//! Surfaces the state `ccosel-server` actually holds today: which protocol it speaks, which
//! directory it is serving files from ([`ServerInfo`]), and what is currently in that shared
//! jail ([`ListDir`], the same query the File Browser uses). A Refresh button forces the server
//! to be asked again rather than trusting a stale cache, which is the one piece of "managing"
//! the server supports right now — it exposes no other runtime-mutable state (no jobs, no
//! connected-client registry) as of this writing. This app is meant to grow into those as the
//! server grows them, not to invent management surface the server does not have.

use ccosel_proto::fs::{EntryKind, ListDir, ListDirReq};
use ccosel_proto::info::{ServerInfo, ServerInfoReq};
use ccosel_sdk::{App, Poll, Ui};

pub struct ServerDashboard {
    /// The directory currently being inspected under the jail root.
    path: String,
}

impl Default for ServerDashboard {
    fn default() -> Self {
        Self {
            path: "/".to_owned(),
        }
    }
}

/// A single glyph per kind, matching the convention the File Browser and the shell's app icons
/// already use.
fn icon_for(kind: EntryKind) -> &'static str {
    match kind {
        EntryKind::Dir => "📁",
        EntryKind::Symlink => "🔗",
        EntryKind::File | EntryKind::Other => "📄",
    }
}

/// Human-readable size without touching float `Display`, which would drag 20-40 KB of
/// float-formatting machinery into a module every user downloads.
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

impl ServerDashboard {
    fn go_up(&mut self) {
        if self.path == "/" {
            return;
        }
        match self.path.rfind('/') {
            Some(0) | None => self.path = "/".to_owned(),
            Some(i) => self.path.truncate(i),
        }
    }

    fn enter(&mut self, name: &str) {
        if !self.path.ends_with('/') {
            self.path.push('/');
        }
        self.path.push_str(name);
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

impl App for ServerDashboard {
    fn update(&mut self, ui: &mut Ui<'_>) {
        ui.label("Server Dashboard");
        ui.separator();

        ui.label("Server");
        match ui.rpc().get::<ServerInfo>(&ServerInfoReq) {
            Poll::Pending => ui.label("Loading…"),
            Poll::Failed(err) => {
                ui.label(err.message());
                if ui.button("Retry server info").clicked() {
                    ui.rpc().invalidate::<ServerInfo>(&ServerInfoReq);
                }
            }
            Poll::Ready(info) => {
                ui.horizontal(|ui| {
                    ui.label("Protocol version");
                    ui.label(itoa(u64::from(info.proto_version)).as_str());
                });
                ui.horizontal(|ui| {
                    ui.label("Shared root");
                    ui.label(info.root.as_str());
                });
            }
        }

        ui.separator();
        ui.label("Shared storage");

        // Button presses are recorded and acted on after the closures below: a request borrows
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
            ui.tooltip("Ask the server again instead of trusting the cache");
        });

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
        }
        if refresh {
            ui.rpc().invalidate::<ServerInfo>(&ServerInfoReq);
            ui.rpc()
                .invalidate::<ListDir>(&ListDirReq { path: &self.path });
        }

        ui.separator();

        let mut retry = false;
        let mut enter: Option<String> = None;

        // Bound to a local so the borrow of `self.path` ends before the arms run.
        let listing = ui.rpc().get::<ListDir>(&ListDirReq { path: &self.path });

        match listing {
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
                let dirs = listing.entries.iter().filter(|e| e.is_dir()).count();
                let files = listing.entries.len() - dirs;
                let bytes: u64 = listing
                    .entries
                    .iter()
                    .filter(|e| !e.is_dir())
                    .map(|e| e.size)
                    .sum();

                ui.label(
                    format!("{dirs} folders, {files} files, {} total", human_size(bytes)).as_str(),
                );
                if listing.truncated {
                    ui.label("(listing truncated)");
                }

                if listing.entries.is_empty() {
                    ui.label("(empty directory)");
                }

                for entry in &listing.entries {
                    ui.push_id(&entry.name, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(icon_for(entry.kind));
                            if entry.is_dir() {
                                if ui.button(entry.name.as_str()).clicked() {
                                    enter = Some(entry.name.clone());
                                }
                            } else {
                                ui.label(entry.name.as_str());
                                ui.label(format!("· {}", human_size(entry.size)).as_str());
                            }
                        });
                    });
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
    }
}

#[cfg(test)]
mod tests;

ccosel_sdk::ccosel_app!(ServerDashboard);
