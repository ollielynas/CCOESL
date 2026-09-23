//! Toggling the browser's own fullscreen mode.
//!
//! This is the tab's fullscreen, via the DOM Fullscreen API — not anything the shell draws
//! itself. `web_sys::Document::request_fullscreen` only succeeds when called from inside a user
//! gesture (the click that triggered it), so `toggle` exists to be called straight from a
//! button's `clicked()`, never from a timer or a reply handler.

/// `None` when there is no `window`/`document` to ask — never expected in a browser, but a
/// missing DOM handle should disable the toggle, not panic a frame that runs every tick.
fn document() -> Option<web_sys::Document> {
    web_sys::window()?.document()
}

/// Whether the page is currently fullscreen, however it got there (this toggle, or the
/// browser's own F11). Cheap enough to call every frame: one property read, no allocation.
pub fn is_active() -> bool {
    document().and_then(|d| d.fullscreen_element()).is_some()
}

/// Leaves fullscreen if the page is in it, requests it otherwise. Must be called from a user
/// gesture (a click), or the browser silently refuses the request.
pub fn toggle() {
    let Some(doc) = document() else { return };
    if doc.fullscreen_element().is_some() {
        doc.exit_fullscreen();
    } else if let Some(root) = doc.document_element() {
        // Fullscreening the document root, not just the canvas, so the boot screen and any
        // future chrome outside the canvas are covered too.
        if let Err(err) = root.request_fullscreen() {
            log::warn!("request_fullscreen failed: {err:?}");
        }
    }
}
