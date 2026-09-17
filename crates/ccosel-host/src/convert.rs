//! ABI value types -> egui value types.
//!
//! The ABI deliberately carries its own copies of these (so guests don't link `emath`), which
//! makes this module the one place the two vocabularies meet.

use ccosel_abi::{Align, Layout, ScopeKind, Vec2};

pub fn vec2(v: Vec2) -> egui::Vec2 {
    egui::vec2(v.x, v.y)
}

pub fn align(a: Align) -> egui::Align {
    match a {
        Align::Min => egui::Align::Min,
        Align::Center => egui::Align::Center,
        Align::Max => egui::Align::Max,
    }
}

pub fn layout(l: Layout) -> egui::Layout {
    match l.kind {
        ScopeKind::Horizontal => egui::Layout::left_to_right(align(l.cross_align)),
        ScopeKind::Vertical | ScopeKind::Group | ScopeKind::Frame => {
            egui::Layout::top_down(align(l.cross_align))
        }
    }
}

pub fn rect_array(r: egui::Rect) -> [f32; 4] {
    [r.min.x, r.min.y, r.max.x, r.max.y]
}
