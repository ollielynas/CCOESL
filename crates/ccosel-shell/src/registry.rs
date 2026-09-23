//! What apps exist and where to get them.
//!
//! Static for now. This becomes `GET /manifest` — the one mutable URL in the app-delivery
//! path, mapping `app_id -> {content hash, size, icon, min_abi_version}` while every module
//! itself is served from an immutable content-addressed URL. Keeping the shape here now means
//! swapping the source later touches one function.

#[derive(Clone)]
pub struct AppEntry {
    pub id: &'static str,
    pub name: &'static str,
    /// Single-glyph stand-in until the icon pipeline exists.
    pub icon: &'static str,
    pub url: &'static str,
    pub default_size: [f32; 2],
}

pub fn catalog() -> Vec<AppEntry> {
    vec![
        AppEntry {
            id: "file-browser",
            name: "Files",
            icon: "🗀",
            url: "./dist/file-browser.wasm",
            default_size: [420.0, 320.0],
        },
        AppEntry {
            id: "clock",
            name: "Clock",
            icon: "◴",
            url: "./dist/clock.wasm",
            default_size: [240.0, 200.0],
        },
        AppEntry {
            id: "rust-compiler",
            name: "Compiler",
            icon: "⚙",
            url: "./dist/rust-compiler.wasm",
            default_size: [480.0, 360.0],
        },
    ]
}
