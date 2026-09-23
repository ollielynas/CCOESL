//! Fetching bytes over HTTP.
//!
//! Deliberately plain `fetch`: app modules are served from content-addressed, `immutable` URLs,
//! so the browser's own HTTP cache does the right thing without any help from us. The
//! IndexedDB module cache layers on top of this later, to survive across sessions and to hold
//! *compiled* `WebAssembly.Module` objects rather than raw bytes.

use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

pub async fn get_bytes(url: &str) -> Result<Vec<u8>, String> {
    let window = web_sys::window().ok_or("no window")?;
    let response = JsFuture::from(window.fetch_with_str(url))
        .await
        .map_err(|e| format!("fetch {url}: {e:?}"))?
        .dyn_into::<web_sys::Response>()
        .map_err(|_| "fetch did not return a Response".to_owned())?;

    if !response.ok() {
        return Err(format!("{url}: HTTP {}", response.status()));
    }

    let buffer = JsFuture::from(
        response
            .array_buffer()
            .map_err(|e| format!("array_buffer: {e:?}"))?,
    )
    .await
    .map_err(|e| format!("array_buffer: {e:?}"))?;

    Ok(js_sys::Uint8Array::new(&buffer).to_vec())
}

/// POST bytes and return the response body.
pub async fn post_bytes(url: &str, body: &[u8]) -> Result<Vec<u8>, String> {
    let window = web_sys::window().ok_or("no window")?;

    let init = web_sys::RequestInit::new();
    init.set_method("POST");
    // `Uint8Array::from` copies, so the buffer is not aliased into the request.
    init.set_body(&js_sys::Uint8Array::from(body).into());

    let request = web_sys::Request::new_with_str_and_init(url, &init)
        .map_err(|e| format!("request {url}: {e:?}"))?;
    request
        .headers()
        .set("content-type", "application/octet-stream")
        .map_err(|e| format!("headers: {e:?}"))?;

    let response = JsFuture::from(window.fetch_with_request(&request))
        .await
        .map_err(|e| format!("fetch {url}: {e:?}"))?
        .dyn_into::<web_sys::Response>()
        .map_err(|_| "fetch did not return a Response".to_owned())?;

    if !response.ok() {
        return Err(format!("{url}: HTTP {}", response.status()));
    }

    let buffer = JsFuture::from(
        response
            .array_buffer()
            .map_err(|e| format!("array_buffer: {e:?}"))?,
    )
    .await
    .map_err(|e| format!("array_buffer: {e:?}"))?;

    Ok(js_sys::Uint8Array::new(&buffer).to_vec())
}
