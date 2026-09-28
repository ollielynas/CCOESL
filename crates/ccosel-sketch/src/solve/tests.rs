use ccosel_proto::sketch::{Geo, Plane, V_AXIS};

use super::*;
use crate::edit::add_rectangle;

fn sketch() -> Sketch {
    Sketch::new(Plane::GROUND)
}

fn line(s: &mut Sketch, a: P2, b: P2) -> i32 {
    s.geos.push(Geo::new(Curve::Line(a, b)));
    s.geos.len() as i32 - 1
}

fn p(g: i32, pos: Pos) -> PointRef {
    PointRef::new(g, pos)
}

fn at(s: &Sketch, r: PointRef) -> P2 {
    crate::geom::point(s, r).unwrap()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

#[test]
fn a_free_line_has_four_degrees_of_freedom() {
    let mut s = sketch();
    line(&mut s, [0.0, 0.0], [10.0, 3.0]);
    let r = solve(&mut s);
    assert!(r.converged);
    assert_eq!(r.dof, 4);
    assert!(!r.fully_constrained());
}

#[test]
fn horizontal_and_length_move_the_line_as_little_as_possible() {
    let mut s = sketch();
    let g = line(&mut s, [0.0, 0.0], [10.0, 1.0]);
    s.constraints.push(Constraint::Horizontal(g));
    s.constraints.push(Constraint::Distance(g, 20.0));
    let r = solve(&mut s);
    assert!(r.converged);
    assert_eq!(r.dof, 2);
    let (a, b) = (at(&s, p(g, Pos::Start)), at(&s, p(g, Pos::End)));
    assert!(close(a[1], b[1]));
    assert!(close(dist(a, b), 20.0));
    // Least change: the line grows about its middle rather than jumping somewhere.
    assert!(close((a[0] + b[0]) / 2.0, 5.0));
}

#[test]
fn a_dimensioned_locked_rectangle_is_fully_constrained() {
    let mut s = sketch();
    let [bottom, right, ..] = add_rectangle(&mut s, [1.0, 2.0], [31.0, 22.0], false);
    s.constraints.push(Constraint::Distance(bottom, 40.0));
    s.constraints.push(Constraint::Distance(right, 25.0));
    s.constraints.push(Constraint::Coincident(
        p(bottom, Pos::Start),
        PointRef::ORIGIN,
    ));
    let r = solve(&mut s);
    assert!(r.converged, "{r:?}");
    assert_eq!(r.dof, 0);
    assert!(r.fully_constrained());
    assert!(r.redundant.is_empty() && r.conflicting.is_empty());
    let far = at(&s, p(right, Pos::End));
    assert!(close(far[0], 40.0) && close(far[1], 25.0), "{far:?}");
}

#[test]
fn a_redundant_constraint_is_named_and_a_conflicting_one_too() {
    let mut s = sketch();
    let [bottom, _, top, _] = add_rectangle(&mut s, [0.0, 0.0], [10.0, 5.0], false);
    // Opposite sides of a rectangle are already parallel.
    s.constraints.push(Constraint::Parallel(bottom, top));
    let r = solve(&mut s);
    assert!(r.converged);
    assert_eq!(r.redundant, vec![s.constraints.len() - 1]);

    let mut s = sketch();
    let g = line(&mut s, [0.0, 0.0], [10.0, 0.0]);
    s.constraints.push(Constraint::Horizontal(g));
    s.constraints.push(Constraint::Vertical(g));
    s.constraints.push(Constraint::Distance(g, 10.0));
    let before = s.clone();
    let r = solve(&mut s);
    assert!(!r.converged);
    assert!(!r.conflicting.is_empty());
    assert_eq!(s, before, "an unsolvable sketch is left alone");
}

#[test]
fn every_kind_of_constraint_is_satisfied() {
    let mut s = sketch();
    let l1 = line(&mut s, [0.0, 0.0], [10.0, 1.0]);
    let l2 = line(&mut s, [12.0, 3.0], [20.0, 9.0]);
    s.geos.push(Geo::new(Curve::Circle {
        center: [40.0, 5.0],
        radius: 4.0,
    }));
    let c1 = 2;
    s.geos.push(Geo::new(Curve::Arc {
        center: [60.0, 0.0],
        radius: 6.0,
        start: 0.2,
        end: 2.0,
    }));
    let a1 = 3;
    s.geos.push(Geo::new(Curve::Point([3.0, 7.0])));
    let pt = 4;
    s.constraints = vec![
        Constraint::Coincident(p(l1, Pos::End), p(l2, Pos::Start)),
        Constraint::Perpendicular(l1, l2),
        Constraint::Tangent(l2, c1),
        Constraint::Equal(c1, a1),
        Constraint::Radius(a1, 5.0),
        Constraint::PointOnObject(p(pt, Pos::Start), c1),
        Constraint::DistanceX(PointRef::ORIGIN, p(l1, Pos::Start), 0.0),
        Constraint::DistanceY(PointRef::ORIGIN, p(l1, Pos::Start), 0.0),
        Constraint::Angle(l1, 0.3),
        Constraint::DistancePointLine(p(a1, Pos::Mid), l1, 30.0),
        Constraint::HorizontalPoints(p(a1, Pos::Mid), p(c1, Pos::Mid)),
    ];
    let r = solve(&mut s);
    assert!(r.converged, "{r:?}");
    assert!(r.malformed.is_empty());
    let d1 = sub(at(&s, p(l1, Pos::End)), at(&s, p(l1, Pos::Start)));
    assert!(close(d1[1].atan2(d1[0]), 0.3));
    let Curve::Arc { radius, .. } = s.geos[a1 as usize].curve else {
        panic!()
    };
    assert!(close(radius, 5.0));

    // And the rest, on a fresh sketch.
    let mut s = sketch();
    let l1 = line(&mut s, [0.0, 0.0], [10.0, 0.0]);
    let l2 = line(&mut s, [0.0, 5.0], [8.0, 7.0]);
    let l3 = line(&mut s, [3.0, -4.0], [4.0, 9.0]);
    s.geos.push(Geo::new(Curve::Circle {
        center: [30.0, 0.0],
        radius: 3.0,
    }));
    s.geos.push(Geo::new(Curve::Circle {
        center: [36.0, 1.0],
        radius: 2.0,
    }));
    s.constraints = vec![
        Constraint::Parallel(l1, l2),
        Constraint::Equal(l1, l2),
        Constraint::AngleBetween(l1, l3, 1.2),
        Constraint::VerticalPoints(p(l3, Pos::Start), p(l2, Pos::Start)),
        Constraint::Symmetric(p(l2, Pos::Start), p(l2, Pos::End), V_AXIS),
        Constraint::SymmetricPoint(p(3, Pos::Mid), p(4, Pos::Mid), p(l1, Pos::End)),
        Constraint::Tangent(3, 4),
        Constraint::Diameter(4, 3.0),
        Constraint::DistancePoints(p(l3, Pos::Start), p(l3, Pos::End), 12.0),
    ];
    let r = solve(&mut s);
    assert!(r.converged, "{r:?}");
    let a = at(&s, p(l2, Pos::Start));
    let b = at(&s, p(l2, Pos::End));
    assert!(
        close(a[0], -b[0]) && close(a[1], b[1]),
        "mirrored across the Y axis"
    );
}

#[test]
fn collinear_lines_via_tangent() {
    let mut s = sketch();
    let l1 = line(&mut s, [0.0, 0.0], [10.0, 0.0]);
    let l2 = line(&mut s, [12.0, 1.0], [20.0, 2.0]);
    s.constraints.push(Constraint::Tangent(l1, l2));
    assert!(solve(&mut s).converged);
    let d = at(&s, p(l2, Pos::End));
    let o = at(&s, p(l1, Pos::Start));
    let e = at(&s, p(l1, Pos::End));
    assert!(cross(sub(e, o), sub(d, o)).abs() < 1e-6);
}

#[test]
fn internal_tangency_stays_internal() {
    let mut s = sketch();
    s.geos.push(Geo::new(Curve::Circle {
        center: [0.0, 0.0],
        radius: 10.0,
    }));
    s.geos.push(Geo::new(Curve::Circle {
        center: [3.0, 0.0],
        radius: 4.0,
    }));
    s.constraints.push(Constraint::Tangent(0, 1));
    assert!(solve(&mut s).converged);
    let (
        Curve::Circle {
            radius: r1,
            center: c1,
        },
        Curve::Circle {
            radius: r2,
            center: c2,
        },
    ) = (s.geos[0].curve, s.geos[1].curve)
    else {
        panic!()
    };
    // Inside and touching: the centres are the difference of the radii apart, not the sum.
    assert!(
        close(dist(c1, c2), (r1 - r2).abs()),
        "{c1:?} {c2:?} {r1} {r2}"
    );
}

#[test]
fn malformed_constraints_are_reported_not_fatal() {
    let mut s = sketch();
    s.geos.push(Geo::new(Curve::Circle {
        center: [0.0, 0.0],
        radius: 1.0,
    }));
    s.constraints.push(Constraint::Horizontal(0));
    let r = solve(&mut s);
    assert!(r.converged);
    assert_eq!(r.malformed, vec![0]);
}

#[test]
fn dragging_a_free_end_follows_the_pointer() {
    let mut s = sketch();
    let g = line(&mut s, [0.0, 0.0], [10.0, 0.0]);
    let r = drag(
        &mut s,
        Grab::Point(p(g, Pos::End)),
        [10.0, 0.0],
        [12.0, 5.0],
    );
    assert!(r.converged);
    assert_eq!(at(&s, p(g, Pos::End)), [12.0, 5.0]);
    assert_eq!(at(&s, p(g, Pos::Start)), [0.0, 0.0]);
}

#[test]
fn dragging_a_constrained_end_stays_within_the_constraints() {
    let mut s = sketch();
    let g = line(&mut s, [0.0, 0.0], [10.0, 0.0]);
    s.constraints.push(Constraint::Horizontal(g));
    s.constraints.push(Constraint::Distance(g, 10.0));
    drag(
        &mut s,
        Grab::Point(p(g, Pos::End)),
        [10.0, 0.0],
        [15.0, 3.0],
    );
    let (a, b) = (at(&s, p(g, Pos::Start)), at(&s, p(g, Pos::End)));
    assert!(close(a[1], b[1]));
    assert!(close(dist(a, b), 10.0));
    assert!(close(b[1], 3.0), "it went as far as it could: {b:?}");
}

#[test]
fn dragging_edges_and_centres() {
    let mut s = sketch();
    let g = line(&mut s, [0.0, 0.0], [10.0, 0.0]);
    drag(&mut s, Grab::Edge(g), [5.0, 0.0], [5.0, 4.0]);
    assert_eq!(at(&s, p(g, Pos::Start)), [0.0, 4.0]);

    s.geos.push(Geo::new(Curve::Circle {
        center: [20.0, 0.0],
        radius: 2.0,
    }));
    drag(&mut s, Grab::Edge(1), [22.0, 0.0], [25.0, 0.0]);
    assert!(matches!(s.geos[1].curve, Curve::Circle { radius, .. } if close(radius, 5.0)));
    drag(
        &mut s,
        Grab::Point(p(1, Pos::Mid)),
        [20.0, 0.0],
        [21.0, 1.0],
    );
    assert_eq!(at(&s, p(1, Pos::Mid)), [21.0, 1.0]);

    s.geos.push(Geo::new(Curve::Arc {
        center: [0.0, 0.0],
        radius: 5.0,
        start: 0.0,
        end: 1.0,
    }));
    drag(&mut s, Grab::Point(p(2, Pos::End)), [0.0; 2], [0.0, 9.0]);
    let e = at(&s, p(2, Pos::End));
    assert!(
        close(e[0], 0.0) && close(e[1], 5.0),
        "keeps its radius: {e:?}"
    );

    s.geos.push(Geo::new(Curve::Point([1.0, 1.0])));
    drag(&mut s, Grab::Edge(3), [1.0, 1.0], [2.0, 2.0]);
    assert_eq!(at(&s, p(3, Pos::Start)), [2.0, 2.0]);
}

#[test]
fn fixed_geometry_does_not_move() {
    let mut s = sketch();
    s.geos.push(Geo {
        curve: Curve::Line([0.0, 0.0], [10.0, 0.0]),
        construction: true,
        fixed: true,
    });
    let g = line(&mut s, [0.0, 5.0], [10.0, 7.0]);
    s.constraints
        .push(Constraint::Coincident(p(g, Pos::Start), p(0, Pos::End)));
    let r = solve(&mut s);
    assert!(r.converged);
    assert_eq!(
        at(&s, p(0, Pos::End)),
        [10.0, 0.0],
        "fixed geometry is not touched at all"
    );
    let start = at(&s, p(g, Pos::Start));
    assert!(close(start[0], 10.0) && close(start[1], 0.0));
    drag(&mut s, Grab::Edge(0), [5.0, 0.0], [5.0, 9.0]);
    let e = at(&s, p(0, Pos::End));
    assert!(close(e[0], 10.0) && close(e[1], 0.0));
    // Grabbing nothing just solves.
    assert!(drag(&mut s, Grab::Point(PointRef::ORIGIN), [0.0; 2], [1.0, 1.0]).converged);
}

#[test]
fn measuring_gives_what_a_dimension_would_hold() {
    let mut s = sketch();
    let g = line(&mut s, [0.0, 0.0], [3.0, 4.0]);
    assert_eq!(measure(&s, &Constraint::Distance(g, 0.0)), Some(5.0));
    assert_eq!(
        measure(
            &s,
            &Constraint::DistanceX(PointRef::ORIGIN, p(g, Pos::End), 0.0)
        ),
        Some(3.0)
    );
    assert!(close(
        measure(&s, &Constraint::Angle(g, 0.0)).unwrap(),
        4f64.atan2(3.0)
    ));
    assert_eq!(measure(&s, &Constraint::Horizontal(g)), None);
    assert_eq!(measure(&s, &Constraint::Radius(g, 0.0)), None);
}
