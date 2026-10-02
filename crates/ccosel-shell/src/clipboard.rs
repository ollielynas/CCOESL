//! Copying text to the clipboard, for `CopyLink` (sharing a file through the Viewer).
//!
//! `navigator.clipboard` exists only in a secure context, and CCOSEL is usually reached over plain
//! HTTP on the LAN, where it is missing. There the old way still works: select the text in a
//! hidden text area and run the `copy` command. Both need the click that asked for it to be
//! recent, which is why the shell acts on a `CopyLink` in the frame its click is drawn.

use wasm_bindgen::JsCast;

pub fn copy(text: &str) -> Result<(), String> {
    let window = web_sys::window().ok_or("no window")?;
    if window.is_secure_context() {
        // Settles later; a refusal there is the browser's to report.
        let _ = window.navigator().clipboard().write_text(text);
        return Ok(());
    }
    let document = window.document().ok_or("no document")?;
    let area: web_sys::HtmlTextAreaElement = document
        .create_element("textarea")
        .map_err(|e| format!("{e:?}"))?
        .dyn_into()
        .map_err(|_| "not a text area".to_owned())?;
    area.set_value(text);
    // Off-screen rather than hidden: a hidden element can't be selected.
    let _ = area
        .style()
        .set_property("position", "fixed")
        .and_then(|()| area.style().set_property("left", "-10000px"));
    let body = document.body().ok_or("no body")?;
    body.append_child(&area).map_err(|e| format!("{e:?}"))?;
    area.select();
    let copied = document
        .dyn_ref::<web_sys::HtmlDocument>()
        .map(|d| d.exec_command("copy").unwrap_or(false))
        .unwrap_or(false);
    area.remove();
    if copied {
        Ok(())
    } else {
        Err("the browser didn't allow copying".to_owned())
    }
}
