//! Drawing and editing a sketch: the Sketcher's geometry tools, and trim, extend, split,
//! fillet, offset, delete and construction mode.
//!
//! Each works on the sketch data and keeps its constraints sensible, the way FreeCAD does:
//! a trimmed or split curve keeps what still applies to it, new ends are attached to what they
//! were cut against, and anything that no longer makes sense (a length on a line that is now
//! shorter, a coincidence at a corner a fillet rounded off) is dropped.

use std::collections::BTreeSet;
use std::f64::consts::PI;

use ccosel_proto::sketch::{Constraint, Curve, Geo, Item, PointRef, Pos, Sketch};

use crate::geom::{
    P2, add, closest_on_segment, curve, dist, dot, intersections, len, line_circle, line_line,
    param, polar, scale, sub, sweep_to, within, wrap,
};

/// Lines within this of level or plumb are made horizontal or vertical as they are drawn, as
/// FreeCAD's auto-constraints do (radians; about 2°).
const AUTO_HV: f64 = 0.035;

/// What a newly drawn point landed on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Attach {
    Nothing,
    Point(PointRef),
    Curve(i32),
}

fn push(sk: &mut Sketch, curve: Curve, construction: bool) -> i32 {
    sk.geos.push(Geo {
        curve,
        construction,
        fixed: false,
    });
    sk.geos.len() as i32 - 1
}

/// Hold `p` where it landed: coincident with a point, or on a curve.
pub fn attach(sk: &mut Sketch, p: PointRef, to: Attach) {
    match to {
        Attach::Point(q) if q != p => sk.constraints.push(Constraint::Coincident(p, q)),
        Attach::Curve(g) if g != p.geo => sk.constraints.push(Constraint::PointOnObject(p, g)),
        _ => {}
    }
}

/// Make line `g` horizontal or vertical if it is drawn nearly so. Whether it did.
pub fn auto_level(sk: &mut Sketch, g: i32) -> bool {
    let Some(Curve::Line(a, b)) = curve(sk, g) else {
        return false;
    };
    let d = sub(b, a);
    if len(d) < 1e-9 {
        return false;
    }
    let angle = d[1].atan2(d[0]).abs();
    if angle < AUTO_HV || (PI - angle) < AUTO_HV {
        sk.constraints.push(Constraint::Horizontal(g));
        true
    } else if (angle - PI / 2.0).abs() < AUTO_HV {
        sk.constraints.push(Constraint::Vertical(g));
        true
    } else {
        false
    }
}

pub fn add_point(sk: &mut Sketch, p: P2) -> i32 {
    push(sk, Curve::Point(p), false)
}

pub fn add_line(sk: &mut Sketch, a: P2, b: P2, construction: bool) -> i32 {
    push(sk, Curve::Line(a, b), construction)
}

/// FreeCAD's rectangle: four lines, joined at the corners, two level and two plumb. Returns
/// the four ids, bottom first, going round.
pub fn add_rectangle(sk: &mut Sketch, a: P2, c: P2, construction: bool) -> [i32; 4] {
    let corners = [a, [c[0], a[1]], c, [a[0], c[1]]];
    let ids: [i32; 4] =
        std::array::from_fn(|i| add_line(sk, corners[i], corners[(i + 1) % 4], construction));
    for i in 0..4 {
        sk.constraints.push(Constraint::Coincident(
            PointRef::new(ids[i], Pos::End),
            PointRef::new(ids[(i + 1) % 4], Pos::Start),
        ));
    }
    sk.constraints.push(Constraint::Horizontal(ids[0]));
    sk.constraints.push(Constraint::Horizontal(ids[2]));
    sk.constraints.push(Constraint::Vertical(ids[1]));
    sk.constraints.push(Constraint::Vertical(ids[3]));
    ids
}

pub fn add_circle(sk: &mut Sketch, center: P2, radius: f64, construction: bool) -> i32 {
    push(
        sk,
        Curve::Circle {
            center,
            radius: radius.abs(),
        },
        construction,
    )
}

/// An arc about `center`, counter-clockwise from the direction of `from` to that of `to`, with
/// the radius of `from`.
pub fn add_arc(sk: &mut Sketch, center: P2, from: P2, to: P2, construction: bool) -> i32 {
    let start = (from[1] - center[1]).atan2(from[0] - center[0]);
    let end = (to[1] - center[1]).atan2(to[0] - center[0]);
    let mut span = sweep_to(start, end);
    if span < 1e-9 {
        span = 2.0 * PI;
    }
    push(
        sk,
        Curve::Arc {
            center,
            radius: dist(from, center),
            start: wrap(start),
            end: wrap(start) + span,
        },
        construction,
    )
}

/// External geometry: a model edge copied onto the sketch plane, fixed, as construction.
pub fn add_external(sk: &mut Sketch, a: P2, b: P2) -> i32 {
    sk.geos.push(Geo {
        curve: Curve::Line(a, b),
        construction: true,
        fixed: true,
    });
    sk.geos.len() as i32 - 1
}

/// Remove geometry (and every constraint naming it), renumbering what follows.
fn remove_geos(sk: &mut Sketch, gone: &BTreeSet<i32>) {
    if gone.is_empty() {
        return;
    }
    let map = |g: i32| -> Option<i32> {
        if g < 0 {
            return Some(g);
        }
        if gone.contains(&g) {
            return None;
        }
        Some(g - gone.range(..g).count() as i32)
    };
    sk.constraints = sk.constraints.iter().filter_map(|c| c.remap(map)).collect();
    let mut i = 0;
    sk.geos.retain(|_| {
        let keep = !gone.contains(&i);
        i += 1;
        keep
    });
}

/// Delete what is selected: geometry with its constraints, constraints, and a point's
/// constraints.
pub fn delete(sk: &mut Sketch, items: &[Item]) {
    let mut cons: BTreeSet<usize> = BTreeSet::new();
    let mut geos: BTreeSet<i32> = BTreeSet::new();
    for item in items {
        match *item {
            Item::Geo(g) if g >= 0 && (g as usize) < sk.geos.len() => {
                geos.insert(g);
            }
            Item::Constraint(c) => {
                cons.insert(c as usize);
            }
            Item::Point(p) => {
                for (i, c) in sk.constraints.iter().enumerate() {
                    if c.points().contains(&p) {
                        cons.insert(i);
                    }
                }
            }
            _ => {}
        }
    }
    let mut i = 0;
    sk.constraints.retain(|_| {
        let keep = !cons.contains(&i);
        i += 1;
        keep
    });
    remove_geos(sk, &geos);
}

/// Flip geometry between normal and construction.
pub fn toggle_construction(sk: &mut Sketch, geos: &[i32]) {
    for &g in geos {
        if let Some(geo) = usize::try_from(g).ok().and_then(|g| sk.geos.get_mut(g))
            && !geo.fixed
        {
            geo.construction = !geo.construction;
        }
    }
}

/// Drop constraints that pin point `p` (it has moved), and lengths of `g` (it has changed).
fn release_end(sk: &mut Sketch, g: i32, p: Pos) {
    let r = PointRef::new(g, p);
    sk.constraints.retain(|c| {
        !c.points().contains(&r) && !matches!(c, Constraint::Distance(x, _) if *x == g)
    });
}

/// Move every constraint on point `from` to point `to`.
fn move_point_constraints(sk: &mut Sketch, from: PointRef, to: PointRef) {
    for c in sk.constraints.iter_mut() {
        if c.points().contains(&from) {
            let swap = |r: PointRef| if r == from { to } else { r };
            *c = match *c {
                Constraint::Coincident(a, b) => Constraint::Coincident(swap(a), swap(b)),
                Constraint::PointOnObject(a, g) => Constraint::PointOnObject(swap(a), g),
                Constraint::HorizontalPoints(a, b) => {
                    Constraint::HorizontalPoints(swap(a), swap(b))
                }
                Constraint::VerticalPoints(a, b) => Constraint::VerticalPoints(swap(a), swap(b)),
                Constraint::Symmetric(a, b, g) => Constraint::Symmetric(swap(a), swap(b), g),
                Constraint::SymmetricPoint(a, b, m) => {
                    Constraint::SymmetricPoint(swap(a), swap(b), swap(m))
                }
                Constraint::DistancePoints(a, b, v) => {
                    Constraint::DistancePoints(swap(a), swap(b), v)
                }
                Constraint::DistancePointLine(a, g, v) => {
                    Constraint::DistancePointLine(swap(a), g, v)
                }
                Constraint::DistanceX(a, b, v) => Constraint::DistanceX(swap(a), swap(b), v),
                Constraint::DistanceY(a, b, v) => Constraint::DistanceY(swap(a), swap(b), v),
                other => other,
            };
        }
    }
}

/// Where along `g` every other curve crosses it, with the curve that does. Along a line, in
/// `0..=1`; around an arc, in radians from its start; around a circle, radians from +X.
fn cuts(sk: &Sketch, g: i32) -> Vec<(f64, i32)> {
    let Some(c) = curve(sk, g) else {
        return Vec::new();
    };
    let mut out: Vec<(f64, i32)> = Vec::new();
    for (i, other) in sk.geos.iter().enumerate() {
        let i = i as i32;
        if i == g || matches!(other.curve, Curve::Point(_)) {
            continue;
        }
        for p in intersections(&c, &other.curve) {
            out.push((param(&c, p), i));
        }
    }
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    out
}

/// A piece of a line or arc: the part of `c` between parameters `lo` and `hi`.
fn piece(c: Curve, lo: f64, hi: f64) -> Curve {
    match c {
        Curve::Line(a, b) => {
            Curve::Line(add(a, scale(sub(b, a), lo)), add(a, scale(sub(b, a), hi)))
        }
        Curve::Arc {
            center,
            radius,
            start,
            ..
        } => Curve::Arc {
            center,
            radius,
            start: start + lo,
            end: start + hi,
        },
        other => other,
    }
}

fn span(c: &Curve) -> f64 {
    match *c {
        Curve::Arc { start, end, .. } => end - start,
        _ => 1.0,
    }
}

/// Cut a line or arc in two at parameter `t`, the new piece taking the far end with its
/// constraints. Returns the new piece's id.
fn cut_in_two(sk: &mut Sketch, g: i32, t: f64, t2: f64) -> i32 {
    let c = sk.geos[g as usize].curve;
    let construction = sk.geos[g as usize].construction;
    let s = span(&c);
    sk.geos[g as usize].curve = piece(c, 0.0, t);
    let new = push(sk, piece(c, t2, s), construction);
    move_point_constraints(sk, PointRef::new(g, Pos::End), PointRef::new(new, Pos::End));
    sk.constraints
        .retain(|x| !matches!(x, Constraint::Distance(y, _) if *y == g));
    // The two pieces are still one line, or one circle.
    match c {
        Curve::Line(..) => sk.constraints.push(Constraint::Tangent(g, new)),
        _ => {
            sk.constraints.push(Constraint::Coincident(
                PointRef::new(g, Pos::Mid),
                PointRef::new(new, Pos::Mid),
            ));
            sk.constraints.push(Constraint::Equal(g, new));
        }
    }
    new
}

/// Trim: remove the part of curve `g` around `at`, between the nearest curves crossing it (or
/// its ends). A curve crossed by nothing is deleted.
pub fn trim(sk: &mut Sketch, g: i32, at: P2) -> Result<(), &'static str> {
    let c = curve(sk, g)
        .filter(|_| g >= 0)
        .ok_or("Pick a line, circle or arc to trim.")?;
    let t = param(&c, at);
    let cuts = cuts(sk, g);
    match c {
        Curve::Point(_) => return Err("Pick a line, circle or arc to trim."),
        Curve::Circle { center, radius } => {
            if cuts.len() < 2 {
                delete(sk, &[Item::Geo(g)]);
                return Ok(());
            }
            // The removed piece runs from the last cut before `t` to the first after,
            // round the circle; the arc left is the rest.
            let after = cuts.iter().find(|x| x.0 > t).unwrap_or(&cuts[0]);
            let before = cuts
                .iter()
                .rev()
                .find(|x| x.0 < t)
                .unwrap_or(&cuts[cuts.len() - 1]);
            let start = after.0;
            let mut end = before.0;
            if end <= start {
                end += 2.0 * PI;
            }
            sk.geos[g as usize].curve = Curve::Arc {
                center,
                radius,
                start,
                end,
            };
            attach(sk, PointRef::new(g, Pos::Start), Attach::Curve(after.1));
            attach(sk, PointRef::new(g, Pos::End), Attach::Curve(before.1));
        }
        Curve::Line(..) | Curve::Arc { .. } => {
            let s = span(&c);
            let eps = 1e-9 * s.max(1.0);
            let inner: Vec<(f64, i32)> = cuts
                .into_iter()
                .filter(|x| x.0 > eps && x.0 < s - eps)
                .collect();
            let lo = inner.iter().rev().find(|x| x.0 < t).copied();
            let hi = inner.iter().find(|x| x.0 > t).copied();
            match (lo, hi) {
                (None, None) => delete(sk, &[Item::Geo(g)]),
                (Some((l, by)), None) => {
                    release_end(sk, g, Pos::End);
                    sk.geos[g as usize].curve = piece(c, 0.0, l);
                    attach(sk, PointRef::new(g, Pos::End), Attach::Curve(by));
                }
                (None, Some((h, by))) => {
                    release_end(sk, g, Pos::Start);
                    sk.geos[g as usize].curve = piece(c, h, s);
                    attach(sk, PointRef::new(g, Pos::Start), Attach::Curve(by));
                }
                (Some((l, by_l)), Some((h, by_h))) => {
                    let new = cut_in_two(sk, g, l, h);
                    // Cut apart, not split: the two pieces are no longer joined end to end.
                    attach(sk, PointRef::new(g, Pos::End), Attach::Curve(by_l));
                    attach(sk, PointRef::new(new, Pos::Start), Attach::Curve(by_h));
                }
            }
        }
    }
    Ok(())
}

/// Split: cut curve `g` in two at `at`, the pieces joined end to end.
pub fn split(sk: &mut Sketch, g: i32, at: P2) -> Result<(), &'static str> {
    let c = curve(sk, g)
        .filter(|_| g >= 0)
        .ok_or("Pick a line or arc to split.")?;
    if !matches!(c, Curve::Line(..) | Curve::Arc { .. }) {
        return Err("Pick a line or arc to split.");
    }
    let s = span(&c);
    let t = param(&c, at).clamp(0.0, s);
    if t < 1e-6 * s || t > s * (1.0 - 1e-6) {
        return Err("Pick a point away from the ends.");
    }
    let new = cut_in_two(sk, g, t, t);
    sk.constraints.push(Constraint::Coincident(
        PointRef::new(g, Pos::End),
        PointRef::new(new, Pos::Start),
    ));
    Ok(())
}

/// Extend: lengthen line or arc `g` from the end nearer `near` to the next curve it would meet.
pub fn extend(sk: &mut Sketch, g: i32, near: P2) -> Result<(), &'static str> {
    let c = curve(sk, g)
        .filter(|_| g >= 0)
        .ok_or("Pick a line or arc near the end to extend.")?;
    let none = "It would not meet anything.";
    match c {
        Curve::Line(a, b) => {
            let at_end = dist(near, b) <= dist(near, a);
            let mut best: Option<(f64, i32, P2)> = None;
            for (i, other) in sk.geos.iter().enumerate() {
                let i = i as i32;
                if i == g {
                    continue;
                }
                let hits = match other.curve {
                    Curve::Line(p, q) => line_line(a, b, p, q).into_iter().collect(),
                    Curve::Circle { center, radius } | Curve::Arc { center, radius, .. } => {
                        line_circle(a, b, center, radius)
                    }
                    Curve::Point(_) => Vec::new(),
                };
                for p in hits {
                    if !within(&other.curve, p) {
                        continue;
                    }
                    let t = param(&c, p);
                    let beyond = if at_end { t - 1.0 } else { -t };
                    if beyond > 1e-9 && best.is_none_or(|(d, _, _)| beyond < d) {
                        best = Some((beyond, i, p));
                    }
                }
            }
            let (_, by, p) = best.ok_or(none)?;
            let pos = if at_end { Pos::End } else { Pos::Start };
            release_end(sk, g, pos);
            sk.geos[g as usize].curve = if at_end {
                Curve::Line(a, p)
            } else {
                Curve::Line(p, b)
            };
            attach(sk, PointRef::new(g, pos), Attach::Curve(by));
            Ok(())
        }
        Curve::Arc {
            center,
            radius,
            start,
            end,
        } => {
            let at_end =
                dist(near, polar(center, radius, end)) <= dist(near, polar(center, radius, start));
            let full = Curve::Circle { center, radius };
            let mut best: Option<(f64, i32)> = None;
            for (i, other) in sk.geos.iter().enumerate() {
                let i = i as i32;
                if i == g {
                    continue;
                }
                for p in intersections(&full, &other.curve) {
                    let ang = (p[1] - center[1]).atan2(p[0] - center[0]);
                    let beyond = if at_end {
                        sweep_to(end, ang)
                    } else {
                        sweep_to(ang, start)
                    };
                    if beyond > 1e-9
                        && beyond < 2.0 * PI - (end - start) - 1e-9
                        && best.is_none_or(|(d, _)| beyond < d)
                    {
                        best = Some((beyond, i));
                    }
                }
            }
            let (by_angle, by) = best.ok_or(none)?;
            let pos = if at_end { Pos::End } else { Pos::Start };
            release_end(sk, g, pos);
            sk.geos[g as usize].curve = if at_end {
                Curve::Arc {
                    center,
                    radius,
                    start,
                    end: end + by_angle,
                }
            } else {
                Curve::Arc {
                    center,
                    radius,
                    start: start - by_angle,
                    end,
                }
            };
            sk.geos[g as usize].curve = crate::geom::normalize(sk.geos[g as usize].curve);
            attach(sk, PointRef::new(g, pos), Attach::Curve(by));
            Ok(())
        }
        _ => Err("Pick a line or arc near the end to extend."),
    }
}

/// Fillet: round the corner nearest `near` (where two lines end together) with an arc of
/// `radius`, tangent to both.
pub fn fillet(sk: &mut Sketch, near: P2, radius: f64, tol: f64) -> Result<i32, &'static str> {
    let need = "Pick a corner where two lines meet.";
    let mut ends: Vec<(i32, Pos, P2, P2)> = Vec::new();
    for (i, geo) in sk.geos.iter().enumerate() {
        if let Curve::Line(a, b) = geo.curve {
            if dist(a, near) <= tol {
                ends.push((i as i32, Pos::Start, a, b));
            }
            if dist(b, near) <= tol {
                ends.push((i as i32, Pos::End, b, a));
            }
        }
    }
    let [(g1, p1, c1, far1), (g2, p2, c2, far2)] = ends[..] else {
        return Err(need);
    };
    if g1 == g2 || dist(c1, c2) > tol {
        return Err(need);
    }
    let corner = scale(add(c1, c2), 0.5);
    let u1 = scale(sub(far1, corner), 1.0 / len(sub(far1, corner)).max(1e-300));
    let u2 = scale(sub(far2, corner), 1.0 / len(sub(far2, corner)).max(1e-300));
    let theta = dot(u1, u2).clamp(-1.0, 1.0).acos();
    if !(1e-6..PI - 1e-6).contains(&theta) || radius.is_nan() || radius <= 0.0 {
        return Err("Those lines are in line; there is no corner to round.");
    }
    let back = radius / (theta / 2.0).tan();
    if back >= dist(far1, corner) || back >= dist(far2, corner) {
        return Err("That radius is too big for these lines.");
    }
    let t1 = add(corner, scale(u1, back));
    let t2 = add(corner, scale(u2, back));
    let bis = scale(add(u1, u2), 1.0 / len(add(u1, u2)));
    let center = add(corner, scale(bis, radius / (theta / 2.0).sin()));
    let a1 = (t1[1] - center[1]).atan2(t1[0] - center[0]);
    let a2 = (t2[1] - center[1]).atan2(t2[0] - center[0]);
    // The short way round, which is the side facing the corner.
    let (from, to, first_is_1) = if sweep_to(a1, a2) < PI {
        (t1, t2, true)
    } else {
        (t2, t1, false)
    };

    release_end(sk, g1, p1);
    release_end(sk, g2, p2);
    let set = |sk: &mut Sketch, g: i32, p: Pos, to: P2| {
        if let Curve::Line(a, b) = sk.geos[g as usize].curve {
            sk.geos[g as usize].curve = if p == Pos::Start {
                Curve::Line(to, b)
            } else {
                Curve::Line(a, to)
            };
        }
    };
    set(sk, g1, p1, t1);
    set(sk, g2, p2, t2);
    let arc = add_arc(sk, center, from, to, false);
    let (s1, s2) = if first_is_1 {
        (Pos::Start, Pos::End)
    } else {
        (Pos::End, Pos::Start)
    };
    sk.constraints.extend([
        Constraint::Coincident(PointRef::new(g1, p1), PointRef::new(arc, s1)),
        Constraint::Coincident(PointRef::new(g2, p2), PointRef::new(arc, s2)),
        Constraint::Tangent(g1, arc),
        Constraint::Tangent(g2, arc),
    ]);
    Ok(arc)
}

/// Offset: a copy of each line, circle and arc, `d` to its left (a line, walking from start to
/// end) or outward (a circle or arc; inward if negative). Returns the new ids.
pub fn offset(sk: &mut Sketch, geos: &[i32], d: f64) -> Vec<i32> {
    let mut out = Vec::new();
    for &g in geos {
        let Some(c) = curve(sk, g).filter(|_| g >= 0) else {
            continue;
        };
        let moved = match c {
            Curve::Line(a, b) => {
                let dir = sub(b, a);
                let l = len(dir);
                if l < 1e-12 {
                    continue;
                }
                let n = [-dir[1] / l * d, dir[0] / l * d];
                Curve::Line(add(a, n), add(b, n))
            }
            Curve::Circle { center, radius } if radius + d > 0.0 => Curve::Circle {
                center,
                radius: radius + d,
            },
            Curve::Arc {
                center,
                radius,
                start,
                end,
            } if radius + d > 0.0 => Curve::Arc {
                center,
                radius: radius + d,
                start,
                end,
            },
            _ => continue,
        };
        out.push(push(sk, moved, false));
    }
    out
}

/// Where on a curve a click lands, for the tools that take a point on one.
pub fn closest_on(sk: &Sketch, g: i32, p: P2) -> Option<P2> {
    // The axes are infinite lines, not the unit segments `curve` stands them in with.
    match g {
        ccosel_proto::sketch::H_AXIS => return Some([p[0], 0.0]),
        ccosel_proto::sketch::V_AXIS => return Some([0.0, p[1]]),
        _ => {}
    }
    Some(match curve(sk, g)? {
        Curve::Line(a, b) => closest_on_segment(a, b, p).0,
        Curve::Circle { center, radius } | Curve::Arc { center, radius, .. } => {
            let d = sub(p, center);
            let l = len(d);
            if l < 1e-12 {
                return None;
            }
            add(center, scale(d, radius / l))
        }
        Curve::Point(q) => q,
    })
}

#[cfg(test)]
mod tests;
