//! Loads the images apps draw (`Ui::image`) by having the *browser* fetch and decode them.
//!
//! The browser already carries a decoder for every format it shows — JPEG, PNG, GIF, WebP,
//! AVIF, BMP, and HEIC in Safari — so decoding there costs the shell nothing, where linking
//! decoders would cost it hundreds of kilobytes per format. The fetch goes through the browser's
//! HTTP cache, with the session cookie, like any other request from the page.
//!
//! The decoded pixels are read back through a canvas, scaled down first so a phone photo
//! doesn't become a 200 MB texture.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use egui::ColorImage;
use egui::load::{ImageLoadResult, ImageLoader, ImagePoll, LoadError, SizeHint};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;

/// The longest side an image is kept at. Plenty for a window, and a bound on memory: one
/// 4096×4096 image is 64 MB of RGBA.
const MAX_SIDE: u32 = 4096;

enum Entry {
    Pending,
    Ready(Arc<ColorImage>),
    Failed(String),
}

#[derive(Default)]
pub struct BrowserImageLoader {
    cache: Arc<Mutex<HashMap<String, Entry>>>,
}

/// Whether `uri` is one this loader fetches: a URL on this server. Anything else (an `http://`
/// image elsewhere, `bytes://` from egui itself) is left to other loaders.
fn ours(uri: &str) -> bool {
    uri.starts_with('/') && !uri.starts_with("//")
}

impl ImageLoader for BrowserImageLoader {
    fn id(&self) -> &str {
        "ccosel::BrowserImageLoader"
    }

    fn load(&self, ctx: &egui::Context, uri: &str, _: SizeHint) -> ImageLoadResult {
        if !ours(uri) {
            return Err(LoadError::NotSupported);
        }
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        match cache.get(uri) {
            Some(Entry::Ready(image)) => {
                return Ok(ImagePoll::Ready {
                    image: image.clone(),
                });
            }
            Some(Entry::Failed(e)) => return Err(LoadError::Loading(e.clone())),
            Some(Entry::Pending) => return Ok(ImagePoll::Pending { size: None }),
            None => {}
        }
        cache.insert(uri.to_owned(), Entry::Pending);
        drop(cache);

        let (cache, ctx, uri) = (self.cache.clone(), ctx.clone(), uri.to_owned());
        wasm_bindgen_futures::spawn_local(async move {
            let entry = match decode(&uri).await {
                Ok(image) => Entry::Ready(Arc::new(image)),
                Err(e) => Entry::Failed(e),
            };
            cache
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(uri, entry);
            // Finished outside the frame loop, so nothing would redraw on its own.
            ctx.request_repaint();
        });
        Ok(ImagePoll::Pending { size: None })
    }

    fn forget(&self, uri: &str) {
        self.cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(uri);
    }

    fn forget_all(&self) {
        self.cache.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    fn byte_size(&self) -> usize {
        self.cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .map(|e| match e {
                Entry::Ready(image) => image.pixels.len() * 4,
                _ => 0,
            })
            .sum()
    }

    fn has_pending(&self) -> bool {
        self.cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .any(|e| matches!(e, Entry::Pending))
    }
}

/// The size an image of `w`×`h` is kept at: as it is, or shrunk to fit [`MAX_SIDE`].
pub fn kept_size(w: u32, h: u32) -> (u32, u32) {
    let longest = w.max(h);
    if longest <= MAX_SIDE {
        return (w, h);
    }
    let scale = f64::from(MAX_SIDE) / f64::from(longest);
    let fit = |n: u32| ((f64::from(n) * scale).round() as u32).max(1);
    (fit(w), fit(h))
}

fn js_err(what: &str) -> impl Fn(wasm_bindgen::JsValue) -> String + '_ {
    move |e| format!("{what}: {e:?}")
}

async fn decode(uri: &str) -> Result<ColorImage, String> {
    let window = web_sys::window().ok_or("no window")?;
    let response: web_sys::Response = JsFuture::from(window.fetch_with_str(uri))
        .await
        .map_err(js_err("fetch"))?
        .dyn_into()
        .map_err(|_| "fetch did not return a Response".to_owned())?;
    if !response.ok() {
        return Err(format!("{uri}: HTTP {}", response.status()));
    }
    let blob: web_sys::Blob = JsFuture::from(response.blob().map_err(js_err("blob"))?)
        .await
        .map_err(js_err("blob"))?
        .dyn_into()
        .map_err(|_| "not a Blob".to_owned())?;
    let bitmap: web_sys::ImageBitmap = JsFuture::from(
        window
            .create_image_bitmap_with_blob(&blob)
            .map_err(js_err("decode"))?,
    )
    .await
    // The browser has no decoder for it, such as HEIC outside Safari.
    .map_err(|_| "this browser can't show this kind of picture".to_owned())?
    .dyn_into()
    .map_err(|_| "not an ImageBitmap".to_owned())?;

    let (w, h) = kept_size(bitmap.width(), bitmap.height());
    let canvas: web_sys::HtmlCanvasElement = window
        .document()
        .ok_or("no document")?
        .create_element("canvas")
        .map_err(js_err("canvas"))?
        .dyn_into()
        .map_err(|_| "not a canvas".to_owned())?;
    canvas.set_width(w);
    canvas.set_height(h);
    let ctx2d: web_sys::CanvasRenderingContext2d = canvas
        .get_context("2d")
        .map_err(js_err("2d context"))?
        .ok_or("no 2d context")?
        .dyn_into()
        .map_err(|_| "not a 2d context".to_owned())?;
    ctx2d
        .draw_image_with_image_bitmap_and_dw_and_dh(&bitmap, 0.0, 0.0, f64::from(w), f64::from(h))
        .map_err(js_err("draw"))?;
    bitmap.close();
    let data = ctx2d
        .get_image_data(0.0, 0.0, f64::from(w), f64::from(h))
        .map_err(js_err("read back"))?
        .data();
    Ok(ColorImage::from_rgba_unmultiplied(
        [w as usize, h as usize],
        &data,
    ))
}

#[cfg(test)]
mod tests {
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::*;

    #[wasm_bindgen_test]
    fn only_this_servers_urls_are_fetched() {
        assert!(ours("/files/a.png?inline=1"));
        assert!(!ours("//evil.example/a.png"));
        assert!(!ours("https://example.com/a.png"));
        assert!(!ours("bytes://wallpaper"));
    }

    #[wasm_bindgen_test]
    fn big_pictures_are_shrunk_to_fit_keeping_their_shape() {
        assert_eq!(kept_size(800, 600), (800, 600));
        assert_eq!(kept_size(4096, 10), (4096, 10));
        assert_eq!(kept_size(8192, 6144), (4096, 3072));
        assert_eq!(kept_size(3000, 12000), (1024, 4096));
        assert_eq!(kept_size(100_000, 1), (4096, 1), "never zero");
    }
}
