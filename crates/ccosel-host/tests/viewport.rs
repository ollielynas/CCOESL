//! The 3D viewport, driven through a real offscreen `egui::Context`: the shell's side of the
//! mesh fetch, the camera, clicks and push/pull, and server rendering.

use ccosel_abi::event::{decode_batch, event_kind};
use ccosel_abi::view3d::{
    Anchor, NO_FACE, Render, Snap, ViewAction, ViewEvent, ViewTool, Viewport, decode_view_event,
};
use ccosel_abi::{Cmd, Encoder, RespRecord, Vec2};
use ccosel_host::Replayer;
use ccosel_proto::cad::MeshData;
use ccosel_proto::scene2d::Scene2D;

const APP: u64 = 7;
const VIEW: u64 = 42;
const MESH: &str = "/cad/mesh/box";

fn view(tool: ViewTool, render: Render) -> Viewport<'static> {
    Viewport {
        size: Vec2::new(400.0, 300.0),
        mesh: MESH,
        tool,
        anchor: None,
        path: &[],
        preview: None,
        selected: NO_FACE,
        render,
        sketch: &[],
        value: 0.0,
        view: ccosel_abi::view3d::ViewCommand::default(),
    }
}

fn encode(v: Viewport<'_>) -> Vec<u8> {
    let mut e = Encoder::new();
    e.push(&Cmd::Viewport3d { id: VIEW, view: v });
    e.as_slice().to_vec()
}

/// A 100 mm cube, one face per side: bottom, top, front (-Y), back, left (-X), right.
fn cube() -> MeshData {
    let s = 100.0;
    let p = vec![
        [0.0, 0.0, 0.0],
        [s, 0.0, 0.0],
        [s, s, 0.0],
        [0.0, s, 0.0],
        [0.0, 0.0, s],
        [s, 0.0, s],
        [s, s, s],
        [0.0, s, s],
    ];
    MeshData {
        vertices: p.clone(),
        positions: p,
        triangles: vec![
            [0, 2, 1],
            [0, 3, 2],
            [4, 5, 6],
            [4, 6, 7],
            [0, 1, 5],
            [0, 5, 4],
            [2, 3, 7],
            [2, 7, 6],
            [0, 4, 7],
            [0, 7, 3],
            [1, 2, 6],
            [1, 6, 5],
        ],
        tri_face: vec![0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5],
        face_normals: vec![
            [0.0, 0.0, -1.0],
            [0.0, 0.0, 1.0],
            [0.0, -1.0, 0.0],
            [0.0, 1.0, 0.0],
            [-1.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
        ],
        loose: vec![false; 6],
        ..MeshData::default()
    }
}

fn raw(events: Vec<egui::Event>) -> egui::RawInput {
    egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(800.0, 600.0),
        )),
        events,
        ..Default::default()
    }
}

struct Rig {
    ctx: egui::Context,
    r: Replayer,
    shapes: Vec<egui::epaint::ClippedShape>,
}

impl Rig {
    fn new() -> Self {
        Self {
            ctx: egui::Context::default(),
            r: Replayer::new(),
            shapes: Vec::new(),
        }
    }

    fn frame(&mut self, buf: &[u8], events: Vec<egui::Event>) -> RespRecord {
        let mut out = Vec::new();
        let mut full = self.ctx.run_ui(raw(events), |ui| {
            out = self.r.replay(ui, APP, buf).expect("valid frame");
        });
        full.textures_delta.clear();
        self.shapes = full.shapes;
        *out.iter()
            .find(|r| r.local_id == VIEW)
            .expect("the viewport reports")
    }

    fn loaded(buf: &[u8]) -> (Self, egui::Rect) {
        let mut rig = Self::new();
        rig.frame(buf, vec![]);
        assert_eq!(rig.r.take_mesh_fetches(), vec![MESH.to_owned()]);
        let bytes = postcard::to_allocvec(&cube()).unwrap();
        rig.r.mesh_done(MESH, Ok(&bytes));
        let rec = rig.frame(buf, vec![]);
        let rect = egui::Rect::from_min_max(
            egui::pos2(rec.rect[0], rec.rect[1]),
            egui::pos2(rec.rect[2], rec.rect[3]),
        );
        (rig, rect)
    }

    fn triangles_drawn(&self) -> usize {
        self.shapes
            .iter()
            .map(|s| match &s.shape {
                egui::Shape::Mesh(m) => m.indices.len() / 3,
                _ => 0,
            })
            .sum()
    }

    fn texts(&self) -> Vec<String> {
        self.shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) => Some(t.galley.text().to_owned()),
                _ => None,
            })
            .collect()
    }

    fn events(&mut self) -> Vec<ViewEvent> {
        let Some(batch) = self.r.take_view_events() else {
            return Vec::new();
        };
        decode_batch(&batch)
            .unwrap()
            .into_iter()
            .map(|e| {
                assert_eq!(e.kind, event_kind::VIEWPORT);
                decode_view_event(e.payload).unwrap()
            })
            .collect()
    }
}

fn press(pos: egui::Pos2, button: egui::PointerButton, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button,
        pressed,
        modifiers: egui::Modifiers::default(),
    }
}

/// A press at `from`, a drag through `to` in steps, and a release, one step per frame.
fn drag(rig: &mut Rig, buf: &[u8], from: egui::Pos2, to: egui::Pos2, button: egui::PointerButton) {
    rig.frame(
        buf,
        vec![egui::Event::PointerMoved(from), press(from, button, true)],
    );
    for k in 1..=6 {
        let p = from + (to - from) * (k as f32 / 6.0);
        rig.frame(buf, vec![egui::Event::PointerMoved(p)]);
    }
    rig.frame(buf, vec![press(to, button, false)]);
}

#[test]
fn a_mesh_is_fetched_once_then_drawn() {
    let buf = encode(view(ViewTool::Select, Render::Local));
    let mut rig = Rig::new();
    let rec = rig.frame(&buf, vec![]);
    assert_eq!(rec.rect[2] - rec.rect[0], 400.0);
    assert!(rig.texts().iter().any(|t| t.contains("Loading")));
    assert_eq!(rig.r.take_mesh_fetches(), vec![MESH.to_owned()]);
    rig.frame(&buf, vec![]);
    assert!(rig.r.take_mesh_fetches().is_empty(), "asked once");

    let bytes = postcard::to_allocvec(&cube()).unwrap();
    rig.r.mesh_done(MESH, Ok(&bytes));
    rig.frame(&buf, vec![]);
    // The camera frames the cube on arrival: three faces, two triangles each.
    assert_eq!(rig.triangles_drawn(), 6);
    assert!(!rig.texts().iter().any(|t| t.contains("Loading")));
}

#[test]
fn a_mesh_that_fails_says_so_without_panicking() {
    let buf = encode(view(ViewTool::Select, Render::Local));
    let mut rig = Rig::new();
    rig.frame(&buf, vec![]);
    rig.r.take_mesh_fetches();
    rig.r.mesh_done(MESH, Ok(&[0xff, 0x00, 0x13]));
    rig.frame(&buf, vec![]);
    assert!(rig.texts().iter().any(|t| t.contains("does not decode")));

    let mut bad = cube();
    bad.triangles[0][0] = 99;
    let mut rig = Rig::new();
    rig.frame(&buf, vec![]);
    rig.r.take_mesh_fetches();
    rig.r
        .mesh_done(MESH, Ok(&postcard::to_allocvec(&bad).unwrap()));
    rig.frame(&buf, vec![]);
    assert!(rig.texts().iter().any(|t| t.contains("out of range")));
}

#[test]
fn clicking_the_top_face_reports_it() {
    let buf = encode(view(ViewTool::Select, Render::Local));
    let (mut rig, rect) = Rig::loaded(&buf);
    // The camera frames the cube from above, so the middle of the view is on the top face.
    let c = rect.center();
    rig.frame(&buf, vec![egui::Event::PointerMoved(c)]);
    rig.frame(&buf, vec![press(c, egui::PointerButton::Primary, true)]);
    rig.frame(&buf, vec![press(c, egui::PointerButton::Primary, false)]);
    let events = rig.events();
    assert_eq!(events.len(), 1);
    let e = events[0];
    assert_eq!(e.id, VIEW);
    assert_eq!(e.action, ViewAction::Click);
    assert_ne!(e.face, NO_FACE);
    assert!(rig.events().is_empty(), "taken once");
}

#[test]
fn drawing_a_rectangle_stays_on_the_anchor_plane() {
    let anchored = Viewport {
        anchor: Some(Anchor {
            point: [0.0, 0.0, 100.0],
            normal: [0.0, 0.0, 1.0],
        }),
        ..view(ViewTool::Rect, Render::Local)
    };
    let buf = encode(anchored);
    let (mut rig, rect) = Rig::loaded(&buf);
    let p = rect.center() + egui::vec2(37.0, 21.0);
    rig.frame(&buf, vec![egui::Event::PointerMoved(p)]);
    // The rubber-band rectangle is drawn while the pointer moves.
    assert!(
        rig.shapes
            .iter()
            .any(|s| matches!(s.shape, egui::Shape::LineSegment { .. }))
    );
    rig.frame(&buf, vec![press(p, egui::PointerButton::Primary, true)]);
    rig.frame(&buf, vec![press(p, egui::PointerButton::Primary, false)]);
    let e = rig.events()[0];
    assert!((e.point[2] - 100.0).abs() < 1e-3, "{:?}", e.point);
    assert_eq!(e.normal, [0.0, 0.0, 1.0]);
}

/// A pixel over the cube's top face, found the way a user would: by clicking.
fn top_face_pixel(rig: &mut Rig, rect: egui::Rect) -> egui::Pos2 {
    let buf = encode(view(ViewTool::Select, Render::Local));
    for dy in (0..160).step_by(8) {
        let p = rect.center() - egui::vec2(0.0, dy as f32);
        rig.frame(&buf, vec![egui::Event::PointerMoved(p)]);
        rig.frame(&buf, vec![press(p, egui::PointerButton::Primary, true)]);
        rig.frame(&buf, vec![press(p, egui::PointerButton::Primary, false)]);
        if rig.events().iter().any(|e| e.face == 1) {
            return p;
        }
    }
    panic!("the top face is not on screen");
}

#[test]
fn pulling_the_top_face_up_reports_a_positive_distance() {
    let buf = encode(view(ViewTool::PushPull, Render::Local));
    let (mut rig, rect) = Rig::loaded(&buf);
    let from = top_face_pixel(&mut rig, rect);
    rig.frame(&buf, vec![]);
    let before = rig.triangles_drawn();
    rig.frame(
        &buf,
        vec![
            egui::Event::PointerMoved(from),
            press(from, egui::PointerButton::Primary, true),
        ],
    );
    for k in 1..=5 {
        rig.frame(
            &buf,
            vec![egui::Event::PointerMoved(
                from - egui::vec2(0.0, 12.0 * k as f32),
            )],
        );
    }
    assert!(
        rig.triangles_drawn() > before,
        "the extrusion is drawn during the drag"
    );
    assert!(rig.texts().iter().any(|t| t.ends_with(" mm")));
    let to = from - egui::vec2(0.0, 60.0);
    rig.frame(&buf, vec![press(to, egui::PointerButton::Primary, false)]);
    let events = rig.events();
    assert_eq!(events.len(), 1);
    let e = events[0];
    assert_eq!(e.action, ViewAction::PushPull);
    assert_eq!(e.face, 1, "the top");
    assert_eq!(e.normal, [0.0, 0.0, 1.0]);
    assert!(e.distance > 1.0, "{}", e.distance);
    // Released but not yet taken over by the guest: still drawn, so nothing flickers.
    rig.frame(&buf, vec![]);
    assert!(rig.triangles_drawn() > before);
}

#[test]
fn a_guest_preview_is_kept_while_the_new_mesh_loads() {
    let with_preview = Viewport {
        preview: Some(ccosel_abi::view3d::Extrude {
            face: 1,
            distance: 50.0,
        }),
        ..view(ViewTool::PushPull, Render::Local)
    };
    let (mut rig, _) = Rig::loaded(&encode(view(ViewTool::PushPull, Render::Local)));
    let plain = rig.triangles_drawn();
    rig.frame(&encode(with_preview), vec![]);
    let previewed = rig.triangles_drawn();
    assert!(previewed > plain);
    // The rebuild finished: the guest names the new mesh and drops its preview. Until the
    // new mesh arrives the old one and the preview stay up.
    let next = Viewport {
        mesh: "/cad/mesh/taller",
        ..view(ViewTool::PushPull, Render::Local)
    };
    rig.frame(&encode(next), vec![]);
    assert_eq!(rig.triangles_drawn(), previewed);
    assert_eq!(
        rig.r.take_mesh_fetches(),
        vec!["/cad/mesh/taller".to_owned()]
    );
}

#[test]
fn server_rendering_asks_for_one_frame_at_a_time() {
    let buf = encode(view(ViewTool::Orbit, Render::Server));
    let (mut rig, rect) = Rig::loaded(&buf);
    let jobs = rig.r.take_render_jobs();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].req.mesh, MESH);
    assert_eq!((jobs[0].req.width, jobs[0].req.height), (400, 300));
    // Orbit while the first is still out: no second request piles up.
    drag(
        &mut rig,
        &buf,
        rect.center(),
        rect.center() + egui::vec2(80.0, 0.0),
        egui::PointerButton::Primary,
    );
    assert!(rig.r.take_render_jobs().is_empty());
    assert_eq!(
        rig.triangles_drawn(),
        0,
        "nothing to show until a frame arrives"
    );

    // The server answers with a frame for the old camera; the shell draws it and asks again
    // for the camera as it is now.
    let frame = Scene2D {
        width: 400,
        height: 300,
        points: vec![[0, 0], [800, 0], [0, 800]],
        tris: vec![[0, 1, 2]],
        tri_colors: vec![ccosel_proto::scene2d::rgba(200, 200, 200, 255)],
        ..Scene2D::default()
    };
    rig.r
        .render_done(jobs[0].key, Ok(&postcard::to_allocvec(&frame).unwrap()));
    rig.frame(&buf, vec![]);
    assert!(rig.triangles_drawn() >= 1);
    let again = rig.r.take_render_jobs();
    assert_eq!(again.len(), 1);
    assert_ne!(
        again[0].req.camera, jobs[0].req.camera,
        "the orbit moved the camera"
    );
}

#[test]
fn auto_renders_a_small_model_locally() {
    let buf = encode(view(ViewTool::Select, Render::Auto));
    let (mut rig, _) = Rig::loaded(&buf);
    rig.frame(&buf, vec![]);
    assert!(rig.r.take_render_jobs().is_empty());
    assert_eq!(rig.triangles_drawn(), 6);
}

#[test]
fn a_bad_server_frame_is_reported_not_drawn() {
    let buf = encode(view(ViewTool::Select, Render::Server));
    let (mut rig, _) = Rig::loaded(&buf);
    let job = rig.r.take_render_jobs().remove(0);
    let corrupt = Scene2D {
        tris: vec![[0, 1, 2]],
        tri_colors: vec![0],
        ..Scene2D::default()
    };
    rig.r
        .render_done(job.key, Ok(&postcard::to_allocvec(&corrupt).unwrap()));
    // An answer for a job nobody asked for is ignored.
    rig.r.render_done(job.key + 100, Err("nope".to_owned()));
    rig.frame(&buf, vec![]);
    assert!(rig.texts().iter().any(|t| t.contains("corrupt")));
}

#[test]
fn the_wheel_zooms_and_the_middle_button_orbits() {
    let buf = encode(view(ViewTool::Line, Render::Server));
    let (mut rig, rect) = Rig::loaded(&buf);
    let first = rig.r.take_render_jobs().remove(0);
    let ok = postcard::to_allocvec(&Scene2D::default()).unwrap();
    rig.r.render_done(first.key, Ok(&ok));

    rig.frame(
        &buf,
        vec![
            egui::Event::PointerMoved(rect.center()),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, 120.0),
                modifiers: egui::Modifiers::default(),
                phase: egui::TouchPhase::Move,
            },
        ],
    );
    // egui smooths the wheel over a few frames.
    for _ in 0..10 {
        rig.frame(&buf, vec![]);
    }
    let zoomed = rig.r.take_render_jobs().remove(0);
    assert!(zoomed.req.camera.distance < first.req.camera.distance);
    rig.r.render_done(zoomed.key, Ok(&ok));

    drag(
        &mut rig,
        &buf,
        rect.center(),
        rect.center() + egui::vec2(0.0, 50.0),
        egui::PointerButton::Middle,
    );
    // The request that went out as the drag began still has the old camera; once it is
    // answered, the next one carries the orbit.
    let during = rig.r.take_render_jobs().remove(0);
    rig.r.render_done(during.key, Ok(&ok));
    rig.frame(&buf, vec![]);
    let orbited = rig.r.take_render_jobs().remove(0);
    assert_ne!(orbited.req.camera.elevation, zoomed.req.camera.elevation);
    assert!(rig.events().is_empty(), "camera moves are not gestures");
}

#[test]
fn a_hostile_viewport_does_not_panic() {
    for size in [
        Vec2::new(f32::NAN, f32::INFINITY),
        Vec2::new(-5.0, 0.0),
        Vec2::new(1e30, 1e30),
    ] {
        let buf = encode(Viewport {
            size,
            anchor: Some(Anchor {
                point: [f32::NAN; 3],
                normal: [0.0; 3],
            }),
            ..view(ViewTool::Rect, Render::Auto)
        });
        let mut rig = Rig::new();
        rig.frame(
            &buf,
            vec![egui::Event::PointerMoved(egui::pos2(100.0, 100.0))],
        );
    }
}

#[test]
fn snapping_is_shown_while_drawing() {
    let anchored = Viewport {
        anchor: Some(Anchor {
            point: [0.0, 0.0, 100.0],
            normal: [0.0, 0.0, 1.0],
        }),
        ..view(ViewTool::Line, Render::Local)
    };
    let buf = encode(anchored);
    let (mut rig, rect) = Rig::loaded(&buf);
    // Hunt for a pixel that snaps to something: the cube's far top corner, on screen
    // somewhere inside the view.
    let mut labels = Vec::new();
    for dx in (-150..=150).step_by(10) {
        for dy in (-120..=120).step_by(10) {
            let p = rect.center() + egui::vec2(dx as f32, dy as f32);
            rig.frame(&buf, vec![egui::Event::PointerMoved(p)]);
            labels.extend(rig.texts());
        }
    }
    assert!(labels.iter().any(|l| l == "Endpoint"), "{labels:?}");
    let _ = Snap::Endpoint;
}
