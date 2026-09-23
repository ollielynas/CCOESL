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
use ccosel_transport::{PendingKey, Transport};
use js_sys::WebAssembly;

use crate::http_wire::{HttpWire, Inbox};

use crate::app_window::AppWindow;
use crate::fetch;
use crate::fullscreen;
use crate::registry::{AppEntry, catalog};
use crate::theme;

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
    /// The shell owns the pending-call table, not the guests. An app can be closed or
    /// suspended with calls outstanding without the shell losing track of them.
    transport: Transport,
    /// Replies land here from `fetch` callbacks and are applied at the top of the next frame.
    replies: Inbox,
}

impl Desktop {
    pub fn new(egui_ctx: egui::Context) -> Self {
        theme::apply(&egui_ctx);

        let replies: Inbox = Rc::new(RefCell::new(Vec::new()));
        let wire = HttpWire::new("/rpc", replies.clone(), egui_ctx.clone());

        let mut desktop = Self {
            registry: catalog(),
            windows: Vec::new(),
            modules: Rc::new(RefCell::new(HashMap::new())),
            inbox: Rc::new(RefCell::new(Vec::new())),
            pending: Rc::new(RefCell::new(0)),
            next_instance_id: Rc::new(RefCell::new(1)),
            errors: Vec::new(),
            egui_ctx,
            transport: Transport::new(Box::new(wire)),
            replies,
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
        let now_ms = ctx.input(|i| i.time) * 1000.0;

        // Apply anything that arrived since the last frame, so guests see replies before they
        // render this frame.
        let replies: Vec<_> = self.replies.borrow_mut().drain(..).collect();
        if !replies.is_empty() {
            self.transport.on_replies(replies);
        }
        // Expire deadlines and reap calls belonging to windows that have gone.
        self.transport.tick(now_ms);

        theme::paint_wallpaper(&ctx);
        self.taskbar(ui);
        self.empty_state(ui);

        // Closing a window drops the instance, which is the only way to reclaim a guest's
        // memory — wasm linear memory cannot shrink, so a live instance holds its high-water
        // mark forever.
        for window in &mut self.windows {
            let mut open = window.open;
            egui::Window::new(icon_title(window.icon, window.color, &window.title))
                .id(egui::Id::new(("app-window", window.instance_id)))
                .default_size(window.default_size)
                .open(&mut open)
                .show(&ctx, |ui| {
                    // `auto_shrink(false)` claims the whole window body regardless of how much
                    // the app actually drew, so dragging the window bigger than its content
                    // leaves blank space rather than the window snapping back to fit. It also
                    // means a long listing scrolls instead of growing the window without bound.
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| window.ui(ui));
                });

            // Anything the guest asked for during that frame.
            let sink = window.sink();
            for call in window.take_outbox() {
                self.transport.enqueue(
                    PendingKey {
                        instance: window.instance_id,
                        call: call.call_id,
                    },
                    call.method as u16,
                    call.args,
                    sink.clone(),
                    now_ms,
                );
            }
            for call_id in window.take_cancels() {
                self.transport.cancel(PendingKey {
                    instance: window.instance_id,
                    call: call_id,
                });
            }

            if !open {
                window.close();
            }
        }

        // One flush per frame: calls made during a frame share a single request, which is the
        // coalescing window, with no timer to arm and nothing to poll.
        self.transport.flush();

        for window in self.windows.iter().filter(|w| !w.open) {
            self.transport.forget_instance(window.instance_id);
        }
        self.windows.retain(|w| w.open);
    }

    /// What the desktop says when nothing is open. `theme::paint_wallpaper` already covers the
    /// background layer beneath this, so all that is left is the affordance.
    fn empty_state(&self, ui: &mut egui::Ui) {
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                if self.windows.is_empty() && *self.pending.borrow() == 0 {
                    ui.centered_and_justified(|ui| {
                        ui.label("Open an app from the taskbar");
                    });
                }
            });
    }

    fn taskbar(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette(ui.ctx());
        let mut to_launch: Option<AppEntry> = None;
        let mut to_focus: Option<u64> = None;

        // No explicit border is painted here: the panel's own separator line, drawn from
        // `widgets.noninteractive.bg_stroke` (the palette's `border`), already runs along its
        // top edge, which is exactly the "top border" the taskbar wants.
        egui::Panel::bottom("taskbar")
            .exact_size(44.0)
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.menu_button("  Apps  ", |ui| {
                        for entry in &self.registry {
                            let label = icon_title(entry.icon, entry.color, entry.name);
                            if ui.button(label).clicked() {
                                to_launch = Some(entry.clone());
                                ui.close();
                            }
                        }
                    });

                    ui.separator();

                    for window in &self.windows {
                        let label = icon_title(window.icon, window.color, &window.title);
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
                        // Rightmost, like a system-tray icon: the one control here that isn't
                        // status. The label reflects whatever state the browser is actually in,
                        // since F11 or Escape can leave fullscreen without going through this
                        // button at all.
                        let icon = if fullscreen::is_active() {
                            "⤡"
                        } else {
                            "⛶"
                        };
                        if ui.button(icon).on_hover_text("Toggle fullscreen").clicked() {
                            fullscreen::toggle();
                        }
                        ui.separator();

                        let bytes: usize = self.windows.iter().map(|w| w.command_bytes()).sum();
                        ui.label(status_text(format!("{bytes} B/frame"), p));
                        ui.separator();
                        ui.label(status_text(format!("{} running", self.windows.len()), p));
                        if !self.errors.is_empty() {
                            ui.separator();
                            let msg = self.errors.join("\n");
                            ui.label(
                                egui::RichText::new(egui_phosphor::regular::WARNING)
                                    .color(ui.visuals().error_fg_color),
                            )
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
            ui.ctx().move_to_top(egui::LayerId::new(
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
        entry.color,
        entry.default_size,
    ))
}

/// `WidgetText` for a window title or taskbar entry: the app's Phosphor icon in its own
/// colour, followed by its name in whatever colour the surrounding widget would normally use.
/// Built as one `LayoutJob` rather than two widgets, so it drops into anything that takes a
/// title — window titles included, which can't host a custom-painted child. This, plus the
/// registry's per-app colour, is what "brighter app badge colours" means here: every app's
/// mark carries its own colour everywhere it appears, rather than one uniform grey label.
fn icon_title(icon: &str, color: egui::Color32, text: &str) -> egui::WidgetText {
    let mut job = egui::text::LayoutJob::default();
    job.append(
        icon,
        0.0,
        egui::TextFormat {
            color,
            ..Default::default()
        },
    );
    job.append(
        &format!("  {text}"),
        0.0,
        egui::TextFormat {
            color: egui::Color32::PLACEHOLDER,
            ..Default::default()
        },
    );
    job.into()
}

fn status_text(text: String, p: &theme::Palette) -> egui::RichText {
    egui::RichText::new(text).small().color(p.text_dim)
}
