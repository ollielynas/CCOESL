//! Acting on `UploadFolder` and `OpenUrl` clicks: the folder picker, the upload, the new tab.
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
use web_sys::{File, HtmlInputElement};

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
            return format!("Uploading {name}: {}/{}{skipped}", self.done, self.total);
        }
        match self.failures.len() {
            0 => format!("Uploaded {name}: {} files{skipped}", self.done),
            n => format!(
                "Uploaded {name}: {} of {} files, {n} failed{skipped}",
                self.done.saturating_sub(n),
                self.total
            ),
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

    /// Open the folder picker for a click on an `UploadFolder` button, then upload what's
    /// picked into `dest`.
    ///
    /// Browsers only open a picker in response to a user action. The desktop calls this in the
    /// same frame the click is drawn, which is well inside that allowance.
    pub fn start(&self, instance: u64, widget: u64, dest: String, ctx: egui::Context) {
        self.pick(instance, widget, Dest::Folder(dest), ctx);
    }

    /// Open the folder picker for a click on an `UploadProject` button, then upload what's
    /// picked, minus what `.gitignore` excludes, into a new temporary folder on the server.
    pub fn start_project(&self, instance: u64, widget: u64, ctx: egui::Context) {
        self.pick(instance, widget, Dest::Scratch, ctx);
    }

    fn pick(&self, instance: u64, widget: u64, dest: Dest, ctx: egui::Context) {
        let seq = self.next_seq.get();
        self.next_seq.set(seq + 1);
        let target = Target {
            seq,
            instance,
            widget,
            dest,
        };
        let project = matches!(target.dest, Dest::Scratch);
        if let Err(e) = open_picker(self.all.clone(), target, ctx) {
            self.all.borrow_mut().push(Upload {
                seq,
                instance,
                widget,
                project,
                finished: true,
                failures: vec![format!("couldn't open the folder picker: {e:?}")],
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
    ctx: egui::Context,
) -> Result<(), JsValue> {
    let document = web_sys::window()
        .and_then(|w| w.document())
        .ok_or("no document")?;
    let input: HtmlInputElement = document.create_element("input")?.dyn_into()?;
    input.set_type("file");
    input.set_attribute("webkitdirectory", "")?;
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
            wasm_bindgen_futures::spawn_local(upload_all(all, target, files, ctx));
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
/// into `all` as each one lands.
async fn upload_all(
    all: Rc<RefCell<Vec<Upload>>>,
    target: Target,
    files: Vec<File>,
    ctx: egui::Context,
) {
    let relative: Vec<String> = files.iter().map(relative_path).collect();
    let folder = relative
        .first()
        .and_then(|r| r.split('/').next())
        .unwrap_or_default()
        .to_owned();
    all.borrow_mut().push(Upload {
        seq: target.seq,
        instance: target.instance,
        widget: target.widget,
        project: matches!(target.dest, Dest::Scratch),
        folder,
        total: files.len(),
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
            update(&mut |u| {
                u.skipped = u.total - kept;
                u.total = kept;
                u.scratch = Some(id);
            });
            (files, relative, ccosel_proto::scratch::path(id))
        }
    };

    for (file, rel) in files.iter().zip(&relative) {
        let result = match upload_target(&dest, rel) {
            None => Err(format!("{rel}: not a valid path")),
            Some((dir, name)) => match read(file).await {
                Err(e) => Err(format!("{rel}: couldn't read it: {e}")),
                Ok(bytes) => fetch::post_bytes(&upload_url(&dir, &name), &bytes)
                    .await
                    .map(|_| ())
                    .map_err(|e| format!("{rel}: {e}")),
            },
        };
        update(&mut |u| {
            u.done += 1;
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
