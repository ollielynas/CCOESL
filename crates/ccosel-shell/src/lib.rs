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
mod desktop;
mod fetch;
mod http_wire;
mod registry;

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

    eframe::WebRunner::new()
        .start(
            canvas,
            eframe::WebOptions::default(),
            Box::new(|cc| {
                Ok(Box::new(Shell {
                    desktop: Desktop::new(cc.egui_ctx.clone()),
                }))
            }),
        )
        .await
}
