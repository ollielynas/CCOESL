//! Desktop background, chosen by connection quality (issue #3).
//!
//! The decision itself — given a coarse read of the connection, image or shapes? — is pure and
//! lives in `ccosel-connection`, where it is unit-tested without a browser. Everything here is
//! the wasm-only glue `ccosel-connection` cannot have: reading `navigator.connection`, fetching
//! and decoding the wallpaper image, and drawing the fallback shapes. None of it can be
//! exercised without a real browser, so it stays thin on purpose — the decision is where the
//! judgment calls are, and that is what has tests.

use ccosel_connection::{Background, Quality, Signal};

/// Reads `navigator.connection` and turns it into a background choice. Best-effort throughout:
/// the API itself can be absent (true of Firefox and Safari today, so `Navigator::connection()`
/// returns `Err`), and even where present the browser may decline to report a given field. Any
/// gap degrades to [`Quality::Unknown`], which `ccosel_connection::choose_background` treats as
/// "don't spend the bytes."
pub fn detect() -> Background {
    let quality = read_signal()
        .as_ref()
        .map_or(Quality::Unknown, ccosel_connection::classify);
    ccosel_connection::choose_background(quality)
}

fn read_signal() -> Option<Signal> {
    let navigator = web_sys::window()?.navigator();
    let connection = navigator.connection().ok()?;

    // web-sys's pinned 0.3.104 release only binds the Network Information API's older, now
    // largely unimplemented `type` field — not the `effectiveType`/`downlink`/`saveData` that
    // real browsers (Chromium-based ones) actually expose. Those are read dynamically off the
    // raw JS object instead of through a typed getter.
    let save_data = js_sys::Reflect::get(&connection, &"saveData".into())
        .ok()
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let effective_type = js_sys::Reflect::get(&connection, &"effectiveType".into())
        .ok()
        .and_then(|v| v.as_string());
    let downlink_mbps = js_sys::Reflect::get(&connection, &"downlink".into())
        .ok()
        .and_then(|v| v.as_f64())
        .map(|v| v as f32);

    Some(Signal {
        effective_type,
        downlink_mbps,
        save_data,
    })
}

/// Fetches and decodes the wallpaper image, ready for `egui::Context::load_texture`. Only ever
/// called when [`detect`] chose [`Background::Image`], so this is exactly the extra network
/// traffic that choice implies — nothing is fetched when the connection looked too weak for it.
pub async fn fetch_wallpaper(url: &str) -> Result<egui::ColorImage, String> {
    let bytes = crate::fetch::get_bytes(url).await?;
    let decoded = image::load_from_memory(&bytes).map_err(|e| format!("decode wallpaper: {e}"))?;
    let rgba = decoded.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    Ok(egui::ColorImage::from_rgba_unmultiplied(
        size,
        rgba.as_raw(),
    ))
}

/// Draws a handful of soft, translucent circles: the "abstract shapes" background for a
/// connection too weak (or too uncertain) to justify downloading a real image. Purely local —
/// no allocation beyond what `egui::Painter` already does, and no network — so it is always
/// available as the fallback, including while an image is still loading.
pub fn draw_shapes(painter: &egui::Painter, rect: egui::Rect, dark_mode: bool) {
    // Relative (x, y, radius, alpha) offsets so the layout reflows with any window size.
    const SHAPES: [(f32, f32, f32, u8); 6] = [
        (0.12, 0.22, 0.16, 40),
        (0.82, 0.16, 0.22, 32),
        (0.55, 0.68, 0.28, 26),
        (0.22, 0.82, 0.14, 42),
        (0.92, 0.78, 0.18, 30),
        (0.42, 0.38, 0.10, 46),
    ];
    let (r, g, b) = if dark_mode {
        (0x6f, 0x8f, 0xc9)
    } else {
        (0x9e, 0xb4, 0xd9)
    };
    for (x, y, radius_frac, alpha) in SHAPES {
        let center = egui::pos2(
            rect.min.x + rect.width() * x,
            rect.min.y + rect.height() * y,
        );
        let radius = rect.width().min(rect.height()) * radius_frac;
        painter.circle_filled(
            center,
            radius,
            egui::Color32::from_rgba_unmultiplied(r, g, b, alpha),
        );
    }
}
