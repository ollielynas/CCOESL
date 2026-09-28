# CCOSEL style guides

**Chosen: [Brutal](brutal.md)** (issue #42). The other five candidates were rendered for
comparison and aren't kept in the repo; their rows below are for the record.

Six candidate visual languages for the shell and its apps. Every screenshot here is **real egui 0.36 output**,
rendered headless from the same `Tokens` → `apply()` code shown below, so nothing in them is
out of reach.

| Guide | Mood | Variants | Font cost (shell only) |
|---|---|---|---|
| Slate | Calm, modern, "default good" | dark + light | Inter, JetBrains Mono (~190 KB) |
| Paper | Warm, editorial, quiet | light + dark | Inter, Source Serif, Plex Mono (~160 KB) |
| Retro 95 | Bevelled nostalgia | light | none (egui default fonts) |
| Phosphor | Green-screen terminal | dark | IBM Plex Mono (~85 KB) |
| Bloom | Soft, rounded, friendly | light + dark | Nunito, JetBrains Mono (~140 KB) |
| [Brutal](brutal.md) | Loud, flat, hard shadows | light | Space Grotesk, Space Mono (~105 KB) |

Fonts live in the **shell**, never in apps, so they don't count against the 100 KiB app budget.

## How a style is implemented

Guests never link egui. They emit commands that the shell replays through *its* `egui::Ui`,
so a style is entirely shell-side:

1. **`Tokens`**: one struct of colours, radii, sizes and fonts per style/variant. Each guide
   gives its full value.
2. **`apply(ctx, &tokens)`**: turns tokens into `egui::Style` + fonts. This function is the
   same for every guide (below). It replaces the body of `theme::apply` in
   `crates/ccosel-shell/src/theme.rs`.
3. **Chrome**: title bar and dock, painted by the shell in `desktop.rs`. Each guide describes
   its title bar; a few need a small painter (bevel, hard shadow).
4. **Replay**: `crates/ccosel-host/src/replay.rs:176` draws every button with
   `frame_when_inactive(false)`, i.e. flat until hovered. All six guides draw framed buttons,
   so that flag goes to `true` (or becomes a token).

```rust
pub struct Tokens {
    pub dark: bool,
    // wallpaper
    pub wall_top: Color32, pub wall_bottom: Color32,
    // surfaces: window body, inputs/rows/groups, hover
    pub surface: Color32, pub surface_alt: Color32, pub hover: Color32,
    pub border: Color32, pub border_strong: Color32,
    pub text: Color32, pub text_dim: Color32,
    pub accent: Color32, pub on_accent: Color32,
    pub sel_bg: Color32, pub sel_fg: Color32,
    pub danger: Color32, pub success: Color32, pub warn: Color32,
    pub r_window: u8, pub r_widget: u8, pub stroke: f32,
    pub shadow: Shadow,
    pub body: f32, pub heading: f32, pub small: f32, pub mono: f32,
    pub pad: Vec2, pub spacing: Vec2, pub margin: i8,
}

pub fn apply(ctx: &egui::Context, t: &Tokens) {
    install_fonts(ctx, t); // body font first in Proportional, heading font as FontFamily::Name("heading")
    let theme = if t.dark { egui::Theme::Dark } else { egui::Theme::Light };
    let mut s = (*ctx.style_of(theme)).clone();

    s.spacing.item_spacing = t.spacing;
    s.spacing.button_padding = t.pad;
    s.spacing.window_margin = Margin::same(t.margin);
    s.spacing.interact_size.y = t.body + 2.0 * t.pad.y;
    s.text_styles = [
        (TextStyle::Heading, FontId::new(t.heading, FontFamily::Name("heading".into()))),
        (TextStyle::Body, FontId::proportional(t.body)),
        (TextStyle::Button, FontId::proportional(t.body)),
        (TextStyle::Small, FontId::proportional(t.small)),
        (TextStyle::Monospace, FontId::monospace(t.mono)),
    ].into();

    let v = &mut s.visuals;
    v.dark_mode = t.dark;
    v.panel_fill = t.surface;
    v.window_fill = t.surface;
    v.window_stroke = Stroke::new(t.stroke, t.border_strong);
    v.window_corner_radius = CornerRadius::same(t.r_window);
    v.menu_corner_radius = CornerRadius::same(t.r_widget);
    v.window_shadow = t.shadow;
    v.popup_shadow = t.shadow;
    v.extreme_bg_color = t.surface_alt;           // text-edit background
    v.faint_bg_color = t.surface_alt;
    v.code_bg_color = t.surface_alt;
    v.hyperlink_color = t.accent;
    v.warn_fg_color = t.warn;
    v.error_fg_color = t.danger;
    v.weak_text_color = Some(t.text_dim);          // what `.weak()` resolves to
    v.selection.bg_fill = t.sel_bg;
    v.selection.stroke = Stroke::new(t.stroke.max(1.5), t.sel_fg); // also the focus ring
    v.text_cursor.stroke = Stroke::new(2.0, t.accent);

    let w = |fill, stroke, fg| WidgetVisuals {
        bg_fill: fill, weak_bg_fill: fill,
        bg_stroke: Stroke::new(t.stroke, stroke),
        corner_radius: CornerRadius::same(t.r_widget),
        fg_stroke: Stroke::new(1.0, fg),
        expansion: 0.0,
    };
    v.widgets.noninteractive = w(t.surface, t.border, t.text);      // separators, group frames
    v.widgets.inactive = w(t.surface_alt, t.border, t.text);        // idle buttons
    v.widgets.hovered = w(t.hover, t.border_strong, t.text);
    v.widgets.active = w(pressed_fill(t), t.border_strong, text_strong(t)); // see gotcha 1
    v.widgets.open = w(t.hover, t.border_strong, t.text);
    ctx.set_style_of(theme, s);
}
```

### egui gotchas found while rendering these

1. **`RichText::strong()` uses `widgets.active.fg_stroke`.** If "pressed" is accent-filled with
   white text, then strong text is white too, and on a light theme it disappears. So *pressed* is
   a quiet surface (`border`/`border_strong`) with the strongest text colour, and the accent
   fill is only for explicit primary buttons.
2. **Set `weak_text_color`.** Without it, `.weak()` is `text` at reduced alpha, which looks
   muddy on coloured surfaces. With it, apps say `weak` and never pick a colour themselves.
3. **Fonts apply from the next frame.** `set_fonts` then a `FontFamily::Name("heading")` lookup
   in the same frame panics. Call `apply` before the first frame, as `theme::apply` already does.
4. **Glyph coverage.** The bundled fonts are Latin subsets. Keep egui's default fonts *after*
   them in each family (so `›`, `…`, emoji still resolve) and add Phosphor for icons.
5. **Selection stroke is also the focus ring** on text edits, so `sel_fg` has to read as a
   border colour as well as a text colour.

## Rules for every app, whichever style wins

These are about composition, which tokens can't fix. Most of what looks "off" in the current apps
lives here.

1. **Anatomy of a window:** toolbar row → separator → content → separator → status line
   (small, weak). Not every app needs all five, but they appear in this order.
2. **One primary action per window**, and only if there is a clear default (Start, Save, Delete in
   a confirm). Everything else is a default button.
3. **Secondary text is `weak`, never a hand-picked grey.** Sizes, counts, timestamps, hints and
   status all go through `weak` + `small`.
4. **Icons are Phosphor, with two spaces before the label**: `format!("{}  Refresh", ph::ARROW_CLOCKWISE)`.
   No emoji. They render differently per platform and clash with every palette.
5. **Rows, not buttons, for lists.** A list entry is a full-width row with icon, name and
   right-aligned weak metadata; selection fills the row with `sel_bg`.
6. **Numbers that change use monospace**, so a ticking clock doesn't jitter.
7. **Colour carries meaning only:** accent = selected/primary, danger = destructive,
   success/warn = status. Apps don't bring their own colours into the window body. An app's
   identity colour lives on its dock badge.
8. **Destructive actions confirm** in a dialog (`ui.window`) with the danger button on the right
   and Cancel to its left.

## SDK additions these guides assume

The mockups use a few things the SDK can't express yet. Each is small and style-agnostic:
the app says *what* a thing is, and the style decides how it looks.

| Proposed | Replays as | Used for |
|---|---|---|
| `ui.heading(text)` | `RichText::heading()` | dialog titles, section titles |
| `ui.weak(text)` / `ui.small(text)` | `.weak()`, `.small()` | metadata, status lines |
| `ui.mono(text)` | `.monospace()` | clocks, sizes, code |
| `ui.button_primary(text)` / `ui.button_danger(text)` | `Button::fill(accent / danger)` | rule 2, rule 8 |
| `ui.row(selected, \|ui\| ..)` | full-width `Frame` + `sel_bg` | lists (rule 5) |
| `ui.right(\|ui\| ..)` | `Layout::right_to_left` | row metadata, dialog buttons |
| `ui.icon(ph::NAME)` or icon text in labels | Phosphor glyph | toolbars, rows |

Each is one new opcode (or a flag on an existing one), so they're additive and not an ABI break.

The screenshots come from a throwaway renderer (egui_kittest + lavapipe). The same
`Tokens` values will drop into `theme.rs` once a direction is picked.
