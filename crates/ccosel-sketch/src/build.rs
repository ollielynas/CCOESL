//! Turning a selection into constraints: what each of FreeCAD's constraint buttons does with
//! what is selected.
//!
//! Dimensions are made with the value the geometry already has, as in FreeCAD, so adding one
//! never moves anything; the user then types the value they want.

use ccosel_proto::sketch::{Constraint, Curve, Item, PointRef, Pos, Sketch};

use crate::geom::{curve, point};
use crate::solve::measure;

/// One of the Sketcher's constraint commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Coincident,
    PointOnObject,
    Horizontal,
    Vertical,
    Parallel,
    Perpendicular,
    Tangent,
    Equal,
    Symmetric,
    /// FreeCAD's Lock: the point's X and Y distance from the origin.
    Lock,
    Distance,
    DistanceX,
    DistanceY,
    Radius,
    Diameter,
    Angle,
}

impl Kind {
    pub const ALL: [Self; 16] = [
        Self::Coincident,
        Self::PointOnObject,
        Self::Horizontal,
        Self::Vertical,
        Self::Parallel,
        Self::Perpendicular,
        Self::Tangent,
        Self::Equal,
        Self::Symmetric,
        Self::Lock,
        Self::Distance,
        Self::DistanceX,
        Self::DistanceY,
        Self::Radius,
        Self::Diameter,
        Self::Angle,
    ];

    /// What to select for it, as the error when the selection is wrong.
    pub fn needs(self) -> &'static str {
        match self {
            Self::Coincident => "Select two or more points.",
            Self::PointOnObject => "Select a point and a line, circle or arc.",
            Self::Horizontal => "Select lines, or two points.",
            Self::Vertical => "Select lines, or two points.",
            Self::Parallel => "Select two or more lines.",
            Self::Perpendicular => "Select two lines.",
            Self::Tangent => "Select two curves, at least one a circle or arc, or two lines.",
            Self::Equal => "Select two or more lines, or two or more circles and arcs.",
            Self::Symmetric => "Select two points and a line, or three points.",
            Self::Lock => "Select one or more points.",
            Self::Distance => "Select a line, two points, or a point and a line.",
            Self::DistanceX => "Select a line, two points, or a point.",
            Self::DistanceY => "Select a line, two points, or a point.",
            Self::Radius => "Select circles or arcs.",
            Self::Diameter => "Select circles or arcs.",
            Self::Angle => "Select one or two lines.",
        }
    }
}

fn is_line(sk: &Sketch, g: i32) -> bool {
    matches!(curve(sk, g), Some(Curve::Line(..)))
}

fn is_round(sk: &Sketch, g: i32) -> bool {
    matches!(curve(sk, g), Some(Curve::Circle { .. } | Curve::Arc { .. }))
}

/// A line's own points, so "Distance" on a line and on its two ends mean the same.
fn line_ends(g: i32) -> (PointRef, PointRef) {
    (PointRef::new(g, Pos::Start), PointRef::new(g, Pos::End))
}

fn split(sel: &[Item]) -> (Vec<PointRef>, Vec<i32>) {
    let mut points = Vec::new();
    let mut geos = Vec::new();
    for item in sel {
        match *item {
            Item::Point(p) => points.push(p),
            Item::Geo(g) => geos.push(g),
            Item::Constraint(_) => {}
        }
    }
    (points, geos)
}

fn dim(sk: &Sketch, c: Constraint) -> Option<Constraint> {
    Some(c.with_value(measure(sk, &c)?))
}

/// The constraints `kind` makes from `sel`, or what it needs selected instead.
pub fn constraints_for(
    kind: Kind,
    sel: &[Item],
    sk: &Sketch,
) -> Result<Vec<Constraint>, &'static str> {
    let (points, geos) = split(sel);
    let bad = kind.needs();
    let out: Vec<Constraint> = match kind {
        Kind::Coincident if points.len() >= 2 && geos.is_empty() => points
            .windows(2)
            .map(|w| Constraint::Coincident(w[0], w[1]))
            .collect(),
        Kind::PointOnObject if points.len() == 1 && geos.len() == 1 => {
            vec![Constraint::PointOnObject(points[0], geos[0])]
        }
        Kind::Horizontal | Kind::Vertical => {
            let h = kind == Kind::Horizontal;
            if points.is_empty()
                && !geos.is_empty()
                && geos.iter().all(|&g| g >= 0 && is_line(sk, g))
            {
                geos.iter()
                    .map(|&g| {
                        if h {
                            Constraint::Horizontal(g)
                        } else {
                            Constraint::Vertical(g)
                        }
                    })
                    .collect()
            } else if points.len() == 2 && geos.is_empty() {
                vec![if h {
                    Constraint::HorizontalPoints(points[0], points[1])
                } else {
                    Constraint::VerticalPoints(points[0], points[1])
                }]
            } else {
                return Err(bad);
            }
        }
        Kind::Parallel
            if points.is_empty() && geos.len() >= 2 && geos.iter().all(|&g| is_line(sk, g)) =>
        {
            geos[1..]
                .iter()
                .map(|&g| Constraint::Parallel(geos[0], g))
                .collect()
        }
        Kind::Perpendicular
            if points.is_empty() && geos.len() == 2 && geos.iter().all(|&g| is_line(sk, g)) =>
        {
            vec![Constraint::Perpendicular(geos[0], geos[1])]
        }
        Kind::Tangent
            if points.is_empty()
                && geos.len() == 2
                && geos.iter().all(|&g| g >= 0)
                && (geos.iter().any(|&g| is_round(sk, g))
                    || geos.iter().all(|&g| is_line(sk, g))) =>
        {
            vec![Constraint::Tangent(geos[0], geos[1])]
        }
        Kind::Equal
            if points.is_empty()
                && geos.len() >= 2
                && geos.iter().all(|&g| g >= 0)
                && (geos.iter().all(|&g| is_line(sk, g))
                    || geos.iter().all(|&g| is_round(sk, g))) =>
        {
            geos[1..]
                .iter()
                .map(|&g| Constraint::Equal(geos[0], g))
                .collect()
        }
        Kind::Symmetric if points.len() == 2 && geos.len() == 1 && is_line(sk, geos[0]) => {
            vec![Constraint::Symmetric(points[0], points[1], geos[0])]
        }
        Kind::Symmetric if points.len() == 3 && geos.is_empty() => {
            vec![Constraint::SymmetricPoint(points[0], points[1], points[2])]
        }
        Kind::Lock if !points.is_empty() && geos.is_empty() => {
            let mut out = Vec::new();
            for &p in &points {
                let at = point(sk, p).ok_or(bad)?;
                out.push(Constraint::DistanceX(PointRef::ORIGIN, p, at[0]));
                out.push(Constraint::DistanceY(PointRef::ORIGIN, p, at[1]));
            }
            out
        }
        Kind::Distance => {
            let c = match (points.as_slice(), geos.as_slice()) {
                ([], [g]) if *g >= 0 && is_line(sk, *g) => Constraint::Distance(*g, 0.0),
                ([a, b], []) => Constraint::DistancePoints(*a, *b, 0.0),
                ([a], [g]) if is_line(sk, *g) => Constraint::DistancePointLine(*a, *g, 0.0),
                _ => return Err(bad),
            };
            vec![dim(sk, c).ok_or(bad)?]
        }
        Kind::DistanceX | Kind::DistanceY => {
            let (a, b) = match (points.as_slice(), geos.as_slice()) {
                ([], [g]) if *g >= 0 && is_line(sk, *g) => line_ends(*g),
                ([a, b], []) => (*a, *b),
                ([p], []) => (PointRef::ORIGIN, *p),
                _ => return Err(bad),
            };
            let c = if kind == Kind::DistanceX {
                Constraint::DistanceX(a, b, 0.0)
            } else {
                Constraint::DistanceY(a, b, 0.0)
            };
            vec![dim(sk, c).ok_or(bad)?]
        }
        Kind::Radius | Kind::Diameter
            if points.is_empty() && !geos.is_empty() && geos.iter().all(|&g| is_round(sk, g)) =>
        {
            geos.iter()
                .map(|&g| {
                    let c = if kind == Kind::Radius {
                        Constraint::Radius(g, 0.0)
                    } else {
                        Constraint::Diameter(g, 0.0)
                    };
                    dim(sk, c).ok_or(bad)
                })
                .collect::<Result<_, _>>()?
        }
        Kind::Angle if points.is_empty() && geos.iter().all(|&g| is_line(sk, g)) => {
            let c = match geos.as_slice() {
                [g] if *g >= 0 => Constraint::Angle(*g, 0.0),
                [a, b] => Constraint::AngleBetween(*a, *b, 0.0),
                _ => return Err(bad),
            };
            vec![dim(sk, c).ok_or(bad)?]
        }
        _ => return Err(bad),
    };
    Ok(out)
}

#[cfg(test)]
mod tests;
