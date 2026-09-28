use ccosel_proto::sketch::{Geo, H_AXIS, Plane};

use super::*;

fn sample() -> Sketch {
    let mut s = Sketch::new(Plane::GROUND);
    s.geos = vec![
        Geo::new(Curve::Line([0.0, 0.0], [3.0, 4.0])),
        Geo::new(Curve::Line([0.0, 5.0], [6.0, 5.0])),
        Geo::new(Curve::Circle {
            center: [10.0, 0.0],
            radius: 2.0,
        }),
        Geo::new(Curve::Arc {
            center: [20.0, 0.0],
            radius: 3.0,
            start: 0.0,
            end: 1.0,
        }),
    ];
    s
}

fn pt(g: i32, pos: Pos) -> Item {
    Item::Point(PointRef::new(g, pos))
}

#[test]
fn point_constraints() {
    let s = sample();
    let two = [pt(0, Pos::End), pt(1, Pos::Start)];
    assert_eq!(
        constraints_for(Kind::Coincident, &two, &s).unwrap(),
        vec![Constraint::Coincident(
            PointRef::new(0, Pos::End),
            PointRef::new(1, Pos::Start)
        )]
    );
    assert!(constraints_for(Kind::Coincident, &two[..1], &s).is_err());
    assert_eq!(
        constraints_for(Kind::PointOnObject, &[pt(0, Pos::End), Item::Geo(2)], &s)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        constraints_for(Kind::Horizontal, &two, &s).unwrap()[0],
        Constraint::HorizontalPoints(PointRef::new(0, Pos::End), PointRef::new(1, Pos::Start))
    );
    assert!(matches!(
        constraints_for(Kind::Vertical, &two, &s).unwrap()[0],
        Constraint::VerticalPoints(..)
    ));
    let lock = constraints_for(Kind::Lock, &[pt(0, Pos::End)], &s).unwrap();
    assert_eq!(
        lock,
        vec![
            Constraint::DistanceX(PointRef::ORIGIN, PointRef::new(0, Pos::End), 3.0),
            Constraint::DistanceY(PointRef::ORIGIN, PointRef::new(0, Pos::End), 4.0)
        ]
    );
    assert!(
        constraints_for(
            Kind::Symmetric,
            &[pt(0, Pos::Start), pt(1, Pos::End), pt(0, Pos::End)],
            &s
        )
        .is_ok()
    );
    assert!(
        constraints_for(
            Kind::Symmetric,
            &[pt(0, Pos::Start), pt(1, Pos::End), Item::Geo(H_AXIS)],
            &s
        )
        .is_ok()
    );
}

#[test]
fn line_constraints() {
    let s = sample();
    let lines = [Item::Geo(0), Item::Geo(1)];
    assert_eq!(
        constraints_for(Kind::Horizontal, &lines, &s).unwrap(),
        vec![Constraint::Horizontal(0), Constraint::Horizontal(1)]
    );
    assert_eq!(
        constraints_for(Kind::Vertical, &lines[..1], &s).unwrap(),
        vec![Constraint::Vertical(0)]
    );
    assert!(constraints_for(Kind::Horizontal, &[Item::Geo(2)], &s).is_err());
    assert!(constraints_for(Kind::Parallel, &lines, &s).is_ok());
    assert!(constraints_for(Kind::Perpendicular, &lines, &s).is_ok());
    assert!(constraints_for(Kind::Perpendicular, &lines[..1], &s).is_err());
    assert!(constraints_for(Kind::Equal, &lines, &s).is_ok());
    assert!(constraints_for(Kind::Tangent, &lines, &s).is_ok());
    assert!(constraints_for(Kind::Tangent, &[Item::Geo(0), Item::Geo(2)], &s).is_ok());
    assert!(constraints_for(Kind::Equal, &[Item::Geo(0), Item::Geo(2)], &s).is_err());
}

#[test]
fn dimensions_take_the_current_values() {
    let s = sample();
    assert_eq!(
        constraints_for(Kind::Distance, &[Item::Geo(0)], &s).unwrap(),
        vec![Constraint::Distance(0, 5.0)]
    );
    assert_eq!(
        constraints_for(Kind::Distance, &[pt(0, Pos::Start), pt(1, Pos::End)], &s).unwrap()[0]
            .value(),
        Some(61f64.sqrt())
    );
    assert_eq!(
        constraints_for(Kind::Distance, &[pt(0, Pos::Start), Item::Geo(1)], &s).unwrap()[0].value(),
        Some(5.0)
    );
    assert_eq!(
        constraints_for(Kind::DistanceX, &[Item::Geo(1)], &s).unwrap()[0].value(),
        Some(6.0)
    );
    assert_eq!(
        constraints_for(Kind::DistanceY, &[pt(1, Pos::End)], &s).unwrap()[0].value(),
        Some(5.0)
    );
    assert_eq!(
        constraints_for(Kind::Radius, &[Item::Geo(2), Item::Geo(3)], &s).unwrap(),
        vec![Constraint::Radius(2, 2.0), Constraint::Radius(3, 3.0)]
    );
    assert_eq!(
        constraints_for(Kind::Diameter, &[Item::Geo(2)], &s).unwrap(),
        vec![Constraint::Diameter(2, 4.0)]
    );
    assert!(constraints_for(Kind::Radius, &[Item::Geo(0)], &s).is_err());
    let a = constraints_for(Kind::Angle, &[Item::Geo(0)], &s).unwrap()[0];
    assert!((a.value().unwrap() - 4f64.atan2(3.0)).abs() < 1e-9);
    let between = constraints_for(Kind::Angle, &[Item::Geo(1), Item::Geo(0)], &s).unwrap()[0];
    assert!((between.value().unwrap() - 4f64.atan2(3.0)).abs() < 1e-9);
    assert!(constraints_for(Kind::Angle, &[], &s).is_err());
    assert!(constraints_for(Kind::Distance, &[Item::Geo(2)], &s).is_err());
    assert!(constraints_for(Kind::DistanceX, &[], &s).is_err());
}

#[test]
fn every_kind_explains_itself() {
    for k in Kind::ALL {
        assert!(k.needs().starts_with("Select"));
        assert_eq!(
            constraints_for(k, &[Item::Constraint(0)], &sample()),
            Err(k.needs())
        );
    }
}
