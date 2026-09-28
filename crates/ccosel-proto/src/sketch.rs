//! A 2D sketch, as FreeCAD's Sketcher has it: geometry on a plane, held in place by
//! constraints.
//!
//! Geometry and constraints are numbered and named the way FreeCAD numbers and names them, so
//! the server can hand a sketch to `Sketcher::SketchObject` one for one: geometry `i` is
//! FreeCAD's geometry `i`, geometry `-1` is its horizontal axis (whose start is the sketch
//! origin) and `-2` its vertical axis, and a point is `(geometry, Pos)` with FreeCAD's
//! `PointPos` numbering. Coordinates are millimetres in the sketch's own plane; angles radians.
//!
//! The app owns a sketch; the shell solves and edits it (`ccosel-sketch`) and hands the result
//! back as a [`SketchEvent`]. Both cross the guest boundary as postcard bytes the ABI treats as
//! opaque, like RPC payloads, so this crate stays out of `ccosel-abi`.

use alloc::vec::Vec;
use serde::{Deserialize, Serialize};

/// FreeCAD's `H_Axis`: the sketch's X axis. Its [`Pos::Start`] is the sketch origin.
pub const H_AXIS: i32 = -1;
/// FreeCAD's `V_Axis`: the sketch's Y axis.
pub const V_AXIS: i32 = -2;

/// Most geometry a sketch may have. A sketch drawn by hand has tens.
pub const MAX_GEOS: usize = 1000;
pub const MAX_CONSTRAINTS: usize = 2000;

/// Where the sketch lies: its origin, its X direction and its normal, in world coordinates.
/// The Y direction is `normal × x_dir`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Plane {
    pub origin: [f64; 3],
    pub x_dir: [f64; 3],
    pub normal: [f64; 3],
}

impl Plane {
    pub const GROUND: Self = Self {
        origin: [0.0; 3],
        x_dir: [1.0, 0.0, 0.0],
        normal: [0.0, 0.0, 1.0],
    };

    pub fn y_dir(&self) -> [f64; 3] {
        let (n, x) = (self.normal, self.x_dir);
        [
            n[1] * x[2] - n[2] * x[1],
            n[2] * x[0] - n[0] * x[2],
            n[0] * x[1] - n[1] * x[0],
        ]
    }

    /// A sketch point in world coordinates.
    pub fn to_world(&self, p: [f64; 2]) -> [f64; 3] {
        let y = self.y_dir();
        [
            self.origin[0] + self.x_dir[0] * p[0] + y[0] * p[1],
            self.origin[1] + self.x_dir[1] * p[0] + y[1] * p[1],
            self.origin[2] + self.x_dir[2] * p[0] + y[2] * p[1],
        ]
    }

    /// A world point in sketch coordinates, projected onto the plane.
    pub fn to_sketch(&self, w: [f64; 3]) -> [f64; 2] {
        let d = [
            w[0] - self.origin[0],
            w[1] - self.origin[1],
            w[2] - self.origin[2],
        ];
        let y = self.y_dir();
        [
            d[0] * self.x_dir[0] + d[1] * self.x_dir[1] + d[2] * self.x_dir[2],
            d[0] * y[0] + d[1] * y[1] + d[2] * y[2],
        ]
    }
}

/// Which point of a geometry: FreeCAD's `PointPos`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Pos {
    /// The curve itself rather than a point on it.
    Edge,
    Start,
    End,
    /// A circle's or arc's centre.
    Mid,
}

impl Pos {
    /// FreeCAD's number for it.
    pub const fn freecad(self) -> i32 {
        match self {
            Self::Edge => 0,
            Self::Start => 1,
            Self::End => 2,
            Self::Mid => 3,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PointRef {
    pub geo: i32,
    pub pos: Pos,
}

impl PointRef {
    /// The sketch origin.
    pub const ORIGIN: Self = Self {
        geo: H_AXIS,
        pos: Pos::Start,
    };

    pub const fn new(geo: i32, pos: Pos) -> Self {
        Self { geo, pos }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Curve {
    Point([f64; 2]),
    Line([f64; 2], [f64; 2]),
    Circle {
        center: [f64; 2],
        radius: f64,
    },
    /// Counter-clockwise from `start` to `end` (radians), with `start < end <= start + 2π`.
    Arc {
        center: [f64; 2],
        radius: f64,
        start: f64,
        end: f64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Geo {
    pub curve: Curve,
    /// Construction geometry guides the drawing but makes no edge of the shape.
    pub construction: bool,
    /// Fixed in place: external geometry copied from the model, which the solver cannot move.
    pub fixed: bool,
}

impl Geo {
    pub const fn new(curve: Curve) -> Self {
        Self {
            curve,
            construction: false,
            fixed: false,
        }
    }
}

/// A constraint, as FreeCAD's Sketcher names it. Geometry ids are `i32` so the axes (negative
/// ids) can be named too.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Constraint {
    Coincident(PointRef, PointRef),
    PointOnObject(PointRef, i32),
    Horizontal(i32),
    Vertical(i32),
    /// Two points level with each other.
    HorizontalPoints(PointRef, PointRef),
    VerticalPoints(PointRef, PointRef),
    Parallel(i32, i32),
    Perpendicular(i32, i32),
    Tangent(i32, i32),
    Equal(i32, i32),
    /// The two points mirror each other across the line.
    Symmetric(PointRef, PointRef, i32),
    /// The two points mirror each other through the third.
    SymmetricPoint(PointRef, PointRef, PointRef),
    /// A line's length.
    Distance(i32, f64),
    DistancePoints(PointRef, PointRef, f64),
    DistancePointLine(PointRef, i32, f64),
    /// The second point's X minus the first's. With [`PointRef::ORIGIN`] first, a lock.
    DistanceX(PointRef, PointRef, f64),
    DistanceY(PointRef, PointRef, f64),
    Radius(i32, f64),
    Diameter(i32, f64),
    /// A line's angle from the X axis.
    Angle(i32, f64),
    /// The angle from the first line's direction to the second's, counter-clockwise.
    AngleBetween(i32, i32, f64),
}

impl Constraint {
    /// The number it holds, for the dimensional ones.
    pub fn value(&self) -> Option<f64> {
        match *self {
            Self::Distance(_, v)
            | Self::DistancePoints(_, _, v)
            | Self::DistancePointLine(_, _, v)
            | Self::DistanceX(_, _, v)
            | Self::DistanceY(_, _, v)
            | Self::Radius(_, v)
            | Self::Diameter(_, v)
            | Self::Angle(_, v)
            | Self::AngleBetween(_, _, v) => Some(v),
            _ => None,
        }
    }

    /// The same constraint holding `v`, for the dimensional ones.
    pub fn with_value(self, v: f64) -> Self {
        match self {
            Self::Distance(g, _) => Self::Distance(g, v),
            Self::DistancePoints(a, b, _) => Self::DistancePoints(a, b, v),
            Self::DistancePointLine(a, g, _) => Self::DistancePointLine(a, g, v),
            Self::DistanceX(a, b, _) => Self::DistanceX(a, b, v),
            Self::DistanceY(a, b, _) => Self::DistanceY(a, b, v),
            Self::Radius(g, _) => Self::Radius(g, v),
            Self::Diameter(g, _) => Self::Diameter(g, v),
            Self::Angle(g, _) => Self::Angle(g, v),
            Self::AngleBetween(a, b, _) => Self::AngleBetween(a, b, v),
            other => other,
        }
    }

    /// Every geometry id it mentions.
    pub fn geos(&self) -> Vec<i32> {
        match *self {
            Self::Coincident(a, b)
            | Self::HorizontalPoints(a, b)
            | Self::VerticalPoints(a, b)
            | Self::DistancePoints(a, b, _)
            | Self::DistanceX(a, b, _)
            | Self::DistanceY(a, b, _) => alloc::vec![a.geo, b.geo],
            Self::PointOnObject(a, g) | Self::DistancePointLine(a, g, _) => {
                alloc::vec![a.geo, g]
            }
            Self::Horizontal(g)
            | Self::Vertical(g)
            | Self::Distance(g, _)
            | Self::Radius(g, _)
            | Self::Diameter(g, _)
            | Self::Angle(g, _) => alloc::vec![g],
            Self::Parallel(a, b)
            | Self::Perpendicular(a, b)
            | Self::Tangent(a, b)
            | Self::Equal(a, b)
            | Self::AngleBetween(a, b, _) => alloc::vec![a, b],
            Self::Symmetric(a, b, g) => alloc::vec![a.geo, b.geo, g],
            Self::SymmetricPoint(a, b, c) => alloc::vec![a.geo, b.geo, c.geo],
        }
    }

    /// Every point it mentions.
    pub fn points(&self) -> Vec<PointRef> {
        match *self {
            Self::Coincident(a, b)
            | Self::HorizontalPoints(a, b)
            | Self::VerticalPoints(a, b)
            | Self::DistancePoints(a, b, _)
            | Self::DistanceX(a, b, _)
            | Self::DistanceY(a, b, _)
            | Self::Symmetric(a, b, _) => alloc::vec![a, b],
            Self::PointOnObject(a, _) | Self::DistancePointLine(a, _, _) => alloc::vec![a],
            Self::SymmetricPoint(a, b, c) => alloc::vec![a, b, c],
            _ => Vec::new(),
        }
    }

    /// Rewrite every geometry id through `map`, or `None` if `map` drops one.
    pub fn remap(self, map: impl Fn(i32) -> Option<i32>) -> Option<Self> {
        let p = |r: PointRef| -> Option<PointRef> {
            Some(PointRef {
                geo: map(r.geo)?,
                pos: r.pos,
            })
        };
        Some(match self {
            Self::Coincident(a, b) => Self::Coincident(p(a)?, p(b)?),
            Self::PointOnObject(a, g) => Self::PointOnObject(p(a)?, map(g)?),
            Self::Horizontal(g) => Self::Horizontal(map(g)?),
            Self::Vertical(g) => Self::Vertical(map(g)?),
            Self::HorizontalPoints(a, b) => Self::HorizontalPoints(p(a)?, p(b)?),
            Self::VerticalPoints(a, b) => Self::VerticalPoints(p(a)?, p(b)?),
            Self::Parallel(a, b) => Self::Parallel(map(a)?, map(b)?),
            Self::Perpendicular(a, b) => Self::Perpendicular(map(a)?, map(b)?),
            Self::Tangent(a, b) => Self::Tangent(map(a)?, map(b)?),
            Self::Equal(a, b) => Self::Equal(map(a)?, map(b)?),
            Self::Symmetric(a, b, g) => Self::Symmetric(p(a)?, p(b)?, map(g)?),
            Self::SymmetricPoint(a, b, c) => Self::SymmetricPoint(p(a)?, p(b)?, p(c)?),
            Self::Distance(g, v) => Self::Distance(map(g)?, v),
            Self::DistancePoints(a, b, v) => Self::DistancePoints(p(a)?, p(b)?, v),
            Self::DistancePointLine(a, g, v) => Self::DistancePointLine(p(a)?, map(g)?, v),
            Self::DistanceX(a, b, v) => Self::DistanceX(p(a)?, p(b)?, v),
            Self::DistanceY(a, b, v) => Self::DistanceY(p(a)?, p(b)?, v),
            Self::Radius(g, v) => Self::Radius(map(g)?, v),
            Self::Diameter(g, v) => Self::Diameter(map(g)?, v),
            Self::Angle(g, v) => Self::Angle(map(g)?, v),
            Self::AngleBetween(a, b, v) => Self::AngleBetween(map(a)?, map(b)?, v),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sketch {
    pub plane: Plane,
    pub geos: Vec<Geo>,
    pub constraints: Vec<Constraint>,
}

impl Sketch {
    pub fn new(plane: Plane) -> Self {
        Self {
            plane,
            geos: Vec::new(),
            constraints: Vec::new(),
        }
    }

    /// Whether every id points somewhere, every number is finite and it is within the limits.
    pub fn is_valid(&self) -> bool {
        let n = self.geos.len() as i32;
        let geo_ok = |g: i32| (0..n).contains(&g) || g == H_AXIS || g == V_AXIS;
        let finite = |v: &[f64]| v.iter().all(|c| c.is_finite());
        self.geos.len() <= MAX_GEOS
            && self.constraints.len() <= MAX_CONSTRAINTS
            && finite(&self.plane.origin)
            && finite(&self.plane.x_dir)
            && finite(&self.plane.normal)
            && self.geos.iter().all(|g| match g.curve {
                Curve::Point(p) => finite(&p),
                Curve::Line(a, b) => finite(&a) && finite(&b),
                Curve::Circle { center, radius } => finite(&center) && radius.is_finite(),
                Curve::Arc {
                    center,
                    radius,
                    start,
                    end,
                } => finite(&center) && finite(&[radius, start, end]),
            })
            && self
                .constraints
                .iter()
                .all(|c| c.geos().into_iter().all(geo_ok) && c.value().is_none_or(f64::is_finite))
    }
}

/// Something in a sketch the user can select.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Item {
    Geo(i32),
    Point(PointRef),
    Constraint(u32),
}

/// What the app shows the shell: the sketch, and what is selected in it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SketchScene {
    pub sketch: Sketch,
    pub selected: Vec<Item>,
}

/// What the shell reports back.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SketchEvent {
    /// The sketch after an edit: something drawn, dragged, trimmed, or solved into place after
    /// the app changed a constraint. The app replaces its copy with it.
    Edited(Sketch),
    /// A click in the Select tool: this item, or `None` for empty space.
    Picked(Option<Item>),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plane_round_trips_points() {
        let plane = Plane {
            origin: [10.0, 0.0, 5.0],
            x_dir: [0.0, 1.0, 0.0],
            normal: [1.0, 0.0, 0.0],
        };
        assert_eq!(plane.y_dir(), [0.0, 0.0, 1.0]);
        let w = plane.to_world([3.0, 4.0]);
        assert_eq!(w, [10.0, 3.0, 9.0]);
        assert_eq!(plane.to_sketch(w), [3.0, 4.0]);
    }

    #[test]
    fn remapping_drops_constraints_on_removed_geometry() {
        let c = Constraint::Coincident(PointRef::new(0, Pos::End), PointRef::new(2, Pos::Start));
        let shifted = c.remap(|g| Some(if g > 1 { g - 1 } else { g }));
        assert_eq!(
            shifted,
            Some(Constraint::Coincident(
                PointRef::new(0, Pos::End),
                PointRef::new(1, Pos::Start)
            ))
        );
        assert_eq!(c.remap(|g| (g != 2).then_some(g)), None);
        // The axes pass through untouched.
        let lock = Constraint::DistanceX(PointRef::ORIGIN, PointRef::new(3, Pos::Start), 5.0);
        assert_eq!(
            lock.remap(|g| Some(if g >= 0 { g - 1 } else { g }))
                .unwrap()
                .geos(),
            alloc::vec![H_AXIS, 2]
        );
    }

    #[test]
    fn values_read_and_write() {
        let c = Constraint::Radius(1, 5.0);
        assert_eq!(c.value(), Some(5.0));
        assert_eq!(c.with_value(7.0), Constraint::Radius(1, 7.0));
        assert_eq!(Constraint::Horizontal(0).value(), None);
        assert_eq!(
            Constraint::Horizontal(0).with_value(3.0),
            Constraint::Horizontal(0)
        );
    }

    #[test]
    fn validity() {
        let mut s = Sketch::new(Plane::GROUND);
        s.geos.push(Geo::new(Curve::Line([0.0; 2], [1.0, 0.0])));
        s.constraints.push(Constraint::Horizontal(0));
        s.constraints.push(Constraint::PointOnObject(
            PointRef::new(0, Pos::Start),
            V_AXIS,
        ));
        assert!(s.is_valid());
        s.constraints.push(Constraint::Vertical(1));
        assert!(!s.is_valid());
        s.constraints.pop();
        s.geos[0].curve = Curve::Line([f64::NAN, 0.0], [1.0, 0.0]);
        assert!(!s.is_valid());
    }

    #[test]
    fn freecad_point_numbers() {
        assert_eq!(
            [Pos::Edge, Pos::Start, Pos::End, Pos::Mid].map(Pos::freecad),
            [0, 1, 2, 3]
        );
    }
}
