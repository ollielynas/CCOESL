//! ABI value types -> egui value types.
//!
//! The ABI deliberately carries its own copies of these (so guests don't link `emath`), which
//! makes this module the one place the two vocabularies meet.

use ccosel_abi::Vec2;

pub fn vec2(v: Vec2) -> egui::Vec2 {
    egui::vec2(v.x, v.y)
}

pub fn rect_array(r: egui::Rect) -> [f32; 4] {
    [r.min.x, r.min.y, r.max.x, r.max.y]
}
