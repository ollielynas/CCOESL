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
    /// A Phosphor glyph, drawn on `color` as a badge — see `theme::paint_badge`.
    pub icon: &'static str,
    /// The badge's background. Each app gets its own, so its windows and dock entries
    /// stay visually identifiable at a glance instead of blurring into one grey list.
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
            color: egui::Color32::from_rgb(0x3b, 0x82, 0xf6),
            url: "./dist/file-browser.wasm",
            default_size: [420.0, 320.0],
        },
        AppEntry {
            id: "clock",
            name: "Clock",
            icon: egui_phosphor::regular::CLOCK,
            color: egui::Color32::from_rgb(0xf5, 0x9e, 0x0b),
            url: "./dist/clock.wasm",
            default_size: [240.0, 200.0],
        },
        AppEntry {
            id: "server-dashboard",
            name: "Server",
            icon: "🛠",
            url: "./dist/server-dashboard.wasm",
            default_size: [380.0, 420.0],
        },
    ]
}
