//! The Docs app: read, write and find Markdown documents on the server.
//!
//! It is a file browser that shows only folders and `.md` files, opens a document by rendering
//! it, and edits one in a Markdown editor with a live preview. It enforces nothing itself:
//! who may read or change what is the server's decision (see `ccosel-server`'s `access`), and
//! this app only asks, so it can offer "Edit" and "New document" where they will work and say
//! "Read-only" where they will not.
//!
//! Three places are one click away: **Documentation** (`/Docs`, written by the developers and
//! read-only here), **Shared** (the whole server) and **My documents** (`/home/{user}`, which
//! only the signed-in user can see).

use ccosel_proto::fs::{
    Access, CreateDir, EntryKind, FileText, ListDir, ListDirReq, PathReq, ReadFile, Search,
    SearchReq, WriteFile, WriteFileReq,
};
use ccosel_sdk::{App, CallId, Poll, Text, TextStyle, Ui};

pub mod markdown;
pub mod paths;
pub mod render;

use render::Follow;

/// Where the developer-maintained documentation lives.
pub const DOCS_ROOT: &str = "/Docs";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum View {
    /// A folder's documents and subfolders.
    Browse(String),
    /// A document, rendered.
    Read(String),
    /// A document in the editor.
    Edit(String),
    /// Search results for a query.
    Search(String),
}

/// A write in flight, and what to do when it lands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pending {
    Save(CallId),
    NewDoc(CallId),
    NewFolder(CallId),
}

pub struct Docs {
    pub view: View,
    /// Documents and folders visited before this one, for Back.
    history: Vec<View>,
    pub search: Text,
    /// The name typed for a new document or folder.
    pub new_name: Text,
    pub editor: Text,
    /// The editor's text as last loaded or saved, to tell whether there are unsaved changes.
    saved_text: String,
    /// Whether the editor has been filled for the document in `view` yet.
    editor_loaded: bool,
    pub preview: bool,
    pending: Option<Pending>,
    /// Path of the document or folder being created, while `pending` is a create.
    creating: String,
    /// A one-line result of the last action: "Saved", or why it failed.
    pub status: Option<String>,
    /// Something changed this frame that the next frame will draw differently, such as a new
    /// view. Asks the shell for that frame now rather than at the next input.
    changed: bool,
}

impl Default for Docs {
    fn default() -> Self {
        Self {
            view: View::Browse(DOCS_ROOT.to_owned()),
            history: Vec::new(),
            search: Text::new(""),
            new_name: Text::new(""),
            editor: Text::new(""),
            saved_text: String::new(),
            editor_loaded: false,
            preview: true,
            pending: None,
            creating: String::new(),
            status: None,
            changed: false,
        }
    }
}

/// What the user asked for this frame. Collected while drawing and acted on afterwards, since
/// drawing borrows the state that acting changes.
#[derive(Default)]
struct Actions {
    go: Option<View>,
    back: bool,
    edit: bool,
    save: bool,
    close_editor: bool,
    new_doc: bool,
    new_folder: bool,
    insert: Option<&'static str>,
    refresh: bool,
}

impl Docs {
    fn open(&mut self, view: View) {
        if view == self.view {
            return;
        }
        let from = core::mem::replace(&mut self.view, view);
        // Leaving the editor is not a place to come back to; its document is.
        let from = match from {
            View::Edit(path) => View::Read(path),
            other => other,
        };
        if self.history.last() != Some(&from) {
            self.history.push(from);
        }
        self.editor_loaded = false;
        self.status = None;
    }

    fn back(&mut self) {
        if let Some(prev) = self.history.pop() {
            self.view = prev;
            self.editor_loaded = false;
            self.status = None;
        }
    }

    /// Whether the editor holds changes that have not been saved.
    pub fn dirty(&self) -> bool {
        matches!(self.view, View::Edit(_))
            && self.editor_loaded
            && self.editor.as_str() != self.saved_text
    }

    fn places(&mut self, ui: &mut Ui<'_>, user: Option<&str>, act: &mut Actions) {
        ui.horizontal(|ui| {
            if ui.button("📘 Documentation").clicked() {
                act.go = Some(View::Browse(DOCS_ROOT.to_owned()));
            }
            ui.tooltip("How to use each app. Maintained by the developers, read-only here.");
            if ui.button("🗂 Shared").clicked() {
                act.go = Some(View::Browse("/".to_owned()));
            }
            ui.tooltip("Everything on the server you can see");
            match user {
                Some(name) => {
                    if ui.button("🏠 My documents").clicked() {
                        act.go = Some(View::Browse(paths::join("/home", name)));
                    }
                    ui.tooltip("Your private folder. Only you can see what is in it.");
                }
                None => {
                    ui.styled("Sign in for a private folder", TextStyle::WEAK);
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label("🔍");
            ui.text_edit(&mut self.search);
            ui.tooltip("Search document names and text");
            let query = self.search.as_str().trim();
            if ui.button("Search").clicked() && !query.is_empty() {
                act.go = Some(View::Search(query.to_owned()));
            }
            ui.push_id("back", |ui| {
                if !self.history.is_empty() && ui.button("← Back").clicked() {
                    act.back = true;
                }
            });
        });
        ui.separator();
    }

    /// A clickable trail from the root to `path`.
    fn crumbs(ui: &mut Ui<'_>, path: &str, act: &mut Actions) {
        ui.horizontal(|ui| {
            if ui.button("/").clicked() {
                act.go = Some(View::Browse("/".to_owned()));
            }
            let mut acc = String::new();
            let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
            for (i, seg) in segments.iter().enumerate() {
                acc.push('/');
                acc.push_str(seg);
                if i > 0 {
                    ui.label("›");
                }
                ui.push_id(&acc, |ui| {
                    let last = i + 1 == segments.len();
                    if last {
                        ui.styled(seg, TextStyle::STRONG);
                    } else if ui.button(seg).clicked() {
                        act.go = Some(View::Browse(acc.clone()));
                    }
                });
            }
        });
    }

    fn browse(&mut self, ui: &mut Ui<'_>, dir: &str, act: &mut Actions) {
        Self::crumbs(ui, dir, act);
        let writable = matches!(
            ui.rpc().get::<Access>(&PathReq { path: dir }),
            Poll::Ready(a) if a.write
        );

        match ui.rpc().get::<ListDir>(&ListDirReq { path: dir }) {
            Poll::Pending => {
                ui.label("Loading…");
            }
            Poll::Failed(e) => {
                ui.label(e.message());
                if ui.button("Retry").clicked() {
                    act.refresh = true;
                }
            }
            Poll::Ready(listing) => {
                let mut shown = 0;
                for entry in &listing.entries {
                    let path = paths::join(dir, &entry.name);
                    let is_dir = entry.kind == EntryKind::Dir;
                    // Only what this app can open: folders to look in, and documents.
                    if !is_dir && !paths::is_doc(&entry.name) {
                        continue;
                    }
                    shown += 1;
                    ui.push_id(&entry.name, |ui| {
                        ui.group(|ui| {
                            ui.horizontal(|ui| {
                                let label = if is_dir {
                                    format!("📁 {}", entry.name)
                                } else {
                                    format!("📄 {}", paths::title(&entry.name))
                                };
                                if ui.button(&label).clicked() {
                                    act.go = Some(if is_dir {
                                        View::Browse(path.clone())
                                    } else {
                                        View::Read(path.clone())
                                    });
                                }
                            });
                        });
                    });
                }
                if shown == 0 {
                    ui.styled(
                        "No documents here yet.",
                        TextStyle::WEAK | TextStyle::ITALIC,
                    );
                }
                if listing.truncated {
                    ui.styled("(some entries are not shown)", TextStyle::WEAK);
                }
            }
        }

        ui.separator();
        if writable {
            ui.horizontal(|ui| {
                ui.label("Name");
                ui.text_edit(&mut self.new_name);
                if ui.button("📄 New document").clicked() {
                    act.new_doc = true;
                }
                if ui.button("📁 New folder").clicked() {
                    act.new_folder = true;
                }
            });
        } else {
            ui.styled("🔒 Read-only folder", TextStyle::WEAK);
            ui.tooltip("You can read what is here but not add to it.");
        }
    }

    fn read(ui: &mut Ui<'_>, path: &str, act: &mut Actions) {
        let doc = ui.rpc().get::<ReadFile>(&PathReq { path });
        ui.horizontal(|ui| {
            if ui.button("📂 Folder").clicked() {
                act.go = Some(View::Browse(paths::parent(path)));
            }
            ui.tooltip(&paths::parent(path));
            ui.styled(paths::title(path), TextStyle::STRONG);
            ui.push_id("mode", |ui| match &doc {
                Poll::Ready(file) if file.writable => {
                    if ui.button("✏ Edit").clicked() {
                        act.edit = true;
                    }
                }
                Poll::Ready(_) => {
                    ui.styled("🔒 Read-only", TextStyle::WEAK);
                    ui.tooltip("You can read this document but not change it.");
                }
                _ => {}
            });
            ui.open_url("⬇ Download", &paths::download_url(path));
        });
        ui.separator();
        match doc {
            Poll::Pending => {
                ui.label("Loading…");
            }
            Poll::Failed(e) => {
                ui.label(e.message());
                if ui.button("Retry").clicked() {
                    act.refresh = true;
                }
            }
            Poll::Ready(file) => {
                if let Some(Follow::Doc(to)) =
                    render::document(ui, path, &markdown::parse(&file.text))
                {
                    act.go = Some(View::Read(to));
                }
            }
        }
    }

    fn edit(&mut self, ui: &mut Ui<'_>, path: &str, act: &mut Actions) {
        if !self.editor_loaded {
            match ui.rpc().get::<ReadFile>(&PathReq { path }) {
                Poll::Pending => {
                    ui.label("Loading…");
                    return;
                }
                Poll::Failed(e) => {
                    ui.label(e.message());
                    if ui.button("Close").clicked() {
                        act.close_editor = true;
                    }
                    return;
                }
                Poll::Ready(file) => self.load(&file),
            }
        }

        let saving = matches!(self.pending, Some(Pending::Save(_)));
        ui.horizontal(|ui| {
            ui.styled(paths::title(path), TextStyle::STRONG);
            let dirty = self.dirty();
            ui.push_id("unsaved", |ui| {
                if dirty {
                    ui.styled("● unsaved", TextStyle::WEAK);
                }
            });
            ui.push_id("save", |ui| {
                if saving {
                    ui.label("Saving…");
                } else if ui.button("💾 Save").clicked() {
                    act.save = true;
                }
            });
            if ui.button("✔ Done").clicked() {
                act.close_editor = true;
            }
            ui.tooltip("Back to reading. Save first to keep your changes.");
            let label = if self.preview {
                "Hide preview"
            } else {
                "Show preview"
            };
            if ui.button(label).clicked() {
                self.preview = !self.preview;
            }
        });
        // Markdown snippets, added at the end of the document: the shell does not tell apps
        // where the cursor is.
        ui.horizontal(|ui| {
            for (label, tip, snippet) in [
                ("H", "Heading", "\n## Heading\n"),
                ("B", "Bold", "**bold**"),
                ("I", "Italic", "*italic*"),
                ("🔗", "Link to a page or file", "[text](other-document.md)"),
                ("•", "Bulleted list", "\n- item\n"),
                ("1.", "Numbered list", "\n1. item\n"),
                ("☐", "Task", "\n- [ ] task\n"),
                ("{}", "Code block", "\n```\ncode\n```\n"),
            ] {
                if ui.button(label).clicked() {
                    act.insert = Some(snippet);
                }
                ui.tooltip(tip);
            }
        });
        ui.text_edit_multiline(&mut self.editor);
        if self.preview {
            ui.separator();
            ui.styled("Preview", TextStyle::WEAK);
            // Links are live in the preview too, but following one would leave the editor.
            if let Some(Follow::Doc(to)) =
                render::document(ui, path, &markdown::parse(self.editor.as_str()))
                && !self.dirty()
            {
                act.go = Some(View::Read(to));
            }
        }
    }

    fn load(&mut self, file: &FileText) {
        self.editor.set(&file.text);
        self.saved_text.clone_from(&file.text);
        self.editor_loaded = true;
    }

    fn results(ui: &mut Ui<'_>, query: &str, act: &mut Actions) {
        ui.horizontal(|ui| {
            ui.label("Results for");
            ui.styled(query, TextStyle::STRONG);
        });
        let req = SearchReq {
            path: "/",
            query,
            suffix: ".md",
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
                    ui.styled("No documents match.", TextStyle::WEAK | TextStyle::ITALIC);
                }
                for hit in &found.hits {
                    ui.push_id(&hit.path, |ui| {
                        ui.group(|ui| {
                            if ui
                                .button(&format!("📄 {}", paths::title(&hit.path)))
                                .clicked()
                            {
                                act.go = Some(View::Read(hit.path.clone()));
                            }
                            ui.styled(&paths::parent(&hit.path), TextStyle::WEAK);
                            if !hit.line.is_empty() {
                                ui.styled(&hit.line, TextStyle::ITALIC);
                            }
                        });
                    });
                }
                if found.truncated {
                    ui.styled(
                        "(more matches than shown; try a longer search)",
                        TextStyle::WEAK,
                    );
                }
            }
        }
    }

    /// Check on the write in flight, if any, and act on how it went.
    fn poll_pending(&mut self, ui: &mut Ui<'_>) {
        let Some(pending) = self.pending else {
            return;
        };
        let rpc = ui.rpc();
        let (poll, what) = match pending {
            Pending::Save(id) => (rpc.outcome::<WriteFile>(id).settled(), "save"),
            Pending::NewDoc(id) => (
                rpc.outcome::<WriteFile>(id).settled(),
                "create the document",
            ),
            Pending::NewFolder(id) => (rpc.outcome::<CreateDir>(id).settled(), "create the folder"),
        };
        let result = match poll {
            None => return,
            Some(r) => r,
        };
        self.pending = None;
        if let Err(message) = result {
            self.status = Some(format!("Could not {what}: {message}"));
            return;
        }
        match pending {
            Pending::Save(_) => {
                if let View::Edit(path) = &self.view {
                    rpc.invalidate::<ReadFile>(&PathReq { path });
                    rpc.invalidate::<Search>(&SearchReq {
                        path: "/",
                        query: self.search.as_str().trim(),
                        suffix: ".md",
                    });
                }
                self.saved_text = self.editor.as_str().to_owned();
                self.status = Some("Saved".to_owned());
            }
            Pending::NewDoc(_) => {
                let created = core::mem::take(&mut self.creating);
                rpc.invalidate::<ListDir>(&ListDirReq {
                    path: &paths::parent(&created),
                });
                self.new_name.set("");
                self.open(View::Edit(created));
            }
            Pending::NewFolder(_) => {
                let created = core::mem::take(&mut self.creating);
                rpc.invalidate::<ListDir>(&ListDirReq {
                    path: &paths::parent(&created),
                });
                self.new_name.set("");
                self.open(View::Browse(created));
            }
        }
    }

    fn apply(&mut self, ui: &mut Ui<'_>, act: Actions) {
        let rpc = ui.rpc();
        if act.back {
            self.back();
        }
        if let Some(view) = act.go {
            self.open(view);
        }
        if let Some(snippet) = act.insert {
            let mut text = self.editor.as_str().to_owned();
            text.push_str(snippet);
            self.editor.set(&text);
        }
        if act.refresh {
            match &self.view {
                View::Browse(path) => rpc.invalidate::<ListDir>(&ListDirReq { path }),
                View::Read(path) | View::Edit(path) => {
                    rpc.invalidate::<ReadFile>(&PathReq { path })
                }
                View::Search(_) => {}
            }
        }
        if act.edit
            && let View::Read(path) = &self.view
        {
            let path = path.clone();
            self.open(View::Edit(path));
        }
        if act.close_editor
            && let View::Edit(path) = &self.view
        {
            let path = path.clone();
            self.view = View::Read(path);
            self.editor_loaded = false;
        }
        if act.save
            && self.pending.is_none()
            && let View::Edit(path) = &self.view
        {
            self.status = None;
            self.pending = Some(Pending::Save(rpc.send::<WriteFile>(&WriteFileReq {
                path,
                text: self.editor.as_str(),
                create_only: false,
            })));
        }
        if (act.new_doc || act.new_folder) && self.pending.is_none() {
            self.create(ui, act.new_doc);
        }
    }

    fn create(&mut self, ui: &mut Ui<'_>, doc: bool) {
        let View::Browse(dir) = &self.view else {
            return;
        };
        let Some(name) = paths::new_name(self.new_name.as_str(), doc) else {
            self.status =
                Some("Type a name first (no slashes, not starting with a dot).".to_owned());
            return;
        };
        let path = paths::join(dir, &name);
        let rpc = ui.rpc();
        // The server refuses to overwrite, but saying so here reads better than its error.
        if let Poll::Ready(listing) = rpc.get::<ListDir>(&ListDirReq { path: dir })
            && listing.entries.iter().any(|e| e.name == name)
        {
            self.status = Some(format!("“{name}” already exists here."));
            return;
        }
        self.status = None;
        self.pending = Some(if doc {
            let heading = format!("# {}\n\n", paths::title(&name));
            Pending::NewDoc(rpc.send::<WriteFile>(&WriteFileReq {
                path: &path,
                text: &heading,
                create_only: true,
            }))
        } else {
            Pending::NewFolder(rpc.send::<CreateDir>(&PathReq { path: &path }))
        });
        self.creating = path;
    }
}

/// Collapse a command's outcome to "not yet", done, or failed with a message.
trait Outcome {
    fn settled(self) -> Option<Result<(), &'static str>>;
}

impl<T> Outcome for Poll<T> {
    fn settled(self) -> Option<Result<(), &'static str>> {
        match self {
            Poll::Pending => None,
            Poll::Ready(_) => Some(Ok(())),
            Poll::Failed(e) => Some(Err(e.message())),
        }
    }
}

impl App for Docs {
    fn wants_repaint_after_ms(&self) -> u32 {
        if self.changed {
            0
        } else {
            ccosel_sdk::REPAINT_ON_INPUT_ONLY
        }
    }

    fn update(&mut self, ui: &mut Ui<'_>) {
        let before = (self.view.clone(), self.pending, self.status.clone());
        self.poll_pending(ui);

        // Who the server thinks this is, which is where "My documents" lives.
        let user = match ui.rpc().get::<Access>(&PathReq { path: "/" }) {
            Poll::Ready(a) => a.user.clone(),
            _ => None,
        };

        let mut act = Actions::default();
        self.places(ui, user.as_deref(), &mut act);
        // Under its own id, like every widget that comes and goes: an auto id shifts every
        // sibling after it, and a text field whose id moves loses what was typed into it.
        ui.push_id("status", |ui| {
            if let Some(status) = &self.status {
                ui.styled(status, TextStyle::ITALIC);
            }
        });
        match self.view.clone() {
            View::Browse(dir) => self.browse(ui, &dir, &mut act),
            View::Read(path) => Self::read(ui, &path, &mut act),
            View::Edit(path) => self.edit(ui, &path, &mut act),
            View::Search(query) => Self::results(ui, &query, &mut act),
        }
        self.apply(ui, act);
        self.changed = before != (self.view.clone(), self.pending, self.status.clone());
    }
}

#[cfg(test)]
mod tests;

ccosel_sdk::ccosel_app!(Docs);
