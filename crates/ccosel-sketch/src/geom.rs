//! Reading positions out of a sketch, distances to its curves, and where its curves cross.

use std::f64::consts::TAU;

use ccosel_proto::sketch::{Curve, H_AXIS, PointRef, Pos, Sketch, V_AXIS};

pub type P2 = [f64; 2];

pub fn sub(a: P2, b: P2) -> P2 {
    [a[0] - b[0], a[1] - b[1]]
}

pub fn add(a: P2, b: P2) -> P2 {
    [a[0] + b[0], a[1] + b[1]]
}

pub fn scale(a: P2, s: f64) -> P2 {
    [a[0] * s, a[1] * s]
}

pub fn dot(a: P2, b: P2) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}

pub fn cross(a: P2, b: P2) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}

pub fn len(a: P2) -> f64 {
    dot(a, a).sqrt()
}

pub fn dist(a: P2, b: P2) -> f64 {
    len(sub(a, b))
}

pub fn lerp(a: P2, b: P2, t: f64) -> P2 {
    add(a, scale(sub(b, a), t))
}

pub fn polar(c: P2, r: f64, angle: f64) -> P2 {
    [c[0] + r * angle.cos(), c[1] + r * angle.sin()]
}

/// `a` wrapped into `[0, 2π)`.
pub fn wrap(a: f64) -> f64 {
    let w = a.rem_euclid(TAU);
    if w >= TAU { 0.0 } else { w }
}

/// Angle `a` measured counter-clockwise from `start`, in `[0, 2π)`.
pub fn sweep_to(start: f64, a: f64) -> f64 {
    wrap(a - start)
}

/// An arc with `end` put in `(start, start + 2π]` and a positive radius, as `Curve::Arc`
/// promises.
pub fn normalize(c: Curve) -> Curve {
    match c {
        Curve::Arc {
            center,
            radius,
            start,
            end,
        } => {
            let (start, end) = if radius < 0.0 {
                (start + std::f64::consts::PI, end + std::f64::consts::PI)
            } else {
                (start, end)
            };
            let start = wrap(start);
            let mut span = sweep_to(start, end);
            if span <= 1e-12 {
                span = TAU;
            }
            Curve::Arc {
                center,
                radius: radius.abs(),
                start,
                end: start + span,
            }
        }
        Curve::Circle { center, radius } => Curve::Circle {
            center,
            radius: radius.abs(),
        },
        other => other,
    }
}

/// Where a point of a curve is. `None` for a position the curve does not have.
pub fn curve_point(c: &Curve, pos: Pos) -> Option<P2> {
    match (c, pos) {
        (Curve::Point(p), Pos::Start | Pos::Mid) => Some(*p),
        (Curve::Line(a, _), Pos::Start) => Some(*a),
        (Curve::Line(_, b), Pos::End) => Some(*b),
        (Curve::Circle { center, .. } | Curve::Arc { center, .. }, Pos::Mid) => Some(*center),
        (
            Curve::Arc {
                center,
                radius,
                start,
                ..
            },
            Pos::Start,
        ) => Some(polar(*center, *radius, *start)),
        (
            Curve::Arc {
                center,
                radius,
                end,
                ..
            },
            Pos::End,
        ) => Some(polar(*center, *radius, *end)),
        _ => None,
    }
}

/// Where `r` is in `sketch`: a geometry's point, or the origin.
pub fn point(sketch: &Sketch, r: PointRef) -> Option<P2> {
    if r.geo == H_AXIS || r.geo == V_AXIS {
        return (r.pos == Pos::Start).then_some([0.0, 0.0]);
    }
    curve_point(&sketch.geos.get(usize::try_from(r.geo).ok()?)?.curve, r.pos)
}

/// A geometry's curve, the axes being lines through the origin.
pub fn curve(sketch: &Sketch, g: i32) -> Option<Curve> {
    match g {
        H_AXIS => Some(Curve::Line([0.0, 0.0], [1.0, 0.0])),
        V_AXIS => Some(Curve::Line([0.0, 0.0], [0.0, 1.0])),
        _ => Some(sketch.geos.get(usize::try_from(g).ok()?)?.curve),
    }
}

/// The points a curve has, for snapping and picking.
pub fn points_of(c: &Curve) -> &'static [Pos] {
    match c {
        Curve::Point(_) => &[Pos::Start],
        Curve::Line(..) => &[Pos::Start, Pos::End],
        Curve::Circle { .. } => &[Pos::Mid],
        Curve::Arc { .. } => &[Pos::Start, Pos::End, Pos::Mid],
    }
}

/// The closest point to `p` on the segment `a`–`b`, and its parameter in `[0, 1]`.
pub fn closest_on_segment(a: P2, b: P2, p: P2) -> (P2, f64) {
    let d = sub(b, a);
    let l2 = dot(d, d);
    let t = if l2 > 0.0 {
        (dot(sub(p, a), d) / l2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (lerp(a, b, t), t)
}

/// Whether angle `a` lies on the arc from `start` to `end` (with a little slack).
pub fn on_arc(start: f64, end: f64, a: f64) -> bool {
    sweep_to(start, a) <= (end - start) + 1e-9 || sweep_to(start, a) >= TAU - 1e-9
}

/// How far `p` is from a curve (the axes being infinite lines).
pub fn distance_to(sketch: &Sketch, g: i32, p: P2) -> Option<f64> {
    if g == H_AXIS {
        return Some(p[1].abs());
    }
    if g == V_AXIS {
        return Some(p[0].abs());
    }
    Some(match curve(sketch, g)? {
        Curve::Point(q) => dist(p, q),
        Curve::Line(a, b) => dist(closest_on_segment(a, b, p).0, p),
        Curve::Circle { center, radius } => (dist(p, center) - radius).abs(),
        Curve::Arc {
            center,
            radius,
            start,
            end,
        } => {
            let ang = (p[1] - center[1]).atan2(p[0] - center[0]);
            if on_arc(start, end, ang) {
                (dist(p, center) - radius).abs()
            } else {
                dist(p, polar(center, radius, start)).min(dist(p, polar(center, radius, end)))
            }
        }
    })
}

/// The nearest point of any geometry (or the origin) within `tol` of `p`.
pub fn nearest_point(sketch: &Sketch, p: P2, tol: f64) -> Option<PointRef> {
    let mut best: Option<(f64, PointRef)> = None;
    let mut consider = |r: PointRef, q: P2| {
        let d = dist(p, q);
        if d <= tol && best.is_none_or(|(b, _)| d < b) {
            best = Some((d, r));
        }
    };
    consider(PointRef::ORIGIN, [0.0, 0.0]);
    for (i, g) in sketch.geos.iter().enumerate() {
        for &pos in points_of(&g.curve) {
            if let Some(q) = curve_point(&g.curve, pos) {
                consider(PointRef::new(i as i32, pos), q);
            }
        }
    }
    best.map(|(_, r)| r)
}

/// The nearest curve within `tol` of `p`, the axes included (after everything else).
pub fn nearest_curve(sketch: &Sketch, p: P2, tol: f64) -> Option<i32> {
    let mut best: Option<(f64, i32)> = None;
    for g in 0..sketch.geos.len() as i32 {
        if let Some(d) = distance_to(sketch, g, p)
            && d <= tol
            && best.is_none_or(|(b, _)| d < b)
        {
            best = Some((d, g));
        }
    }
    if best.is_none() {
        for g in [H_AXIS, V_AXIS] {
            if let Some(d) = distance_to(sketch, g, p)
                && d <= tol
                && best.is_none_or(|(b, _)| d < b)
            {
                best = Some((d, g));
            }
        }
    }
    best.map(|(_, g)| g)
}

/// Where two curves cross, points of a curve itself excepted. Segments and arcs count only
/// within their extent.
pub fn intersections(a: &Curve, b: &Curve) -> Vec<P2> {
    let raw = match (a, b) {
        (Curve::Line(p, q), Curve::Line(r, s)) => line_line(*p, *q, *r, *s).into_iter().collect(),
        (Curve::Line(p, q), c) | (c, Curve::Line(p, q)) => match circle_of(c) {
            Some((center, radius)) => line_circle(*p, *q, center, radius),
            None => Vec::new(),
        },
        (c, d) => match (circle_of(c), circle_of(d)) {
            (Some((c1, r1)), Some((c2, r2))) => circle_circle(c1, r1, c2, r2),
            _ => Vec::new(),
        },
    };
    raw.into_iter()
        .filter(|p| within(a, *p) && within(b, *p))
        .collect()
}

fn circle_of(c: &Curve) -> Option<(P2, f64)> {
    match *c {
        Curve::Circle { center, radius } | Curve::Arc { center, radius, .. } => {
            Some((center, radius))
        }
        _ => None,
    }
}

/// Whether a point already known to be on the curve's full line or circle is within its
/// extent.
pub fn within(c: &Curve, p: P2) -> bool {
    match *c {
        Curve::Line(a, b) => {
            let d = sub(b, a);
            let l2 = dot(d, d);
            l2 > 0.0 && (-1e-9..=1.0 + 1e-9).contains(&(dot(sub(p, a), d) / l2))
        }
        Curve::Arc {
            center, start, end, ..
        } => on_arc(start, end, (p[1] - center[1]).atan2(p[0] - center[0])),
        Curve::Circle { .. } => true,
        Curve::Point(q) => dist(p, q) < 1e-9,
    }
}

/// Where the infinite lines through `p q` and `r s` cross.
pub fn line_line(p: P2, q: P2, r: P2, s: P2) -> Option<P2> {
    let d1 = sub(q, p);
    let d2 = sub(s, r);
    let den = cross(d1, d2);
    if den.abs() < 1e-12 {
        return None;
    }
    let t = cross(sub(r, p), d2) / den;
    Some(add(p, scale(d1, t)))
}

/// Where the infinite line through `p q` meets a circle.
pub fn line_circle(p: P2, q: P2, c: P2, r: f64) -> Vec<P2> {
    let d = sub(q, p);
    let f = sub(p, c);
    let a = dot(d, d);
    if a == 0.0 {
        return Vec::new();
    }
    let b = 2.0 * dot(f, d);
    let k = dot(f, f) - r * r;
    let disc = b * b - 4.0 * a * k;
    if disc < -1e-9 {
        return Vec::new();
    }
    let s = disc.max(0.0).sqrt();
    let mut out = vec![add(p, scale(d, (-b - s) / (2.0 * a)))];
    if s > 1e-12 {
        out.push(add(p, scale(d, (-b + s) / (2.0 * a))));
    }
    out
}

pub fn circle_circle(c1: P2, r1: f64, c2: P2, r2: f64) -> Vec<P2> {
    let d = dist(c1, c2);
    if d < 1e-12 || d > r1 + r2 + 1e-9 || d < (r1 - r2).abs() - 1e-9 {
        return Vec::new();
    }
    let a = (r1 * r1 - r2 * r2 + d * d) / (2.0 * d);
    let h = (r1 * r1 - a * a).max(0.0).sqrt();
    let m = add(c1, scale(sub(c2, c1), a / d));
    let off = scale([-(c2[1] - c1[1]), c2[0] - c1[0]], h / d);
    if h < 1e-12 {
        vec![m]
    } else {
        vec![add(m, off), sub(m, off)]
    }
}

/// How far along a curve a point on it is: `0..=1` along a line, radians from the start of an
/// arc, radians from the X axis on a circle.
pub fn param(c: &Curve, p: P2) -> f64 {
    match *c {
        Curve::Line(a, b) => {
            let d = sub(b, a);
            dot(sub(p, a), d) / dot(d, d).max(1e-300)
        }
        Curve::Circle { center, .. } => wrap((p[1] - center[1]).atan2(p[0] - center[0])),
        Curve::Arc { center, start, .. } => {
            sweep_to(start, (p[1] - center[1]).atan2(p[0] - center[0]))
        }
        Curve::Point(_) => 0.0,
    }
}

#[cfg(test)]
mod tests;
