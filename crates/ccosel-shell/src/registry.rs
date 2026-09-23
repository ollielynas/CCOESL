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
    /// A Phosphor glyph, rendered in `color` wherever the app's identity needs to stand out
    /// (the taskbar, a window's title bar).
    pub icon: &'static str,
    /// The app's own colour. Every app gets a different one, so its taskbar entry and open
    /// windows stay identifiable at a glance instead of blurring into one grey list.
    pub color: egui::Color32,
    pub url: &'static str,
    pub default_size: [f32; 2],
}

pub fn catalog() -> Vec<AppEntry> {
    vec![
        AppEntry {
            id: "file-browser",
            name: "Files",
            icon: egui_phosphor::regular::FOLDER,
            color: egui::Color32::from_rgb(0x3B, 0x82, 0xF6),
            url: "./dist/file-browser.wasm",
            default_size: [420.0, 320.0],
        },
        AppEntry {
            id: "clock",
            name: "Clock",
            icon: egui_phosphor::regular::CLOCK,
            color: egui::Color32::from_rgb(0xF5, 0x9E, 0x0B),
            url: "./dist/clock.wasm",
            default_size: [240.0, 200.0],
        },
    ]
}
