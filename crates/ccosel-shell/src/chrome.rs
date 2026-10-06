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
    /// The app has had its say about the window's size ([`Self::size_once`]), or the window
    /// was put back where someone left it, so it's theirs to size from now on.
    sized: bool,
}

impl Placement {
    /// A window remembered from last time: back at `rect`, minimised or maximised as it was.
    /// For a maximised one, `rect` is where it goes when un-maximised.
    pub fn restored(rect: Rect, minimized: bool, maximized: bool) -> Self {
        Self {
            minimized,
            maximized_from: maximized.then_some(rect),
            // The first time it is shown it is pinned there once, the way un-maximising works.
            restore_to: (!maximized).then_some(rect),
            reposition_to: None,
            sized: true,
        }
    }

    /// Size the window, currently at `current`, so its content area is `content`, as the app
    /// asked: once, the first time it asks, and only once the window has been shown. Kept at
    /// the same top-left corner, moved back onto `desktop` if that takes it off the edge. See
    /// [`fit_window`] for the limits.
    pub fn size_once(&mut self, content: Vec2, current: Option<Rect>, desktop: Rect) {
        if self.sized || self.is_maximized() {
            return;
        }
        let Some(current) = current else {
            return;
        };
        self.sized = true;
        let size = fit_window(content, desktop.size());
        let mut rect = Rect::from_min_size(current.min, size);
        rect = rect.translate(vec2(
            (desktop.right() - rect.right()).min(0.0),
            (desktop.bottom() - rect.bottom()).min(0.0),
        ));
        rect = rect.translate(vec2(
            (desktop.left() - rect.left()).max(0.0),
            (desktop.top() - rect.top()).max(0.0),
        ));
        self.restore_to = Some(rect);
    }

    pub fn is_maximized(&self) -> bool {
        self.maximized_from.is_some()
    }

    /// While maximised, the rect it goes back to: the one to remember it by.
    pub fn normal_rect(&self) -> Option<Rect> {
        self.maximized_from
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

/// The most of the desktop, across and down, a window an app sizes ([`Placement::size_once`])
/// may take: room is left to see there is a desktop behind it.
pub const MOST_OF_DESKTOP: f32 = 0.8;
/// The longest side of the content an app asks for is brought up to at least this, so a tiny
/// picture doesn't make a window too small to use.
pub const SMALLEST_LONG_SIDE: f32 = 320.0;
/// And no side is smaller than this, so a 1×8000 strip still gets a window you can hold.
pub const SMALLEST_SIDE: f32 = 200.0;

/// The window size, title bar and margins included, for content of `content`'s shape on a
/// desktop of `desktop`: as big as asked if that fits in [`MOST_OF_DESKTOP`], else scaled down
/// to fit keeping its shape; scaled up if its longest side is under [`SMALLEST_LONG_SIDE`]; and
/// each side at least [`SMALLEST_SIDE`].
pub fn fit_window(content: Vec2, desktop: Vec2) -> Vec2 {
    let margin = 2.0 * f32::from(theme::tokens().margin);
    let chrome = vec2(margin, TITLE_BAR_HEIGHT + margin);
    let room = (desktop * MOST_OF_DESKTOP - chrome).max(Vec2::splat(SMALLEST_SIDE));
    let content = content.max(Vec2::splat(1.0));
    let mut scale = (room.x / content.x).min(room.y / content.y).min(1.0);
    let longest = content.max_elem() * scale;
    if longest < SMALLEST_LONG_SIDE {
        scale *= SMALLEST_LONG_SIDE / longest;
    }
    let fitted = (content * scale)
        .max(Vec2::splat(SMALLEST_SIDE))
        .min(room.max(Vec2::splat(SMALLEST_SIDE)));
    fitted + chrome
}

/// The part of `area` a window can fill and still have its hard `shadow` inside `area`.
fn within_shadow(area: Rect, shadow: egui::Shadow) -> Rect {
    let [x, y] = shadow.offset;
    Rect::from_min_max(
        area.min,
        area.max - vec2(f32::from(x).max(0.0), f32::from(y).max(0.0)),
    )
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
    let mut frame = egui::Frame::window(&ctx.global_style()).inner_margin(0);
    // A maximised window fills the desktop edge to edge. Its shadow would have nowhere to go
    // but under the dock and off the screen, so it has none.
    let bounds = if placement.is_maximized() {
        frame.shadow = egui::Shadow::NONE;
        desktop
    } else {
        within_shadow(desktop, frame.shadow)
    };
    let mut window = egui::Window::new(title)
        .id(id)
        .title_bar(false)
        .default_size(default_size)
        // Kept between the status bar and the dock: the title bar is the only way to move a
        // window, so it must never end up underneath either of them.
        .constrain_to(bounds)
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
        let maximized = placement.is_maximized();
        actions = title_bar(ui, handle, icon, title, highlight, maximized);
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
    /// On a maximised window's maximise button: two overlapping squares, the window it goes
    /// back to.
    Restore,
    Close,
}

/// The bar, with a rule under it: filled with `highlight` (the app's own colour, on the active
/// window) or white. Its icon, title and buttons are ink or white, whichever reads on the fill.
/// Dragging it moves the window, double-clicking it maximises, and `– □ ×` minimise, maximise
/// and close. On a `maximized` window the middle button shows [`Glyph::Restore`] instead.
fn title_bar(
    ui: &mut Ui,
    handle: Id,
    icon: &str,
    title: &str,
    highlight: Option<Color32>,
    maximized: bool,
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
        (
            if maximized {
                Glyph::Restore
            } else {
                Glyph::Maximize
            },
            "maximize",
            &mut actions.maximize,
        ),
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
        Glyph::Restore => {
            // The back square shows only above and to the right of the front one.
            const FRONT: f32 = HALF * 2.0 - 3.0;
            let front = Rect::from_min_size(c + vec2(-HALF, HALF - FRONT), Vec2::splat(FRONT));
            let back = front.translate(vec2(3.0, -3.0));
            painter.line_segment([back.left_top(), back.right_top()], stroke);
            painter.line_segment([back.right_top(), back.right_bottom()], stroke);
            painter.line_segment(
                [back.left_top(), Pos2::new(back.left(), front.top())],
                stroke,
            );
            painter.line_segment(
                [back.right_bottom(), Pos2::new(front.right(), back.bottom())],
                stroke,
            );
            painter.rect_stroke(front, 0, stroke, StrokeKind::Middle);
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
    }

    impl Rig {
        fn new(n: usize) -> Self {
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
                        vec2(300.0, 200.0),
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

        /// Every painted rect, including those nested in a `Shape::Vec`, which is how a
        /// window's frame and shadow are painted.
        fn painted_rects(&self) -> Vec<Rect> {
            fn walk(shape: &egui::Shape, out: &mut Vec<Rect>) {
                match shape {
                    egui::Shape::Vec(shapes) => {
                        for s in shapes {
                            walk(s, out);
                        }
                    }
                    egui::Shape::Rect(r) => out.push(r.rect),
                    _ => {}
                }
            }
            let mut out = Vec::new();
            for s in &self.shapes {
                walk(&s.shape, &mut out);
            }
            out
        }

        /// Every painted rect that reaches past `area`.
        fn painted_outside(&self, area: Rect) -> Vec<Rect> {
            self.painted_rects()
                .into_iter()
                .filter(|r| !area.contains_rect(*r))
                .collect()
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

    /// Dragged into the bottom-right corner, a window stops short by its shadow, so the
    /// shadow stays on the desktop too.
    #[wasm_bindgen_test]
    fn an_ordinary_window_keeps_its_shadow_on_the_desktop() {
        let mut rig = Rig::new(1);
        let grip = rig.title_bar(0).left_center() + vec2(60.0, 0.0);
        rig.drag(grip, vec2(2000.0, 2000.0));

        let [x, y] = theme::tokens().shadow.offset;
        let shadow = vec2(f32::from(x), f32::from(y));
        assert_eq!(rig.rect(0).max, DESKTOP.max - shadow);
        assert!(rig.painted_outside(DESKTOP).is_empty());
    }

    /// The fix for #82: a maximised window used to stop short of the right and bottom edges
    /// to leave room for its shadow, which showed as a gap.
    #[wasm_bindgen_test]
    fn a_maximized_window_fills_the_desktop_with_no_gap_and_no_shadow() {
        let mut rig = Rig::new(1);
        rig.click(rig.title_button(0, 1));
        rig.frame(vec![]);

        assert_eq!(rig.rect(0), DESKTOP);
        assert_eq!(
            rig.painted_outside(DESKTOP),
            Vec::<Rect>::new(),
            "nothing, its shadow included, spills past the desktop"
        );

        // Restored, it casts its shadow again.
        rig.click(rig.title_button(0, 1));
        rig.frame(vec![]);
        rig.frame(vec![]);
        assert!(
            !rig.painted_outside(rig.rect(0)).is_empty(),
            "a restored window has its shadow back"
        );
    }

    #[wasm_bindgen_test]
    fn dragging_the_body_does_not_move_the_window() {
        let mut rig = Rig::new(1);
        let before = rig.rect(0);
        let body = before.center() + vec2(0.0, 40.0);

        rig.drag(body, vec2(120.0, 80.0));

        assert_eq!(rig.rect(0), before);
    }

    /// The maximise button swaps to a restore glyph while the window is maximised, and back.
    #[wasm_bindgen_test]
    fn the_maximize_button_shows_restore_while_maximized() {
        let mut rig = Rig::new(1);
        let button =
            |rig: &Rig| Rect::from_center_size(rig.title_button(0, 1), Vec2::splat(TITLE_BUTTON));
        // The glyph is the one outlined square drawn inside the button.
        let square = |rig: &Rig| {
            let b = button(rig);
            let inside: Vec<Rect> = rig
                .painted_rects()
                .into_iter()
                .filter(|r| b.contains_rect(*r) && r.size() != b.size())
                .collect();
            assert_eq!(inside.len(), 1, "one square in the button: {inside:?}");
            inside[0].size()
        };
        let maximize = square(&rig);

        rig.click(rig.title_button(0, 1));
        rig.frame(vec![]);
        let restore = square(&rig);
        assert!(
            restore.x < maximize.x && restore.y < maximize.y,
            "restore's front square ({restore:?}) is smaller than maximise's ({maximize:?})"
        );

        rig.click(rig.title_button(0, 1));
        rig.frame(vec![]);
        rig.frame(vec![]);
        assert_eq!(square(&rig), maximize);
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

    /// A window reopened from a remembered desktop goes back where it was, and is an ordinary
    /// window after that.
    #[wasm_bindgen_test]
    fn a_restored_window_opens_where_it_was() {
        let mut rig = Rig::new(1);
        let was = Rect::from_min_size(egui::pos2(200.0, 150.0), vec2(320.0, 240.0));
        rig.windows[0].1 = Placement::restored(was, false, false);
        rig.frame(vec![]);
        rig.frame(vec![]);
        assert_eq!(rig.rect(0), was);
        assert_eq!(rig.windows[0].1.normal_rect(), None);

        let grip = rig.title_bar(0).left_center() + vec2(60.0, 0.0);
        rig.drag(grip, vec2(20.0, 10.0));
        assert_eq!(rig.rect(0).min - was.min, vec2(20.0, 10.0));
    }

    #[wasm_bindgen_test]
    fn a_restored_maximized_window_is_maximized_and_unmaximizes_to_where_it_was() {
        let mut rig = Rig::new(1);
        let was = Rect::from_min_size(egui::pos2(200.0, 150.0), vec2(320.0, 240.0));
        rig.windows[0].1 = Placement::restored(was, false, true);
        rig.frame(vec![]);
        assert!(rig.windows[0].1.is_maximized());
        assert_eq!(rig.rect(0), DESKTOP);
        assert_eq!(rig.windows[0].1.normal_rect(), Some(was));

        rig.click(rig.title_button(0, 1));
        rig.frame(vec![]);
        rig.frame(vec![]);
        assert_eq!(rig.rect(0), was);
    }

    /// The window's chrome: title bar and margins, around the content an app asks for.
    fn chrome() -> Vec2 {
        let m = 2.0 * f32::from(theme::tokens().margin);
        vec2(m, TITLE_BAR_HEIGHT + m)
    }

    #[wasm_bindgen_test]
    fn a_window_fits_its_content_within_the_desktop() {
        let desk = vec2(1200.0, 800.0);
        // Fits: exactly as asked, plus the chrome.
        assert_eq!(
            fit_window(vec2(640.0, 480.0), desk),
            vec2(640.0, 480.0) + chrome()
        );
        // Too big: scaled down to fit in 80% of the desktop, keeping its shape.
        let big = fit_window(vec2(4032.0, 3024.0), desk) - chrome();
        assert!(
            big.y <= desk.y * MOST_OF_DESKTOP - chrome().y + 0.01,
            "{big:?}"
        );
        assert!(
            (big.x / big.y - 4.0 / 3.0).abs() < 0.01,
            "shape kept: {big:?}"
        );
        // Tiny: scaled up to a usable window.
        assert_eq!(
            fit_window(vec2(1.0, 1.0), desk) - chrome(),
            Vec2::splat(SMALLEST_LONG_SIDE)
        );
        let small = fit_window(vec2(16.0, 8.0), desk) - chrome();
        assert_eq!(
            small,
            vec2(SMALLEST_LONG_SIDE, SMALLEST_SIDE),
            "and no side too thin"
        );
        // A strip: long side to fit, short side no thinner than the smallest.
        let strip = fit_window(vec2(1.0, 8000.0), desk) - chrome();
        assert_eq!(strip.x, SMALLEST_SIDE);
        assert!(strip.y <= desk.y * MOST_OF_DESKTOP);
        let wide = fit_window(vec2(65535.0, 1.0), desk) - chrome();
        assert_eq!(wide.y, SMALLEST_SIDE);
        assert!(wide.x <= desk.x * MOST_OF_DESKTOP);
    }

    #[wasm_bindgen_test]
    fn an_app_sizes_its_window_once_and_then_it_is_the_persons() {
        let mut rig = Rig::new(1);
        let before = rig.rect(0);
        let current = Some(before);
        rig.windows[0]
            .1
            .size_once(vec2(400.0, 200.0), current, DESKTOP);
        rig.frame(vec![]);
        rig.frame(vec![]);
        let sized = rig.rect(0);
        assert_eq!(sized.min, before.min, "where it was");
        assert_eq!(sized.size(), vec2(400.0, 200.0) + chrome());

        // Asked again, nothing changes: the person may have sized it since.
        rig.windows[0]
            .1
            .size_once(vec2(900.0, 100.0), Some(sized), DESKTOP);
        rig.frame(vec![]);
        rig.frame(vec![]);
        assert_eq!(rig.rect(0), sized);
    }

    #[wasm_bindgen_test]
    fn a_sized_window_is_kept_on_the_desktop() {
        let mut p = Placement::default();
        let near_edge = Rect::from_min_size(Pos2::new(1100.0, 700.0), vec2(300.0, 200.0));
        p.size_once(vec2(500.0, 300.0), Some(near_edge), DESKTOP);
        let rect = p.restore_to.unwrap();
        assert!(DESKTOP.contains_rect(rect), "{rect:?}");
        // Not before it has been shown, so it isn't sized from nowhere.
        let mut p = Placement::default();
        p.size_once(vec2(500.0, 300.0), None, DESKTOP);
        assert_eq!(p.restore_to, None);
        p.size_once(vec2(500.0, 300.0), Some(near_edge), DESKTOP);
        assert!(p.restore_to.is_some(), "and then it is");
    }

    #[wasm_bindgen_test]
    fn a_restored_window_keeps_the_size_it_was_left_at() {
        let was = Rect::from_min_size(Pos2::new(200.0, 150.0), vec2(320.0, 240.0));
        let mut p = Placement::restored(was, false, false);
        p.restore_to = None;
        p.size_once(vec2(1000.0, 1000.0), Some(was), DESKTOP);
        assert_eq!(p.restore_to, None);
    }

    #[wasm_bindgen_test]
    fn a_restored_minimized_window_stays_minimized() {
        let was = Rect::from_min_size(egui::pos2(200.0, 150.0), vec2(320.0, 240.0));
        assert!(Placement::restored(was, true, false).minimized);
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
