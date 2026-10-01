//! Audio, video and PDFs (`Ui::media`), played or shown by the browser in elements laid over
//! the canvas.
//!
//! egui can draw neither, and the browser already has every decoder it ships plus its own player
//! controls. Replay reserves the space and reports where it is ([`MediaSlot`]); this keeps one
//! `<video>`, `<audio>` or `<iframe>` per slot sitting exactly over it.
//!
//! An element is on top of the whole canvas, so it can't be partly covered by another window
//! the way egui content is. Instead it is hidden while anything else is over it, and egui's
//! placeholder shows through; a hidden player keeps playing, so music isn't cut off by opening
//! another window. It goes away for good when its slot does: the window closed or minimised, or
//! the app stopped drawing it.

use std::collections::HashMap;

use ccosel_abi::MediaKind;
use ccosel_host::MediaSlot;
use wasm_bindgen::JsCast;

/// One media slot as drawn this frame, and the egui layer it was drawn in: a window's own area
/// on the desktop, or the background on an app's own page, where the app is drawn under every
/// area. (`layer_id_at` reports the background, not `None`, wherever no area is.)
pub struct Placed<'a> {
    pub instance: u64,
    pub layer: Option<egui::LayerId>,
    pub slot: &'a MediaSlot,
}

struct Overlay {
    el: web_sys::HtmlElement,
    src: String,
}

#[derive(Default)]
pub struct MediaOverlay {
    /// Keyed by window instance and the app's widget id, so two windows playing the same file
    /// get a player each.
    elements: HashMap<(u64, u64), Overlay>,
}

impl MediaOverlay {
    /// Bring the players in line with this frame: create, move, show or hide each, and remove
    /// those whose slot is gone.
    pub fn sync(&mut self, ctx: &egui::Context, placed: &[Placed<'_>]) -> Result<(), String> {
        let mut seen = Vec::with_capacity(placed.len());
        for p in placed {
            let key = (p.instance, p.slot.local_id);
            seen.push(key);
            if self.elements.get(&key).is_some_and(|o| o.src != p.slot.src) {
                // A different file in the same place: a fresh element, not a swapped `src`, so
                // the old one's playback state doesn't carry over.
                if let Some(old) = self.elements.remove(&key) {
                    old.el.remove();
                }
            }
            let o = match self.elements.entry(key) {
                std::collections::hash_map::Entry::Occupied(o) => o.into_mut(),
                std::collections::hash_map::Entry::Vacant(v) => v.insert(Overlay {
                    el: create(p.slot.kind, &p.slot.src)?,
                    src: p.slot.src.clone(),
                }),
            };
            place(ctx, &o.el, p.slot, uncovered(ctx, p.layer, p.slot.visible))?;
        }
        self.elements.retain(|key, o| {
            let keep = seen.contains(key);
            if !keep {
                o.el.remove();
            }
            keep
        });
        Ok(())
    }
}

/// Whether nothing is drawn over `visible` but its own layer: checked at the corners and the
/// middle, which catches a window or menu overlapping it in practice.
pub fn uncovered(ctx: &egui::Context, layer: Option<egui::LayerId>, visible: egui::Rect) -> bool {
    let inner = visible.shrink(2.0);
    if !inner.is_positive() {
        return false;
    }
    [
        inner.left_top(),
        inner.right_top(),
        inner.left_bottom(),
        inner.right_bottom(),
        inner.center(),
    ]
    .into_iter()
    .all(|p| ctx.layer_id_at(p) == layer)
}

fn create(kind: MediaKind, src: &str) -> Result<web_sys::HtmlElement, String> {
    let document = web_sys::window()
        .and_then(|w| w.document())
        .ok_or("no document")?;
    let tag = match kind {
        MediaKind::Video => "video",
        MediaKind::Audio => "audio",
        MediaKind::Document => "iframe",
    };
    let el: web_sys::HtmlElement = document
        .create_element(tag)
        .map_err(|e| format!("{tag}: {e:?}"))?
        .dyn_into()
        .map_err(|_| format!("{tag} is not an element"))?;
    let set = |k: &str, v: &str| {
        el.set_attribute(k, v)
            .map_err(|e| format!("{tag} {k}: {e:?}"))
    };
    set("src", src)?;
    if kind != MediaKind::Document {
        set("controls", "")?;
        // Enough to show the length and first frame without fetching the whole file.
        set("preload", "metadata")?;
        // In the page on phones, not taken over to full screen.
        set("playsinline", "")?;
    } else {
        set("title", "Document")?;
    }
    let style = el.style();
    for (k, v) in [
        ("position", "fixed"),
        ("border", "0"),
        ("margin", "0"),
        ("box-sizing", "border-box"),
        (
            "background",
            if kind == MediaKind::Video {
                "#000"
            } else {
                "transparent"
            },
        ),
        ("z-index", "10"),
        ("display", "none"),
    ] {
        style
            .set_property(k, v)
            .map_err(|e| format!("style {k}: {e:?}"))?;
    }
    document
        .body()
        .ok_or("no body")?
        .append_child(&el)
        .map_err(|e| format!("append {tag}: {e:?}"))?;
    Ok(el)
}

/// Lays `el` over `slot`, in CSS pixels relative to the page, with whatever of it is scrolled or
/// clipped out of view cut off.
fn place(
    ctx: &egui::Context,
    el: &web_sys::HtmlElement,
    slot: &MediaSlot,
    shown: bool,
) -> Result<(), String> {
    let style = el.style();
    let set = |k: &str, v: &str| {
        style
            .set_property(k, v)
            .map_err(|e| format!("style {k}: {e:?}"))
    };
    if !shown {
        return set("display", "none");
    }
    let origin = canvas_origin().unwrap_or_default();
    // egui points to CSS pixels: the page's zoom is the only difference between them.
    let css = |r: egui::Rect| r * ctx.zoom_factor();
    let rect = css(slot.rect);
    let visible = css(slot.visible);
    set("left", &format!("{}px", origin.x + rect.left()))?;
    set("top", &format!("{}px", origin.y + rect.top()))?;
    set("width", &format!("{}px", rect.width()))?;
    set("height", &format!("{}px", rect.height()))?;
    set(
        "clip-path",
        &format!(
            "inset({}px {}px {}px {}px)",
            (visible.top() - rect.top()).max(0.0),
            (rect.right() - visible.right()).max(0.0),
            (rect.bottom() - visible.bottom()).max(0.0),
            (visible.left() - rect.left()).max(0.0),
        ),
    )?;
    set("display", "block")
}

/// Where the canvas egui draws into starts on the page, in CSS pixels.
fn canvas_origin() -> Option<egui::Pos2> {
    let canvas = web_sys::window()?.document()?.get_element_by_id("canvas")?;
    let r = canvas.get_bounding_client_rect();
    Some(egui::pos2(r.left() as f32, r.top() as f32))
}

#[cfg(test)]
mod tests {
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::*;

    /// One frame with a window-like area at `area`, returning the context to ask about it.
    fn with_area(area: egui::Rect) -> (egui::Context, egui::LayerId) {
        let ctx = egui::Context::default();
        let id = egui::Id::new("window");
        for _ in 0..2 {
            let mut full = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 600.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    egui::Area::new(id)
                        .fixed_pos(area.min)
                        .show(ui.ctx(), |ui| ui.allocate_space(area.size()));
                },
            );
            full.textures_delta.clear();
        }
        (ctx, egui::LayerId::new(egui::Order::Middle, id))
    }

    #[wasm_bindgen_test]
    fn media_inside_its_own_window_is_uncovered() {
        let window = egui::Rect::from_min_size(egui::pos2(100.0, 100.0), egui::vec2(300.0, 200.0));
        let (ctx, layer) = with_area(window);
        let media = egui::Rect::from_min_size(egui::pos2(120.0, 120.0), egui::vec2(200.0, 100.0));
        assert!(uncovered(&ctx, Some(layer), media));
    }

    #[wasm_bindgen_test]
    fn media_under_another_window_is_covered() {
        let other = egui::Rect::from_min_size(egui::pos2(100.0, 100.0), egui::vec2(300.0, 200.0));
        let (ctx, _) = with_area(other);
        // On an app's own page the app is under every area, so any area over it covers it.
        let media = egui::Rect::from_min_size(egui::pos2(50.0, 50.0), egui::vec2(200.0, 100.0));
        assert!(
            !uncovered(&ctx, background(), media),
            "a corner is under the area"
        );
        let clear = egui::Rect::from_min_size(egui::pos2(450.0, 350.0), egui::vec2(100.0, 50.0));
        assert!(uncovered(&ctx, background(), clear));
    }

    /// Where an app's own page draws: under every area.
    fn background() -> Option<egui::LayerId> {
        Some(egui::LayerId::background())
    }

    #[wasm_bindgen_test]
    fn nothing_to_show_is_never_uncovered() {
        let (ctx, _) = with_area(egui::Rect::from_min_size(
            egui::pos2(600.0, 500.0),
            egui::vec2(10.0, 10.0),
        ));
        assert!(!uncovered(
            &ctx,
            None,
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::Vec2::ZERO)
        ));
    }
}
