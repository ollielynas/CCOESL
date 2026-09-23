//! The desktop: wallpaper, window manager, status bar, dock.
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

/// Dock badge geometry, shared between `dock_item` (which paints it) and `app_menu` (which
/// needs to know the dock's on-screen height so its popup can sit above it without overlapping).
const DOCK_BADGE: f32 = 42.0;
const DOCK_LIFT: f32 = 5.0;
/// Room under the badge for the running pill, plus the headroom the lift needs.
const DOCK_GUTTER: f32 = 8.0;
/// Matches the dock frame's `inner_margin` (`Margin::symmetric(10, 6)`): 6px top and bottom.
const DOCK_FRAME_MARGIN_V: f32 = 6.0;
/// Matches the dock area's own anchor offset: how far its bottom edge sits above the screen's.
const DOCK_BOTTOM_OFFSET: f32 = 16.0;
/// The dock's total on-screen height: badge, lift headroom, pill gutter, and the frame's
/// vertical margin on both edges.
const DOCK_HEIGHT: f32 = DOCK_BADGE + DOCK_LIFT + DOCK_GUTTER + DOCK_FRAME_MARGIN_V * 2.0;
/// Clear space between the dock's top edge and the app menu popup above it.
const MENU_GAP: f32 = 12.0;

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
    /// Whether the app menu popup is open.
    app_menu_open: bool,
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
            app_menu_open: false,
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
        self.status_bar(&ctx);
        self.empty_state(ui);
        self.app_menu(&ctx);
        self.dock(&ctx);

        // Closing a window drops the instance, which is the only way to reclaim a guest's
        // memory — wasm linear memory cannot shrink, so a live instance holds its high-water
        // mark forever.
        for window in &mut self.windows {
            let mut open = window.open;
            // Outlined in its own app's accent, so which app a window belongs to stays legible
            // from its edge alone once several of them overlap.
            let frame = egui::Frame::window(&ctx.style_of(ctx.theme()))
                .stroke(egui::Stroke::new(1.0, window.color.gamma_multiply(0.5)));
            egui::Window::new(icon_title(window.icon, window.color, &window.title))
                .id(egui::Id::new(("app-window", window.instance_id)))
                .default_size(window.default_size)
                .frame(frame)
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

    /// The menu bar: the shell's own mark on the left, system health on the right.
    fn status_bar(&self, ctx: &egui::Context) {
        let p = theme::palette(ctx);
        let width = ctx.viewport_rect().width();
        const PAD: f32 = 14.0;

        let frame = egui::Frame::NONE
            .fill(theme::glass(p))
            .inner_margin(egui::Margin::symmetric(PAD as i8, 5));

        egui::Area::new(egui::Id::new("status-bar"))
            .anchor(egui::Align2::LEFT_TOP, egui::vec2(0.0, 0.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                frame.show(ui, |ui| {
                    ui.set_width(width - PAD * 2.0);
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(egui_phosphor::regular::SQUARES_FOUR)
                                .size(15.0)
                                .color(p.accent),
                        );
                        ui.label(egui::RichText::new("CCOSEL").color(p.text));

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            // Rightmost, like a system-tray icon: the one control here that
                            // isn't status. The icon reflects whatever state the browser is
                            // actually in, since F11 or Escape can leave fullscreen without
                            // going through this button at all.
                            let icon = if fullscreen::is_active() {
                                egui_phosphor::regular::ARROWS_IN
                            } else {
                                egui_phosphor::regular::ARROWS_OUT
                            };
                            if ui
                                .add(egui::Button::new(
                                    egui::RichText::new(icon).size(13.0).color(p.text_dim),
                                ))
                                .on_hover_text("Toggle fullscreen")
                                .clicked()
                            {
                                fullscreen::toggle();
                            }

                            if !self.errors.is_empty() {
                                ui.label(
                                    egui::RichText::new(egui_phosphor::regular::WARNING)
                                        .color(ui.visuals().error_fg_color),
                                )
                                .on_hover_text(self.errors.join("\n"));
                            }
                            if *self.pending.borrow() > 0 {
                                ui.spinner();
                            }
                            // The per-frame byte count is the number this whole architecture is
                            // organised around, so it stays on screen rather than behind a menu.
                            let bytes: usize = self.windows.iter().map(|w| w.command_bytes()).sum();
                            ui.label(status_text(format!("{bytes} B/frame"), p));
                            ui.label(status_text("\u{00b7}".to_owned(), p));
                            ui.label(status_text(format!("{} running", self.windows.len()), p));
                        });
                    });
                });
            });
    }

    /// What the desktop says when nothing is open. A blank screen with no affordance is a
    /// worse first impression than one line pointing at the dock.
    fn empty_state(&self, ui: &mut egui::Ui) {
        let p = theme::palette(ui.ctx());
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                if !self.windows.is_empty() || *self.pending.borrow() > 0 {
                    return;
                }
                ui.vertical_centered(|ui| {
                    ui.add_space(ui.available_height() * 0.30);
                    ui.label(
                        egui::RichText::new(egui_phosphor::regular::SQUARES_FOUR)
                            .size(44.0)
                            .color(p.text_dim.gamma_multiply(0.55)),
                    );
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new("CCOSEL").size(24.0).color(p.text));
                    ui.label(egui::RichText::new("Pick an app from the dock").color(p.text_dim));
                });
            });
    }

    /// The app menu: a popup above the dock listing every installed app with its icon and
    /// name. Clicking one launches it and closes the menu.
    fn app_menu(&mut self, ctx: &egui::Context) {
        if !self.app_menu_open {
            return;
        }
        let p = theme::palette(ctx);
        let mut to_launch: Option<AppEntry> = None;

        let frame = egui::Frame::NONE
            .fill(theme::glass(p))
            .stroke(egui::Stroke::new(1.0, theme::glass_border(p)))
            .corner_radius(egui::CornerRadius::same(14))
            .inner_margin(egui::Margin::same(10))
            .shadow(egui::Shadow {
                offset: [0, 8],
                blur: 24,
                spread: 0,
                color: p.shadow,
            });

        // Anchored the same distance above the dock's own top edge (dock height + its own
        // anchor offset) plus a fixed gap, computed from the dock's real geometry rather than a
        // guessed constant, so the two can never drift back into overlapping.
        let menu_bottom_offset = DOCK_BOTTOM_OFFSET + DOCK_HEIGHT + MENU_GAP;

        egui::Area::new(egui::Id::new("app-menu"))
            .anchor(
                egui::Align2::CENTER_BOTTOM,
                egui::vec2(0.0, -menu_bottom_offset),
            )
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                frame.show(ui, |ui| {
                    ui.vertical(|ui| {
                        ui.set_min_width(200.0);
                        for entry in &self.registry {
                            // Allocate the *whole* row as one click target first, then paint the
                            // badge and label into it, so there is no separate `ui.label` widget
                            // sitting on top able to swallow the click before the row sees it —
                            // any point in the row launches the app, not just the icon glyph.
                            let row_size = egui::vec2(ui.available_width(), 32.0);
                            let (rect, response) =
                                ui.allocate_exact_size(row_size, egui::Sense::click());

                            if ui.is_rect_visible(rect) {
                                if response.hovered() {
                                    ui.painter().rect_filled(
                                        rect,
                                        egui::CornerRadius::same(8),
                                        p.surface_hover,
                                    );
                                }

                                let badge_rect = egui::Rect::from_center_size(
                                    egui::pos2(rect.min.x + 16.0, rect.center().y),
                                    egui::vec2(32.0, 32.0),
                                );
                                theme::paint_badge(
                                    ui.painter(),
                                    badge_rect,
                                    entry.icon,
                                    entry.color,
                                );

                                ui.painter().text(
                                    egui::pos2(rect.min.x + 42.0, rect.center().y),
                                    egui::Align2::LEFT_CENTER,
                                    entry.name,
                                    egui::FontId::proportional(14.0),
                                    p.text,
                                );
                            }

                            if response.clicked() {
                                to_launch = Some(entry.clone());
                                self.app_menu_open = false;
                            }
                        }
                    });
                });
            });

        if let Some(entry) = to_launch {
            self.launch(&entry);
        }
    }

    /// The dock: a launcher icon on the left, then every open window. The launcher opens a
    /// popup listing all installed apps. Clicking a window raises that one.
    ///
    /// A floating area rather than a panel, so windows pass underneath it instead of the
    /// desktop permanently losing a full-width strip of height to mostly-empty chrome.
    fn dock(&mut self, ctx: &egui::Context) {
        let p = theme::palette(ctx);
        let mut to_focus: Option<u64> = None;

        let frame = egui::Frame::NONE
            .fill(theme::glass(p))
            .stroke(egui::Stroke::new(1.0, theme::glass_border(p)))
            .corner_radius(egui::CornerRadius::same(22))
            .inner_margin(egui::Margin::symmetric(10, DOCK_FRAME_MARGIN_V as i8))
            .shadow(egui::Shadow {
                offset: [0, 12],
                blur: 36,
                spread: 0,
                color: p.shadow,
            });

        egui::Area::new(egui::Id::new("dock"))
            .anchor(
                egui::Align2::CENTER_BOTTOM,
                egui::vec2(0.0, -DOCK_BOTTOM_OFFSET),
            )
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                frame.show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;

                        // Launcher icon: opens the app menu.
                        let launcher = dock_item(
                            ui,
                            egui_phosphor::regular::SQUARES_FOUR,
                            p.accent,
                            "All apps",
                            false,
                        );
                        if launcher.clicked() {
                            self.app_menu_open = !self.app_menu_open;
                        }

                        if !self.windows.is_empty() {
                            ui.separator();
                            for window in &self.windows {
                                let item =
                                    dock_item(ui, window.icon, window.color, &window.title, false);
                                if item.clicked() {
                                    to_focus = Some(window.instance_id);
                                }
                            }
                        }
                    });
                });
            });

        if let Some(id) = to_focus {
            // egui tracks z-order per area, so "focus" is just moving that area to the top.
            ctx.move_to_top(egui::LayerId::new(
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

/// `WidgetText` for a window title or menu row: the app's badge glyph in its own color,
/// followed by its name in whatever color the surrounding widget would normally use. Built as
/// one `LayoutJob` rather than two widgets, so it drops into anything that takes a title —
/// window titles included, which can't host a custom-painted child.
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

/// One dock tile: a badge that lifts and lights up under the pointer, with a pill beneath it
/// when the app has windows open. The hover response is most of what separates a dock from a
/// row of pictures — it is the affordance saying these are pressable.
fn dock_item(
    ui: &mut egui::Ui,
    icon: &str,
    color: egui::Color32,
    tooltip: &str,
    running: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(DOCK_BADGE + 10.0, DOCK_BADGE + DOCK_LIFT + DOCK_GUTTER),
        egui::Sense::click(),
    );

    if ui.is_rect_visible(rect) {
        let t = ui
            .ctx()
            .animate_bool_responsive(response.id, response.hovered());
        let size = DOCK_BADGE + 5.0 * t;
        let badge = egui::Rect::from_center_size(
            egui::pos2(
                rect.center().x,
                rect.bottom() - DOCK_GUTTER - DOCK_BADGE / 2.0 - DOCK_LIFT * t,
            ),
            egui::vec2(size, size),
        );

        theme::paint_glow(ui.painter(), badge, color, t);
        theme::paint_badge(ui.painter(), badge, icon, color);

        if running {
            // Widens under the pointer: the same mark reads as "open" at rest and as "this is
            // the one you are about to raise" on hover.
            let pill = egui::Rect::from_center_size(
                egui::pos2(rect.center().x, rect.bottom() - 3.0),
                egui::vec2(5.0 + 13.0 * t, 3.0),
            );
            ui.painter()
                .rect_filled(pill, egui::CornerRadius::same(2), color);
        }
    }

    response.on_hover_text(tooltip)
}

fn status_text(text: String, p: &theme::Palette) -> egui::RichText {
    egui::RichText::new(text).small().color(p.text_dim)
}
