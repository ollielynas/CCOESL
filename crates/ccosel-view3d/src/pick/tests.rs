use super::*;
use crate::mesh::fixtures::{cuboid, square};
use ccosel_proto::cad::CameraState;

const W: f32 = 400.0;
const H: f32 = 300.0;

fn close3(a: V3, b: V3) -> bool {
    (0..3).all(|i| (a[i] - b[i]).abs() < 1e-2)
}

/// Looking down at the ground from high above the point (50, 50, 0).
fn from_above() -> Camera {
    Camera::new(CameraState {
        target: [50.0, 50.0, 0.0],
        distance: 400.0,
        azimuth: -std::f32::consts::FRAC_PI_2,
        elevation: 1.5,
        fov_y: 0.8,
    })
}

/// In front of (on -Y) and a little above a 100 mm cube.
fn front_and_above() -> Camera {
    Camera::new(CameraState {
        target: [50.0, 50.0, 50.0],
        distance: 600.0,
        azimuth: -std::f32::consts::FRAC_PI_2,
        elevation: 0.3,
        fov_y: 0.8,
    })
}

fn screen(cam: &Camera, p: V3) -> [f32; 2] {
    let s = cam.project(p, W, H).unwrap();
    [s[0], s[1]]
}

/// `d` plus one loose triangle, as face 6.
fn with_loose_triangle(mut d: ccosel_proto::cad::MeshData, pts: [V3; 3]) -> Mesh {
    let base = d.positions.len() as u32;
    d.positions.extend(pts);
    d.vertices.extend(pts);
    d.triangles.push([base, base + 1, base + 2]);
    d.tri_face.push(6);
    d.face_normals.push([0.0, 0.0, 1.0]);
    d.loose.push(true);
    Mesh::new(d).unwrap()
}

#[test]
fn the_nearest_face_is_hit() {
    let m = Mesh::new(cuboid(100.0, 100.0, 100.0)).unwrap();
    let ray = Ray {
        origin: [50.0, 50.0, 500.0],
        dir: [0.0, 0.0, -1.0],
    };
    let hit = raycast(&m, &ray).unwrap();
    assert_eq!(hit.face, 1, "the top, not the bottom behind it");
    assert!(close3(hit.point, [50.0, 50.0, 100.0]));
    assert_eq!(hit.normal, [0.0, 0.0, 1.0]);
}

#[test]
fn a_ray_that_misses_hits_nothing() {
    let m = Mesh::new(cuboid(1.0, 1.0, 1.0)).unwrap();
    let ray = Ray {
        origin: [5.0, 5.0, 5.0],
        dir: [0.0, 0.0, 1.0],
    };
    assert_eq!(raycast(&m, &ray), None);
}

#[test]
fn a_loose_face_on_a_solid_wins_the_tie() {
    let m = with_loose_triangle(
        cuboid(100.0, 100.0, 100.0),
        [
            [20.0, 20.0, 100.0],
            [60.0, 20.0, 100.0],
            [60.0, 60.0, 100.0],
        ],
    );
    let ray = Ray {
        origin: [50.0, 30.0, 500.0],
        dir: [0.0, 0.0, -1.0],
    };
    assert_eq!(raycast(&m, &ray).unwrap().face, 6);
}

#[test]
fn empty_space_lands_on_the_ground() {
    let cam = from_above();
    let r = resolve(None, &cam, W, H, [W / 2.0, H / 2.0], None).unwrap();
    assert_eq!(r.snap, Snap::Free);
    assert_eq!(r.face, NO_FACE);
    assert!(close3(r.point, [50.0, 50.0, 0.0]));
    assert_eq!(r.normal, [0.0, 0.0, 1.0]);
}

#[test]
fn looking_at_the_sky_lands_nowhere() {
    let cam = Camera::new(CameraState {
        elevation: 0.0,
        ..CameraState::default()
    });
    assert_eq!(resolve(None, &cam, W, H, [W / 2.0, 0.0], None), None);
}

#[test]
fn the_pointer_snaps_to_a_nearby_corner() {
    let m = Mesh::new(square(100.0)).unwrap();
    let cam = from_above();
    let s = screen(&cam, [100.0, 0.0, 0.0]);
    let r = resolve(Some(&m), &cam, W, H, [s[0] + 4.0, s[1] - 3.0], None).unwrap();
    assert_eq!(r.snap, Snap::Endpoint);
    assert_eq!(r.point, [100.0, 0.0, 0.0]);
}

#[test]
fn midpoints_are_snapped_to() {
    let m = Mesh::new(cuboid(100.0, 100.0, 100.0)).unwrap();
    let cam = from_above();
    let s = screen(&cam, [50.0, 0.0, 100.0]);
    let r = resolve(Some(&m), &cam, W, H, s, None).unwrap();
    assert_eq!(r.snap, Snap::Midpoint);
    assert_eq!(r.point, [50.0, 0.0, 100.0]);
}

#[test]
fn corners_hidden_behind_a_face_are_not_snapped_to() {
    let m = Mesh::new(cuboid(100.0, 100.0, 100.0)).unwrap();
    let cam = front_and_above();
    // The bottom back corner is behind the box from here.
    let hidden = [0.0, 100.0, 0.0];
    let r = resolve(Some(&m), &cam, W, H, screen(&cam, hidden), None).unwrap();
    assert_ne!(r.point, hidden);
    assert_ne!(r.face, NO_FACE, "the pointer is over the box");
}

#[test]
fn a_face_under_the_pointer_gives_its_plane() {
    let m = Mesh::new(cuboid(100.0, 100.0, 100.0)).unwrap();
    let cam = from_above();
    let s = screen(&cam, [30.0, 70.0, 100.0]);
    let r = resolve(Some(&m), &cam, W, H, s, None).unwrap();
    assert_eq!(r.snap, Snap::OnFace);
    assert_eq!(r.face, 1);
    assert_eq!(r.normal, [0.0, 0.0, 1.0]);
}

#[test]
fn a_loose_face_seen_from_below_faces_the_viewer() {
    let m = Mesh::new(square(100.0)).unwrap();
    let cam = Camera::new(CameraState {
        target: [50.0, 50.0, 0.0],
        distance: 400.0,
        azimuth: 0.0,
        elevation: -1.2,
        fov_y: 0.8,
    });
    let r = resolve(Some(&m), &cam, W, H, screen(&cam, [30.0, 60.0, 0.0]), None).unwrap();
    assert_eq!(r.snap, Snap::OnFace);
    assert_eq!(r.normal, [0.0, 0.0, -1.0]);
}

#[test]
fn with_an_anchor_points_stay_on_its_plane_and_follow_the_axes() {
    let cam = from_above();
    let anchor = Anchor {
        point: [0.0, 0.0, 0.0],
        normal: [0.0, 0.0, 1.0],
    };
    // Near the red axis: snapped onto it.
    let on_x = screen(&cam, [70.0, 0.0, 0.0]);
    let r = resolve(None, &cam, W, H, [on_x[0], on_x[1] + 4.0], Some(anchor)).unwrap();
    assert_eq!(r.snap, Snap::AxisX);
    // On the axis exactly, and near where the pointer is along it.
    assert_eq!((r.point[1], r.point[2]), (0.0, 0.0));
    assert!((r.point[0] - 70.0).abs() < 2.0, "{:?}", r.point);

    let on_y = screen(&cam, [0.0, 80.0, 0.0]);
    let r = resolve(None, &cam, W, H, [on_y[0] + 3.0, on_y[1]], Some(anchor)).unwrap();
    assert_eq!(r.snap, Snap::AxisY);

    // Well off both axes: free, but still on the plane.
    let off = screen(&cam, [60.0, 70.0, 0.0]);
    let r = resolve(None, &cam, W, H, off, Some(anchor)).unwrap();
    assert_eq!(r.snap, Snap::Free);
    assert!(r.point[2].abs() < 1e-3);
}

#[test]
fn a_snapped_corner_off_the_plane_is_projected_onto_it() {
    let m = Mesh::new(cuboid(100.0, 100.0, 100.0)).unwrap();
    let cam = from_above();
    let anchor = Anchor {
        point: [0.0, 0.0, 0.0],
        normal: [0.0, 0.0, 1.0],
    };
    let corner = screen(&cam, [100.0, 100.0, 100.0]);
    let r = resolve(Some(&m), &cam, W, H, corner, Some(anchor)).unwrap();
    assert_eq!(r.snap, Snap::Endpoint);
    assert!(close3(r.point, [100.0, 100.0, 0.0]));
}

#[test]
fn push_pull_follows_the_pointer_along_the_normal() {
    let m = Mesh::new(square(100.0)).unwrap();
    // From the side, so up the screen is up the normal.
    let cam = Camera::new(CameraState {
        target: [50.0, 50.0, 50.0],
        distance: 600.0,
        azimuth: -std::f32::consts::FRAC_PI_2,
        elevation: 0.0,
        fov_y: 0.8,
    });
    let hit = Hit {
        face: 0,
        point: [50.0, 50.0, 0.0],
        normal: [0.0, 0.0, 1.0],
        t: 1.0,
    };
    let mut drag = PushPull::start(&m, &hit).unwrap();
    drag.update(&m, &cam, W, H, screen(&cam, [50.0, 50.0, 80.0]));
    assert!((drag.distance - 80.0).abs() < 0.5, "{}", drag.distance);
    assert!(!drag.snapped);
}

#[test]
fn push_pull_snaps_level_with_a_corner() {
    let m = with_loose_triangle(
        cuboid(100.0, 100.0, 100.0),
        [[200.0, 0.0, 0.0], [300.0, 0.0, 0.0], [300.0, 100.0, 0.0]],
    );
    let cam = Camera::new(CameraState {
        target: [150.0, 50.0, 50.0],
        distance: 900.0,
        azimuth: -1.2,
        elevation: 0.3,
        fov_y: 0.8,
    });
    let hit = Hit {
        face: 6,
        point: [250.0, 30.0, 0.0],
        normal: [0.0, 0.0, 1.0],
        t: 1.0,
    };
    let mut drag = PushPull::start(&m, &hit).unwrap();
    drag.update(&m, &cam, W, H, screen(&cam, [100.0, 0.0, 100.0]));
    assert!(drag.snapped);
    assert!((drag.distance - 100.0).abs() < 1e-3);
}

#[test]
fn curved_faces_cannot_be_pushed() {
    let mut d = square(1.0);
    d.face_normals[0] = [0.0; 3];
    let m = Mesh::new(d).unwrap();
    let hit = Hit {
        face: 0,
        point: [0.5, 0.5, 0.0],
        normal: [0.0, 0.0, 1.0],
        t: 1.0,
    };
    assert_eq!(PushPull::start(&m, &hit), None);
}
