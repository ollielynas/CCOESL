//! Acting on `UploadFolder` and `OpenUrl` clicks: the folder picker, the upload, the new tab.
//! Also files and folders dropped onto an app window, which upload exactly as if they had been
//! picked with that window's upload button.
//!
//! An app can't reach the browser, so it draws a button and the shell does the work when it's
//! clicked. The decisions (where each file goes, how its URL is encoded, which URLs an app may
//! open, what the status bar says) are plain functions at the top, tested in `upload/tests.rs`.
//! The browser calls below them are kept thin.

use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    DragEvent, File, FileSystemDirectoryEntry, FileSystemEntry, FileSystemFileEntry,
    HtmlInputElement,
};

use crate::fetch;

/// Where one picked file goes: `(directory, filename)` for `POST /upload`.
///
/// `relative` is the browser's `webkitRelativePath`, which starts with the picked folder's own
/// name (`photos/2024/a.jpg`). That name is kept, so uploading `photos` into `/Documents`
/// creates `/Documents/photos`, the way copying a folder anywhere else behaves, rather than
/// spilling its contents into `/Documents`.
///
/// `None` for a path that is empty or has an empty, `.` or `..` part: the server would refuse
/// it anyway, and refusing here keeps it out of the request entirely.
pub fn upload_target(dest: &str, relative: &str) -> Option<(String, String)> {
    let parts: Vec<&str> = relative.split('/').collect();
    if parts
        .iter()
        .any(|p| p.is_empty() || *p == "." || *p == "..")
    {
        return None;
    }
    let (filename, dirs) = parts.split_last()?;
    let mut dir = dest.trim_end_matches('/').to_owned();
    for d in dirs {
        dir.push('/');
        dir.push_str(d);
    }
    if dir.is_empty() {
        dir.push('/');
    }
    Some((dir, (*filename).to_owned()))
}

/// `POST /upload?path=..&filename=..` for one file, each value percent-encoded on its own so a
/// space, `#`, `?`, `&`, `%` or non-ASCII name reaches the server byte for byte.
pub fn upload_url(dir: &str, filename: &str) -> String {
    let mut url = "/upload?path=".to_owned();
    percent_encode(dir, &mut url);
    url.push_str("&filename=");
    percent_encode(filename, &mut url);
    url
}

/// Everything but RFC 3986's unreserved characters, byte by byte over the UTF-8.
fn percent_encode(s: &str, out: &mut String) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    for &b in s.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push('%');
            out.push(HEX[usize::from(b >> 4)] as char);
            out.push(HEX[usize::from(b & 0xF)] as char);
        }
    }
}

/// Which of `paths` a project upload sends: everything except what the `.gitignore` files among
/// them exclude, and anything inside `.git/` (the repository itself, never wanted for a build
/// and often the biggest thing in the folder). `gitignores` are `(path, contents)` of those
/// files; the result lines up with `paths`.
pub fn project_files(paths: &[String], gitignores: &[(String, String)]) -> Vec<bool> {
    let mut sorted: Vec<&(String, String)> = gitignores.iter().collect();
    // Shallowest first: deeper files take precedence, and the filter gives later rules it.
    sorted.sort_by_key(|(p, _)| p.matches('/').count());
    let mut filter = ccosel_gitignore::Filter::new();
    for (path, contents) in sorted {
        let base = path.rsplit_once('/').map_or("", |(dir, _)| dir);
        filter.add(base, contents);
    }
    paths
        .iter()
        .map(|p| !p.split('/').any(|part| part == ".git") && !filter.is_ignored(p))
        .collect()
}

/// A dropped file or folder, read into a tree. Generic over the file handle so flattening it
/// can be tested without a browser.
#[derive(Debug, PartialEq)]
pub enum Entry<F> {
    File(String, F),
    Dir(String, Vec<Entry<F>>),
}

/// Every file under `entries`, with its path relative to where they were dropped:
/// `photos/2024/a.jpg` for a dropped `photos` folder, `a.jpg` for a dropped file. That is the
/// same shape as a picked folder's `webkitRelativePath`, so both go through [`upload_target`].
/// Empty folders contribute nothing: there is no file to upload.
pub fn flatten<F>(entries: Vec<Entry<F>>) -> Vec<(String, F)> {
    fn walk<F>(prefix: &str, entries: Vec<Entry<F>>, out: &mut Vec<(String, F)>) {
        for entry in entries {
            match entry {
                Entry::File(name, f) => out.push((format!("{prefix}{name}"), f)),
                Entry::Dir(name, children) => walk(&format!("{prefix}{name}/"), children, out),
            }
        }
    }
    let mut out = Vec::new();
    walk("", entries, &mut out);
    out
}

/// What an upload is called in the status bar: the folder or file, when one thing was chosen,
/// otherwise how many.
pub fn upload_label(relative: &[String]) -> String {
    let mut tops: Vec<&str> = relative
        .iter()
        .filter_map(|r| r.split('/').next())
        .collect();
    tops.sort_unstable();
    tops.dedup();
    match tops.as_slice() {
        [] => "folder".to_owned(),
        [one] => (*one).to_owned(),
        many => format!("{} items", many.len()),
    }
}

/// Why a file of `size` bytes won't be sent, if it won't. Checked before the file is read, so an
/// oversized file never lands in memory. See [`ccosel_proto::upload`] for why this is a cap
/// rather than chunking.
pub fn refuse_size(size: u64) -> Option<String> {
    let max = ccosel_proto::upload::MAX_FILE_BYTES;
    (size > max).then(|| {
        format!(
            "{} is over the {} upload limit",
            human_bytes(size),
            human_bytes(max)
        )
    })
}

/// A byte count for people: `512 B`, `12.3 KB`, `4.0 MB`, `1.2 GB`.
pub fn human_bytes(n: u64) -> String {
    const UNITS: [&str; 3] = ["KB", "MB", "GB"];
    if n < 1024 {
        return format!("{n} B");
    }
    let mut value = n as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

/// Where a drop on a window goes: the same place as a click on that window's upload button.
/// A window with several goes to the first; one with none can't take a drop.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DropTarget {
    /// An `UploadFolder` button: its widget id and destination folder.
    Folder(u64, String),
    /// An `UploadProject` button: its widget id.
    Project(u64),
}

pub fn drop_target(folders: &[(u64, String)], projects: &[u64]) -> Option<DropTarget> {
    folders
        .first()
        .map(|(id, dest)| DropTarget::Folder(*id, dest.clone()))
        .or_else(|| projects.first().map(|&id| DropTarget::Project(id)))
}

/// Whether the shell may open this `OpenUrl` target in a new tab.
///
/// The URL comes from an app, and an app is untrusted code, so this is deliberately narrow: a
/// same-origin path (one leading `/`, never the protocol-relative `//host`, which is another
/// origin) or a full `http://` / `https://` URL. `javascript:`, `data:`, `file:`, every other
/// scheme and anything else is refused.
pub fn may_open(url: &str) -> bool {
    // Browsers strip leading whitespace and control characters before parsing, so a check
    // that didn't would be fooled by " javascript:..".
    let url = url.trim_start_matches(|c: char| c.is_whitespace() || c.is_control());
    if url.chars().any(|c| c.is_control()) {
        return false;
    }
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return true;
    }
    // `/\host` is also read as protocol-relative by browsers.
    url.starts_with('/') && !url.starts_with("//") && !url.starts_with("/\\")
}

/// One folder upload, from the click to its last file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Upload {
    /// Unique per upload, so the async task finds its own entry however the list changes.
    pub seq: u64,
    /// The window and the `UploadFolder` button it came from, to tell when it's finished.
    pub instance: u64,
    pub widget: u64,
    /// The picked folder's name, for the status bar. Empty until the picker returns.
    pub folder: String,
    pub done: usize,
    pub total: usize,
    /// Bytes of the files done so far, sent or failed, and of all of them. Advances a file at a
    /// time: `fetch` reports no progress within one request.
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub failures: Vec<String>,
    pub finished: bool,
    /// A project upload (`UploadProject`) rather than a folder upload (`UploadFolder`).
    pub project: bool,
    /// Project uploads: files left out by `.gitignore` or for being in `.git/`.
    pub skipped: usize,
    /// Project uploads: the temporary folder it went into, once there is one.
    pub scratch: Option<u32>,
}

impl Upload {
    /// What the status bar says about it.
    pub fn status(&self) -> String {
        let name = if self.folder.is_empty() {
            "folder"
        } else {
            self.folder.as_str()
        };
        let skipped = match self.skipped {
            0 => String::new(),
            n => format!(", {n} ignored"),
        };
        if !self.finished {
            return format!(
                "Uploading {name}: {}/{} files, {} of {}{skipped}",
                self.done,
                self.total,
                human_bytes(self.bytes_done),
                human_bytes(self.bytes_total)
            );
        }
        match self.failures.len() {
            0 => format!(
                "Uploaded {name}: {} files, {}{skipped}",
                self.done,
                human_bytes(self.bytes_total)
            ),
            n => format!(
                "Uploaded {name}: {} of {} files, {n} failed{skipped}",
                self.done.saturating_sub(n),
                self.total
            ),
        }
    }

    /// How far along it is, from 0 to 1: by bytes, or by files while no sizes are known yet
    /// (before the picker returns, or for a folder of empty files).
    pub fn fraction(&self) -> f32 {
        if self.finished {
            1.0
        } else if self.bytes_total > 0 {
            (self.bytes_done as f64 / self.bytes_total as f64) as f32
        } else if self.total > 0 {
            self.done as f32 / self.total as f32
        } else {
            0.0
        }
    }
}

/// Uploads in flight or just finished. Shared with the async upload tasks, which update it as
/// each file lands; the desktop reads it once a frame.
#[derive(Clone, Default)]
pub struct Uploads {
    all: Rc<RefCell<Vec<Upload>>>,
    next_seq: Rc<std::cell::Cell<u64>>,
}

impl Uploads {
    /// Uploads that finished since the last call. The desktop tells each one's window, and
    /// turns failures into errors the user can see.
    pub fn take_finished(&self) -> Vec<Upload> {
        let mut all = self.all.borrow_mut();
        let (finished, running) = all.drain(..).partition(|u| u.finished);
        *all = running;
        finished
    }

    /// Status-bar text for whatever is running, if anything.
    pub fn status(&self) -> Option<String> {
        let all = self.all.borrow();
        match all.as_slice() {
            [] => None,
            [one] => Some(one.status()),
            many => Some(format!("{} uploads running", many.len())),
        }
    }

    /// How far along everything running is, for the status bar's progress bar.
    pub fn progress(&self) -> Option<f32> {
        let all = self.all.borrow();
        if all.is_empty() {
            return None;
        }
        let (done, total) = all
            .iter()
            .fold((0, 0), |(d, t), u| (d + u.bytes_done, t + u.bytes_total));
        Some(if total > 0 {
            (done as f64 / total as f64) as f32
        } else {
            all.iter().map(Upload::fraction).sum::<f32>() / all.len() as f32
        })
    }

    /// Upload what was dropped on a window, as if it had been picked with that window's
    /// `target` button.
    pub fn start_dropped(
        &self,
        instance: u64,
        target: DropTarget,
        entries: Vec<FileSystemEntry>,
        ctx: egui::Context,
    ) {
        let target = self.target(instance, target);
        let all = self.all.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let mut tree = Vec::new();
            let mut unreadable = Vec::new();
            for entry in entries {
                match read_entry(entry.clone()).await {
                    Ok(e) => tree.push(e),
                    Err(e) => unreadable.push(format!("{}: couldn't read it: {e}", entry.name())),
                }
            }
            let (relative, files): (Vec<String>, Vec<File>) = flatten(tree).into_iter().unzip();
            upload_all(all, target, files, relative, unreadable, ctx).await;
        });
    }

    fn target(&self, instance: u64, target: DropTarget) -> Target {
        let seq = self.next_seq.get();
        self.next_seq.set(seq + 1);
        let (widget, dest) = match target {
            DropTarget::Folder(widget, dest) => (widget, Dest::Folder(dest)),
            DropTarget::Project(widget) => (widget, Dest::Scratch),
        };
        Target {
            seq,
            instance,
            widget,
            dest,
        }
    }

    /// Open the picker for a click on an `UploadFolder` button, or with `files` an
    /// `UploadFiles` one, then upload what's picked into `dest`.
    ///
    /// Browsers only open a picker in response to a user action. The desktop calls this in the
    /// same frame the click is drawn, which is well inside that allowance.
    pub fn start(&self, instance: u64, widget: u64, dest: String, files: bool, ctx: egui::Context) {
        self.pick(instance, widget, Dest::Folder(dest), files, ctx);
    }

    /// Open the folder picker for a click on an `UploadProject` button, then upload what's
    /// picked, minus what `.gitignore` excludes, into a new temporary folder on the server.
    pub fn start_project(&self, instance: u64, widget: u64, ctx: egui::Context) {
        self.pick(instance, widget, Dest::Scratch, false, ctx);
    }

    /// `files`: a picker that chooses files, rather than one folder.
    fn pick(&self, instance: u64, widget: u64, dest: Dest, files: bool, ctx: egui::Context) {
        let target = self.target(
            instance,
            match dest {
                Dest::Folder(dest) => DropTarget::Folder(widget, dest),
                Dest::Scratch => DropTarget::Project(widget),
            },
        );
        let (seq, project) = (target.seq, matches!(target.dest, Dest::Scratch));
        if let Err(e) = open_picker(self.all.clone(), target, files, ctx) {
            self.all.borrow_mut().push(Upload {
                seq,
                instance,
                widget,
                project,
                finished: true,
                failures: vec![format!("couldn't open the file picker: {e:?}")],
                ..Default::default()
            });
        }
    }
}

/// Which button an upload came from, and where it goes.
struct Target {
    seq: u64,
    instance: u64,
    widget: u64,
    dest: Dest,
}

enum Dest {
    /// Into this jail folder, everything picked.
    Folder(String),
    /// Into a new temporary folder, minus what `.gitignore` excludes.
    Scratch,
}

fn open_picker(
    all: Rc<RefCell<Vec<Upload>>>,
    target: Target,
    files: bool,
    ctx: egui::Context,
) -> Result<(), JsValue> {
    let document = web_sys::window()
        .and_then(|w| w.document())
        .ok_or("no document")?;
    let input: HtmlInputElement = document.create_element("input")?.dyn_into()?;
    input.set_type("file");
    if !files {
        input.set_attribute("webkitdirectory", "")?;
    }
    input.set_attribute("multiple", "")?;
    input.set_attribute("style", "display:none")?;
    // In the page rather than detached: some browsers ignore `click()` on a detached input.
    document.body().ok_or("no body")?.append_child(&input)?;

    let picked = input.clone();
    let on_change = Closure::once(move || {
        picked.remove();
        let files: Vec<File> = picked
            .files()
            .map(|list| (0..list.length()).filter_map(|i| list.get(i)).collect())
            .unwrap_or_default();
        if !files.is_empty() {
            let relative = files.iter().map(relative_path).collect();
            wasm_bindgen_futures::spawn_local(upload_all(
                all,
                target,
                files,
                relative,
                Vec::new(),
                ctx,
            ));
        }
    });
    input.add_event_listener_with_callback("change", on_change.as_ref().unchecked_ref())?;
    // Fires once, when the picker closes with a choice. The closure lives until then; a
    // picker closed with Cancel leaks this one small closure, which is the cost of not
    // tracking a DOM node's lifetime from Rust.
    on_change.forget();

    let cancelled = input.clone();
    let on_cancel = Closure::once(move || cancelled.remove());
    input.add_event_listener_with_callback("cancel", on_cancel.as_ref().unchecked_ref())?;
    on_cancel.forget();

    input.click();
    Ok(())
}

/// Upload `files` one at a time, so memory holds one file's bytes at a time, reporting progress
/// into `all` as each one lands. `relative` lines up with `files`; `failures` are problems found
/// before the upload started (a dropped entry that couldn't be read), reported with the rest.
async fn upload_all(
    all: Rc<RefCell<Vec<Upload>>>,
    target: Target,
    files: Vec<File>,
    relative: Vec<String>,
    failures: Vec<String>,
    ctx: egui::Context,
) {
    all.borrow_mut().push(Upload {
        seq: target.seq,
        instance: target.instance,
        widget: target.widget,
        project: matches!(target.dest, Dest::Scratch),
        folder: upload_label(&relative),
        total: files.len(),
        bytes_total: total_size(&files),
        failures,
        ..Default::default()
    });
    let update = |f: &mut dyn FnMut(&mut Upload)| {
        if let Some(u) = all.borrow_mut().iter_mut().find(|u| u.seq == target.seq) {
            f(u);
        }
        ctx.request_repaint();
    };
    ctx.request_repaint();

    let (files, relative, dest) = match target.dest {
        Dest::Folder(dest) => (files, relative, dest),
        Dest::Scratch => {
            let (files, relative) = match only_project_files(files, relative).await {
                Ok(kept) => kept,
                Err(e) => {
                    update(&mut |u| {
                        u.failures.push(e.clone());
                        u.finished = true;
                    });
                    return;
                }
            };
            let id = match new_scratch().await {
                Ok(id) => id,
                Err(e) => {
                    update(&mut |u| {
                        u.failures
                            .push(format!("couldn't make a folder to upload into: {e}"));
                        u.finished = true;
                    });
                    return;
                }
            };
            let kept = files.len();
            let bytes = total_size(&files);
            update(&mut |u| {
                u.skipped = u.total - kept;
                u.total = kept;
                u.bytes_total = bytes;
                u.scratch = Some(id);
            });
            (files, relative, ccosel_proto::scratch::path(id))
        }
    };

    for (file, rel) in files.iter().zip(&relative) {
        let size = file_size(file);
        let result = match (upload_target(&dest, rel), refuse_size(size)) {
            (None, _) => Err(format!("{rel}: not a valid path")),
            (_, Some(why)) => Err(format!("{rel}: {why}")),
            (Some((dir, name)), None) => match read(file).await {
                Err(e) => Err(format!("{rel}: couldn't read it: {e}")),
                Ok(bytes) => fetch::post_bytes(&upload_url(&dir, &name), &bytes)
                    .await
                    .map(|_| ())
                    .map_err(|e| format!("{rel}: {e}")),
            },
        };
        update(&mut |u| {
            u.done += 1;
            u.bytes_done += size;
            if let Err(e) = &result {
                u.failures.push(e.clone());
            }
        });
    }
    update(&mut |u| u.finished = true);
}

/// Drop what [`project_files`] leaves out. Reads every `.gitignore` among `files` first,
/// which is the only reading done before the upload proper.
async fn only_project_files(
    files: Vec<File>,
    relative: Vec<String>,
) -> Result<(Vec<File>, Vec<String>), String> {
    let mut gitignores = Vec::new();
    for (file, rel) in files.iter().zip(&relative) {
        if rel.rsplit('/').next() == Some(".gitignore") {
            let bytes = read(file)
                .await
                .map_err(|e| format!("{rel}: couldn't read it: {e}"))?;
            gitignores.push((rel.clone(), String::from_utf8_lossy(&bytes).into_owned()));
        }
    }
    let keep = project_files(&relative, &gitignores);
    Ok(files
        .into_iter()
        .zip(relative)
        .zip(keep)
        .filter_map(|(pair, keep)| keep.then_some(pair))
        .unzip())
}

/// `POST /scratch`: a new temporary folder on the server, as its id.
async fn new_scratch() -> Result<u32, String> {
    let body = fetch::post_bytes("/scratch", &[])
        .await
        .map_err(|e| e.to_string())?;
    String::from_utf8_lossy(&body)
        .trim()
        .parse()
        .map_err(|_| "the server's answer wasn't a folder id".to_owned())
}

/// `File.webkitRelativePath`, which `web-sys` doesn't bind.
fn relative_path(file: &File) -> String {
    js_sys::Reflect::get(file, &"webkitRelativePath".into())
        .ok()
        .and_then(|v| v.as_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| file.name())
}

fn file_size(file: &File) -> u64 {
    file.size() as u64
}

fn total_size(files: &[File]) -> u64 {
    files.iter().map(file_size).sum()
}

/// Read a dropped entry, and everything under it if it's a folder, into an [`Entry`] tree.
async fn read_entry(entry: FileSystemEntry) -> Result<Entry<File>, String> {
    let name = entry.name();
    if entry.is_file() {
        let entry: FileSystemFileEntry = entry.unchecked_into();
        let file = callback_promise(|ok, err| {
            entry.file_with_callback_and_callback(ok, err);
            Ok(())
        })
        .await?
        .unchecked_into();
        return Ok(Entry::File(name, file));
    }
    let dir: FileSystemDirectoryEntry = entry.unchecked_into();
    let reader = dir.create_reader();
    let mut children = Vec::new();
    // `readEntries` hands a folder over in batches (100 at a time in Chrome), and an empty
    // batch means the end.
    loop {
        let batch: js_sys::Array =
            callback_promise(|ok, err| reader.read_entries_with_callback_and_callback(ok, err))
                .await?
                .unchecked_into();
        if batch.length() == 0 {
            break;
        }
        for child in batch.iter() {
            // Boxed: an async fn can't recurse into itself directly.
            let child = Box::pin(read_entry(child.unchecked_into())).await?;
            children.push(child);
        }
    }
    Ok(Entry::Dir(name, children))
}

/// Await one of the callback-style File System Entry APIs. One that throws instead of calling
/// back rejects, the same as one that calls its error callback.
async fn callback_promise(
    call: impl Fn(&js_sys::Function, &js_sys::Function) -> Result<(), JsValue>,
) -> Result<JsValue, String> {
    let promise = js_sys::Promise::new(&mut |ok, err: js_sys::Function| {
        if let Err(thrown) = call(&ok, &err) {
            let _ = err.call1(&JsValue::NULL, &thrown);
        }
    });
    JsFuture::from(promise).await.map_err(|e| format!("{e:?}"))
}

/// Things dropped on the page, waiting for the desktop to route them to the window underneath.
#[derive(Clone, Default)]
pub struct Drops {
    queue: Rc<RefCell<Vec<Dropped>>>,
}

/// One drop: where it landed, in egui points, and the entries it carried.
pub struct Dropped {
    pub pos: egui::Pos2,
    pub entries: Vec<FileSystemEntry>,
}

impl Drops {
    pub fn take(&self) -> Vec<Dropped> {
        std::mem::take(&mut *self.queue.borrow_mut())
    }

    /// Listen for drops on the page. Registered on the window in the capture phase so it runs
    /// before eframe's own handler, which it then stops: eframe would read every dropped file
    /// into memory at once, and can't see into folders anyway.
    pub fn listen(&self, ctx: egui::Context) -> Result<(), JsValue> {
        let window = web_sys::window().ok_or("no window")?;
        let options = web_sys::AddEventListenerOptions::new();
        options.set_capture(true);

        let on_over = Closure::<dyn FnMut(DragEvent)>::new(|event: DragEvent| {
            // Without this the browser opens the dropped file itself instead of dropping it.
            event.prevent_default();
            event.stop_propagation();
            if let Some(dt) = event.data_transfer() {
                dt.set_drop_effect("copy");
            }
        });
        window.add_event_listener_with_callback_and_add_event_listener_options(
            "dragover",
            on_over.as_ref().unchecked_ref(),
            &options,
        )?;
        on_over.forget();

        let queue = self.queue.clone();
        let on_drop = Closure::<dyn FnMut(DragEvent)>::new(move |event: DragEvent| {
            event.prevent_default();
            event.stop_propagation();
            // The entries must be taken now: a drop's data is gone once its handler returns.
            let Some(items) = event.data_transfer().map(|dt| dt.items()) else {
                return;
            };
            let entries: Vec<FileSystemEntry> = (0..items.length())
                .filter_map(|i| items.get(i))
                .filter(|item| item.kind() == "file")
                .filter_map(|item| item.webkit_get_as_entry().ok().flatten())
                .collect();
            if entries.is_empty() {
                return;
            }
            // Client pixels to egui points. The canvas fills the page from its top-left corner.
            let zoom = ctx.zoom_factor();
            let pos = egui::pos2(
                event.client_x() as f32 / zoom,
                event.client_y() as f32 / zoom,
            );
            queue.borrow_mut().push(Dropped { pos, entries });
            ctx.request_repaint();
        });
        window.add_event_listener_with_callback_and_add_event_listener_options(
            "drop",
            on_drop.as_ref().unchecked_ref(),
            &options,
        )?;
        // For the life of the page, like the shell itself.
        on_drop.forget();
        Ok(())
    }
}

async fn read(file: &File) -> Result<Vec<u8>, String> {
    let buffer = JsFuture::from(file.array_buffer())
        .await
        .map_err(|e| format!("{e:?}"))?;
    Ok(js_sys::Uint8Array::new(&buffer).to_vec())
}

/// Open an app's `OpenUrl` target in a new tab, if [`may_open`] allows it.
pub fn open_url(url: &str) -> Result<(), String> {
    if !may_open(url) {
        return Err(format!(
            "refused to open {url:?}: only http(s) and same-site links"
        ));
    }
    let window = web_sys::window().ok_or("no window")?;
    // `noopener`: the new tab gets no handle back to this page.
    window
        .open_with_url_and_target_and_features(url, "_blank", "noopener")
        .map_err(|e| format!("couldn't open {url}: {e:?}"))?;
    Ok(())
}

#[cfg(test)]
mod tests;
