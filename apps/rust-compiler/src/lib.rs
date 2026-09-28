//! The Rust Compiler app.
//!
//! Upload a project from this computer, or browse to one already on the server, press Build,
//! watch it compile, download the binary the server's own toolchain produced.
//!
//! An uploaded project is temporary: it goes into a scratch folder the server deletes after an
//! hour unused (`ccosel_proto::scratch`), leaving out whatever its `.gitignore` excludes, so
//! `target/` never crosses the network and nothing lingers in the shared folder.
//!
//! A build takes minutes, so `Compile` is not a call that waits for one — it is a **poll**.
//! The first request for a `(path, generation)` starts the job server-side; every later one
//! reports how far it has got. `generation` is bumped once per Build press, which makes it the
//! idempotency key: the poll loop below runs every frame and can never start a second build.
//!
//! Two pieces of app state earn their place. `building` is the directory a build was asked for,
//! deliberately *not* `path`, so navigating away mid-build keeps showing that build instead of
//! silently retargeting it. `last_status` is the most recent snapshot, held so the bar keeps
//! drawing while the next poll is in flight rather than flickering back to "loading".

use std::rc::Rc;

use ccosel_proto::build::{Compile, CompileReq, CompileStatus};
use ccosel_proto::fs::{ListDir, ListDirReq};
use ccosel_proto::scratch;
use ccosel_sdk::{App, Poll, Ui};

const MANIFEST: &str = "Cargo.toml";

/// 4 Hz while a build runs. `ARCHITECTURE.md` caps compile progress here on purpose: a bad LAN
/// plus a chatty progress feed is the fastest way to make this feel broken.
const POLL_MS: u32 = 250;

pub struct RustCompiler {
    path: String,
    building: Option<String>,
    /// Bumped once per Build press. See the module docs: this is the idempotency key.
    generation: u32,
    last_status: Option<Rc<CompileStatus>>,
    /// Drives `wants_repaint_after_ms`. Without a timer the app would only re-run on input,
    /// and a build would appear to stop the moment you stopped moving the mouse.
    polling: bool,
    /// The scratch folder of the last finished upload the app has acted on, so each one is
    /// opened once rather than every frame.
    uploaded: Option<u32>,
}

impl Default for RustCompiler {
    fn default() -> Self {
        Self {
            path: "/".to_owned(),
            building: None,
            generation: 0,
            last_status: None,
            polling: false,
            uploaded: None,
        }
    }
}

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

/// `path` as the user should read it: an uploaded project's `/.scratch/<id>` prefix becomes
/// "Uploaded: ", the same wording as its breadcrumb.
fn display_path(path: &str) -> String {
    let Some(id) = scratch::id_of(path) else {
        return path.to_owned();
    };
    let rest = &path[scratch::path(id).len()..];
    let mut s = String::from("⬆ Uploaded: ");
    s.push_str(rest.trim_start_matches('/'));
    s
}

/// `/.scratch`, the directory holding every scratch folder.
fn scratch_dir() -> String {
    let mut s = String::from("/");
    s.push_str(scratch::DIR);
    s
}

/// The id if `path` is exactly a scratch folder (`/.scratch/<id>`), not something inside one.
fn scratch_root(path: &str) -> Option<u32> {
    let id = scratch::id_of(path)?;
    (path.trim_end_matches('/') == scratch::path(id)).then_some(id)
}

/// `/files/<path>`, each segment percent-encoded. `path` is jail-relative and its segments come
/// from the project's own directory and crate names, which may hold spaces or `#`.
pub fn download_url(path: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut url = String::from("/files");
    for segment in path.split('/').filter(|s| !s.is_empty()) {
        url.push('/');
        for &b in segment.as_bytes() {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
                url.push(b as char);
            } else {
                url.push('%');
                url.push(HEX[usize::from(b >> 4)] as char);
                url.push(HEX[usize::from(b & 0xF)] as char);
            }
        }
    }
    url
}

/// Hand-rolled for the same reason the File Browser hand-rolls it: float `Display` drags
/// 20–40 KB of formatting machinery into a module every user downloads.
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

impl RustCompiler {
    fn go_up(&mut self) {
        if self.path == "/" {
            return;
        }
        match self.path.rfind('/') {
            Some(0) | None => self.path = "/".to_owned(),
            Some(i) => self.path.truncate(i),
        }
        // A scratch folder holds only the uploaded project and would open it straight back up,
        // so going up out of the project lands at the top instead.
        if scratch_root(&self.path).is_some() || self.path == scratch_dir() {
            self.path = "/".to_owned();
        }
    }

    fn enter(&mut self, name: &str) {
        if !self.path.ends_with('/') {
            self.path.push('/');
        }
        self.path.push_str(name);
    }

    /// The path as a chain of `(label, full path)` breadcrumbs, root first. Mirrors the File
    /// Browser's `crumbs`: a project worth building is often a few directories deep, and
    /// getting there — or back to an ancestor — should be one click, not a string of "Up"s.
    ///
    /// Inside an uploaded project, `/.scratch/<id>` is an implementation detail: it shows as a
    /// single "Uploaded" crumb that leads to the project itself.
    fn crumbs(&self) -> Vec<(String, String)> {
        let mut out = vec![("🏠".to_owned(), "/".to_owned())];
        let mut acc = String::new();
        let mut segs = self.path.split('/').filter(|s| !s.is_empty()).peekable();
        if let Some(id) = scratch::id_of(&self.path) {
            acc = scratch::path(id);
            segs.next();
            segs.next();
            if let Some(project) = segs.next() {
                acc.push('/');
                acc.push_str(project);
                out.push((format!("⬆ Uploaded: {project}"), acc.clone()));
            }
        }
        for seg in segs {
            acc.push('/');
            acc.push_str(seg);
            out.push((seg.to_owned(), acc.clone()));
        }
        out
    }
}

/// The progress indicator, plus what cargo is chewing on right now.
fn draw_progress(ui: &mut Ui<'_>, status: Option<&CompileStatus>) {
    let Some(status) = status else {
        ui.label("Compiling… Starting…");
        return;
    };

    let label = if status.units_total > 0 {
        format!(
            "Compiling… {}/{} crates",
            status.units_done, status.units_total
        )
    } else {
        format!("Compiling… {} crates", status.units_done)
    };
    ui.label(label.as_str());

    if !status.current.is_empty() {
        ui.horizontal(|ui| {
            ui.label("  ");
            ui.label(status.current.as_str());
        });
    }
}

fn draw_result(ui: &mut Ui<'_>, status: &CompileStatus) {
    let Some(result) = &status.result else {
        return;
    };

    ui.label(if result.success {
        "✅ Build succeeded"
    } else {
        "❌ Build failed"
    });

    for binary in &result.binaries {
        ui.push_id(&binary.path, |ui| {
            ui.group(|ui| {
                ui.horizontal(|ui| {
                    ui.label("📦");
                    ui.label(binary.name.as_str());
                    ui.label(human_size(binary.size).as_str());
                    // The shell opens it in a new tab; the server sends it as a download.
                    ui.open_url("Download", &download_url(&binary.path));
                });
            });
        });
    }

    if !result.output.is_empty() {
        ui.separator();
        if result.output_truncated {
            ui.label("(earlier output trimmed)");
        }
        // One label per line: the ABI has no multi-line text widget, and the shell's window
        // already scrolls.
        for line in result.output.lines() {
            ui.label(line);
        }
    }
}

impl App for RustCompiler {
    fn update(&mut self, ui: &mut Ui<'_>) {
        // Clicks are recorded here and acted on below: a request borrows `self.path`, and
        // navigation mutates it.
        let mut go_up = false;
        let mut go_to: Option<String> = None;
        let mut enter: Option<String> = None;
        let mut build = false;

        let mut uploaded: Option<u32> = None;
        ui.horizontal(|ui| {
            if ui.button("⬆ Up").clicked() {
                go_up = true;
            }
            ui.tooltip("Go to parent directory");
            let r = ui.upload_project();
            ui.tooltip(
                "Upload a Rust project from this computer to build it. Files its .gitignore \
                 excludes are left out, and the upload is deleted after an hour unused.",
            );
            if r.uploaded_project() != self.uploaded {
                uploaded = r.uploaded_project();
            }
        });
        if let Some(id) = uploaded {
            // A new upload landed: open it, and drop whatever the last build was of.
            self.uploaded = Some(id);
            self.path = scratch::path(id);
            self.building = None;
            self.last_status = None;
        }

        // A clickable trail, not just a path label: reaching a project a few directories deep
        // — or backing out to one on the way — is one click instead of several "Up"s.
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
        ui.separator();

        // Bound to a local so the borrow of `self.path` ends before the arms run.
        let listing = ui.rpc().get::<ListDir>(&ListDirReq { path: &self.path });

        let mut is_project = false;
        let in_scratch_root = scratch_root(&self.path).is_some();
        match listing {
            Poll::Pending => {
                ui.label("Loading…");
            }
            Poll::Failed(err) => {
                ui.label(err.message());
            }
            Poll::Ready(listing) => {
                is_project = listing
                    .entries
                    .iter()
                    .any(|e| e.name == MANIFEST && !e.is_dir());

                // Only directories: this picker exists to choose a project, and listing every
                // source file in a crate would bury the one thing you can actually click.
                // A scratch folder holds just the uploaded project: open it rather than make
                // the user click the only thing there.
                let mut only = listing.entries.iter().filter(|e| e.is_dir());
                if in_scratch_root && let (Some(project), None) = (only.next(), only.next()) {
                    enter = Some(project.name.clone());
                }
                let mut dirs = 0usize;
                for entry in listing.entries.iter().filter(|e| e.is_dir()) {
                    dirs += 1;
                    // Salted by name, not position, so a listing that changes underneath does
                    // not land a click on whichever row moved into the slot.
                    ui.push_id(&entry.name, |ui| {
                        ui.group(|ui| {
                            ui.horizontal(|ui| {
                                ui.label("📁");
                                if ui.button(&entry.name).clicked() {
                                    enter = Some(entry.name.clone());
                                }
                            });
                        });
                    });
                }
                if dirs == 0 {
                    ui.label("(no subdirectories)");
                }
            }
        }

        ui.separator();
        if is_project {
            ui.horizontal(|ui| {
                if ui.button("🔨 Build").clicked() {
                    build = true;
                }
                ui.tooltip("Run cargo build --release on the server");
                ui.label("Cargo.toml found here");
            });
        } else {
            ui.label("No Cargo.toml here — open a crate directory to build it.");
        }
        if scratch::id_of(&self.path).is_some() {
            ui.label("Uploaded for this build only: deleted after an hour unused.");
        }

        if go_up {
            self.go_up();
        }
        if let Some(path) = go_to {
            self.path = path;
        }
        if let Some(name) = enter {
            self.enter(&name);
        }
        if build {
            // A new generation is a new job, so this is what makes Build mean *build again*
            // rather than "show me the last result".
            self.generation = self.generation.wrapping_add(1);
            self.building = Some(self.path.clone());
            self.last_status = None;
        }

        let Some(target) = self.building.clone() else {
            self.polling = false;
            return;
        };

        ui.separator();
        ui.horizontal(|ui| {
            ui.label("Build:");
            ui.label(&display_path(&target));
        });

        let req = CompileReq {
            path: &target,
            generation: self.generation,
        };

        match ui.rpc().get::<Compile>(&req) {
            Poll::Ready(status) => {
                self.last_status = Some(status.clone());
                if status.finished {
                    self.polling = false;
                    draw_result(ui, &status);
                } else {
                    self.polling = true;
                    draw_progress(ui, Some(&status));
                    // Drop the snapshot we just drew so next frame asks the server again. The
                    // job is keyed by `(path, generation)`, so this re-reads progress — it
                    // never starts a second build.
                    ui.rpc().invalidate::<Compile>(&req);
                }
            }
            // A poll is on the wire. Draw the previous snapshot rather than flickering back to
            // a blank bar between ticks.
            Poll::Pending => {
                self.polling = true;
                let last = self.last_status.clone();
                draw_progress(ui, last.as_deref());
            }
            Poll::Failed(err) => {
                self.polling = false;
                ui.label(err.message());
            }
        }
    }

    fn wants_repaint_after_ms(&self) -> u32 {
        if self.polling {
            POLL_MS
        } else {
            ccosel_sdk::REPAINT_ON_INPUT_ONLY
        }
    }
}

#[cfg(test)]
mod tests;

ccosel_sdk::ccosel_app!(RustCompiler);
