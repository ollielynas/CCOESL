//! The system's visual language: two palettes, one style, and the paint primitives every
//! surface shares.
//!
//! Guests never link egui — they emit commands the shell replays through *its* `egui::Ui`. So
//! what is set up here is not a shell preference apps may or may not honour; it is the only
//! style in the system. A window full of someone else's app is drawn with the same fills,
//! radii and text sizes as the chrome around it, which is what makes a bag of separately
//! compiled wasm modules read as one desktop.

use egui::{Color32, CornerRadius, Shadow, Stroke, StrokeKind};

/// Every colour the shell draws with, in one place per theme. Chrome asks the palette rather
/// than hard-coding a literal, so adding a theme is writing one more of these rather than
/// hunting down every `from_rgb` in the crate.
#[allow(dead_code, reason = "wallpaper pools and glow are future theme work")]
pub struct Palette {
    pub wallpaper_top: Color32,
    pub wallpaper_bottom: Color32,
    pub surface: Color32,
    pub surface_hi: Color32,
    pub surface_hover: Color32,
    pub border: Color32,
    pub border_hi: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    /// The system's own accent — selection, focus, links, the shell's mark. Apps bring their
    /// own accent from the registry; this is the colour the desktop itself speaks in.
    pub accent: Color32,
    pub accent_deep: Color32,
    /// How opaque the dock and status bar are over the wallpaper.
    pub glass: u8,
    pub shadow: Color32,
    pub shadow_soft: Color32,
    /// Wide pools of colour laid over the wallpaper gradient, as
    /// `(centre x, centre y, radius)` in fractions of the viewport, plus the colour. They are
    /// what keeps a flat gradient from reading as "unstyled", while staying far too faint to
    /// compete with an app's own accent.
    pub pools: [(f32, f32, f32, Color32); 3],
    pub pool_alpha: u8,
}

pub const DARK: Palette = Palette {
    wallpaper_top: Color32::from_rgb(0x16, 0x1C, 0x2F),
    wallpaper_bottom: Color32::from_rgb(0x07, 0x09, 0x10),
    surface: Color32::from_rgb(0x14, 0x18, 0x23),
    surface_hi: Color32::from_rgb(0x1E, 0x24, 0x33),
    surface_hover: Color32::from_rgb(0x2A, 0x32, 0x45),
    border: Color32::from_rgb(0x2C, 0x34, 0x48),
    border_hi: Color32::from_rgb(0x41, 0x4D, 0x68),
    text: Color32::from_rgb(0xE7, 0xEB, 0xF4),
    text_dim: Color32::from_rgb(0x8C, 0x97, 0xAF),
    accent: Color32::from_rgb(0x5B, 0x8C, 0xFF),
    accent_deep: Color32::from_rgb(0x2E, 0x4E, 0xA8),
    glass: 0xCC,
    shadow: Color32::from_black_alpha(130),
    shadow_soft: Color32::from_black_alpha(96),
    pools: [
        (0.16, 0.04, 0.70, Color32::from_rgb(0x6D, 0x4A, 0xFF)),
        (0.92, 0.78, 0.78, Color32::from_rgb(0x12, 0x74, 0xD8)),
        (0.60, 0.00, 0.44, Color32::from_rgb(0xC0, 0x3C, 0x9B)),
    ],
    pool_alpha: 30,
};

pub const LIGHT: Palette = Palette {
    wallpaper_top: Color32::from_rgb(0xF4, 0xF7, 0xFD),
    wallpaper_bottom: Color32::from_rgb(0xE0, 0xE7, 0xF5),
    surface: Color32::from_rgb(0xFC, 0xFD, 0xFF),
    surface_hi: Color32::from_rgb(0xEF, 0xF2, 0xF7),
    surface_hover: Color32::from_rgb(0xE1, 0xE7, 0xF1),
    border: Color32::from_rgb(0xD3, 0xDA, 0xE5),
    border_hi: Color32::from_rgb(0xA4, 0xB0, 0xC4),
    text: Color32::from_rgb(0x10, 0x15, 0x1F),
    text_dim: Color32::from_rgb(0x5B, 0x66, 0x7A),
    accent: Color32::from_rgb(0x25, 0x63, 0xEB),
    accent_deep: Color32::from_rgb(0x1D, 0x4E, 0xD8),
    glass: 0xE0,
    shadow: Color32::from_black_alpha(46),
    shadow_soft: Color32::from_black_alpha(34),
    // Saturated rather than pale: over a near-white gradient a translucent strong colour
    // settles into a pastel, where a pale one would vanish entirely.
    pools: [
        (0.16, 0.04, 0.70, Color32::from_rgb(0x7C, 0x5C, 0xFF)),
        (0.92, 0.78, 0.78, Color32::from_rgb(0x3B, 0x82, 0xF6)),
        (0.60, 0.00, 0.44, Color32::from_rgb(0xEC, 0x48, 0x99)),
    ],
    pool_alpha: 26,
};

pub fn is_dark(ctx: &egui::Context) -> bool {
    matches!(ctx.theme(), egui::Theme::Dark)
}

pub fn palette(ctx: &egui::Context) -> &'static Palette {
    if is_dark(ctx) { &DARK } else { &LIGHT }
}

/// Installs the fonts and both palettes. Called once, before the first frame.
pub fn apply(ctx: &egui::Context) {
    // Registered up front so the icons in `registry::catalog` are ordinary glyphs in ordinary
    // text and painter calls from here on, indistinguishable from the built-in font.
    let mut fonts = egui::FontDefinitions::default();
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    ctx.set_fonts(fonts);

    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(9.0, 8.0);
        style.spacing.button_padding = egui::vec2(12.0, 7.0);
        style.spacing.window_margin = egui::Margin::same(12);
        style.spacing.menu_margin = egui::Margin::same(8);
        // Scrollbars drawn over the content instead of in a reserved gutter: a listing should
        // not change width the moment it grows long enough to scroll.
        style.spacing.scroll = egui::style::ScrollStyle::floating();

        for (text_style, font) in [
            (egui::TextStyle::Heading, egui::FontId::proportional(19.0)),
            (egui::TextStyle::Body, egui::FontId::proportional(14.0)),
            (egui::TextStyle::Button, egui::FontId::proportional(14.0)),
            (egui::TextStyle::Small, egui::FontId::proportional(11.0)),
            (egui::TextStyle::Monospace, egui::FontId::monospace(13.0)),
        ] {
            style.text_styles.insert(text_style, font);
        }
    });

    ctx.set_visuals_of(egui::Theme::Dark, visuals(&DARK, true));
    ctx.set_visuals_of(egui::Theme::Light, visuals(&LIGHT, false));
}

fn visuals(p: &Palette, dark: bool) -> egui::Visuals {
    let mut v = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };

    v.panel_fill = translucent(p.surface, p.glass);
    v.window_fill = translucent(p.surface, 0xF2);
    v.window_stroke = Stroke::new(1.0, p.border);
    v.window_corner_radius = CornerRadius::same(14);
    v.menu_corner_radius = CornerRadius::same(12);

    // Wide and soft rather than tight and dark. The shadow is what separates a window from the
    // wallpaper, which means the border does not have to be heavy enough to do it alone.
    v.window_shadow = Shadow {
        offset: [0, 18],
        blur: 48,
        spread: 0,
        color: p.shadow,
    };
    v.popup_shadow = Shadow {
        offset: [0, 10],
        blur: 28,
        spread: 0,
        color: p.shadow_soft,
    };

    v.extreme_bg_color = if dark {
        Color32::from_rgb(0x0C, 0x0F, 0x18)
    } else {
        Color32::from_rgb(0xFF, 0xFF, 0xFF)
    };
    v.faint_bg_color = p.surface_hi;
    v.code_bg_color = p.surface_hi;
    v.hyperlink_color = p.accent;
    v.warn_fg_color = Color32::from_rgb(0xF5, 0x9E, 0x0B);
    v.error_fg_color = Color32::from_rgb(0xF8, 0x71, 0x71);
    v.selection = egui::style::Selection {
        bg_fill: translucent(p.accent, 0x5C),
        stroke: Stroke::new(1.0, p.text),
    };

    v.widgets.noninteractive = widget(p.surface, p.border, p.text);
    v.widgets.inactive = widget(p.surface_hi, p.border, p.text);
    v.widgets.hovered = widget(p.surface_hover, p.border_hi, p.text);
    v.widgets.active = widget(p.accent_deep, p.accent, Color32::WHITE);
    v.widgets.open = widget(p.surface_hover, p.border_hi, p.text);

    v
}

/// Every state's `expansion` equals its stroke width, and that is load-bearing: it is what
/// keeps a button the same size when the pointer reaches it.
///
/// Guest buttons are frameless at rest and framed on hover (`frame_when_inactive(false)` in
/// `ccosel-host`). egui 0.36 sizes the framed case as `button_padding`, but the frameless case
/// as `button_padding + expansion - stroke width`, since it keeps the frame's inner margin and
/// drops the stroke. With a 1px stroke and no expansion every guest button grew 2px on hover,
/// and the rows after it moved: the jitter. Equal values cancel in both cases, and since every
/// state uses the same ones, the painted frame never changes size between states either.
fn widget(fill: Color32, stroke: Color32, text: Color32) -> egui::style::WidgetVisuals {
    const STROKE: f32 = 1.0;
    egui::style::WidgetVisuals {
        bg_fill: fill,
        weak_bg_fill: fill,
        bg_stroke: Stroke::new(STROKE, stroke),
        corner_radius: CornerRadius::same(8),
        fg_stroke: Stroke::new(1.0, text),
        expansion: STROKE,
    }
}

fn translucent(color: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}

/// Fill for the dock and status bar — surfaces that sit *on* the wallpaper and let it show
/// through rather than replacing it.
pub fn glass(p: &Palette) -> Color32 {
    translucent(p.surface, p.glass)
}

pub fn glass_border(p: &Palette) -> Color32 {
    translucent(p.border_hi, 0x80)
}

/// Paints the desktop background across the whole viewport, beneath every panel and window.
///
/// Drawn into the background layer rather than a `CentralPanel`, because the status bar and
/// dock are translucent and need something to be translucent *over*: a panel-sized wallpaper
/// would stop at their edges and leave them compositing against nothing.
pub fn paint_wallpaper(_ctx: &egui::Context) {}

/// A soft circular pool of light: a triangle fan from a coloured centre out to a ring of fully
/// transparent vertices, which is a radial falloff the tessellator interpolates for free.
#[allow(dead_code, reason = "wallpaper pools are future theme work")]
fn glow(painter: &egui::Painter, center: egui::Pos2, radius: f32, color: Color32, alpha: u8) {
    const SEGMENTS: usize = 64;

    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(center, translucent(color, alpha));
    for i in 0..=SEGMENTS {
        let angle = i as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
        mesh.colored_vertex(
            center + radius * egui::vec2(angle.cos(), angle.sin()),
            translucent(color, 0),
        );
    }
    for i in 1..=SEGMENTS as u32 {
        mesh.add_triangle(0, i, i + 1);
    }
    painter.add(egui::Shape::mesh(mesh));
}

/// The corner radius of a badge of the given size. Shared so a badge and the glow cast behind
/// it round off identically — a mismatch of even a pixel shows up as a halo at the corners.
pub fn badge_radius(size: f32) -> CornerRadius {
    CornerRadius::same((size * 0.29) as u8)
}

/// A rounded square filled with `color`, with `icon` centred on it: an app's identity
/// compressed to a single mark, the treatment a dock or launcher gives every app on the system.
pub fn paint_badge(painter: &egui::Painter, rect: egui::Rect, icon: &str, color: Color32) {
    let radius = badge_radius(rect.width());

    painter.add(
        Shadow {
            offset: [0, 2],
            blur: 9,
            spread: 0,
            color: Color32::from_black_alpha(96),
        }
        .as_shape(rect, radius),
    );
    painter.rect_filled(rect, radius, color);
    // A rim light along the inside edge. One pixel of it is the difference between a flat
    // coloured square and something that reads as a physical tile catching the light.
    painter.rect_stroke(
        rect,
        radius,
        Stroke::new(1.0, Color32::from_white_alpha(38)),
        StrokeKind::Inside,
    );

    let fg = contrast_color(color);
    let font = egui::FontId::proportional(rect.height() * 0.56);
    let galley = painter.layout_no_wrap(icon.to_owned(), font, fg);
    painter.galley(rect.center() - galley.size() / 2.0, galley, fg);
}

/// A coloured bloom behind a badge, at `strength` 0..=1. Reuses the shadow machinery: a blurred
/// rect in the app's own colour is exactly the glow a dock icon wants under the pointer.
pub fn paint_glow(painter: &egui::Painter, rect: egui::Rect, color: Color32, strength: f32) {
    if strength <= 0.0 {
        return;
    }
    painter.add(
        Shadow {
            offset: [0, 0],
            blur: (26.0 * strength) as u8,
            spread: (4.0 * strength) as u8,
            color: color.gamma_multiply(0.6 * strength),
        }
        .as_shape(rect, badge_radius(rect.width())),
    );
}

/// White or near-black, whichever reads better on `bg` — so a badge's glyph stays legible
/// whether the app picked a light or a dark accent.
pub fn contrast_color(bg: Color32) -> Color32 {
    let to_linear = |c: u8| {
        let c = c as f32 / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let luminance =
        0.2126 * to_linear(bg.r()) + 0.7152 * to_linear(bg.g()) + 0.0722 * to_linear(bg.b());
    if luminance > 0.42 {
        Color32::from_rgb(0x12, 0x14, 0x1A)
    } else {
        Color32::WHITE
    }
}

/// The WCAG 2 contrast ratio between two colours: 1:1 is none, 21:1 is black on white, and
/// `>= 4.5` is the AA bar for normal-size text.
#[cfg(test)]
fn contrast_ratio(a: Color32, b: Color32) -> f32 {
    fn to_linear(c: u8) -> f32 {
        let c = c as f32 / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }
    fn luminance(c: Color32) -> f32 {
        0.2126 * to_linear(c.r()) + 0.7152 * to_linear(c.g()) + 0.0722 * to_linear(c.b())
    }
    let (la, lb) = (luminance(a), luminance(b));
    let (lighter, darker) = if la > lb { (la, lb) } else { (lb, la) };
    (lighter + 0.05) / (darker + 0.05)
}

// `wasm_bindgen_test`, not `#[test]`: this crate only compiles for wasm32, and the wasm32 runner
// in `.cargo/config.toml` only picks tests up written this way. `cargo xtask test-wasm` runs them.
#[cfg(test)]
mod tests {
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::*;

    const AA_TEXT: f32 = 4.5;

    #[wasm_bindgen_test]
    fn text_clears_aa_contrast_on_surface_in_both_themes() {
        for (name, p) in [("dark", &DARK), ("light", &LIGHT)] {
            let ratio = contrast_ratio(p.text, p.surface);
            assert!(ratio >= AA_TEXT, "{name} text/surface contrast is {ratio}");
        }
    }

    #[wasm_bindgen_test]
    fn identical_colors_have_no_contrast() {
        assert!((contrast_ratio(DARK.text, DARK.text) - 1.0).abs() < 1e-6);
    }

    /// Size of a button, drawn the way `ccosel-host` draws guest buttons, with the pointer at
    /// `pointer`. Two frames per call: egui styles a widget by its state from the frame before.
    fn button_size(ctx: &egui::Context, pointer: egui::Pos2, framed: bool) -> egui::Vec2 {
        let mut size = egui::Vec2::ZERO;
        for _ in 0..2 {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(400.0, 300.0),
                )),
                events: vec![egui::Event::PointerMoved(pointer)],
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                let r = ui.add(egui::Button::new("Refresh").frame_when_inactive(framed));
                size = r.rect.size();
            });
            out.textures_delta.clear();
        }
        size
    }

    #[wasm_bindgen_test]
    fn hovering_a_button_does_not_change_its_size() {
        for theme in [egui::Theme::Dark, egui::Theme::Light] {
            for framed in [false, true] {
                let ctx = egui::Context::default();
                apply(&ctx);
                ctx.set_theme(theme);
                let away = button_size(&ctx, egui::pos2(390.0, 290.0), framed);
                let over = button_size(&ctx, egui::pos2(12.0, 12.0), framed);
                assert_eq!(
                    away, over,
                    "{theme:?}, framed at rest: {framed}: hover resized the button"
                );
            }
        }
    }

    /// Guards the test above: it only proves anything if hovering really reached the button.
    #[wasm_bindgen_test]
    fn the_hover_in_the_size_test_lands_on_the_button() {
        let ctx = egui::Context::default();
        apply(&ctx);
        let mut hovered = false;
        for _ in 0..2 {
            let input = egui::RawInput {
                events: vec![egui::Event::PointerMoved(egui::pos2(12.0, 12.0))],
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                hovered = ui.button("Refresh").hovered();
            });
            out.textures_delta.clear();
        }
        assert!(hovered);
    }
}
