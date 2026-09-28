use ccosel_proto::sketch::{Plane, V_AXIS};

use super::*;
use crate::geom::point;
use crate::solve::solve;

fn sketch() -> Sketch {
    Sketch::new(Plane::GROUND)
}

fn close(a: P2, b: P2) -> bool {
    dist(a, b) < 1e-6
}

fn at(s: &Sketch, g: i32, pos: Pos) -> P2 {
    point(s, PointRef::new(g, pos)).unwrap()
}

#[test]
fn a_rectangle_is_four_joined_lines_level_and_plumb() {
    let mut s = sketch();
    let ids = add_rectangle(&mut s, [0.0, 0.0], [10.0, 5.0], false);
    assert_eq!(ids, [0, 1, 2, 3]);
    assert_eq!(s.constraints.len(), 8);
    let r = solve(&mut s);
    assert!(r.converged);
    // Four free numbers: where it is, and how wide and tall.
    assert_eq!(r.dof, 4);
}

#[test]
fn drawing_attaches_and_levels() {
    let mut s = sketch();
    let g = add_line(&mut s, [0.0, 0.0], [10.0, 0.1], false);
    assert!(auto_level(&mut s, g));
    assert_eq!(s.constraints, vec![Constraint::Horizontal(0)]);
    let v = add_line(&mut s, [0.0, 0.0], [0.1, 10.0], false);
    assert!(auto_level(&mut s, v));
    assert_eq!(s.constraints[1], Constraint::Vertical(1));
    let d = add_line(&mut s, [0.0, 0.0], [5.0, 5.0], true);
    assert!(!auto_level(&mut s, d));
    assert!(s.geos[2].construction);
    attach(
        &mut s,
        PointRef::new(d, Pos::Start),
        Attach::Point(PointRef::new(g, Pos::Start)),
    );
    attach(&mut s, PointRef::new(d, Pos::End), Attach::Curve(V_AXIS));
    attach(&mut s, PointRef::new(d, Pos::End), Attach::Nothing);
    attach(&mut s, PointRef::new(d, Pos::End), Attach::Curve(d));
    assert_eq!(s.constraints.len(), 4);
    let c = add_circle(&mut s, [0.0; 2], -3.0, false);
    assert!(matches!(s.geos[c as usize].curve, Curve::Circle { radius, .. } if radius == 3.0));
    let pt = add_point(&mut s, [1.0, 2.0]);
    assert_eq!(at(&s, pt, Pos::Start), [1.0, 2.0]);
    let e = add_external(&mut s, [0.0; 2], [1.0, 0.0]);
    assert!(s.geos[e as usize].fixed && s.geos[e as usize].construction);
    assert!(!auto_level(&mut s, c));
}

#[test]
fn arcs_run_counter_clockwise_from_the_first_point() {
    let mut s = sketch();
    let a = add_arc(&mut s, [0.0; 2], [5.0, 0.0], [0.0, 9.0], false);
    assert!(close(at(&s, a, Pos::Start), [5.0, 0.0]));
    assert!(close(at(&s, a, Pos::End), [0.0, 5.0]));
    let full = add_arc(&mut s, [0.0; 2], [5.0, 0.0], [5.0, 0.0], false);
    assert!(
        matches!(s.geos[full as usize].curve, Curve::Arc { start, end, .. } if (end - start - 2.0 * PI).abs() < 1e-9)
    );
}

#[test]
fn deleting_geometry_renumbers_and_drops_its_constraints() {
    let mut s = sketch();
    add_rectangle(&mut s, [0.0, 0.0], [10.0, 5.0], false);
    delete(&mut s, &[Item::Geo(1)]);
    assert_eq!(s.geos.len(), 3);
    assert!(s.is_valid());
    // Coincidences at both ends of line 1, and its Vertical, went with it.
    assert_eq!(s.constraints.len(), 5);
    delete(&mut s, &[Item::Constraint(0)]);
    assert_eq!(s.constraints.len(), 4);
    delete(&mut s, &[Item::Point(PointRef::new(2, Pos::End))]);
    assert!(
        s.constraints
            .iter()
            .all(|c| !c.points().contains(&PointRef::new(2, Pos::End)))
    );
    delete(&mut s, &[Item::Geo(99), Item::Geo(-1)]);
    assert_eq!(s.geos.len(), 3);
}

#[test]
fn construction_toggles_but_not_on_external_geometry() {
    let mut s = sketch();
    add_line(&mut s, [0.0; 2], [1.0, 0.0], false);
    add_external(&mut s, [0.0; 2], [0.0, 1.0]);
    toggle_construction(&mut s, &[0, 1, 5]);
    assert!(s.geos[0].construction);
    assert!(s.geos[1].construction, "external stays construction");
}

/// A horizontal line from x = 0 to 30 crossed by vertical lines at 10 and 20.
fn crossed() -> Sketch {
    let mut s = sketch();
    let h = add_line(&mut s, [0.0, 0.0], [30.0, 0.0], false);
    s.constraints.push(Constraint::Horizontal(h));
    s.constraints.push(Constraint::Distance(h, 30.0));
    add_line(&mut s, [10.0, -5.0], [10.0, 5.0], false);
    add_line(&mut s, [20.0, -5.0], [20.0, 5.0], false);
    s
}

#[test]
fn trimming_the_middle_leaves_two_pieces_attached_where_they_were_cut() {
    let mut s = crossed();
    trim(&mut s, 0, [15.0, 0.0]).unwrap();
    assert_eq!(s.geos.len(), 4);
    assert!(close(at(&s, 0, Pos::End), [10.0, 0.0]));
    assert!(close(at(&s, 3, Pos::Start), [20.0, 0.0]));
    assert!(close(at(&s, 3, Pos::End), [30.0, 0.0]));
    assert!(
        s.constraints
            .contains(&Constraint::PointOnObject(PointRef::new(0, Pos::End), 1))
    );
    assert!(
        s.constraints
            .contains(&Constraint::PointOnObject(PointRef::new(3, Pos::Start), 2))
    );
    assert!(
        !s.constraints
            .iter()
            .any(|c| matches!(c, Constraint::Distance(0, _))),
        "its length changed"
    );
    assert!(s.constraints.contains(&Constraint::Horizontal(0)));
    assert!(solve(&mut s).converged);
}

#[test]
fn trimming_an_end_or_an_uncrossed_line() {
    let mut s = crossed();
    trim(&mut s, 0, [25.0, 0.0]).unwrap();
    assert!(close(at(&s, 0, Pos::End), [20.0, 0.0]));
    trim(&mut s, 0, [2.0, 0.0]).unwrap();
    assert!(close(at(&s, 0, Pos::Start), [10.0, 0.0]));
    let mut s = sketch();
    add_line(&mut s, [0.0; 2], [1.0, 0.0], false);
    trim(&mut s, 0, [0.5, 0.0]).unwrap();
    assert!(s.geos.is_empty());
    assert!(trim(&mut s, 3, [0.0; 2]).is_err());
}

#[test]
fn trimming_a_circle_makes_an_arc_and_a_lonely_circle_goes() {
    let mut s = sketch();
    add_circle(&mut s, [0.0, 0.0], 10.0, false);
    add_line(&mut s, [-20.0, 0.0], [20.0, 0.0], false);
    trim(&mut s, 0, [0.0, 10.0]).unwrap();
    let Curve::Arc { start, end, .. } = s.geos[0].curve else {
        panic!("an arc")
    };
    assert!(
        (start - PI).abs() < 1e-9 && (end - 2.0 * PI).abs() < 1e-9,
        "the lower half"
    );
    let mut s = sketch();
    add_circle(&mut s, [0.0, 0.0], 10.0, false);
    trim(&mut s, 0, [10.0, 0.0]).unwrap();
    assert!(s.geos.is_empty());
    add_point(&mut s, [0.0; 2]);
    assert!(trim(&mut s, 0, [0.0; 2]).is_err());
}

#[test]
fn trimming_an_arc() {
    let mut s = sketch();
    add_arc(&mut s, [0.0; 2], [10.0, 0.0], [-10.0, 0.0], false);
    add_line(&mut s, [0.0, -20.0], [0.0, 20.0], false);
    trim(&mut s, 0, polar([0.0; 2], 10.0, 0.3)).unwrap();
    assert!(close(at(&s, 0, Pos::Start), [0.0, 10.0]));
}

#[test]
fn splitting_keeps_the_pieces_joined_and_in_line() {
    let mut s = crossed();
    split(&mut s, 0, [12.0, 0.3]).unwrap();
    assert_eq!(s.geos.len(), 4);
    assert!(close(at(&s, 0, Pos::End), [12.0, 0.0]));
    assert!(close(at(&s, 3, Pos::Start), [12.0, 0.0]));
    assert!(s.constraints.contains(&Constraint::Tangent(0, 3)));
    assert!(s.constraints.contains(&Constraint::Coincident(
        PointRef::new(0, Pos::End),
        PointRef::new(3, Pos::Start)
    )));
    assert!(solve(&mut s).converged);
    assert!(split(&mut s, 0, [0.0, 0.0]).is_err(), "not at an end");
    let mut s = sketch();
    add_circle(&mut s, [0.0; 2], 1.0, false);
    assert!(split(&mut s, 0, [1.0, 0.0]).is_err());
    add_arc(&mut s, [0.0; 2], [5.0, 0.0], [0.0, 5.0], false);
    split(&mut s, 1, polar([0.0; 2], 5.0, 0.7)).unwrap();
    assert!(s.constraints.contains(&Constraint::Equal(1, 2)));
    assert!(solve(&mut s).converged);
}

#[test]
fn extending_a_line_to_the_next_curve() {
    let mut s = crossed();
    s.geos[0].curve = Curve::Line([0.0, 0.0], [5.0, 0.0]);
    s.constraints.clear();
    extend(&mut s, 0, [4.0, 0.0]).unwrap();
    assert!(close(at(&s, 0, Pos::End), [10.0, 0.0]));
    assert!(
        s.constraints
            .contains(&Constraint::PointOnObject(PointRef::new(0, Pos::End), 1))
    );
    // From the start: nothing lies that way.
    assert!(extend(&mut s, 0, [0.5, 0.0]).is_err());
    add_circle(&mut s, [50.0, 0.0], 5.0, false);
    s.geos[0].curve = Curve::Line([30.0, 0.0], [40.0, 0.0]);
    extend(&mut s, 0, [39.0, 0.0]).unwrap();
    assert!(close(at(&s, 0, Pos::End), [45.0, 0.0]));
}

#[test]
fn extending_an_arc_round_to_a_line() {
    let mut s = sketch();
    add_arc(
        &mut s,
        [0.0; 2],
        [10.0, 0.0],
        polar([0.0; 2], 10.0, 1.0),
        false,
    );
    add_line(&mut s, [-20.0, 0.0], [0.0, 0.0], false);
    extend(&mut s, 0, polar([0.0; 2], 10.0, 0.9)).unwrap();
    assert!(close(at(&s, 0, Pos::End), [-10.0, 0.0]));
    assert!(extend(&mut s, 1, [0.0, 0.0]).is_ok() || s.geos.len() == 2);
    add_circle(&mut s, [0.0; 2], 1.0, false);
    assert!(extend(&mut s, 2, [1.0, 0.0]).is_err());
}

#[test]
fn a_fillet_rounds_a_corner_with_a_tangent_arc() {
    let mut s = sketch();
    let [bottom, right, ..] = add_rectangle(&mut s, [0.0, 0.0], [20.0, 10.0], false);
    let arc = fillet(&mut s, [20.0, 0.0], 3.0, 0.5).unwrap();
    let Curve::Arc { center, radius, .. } = s.geos[arc as usize].curve else {
        panic!()
    };
    assert!(close(center, [17.0, 3.0]) && (radius - 3.0).abs() < 1e-9);
    assert!(close(at(&s, bottom, Pos::End), [17.0, 0.0]));
    assert!(close(at(&s, right, Pos::Start), [20.0, 3.0]));
    assert!(s.constraints.contains(&Constraint::Tangent(bottom, arc)));
    let r = solve(&mut s);
    assert!(r.converged, "{r:?}");
    assert!(
        close(at(&s, bottom, Pos::End), [17.0, 0.0]),
        "already solved"
    );

    assert!(fillet(&mut s, [0.0, 0.0], 50.0, 0.5).is_err(), "too big");
    assert!(fillet(&mut s, [5.0, 5.0], 1.0, 0.5).is_err(), "no corner");
    assert!(fillet(&mut s, [0.0, 0.0], -1.0, 0.5).is_err());
    let mut s = sketch();
    add_line(&mut s, [0.0; 2], [5.0, 0.0], false);
    add_line(&mut s, [5.0, 0.0], [10.0, 0.0], false);
    assert!(fillet(&mut s, [5.0, 0.0], 1.0, 0.5).is_err(), "straight on");
}

#[test]
fn offsets_to_the_left_and_outward() {
    let mut s = sketch();
    add_line(&mut s, [0.0, 0.0], [10.0, 0.0], false);
    add_circle(&mut s, [0.0; 2], 5.0, false);
    add_arc(&mut s, [0.0; 2], [3.0, 0.0], [0.0, 3.0], true);
    add_line(&mut s, [1.0, 1.0], [1.0, 1.0], false);
    let ids = offset(&mut s, &[0, 1, 2, 3, 9], 2.0);
    assert_eq!(ids, vec![4, 5, 6]);
    assert!(close(at(&s, 4, Pos::Start), [0.0, 2.0]));
    assert!(matches!(s.geos[5].curve, Curve::Circle { radius, .. } if radius == 7.0));
    assert!(!s.geos[6].construction);
    assert!(
        offset(&mut s, &[1], -9.0).is_empty(),
        "a circle cannot go below nothing"
    );
}

#[test]
fn closest_points_on_curves() {
    let mut s = sketch();
    add_line(&mut s, [0.0, 0.0], [10.0, 0.0], false);
    add_circle(&mut s, [0.0; 2], 5.0, false);
    add_point(&mut s, [3.0, 3.0]);
    assert!(close(closest_on(&s, 0, [4.0, 3.0]).unwrap(), [4.0, 0.0]));
    assert!(close(closest_on(&s, 1, [0.0, 9.0]).unwrap(), [0.0, 5.0]));
    assert_eq!(closest_on(&s, 1, [0.0, 0.0]), None);
    assert_eq!(closest_on(&s, 2, [0.0, 0.0]), Some([3.0, 3.0]));
}
