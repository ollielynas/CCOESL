use ccosel_proto::sketch::{Geo, Plane};

use super::*;

fn close(a: P2, b: P2) -> bool {
    dist(a, b) < 1e-9
}

fn sketch(curves: &[Curve]) -> Sketch {
    let mut s = Sketch::new(Plane::GROUND);
    s.geos = curves.iter().map(|c| Geo::new(*c)).collect();
    s
}

#[test]
fn arc_points_and_normalising() {
    let arc = Curve::Arc {
        center: [0.0, 0.0],
        radius: 2.0,
        start: 0.0,
        end: std::f64::consts::FRAC_PI_2,
    };
    assert!(close(curve_point(&arc, Pos::Start).unwrap(), [2.0, 0.0]));
    assert!(close(curve_point(&arc, Pos::End).unwrap(), [0.0, 2.0]));
    assert_eq!(curve_point(&arc, Pos::Mid), Some([0.0, 0.0]));
    assert_eq!(curve_point(&arc, Pos::Edge), None);
    // An arc given the wrong way round, with a negative radius, comes out right.
    let Curve::Arc {
        radius, start, end, ..
    } = normalize(Curve::Arc {
        center: [0.0, 0.0],
        radius: -1.0,
        start: 7.0,
        end: 6.0,
    })
    else {
        panic!()
    };
    assert!(radius > 0.0 && (0.0..TAU).contains(&start) && end > start && end <= start + TAU);
    assert_eq!(
        normalize(Curve::Circle {
            center: [0.0; 2],
            radius: -3.0
        }),
        Curve::Circle {
            center: [0.0; 2],
            radius: 3.0
        }
    );
}

#[test]
fn axes_and_origin() {
    let s = sketch(&[]);
    assert_eq!(point(&s, PointRef::ORIGIN), Some([0.0, 0.0]));
    assert_eq!(point(&s, PointRef::new(V_AXIS, Pos::End)), None);
    assert_eq!(distance_to(&s, H_AXIS, [3.0, -2.0]), Some(2.0));
    assert_eq!(distance_to(&s, V_AXIS, [3.0, -2.0]), Some(3.0));
    assert_eq!(point(&s, PointRef::new(4, Pos::Start)), None);
}

#[test]
fn picking_prefers_points_then_curves_then_axes() {
    let s = sketch(&[
        Curve::Line([10.0, 10.0], [20.0, 10.0]),
        Curve::Circle {
            center: [50.0, 50.0],
            radius: 5.0,
        },
    ]);
    assert_eq!(
        nearest_point(&s, [19.5, 10.2], 1.0),
        Some(PointRef::new(0, Pos::End))
    );
    assert_eq!(nearest_point(&s, [0.1, 0.1], 1.0), Some(PointRef::ORIGIN));
    assert_eq!(nearest_curve(&s, [15.0, 10.5], 1.0), Some(0));
    assert_eq!(nearest_curve(&s, [55.4, 50.0], 1.0), Some(1));
    assert_eq!(nearest_curve(&s, [30.0, 0.4], 1.0), Some(H_AXIS));
    assert_eq!(nearest_curve(&s, [30.0, 30.0], 1.0), None);
}

#[test]
fn distances_to_arcs_respect_their_extent() {
    let s = sketch(&[Curve::Arc {
        center: [0.0, 0.0],
        radius: 10.0,
        start: 0.0,
        end: std::f64::consts::FRAC_PI_2,
    }]);
    assert!((distance_to(&s, 0, [7.0, 7.0]).unwrap() - (98f64.sqrt() - 10.0).abs()).abs() < 1e-9);
    // Round the back of the arc, the nearest point is an end.
    assert!((distance_to(&s, 0, [-10.0, 0.0]).unwrap() - 200f64.sqrt()).abs() < 1e-9);
    assert!(distance_to(&s, 0, [0.0, 0.0]).is_some());
    let p = sketch(&[Curve::Point([1.0, 1.0])]);
    assert_eq!(distance_to(&p, 0, [1.0, 4.0]), Some(3.0));
}

#[test]
fn intersections_of_every_pair() {
    let l1 = Curve::Line([0.0, 0.0], [10.0, 10.0]);
    let l2 = Curve::Line([0.0, 10.0], [10.0, 0.0]);
    assert_eq!(intersections(&l1, &l2), vec![[5.0, 5.0]]);
    // Segments that would cross only if longer do not.
    let short = Curve::Line([0.0, 10.0], [2.0, 8.0]);
    assert!(intersections(&l1, &short).is_empty());
    let parallel = Curve::Line([1.0, 0.0], [11.0, 10.0]);
    assert!(intersections(&l1, &parallel).is_empty());

    let circle = Curve::Circle {
        center: [5.0, 5.0],
        radius: 5.0,
    };
    let across = Curve::Line([-10.0, 5.0], [20.0, 5.0]);
    let mut hits = intersections(&across, &circle);
    hits.sort_by(|a, b| a[0].total_cmp(&b[0]));
    assert!(close(hits[0], [0.0, 5.0]) && close(hits[1], [10.0, 5.0]));
    let touching = Curve::Line([0.0, 10.0], [10.0, 10.0]);
    assert_eq!(intersections(&touching, &circle).len(), 1);

    let upper = Curve::Arc {
        center: [5.0, 5.0],
        radius: 5.0,
        start: 0.0,
        end: std::f64::consts::PI,
    };
    let low = Curve::Line([-10.0, 2.0], [20.0, 2.0]);
    assert!(intersections(&low, &upper).is_empty(), "below the arc");

    let c2 = Curve::Circle {
        center: [13.0, 5.0],
        radius: 5.0,
    };
    assert_eq!(intersections(&circle, &c2).len(), 2);
    let far = Curve::Circle {
        center: [50.0, 5.0],
        radius: 1.0,
    };
    assert!(intersections(&circle, &far).is_empty());
    assert!(intersections(&Curve::Point([0.0; 2]), &circle).is_empty());
    assert!(within(&Curve::Point([1.0, 2.0]), [1.0, 2.0]));
}

#[test]
fn parameters_along_curves() {
    let l = Curve::Line([0.0, 0.0], [10.0, 0.0]);
    assert_eq!(param(&l, [2.5, 0.0]), 0.25);
    let a = Curve::Arc {
        center: [0.0, 0.0],
        radius: 1.0,
        start: 1.0,
        end: 3.0,
    };
    assert!((param(&a, polar([0.0; 2], 1.0, 2.0)) - 1.0).abs() < 1e-12);
    let c = Curve::Circle {
        center: [0.0; 2],
        radius: 1.0,
    };
    assert!((param(&c, [0.0, -1.0]) - 1.5 * std::f64::consts::PI).abs() < 1e-12);
    assert_eq!(param(&Curve::Point([0.0; 2]), [1.0, 1.0]), 0.0);
    assert_eq!(closest_on_segment([0.0; 2], [0.0; 2], [3.0, 3.0]).1, 0.0);
    assert!(line_circle([0.0; 2], [0.0; 2], [0.0; 2], 1.0).is_empty());
    assert!(circle_circle([0.0; 2], 1.0, [0.0; 2], 1.0).is_empty());
    assert_eq!(circle_circle([0.0; 2], 1.0, [2.0, 0.0], 1.0).len(), 1);
    assert_eq!(wrap(-0.5), TAU - 0.5);
    assert!(on_arc(6.0, 7.0, 0.5));
    assert_eq!(points_of(&c), &[Pos::Mid]);
    assert_eq!(lerp([0.0; 2], [2.0, 2.0], 0.5), [1.0, 1.0]);
}
