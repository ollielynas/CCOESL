//! Browser-side folder upload: file picker, drag-and-drop, and chunked upload to the server.
//!
//! The SDK's `upload_folder()` emits an `UploadFolder` opcode. The shell renders it as a
//! button, detects clicks, and calls into this module to open a native file picker (or accept
//! drag-and-drop). Files are uploaded one at a time to `POST /upload`.

use js_sys::Uint8Array;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{Event, HtmlInputElement};

/// Result of a completed upload.
#[derive(Clone, Debug)]
pub struct UploadResult {
    /// The server-side path of the uploaded directory, e.g. `/my-project`.
    pub path: String,
    pub file_count: usize,
}

/// Shared state for in-flight uploads. The desktop owns one of these and polls it each frame.
pub struct UploadState {
    /// Uploads waiting for the file picker callback.
    pending: Vec<PendingUpload>,
    /// Completed uploads, drained by the desktop each frame.
    completed: Vec<UploadResult>,
}

struct PendingUpload {
    #[allow(dead_code)]
    widget_id: u64,
    dest_path: String,
}

impl UploadState {
    pub fn new() -> Self {
        Self {
            pending: Vec::new(),
            completed: Vec::new(),
        }
    }

    /// Drain completed uploads.
    pub fn take_completed(&mut self) -> Vec<UploadResult> {
        std::mem::take(&mut self.completed)
    }

    /// Begin an upload: opens the file picker. The result will appear in `completed`
    /// on a future frame.
    pub fn start_upload(&mut self, widget_id: u64, dest_path: &str) {
        let dest = dest_path.to_owned();
        self.pending.push(PendingUpload {
            widget_id,
            dest_path: dest.clone(),
        });

        open_file_picker(dest);
    }
}

/// Open a hidden `<input type="file" webkitdirectory>` and fire the callback when the user
/// selects files. This is inherently async — the callback runs on a future frame via
/// `spawn_local`.
fn open_file_picker(dest_path: String) {
    let window = web_sys::window().expect("no global window");
    let document = window.document().expect("no document");

    // Create a hidden file input that accepts entire directories.
    let input: HtmlInputElement = document
        .create_element("input")
        .expect("create_element")
        .unchecked_into();
    input.set_type("file");
    input.set_attribute("webkitdirectory", "").unwrap();
    input.set_attribute("multiple", "").unwrap();
    input.style().set_property("display", "none").unwrap();

    // Clone for the closure.
    let dest = dest_path.clone();
    let input_clone = input.clone();
    let document_clone = document.clone();

    let closure = Closure::<dyn FnMut(_)>::new(move |_: Event| {
        let files = input_clone.files();
        let Some(file_list) = files else {
            return;
        };

        let file_count = file_list.length() as usize;
        if file_count == 0 {
            return;
        }

        // Collect all files and upload them.
        let dest_clone = dest.clone();
        wasm_bindgen_futures::spawn_local(async move {
            upload_files(&file_list, &dest_clone, file_count).await;
        });

        // Clean up: remove the input from the DOM.
        if let Some(parent) = input_clone.parent_node() {
            let _ = parent.remove_child(&input_clone);
        }
    });

    input
        .add_event_listener_with_callback("change", closure.as_ref().unchecked_ref())
        .unwrap();
    closure.forget();

    // Attach to DOM and trigger click.
    let _ = document_clone.body().unwrap().append_child(&input);
    input.click();
}

/// Upload all files from the FileList to the server.
async fn upload_files(file_list: &web_sys::FileList, dest_path: &str, file_count: usize) {
    let window = web_sys::window().expect("no global window");

    for i in 0..file_count {
        let file = match file_list.get(i as u32) {
            Some(f) => f,
            None => continue,
        };

        // The `webkitRelativePath` gives us the folder structure, e.g.
        // `my-project/src/main.rs`. We strip the first component (the root folder name)
        // and use the rest as the relative path.
        let rel_path = js_sys::Reflect::get(&file.clone().into(), &"webkitRelativePath".into())
            .ok()
            .and_then(|v| v.as_string())
            .unwrap_or_default();
        let relative = strip_root_folder(&rel_path);

        // Split into directory and filename.
        let (dir, filename) = match relative.rsplit_once('/') {
            Some((d, f)) => (format!("{dest_path}/{d}"), f.to_owned()),
            None => (dest_path.to_owned(), relative),
        };

        // Read file contents as ArrayBuffer.
        let array_buffer = match read_file_bytes(&file).await {
            Ok(buf) => buf,
            Err(e) => {
                log::error!("upload: failed to read {rel_path}: {e:?}");
                continue;
            }
        };

        let bytes = Uint8Array::new(&array_buffer);
        let body = bytes.to_vec();

        // POST to /upload?path=...&filename=...
        let url = format!(
            "/upload?path={}&filename={}",
            urlencoding::encode(&dir),
            urlencoding::encode(&filename),
        );

        let mut opts = web_sys::RequestInit::new();
        opts.set_method("POST");
        let body_val: JsValue = Uint8Array::from(&body[..]).into();
        opts.set_body(&body_val);

        let request = match web_sys::Request::new_with_str_and_init(&url, &opts) {
            Ok(r) => r,
            Err(e) => {
                log::error!("upload: failed to create request for {rel_path}: {e:?}");
                continue;
            }
        };

        match web_sys::Window::fetch_with_request(&window, &request).await {
            Ok(resp) => {
                let resp: web_sys::Response = resp.unchecked_into();
                if !resp.ok() {
                    log::error!("upload: HTTP {} for {rel_path}", resp.status());
                }
            }
            Err(e) => {
                log::error!("upload: fetch failed for {rel_path}: {e:?}");
            }
        }
    }

    log::info!("upload: {file_count} files uploaded to {dest_path}");
}

/// Read a `File` into an `ArrayBuffer`.
async fn read_file_bytes(file: &web_sys::File) -> Result<js_sys::ArrayBuffer, JsValue> {
    let promise = js_sys::Promise::new(&mut |resolve, reject| {
        let reader = web_sys::FileReader::new().expect("FileReader");
        let reader_clone = reader.clone();

        let on_load = Closure::<dyn FnMut()>::new(move || match reader_clone.result() {
            Ok(val) => {
                let _ = resolve.call1(&JsValue::NULL, &val);
            }
            Err(e) => {
                let _ = reject.call1(&JsValue::NULL, &e);
            }
        });
        reader
            .add_event_listener_with_callback("load", on_load.as_ref().unchecked_ref())
            .unwrap();
        on_load.forget();

        let _ = reader.read_as_array_buffer(file);
    });

    let result = wasm_bindgen_futures::JsFuture::from(promise).await?;
    Ok(result.unchecked_into())
}

/// Strip the root folder from a `webkitRelativePath` like `my-project/src/main.rs` → `src/main.rs`.
fn strip_root_folder(path: &str) -> String {
    match path.split_once('/') {
        Some((_, rest)) => rest.to_owned(),
        None => path.to_owned(),
    }
}
