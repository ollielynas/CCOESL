//! The desktop: wallpaper, window manager, taskbar, launcher.
//!
//! The shell owns everything egui persists — window geometry, z-order, focus, scroll offsets,
//! text state — which is exactly why apps can be one-frame-stale without the user ever seeing
//! it. An app never positions its own window; it fills one the desktop gave it.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use ccosel_host::AppHost;
use ccosel_host_web::{WebHost, WebInstance};
use js_sys::WebAssembly;

use crate::app_window::AppWindow;
use crate::fetch;
use crate::registry::{catalog, AppEntry};

/// A launch in flight, or its outcome. Launching is async (fetch + compile); the render loop
/// is not, so results land here and the next frame picks them up.
enum Launch {
    Ready(Box<AppWindow<WebInstance>>),
    Failed { name: String, error: String },
}

pub struct Desktop {
    registry: Vec<AppEntry>,
    windows: Vec<AppWindow<WebInstance>>,
    /// Compiled modules, keyed by app id. Compilation is the expensive half of a launch, so a
    /// second window of the same app is nearly free — and this is where the IndexedDB cache
    /// will eventually sit, to make it free across sessions too.
    modules: Rc<RefCell<HashMap<&'static str, WebAssembly::Module>>>,
    inbox: Rc<RefCell<Vec<Launch>>>,
    pending: Rc<RefCell<usize>>,
    next_instance_id: Rc<RefCell<u64>>,
    errors: Vec<String>,
    egui_ctx: egui::Context,
}

impl Desktop {
    pub fn new(egui_ctx: egui::Context) -> Self {
        let mut desktop = Self {
            registry: catalog(),
            windows: Vec::new(),
            modules: Rc::new(RefCell::new(HashMap::new())),
            inbox: Rc::new(RefCell::new(Vec::new())),
            pending: Rc::new(RefCell::new(0)),
            next_instance_id: Rc::new(RefCell::new(1)),
            errors: Vec::new(),
            egui_ctx,
        };
        // Open something on first boot: an empty desktop with no affordance is a worse first
        // impression than a window the user can close.
        if let Some(first) = desktop.registry.first().cloned() {
            desktop.launch(&first);
        }
        desktop
    }

    pub fn launch(&mut self, entry: &AppEntry) {
        let entry = entry.clone();
        let modules = self.modules.clone();
        let inbox = self.inbox.clone();
        let pending = self.pending.clone();
        let next_id = self.next_instance_id.clone();
        let ctx = self.egui_ctx.clone();

        *pending.borrow_mut() += 1;

        wasm_bindgen_futures::spawn_local(async move {
            let result = launch_inner(&entry, &modules, &next_id).await;
            let outcome = match result {
                Ok(window) => Launch::Ready(Box::new(window)),
                Err(error) => Launch::Failed {
                    name: entry.name.to_owned(),
                    error,
                },
            };
            inbox.borrow_mut().push(outcome);
            *pending.borrow_mut() -= 1;
            // The fetch resolved outside the frame loop, so nothing would redraw on its own.
            ctx.request_repaint();
        });
    }

    fn drain_inbox(&mut self) {
        for launch in self.inbox.borrow_mut().drain(..) {
            match launch {
                Launch::Ready(mut w) => {
                    // Two windows of the same app need distinguishable taskbar entries, or the
                    // taskbar stops being a way to find a particular window.
                    let n = self.windows.iter().filter(|x| x.app_id == w.app_id).count();
                    if n > 0 {
                        w.title = format!("{} {}", w.title, n + 1);
                    }
                    self.windows.push(*w);
                }
                Launch::Failed { name, error } => {
                    self.errors.push(format!("{name}: {error}"));
                }
            }
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.drain_inbox();
        let ctx = ui.ctx().clone();

        self.taskbar(ui);
        self.wallpaper(ui);

        // Closing a window drops the instance, which is the only way to reclaim a guest's
        // memory — wasm linear memory cannot shrink, so a live instance holds its high-water
        // mark forever.
        for window in &mut self.windows {
            let mut open = window.open;
            egui::Window::new(format!("{}  {}", window.icon, window.title))
                .id(egui::Id::new(("app-window", window.instance_id)))
                .default_size(window.default_size)
                .open(&mut open)
                .show(&ctx, |ui| window.ui(ui));
            window.open = open;
        }
        self.windows.retain(|w| w.open);
    }

    fn wallpaper(&mut self, ui: &mut egui::Ui) {
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                let rect = ui.max_rect();
                let painter = ui.painter();
                let dark = ui.visuals().dark_mode;
                let (top, bottom) = if dark {
                    (
                        egui::Color32::from_rgb(0x1a, 0x1f, 0x2b),
                        egui::Color32::from_rgb(0x10, 0x12, 0x18),
                    )
                } else {
                    (
                        egui::Color32::from_rgb(0xdd, 0xe4, 0xef),
                        egui::Color32::from_rgb(0xc3, 0xcd, 0xdd),
                    )
                };
                // A cheap vertical gradient: a handful of bands, no texture upload.
                let bands = 48;
                for i in 0..bands {
                    let t = i as f32 / bands as f32;
                    let band = egui::Rect::from_min_max(
                        egui::pos2(rect.min.x, rect.min.y + rect.height() * t),
                        egui::pos2(
                            rect.max.x,
                            rect.min.y + rect.height() * (t + 1.0 / bands as f32) + 1.0,
                        ),
                    );
                    painter.rect_filled(band, 0.0, lerp_color(top, bottom, t));
                }

                if self.windows.is_empty() && *self.pending.borrow() == 0 {
                    ui.centered_and_justified(|ui| {
                        ui.label("Open an app from the taskbar");
                    });
                }
            });
    }

    fn taskbar(&mut self, ui: &mut egui::Ui) {
        let mut to_launch: Option<AppEntry> = None;
        let mut to_focus: Option<u64> = None;

        egui::Panel::bottom("taskbar")
            .exact_size(40.0)
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.menu_button("  Apps  ", |ui| {
                        for entry in &self.registry {
                            if ui.button(format!("{}  {}", entry.icon, entry.name)).clicked() {
                                to_launch = Some(entry.clone());
                                ui.close();
                            }
                        }
                    });

                    ui.separator();

                    for window in &self.windows {
                        let label = format!("{}  {}", window.icon, window.title);
                        if ui.button(label).clicked() {
                            to_focus = Some(window.instance_id);
                        }
                    }

                    let launching = *self.pending.borrow();
                    if launching > 0 {
                        ui.spinner();
                    }

                    // Right-hand status. The per-frame byte count is the number this whole
                    // architecture is organised around, so it is worth keeping visible.
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let bytes: usize = self.windows.iter().map(|w| w.command_bytes()).sum();
                        ui.label(format!("{bytes} B/frame"));
                        ui.separator();
                        ui.label(format!("{} running", self.windows.len()));
                        if !self.errors.is_empty() {
                            ui.separator();
                            let msg = self.errors.join("\n");
                            ui.colored_label(egui::Color32::from_rgb(0xd9, 0x64, 0x5b), "!")
                                .on_hover_text(msg);
                        }
                    });
                });
            });

        if let Some(entry) = to_launch {
            self.launch(&entry);
        }
        if let Some(id) = to_focus {
            // egui tracks z-order per area, so "focus" is just moving that area to the top.
            ui.ctx()
                .move_to_top(egui::LayerId::new(
                    egui::Order::Middle,
                    egui::Id::new(("app-window", id)),
                ));
        }
    }
}

async fn launch_inner(
    entry: &AppEntry,
    modules: &Rc<RefCell<HashMap<&'static str, WebAssembly::Module>>>,
    next_id: &Rc<RefCell<u64>>,
) -> Result<AppWindow<WebInstance>, String> {
    let host = WebHost::new();

    let cached = modules.borrow().get(entry.id).cloned();
    let module = match cached {
        Some(m) => m,
        None => {
            let bytes = fetch::get_bytes(entry.url).await?;
            let m = host.compile(&bytes).await.map_err(|e| e.to_string())?;
            modules.borrow_mut().insert(entry.id, m.clone());
            m
        }
    };

    let instance = host.instantiate(&module).map_err(|e| e.to_string())?;

    let id = {
        let mut n = next_id.borrow_mut();
        *n += 1;
        *n
    };

    Ok(AppWindow::new(
        instance,
        id,
        entry.id,
        entry.name.to_owned(),
        entry.icon,
        entry.default_size,
    ))
}

fn lerp_color(a: egui::Color32, b: egui::Color32, t: f32) -> egui::Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t) as u8;
    egui::Color32::from_rgb(
        l(a.r(), b.r()),
        l(a.g(), b.g()),
        l(a.b(), b.b()),
    )
}
