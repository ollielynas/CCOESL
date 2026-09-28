//! The Docs app: read, write and find Markdown documents on the server.
//!
//! It is a file browser that shows only folders and `.md` files, opens a document by rendering
//! it, and edits one in a Markdown editor with a live preview. It enforces nothing itself:
//! who may read or change what is the server's decision (see `ccosel-server`'s `access`), and
//! this app only asks, so it can offer "Edit" and "New document" where they will work and say
//! "Read-only" where they will not.
//!
//! A sidebar holds search, three places — **Documentation** (`/Docs`, written by the developers
//! and read-only here), **Shared** (the whole server) and **My documents** (`/home/{user}`,
//! which only the signed-in user can see) — and a folder tree of whichever of those you are in.

use ccosel_proto::fs::{
    Access, CreateDir, EntryKind, FileText, ListDir, ListDirReq, PathReq, ReadFile, Search,
    SearchReq, WriteFile, WriteFileReq,
};
use std::collections::BTreeSet;

use ccosel_sdk::{App, CallId, Poll, Text, TextStyle, Ui, icons};

pub mod markdown;
pub mod paths;
pub mod phosphor;
pub mod render;

use render::Follow;

/// Where the developer-maintained documentation lives.
pub const DOCS_ROOT: &str = "/Docs";

/// How deep the sidebar tree goes. Past this, the folder view in the main column still works.
const TREE_DEPTH: usize = 8;

impl View {
    /// The folder or document this view is about, if any.
    fn path(&self) -> Option<&str> {
        match self {
            Self::Browse(p) | Self::Read(p) | Self::Edit(p) => Some(p),
            Self::Search(_) => None,
        }
    }
}

/// A label with an icon in front: how every button in this app is written.
pub fn label(icon: &str, text: &str) -> String {
    format!("{icon}  {text}")
}

/// The sidebar's name for a place, with its icon.
pub fn place_label(root: &str, user: Option<&str>) -> String {
    if root == DOCS_ROOT {
        label(icons::BOOK_OPEN, "Documentation")
    } else if user.is_some_and(|u| root == paths::join("/home", u)) {
        label(icons::HOUSE, "My documents")
    } else {
        label(icons::USERS, "Shared")
    }
}

/// The top of the tree the sidebar shows for `path`: the place it is in.
pub fn section_root(path: &str, user: Option<&str>) -> String {
    let in_folder = |root: &str| path == root || path.starts_with(&format!("{root}/"));
    if in_folder(DOCS_ROOT) {
        return DOCS_ROOT.to_owned();
    }
    if let Some(name) = user {
        let home = paths::join("/home", name);
        if in_folder(&home) {
            return home;
        }
    }
    "/".to_owned()
}

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
    /// Folders open in the sidebar tree.
    pub expanded: BTreeSet<String>,
    /// The tree's top, kept while searching so the sidebar does not jump.
    section: String,
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
            expanded: BTreeSet::new(),
            section: DOCS_ROOT.to_owned(),
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
    /// Leave the editor even though there are unsaved changes.
    discard: bool,
    new_doc: bool,
    new_folder: bool,
    insert: Option<&'static str>,
    refresh: bool,
    /// Open or close this folder in the sidebar tree.
    toggle: Option<String>,
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
        self.reveal();
    }

    /// Open the sidebar tree down to what is on screen, so it always shows where you are.
    fn reveal(&mut self) {
        let Some(path) = self.view.path() else {
            return;
        };
        let folder = match self.view {
            View::Browse(_) => path.to_owned(),
            _ => paths::parent(path),
        };
        let mut acc = String::new();
        for seg in folder.split('/').filter(|s| !s.is_empty()) {
            acc.push('/');
            acc.push_str(seg);
            self.expanded.insert(acc.clone());
        }
    }

    fn back(&mut self) {
        if let Some(prev) = self.history.pop() {
            self.view = prev;
            self.editor_loaded = false;
            self.status = None;
            self.reveal();
        }
    }

    /// Whether the editor holds changes that have not been saved.
    pub fn dirty(&self) -> bool {
        matches!(self.view, View::Edit(_))
            && self.editor_loaded
            && self.editor.as_str() != self.saved_text
    }

    fn sidebar(&mut self, ui: &mut Ui<'_>, user: Option<&str>, act: &mut Actions) {
        ui.horizontal(|ui| {
            ui.text_edit(&mut self.search);
            ui.tooltip("Search document names and text");
            let query = self.search.as_str().trim();
            if ui.button(icons::MAGNIFYING_GLASS).clicked() && !query.is_empty() {
                act.go = Some(View::Search(query.to_owned()));
            }
            ui.tooltip("Search");
        });
        ui.separator();

        // The places, as rows of the same kind as the tree below, with the one you are in
        // highlighted.
        let mut places = vec![
            (
                DOCS_ROOT.to_owned(),
                "How to use each app. Maintained by the developers, read-only here.",
            ),
            ("/".to_owned(), "Everything on the server you can see"),
        ];
        if let Some(name) = user {
            places.push((
                paths::join("/home", name),
                "Your private folder. Only you can see what is in it.",
            ));
        }
        for (root, tip) in &places {
            ui.push_id(root, |ui| {
                let here = self.section == *root;
                if ui.selectable(here, &place_label(root, user)).clicked() {
                    act.go = Some(View::Browse(root.clone()));
                }
                ui.tooltip(tip);
            });
        }
        ui.push_id("signed-out", |ui| {
            if user.is_none() {
                ui.styled("Sign in for a private folder", TextStyle::WEAK);
            }
        });
        ui.separator();

        let root = self.section.clone();
        let current = self.view.path().map(str::to_owned);
        // Only the tree scrolls, and on its own: search and the places stay put, and scrolling
        // a long document does not move the sidebar.
        ui.push_id("tree", |ui| {
            ui.scroll(|ui| self.tree(ui, &root, 0, current.as_deref(), act));
        });
    }

    /// One folder's documents and subfolders in the sidebar, and the open subfolders' under them.
    fn tree(
        &self,
        ui: &mut Ui<'_>,
        dir: &str,
        depth: usize,
        current: Option<&str>,
        act: &mut Actions,
    ) {
        let listing = match ui.rpc().get::<ListDir>(&ListDirReq { path: dir }) {
            Poll::Ready(listing) => listing,
            Poll::Pending => {
                ui.styled("Loading…", TextStyle::WEAK);
                return;
            }
            // The main column says what went wrong; the sidebar just leaves the folder empty.
            Poll::Failed(_) => return,
        };
        let mut shown = 0;
        for entry in &listing.entries {
            let is_dir = entry.kind == EntryKind::Dir;
            if !is_dir && !paths::is_doc(&entry.name) {
                continue;
            }
            // Documentation and the home folders have places of their own above, so the
            // Shared tree does not show them a second time.
            if dir == "/" && is_dir && (entry.name == DOCS_ROOT[1..] || entry.name == "home") {
                continue;
            }
            shown += 1;
            let path = paths::join(dir, &entry.name);
            let open = is_dir && self.expanded.contains(&path);
            let here = current == Some(path.as_str());
            ui.push_id(&entry.name, |ui| {
                let text = if is_dir {
                    label(
                        if open {
                            icons::FOLDER_OPEN
                        } else {
                            icons::FOLDER
                        },
                        &entry.name,
                    )
                } else {
                    label(icons::FILE_TEXT, paths::title(&entry.name))
                };
                if ui.selectable(here, &text).clicked() {
                    if is_dir && here {
                        // A second click on the folder you are in opens or closes it.
                        act.toggle = Some(path.clone());
                    } else {
                        act.go = Some(if is_dir {
                            View::Browse(path.clone())
                        } else {
                            View::Read(path.clone())
                        });
                    }
                }
                if open && depth + 1 < TREE_DEPTH {
                    ui.indent(|ui| self.tree(ui, &path, depth + 1, current, act));
                }
            });
        }
        if shown == 0 {
            ui.styled("Empty", TextStyle::WEAK | TextStyle::ITALIC);
        }
        if listing.truncated {
            ui.styled("(more not shown)", TextStyle::WEAK);
        }
    }

    /// A clickable trail from the place `path` is in down to it. The last step, where you are,
    /// is plain text.
    fn crumbs(&self, ui: &mut Ui<'_>, path: &str, user: Option<&str>, act: &mut Actions) {
        let root = section_root(path, user);
        ui.horizontal(|ui| {
            let segments: Vec<&str> = path[root.len()..]
                .split('/')
                .filter(|s| !s.is_empty())
                .collect();
            let place = place_label(&root, user);
            if segments.is_empty() {
                ui.styled(&place, TextStyle::STRONG);
            } else if ui.button(&place).clicked() {
                act.go = Some(View::Browse(root.clone()));
            }
            let mut acc = root.clone();
            for (i, seg) in segments.iter().enumerate() {
                acc = paths::join(&acc, seg);
                ui.label("›");
                ui.push_id(&acc, |ui| {
                    if i + 1 == segments.len() {
                        ui.styled(paths::title(seg), TextStyle::STRONG);
                    } else if ui.button(seg).clicked() {
                        act.go = Some(View::Browse(acc.clone()));
                    }
                });
            }
        });
    }

    /// A folder, picked in the sidebar. The sidebar already lists what is in it, so this does
    /// not: it offers what can be done here, and shows the folder's README if it has one.
    fn browse(&mut self, ui: &mut Ui<'_>, dir: &str, user: Option<&str>, act: &mut Actions) {
        self.crumbs(ui, dir, user, act);
        let writable = matches!(
            ui.rpc().get::<Access>(&PathReq { path: dir }),
            Poll::Ready(a) if a.write
        );
        // Above the README, so the name field keeps its id whatever the README holds.
        ui.push_id("actions", |ui| {
            if writable {
                ui.horizontal(|ui| {
                    ui.label("Name");
                    ui.text_edit(&mut self.new_name);
                    if ui
                        .button(&label(icons::FILE_PLUS, "New document"))
                        .clicked()
                    {
                        act.new_doc = true;
                    }
                    if ui
                        .button(&label(icons::FOLDER_PLUS, "New folder"))
                        .clicked()
                    {
                        act.new_folder = true;
                    }
                });
            } else {
                ui.styled(
                    &label(icons::LOCK_SIMPLE, "Read-only folder"),
                    TextStyle::WEAK,
                );
                ui.tooltip("You can read what is here but not add to it.");
            }
        });
        ui.separator();

        match ui.rpc().get::<ListDir>(&ListDirReq { path: dir }) {
            Poll::Pending => {
                ui.label("Loading…");
            }
            Poll::Failed(e) => {
                ui.label(e.message());
                if ui.button(&label(icons::ARROW_CLOCKWISE, "Retry")).clicked() {
                    act.refresh = true;
                }
            }
            Poll::Ready(listing) => {
                let readme = listing.entries.iter().find(|e| {
                    e.kind == EntryKind::File
                        && matches!(
                            e.name.to_ascii_lowercase().as_str(),
                            "readme.md" | "index.md"
                        )
                });
                match readme {
                    Some(entry) => {
                        let path = paths::join(dir, &entry.name);
                        match ui.rpc().get::<ReadFile>(&PathReq { path: &path }) {
                            Poll::Ready(file) => {
                                if let Some(Follow::Doc(to)) =
                                    render::document(ui, &path, &markdown::parse(&file.text))
                                {
                                    act.go = Some(View::Read(to));
                                }
                            }
                            Poll::Pending => {
                                ui.label("Loading…");
                            }
                            Poll::Failed(e) => {
                                ui.label(e.message());
                            }
                        }
                    }
                    None => {
                        let docs = listing
                            .entries
                            .iter()
                            .filter(|e| e.kind == EntryKind::File && paths::is_doc(&e.name))
                            .count();
                        let hint = if docs == 0 {
                            "No documents here yet."
                        } else {
                            "Choose a document in the sidebar."
                        };
                        ui.styled(hint, TextStyle::WEAK | TextStyle::ITALIC);
                    }
                }
            }
        }
    }

    fn read(&self, ui: &mut Ui<'_>, path: &str, user: Option<&str>, act: &mut Actions) {
        let doc = ui.rpc().get::<ReadFile>(&PathReq { path });
        self.crumbs(ui, path, user, act);
        ui.horizontal(|ui| {
            ui.push_id("mode", |ui| match &doc {
                Poll::Ready(file) if file.writable => {
                    if ui.button(&label(icons::PENCIL_SIMPLE, "Edit")).clicked() {
                        act.edit = true;
                    }
                }
                Poll::Ready(_) => {
                    ui.styled(&label(icons::LOCK_SIMPLE, "Read-only"), TextStyle::WEAK);
                    ui.tooltip("You can read this document but not change it.");
                }
                _ => {}
            });
            ui.open_url(
                &label(icons::DOWNLOAD_SIMPLE, "Download"),
                &paths::download_url(path),
            );
        });
        ui.separator();
        match doc {
            Poll::Pending => {
                ui.label("Loading…");
            }
            Poll::Failed(e) => {
                ui.label(e.message());
                if ui.button(&label(icons::ARROW_CLOCKWISE, "Retry")).clicked() {
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
                    ui.styled("• Unsaved changes", TextStyle::WEAK);
                }
            });
            ui.push_id("save", |ui| {
                if saving {
                    ui.label("Saving…");
                } else if ui.button(&label(icons::FLOPPY_DISK, "Save")).clicked() {
                    act.save = true;
                }
            });
            if ui.button(&label(icons::CHECK, "Done")).clicked() {
                act.close_editor = true;
            }
            ui.tooltip("Back to reading");
            ui.push_id("discard", |ui| {
                if dirty && ui.button(&label(icons::TRASH, "Discard changes")).clicked() {
                    act.discard = true;
                }
            });
            let toggle = if self.preview {
                label(icons::EYE_SLASH, "Hide preview")
            } else {
                label(icons::EYE, "Show preview")
            };
            if ui.button(&toggle).clicked() {
                self.preview = !self.preview;
            }
        });
        // Markdown snippets, added at the end of the document: the shell does not tell apps
        // where the cursor is.
        ui.horizontal(|ui| {
            for (icon, tip, snippet) in [
                (icons::TEXT_H, "Heading", "\n## Heading\n"),
                (icons::TEXT_B, "Bold", "**bold**"),
                (icons::TEXT_ITALIC, "Italic", "*italic*"),
                (
                    icons::LINK,
                    "Link to a page or file",
                    "[text](other-document.md)",
                ),
                (icons::LIST_BULLETS, "Bulleted list", "\n- item\n"),
                (icons::LIST_NUMBERS, "Numbered list", "\n1. item\n"),
                (icons::CHECK_SQUARE, "Task", "\n- [ ] task\n"),
                (icons::CODE_BLOCK, "Code block", "\n```\ncode\n```\n"),
            ] {
                if ui.button(icon).clicked() {
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
                                .button(&label(icons::FILE_TEXT, paths::title(&hit.path)))
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

    fn apply(&mut self, ui: &mut Ui<'_>, mut act: Actions) {
        let rpc = ui.rpc();
        if act.discard
            && let View::Edit(path) = &self.view
        {
            // Back to reading the document as it was saved.
            let path = path.clone();
            self.view = View::Read(path);
            self.editor_loaded = false;
            self.status = None;
            return;
        }
        // Leaving the editor by any route would lose unsaved changes, so it has to be asked for
        // with Save or Discard instead.
        if (act.back || act.go.is_some() || act.close_editor) && self.dirty() {
            act.back = false;
            act.go = None;
            act.close_editor = false;
            self.status =
                Some("You have unsaved changes. Save them, or discard them to leave.".to_owned());
        }
        if act.back {
            self.back();
        }
        if let Some(view) = act.go {
            self.open(view);
        }
        if let Some(folder) = act.toggle
            && !self.expanded.remove(&folder)
        {
            self.expanded.insert(folder);
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

        if let Some(path) = self.view.path() {
            self.section = section_root(path, user.as_deref());
        }

        let mut act = Actions::default();
        ui.horizontal_top(|ui| {
            ui.side_column(|ui| self.sidebar(ui, user.as_deref(), &mut act));
            ui.vertical(|ui| {
                ui.push_id("back", |ui| {
                    if !self.history.is_empty()
                        && ui.button(&label(icons::ARROW_LEFT, "Back")).clicked()
                    {
                        act.back = true;
                    }
                });
                // Under its own id, like every widget that comes and goes: an auto id shifts
                // every sibling after it, and a text field whose id moves loses what was typed.
                ui.push_id("status", |ui| {
                    if let Some(status) = &self.status {
                        ui.styled(status, TextStyle::ITALIC);
                    }
                });
                // The page scrolls by itself, separately from the sidebar. Keyed by what is open,
                // so each document starts at its top rather than where the last one was left.
                let view = self.view.clone();
                let key = match &view {
                    View::Browse(p) | View::Read(p) | View::Edit(p) => p.clone(),
                    View::Search(q) => format!("search:{q}"),
                };
                ui.push_id(&key, |ui| {
                    ui.scroll(|ui| match view {
                        View::Browse(dir) => self.browse(ui, &dir, user.as_deref(), &mut act),
                        View::Read(path) => self.read(ui, &path, user.as_deref(), &mut act),
                        View::Edit(path) => self.edit(ui, &path, &mut act),
                        View::Search(query) => Self::results(ui, &query, &mut act),
                    })
                });
            });
        });
        self.apply(ui, act);
        self.changed = before != (self.view.clone(), self.pending, self.status.clone());
    }
}

#[cfg(test)]
mod tests;

ccosel_sdk::ccosel_app!(Docs);
