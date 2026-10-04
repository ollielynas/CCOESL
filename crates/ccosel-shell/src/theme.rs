//! The system's visual language: the Brutal style (issue #42), and the paint primitives every
//! surface shares.
//!
//! Guests never link egui — they emit commands the shell replays through *its* `egui::Ui`. So
//! what is set up here is not a shell preference apps may or may not honour; it is the only
//! style in the system. A window full of someone else's app is drawn with the same fills,
//! outlines and text sizes as the chrome around it, which is what makes a bag of separately
//! compiled wasm modules read as one desktop.
//!
//! The spec is `docs/style-guides/brutal.md`: black 2px outlines, square corners, hard offset
//! shadows with no blur, one highlighter-yellow accent, Space Grotesk and Space Mono.

use std::sync::Arc;

use egui::style::{Selection, WidgetVisuals};
use egui::{
    Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Shadow, Stroke,
    StrokeKind, TextStyle, Vec2, vec2,
};

/// Every value the style is made of, in one place. Chrome asks for a token rather than
/// hard-coding a literal, so the look can change here without hunting through the crate.
pub struct Tokens {
    /// Window bodies, bars and popups.
    pub surface: Color32,
    /// Text fields, list rows and groups.
    pub surface_alt: Color32,
    /// A widget under the pointer.
    pub hover: Color32,
    /// Outlines, rules and shadows. Brutal uses one ink for all three and for text.
    pub ink: Color32,
    pub text_dim: Color32,
    /// "This one": the selection and the pressed button. (The active window is marked in its
    /// own app's colour instead.)
    pub accent: Color32,
    pub on_accent: Color32,
    pub danger: Color32,
    #[allow(
        dead_code,
        reason = "no egui visual takes it; it is for the SDK status widgets"
    )]
    pub success: Color32,
    pub warn: Color32,
    pub stroke: f32,
    /// Windows and popups.
    pub shadow: Shadow,
    /// How far a button's, badge's or text field's hard shadow sits down and right of it.
    pub widget_shadow: f32,
    pub body: f32,
    pub heading: f32,
    pub small: f32,
    pub mono: f32,
    pub pad: Vec2,
    pub spacing: Vec2,
    pub margin: i8,
}

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

const INK: Color32 = hex(0x111111);

pub const BRUTAL: Tokens = Tokens {
    surface: Color32::WHITE,
    surface_alt: Color32::WHITE,
    hover: hex(0xFFF3B0),
    ink: INK,
    text_dim: hex(0x5A5A5A),
    accent: hex(0xFFD400),
    on_accent: INK,
    danger: hex(0xFF4D2E),
    success: hex(0x1FAF5A),
    warn: hex(0xFF9F1C),
    stroke: 2.0,
    shadow: Shadow {
        offset: [7, 7],
        blur: 0,
        spread: 0,
        color: INK,
    },
    widget_shadow: 3.0,
    body: 14.0,
    heading: 24.0,
    small: 12.0,
    mono: 13.0,
    pad: vec2(14.0, 7.0),
    spacing: vec2(10.0, 10.0),
    margin: 14,
};

/// The style's tokens. Brutal is light-only, so there is one set whatever the system theme.
pub fn tokens() -> &'static Tokens {
    &BRUTAL
}

/// The family `TextStyle::Heading` resolves to: Space Grotesk 700. Titles use it directly.
pub const HEADING: &str = "heading";

pub fn heading_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(HEADING.into()))
}

/// Whether the *system* is in dark mode. The style ignores it (there is no dark Brutal), but
/// the wallpaper still follows it.
pub fn is_dark(ctx: &egui::Context) -> bool {
    matches!(ctx.theme(), egui::Theme::Dark)
}

/// Installs the fonts, and the Brutal style for both the light and the dark system theme.
/// Called once, before the first frame: fonts only take effect from the next one, and a
/// `FontFamily::Name` lookup before then panics.
pub fn apply(ctx: &egui::Context) {
    ctx.set_fonts(font_definitions());
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        let style = style(&ctx.style_of(theme), tokens());
        ctx.set_style_of(theme, style);
    }
    // A graph's line is a mark, not text, so it can be the accent where a link can't.
    ccosel_host::set_plot_color(ctx, tokens().accent);
}

/// Space Grotesk and Space Mono first in their families, with egui's default fonts and Phosphor
/// behind them in every family. The bundled fonts are Latin subsets, so `›`, `…`, emoji and the
/// icons in `registry::catalog` all come from the fallbacks.
pub fn font_definitions() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    // Registered up front so the icons in `registry::catalog` are ordinary glyphs in ordinary
    // text and painter calls from here on, indistinguishable from the built-in font.
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);

    for (name, bytes) in [
        (
            "space-grotesk",
            &include_bytes!("../fonts/SpaceGrotesk-Regular.ttf")[..],
        ),
        (
            "space-grotesk-bold",
            &include_bytes!("../fonts/SpaceGrotesk-Bold.ttf")[..],
        ),
        (
            "space-mono",
            &include_bytes!("../fonts/SpaceMono-Regular.ttf")[..],
        ),
    ] {
        fonts
            .font_data
            .insert(name.to_owned(), Arc::new(FontData::from_static(bytes)));
    }

    // egui's text font, then Phosphor, then emoji: everything a label could need.
    let fallbacks = fonts.families[&FontFamily::Proportional].clone();
    let with = |first: &str, rest: &[String]| {
        let mut family = vec![first.to_owned()];
        family.extend(rest.iter().filter(|f| *f != first).cloned());
        family
    };

    let mut mono = fonts.families[&FontFamily::Monospace].clone();
    for f in &fallbacks {
        if !mono.contains(f) {
            mono.push(f.clone());
        }
    }

    fonts
        .families
        .insert(FontFamily::Proportional, with("space-grotesk", &fallbacks));
    fonts.families.insert(
        FontFamily::Name(HEADING.into()),
        with("space-grotesk-bold", &fallbacks),
    );
    fonts
        .families
        .insert(FontFamily::Monospace, with("space-mono", &mono));
    fonts
}

/// `base` with the Brutal spacing, text styles and visuals. `base` only contributes what the
/// style doesn't set (animation timings, interaction settings and the like).
fn style(base: &egui::Style, t: &Tokens) -> egui::Style {
    let mut s = base.clone();

    s.spacing.item_spacing = t.spacing;
    s.spacing.button_padding = t.pad;
    s.spacing.window_margin = Margin::same(t.margin);
    s.spacing.menu_margin = Margin::same(8);
    s.spacing.interact_size.y = t.body + 2.0 * t.pad.y;
    // Scrollbars drawn over the content instead of in a reserved gutter: a listing should
    // not change width the moment it grows long enough to scroll.
    s.spacing.scroll = egui::style::ScrollStyle::floating();

    s.text_styles = [
        (TextStyle::Heading, heading_font(t.heading)),
        (TextStyle::Body, FontId::proportional(t.body)),
        (TextStyle::Button, FontId::proportional(t.body)),
        (TextStyle::Small, FontId::proportional(t.small)),
        (TextStyle::Monospace, FontId::monospace(t.mono)),
    ]
    .into();

    s.visuals = visuals(t);
    s
}

fn visuals(t: &Tokens) -> egui::Visuals {
    let mut v = egui::Visuals::light();
    let square = CornerRadius::ZERO;

    v.panel_fill = t.surface;
    v.window_fill = t.surface;
    v.window_stroke = Stroke::new(t.stroke, t.ink);
    v.window_corner_radius = square;
    v.menu_corner_radius = square;
    v.window_shadow = t.shadow;
    v.popup_shadow = t.shadow;

    // Text-edit background, `Grid::striped` and code: all plain white. Brutal separates things
    // with outlines, not tints.
    v.extreme_bg_color = t.surface_alt;
    v.faint_bg_color = t.surface_alt;
    v.code_bg_color = t.surface_alt;
    // Ink, not the accent: yellow text on white is about 1.4:1, far below the 4.5:1 text needs.
    // Links are told apart by their underline instead (see `replay.rs`).
    v.hyperlink_color = t.ink;
    v.warn_fg_color = t.warn;
    v.error_fg_color = t.danger;
    // What `.weak()` resolves to. Without it, weak text is ink at reduced alpha, which is
    // muddy on anything that isn't white.
    v.weak_text_color = Some(t.text_dim);

    // Solid yellow with ink text; never a tint. The stroke is also the focus ring on text
    // fields, so it is the outline ink rather than the accent.
    v.selection = Selection {
        bg_fill: t.accent,
        stroke: Stroke::new(t.stroke, t.on_accent),
    };
    v.text_cursor.stroke = Stroke::new(2.0, t.accent);

    v.widgets.noninteractive = widget(t, t.surface, t.ink); // separators, group frames
    v.widgets.inactive = widget(t, t.surface_alt, t.ink); // idle buttons
    v.widgets.hovered = widget(t, t.hover, t.ink);
    // Pressed is yellow, and its text stays ink: egui also draws `RichText::strong()` in this
    // state's text colour, and white strong text on a white window would vanish.
    v.widgets.active = widget(t, t.accent, t.on_accent);
    v.widgets.open = widget(t, t.hover, t.ink);

    v
}

/// One interaction state. Every state has the same stroke width and no `expansion`, so a
/// button stays exactly the same size, and sits exactly on its shadow, whatever the pointer
/// does.
fn widget(t: &Tokens, fill: Color32, text: Color32) -> WidgetVisuals {
    WidgetVisuals {
        bg_fill: fill,
        weak_bg_fill: fill,
        bg_stroke: Stroke::new(t.stroke, t.ink),
        corner_radius: CornerRadius::ZERO,
        fg_stroke: Stroke::new(1.0, text),
        expansion: 0.0,
    }
}

/// A solid ink block `offset` down and right of `rect`: the hard shadow. Paint it first, then
/// the thing casting it.
pub fn paint_hard_shadow(painter: &egui::Painter, rect: egui::Rect, offset: f32) {
    painter.rect_filled(rect.translate(Vec2::splat(offset)), 0, tokens().ink);
}

/// A full-width ink rule along the top (`Min`) or bottom edge of `rect`, inside it.
pub fn paint_rule(painter: &egui::Painter, rect: egui::Rect, edge: egui::Align) {
    let t = tokens();
    let y = match edge {
        egui::Align::Min => rect.top() + t.stroke / 2.0,
        _ => rect.bottom() - t.stroke / 2.0,
    };
    painter.hline(rect.x_range(), y, Stroke::new(t.stroke, t.ink));
}

/// A square filled with `color`, outlined in ink, with `icon` centred on it: an app's identity
/// compressed to a single mark. `shadow` is false while the badge is pressed, so it reads as
/// pushed in, the same as a button.
pub fn paint_badge(
    painter: &egui::Painter,
    rect: egui::Rect,
    icon: &str,
    color: Color32,
    shadow: bool,
) {
    let t = tokens();
    if shadow {
        paint_hard_shadow(painter, rect, t.widget_shadow);
    }
    painter.rect(
        rect,
        0,
        color,
        Stroke::new(t.stroke, t.ink),
        StrokeKind::Inside,
    );

    let fg = contrast_color(color);
    let font = FontId::proportional(rect.height() * 0.52);
    let galley = painter.layout_no_wrap(icon.to_owned(), font, fg);
    painter.galley(rect.center() - galley.size() / 2.0, galley, fg);
}

/// White or ink, whichever reads better on `bg`, so a badge's glyph stays legible whether the
/// app picked a light or a dark colour.
pub fn contrast_color(bg: Color32) -> Color32 {
    if luminance(bg) > 0.42 {
        tokens().ink
    } else {
        Color32::WHITE
    }
}

/// WCAG relative luminance, 0 (black) to 1 (white).
fn luminance(c: Color32) -> f32 {
    fn to_linear(c: u8) -> f32 {
        let c = c as f32 / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * to_linear(c.r()) + 0.7152 * to_linear(c.g()) + 0.0722 * to_linear(c.b())
}

/// The WCAG 2 contrast ratio between two colours: 1:1 is none, 21:1 is black on white, and
/// `>= 4.5` is the AA bar for normal-size text.
#[cfg(test)]
fn contrast_ratio(a: Color32, b: Color32) -> f32 {
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

    fn applied(theme: egui::Theme) -> egui::Context {
        let ctx = egui::Context::default();
        apply(&ctx);
        ctx.set_theme(theme);
        ctx
    }

    #[wasm_bindgen_test]
    fn both_system_themes_get_the_same_brutal_style() {
        let light = applied(egui::Theme::Light);
        let dark = applied(egui::Theme::Dark);
        // Compared as text: `Style` holds NaN defaults, and NaN never equals itself.
        assert_eq!(
            format!("{:?}", light.style_of(egui::Theme::Light)),
            format!("{:?}", dark.style_of(egui::Theme::Dark))
        );
        assert!(!dark.global_style().visuals.dark_mode);
        // The wallpaper reads the system theme, which the style must not override.
        assert!(is_dark(&dark));
        assert!(!is_dark(&light));
    }

    fn states(v: &egui::Visuals) -> [(&'static str, &WidgetVisuals); 5] {
        let w = &v.widgets;
        [
            ("noninteractive", &w.noninteractive),
            ("inactive", &w.inactive),
            ("hovered", &w.hovered),
            ("active", &w.active),
            ("open", &w.open),
        ]
    }

    /// Every widget state is outlined exactly like a window, square, and the same size: a
    /// button that grew or changed outline on hover would slide off its hard shadow.
    #[wasm_bindgen_test]
    fn every_widget_state_is_outlined_like_a_window_and_stays_square() {
        let ctx = applied(egui::Theme::Light);
        let v = &ctx.global_style().visuals;

        assert!(v.window_stroke.width > 0.0);
        assert_eq!(v.window_corner_radius, CornerRadius::ZERO);
        assert_eq!(v.menu_corner_radius, CornerRadius::ZERO);
        for (name, state) in states(v) {
            assert_eq!(state.bg_stroke, v.window_stroke, "{name}");
            assert_eq!(state.corner_radius, CornerRadius::ZERO, "{name}");
            assert_eq!(state.expansion, 0.0, "{name}");
        }
    }

    #[wasm_bindgen_test]
    fn windows_and_popups_cast_one_hard_shadow_in_the_outline_ink() {
        let ctx = applied(egui::Theme::Light);
        let v = &ctx.global_style().visuals;
        let shadow = v.window_shadow;

        assert_eq!(v.popup_shadow, shadow);
        assert_eq!((shadow.blur, shadow.spread), (0, 0), "hard, not soft");
        assert!(
            shadow.offset[0] > 0 && shadow.offset[1] > 0,
            "down and right"
        );
        assert_eq!(shadow.color, v.window_stroke.color);
    }

    /// Whatever the palette is, every piece of text the style colours reads on what it is drawn
    /// over. This is what `the_palette_matches_the_spec` used to stand in for.
    #[wasm_bindgen_test]
    fn all_styled_text_clears_aa_contrast_on_its_own_background() {
        let ctx = applied(egui::Theme::Light);
        let v = &ctx.global_style().visuals;
        let mut pairs: Vec<(String, Color32, Color32)> = states(v)
            .into_iter()
            .map(|(name, s)| (format!("{name} text"), s.fg_stroke.color, s.bg_fill))
            .collect();
        pairs.extend([
            (
                "selected text".into(),
                v.selection.stroke.color,
                v.selection.bg_fill,
            ),
            ("a link".into(), v.hyperlink_color, v.window_fill),
            (
                "weak text".into(),
                v.weak_text_color.unwrap(),
                v.window_fill,
            ),
            ("text in a field".into(), v.text_color(), v.extreme_bg_color),
        ]);
        for (name, fg, bg) in pairs {
            let ratio = contrast_ratio(fg, bg);
            assert!(ratio >= AA_TEXT, "{name}: {ratio:.2}:1 ({fg:?} on {bg:?})");
        }
    }

    /// Links can't be the accent (see the test above), but a graph's line is a mark, not text,
    /// so graphs keep it.
    #[wasm_bindgen_test]
    fn graphs_are_drawn_in_the_accent_though_links_are_not() {
        let ctx = applied(egui::Theme::Light);
        assert_eq!(ccosel_host::plot_color(&ctx), tokens().accent);
        assert_ne!(ctx.global_style().visuals.hyperlink_color, tokens().accent);
    }

    #[wasm_bindgen_test]
    fn strong_text_is_ink_not_white() {
        // egui draws `RichText::strong()` in `widgets.active.fg_stroke`.
        let ctx = applied(egui::Theme::Light);
        let v = &ctx.global_style().visuals;
        assert_eq!(v.widgets.active.fg_stroke.color, hex(0x111111));
        assert_eq!(v.strong_text_color(), hex(0x111111));
    }

    #[wasm_bindgen_test]
    fn text_clears_aa_contrast_on_every_surface_it_sits_on() {
        let t = tokens();
        for (name, fg, bg) in [
            ("ink on surface", t.ink, t.surface),
            ("dim on surface", t.text_dim, t.surface),
            ("ink on hover", t.ink, t.hover),
            ("ink on accent", t.on_accent, t.accent),
        ] {
            let ratio = contrast_ratio(fg, bg);
            assert!(ratio >= AA_TEXT, "{name}: {ratio}");
        }
    }

    #[wasm_bindgen_test]
    fn identical_colors_have_no_contrast() {
        assert!((contrast_ratio(INK, INK) - 1.0).abs() < 1e-6);
    }

    #[wasm_bindgen_test]
    fn text_styles_use_the_bundled_fonts_and_size_sets_the_hierarchy() {
        let ctx = applied(egui::Theme::Light);
        let styles = &ctx.global_style().text_styles;
        let heading = FontFamily::Name(HEADING.into());
        let (h, body, button, small) = (
            &styles[&TextStyle::Heading],
            &styles[&TextStyle::Body],
            &styles[&TextStyle::Button],
            &styles[&TextStyle::Small],
        );

        assert_eq!(h.family, heading);
        assert_eq!(body.family, FontFamily::Proportional);
        assert_eq!(button, body, "buttons read like body text");
        assert_eq!(small.family, FontFamily::Proportional);
        assert_eq!(styles[&TextStyle::Monospace].family, FontFamily::Monospace);
        // Hierarchy comes from size and weight, not colour.
        assert!(small.size < body.size && body.size < h.size);

        let fonts = font_definitions();
        assert_eq!(
            fonts.families[&FontFamily::Proportional][0],
            "space-grotesk"
        );
        assert_eq!(fonts.families[&heading][0], "space-grotesk-bold");
        assert_eq!(fonts.families[&FontFamily::Monospace][0], "space-mono");
    }

    #[wasm_bindgen_test]
    fn every_family_falls_back_to_egui_defaults_and_phosphor() {
        let fonts = font_definitions();
        for family in [
            FontFamily::Proportional,
            FontFamily::Monospace,
            FontFamily::Name(HEADING.into()),
        ] {
            let list = &fonts.families[&family];
            for fallback in ["Ubuntu-Light", "phosphor", "NotoEmoji-Regular"] {
                let at = list.iter().position(|f| f == fallback);
                assert!(
                    at.is_some_and(|i| i > 0),
                    "{family:?} lacks {fallback} as a fallback: {list:?}"
                );
            }
        }
    }

    /// The fallbacks actually resolve: a glyph missing from the Latin subsets still lays out
    /// with a real glyph, not the replacement box.
    #[wasm_bindgen_test]
    fn glyphs_outside_the_subset_still_render() {
        let ctx = applied(egui::Theme::Light);
        // Fonts land on the first frame after `set_fonts`.
        let mut out = ctx.run_ui(egui::RawInput::default(), |_| {});
        out.textures_delta.clear();
        let icon = egui_phosphor::regular::FOLDER.chars().next().unwrap();
        for family in [
            FontFamily::Proportional,
            FontFamily::Monospace,
            FontFamily::Name(HEADING.into()),
        ] {
            for c in ['›', '…', 'A', icon] {
                let has = ctx.fonts_mut(|f| f.has_glyph(&FontId::new(14.0, family.clone()), c));
                assert!(has, "{family:?} cannot draw {c:?}");
            }
        }
    }

    /// Size of a button with the pointer at `pointer`, held down if `pressed`. Two frames per
    /// call: egui styles a widget by its state from the frame before.
    fn button_size(ctx: &egui::Context, pointer: egui::Pos2, pressed: bool) -> egui::Vec2 {
        let mut size = egui::Vec2::ZERO;
        for _ in 0..2 {
            let mut events = vec![egui::Event::PointerMoved(pointer)];
            if pressed {
                events.push(egui::Event::PointerButton {
                    pos: pointer,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                });
            }
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(400.0, 300.0),
                )),
                events,
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                size = ui.add(egui::Button::new("Refresh")).rect.size();
            });
            out.textures_delta.clear();
        }
        size
    }

    /// A button that grew when hovered or pressed would slide off its own shadow, and push every
    /// row after it along.
    #[wasm_bindgen_test]
    fn hovering_or_pressing_a_button_does_not_change_its_size() {
        for theme in [egui::Theme::Dark, egui::Theme::Light] {
            let ctx = applied(theme);
            let away = button_size(&ctx, egui::pos2(390.0, 290.0), false);
            let over = button_size(&ctx, egui::pos2(20.0, 12.0), false);
            let down = button_size(&ctx, egui::pos2(20.0, 12.0), true);
            assert_eq!(away, over, "{theme:?}: hover resized the button");
            assert_eq!(away, down, "{theme:?}: press resized the button");
        }
    }

    /// What guests actually get: every clickable command, replayed through the real `Replayer`
    /// under this theme, keeps exactly the same rect at rest, hovered and pressed. Anything that
    /// gained its outline only on hover would grow by the stroke and shove its neighbours along.
    #[wasm_bindgen_test]
    fn no_guest_widget_changes_size_on_hover_or_press() {
        use ccosel_abi::{Cmd, Encoder};

        let cases: [(&str, Cmd<'_>); 6] = [
            (
                "button",
                Cmd::Button {
                    id: 1,
                    text: "Refresh",
                },
            ),
            (
                "selectable",
                Cmd::Selectable {
                    id: 1,
                    text: "notes.md",
                    selected: false,
                },
            ),
            (
                "selected selectable",
                Cmd::Selectable {
                    id: 1,
                    text: "notes.md",
                    selected: true,
                },
            ),
            ("upload folder", Cmd::UploadFolder { id: 1, dest: "/" }),
            ("upload project", Cmd::UploadProject { id: 1 }),
            (
                "open url",
                Cmd::OpenUrl {
                    id: 1,
                    label: "Download",
                    url: "/f",
                },
            ),
        ];
        for (name, cmd) in cases {
            let mut e = Encoder::new();
            e.push(&cmd);
            let buf = e.as_slice().to_vec();

            let ctx = applied(egui::Theme::Light);
            let mut replayer = ccosel_host::Replayer::new();
            let mut frame = |events: Vec<egui::Event>| {
                let mut rect = [0.0; 4];
                // Two frames: egui styles a widget by its state from the frame before.
                for _ in 0..2 {
                    let input = egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(400.0, 300.0),
                        )),
                        events: events.clone(),
                        ..Default::default()
                    };
                    let mut out = ctx.run_ui(input, |ui| {
                        rect = replayer.replay(ui, 1, &buf).unwrap()[0].rect;
                    });
                    out.textures_delta.clear();
                }
                rect
            };
            let rest = frame(vec![egui::Event::PointerMoved(egui::pos2(390.0, 290.0))]);
            let inside = egui::pos2(rest[0] + 6.0, rest[1] + 6.0);
            let hover = frame(vec![egui::Event::PointerMoved(inside)]);
            let press = frame(vec![egui::Event::PointerButton {
                pos: inside,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            }]);
            assert_eq!(rest, hover, "{name}: hover changed its rect");
            assert_eq!(rest, press, "{name}: press changed its rect");
        }
    }

    /// Guards the test above: it only proves anything if hovering really reached the button.
    #[wasm_bindgen_test]
    fn the_hover_in_the_size_test_lands_on_the_button() {
        let ctx = applied(egui::Theme::Light);
        let mut hovered = false;
        for _ in 0..2 {
            let input = egui::RawInput {
                events: vec![egui::Event::PointerMoved(egui::pos2(20.0, 12.0))],
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                hovered = ui.button("Refresh").hovered();
            });
            out.textures_delta.clear();
        }
        assert!(hovered);
    }

    #[wasm_bindgen_test]
    fn badge_glyphs_contrast_with_their_fill() {
        assert_eq!(contrast_color(Color32::WHITE), INK);
        assert_eq!(contrast_color(hex(0xFFD400)), INK);
        assert_eq!(contrast_color(hex(0x1D4ED8)), Color32::WHITE);
    }
}
