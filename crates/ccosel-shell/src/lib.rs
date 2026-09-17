//! The CCOSEL shell: the thing the browser actually loads.
//!
//! It owns the egui context and all the state egui persists (window geometry, focus, scroll,
//! text buffers), fetches app modules over the network, instantiates them, and replays their
//! command streams. Apps never touch egui; they never even link it.
//!
//! Right now this is the minimum that puts a real guest app on screen in a browser tab. The
//! window manager, taskbar and launcher come next.

#![cfg(target_arch = "wasm32")]

mod app_window;
mod fetch;

use std::cell::RefCell;
use std::rc::Rc;

use ccosel_host::AppHost;
use ccosel_host_web::{WebHost, WebInstance};
use wasm_bindgen::prelude::*;

use app_window::AppWindow;

/// How the shell is doing at getting an app on screen.
enum Loading {
    Fetching,
    Ready(Box<AppWindow<WebInstance>>),
    Failed(String),
}

pub struct Shell {
    state: Rc<RefCell<Loading>>,
}

impl Shell {
    fn new(ctx: egui::Context, module_url: String) -> Self {
        let state = Rc::new(RefCell::new(Loading::Fetching));

        // Module fetch is async and the render loop is not, so the result lands in a shared
        // slot and the next frame picks it up. Nothing blocks.
        let slot = state.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let next = match load_app(&module_url).await {
                Ok(instance) => Loading::Ready(Box::new(AppWindow::new(instance))),
                Err(e) => Loading::Failed(e),
            };
            *slot.borrow_mut() = next;
            // The fetch resolved outside the frame loop; ask for a repaint so the result is
            // shown rather than waiting for the user to move the mouse.
            ctx.request_repaint();
        });

        Self { state }
    }
}

async fn load_app(url: &str) -> Result<WebInstance, String> {
    let bytes = fetch::get_bytes(url).await?;
    let host = WebHost::new();
    let module = host.compile(&bytes).await.map_err(|e| e.to_string())?;
    host.instantiate(&module).map_err(|e| e.to_string())
}

impl eframe::App for Shell {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| match &mut *self.state.borrow_mut() {
            Loading::Fetching => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Loading app…");
                });
            }
            Loading::Failed(err) => {
                ui.colored_label(egui::Color32::RED, "Failed to load app");
                ui.label(err.as_str());
            }
            Loading::Ready(window) => window.ui(ui),
        });
    }
}

/// Entry point called by the page loader.
#[wasm_bindgen]
pub async fn start(canvas: web_sys::HtmlCanvasElement, module_url: String) -> Result<(), JsValue> {
    console_error_panic_hook::set_once();

    eframe::WebRunner::new()
        .start(
            canvas,
            eframe::WebOptions::default(),
            Box::new(move |cc| Ok(Box::new(Shell::new(cc.egui_ctx.clone(), module_url)))),
        )
        .await
}
