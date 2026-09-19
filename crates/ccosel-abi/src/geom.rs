//! Geometry and colour value types.
//!
//! These are deliberately **copied** from `emath`/`ecolor` rather than depended on. A guest
//! that pulls in `emath` pulls in egui's dependency tree, and the whole point of the
//! command-stream design is that a guest module is tens of KB, not megabytes. The host
//! converts these to the real egui types at the replay boundary.

/// Layout direction for a scope. Mirrors the subset of `egui::Direction` we expose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ScopeKind {
    /// Plain scope, inherits the parent layout.
    Group = 0,
    Horizontal = 1,
    Vertical = 2,
    /// Visually grouped (framed) scope.
    Frame = 3,
}

impl ScopeKind {
    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Group),
            1 => Some(Self::Horizontal),
            2 => Some(Self::Vertical),
            3 => Some(Self::Frame),
            _ => None,
        }
    }
}

/// Cross-axis alignment within a layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Align {
    Min = 0,
    Center = 1,
    Max = 2,
}

impl Align {
    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Min),
            1 => Some(Self::Center),
            2 => Some(Self::Max),
            _ => None,
        }
    }
}

/// A scope's layout: direction plus cross-axis alignment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub kind: ScopeKind,
    pub cross_align: Align,
}

impl Layout {
    pub const fn new(kind: ScopeKind, cross_align: Align) -> Self {
        Self { kind, cross_align }
    }
}

impl Default for Layout {
    fn default() -> Self {
        Self::new(ScopeKind::Group, Align::Min)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct Pos2 {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct Rect {
    pub min: Pos2,
    pub max: Pos2,
}

/// sRGB with premultiplied alpha, matching `ecolor::Color32`'s memory layout.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct Color32(pub [u8; 4]);

impl Vec2 {
    pub const ZERO: Self = Self { x: 0.0, y: 0.0 };
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

impl Pos2 {
    pub const ZERO: Self = Self { x: 0.0, y: 0.0 };
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

impl Rect {
    pub const ZERO: Self = Self {
        min: Pos2::ZERO,
        max: Pos2::ZERO,
    };

    pub const fn from_min_max(min: Pos2, max: Pos2) -> Self {
        Self { min, max }
    }

    pub fn width(&self) -> f32 {
        self.max.x - self.min.x
    }

    pub fn height(&self) -> f32 {
        self.max.y - self.min.y
    }

    /// Flat form used in `RespRecord`.
    pub const fn to_array(self) -> [f32; 4] {
        [self.min.x, self.min.y, self.max.x, self.max.y]
    }

    pub const fn from_array(a: [f32; 4]) -> Self {
        Self {
            min: Pos2::new(a[0], a[1]),
            max: Pos2::new(a[2], a[3]),
        }
    }
}

impl Color32 {
    pub const TRANSPARENT: Self = Self([0, 0, 0, 0]);
    pub const fn from_rgba_premultiplied(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self([r, g, b, a])
    }
    pub const fn to_u32(self) -> u32 {
        u32::from_le_bytes(self.0)
    }
    pub const fn from_u32(v: u32) -> Self {
        Self(v.to_le_bytes())
    }
}
