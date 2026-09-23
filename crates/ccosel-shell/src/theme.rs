//! The system's visual language: two palettes and the style built from them.
//!
//! Guests never link egui — they emit commands the shell replays through *its* `egui::Ui`. So
//! what is set up here is not a shell preference apps may or may not honour; it is the only
//! style in the system. A window full of someone else's app is drawn with the same fills,
//! strokes and radii as the chrome around it, which is what makes a bag of separately
//! compiled wasm modules read as one desktop.

use egui::{Color32, CornerRadius, Shadow, Stroke};

/// Every colour the shell draws with, in one place per theme. Chrome asks the palette rather
/// than hard-coding a literal, so adding a theme is writing one more of these rather than
/// hunting down every `from_rgb` in the crate.
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
    /// The system's own accent — selection, focus, links, the active-widget fill. Apps bring
    /// their own colour from the registry; this is the colour the desktop itself speaks in.
    pub accent: Color32,
    pub accent_deep: Color32,
    pub shadow: Color32,
    pub shadow_soft: Color32,
}

pub const DARK: Palette = Palette {
    // Unchanged from the pre-overhaul gradient: the ticket asks for a brighter, higher-contrast
    // desktop, but only in light mode — dark mode's wallpaper is explicitly out of scope here.
    wallpaper_top: Color32::from_rgb(0x1a, 0x1f, 0x2b),
    wallpaper_bottom: Color32::from_rgb(0x10, 0x12, 0x18),
    surface: Color32::from_rgb(0x14, 0x18, 0x23),
    surface_hi: Color32::from_rgb(0x1E, 0x24, 0x33),
    surface_hover: Color32::from_rgb(0x2A, 0x32, 0x45),
    border: Color32::from_rgb(0x2C, 0x34, 0x48),
    border_hi: Color32::from_rgb(0x41, 0x4D, 0x68),
    text: Color32::from_rgb(0xE7, 0xEB, 0xF4),
    text_dim: Color32::from_rgb(0x8C, 0x97, 0xAF),
    accent: Color32::from_rgb(0x5B, 0x8C, 0xFF),
    accent_deep: Color32::from_rgb(0x2E, 0x4E, 0xA8),
    shadow: Color32::from_black_alpha(130),
    shadow_soft: Color32::from_black_alpha(96),
};

pub const LIGHT: Palette = Palette {
    // Brighter and cooler than the previous `#dde4ef -> #c3cdff` haze: a near-white sky rather
    // than a grey one is most of what "higher contrast" means for the empty desktop itself.
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
    shadow: Color32::from_black_alpha(46),
    shadow_soft: Color32::from_black_alpha(34),
};

fn is_dark(ctx: &egui::Context) -> bool {
    matches!(ctx.theme(), egui::Theme::Dark)
}

pub fn palette(ctx: &egui::Context) -> &'static Palette {
    if is_dark(ctx) { &DARK } else { &LIGHT }
}

/// Installs the icon font and both palettes. Called once, before the first frame.
pub fn apply(ctx: &egui::Context) {
    // Registered up front so a `registry::AppEntry`'s icon, or any other Phosphor glyph, is an
    // ordinary character in an ordinary label from here on — nothing downstream has to know
    // the font exists.
    let mut fonts = egui::FontDefinitions::default();
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    ctx.set_fonts(fonts);

    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(9.0, 8.0);
        style.spacing.button_padding = egui::vec2(12.0, 7.0);
        style.spacing.window_margin = egui::Margin::same(12);
        style.spacing.menu_margin = egui::Margin::same(8);
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

    v.panel_fill = p.surface;
    v.window_fill = p.surface;
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

    v.extreme_bg_color = p.surface_hi;
    v.faint_bg_color = p.surface_hi;
    v.code_bg_color = p.surface_hi;
    v.hyperlink_color = p.accent;
    v.warn_fg_color = Color32::from_rgb(0xF5, 0x9E, 0x0B);
    v.error_fg_color = Color32::from_rgb(0xF8, 0x71, 0x71);
    v.selection = egui::style::Selection {
        bg_fill: translucent(p.accent, 0x5C),
        stroke: Stroke::new(1.0, p.text),
    };

    v.widgets.noninteractive = widget(p.surface, p.border, p.text, 0.0);
    v.widgets.inactive = widget(p.surface_hi, p.border, p.text, 0.0);
    v.widgets.hovered = widget(p.surface_hover, p.border_hi, p.text, 1.0);
    v.widgets.active = widget(p.accent_deep, p.accent, Color32::WHITE, 0.0);
    v.widgets.open = widget(p.surface_hover, p.border_hi, p.text, 0.0);

    v
}

fn widget(
    fill: Color32,
    stroke: Color32,
    text: Color32,
    expansion: f32,
) -> egui::style::WidgetVisuals {
    egui::style::WidgetVisuals {
        bg_fill: fill,
        weak_bg_fill: fill,
        bg_stroke: Stroke::new(1.0, stroke),
        corner_radius: CornerRadius::same(8),
        fg_stroke: Stroke::new(1.0, text),
        expansion,
    }
}

fn translucent(color: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}

/// Paints the desktop background across the whole viewport, beneath every panel and window.
///
/// Painted straight onto the background layer rather than through a `CentralPanel`, so it
/// still covers the full viewport regardless of what panels the desktop adds on top of it —
/// the taskbar included, which needs the wallpaper to already be there to draw its top border
/// against.
pub fn paint_wallpaper(ctx: &egui::Context) {
    let p = palette(ctx);
    let rect = ctx.viewport_rect();
    let painter = ctx.layer_painter(egui::LayerId::background());

    // A cheap vertical gradient: a handful of bands, no texture upload.
    const BANDS: u32 = 48;
    for i in 0..BANDS {
        let t = i as f32 / BANDS as f32;
        let band = egui::Rect::from_min_max(
            egui::pos2(rect.min.x, rect.min.y + rect.height() * t),
            egui::pos2(
                rect.max.x,
                rect.min.y + rect.height() * (t + 1.0 / BANDS as f32) + 1.0,
            ),
        );
        painter.rect_filled(
            band,
            0.0,
            lerp_color(p.wallpaper_top, p.wallpaper_bottom, t),
        );
    }
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t) as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

/// The WCAG 2 contrast ratio between two colours — 1:1 is no contrast at all, 21:1 is black on
/// white. `>= 4.5` is the AA bar for normal-size text, which is what makes "higher contrast"
/// in the ticket a claim a test can check rather than only a human eyeballing a screenshot.
///
/// `cfg(test)`-only: nothing at runtime picks a colour by contrast, so outside the palette
/// check below this would just be dead code the release build carries for nothing.
#[cfg(test)]
pub fn contrast_ratio(a: Color32, b: Color32) -> f32 {
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

// `wasm_bindgen_test`, not plain `#[test]`: this whole crate only compiles for `wasm32`, and
// the workspace's wasm32 test runner (`.cargo/config.toml`) only picks up tests written this
// way — see `crates/ccosel-host-web/tests/web.rs` for the same pattern. Run with
// `cargo test -p ccosel-shell --target wasm32-unknown-unknown` (or `cargo xtask test-wasm`).
#[cfg(test)]
mod tests {
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::*;

    /// The bar the ticket names: normal-size text on its own surface reads as AA-contrast in
    /// both themes, not just one.
    const AA_TEXT: f32 = 4.5;

    #[wasm_bindgen_test]
    fn dark_text_clears_aa_contrast_on_surface() {
        let ratio = contrast_ratio(DARK.text, DARK.surface);
        assert!(
            ratio >= AA_TEXT,
            "dark text/surface contrast is {ratio}, below {AA_TEXT}"
        );
    }

    #[wasm_bindgen_test]
    fn light_text_clears_aa_contrast_on_surface() {
        let ratio = contrast_ratio(LIGHT.text, LIGHT.surface);
        assert!(
            ratio >= AA_TEXT,
            "light text/surface contrast is {ratio}, below {AA_TEXT}"
        );
    }

    #[wasm_bindgen_test]
    fn identical_colors_have_no_contrast() {
        assert!((contrast_ratio(DARK.text, DARK.text) - 1.0).abs() < 1e-6);
    }
}
