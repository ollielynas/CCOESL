//! The desktop: wallpaper, window manager, status bar, dock.
//!
//! The shell owns everything egui persists — window geometry, z-order, focus, scroll offsets,
//! text state — which is exactly why apps can be one-frame-stale without the user ever seeing
//! it. An app never positions its own window; it fills one the desktop gave it.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use ccosel_connection::Background;
use ccosel_host::AppHost;
use ccosel_host_web::{WebHost, WebInstance};
use ccosel_transport::{PendingKey, Transport};
use js_sys::WebAssembly;

use crate::http_wire::{HttpWire, Inbox};

use crate::app_window::AppWindow;
use crate::background;
use crate::chrome;
use crate::fetch;
use crate::fullscreen;
use crate::registry::{AppEntry, catalog};
use crate::theme;
use crate::upload::{self, Drops, Uploads};

/// Dock badge geometry, shared between `dock_item` (which paints it) and `app_menu` (which
/// needs to know the dock's on-screen height so its popup can sit above it without overlapping).
const DOCK_BADGE: f32 = 40.0;
const DOCK_LIFT: f32 = 4.0;
/// Room under the badge for its hard shadow, plus the headroom the lift needs.
const DOCK_GUTTER: f32 = 8.0;
/// Matches the dock frame's vertical `inner_margin`.
const DOCK_FRAME_MARGIN_V: f32 = 6.0;
/// The dock's total on-screen height: badge, lift headroom, shadow gutter, and the frame's
/// vertical margin on both edges.
const DOCK_HEIGHT: f32 = DOCK_BADGE + DOCK_LIFT + DOCK_GUTTER + DOCK_FRAME_MARGIN_V * 2.0;
/// One dock tile's width, and the gap between tiles.
const DOCK_ITEM_WIDTH: f32 = DOCK_BADGE + 10.0;
const DOCK_ITEM_GAP: f32 = 4.0;
/// The rule between the launcher and the open windows, with its margins.
const DOCK_DIVIDER_WIDTH: f32 = 14.0;
/// Clear space between the dock's top edge and the app menu popup above it.
const MENU_GAP: f32 = 12.0;
/// Space between the bars' edges and their content.
const BAR_PAD: f32 = 14.0;
/// The status bar's upload progress bar.
const UPLOAD_BAR_WIDTH: f32 = 90.0;

/// Where the wallpaper image is served from. A plain static file under `web/`, alongside
/// `index.html` — `ServeDir` serves it with no server changes needed.
pub const WALLPAPER_URL: &str = "/wallpaper.jpg";

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
    /// Image or shapes (issue #3), decided once from the connection at boot rather than
    /// re-checked every frame: `navigator.connection` can change mid-session, but re-deciding
    /// continuously would mean a wallpaper that flickers between modes as the estimate jitters.
    background: Background,
    /// Set once the wallpaper image has been fetched and decoded, if `background` is
    /// [`Background::Image`]. `None` either means it is still loading or that mode isn't
    /// active — [`Self::wallpaper`] falls back to the drawn shapes in both cases, so a slow or
    /// failed fetch degrades gracefully instead of leaving a blank background.
    wallpaper_texture: Option<egui::TextureHandle>,
    /// Landing spot for the async wallpaper fetch, mirroring `inbox`/`Launch` above: the fetch
    /// resolves outside the frame loop, so its result is picked up at the top of the next one.
    wallpaper_pending: Rc<RefCell<Option<Result<egui::ColorImage, String>>>>,
    /// Whether the app menu popup is open.
    app_menu_open: bool,
    /// Folder uploads started from apps' `UploadFolder` buttons, or by a drop on their window.
    uploads: Uploads,
    /// Files and folders dropped on the page, until the frame routes them to a window.
    drops: Drops,
    /// The last finished upload's summary and when to stop showing it, so "Uploaded photos:
    /// 12 files" stays in the status bar long enough to read.
    upload_notice: Option<(String, f64)>,
}

impl Desktop {
    pub fn new(egui_ctx: egui::Context) -> Self {
        theme::apply(&egui_ctx);

        let replies: Inbox = Rc::new(RefCell::new(Vec::new()));
        let wire = HttpWire::new("/rpc", replies.clone(), egui_ctx.clone());
        let background = background::detect();

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
            background,
            wallpaper_texture: None,
            wallpaper_pending: Rc::new(RefCell::new(None)),
            app_menu_open: false,
            uploads: Uploads::default(),
            drops: Drops::default(),
            upload_notice: None,
        };
        if let Err(e) = desktop.drops.listen(desktop.egui_ctx.clone()) {
            desktop
                .errors
                .push(format!("drag and drop is unavailable: {e:?}"));
        }
        // Open something on first boot: an empty desktop with no affordance is a worse first
        // impression than a window the user can close.
        if let Some(first) = desktop.registry.first().cloned() {
            desktop.launch(&first);
        }
        if desktop.background == Background::Image {
            desktop.load_wallpaper();
        }
        desktop
    }

    /// Kicks off the async fetch+decode for the wallpaper image. Only called when `background`
    /// is [`Background::Image`] — on a weak connection nothing here ever runs, which is the
    /// whole point of deciding first and fetching second.
    fn load_wallpaper(&self) {
        let pending = self.wallpaper_pending.clone();
        let ctx = self.egui_ctx.clone();
        let max_side = ctx.input(|i| i.max_texture_side);
        wasm_bindgen_futures::spawn_local(async move {
            let result = background::fetch_wallpaper(WALLPAPER_URL, max_side).await;
            *pending.borrow_mut() = Some(result);
            // The fetch resolved outside the frame loop, so nothing would redraw on its own.
            ctx.request_repaint();
        });
    }

    /// Picks up the wallpaper fetch's result, if it has landed since the last frame, and turns
    /// a decoded image into a GPU texture. Mirrors `drain_inbox` below for the same reason: the
    /// async work finishes outside the frame loop.
    fn drain_wallpaper(&mut self) {
        let Some(result) = self.wallpaper_pending.borrow_mut().take() else {
            return;
        };
        match result {
            Ok(image) => {
                let texture =
                    self.egui_ctx
                        .load_texture("wallpaper", image, egui::TextureOptions::default());
                self.wallpaper_texture = Some(texture);
            }
            Err(err) => self.errors.push(format!("wallpaper: {err}")),
        }
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
        self.drain_wallpaper();
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
        self.drain_uploads(now_ms);
        self.route_drops(&ctx);

        self.paint_background(&ctx);
        self.status_bar(&ctx);
        self.empty_state(ui);
        self.app_menu(&ctx);
        self.dock(&ctx);

        let desktop = desktop_rect(&ctx);
        let shown: Vec<egui::Id> = self
            .windows
            .iter()
            .filter(|w| !w.placement.minimized)
            .map(|w| window_id(w.instance_id))
            .collect();
        let active = chrome::active_window(&ctx, &shown);

        // Closing a window drops the instance, which is the only way to reclaim a guest's
        // memory — wasm linear memory cannot shrink, so a live instance holds its high-water
        // mark forever.
        for window in &mut self.windows {
            // A minimised app gets no frames, the same as a suspended one: its replies wait in
            // its queue until its dock item brings it back.
            if window.placement.minimized {
                continue;
            }
            let id = window_id(window.instance_id);
            let mut placement = std::mem::take(&mut window.placement);
            let actions = chrome::show_window(
                &ctx,
                id,
                &mut placement,
                desktop,
                window.default_size.into(),
                window.icon,
                &window.title.clone(),
                // The active window's bar is filled with its app's colour, the same as its dock
                // badge, so the two read as one thing.
                (active == Some(id)).then_some(window.color),
                |ui| {
                    // `auto_shrink(false)` claims the whole window body regardless of how much
                    // the app actually drew, so dragging the window bigger than its content
                    // leaves blank space rather than the window snapping back to fit. It also
                    // means a long listing scrolls instead of growing the window without bound.
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| window.ui(ui));
                },
            );
            window.placement = placement;

            // Acted on in the frame the click is drawn: browsers only open a picker or a tab
            // in response to a user action, and the next frame could be too late.
            if let Some((widget, dest)) = window.clicked_upload() {
                self.uploads
                    .start(window.instance_id, widget, dest, ctx.clone());
            }
            if let Some(widget) = window.clicked_project_upload() {
                self.uploads
                    .start_project(window.instance_id, widget, ctx.clone());
            }
            if let Some(url) = window.clicked_open_url()
                && let Err(e) = upload::open_url(&url)
            {
                self.errors.push(format!("{}: {e}", window.title));
            }

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

            if actions.close {
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

    /// Hand each drop to the window it landed on, which uploads it the way its own upload button
    /// would. Where it landed is looked up in last frame's layers, which is what was on screen
    /// when the user let go.
    fn route_drops(&mut self, ctx: &egui::Context) {
        for dropped in self.drops.take() {
            let layer = ctx.layer_id_at(dropped.pos).map(|l| l.id);
            let window = self
                .windows
                .iter()
                .find(|w| !w.placement.minimized && Some(window_id(w.instance_id)) == layer);
            let Some(window) = window else {
                self.errors
                    .push("upload: drop files onto an app window to upload them".to_owned());
                continue;
            };
            match window.drop_target() {
                Some(target) => self.uploads.start_dropped(
                    window.instance_id,
                    target,
                    dropped.entries,
                    ctx.clone(),
                ),
                None => self
                    .errors
                    .push(format!("upload: {} doesn't take uploads", window.title)),
            }
        }
    }

    /// Tell each finished upload's window, so its app can re-list the folder, and make the
    /// outcome visible: a summary in the status bar, and every failed file in the error list.
    fn drain_uploads(&mut self, now_ms: f64) {
        for done in self.uploads.take_finished() {
            if let Some(w) = self
                .windows
                .iter_mut()
                .find(|w| w.instance_id == done.instance)
            {
                // A project upload that failed before it had a folder tells the app nothing:
                // there's nothing to build, and the error list says why.
                match (done.project, done.scratch) {
                    (true, Some(id)) => w.project_uploaded(done.widget, id),
                    (true, None) => {}
                    (false, _) => w.upload_finished(done.widget),
                }
            }
            for f in &done.failures {
                self.errors.push(format!("upload: {f}"));
            }
            self.upload_notice = Some((done.status(), now_ms + 6000.0));
            // Nothing else may redraw by then, and the notice must still go away.
            self.egui_ctx
                .request_repaint_after(std::time::Duration::from_millis(6100));
        }
        if self
            .upload_notice
            .as_ref()
            .is_some_and(|(_, until)| now_ms > *until)
        {
            self.upload_notice = None;
        }
    }

    /// Issue #3: the photo on a connection that can afford it, drawn shapes otherwise. `Image`
    /// mode shows the shapes too while the photo is loading or if it failed, so the desktop is
    /// never bare.
    fn paint_background(&self, ctx: &egui::Context) {
        let painter = ctx.layer_painter(egui::LayerId::background());
        let rect = ctx.viewport_rect();
        match (self.background, self.wallpaper_texture.as_ref()) {
            (Background::Image, Some(texture)) => background::paint_image(&painter, rect, texture),
            _ => background::draw_shapes(&painter, rect, theme::is_dark(ctx)),
        }
    }

    /// The menu bar: the shell's own mark on the left, system health on the right. White, with
    /// an ink rule along the edge that faces the desktop.
    fn status_bar(&self, ctx: &egui::Context) {
        let t = theme::tokens();
        let width = ctx.viewport_rect().width();

        let frame = egui::Frame::NONE
            .fill(t.surface)
            .inner_margin(egui::Margin::symmetric(BAR_PAD as i8, 5));

        egui::Area::new(egui::Id::new("status-bar"))
            .anchor(egui::Align2::LEFT_TOP, egui::vec2(0.0, 0.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                let bar = frame.show(ui, |ui| {
                    ui.set_width(width - BAR_PAD * 2.0);
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(egui_phosphor::regular::SQUARES_FOUR)
                                .size(15.0)
                                .color(t.ink),
                        );
                        ui.label(
                            egui::RichText::new("CCOSEL")
                                .font(theme::heading_font(14.0))
                                .color(t.ink),
                        );

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
                            // Frameless, like a tray icon: a full Brutal button would make the
                            // bar twice as tall.
                            if ui
                                .add(
                                    egui::Button::new(
                                        egui::RichText::new(icon).size(14.0).color(t.ink),
                                    )
                                    .frame(false),
                                )
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
                            ui.label(status_text(format!("{bytes} B/frame")));
                            ui.label(status_text("\u{00b7}".to_owned()));
                            ui.label(status_text(format!("{} running", self.windows.len())));
                            let upload = self
                                .uploads
                                .status()
                                .or_else(|| self.upload_notice.as_ref().map(|(s, _)| s.clone()));
                            if upload.is_some() {
                                ui.label(status_text("\u{00b7}".to_owned()));
                            }
                            // Right to left, so the bar goes in before the text: on the text's
                            // right it stays put while the text changes width, where on its
                            // left it would jiggle with every file.
                            if let Some(fraction) = self.uploads.progress() {
                                ui.add(
                                    egui::ProgressBar::new(fraction)
                                        .desired_width(UPLOAD_BAR_WIDTH)
                                        .desired_height(8.0),
                                );
                            }
                            if let Some(text) = upload {
                                ui.label(egui::RichText::new(text).small().color(t.ink));
                            }
                        });
                    });
                });
                theme::paint_rule(ui.painter(), bar.response.rect, egui::Align::Max);
            });
    }

    /// What the desktop says when nothing is open. A blank screen with no affordance is a
    /// worse first impression than one line pointing at the dock.
    ///
    /// On a card, like a window: straight onto the wallpaper, text can land on any part of a
    /// photo and be unreadable.
    fn empty_state(&self, ui: &mut egui::Ui) {
        let t = theme::tokens();
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                if !self.windows.is_empty() || *self.pending.borrow() > 0 {
                    return;
                }
                let area = ui.max_rect();
                let card = egui::Rect::from_center_size(
                    egui::pos2(area.center().x, area.top() + area.height() * 0.42),
                    egui::vec2(300.0, 180.0),
                );
                let painter = ui.painter();
                painter.add(t.shadow.as_shape(card, 0));
                painter.rect(
                    card,
                    0,
                    t.surface,
                    egui::Stroke::new(t.stroke, t.ink),
                    egui::StrokeKind::Inside,
                );
                ui.scope_builder(egui::UiBuilder::new().max_rect(card.shrink(24.0)), |ui| {
                    ui.vertical_centered(|ui| {
                        ui.label(
                            egui::RichText::new(egui_phosphor::regular::SQUARES_FOUR)
                                .size(40.0)
                                .color(t.ink),
                        );
                        ui.add_space(4.0);
                        ui.label(
                            egui::RichText::new("CCOSEL")
                                .font(theme::heading_font(t.heading))
                                .color(t.ink),
                        );
                        ui.label(
                            egui::RichText::new("Pick an app from the dock").color(t.text_dim),
                        );
                    });
                });
            });
    }

    /// The app menu: a popup above the dock listing every installed app with its icon and
    /// name. Clicking one launches it and closes the menu.
    fn app_menu(&mut self, ctx: &egui::Context) {
        if !self.app_menu_open {
            return;
        }
        let t = theme::tokens();
        let mut to_launch: Option<AppEntry> = None;

        let frame = egui::Frame::NONE
            .fill(t.surface)
            .stroke(egui::Stroke::new(t.stroke, t.ink))
            .inner_margin(egui::Margin::same(10))
            .shadow(t.shadow);

        // Anchored a fixed gap above the dock's top edge, computed from the dock's real
        // geometry rather than a guessed constant, so the two can never drift into overlapping.
        let menu_bottom_offset = DOCK_HEIGHT + MENU_GAP;

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
                        ui.spacing_mut().item_spacing.y = 0.0;
                        for entry in &self.registry {
                            // Allocate the *whole* row as one click target first, then paint the
                            // badge and label into it, so there is no separate `ui.label` widget
                            // sitting on top able to swallow the click before the row sees it —
                            // any point in the row launches the app, not just the icon glyph.
                            let row_size = egui::vec2(ui.available_width(), 38.0);
                            let (rect, response) =
                                ui.allocate_exact_size(row_size, egui::Sense::click());

                            if ui.is_rect_visible(rect) {
                                if response.hovered() {
                                    ui.painter().rect_filled(rect, 0, t.hover);
                                }

                                let badge_rect = egui::Rect::from_center_size(
                                    egui::pos2(rect.min.x + 17.0, rect.center().y - 1.5),
                                    egui::vec2(26.0, 26.0),
                                );
                                theme::paint_badge(
                                    ui.painter(),
                                    badge_rect,
                                    entry.icon,
                                    entry.color,
                                    !response.is_pointer_button_down_on(),
                                );

                                ui.painter().text(
                                    egui::pos2(rect.min.x + 42.0, rect.center().y),
                                    egui::Align2::LEFT_CENTER,
                                    entry.name,
                                    egui::FontId::proportional(t.body),
                                    t.ink,
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

    /// The dock: a launcher on the left, then every open window, centred in a white bar along
    /// the bottom with an ink rule on top. The launcher opens a popup listing all installed
    /// apps. Clicking a window raises it, bringing it back first if it was minimised.
    ///
    /// An area rather than a panel, so windows pass underneath it instead of the desktop
    /// permanently losing a full-width strip of height to it.
    fn dock(&mut self, ctx: &egui::Context) {
        let t = theme::tokens();
        let width = ctx.viewport_rect().width();
        let mut to_focus: Option<u64> = None;

        let frame = egui::Frame::NONE
            .fill(t.surface)
            .inner_margin(egui::Margin::symmetric(
                BAR_PAD as i8,
                DOCK_FRAME_MARGIN_V as i8,
            ));

        // Tiles are a fixed width, so the row's width is known before it is laid out, which
        // is what lets it be centred without a frame's delay.
        let items = 1 + self.windows.len();
        let divider = if self.windows.is_empty() {
            0.0
        } else {
            DOCK_DIVIDER_WIDTH + DOCK_ITEM_GAP
        };
        let row = items as f32 * DOCK_ITEM_WIDTH + (items - 1) as f32 * DOCK_ITEM_GAP + divider;
        let inner = width - BAR_PAD * 2.0;

        egui::Area::new(egui::Id::new("dock"))
            .anchor(egui::Align2::CENTER_BOTTOM, egui::Vec2::ZERO)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                let bar = frame.show(ui, |ui| {
                    ui.set_width(inner);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = DOCK_ITEM_GAP;
                        ui.add_space(((inner - row) / 2.0).max(0.0));

                        // Launcher: opens the app menu.
                        let launcher =
                            dock_item(ui, egui_phosphor::regular::SQUARES_FOUR, t.ink, "All apps");
                        if launcher.clicked() {
                            self.app_menu_open = !self.app_menu_open;
                        }

                        if !self.windows.is_empty() {
                            let (rect, _) = ui.allocate_exact_size(
                                egui::vec2(DOCK_DIVIDER_WIDTH, DOCK_BADGE),
                                egui::Sense::hover(),
                            );
                            ui.painter().vline(
                                rect.center().x,
                                rect.y_range(),
                                egui::Stroke::new(t.stroke, t.ink),
                            );
                            for window in &self.windows {
                                let item = dock_item(ui, window.icon, window.color, &window.title);
                                if item.clicked() {
                                    to_focus = Some(window.instance_id);
                                }
                            }
                        }
                    });
                });
                theme::paint_rule(ui.painter(), bar.response.rect, egui::Align::Min);
            });

        if let Some(id) = to_focus {
            if let Some(w) = self.windows.iter_mut().find(|w| w.instance_id == id) {
                w.placement.minimized = false;
            }
            // egui tracks z-order per area, so "focus" is just moving that area to the top.
            ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, window_id(id)));
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

fn window_id(instance_id: u64) -> egui::Id {
    egui::Id::new(("app-window", instance_id))
}

/// Where windows live: between the status bar and the dock. A maximised window fills it,
/// short of its own shadow so that stays on screen.
fn desktop_rect(ctx: &egui::Context) -> egui::Rect {
    let mut rect = ctx.content_rect();
    ctx.memory(|m| {
        if let Some(bar) = m.area_rect(egui::Id::new("status-bar")) {
            rect.min.y = bar.max.y;
        }
        if let Some(dock) = m.area_rect(egui::Id::new("dock")) {
            rect.max.y = dock.min.y;
        }
    });
    let [x, y] = theme::tokens().shadow.offset;
    rect.max -= egui::vec2(f32::from(x), f32::from(y));
    rect
}

/// One dock tile: a square badge with a hard shadow, which lifts under the pointer and loses
/// its shadow while pressed. The hover response is most of what separates a dock from a row of
/// pictures: it is the affordance saying these are pressable.
fn dock_item(ui: &mut egui::Ui, icon: &str, color: egui::Color32, tooltip: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(DOCK_ITEM_WIDTH, DOCK_BADGE + DOCK_LIFT + DOCK_GUTTER),
        egui::Sense::click(),
    );

    if ui.is_rect_visible(rect) {
        let t = ui
            .ctx()
            .animate_bool_responsive(response.id, response.hovered());
        let badge = egui::Rect::from_center_size(
            egui::pos2(
                rect.center().x,
                rect.bottom() - DOCK_GUTTER - DOCK_BADGE / 2.0 - DOCK_LIFT * t,
            ),
            egui::vec2(DOCK_BADGE, DOCK_BADGE),
        );
        theme::paint_badge(
            ui.painter(),
            badge,
            icon,
            color,
            !response.is_pointer_button_down_on(),
        );
    }

    response.on_hover_text(tooltip)
}

fn status_text(text: String) -> egui::RichText {
    egui::RichText::new(text)
        .small()
        .color(theme::tokens().text_dim)
}
