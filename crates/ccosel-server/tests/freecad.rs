//! The Modeller's back end against **real FreeCAD**: the geometry is only right if FreeCAD
//! agrees, so this drives the actual worker script through a SketchUp-style sequence and
//! checks the solid it builds.
//!
//! It needs `freecadcmd` (on `PATH`, or named by `CCOSEL_FREECADCMD`). Where there is none it
//! says so loudly and passes, because CI does not install FreeCAD yet — see the PR that added
//! this. `cad.rs` covers the same server code against a fake worker everywhere.

use std::sync::Arc;
use std::time::{Duration, Instant};

use ccosel_proto::cad::{CadOp, ExportFormat, ExportReq, MeshData, Model, RegenResult};
use ccosel_proto::sketch::{Constraint, Curve, Geo, Plane, PointRef, Pos, Sketch};
use ccosel_server::cad_api::{Cad, Launch};
use ccosel_server::fs_api::Jail;

fn square(z: f32, lo: f32, hi: f32) -> CadOp {
    CadOp::Polygon {
        points: vec![[lo, lo, z], [hi, lo, z], [hi, hi, z], [lo, hi, z]],
        normal: [0.0, 0.0, 1.0],
    }
}

/// The CAD service on a FreeCAD already here, or `None` (loudly) where there is none. It
/// never sets off the 820 MB auto-install.
fn freecad(name: &str) -> Option<(Arc<Cad>, std::path::PathBuf)> {
    let temp = std::env::temp_dir().join(format!("ccosel-freecad-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp);
    std::fs::create_dir_all(&temp).unwrap();
    let cad = Arc::new(Cad::with_installer(None, &temp).unwrap());
    if !cad.available() {
        assert!(Launch::freecad(&temp.join("worker.py")).is_none());
        eprintln!(
            "\n*** SKIPPED: FreeCAD is not installed (no freecadcmd on PATH, CCOSEL_FREECADCMD \
             unset). The CAD geometry was NOT checked against FreeCAD. ***\n"
        );
        return None;
    }
    Some((cad, temp))
}

fn build(cad: &Arc<Cad>, ops: &[CadOp]) -> RegenResult {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        if let Some(result) = cad.regenerate(ops).unwrap().result {
            return result;
        }
        assert!(Instant::now() < deadline, "FreeCAD never finished");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn built(r: RegenResult) -> Model {
    match r {
        RegenResult::Ok(m) => m,
        RegenResult::Failed { op, message } => panic!("op {op}: {message}"),
    }
}

/// A 100 × 60 rectangle from the origin, fully constrained, with a circle of radius 10 at
/// (50, 30): a plate with a hole, as a FreeCAD sketch.
fn plate_sketch() -> Sketch {
    let mut sk = Sketch::new(Plane::GROUND);
    let ids = ccosel_sketch_like_rectangle(&mut sk, [0.0, 0.0], [90.0, 50.0]);
    sk.geos.push(Geo::new(Curve::Circle {
        center: [45.0, 25.0],
        radius: 8.0,
    }));
    sk.constraints.extend([
        Constraint::Coincident(PointRef::new(ids[0], Pos::Start), PointRef::ORIGIN),
        Constraint::Distance(ids[0], 100.0),
        Constraint::Distance(ids[1], 60.0),
        Constraint::Radius(4, 10.0),
        Constraint::DistanceX(PointRef::ORIGIN, PointRef::new(4, Pos::Mid), 50.0),
        Constraint::DistanceY(PointRef::ORIGIN, PointRef::new(4, Pos::Mid), 30.0),
    ]);
    sk
}

/// The same rectangle FreeCAD's tool makes: four joined lines, two level, two plumb.
fn ccosel_sketch_like_rectangle(sk: &mut Sketch, a: [f64; 2], c: [f64; 2]) -> [i32; 4] {
    let corners = [a, [c[0], a[1]], c, [a[0], c[1]]];
    for i in 0..4 {
        sk.geos
            .push(Geo::new(Curve::Line(corners[i], corners[(i + 1) % 4])));
    }
    for i in 0..4i32 {
        sk.constraints.push(Constraint::Coincident(
            PointRef::new(i, Pos::End),
            PointRef::new((i + 1) % 4, Pos::Start),
        ));
    }
    sk.constraints.extend([
        Constraint::Horizontal(0),
        Constraint::Horizontal(2),
        Constraint::Vertical(1),
        Constraint::Vertical(3),
    ]);
    [0, 1, 2, 3]
}

#[test]
fn a_sketched_plate_with_a_hole_pads_to_the_volume_freecad_says() {
    let Some((cad, temp)) = freecad("sketch") else {
        return;
    };
    // Not solved here: FreeCAD's solver puts it right.
    let mut ops = vec![CadOp::Sketch(plate_sketch())];
    let sketched = built(build(&cad, &ops));
    assert_eq!(
        sketched.new_faces.len(),
        1,
        "one profile: the plate, with the hole in it"
    );
    // Pad it 5 mm, as the Modeller's Pad does.
    ops.push(CadOp::PushPull {
        after: 1,
        face: sketched.new_faces[0],
        distance: 5.0,
    });
    let padded = built(build(&cad, &ops));
    assert_eq!(padded.solids, 1);
    let expected = (100.0 * 60.0 - std::f64::consts::PI * 100.0) * 5.0;
    assert!(
        (padded.volume_mm3 as f64 - expected).abs() < 2.0,
        "{} vs {expected}",
        padded.volume_mm3
    );

    // Saved as a FreeCAD document, the sketch is there to edit.
    let jail_dir = temp.join("jail");
    std::fs::create_dir_all(&jail_dir).unwrap();
    let jail = Jail::new(&jail_dir).unwrap();
    cad.export(
        &jail,
        None,
        &ExportReq {
            ops: ops.clone(),
            format: ExportFormat::FreeCad,
            path: "/plate.FCStd".to_owned(),
        },
    )
    .unwrap();
    let doc = std::fs::read(jail_dir.join("plate.FCStd")).unwrap();
    assert!(doc.starts_with(b"PK"), "an FCStd is a zip");

    // A sketch FreeCAD cannot solve is the step that failed, with FreeCAD's reason.
    let mut bad = plate_sketch();
    bad.constraints.push(Constraint::Vertical(0));
    match build(&cad, &[CadOp::Sketch(bad)]) {
        RegenResult::Failed { op, message } => {
            assert_eq!(op, 0);
            assert!(
                message.contains("conflict") || message.contains("redundant"),
                "{message}"
            );
        }
        RegenResult::Ok(_) => panic!("a horizontal line cannot also be vertical"),
    }
}

#[test]
fn a_box_with_a_pocket_has_the_volume_freecad_says() {
    let Some((cad, temp)) = freecad("pocket") else {
        return;
    };

    // Draw a 100 mm square, pull it into a cube, draw a 40 mm square on top, push it 30 mm in.
    let mut ops = vec![square(0.0, 0.0, 100.0)];
    ops.push(CadOp::PushPull {
        after: 1,
        face: 0,
        distance: 100.0,
    });
    ops.push(square(100.0, 30.0, 70.0));
    // The loose square is the model's last face after three operations.
    ops.push(CadOp::PushPull {
        after: 3,
        face: 6,
        distance: -30.0,
    });

    let deadline = Instant::now() + Duration::from_secs(120);
    let model = loop {
        let status = cad.regenerate(&ops).unwrap();
        if let Some(result) = status.result {
            match result {
                RegenResult::Ok(m) => break m,
                RegenResult::Failed { op, message } => panic!("op {op}: {message}"),
            }
        }
        assert!(Instant::now() < deadline, "FreeCAD never finished");
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(model.solids, 1);
    // A 100 mm cube less a 40 × 40 × 30 pocket.
    assert_eq!(model.volume_mm3, 1_000_000 - 48_000);
    // Six sides, the pocket's floor and four walls.
    assert_eq!(model.faces, 11);

    let hash = model.mesh.rsplit('/').next().unwrap();
    let mesh: MeshData = postcard::from_bytes(&cad.mesh_bytes(hash).unwrap()).unwrap();
    assert!(mesh.is_consistent());
    assert_eq!(mesh.face_normals.len(), 11);
    assert!(
        mesh.loose.iter().all(|l| !l),
        "the square became part of the solid"
    );

    let jail_dir = temp.join("jail");
    std::fs::create_dir_all(&jail_dir).unwrap();
    let jail = Jail::new(&jail_dir).unwrap();
    let done = cad
        .export(
            &jail,
            None,
            &ExportReq {
                ops,
                format: ExportFormat::Step,
                path: "/pocket.step".to_owned(),
            },
        )
        .unwrap();
    let step = std::fs::read_to_string(jail_dir.join("pocket.step")).unwrap();
    assert!(step.starts_with("ISO-10303-21"));
    assert_eq!(done.bytes, step.len() as u64);
}
