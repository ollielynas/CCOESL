//! The images apps show with `Ui::image`, fetched and decoded by the shell.
//!
//! egui asks this loader for each image a replayed frame draws. Fetching is plain `fetch` of a
//! path on this server, so the browser's HTTP cache and the session cookie both apply, and the
//! pixels never pass through an app's memory. The host only ever hands over same-origin paths
//! (`ccosel_host::same_origin`); this checks again rather than trusting that.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use egui::load::{ImageLoadResult, ImageLoader, ImagePoll, LoadError, SizeHint};

/// Decoded images kept at once. An app that shows more (a long list of thumbnails) has the
/// oldest fetched again, from the browser's cache, if it comes back into view.
const MAX_IMAGES: usize = 32;

/// The longest side kept, so one huge image can't exceed the GPU's texture limit.
const MAX_SIDE: usize = 4096;

enum Entry {
    Pending,
    Ready(Arc<egui::ColorImage>),
    Failed(String),
}

#[derive(Default)]
struct State {
    entries: HashMap<String, Entry>,
    /// Oldest first, for dropping past `MAX_IMAGES`.
    order: VecDeque<String>,
}

#[derive(Default)]
pub struct Images {
    state: Arc<Mutex<State>>,
}

pub fn install(ctx: &egui::Context) {
    ctx.add_image_loader(Arc::new(Images::default()));
}

impl ImageLoader for Images {
    fn id(&self) -> &str {
        "ccosel::images"
    }

    fn load(&self, ctx: &egui::Context, uri: &str, _: SizeHint) -> ImageLoadResult {
        if !ccosel_host::same_origin(uri) {
            return Err(LoadError::NotSupported);
        }
        let mut state = self.state.lock().unwrap();
        match state.entries.get(uri) {
            Some(Entry::Ready(image)) => {
                return Ok(ImagePoll::Ready {
                    image: image.clone(),
                });
            }
            Some(Entry::Pending) => return Ok(ImagePoll::Pending { size: None }),
            Some(Entry::Failed(e)) => return Err(LoadError::Loading(e.clone())),
            None => {}
        }
        state.entries.insert(uri.to_owned(), Entry::Pending);
        state.order.push_back(uri.to_owned());
        while state.order.len() > MAX_IMAGES {
            if let Some(old) = state.order.pop_front() {
                state.entries.remove(&old);
            }
        }
        drop(state);

        let shared = self.state.clone();
        let ctx = ctx.clone();
        let uri = uri.to_owned();
        wasm_bindgen_futures::spawn_local(async move {
            let entry = match crate::fetch::get_bytes(&uri).await {
                Ok(bytes) => match crate::background::decode_image(&bytes, MAX_SIDE) {
                    Ok(image) => Entry::Ready(Arc::new(image)),
                    Err(e) => Entry::Failed(e),
                },
                Err(e) => Entry::Failed(e),
            };
            // Unless it was forgotten, or pushed out, while it loaded.
            if let Some(slot) = shared.lock().unwrap().entries.get_mut(&uri) {
                *slot = entry;
            }
            ctx.request_repaint();
        });
        Ok(ImagePoll::Pending { size: None })
    }

    fn forget(&self, uri: &str) {
        let mut state = self.state.lock().unwrap();
        state.entries.remove(uri);
        state.order.retain(|u| u != uri);
    }

    fn forget_all(&self) {
        let mut state = self.state.lock().unwrap();
        state.entries.clear();
        state.order.clear();
    }

    fn byte_size(&self) -> usize {
        let state = self.state.lock().unwrap();
        state
            .entries
            .values()
            .map(|e| match e {
                Entry::Ready(image) => image.pixels.len() * 4,
                _ => 0,
            })
            .sum()
    }

    fn has_pending(&self) -> bool {
        let state = self.state.lock().unwrap();
        state.entries.values().any(|e| matches!(e, Entry::Pending))
    }
}
