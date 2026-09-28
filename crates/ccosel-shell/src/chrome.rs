//! Window chrome: the frame and title bar the shell draws around every app window, in place of
//! egui's own (issue #42).
//!
//! egui's title bar can't be restyled far enough: it has no minimise or maximise button, and it
//! fills the active window with `widgets.open`, not the app's own colour. So app
//! windows are shown with `title_bar(false)`, and the bar here is the first thing in the body.
//! That costs egui's title-bar drag, so the drag is rebuilt from public API: [`dragged_pos`].

use egui::{Align, Color32, Id, Order, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, vec2};

use crate::theme;

pub const TITLE_BAR_HEIGHT: f32 = 38.0;
const TITLE_BUTTON: f32 = 26.0;
/// Space between the bar's edge and its first and last items.
const TITLE_INSET: f32 = 12.0;

/// What the user asked of a window through its title bar this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TitleActions {
    pub minimize: bool,
    pub maximize: bool,
    pub close: bool,
}

/// Minimised and maximised state. Position and size otherwise live where egui keeps them, in
/// the window's area and resize state.
#[derive(Debug, Default)]
pub struct Placement {
    /// Hidden from the desktop: the window is not shown, and the app gets no frames, until its
    /// dock item is clicked.
    pub minimized: bool,
    /// The rect to go back to, while maximised.
    maximized_from: Option<Rect>,
    /// The rect to go back to on the next frame, just after un-maximising.
    restore_to: Option<Rect>,
    /// The position to go back to on the frame after that. See [`Self::place`].
    reposition_to: Option<Pos2>,
}

impl Placement {
    pub fn is_maximized(&self) -> bool {
        self.maximized_from.is_some()
    }

    /// Maximise a window that currently fills `current`, or restore a maximised one. With no
    /// `current` (the window has never been shown) there is nothing to restore to, so it stays.
    pub fn toggle_maximized(&mut self, current: Option<Rect>) {
        if let Some(from) = self.maximized_from.take() {
            self.restore_to = Some(from);
        } else if current.is_some() {
            self.maximized_from = current;
        }
    }

    /// Pin `window` to `desktop` while maximised. On the frame after, pin it to its old rect
    /// once, which writes that size back into egui's resize state (a private type that can't
    /// be set any other way). That frame still keeps the old position on screen using the
    /// *maximised* size, which can push it left or up, so the frame after sets the position
    /// once more. From then on it is freely movable and resizable.
    fn place<'a>(&mut self, window: egui::Window<'a>, desktop: Rect) -> egui::Window<'a> {
        if self.is_maximized() {
            window.fixed_rect(desktop)
        } else if let Some(rect) = self.restore_to.take() {
            self.reposition_to = Some(rect.min);
            window.fixed_rect(rect)
        } else if let Some(pos) = self.reposition_to.take() {
            window.current_pos(pos)
        } else {
            window
        }
    }
}

/// One app window: Brutal frame, the shell's title bar, then `body` inside the window margin.
#[expect(
    clippy::too_many_arguments,
    reason = "each is one independent window property"
)]
pub fn show_window(
    ctx: &egui::Context,
    id: Id,
    placement: &mut Placement,
    desktop: Rect,
    default_size: Vec2,
    icon: &str,
    title: &str,
    highlight: Option<Color32>,
    body: impl FnOnce(&mut Ui),
) -> TitleActions {
    let handle = title_handle(id);
    // The margin goes on the body instead, so the title bar can run edge to edge.
    let frame = egui::Frame::window(&ctx.global_style()).inner_margin(0);
    let mut window = egui::Window::new(title)
        .id(id)
        .title_bar(false)
        .default_size(default_size)
        // Kept between the status bar and the dock: the title bar is the only way to move a
        // window, so it must never end up underneath either of them.
        .constrain_to(desktop)
        .frame(frame);
    if !placement.is_maximized()
        && let Some(pos) = dragged_pos(ctx, id, handle)
    {
        window = window.current_pos(pos);
    }
    window = placement.place(window, desktop);

    let mut actions = TitleActions::default();
    window.show(ctx, |ui| {
        let spacing = ui.spacing().item_spacing;
        ui.spacing_mut().item_spacing.y = 0.0;
        actions = title_bar(ui, handle, icon, title, highlight);
        ui.spacing_mut().item_spacing = spacing;

        // A window without egui's title bar is dragged from anywhere on its body, so a press
        // meant for the app would move the window. This claims body drags first; the app's own
        // widgets are added after it, so they sit on top and still get theirs.
        ui.interact(
            ui.available_rect_before_wrap(),
            id.with("body"),
            Sense::drag(),
        );

        egui::Frame::NONE
            .inner_margin(theme::tokens().margin)
            .show(ui, body);
    });

    if actions.maximize {
        placement.toggle_maximized(ctx.memory(|m| m.area_rect(id)));
    }
    if actions.minimize {
        placement.minimized = true;
    }
    actions
}

fn title_handle(window: Id) -> Id {
    window.with("title-bar")
}

/// Where the window's top-left should be this frame if its title bar is being dragged: where it
/// was when the press began, plus how far the pointer has moved since. Read before the window
/// is shown, because egui writes the area's position back when it ends, and measured from the
/// press so it neither lags a frame nor drifts with rounding.
fn dragged_pos(ctx: &egui::Context, window: Id, handle: Id) -> Option<Pos2> {
    let start_id = handle.with("drag-start");
    let dragging = ctx.read_response(handle).is_some_and(|r| r.dragged());
    let pointer = ctx.input(|i| Some((i.pointer.press_origin()?, i.pointer.latest_pos()?)));
    let (Some((origin, now)), true) = (pointer, dragging) else {
        ctx.data_mut(|d| d.remove::<Pos2>(start_id));
        return None;
    };
    let current = ctx.memory(|m| m.area_rect(window))?.min;
    let start = ctx.data_mut(|d| *d.get_temp_mut_or(start_id, current));
    Some(start + (now - origin))
}

/// The topmost of `windows` in egui's paint order: the one the user is working in. A window
/// that has not been painted yet is about to be raised to the top, so it wins.
pub fn active_window(ctx: &egui::Context, windows: &[Id]) -> Option<Id> {
    ctx.memory(|m| {
        let order: Vec<Id> = m
            .layer_ids()
            .filter(|l| l.order == Order::Middle)
            .map(|l| l.id)
            .collect();
        windows
            .iter()
            .rev()
            .find(|w| !order.contains(w))
            .or_else(|| order.iter().rev().find(|l| windows.contains(l)))
            .copied()
    })
}

#[derive(Clone, Copy)]
enum Glyph {
    Minimize,
    Maximize,
    Close,
}

/// The bar, with a rule under it: filled with `highlight` (the app's own colour, on the active
/// window) or white. Its icon, title and buttons are ink or white, whichever reads on the fill.
/// Dragging it moves the window, double-clicking it maximises, and `– □ ×` minimise, maximise
/// and close.
fn title_bar(
    ui: &mut Ui,
    handle: Id,
    icon: &str,
    title: &str,
    highlight: Option<Color32>,
) -> TitleActions {
    let t = theme::tokens();
    let (rect, _) =
        ui.allocate_exact_size(vec2(ui.available_width(), TITLE_BAR_HEIGHT), Sense::hover());
    let painter = ui.painter().clone();
    let fill = highlight.unwrap_or(t.surface);
    let fg = theme::contrast_color(fill);
    painter.rect_filled(rect, 0, fill);
    theme::paint_rule(&painter, rect, Align::Max);

    // Centred on the space above the rule.
    let mid = rect.center().y - t.stroke / 2.0;
    let mut actions = TitleActions::default();
    let mut right = rect.right() - TITLE_INSET + (TITLE_BUTTON - 10.0) / 2.0;
    for (glyph, name, hit) in [
        (Glyph::Close, "close", &mut actions.close),
        (Glyph::Maximize, "maximize", &mut actions.maximize),
        (Glyph::Minimize, "minimize", &mut actions.minimize),
    ] {
        let button = Rect::from_min_max(
            Pos2::new(right - TITLE_BUTTON, mid - TITLE_BUTTON / 2.0),
            Pos2::new(right, mid + TITLE_BUTTON / 2.0),
        );
        let r = ui.interact(button, handle.with(name), Sense::click());
        if r.hovered() {
            painter.rect_stroke(button, 0, Stroke::new(t.stroke, fg), StrokeKind::Inside);
        }
        paint_glyph(&painter, button.center(), glyph, fg);
        *hit = r.clicked();
        right = button.left() - 2.0;
    }

    let grip = rect.with_max_x(right);
    let r = ui.interact(grip, handle, Sense::click_and_drag());
    if r.double_clicked() {
        actions.maximize = true;
    }

    let painter = painter.with_clip_rect(grip);
    let mut x = rect.left() + TITLE_INSET;
    let icon = painter.text(
        Pos2::new(x, mid),
        egui::Align2::LEFT_CENTER,
        icon,
        egui::FontId::proportional(16.0),
        fg,
    );
    x = icon.right() + 8.0;
    painter.text(
        Pos2::new(x, mid),
        egui::Align2::LEFT_CENTER,
        title,
        theme::heading_font(15.0),
        fg,
    );

    actions
}

fn paint_glyph(painter: &egui::Painter, c: Pos2, glyph: Glyph, ink: Color32) {
    const HALF: f32 = 5.5;
    let stroke = Stroke::new(1.6, ink);
    match glyph {
        Glyph::Minimize => {
            painter.hline(c.x - HALF..=c.x + HALF, c.y, stroke);
        }
        Glyph::Maximize => {
            painter.rect_stroke(
                Rect::from_center_size(c, Vec2::splat(HALF * 2.0)),
                0,
                stroke,
                StrokeKind::Middle,
            );
        }
        Glyph::Close => {
            painter.line_segment([c + vec2(-HALF, -HALF), c + vec2(HALF, HALF)], stroke);
            painter.line_segment([c + vec2(-HALF, HALF), c + vec2(HALF, -HALF)], stroke);
        }
    }
}

#[cfg(test)]
mod tests {
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::*;

    /// Each test window's app colour: one light (ink text), one dark (white text).
    const COLORS: [Color32; 2] = [
        Color32::from_rgb(0xF5, 0x9E, 0x0B),
        Color32::from_rgb(0x1D, 0x4E, 0xD8),
    ];

    const SCREEN: Rect = Rect::from_min_max(Pos2::ZERO, Pos2::new(1200.0, 900.0));
    const DESKTOP: Rect = Rect::from_min_max(Pos2::new(0.0, 40.0), Pos2::new(1200.0, 820.0));

    /// A desktop with windows on it, driven one frame at a time.
    struct Rig {
        ctx: egui::Context,
        windows: Vec<(Id, Placement)>,
        /// What each window's title bar reported on the last frame.
        actions: Vec<TitleActions>,
        /// The shapes painted on the last frame.
        shapes: Vec<egui::epaint::ClippedShape>,
        /// Seconds since the rig started. Advanced every frame, or windows never finish
        /// fading in.
        time: f64,
        /// The size every window asks for when it opens.
        default_size: Vec2,
    }

    impl Rig {
        fn new(n: usize) -> Self {
            Self::with_default_size(n, vec2(300.0, 200.0))
        }

        fn with_default_size(n: usize, default_size: Vec2) -> Self {
            let ctx = egui::Context::default();
            theme::apply(&ctx);
            let windows = (0..n)
                .map(|i| (Id::new(("win", i)), Placement::default()))
                .collect();
            let mut rig = Self {
                ctx,
                windows,
                actions: Vec::new(),
                shapes: Vec::new(),
                time: 0.0,
                default_size,
            };
            // The first frame installs fonts and lays windows out; the second is the first
            // real one.
            rig.frame(vec![]);
            rig.frame(vec![]);
            rig
        }

        fn frame(&mut self, events: Vec<egui::Event>) {
            self.time += 0.1;
            let input = egui::RawInput {
                screen_rect: Some(SCREEN),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let ids: Vec<Id> = self
                .windows
                .iter()
                .filter(|(_, p)| !p.minimized)
                .map(|(id, _)| *id)
                .collect();
            let active = active_window(&self.ctx, &ids);
            let mut actions = Vec::new();
            let windows = &mut self.windows;
            let default_size = self.default_size;
            let mut out = self.ctx.run_ui(input, |ui| {
                for (i, (id, placement)) in windows.iter_mut().enumerate() {
                    if placement.minimized {
                        actions.push(TitleActions::default());
                        continue;
                    }
                    let a = show_window(
                        ui.ctx(),
                        *id,
                        placement,
                        DESKTOP,
                        default_size,
                        "@",
                        &format!("Window {i}"),
                        (active == Some(*id)).then_some(COLORS[i]),
                        // As the desktop does: the body claims the whole window.
                        |ui| {
                            egui::ScrollArea::vertical()
                                .auto_shrink([false, false])
                                .show(ui, |ui| ui.label("body"));
                        },
                    );
                    actions.push(a);
                }
            });
            out.textures_delta.clear();
            self.actions = actions;
            self.shapes = out.shapes;
        }

        fn rect(&self, i: usize) -> Rect {
            self.ctx
                .memory(|m| m.area_rect(self.windows[i].0))
                .expect("window was never shown")
        }

        /// The bar sits inside the window's outline.
        fn title_bar(&self, i: usize) -> Rect {
            let r = self.rect(i).shrink(theme::tokens().stroke);
            Rect::from_min_size(r.min, vec2(r.width(), TITLE_BAR_HEIGHT))
        }

        /// Where the `n`th title button from the right (0 = close) of window `i` is.
        fn title_button(&self, i: usize, n: usize) -> Pos2 {
            let bar = self.title_bar(i);
            let first =
                bar.right() - TITLE_INSET + (TITLE_BUTTON - 10.0) / 2.0 - TITLE_BUTTON / 2.0;
            Pos2::new(
                first - n as f32 * (TITLE_BUTTON + 2.0),
                bar.center().y - 1.0,
            )
        }

        fn press(&mut self, at: Pos2) {
            self.frame(vec![egui::Event::PointerMoved(at), button(at, true)]);
        }

        fn release(&mut self, at: Pos2) {
            self.frame(vec![egui::Event::PointerMoved(at), button(at, false)]);
        }

        fn click(&mut self, at: Pos2) {
            self.frame(vec![egui::Event::PointerMoved(at)]);
            self.frame(vec![button(at, true), button(at, false)]);
        }

        fn drag(&mut self, from: Pos2, by: Vec2) {
            self.press(from);
            for step in 1..=5 {
                let at = from + by * (step as f32 / 5.0);
                self.frame(vec![egui::Event::PointerMoved(at)]);
            }
            self.release(from + by);
            self.frame(vec![]);
        }

        fn filled_with(&self, rect: Rect, fill: Color32) -> bool {
            self.shapes.iter().any(
                |s| matches!(&s.shape, egui::Shape::Rect(r) if r.rect == rect && r.fill == fill),
            )
        }
    }

    fn button(pos: Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        }
    }

    #[wasm_bindgen_test]
    fn dragging_the_title_bar_moves_the_window_with_the_pointer() {
        let mut rig = Rig::new(1);
        let before = rig.rect(0);
        let grip = rig.title_bar(0).left_center() + vec2(60.0, 0.0);

        rig.drag(grip, vec2(120.0, 80.0));

        let after = rig.rect(0);
        assert_eq!(after.min - before.min, vec2(120.0, 80.0));
        assert_eq!(after.size(), before.size(), "moving must not resize");
    }

    #[wasm_bindgen_test]
    fn windows_stay_on_the_desktop_so_their_title_bar_stays_reachable() {
        let mut rig = Rig::new(1);
        assert!(
            DESKTOP.contains_rect(rig.rect(0)),
            "opened at {:?}",
            rig.rect(0)
        );

        let grip = rig.title_bar(0).left_center() + vec2(60.0, 0.0);
        rig.drag(grip, vec2(-500.0, -500.0));
        assert!(
            DESKTOP.contains_rect(rig.rect(0)),
            "dragged to {:?}",
            rig.rect(0)
        );
    }

    #[wasm_bindgen_test]
    fn a_window_taller_than_the_desktop_opens_below_the_status_bar() {
        // egui only keeps a window inside its area when it fits; one that doesn't is pushed up
        // past the top, under the status bar (#38). A small browser window does this to any app
        // with a tall default size.
        let mut rig = Rig::with_default_size(2, vec2(900.0, DESKTOP.height() + 200.0));
        for _ in 0..10 {
            rig.frame(vec![]);
        }
        for i in 0..2 {
            assert!(
                DESKTOP.contains_rect(rig.rect(i)),
                "window {i} opened at {:?}",
                rig.rect(i)
            );
        }
    }

    #[wasm_bindgen_test]
    fn dragging_the_body_does_not_move_the_window() {
        let mut rig = Rig::new(1);
        let before = rig.rect(0);
        let body = before.center() + vec2(0.0, 40.0);

        rig.drag(body, vec2(120.0, 80.0));

        assert_eq!(rig.rect(0), before);
    }

    #[wasm_bindgen_test]
    fn the_close_button_asks_to_close() {
        let mut rig = Rig::new(1);
        let close = rig.title_button(0, 0);
        rig.frame(vec![egui::Event::PointerMoved(close)]);
        rig.frame(vec![button(close, true), button(close, false)]);
        assert!(rig.actions[0].close);
        assert!(!rig.actions[0].minimize && !rig.actions[0].maximize);
    }

    #[wasm_bindgen_test]
    fn minimize_hides_the_window() {
        let mut rig = Rig::new(1);
        let minimize = rig.title_button(0, 2);
        rig.click(minimize);
        assert!(rig.windows[0].1.minimized);
    }

    #[wasm_bindgen_test]
    fn maximize_fills_the_desktop_and_a_second_click_restores_it() {
        let mut rig = Rig::new(1);
        let before = rig.rect(0);

        rig.click(rig.title_button(0, 1));
        rig.frame(vec![]);
        assert!(rig.windows[0].1.is_maximized());
        assert_eq!(rig.rect(0), DESKTOP);

        rig.click(rig.title_button(0, 1));
        rig.frame(vec![]);
        rig.frame(vec![]);
        assert!(!rig.windows[0].1.is_maximized());
        assert_eq!(rig.rect(0), before);

        // And it is an ordinary window again: it moves.
        let grip = rig.title_bar(0).left_center() + vec2(60.0, 0.0);
        rig.drag(grip, vec2(50.0, 30.0));
        assert_eq!(rig.rect(0).min - before.min, vec2(50.0, 30.0));
    }

    #[wasm_bindgen_test]
    fn a_maximized_window_does_not_move_when_dragged() {
        let mut rig = Rig::new(1);
        rig.click(rig.title_button(0, 1));
        rig.frame(vec![]);
        let grip = rig.title_bar(0).left_center() + vec2(60.0, 0.0);

        rig.drag(grip, vec2(120.0, 80.0));

        assert_eq!(rig.rect(0), DESKTOP);
    }

    #[wasm_bindgen_test]
    fn the_active_title_bar_takes_its_apps_colour_and_the_rest_are_white() {
        let mut rig = Rig::new(2);
        rig.frame(vec![]);
        let t = theme::tokens();
        // Window 1 was shown last, so it starts on top.
        assert!(rig.filled_with(rig.title_bar(1), COLORS[1]));
        assert!(rig.filled_with(rig.title_bar(0), t.surface));

        // Clicking window 0 raises it, and the colour moves with it, in window 0's own colour.
        let grip = rig.title_bar(0).left_center() + vec2(60.0, 0.0);
        rig.click(grip);
        rig.frame(vec![]);
        assert!(rig.filled_with(rig.title_bar(0), COLORS[0]));
        assert!(rig.filled_with(rig.title_bar(1), t.surface));
    }

    #[wasm_bindgen_test]
    fn the_title_reads_on_whatever_colour_the_bar_is() {
        let mut rig = Rig::new(2);
        rig.frame(vec![]);
        let title_color = |title: &str| {
            rig.shapes
                .iter()
                .find_map(|s| match &s.shape {
                    egui::Shape::Text(text) if text.galley.text() == title => {
                        Some(text.galley.job.sections[0].format.color)
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("{title:?} was not painted"))
        };
        // Window 1 is active on a dark blue bar; window 0 is inactive on white.
        assert_eq!(title_color("Window 1"), Color32::WHITE);
        assert_eq!(title_color("Window 0"), theme::tokens().ink);
    }

    #[wasm_bindgen_test]
    fn a_window_that_has_never_been_painted_counts_as_active() {
        let ctx = egui::Context::default();
        let fresh = Id::new("fresh");
        assert_eq!(active_window(&ctx, &[fresh]), Some(fresh));
        assert_eq!(active_window(&ctx, &[]), None);
    }

    #[wasm_bindgen_test]
    fn toggling_maximize_on_a_window_never_shown_does_nothing() {
        let mut p = Placement::default();
        p.toggle_maximized(None);
        assert!(!p.is_maximized());
    }
}
