//! SVG drawn to a PNG in the server, with resvg.
//!
//! An SVG is a document, not a picture: it can hold scripts, links, HTML, and references to
//! other files and URLs. Handed to the browser it would run as part of the page. Drawn here,
//! none of that survives: resvg runs no script and draws no HTML, and the resolver below
//! refuses every reference that isn't a `data:` URI inside the file itself. That matters
//! beyond the browser: the server can read files the viewer can't, so an `<image href=` to
//! another person's photo must never be followed.

use std::path::Path;
use std::sync::{Arc, OnceLock};

use resvg::{tiny_skia, usvg};

/// The longest side a drawing is made at. It's a vector, so it is drawn sharp at this size
/// however small it says it is, and never bigger however large it says it is.
pub const SIDE: u32 = 2048;

/// The largest SVG file drawn. Parsing and drawing take time and memory in proportion, and a
/// real drawing is seldom more than a few megabytes.
pub const MAX_BYTES: u64 = 16 << 20;

/// The system's fonts, loaded once: for `<text>`, which would otherwise not be drawn.
fn fonts() -> Arc<usvg::fontdb::Database> {
    static FONTS: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    FONTS
        .get_or_init(|| {
            let mut db = usvg::fontdb::Database::new();
            db.load_system_fonts();
            Arc::new(db)
        })
        .clone()
}

/// Draw the SVG at `input` into a PNG at `output`. Its own size, as it states it, for the
/// Viewer to shape its window by.
pub fn render(input: &Path, output: &Path) -> Result<(u32, u32), String> {
    let len = std::fs::metadata(input)
        .map_err(|e| format!("couldn't read it: {e}"))?
        .len();
    if len > MAX_BYTES {
        return Err(format!(
            "it's too big to draw here ({} MB; the most is {} MB)",
            len >> 20,
            MAX_BYTES >> 20
        ));
    }
    let data = std::fs::read(input).map_err(|e| format!("couldn't read it: {e}"))?;
    let options = usvg::Options {
        // Nothing relative to anywhere: there is no folder to look in.
        resources_dir: None,
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: usvg::ImageHrefResolver::default_data_resolver(),
            // A path or URL: refused, whatever it is.
            resolve_string: Box::new(|_, _| None),
        },
        fontdb: fonts(),
        ..usvg::Options::default()
    };
    let tree = usvg::Tree::from_data(&data, &options)
        .map_err(|e| format!("it isn't an SVG drawing this can read: {e}"))?;
    let size = tree.size();
    let (w, h) = (size.width(), size.height());
    let scale = SIDE as f32 / w.max(h);
    let px = |v: f32| ((v * scale).round() as u32).clamp(1, SIDE);
    let mut pixmap = tiny_skia::Pixmap::new(px(w), px(h)).ok_or("it has no size to draw at")?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    pixmap
        .save_png(output)
        .map_err(|e| format!("couldn't keep the drawing: {e}"))?;
    Ok((w.round().max(1.0) as u32, h.round().max(1.0) as u32))
}
