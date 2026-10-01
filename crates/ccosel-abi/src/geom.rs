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
    /// Left to right, wrapping onto a new line when the row is full, with no gap between
    /// children: consecutive [`TextStyle`] runs read as one paragraph.
    Wrapped = 4,
    /// A fixed-width column followed by a vertical rule. Meant as the first child of a
    /// top-aligned horizontal scope, with the main content as the second.
    Sidebar = 5,
    /// Children indented by one step of the shell's indent width, for nested lists and trees.
    Indent = 6,
    /// A region that scrolls on its own, filling the rest of the window's height. Two side by
    /// side (a sidebar and a page) scroll independently.
    Scroll = 7,
    /// A terminal's layout: a scrolling region over a row pinned to the bottom, together
    /// filling the rest of the window's height. The first child scope is the region, which
    /// keeps its newest (bottom) line in view; everything after it is the pinned row.
    ScrollFooter = 8,
    /// Children drawn greyed out, ignoring clicks and typing: controls that can't be used
    /// right now. Laid out like `Group`.
    Disabled = 9,
}

impl ScopeKind {
    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Group),
            1 => Some(Self::Horizontal),
            2 => Some(Self::Vertical),
            3 => Some(Self::Frame),
            4 => Some(Self::Wrapped),
            5 => Some(Self::Sidebar),
            6 => Some(Self::Indent),
            7 => Some(Self::Scroll),
            8 => Some(Self::ScrollFooter),
            9 => Some(Self::Disabled),
            _ => None,
        }
    }
}

/// How a `Styled` run of text looks. A bit set, so styles combine: a bold link inside a
/// heading is `heading(2) | STRONG | LINK`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextStyle(pub u8);

impl TextStyle {
    pub const PLAIN: Self = Self(0);
    /// Bits 0-1: heading level, 0 (body text) to 3.
    const HEADING_MASK: u8 = 0b11;
    pub const STRONG: Self = Self(1 << 2);
    pub const ITALIC: Self = Self(1 << 3);
    /// Monospace, on a faint background.
    pub const CODE: Self = Self(1 << 4);
    pub const STRIKE: Self = Self(1 << 5);
    /// De-emphasised colour, for captions and quotes.
    pub const WEAK: Self = Self(1 << 6);
    /// Drawn as a hyperlink and clickable. What a click *does* is the app's business.
    pub const LINK: Self = Self(1 << 7);

    /// A heading style, level clamped to `1..=3`.
    pub const fn heading(level: u8) -> Self {
        let level = if level == 0 {
            1
        } else if level > 3 {
            3
        } else {
            level
        };
        Self(level)
    }

    pub const fn heading_level(self) -> u8 {
        self.0 & Self::HEADING_MASK
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn with(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

impl core::ops::BitOr for TextStyle {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        self.with(rhs)
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
