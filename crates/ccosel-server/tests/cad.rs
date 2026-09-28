//! The server's side of CAD, against a fake worker that speaks the real protocol: jobs, the
//! mesh store, server rendering, export, and surviving a worker that dies. `freecad.rs` runs
//! the same flow against real FreeCAD.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use ccosel_proto::cad::{
    CadOp, CameraState, ExportFormat, ExportReq, MAX_OPS, MeshData, Model, RegenResult, RenderReq,
};
use ccosel_proto::server_error;
use ccosel_server::cad_api::{Cad, Launch};
use ccosel_server::fs_api::Jail;

fn temp(name: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "ccosel-cad-{}-{name}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn fake(name: &str) -> Arc<Cad> {
    let script =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_cad_worker.py");
    let launch = Launch {
        program: PathBuf::from("python3"),
        args: vec![script.display().to_string()],
    };
    Arc::new(Cad::new(Some(launch), &temp(name)))
}

fn square() -> CadOp {
    CadOp::Polygon {
        points: vec![
            [0.0; 3],
            [10.0, 0.0, 0.0],
            [10.0, 10.0, 0.0],
            [0.0, 10.0, 0.0],
        ],
        normal: [0.0, 0.0, 1.0],
    }
}

fn pull(face: u32) -> CadOp {
    CadOp::PushPull {
        after: 1,
        face,
        distance: 5.0,
    }
}

/// Poll as the app does until the job finishes.
fn finish(cad: &Arc<Cad>, ops: &[CadOp]) -> RegenResult {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let status = cad.regenerate(ops).expect("accepted");
        if let Some(result) = status.result {
            assert!(status.finished);
            return result;
        }
        assert!(Instant::now() < deadline, "the rebuild never finished");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn ok(r: RegenResult) -> Model {
    match r {
        RegenResult::Ok(m) => m,
        RegenResult::Failed { op, message } => panic!("op {op} failed: {message}"),
    }
}

#[test]
fn a_rebuild_stores_its_mesh_and_says_where() {
    let cad = fake("stores");
    let model = ok(finish(&cad, &[square()]));
    assert!(model.mesh.starts_with("/cad/mesh/"));
    assert_eq!((model.faces, model.solids, model.triangles), (1, 0, 2));

    let hash = model.mesh.rsplit('/').next().unwrap();
    let bytes = cad.mesh_bytes(hash).expect("stored");
    let mesh: MeshData = postcard::from_bytes(&bytes).unwrap();
    assert!(mesh.is_consistent());
    assert_eq!(mesh.triangles.len(), 2);
    assert_eq!(
        blake3::hash(&bytes).to_hex().as_str(),
        hash,
        "content-addressed"
    );
    assert!(cad.mesh_bytes("0000").is_none());
}

#[test]
fn polling_the_same_ops_never_rebuilds_twice() {
    let cad = fake("polling");
    let first = ok(finish(&cad, &[square()]));
    for _ in 0..5 {
        assert_eq!(ok(finish(&cad, &[square()])), first);
    }
    // `volume` counts the fake's rebuilds: a new op list is the second one.
    let second = ok(finish(&cad, &[square(), pull(0)]));
    assert_eq!((first.volume_mm3, second.volume_mm3), (1, 2));
}

#[test]
fn a_failing_operation_is_named() {
    let cad = fake("failing");
    match finish(&cad, &[square(), pull(999)]) {
        RegenResult::Failed { op, message } => {
            assert_eq!(op, 1);
            assert!(message.contains("999"));
        }
        other => panic!("expected a failure, got {other:?}"),
    }
}

#[test]
fn a_worker_that_dies_is_replaced() {
    let cad = fake("dies");
    match finish(&cad, &[square(), pull(666)]) {
        RegenResult::Failed { op, message } => {
            assert_eq!(op, u32::MAX, "not any one operation's fault");
            assert!(message.contains("worker"), "{message}");
        }
        other => panic!("expected a failure, got {other:?}"),
    }
    // A fresh worker: its rebuild count starts again.
    assert_eq!(ok(finish(&cad, &[square()])).volume_mm3, 1);
}

#[test]
fn without_freecad_the_service_says_so() {
    let cad = Arc::new(Cad::new(None, &temp("none")));
    assert!(!cad.available());
    assert_eq!(cad.regenerate(&[square()]), Err(server_error::UNAVAILABLE));
    let jail_dir = temp("none-jail");
    let jail = Jail::new(&jail_dir).unwrap();
    let err = cad
        .export(
            &jail,
            None,
            &ExportReq {
                ops: vec![square()],
                format: ExportFormat::Step,
                path: "/model.step".to_owned(),
            },
        )
        .unwrap_err();
    assert_eq!(err.0, server_error::UNAVAILABLE);
    assert!(err.1.contains("CCOSEL_FREECADCMD"));
}

#[test]
fn nonsense_op_lists_are_refused_before_freecad_sees_them() {
    let cad = fake("nonsense");
    let nan = CadOp::Polygon {
        points: vec![[f32::NAN, 0.0, 0.0]; 3],
        normal: [0.0, 0.0, 1.0],
    };
    assert_eq!(cad.regenerate(&[nan]), Err(server_error::MALFORMED));
    let far = CadOp::PushPull {
        after: 0,
        face: 0,
        distance: f32::INFINITY,
    };
    assert_eq!(cad.regenerate(&[far]), Err(server_error::MALFORMED));
    let many = vec![square(); MAX_OPS + 1];
    assert_eq!(cad.regenerate(&many), Err(server_error::TOO_LARGE));
    let corners = CadOp::Polygon {
        points: vec![[0.0; 3]; ccosel_proto::cad::MAX_POINTS + 1],
        normal: [0.0, 0.0, 1.0],
    };
    assert_eq!(cad.regenerate(&[corners]), Err(server_error::TOO_LARGE));
}

#[test]
fn a_stored_mesh_renders_on_the_server() {
    let cad = fake("render");
    let model = ok(finish(&cad, &[square()]));
    let req = RenderReq {
        mesh: model.mesh.clone(),
        camera: CameraState {
            target: [5.0, 5.0, 0.0],
            distance: 60.0,
            ..CameraState::default()
        },
        width: 320,
        height: 240,
    };
    let scene = cad.render(&req).expect("the mesh is stored");
    assert!(scene.is_consistent());
    assert_eq!((scene.width, scene.height), (320, 240));
    assert_eq!(scene.tris.len(), 2);

    let huge = RenderReq {
        width: u16::MAX,
        height: 0,
        ..req.clone()
    };
    let scene = cad.render(&huge).unwrap();
    assert_eq!((scene.width, scene.height), (4096, 1), "clamped");

    for mesh in ["/cad/mesh/unknown", "/elsewhere/x", ""] {
        let missing = RenderReq {
            mesh: mesh.to_owned(),
            ..req.clone()
        };
        assert!(cad.render(&missing).is_none(), "{mesh}");
    }
}

#[test]
fn export_writes_into_the_jail_under_the_callers_permissions() {
    let cad = fake("export");
    let jail_dir = temp("export-jail");
    std::fs::create_dir_all(jail_dir.join("shared")).unwrap();
    let jail = Jail::new(&jail_dir).unwrap();
    let req = ExportReq {
        ops: vec![square(), pull(0)],
        format: ExportFormat::Stl,
        path: "/shared/part.stl".to_owned(),
    };
    let done = cad.export(&jail, None, &req).unwrap();
    assert_eq!(done.path, "/shared/part.stl");
    let written = std::fs::read_to_string(jail_dir.join("shared/part.stl")).unwrap();
    assert_eq!(written, "FAKE stl of 2 ops\n");
    assert_eq!(done.bytes, written.len() as u64);

    // Out of the jail, or into a folder that is not there: refused, nothing written.
    for path in ["/../escape.stl", "/missing/part.stl"] {
        let bad = ExportReq {
            path: path.to_owned(),
            ..req.clone()
        };
        let (code, detail) = cad.export(&jail, None, &bad).unwrap_err();
        assert_ne!(code, 0);
        assert_eq!(detail, path);
    }

    // A failing model exports nothing.
    let failing = ExportReq {
        ops: vec![square(), pull(999)],
        path: "/shared/bad.stl".to_owned(),
        ..req
    };
    assert!(cad.export(&jail, None, &failing).is_err());
    assert!(!jail_dir.join("shared/bad.stl").exists());
}

#[test]
fn the_freecad_launch_runs_the_worker_script() {
    // With an explicit command, as for an AppImage.
    let script = PathBuf::from("/tmp/worker.py");
    let launch = temp_env(
        "CCOSEL_FREECADCMD",
        "/opt/FreeCAD.AppImage freecadcmd",
        || Launch::freecad(&script),
    )
    .expect("configured");
    assert_eq!(launch.program, PathBuf::from("/opt/FreeCAD.AppImage"));
    assert_eq!(launch.args[0], "freecadcmd");
    assert_eq!(launch.args[1], "-c");
    assert!(launch.args[2].contains("runpy.run_path(\"/tmp/worker.py\""));
}

/// Run `f` with an environment variable set, restoring it after.
fn temp_env<T>(key: &str, value: &str, f: impl FnOnce() -> T) -> T {
    let old = std::env::var_os(key);
    // SAFETY: only this test touches this variable, and nothing reads the environment
    // concurrently with the set/remove in a way that matters for the value it reads.
    unsafe { std::env::set_var(key, value) };
    let out = f();
    match old {
        // SAFETY: as above.
        Some(v) => unsafe { std::env::set_var(key, v) },
        // SAFETY: as above.
        None => unsafe { std::env::remove_var(key) },
    }
    out
}
