//! The Modeller's back end against **real FreeCAD**: the geometry is only right if FreeCAD
//! agrees, so this drives the actual worker script through a SketchUp-style sequence and
//! checks the solid it builds.
//!
//! It needs `freecadcmd` (on `PATH`, or named by `CCOSEL_FREECADCMD`). Where there is none it
//! says so loudly and passes, because CI does not install FreeCAD yet — see the PR that added
//! this. `cad.rs` covers the same server code against a fake worker everywhere.

use std::sync::Arc;
use std::time::{Duration, Instant};

use ccosel_proto::cad::{CadOp, ExportFormat, ExportReq, MeshData, RegenResult};
use ccosel_server::cad_api::{Cad, Launch};
use ccosel_server::fs_api::Jail;

fn square(z: f32, lo: f32, hi: f32) -> CadOp {
    CadOp::Polygon {
        points: vec![[lo, lo, z], [hi, lo, z], [hi, hi, z], [lo, hi, z]],
        normal: [0.0, 0.0, 1.0],
    }
}

#[test]
fn a_box_with_a_pocket_has_the_volume_freecad_says() {
    let temp = std::env::temp_dir().join(format!("ccosel-freecad-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp);
    std::fs::create_dir_all(&temp).unwrap();
    let cad = Arc::new(Cad::from_env(&temp).unwrap());
    if !cad.available() {
        let script = temp.join("worker.py");
        assert!(Launch::freecad(&script).is_none());
        eprintln!(
            "\n*** SKIPPED: FreeCAD is not installed (no freecadcmd on PATH, CCOSEL_FREECADCMD \
             unset). The CAD geometry was NOT checked against FreeCAD. ***\n"
        );
        return;
    }

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
