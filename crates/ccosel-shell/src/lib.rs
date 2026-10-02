//! The CCOSEL shell: the thing the browser loads.
//!
//! It owns the egui context and everything egui persists — window geometry, z-order, focus,
//! scroll offsets, text buffers — fetches app modules over the network, instantiates them, and
//! replays their command streams. Apps never touch egui; they never even link it.
//!
//! That ownership split is what makes the one-frame response delay invisible: the shell draws
//! every interaction, so nothing the user sees is ever stale. A guest only learns the
//! committed result a frame later.

#![cfg(target_arch = "wasm32")]

mod app_window;
mod background;
mod chrome;
mod clipboard;
mod desktop;
mod fetch;
mod fullscreen;
mod http_wire;
mod image_loader;
mod media;
mod registry;
mod session;
mod theme;
mod upload;

use wasm_bindgen::prelude::*;

use desktop::Desktop;

pub struct Shell {
    desktop: Desktop,
}

impl eframe::App for Shell {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.desktop.ui(ui);
    }
}

/// Entry point called by the page loader.
#[wasm_bindgen]
pub async fn start(canvas: web_sys::HtmlCanvasElement) -> Result<(), JsValue> {
    console_error_panic_hook::set_once();

    // `/app/{id}` is the same page with one app filling it instead of the desktop, and
    // `?open=...` what to open that app on: how a shared link opens a file in the Viewer.
    let location = web_sys::window().map(|w| w.location());
    let solo = location
        .as_ref()
        .and_then(|l| l.pathname().ok())
        .and_then(|path| registry::solo_id(&path).map(str::to_owned));
    let solo_arg = location
        .and_then(|l| l.search().ok())
        .and_then(|search| registry::solo_arg(&search));

    eframe::WebRunner::new()
        .start(
            canvas,
            eframe::WebOptions::default(),
            Box::new(|cc| {
                Ok(Box::new(Shell {
                    desktop: Desktop::new(cc.egui_ctx.clone(), solo, solo_arg),
                }))
            }),
        )
        .await
}
