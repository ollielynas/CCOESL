use super::*;
use crate::mesh::fixtures::{cuboid, square};
use ccosel_proto::cad::CameraState;
use ccosel_proto::scene2d::unpack;

fn box_view() -> Camera {
    // Front-left and above a 100 mm cube: three faces and nine edges visible.
    Camera::new(CameraState {
        target: [50.0, 50.0, 50.0],
        distance: 600.0,
        azimuth: -2.2,
        elevation: 0.5,
        fov_y: 0.6,
    })
}

fn no_axes() -> Style {
    Style {
        axes: false,
        ..Style::default()
    }
}

#[test]
fn a_box_shows_three_faces_and_is_consistent() {
    let m = Mesh::new(cuboid(100.0, 100.0, 100.0)).unwrap();
    let s = render(Some(&m), &box_view(), 400, 300, &no_axes());
    assert!(s.is_consistent());
    assert_eq!(s.tris.len(), 6, "back faces of a solid are culled");
    assert_eq!((s.width, s.height), (400, 300));
    // Everything on screen.
    for i in 0..s.points.len() as u32 {
        let p = s.px(i);
        assert!((0.0..=400.0).contains(&p[0]) && (0.0..=300.0).contains(&p[1]));
    }
}

#[test]
fn shared_corners_share_points() {
    let m = Mesh::new(cuboid(100.0, 100.0, 100.0)).unwrap();
    let s = render(Some(&m), &box_view(), 400, 300, &no_axes());
    let tri_points: std::collections::HashSet<u32> = s.tris.iter().flatten().copied().collect();
    // Three visible faces of a box meet at seven corners.
    assert_eq!(tri_points.len(), 7);
}

#[test]
fn hidden_edges_are_cut_away() {
    let m = Mesh::new(cuboid(100.0, 100.0, 100.0)).unwrap();
    let s = render(Some(&m), &box_view(), 400, 300, &no_axes());
    let total: f32 = s
        .lines
        .iter()
        .map(|[a, b]| {
            let (a, b) = (s.px(*a), s.px(*b));
            ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
        })
        .sum();
    // The same box with nothing to hide its edges: every edge drawn in full.
    let mut wire = cuboid(100.0, 100.0, 100.0);
    wire.triangles.clear();
    wire.tri_face.clear();
    let wire = Mesh::new(wire).unwrap();
    let all = render(Some(&wire), &box_view(), 400, 300, &no_axes());
    let full: f32 = all
        .lines
        .iter()
        .map(|[a, b]| {
            let (a, b) = (all.px(*a), all.px(*b));
            ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
        })
        .sum();
    // Nine of the twelve edges are visible from a corner view; the three meeting at the hidden
    // corner are not.
    assert!(total < full * 0.85, "visible {total} of {full}");
    assert!(total > full * 0.6, "visible {total} of {full}");
}

#[test]
fn faces_are_drawn_far_to_near() {
    let m = Mesh::new(cuboid(100.0, 100.0, 100.0)).unwrap();
    let cam = box_view();
    let s = render(Some(&m), &cam, 400, 300, &no_axes());
    // Recover each triangle's depth by matching its colour back to... simpler: its screen
    // centroid's distance from the nearest corner's projection grows towards the far end.
    let nearest = cam.project([0.0, 0.0, 100.0], 400.0, 300.0).unwrap();
    let first = s.tris.first().unwrap();
    let last = s.tris.last().unwrap();
    let d = |t: &[u32; 3]| {
        let c = t.map(|i| s.px(i));
        let (x, y) = (
            (c[0][0] + c[1][0] + c[2][0]) / 3.0,
            (c[0][1] + c[1][1] + c[2][1]) / 3.0,
        );
        ((x - nearest[0]).powi(2) + (y - nearest[1]).powi(2)).sqrt()
    };
    assert!(
        d(first) >= d(last) * 0.5,
        "drawing order runs toward the viewer"
    );
}

#[test]
fn selection_and_hover_tint_their_face() {
    let m = Mesh::new(square(100.0)).unwrap();
    let cam = Camera::new(CameraState {
        target: [50.0, 50.0, 0.0],
        distance: 400.0,
        elevation: 1.2,
        ..CameraState::default()
    });
    let plain = render(Some(&m), &cam, 200, 200, &no_axes());
    let selected = render(
        Some(&m),
        &cam,
        200,
        200,
        &Style {
            selected: 0,
            ..no_axes()
        },
    );
    let [r, _, b, _] = unpack(selected.tri_colors[0]);
    assert!(b > r, "selected faces are blue");
    assert_ne!(plain.tri_colors[0], selected.tri_colors[0]);
}

#[test]
fn a_loose_face_seen_from_behind_uses_the_back_colour() {
    let m = Mesh::new(square(100.0)).unwrap();
    let above = Camera::new(CameraState {
        target: [50.0, 50.0, 0.0],
        distance: 400.0,
        elevation: 1.0,
        ..CameraState::default()
    });
    let below = Camera::new(CameraState {
        elevation: -1.0,
        ..above.state
    });
    let front = unpack(render(Some(&m), &above, 200, 200, &no_axes()).tri_colors[0]);
    let back = unpack(render(Some(&m), &below, 200, 200, &no_axes()).tri_colors[0]);
    assert!(back[2] > back[0] && front[0] >= front[2]);
}

#[test]
fn a_loose_face_on_a_box_is_drawn_after_the_box_top() {
    let mut d = cuboid(100.0, 100.0, 100.0);
    let base = d.positions.len() as u32;
    // A small loose square in the middle of the top.
    for p in [
        [40.0, 40.0, 100.0],
        [60.0, 40.0, 100.0],
        [60.0, 60.0, 100.0],
        [40.0, 60.0, 100.0],
    ] {
        d.positions.push(p);
    }
    d.triangles.push([base, base + 1, base + 2]);
    d.triangles.push([base, base + 2, base + 3]);
    d.tri_face.extend([6, 6]);
    d.face_normals.push([0.0, 0.0, 1.0]);
    d.loose.push(true);
    let m = Mesh::new(d).unwrap();
    let s = render(
        Some(&m),
        &box_view(),
        400,
        300,
        &Style {
            selected: 6,
            ..no_axes()
        },
    );
    // The loose square is selected, so its triangles are the blue ones. They must come after
    // the two triangles of the top face they sit on, or the top would paint over them.
    let is_blue = |c: u32| {
        let [r, _, b, _] = unpack(c);
        u16::from(b) > u16::from(r) + 30
    };
    let first_blue = s.tri_colors.iter().position(|c| is_blue(*c)).unwrap();
    let top = m.face_tris(1).count();
    let plain_before = s.tri_colors[..first_blue]
        .iter()
        .filter(|c| !is_blue(**c))
        .count();
    assert!(
        plain_before >= top,
        "{plain_before} plain triangles before the loose face"
    );
}

#[test]
fn the_preview_is_translucent_and_does_not_hide_edges() {
    let m = Mesh::new(cuboid(100.0, 100.0, 100.0)).unwrap();
    let base = render(Some(&m), &box_view(), 400, 300, &no_axes());
    let with = render(
        Some(&m),
        &box_view(),
        400,
        300,
        &Style {
            extra: m.extrusion(1, 50.0),
            ..no_axes()
        },
    );
    assert!(with.tris.len() > base.tris.len());
    assert!(with.tri_colors.iter().any(|c| unpack(*c)[3] < 255));
    assert_eq!(with.lines.len(), base.lines.len());
}

#[test]
fn axes_are_drawn_in_their_colours() {
    let s = render(None, &box_view(), 400, 300, &Style::default());
    assert!(s.is_consistent());
    assert!(s.tris.is_empty());
    let colours: std::collections::HashSet<u32> = s.line_colors.iter().copied().collect();
    assert!(colours.contains(&AXIS_X) && colours.contains(&AXIS_Y) && colours.contains(&AXIS_Z));
}

#[test]
fn geometry_behind_the_camera_is_clipped_not_mirrored() {
    let m = Mesh::new(cuboid(100.0, 100.0, 100.0)).unwrap();
    // The eye inside the box, looking out through the +X wall.
    let cam = Camera::new(CameraState {
        target: [90.0, 50.0, 50.0],
        distance: 30.0,
        azimuth: std::f32::consts::PI,
        elevation: 0.0,
        fov_y: 1.0,
    });
    let s = render(Some(&m), &cam, 300, 300, &no_axes());
    assert!(s.is_consistent());
    for i in 0..s.points.len() as u32 {
        assert!(s.px(i).iter().all(|c| c.is_finite()));
    }
}

#[test]
fn clipping_to_the_near_plane_splits_a_triangle() {
    let poly = clip_near(&[[0.0, 0.0, -1.0], [1.0, 0.0, 1.0], [0.0, 1.0, 1.0]], 0.5);
    assert_eq!(poly.len(), 4);
    assert!(poly.iter().all(|p| p[2] >= 0.5 - 1e-6));
    assert!(clip_near(&[[0.0, 0.0, -1.0], [1.0, 0.0, -1.0], [0.0, 1.0, -1.0]], 0.5).is_empty());
}

#[test]
fn segments_are_clipped_to_the_viewport() {
    let (a, b, _, _) = clip_to_rect([-100.0, 50.0], [300.0, 50.0], 1.0, 1.0, 200.0, 100.0).unwrap();
    assert_eq!(a, [0.0, 50.0]);
    assert_eq!(b, [200.0, 50.0]);
    assert!(clip_to_rect([-10.0, -10.0], [-5.0, -20.0], 1.0, 1.0, 100.0, 100.0).is_none());
}
