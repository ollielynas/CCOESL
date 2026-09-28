use ccosel_proto::account::Account;
use ccosel_proto::cad::RegenStatus;
use ccosel_proto::sketch::{
    Constraint, Curve, Geo, Item, PointRef, Sketch, SketchEvent, SketchScene,
};
use ccosel_sdk::testing::{Harness, rpc_error};
use ccosel_sdk::view3d::Snap;

use super::*;

const UP: [f32; 3] = [0.0, 0.0, 1.0];

fn model(mesh: &str, faces: u32, solids: u32, volume: u64) -> Model {
    Model {
        mesh: mesh.to_owned(),
        triangles: faces * 2,
        faces,
        new_faces: Vec::new(),
        solids,
        volume_mm3: volume,
    }
}

fn built(m: Model) -> RegenStatus {
    RegenStatus {
        finished: true,
        elapsed_ms: 40,
        result: Some(RegenResult::Ok(m)),
        note: String::new(),
    }
}

fn failed(op: u32, message: &str) -> RegenStatus {
    RegenStatus {
        finished: true,
        elapsed_ms: 40,
        result: Some(RegenResult::Failed {
            op,
            message: message.to_owned(),
        }),
        note: String::new(),
    }
}

fn running() -> RegenStatus {
    RegenStatus {
        finished: false,
        elapsed_ms: 10,
        result: None,
        note: String::new(),
    }
}

/// A draw tool's toolbar label: its icon and name.
fn draw(name: &str) -> String {
    let (icon, _, _) = DRAW_TOOLS.iter().find(|(_, n, _)| *n == name).unwrap();
    format!("{icon} {name}")
}

fn status_line(h: &Harness<Modeller>) -> String {
    h.app.status()
}

/// A fresh Modeller whose first frame has run.
fn fresh() -> Harness<Modeller> {
    let mut h = Harness::new(Modeller::default());
    h.frame();
    h
}

/// Two clicks with the Rectangle tool, from `a` to `b` on the ground.
fn rectangle(h: &mut Harness<Modeller>, a: [f32; 3], b: [f32; 3]) {
    h.view_click(NO_FACE, a, UP, Snap::Free);
    h.frame();
    h.view_click(NO_FACE, b, UP, Snap::Free);
    h.frame();
}

/// A rectangle on the ground, rebuilt by the "server" into one loose face.
fn with_a_square() -> Harness<Modeller> {
    let mut h = fresh();
    rectangle(&mut h, [0.0; 3], [100.0, 100.0, 0.0]);
    h.reply::<Regenerate>(&built(Model {
        new_faces: vec![0],
        ..model("/cad/mesh/a", 1, 0, 0)
    }));
    h.frame();
    h
}

// ── Part Design ─────────────────────────────────────────────────────────────

#[test]
fn it_opens_ready_to_draw_a_rectangle() {
    let h = fresh();
    assert_eq!(h.app.tool, ViewTool::Rect);
    assert!(h.has_label("Click the first corner, on the ground or a face."));
    assert!(h.has_label("Draw a rectangle on the ground, or create a sketch, to start."));
    let view = &h.viewports()[0];
    assert_eq!(view.mesh, "");
    assert!(view.sketch.is_empty());
    assert_eq!(h.outstanding::<Regenerate>(), 0, "nothing to build yet");
    assert_eq!(h.app.wants_repaint_after_ms(), REPAINT_ON_INPUT_ONLY);
    // FreeCAD's layout: File, Edit, workbench, View on the first row.
    let buttons = h.buttons();
    let pos = |s: &str| buttons.iter().position(|b| b == s).unwrap();
    assert!(pos(icons::FILE_PLUS) < pos(icons::ARROW_COUNTER_CLOCKWISE));
    assert!(pos(icons::ARROW_COUNTER_CLOCKWISE) < pos(icons::ARROWS_OUT));
    assert!(h.selectables().contains(&("Part Design".to_owned(), true)));
    assert!(h.selectables().contains(&("Sketcher".to_owned(), false)));
}

#[test]
fn two_clicks_draw_a_rectangle_and_the_server_builds_it() {
    let mut h = fresh();
    h.view_click(NO_FACE, [10.0, 20.0, 0.0], UP, Snap::Free);
    h.frame();
    h.frame();
    assert!(h.has_label("Click the opposite corner."));
    h.view_click(NO_FACE, [110.0, 70.0, 0.0], UP, Snap::Endpoint);
    h.frame();
    assert_eq!(
        h.app.ops,
        vec![CadOp::Polygon {
            points: vec![
                [10.0, 20.0, 0.0],
                [110.0, 20.0, 0.0],
                [110.0, 70.0, 0.0],
                [10.0, 70.0, 0.0]
            ],
            normal: UP,
        }]
    );
    assert_eq!(
        h.outstanding::<Regenerate>(),
        1,
        "asked for in the same frame"
    );
    h.frame();
    assert!(h.has_label("Building…"));
    assert_eq!(h.app.wants_repaint_after_ms(), POLL_MS);
    h.reply::<Regenerate>(&built(model("/cad/mesh/a", 1, 0, 0)));
    h.frame();
    assert_eq!(h.viewports()[0].mesh, "/cad/mesh/a");
    assert_eq!(status_line(&h), "1 face");
}

#[test]
fn clicking_the_same_corner_twice_is_not_a_rectangle() {
    let mut h = fresh();
    rectangle(&mut h, [5.0, 5.0, 0.0], [5.0, 5.0, 0.0]);
    assert!(h.app.ops.is_empty());
    assert!(
        h.app.anchor.is_some(),
        "still waiting for the opposite corner"
    );
}

#[test]
fn a_rebuild_in_progress_is_polled_and_install_progress_shown() {
    let mut h = fresh();
    rectangle(&mut h, [0.0; 3], [10.0, 10.0, 0.0]);
    h.reply::<Regenerate>(&RegenStatus {
        note: "Installing FreeCAD on the server: downloading, 42% of 820 MB".to_owned(),
        ..running()
    });
    h.frame();
    assert_eq!(
        status_line(&h),
        "Installing FreeCAD on the server: downloading, 42% of 820 MB"
    );
    h.frame();
    assert_eq!(h.outstanding::<Regenerate>(), 1, "asked again");
    h.reply::<Regenerate>(&running());
    h.frame();
    assert_eq!(status_line(&h), "Building…");
    h.frame();
    h.reply::<Regenerate>(&built(model("/cad/mesh/b", 1, 0, 0)));
    h.frame();
    assert_eq!(h.viewports()[0].mesh, "/cad/mesh/b");
    assert_eq!(h.app.wants_repaint_after_ms(), REPAINT_ON_INPUT_ONLY);
}

#[test]
fn a_push_pull_becomes_a_step_and_previews_until_rebuilt() {
    let mut h = with_a_square();
    h.click(&draw("Push/Pull"));
    h.frame();
    h.frame();
    assert!(h.has_label("Drag a face to push or pull it."));
    h.view_push_pull(0, UP, 100.0);
    h.frame();
    h.frame();
    assert_eq!(
        h.app.ops[1],
        CadOp::PushPull {
            after: 1,
            face: 0,
            distance: 100.0
        }
    );
    assert_eq!(
        h.viewports()[0].preview,
        Some(Extrude {
            face: 0,
            distance: 100.0
        })
    );
    assert!(h.has_label("100 mm"));
    h.reply::<Regenerate>(&built(model("/cad/mesh/cube", 6, 1, 1_000_000)));
    h.frame();
    assert_eq!(h.viewports()[0].preview, None);
    assert_eq!(status_line(&h), "6 faces · 1 solid · 1 000 000 mm³");
}

#[test]
fn typing_a_distance_remakes_the_last_push_pull() {
    let mut h = with_a_square();
    h.view_push_pull(0, UP, 37.2);
    h.frame();
    h.type_text(0, "2m");
    h.frame();
    h.click("Apply");
    h.frame();
    assert_eq!(
        h.app.ops[1],
        CadOp::PushPull {
            after: 1,
            face: 0,
            distance: 2000.0
        }
    );
    h.frame();
    assert!(h.has_label("2 000 mm"));
}

#[test]
fn typing_width_and_height_remakes_the_last_rectangle_the_same_way_round() {
    let mut h = fresh();
    rectangle(&mut h, [0.0; 3], [-10.0, 10.0, 0.0]);
    h.type_text(0, "300,200");
    h.frame();
    h.click("Apply");
    h.frame();
    let CadOp::Polygon { points, .. } = &h.app.ops[0] else {
        panic!("a polygon")
    };
    assert!(points.contains(&[-300.0, 200.0, 0.0]));
    h.type_text(0, "nonsense");
    h.frame();
    h.click("Apply");
    h.frame();
    h.frame();
    assert!(h.has_label("Type a length, or width,height for a rectangle."));
}

#[test]
fn the_line_tool_closes_an_outline_on_its_first_corner() {
    let mut h = fresh();
    h.click(&draw("Line"));
    h.frame();
    let corners = [[0.0, 0.0, 0.0], [50.0, 0.0, 0.0], [50.0, 30.0, 0.0]];
    for c in corners {
        h.view_click(NO_FACE, c, UP, Snap::Free);
        h.frame();
    }
    h.view_click(NO_FACE, corners[2], UP, Snap::Endpoint);
    h.frame();
    h.frame();
    assert_eq!(h.viewports()[0].path, corners.to_vec());
    h.view_click(NO_FACE, corners[0], UP, Snap::Endpoint);
    h.frame();
    assert_eq!(
        h.app.ops,
        vec![CadOp::Polygon {
            points: corners.to_vec(),
            normal: UP
        }]
    );
}

#[test]
fn undo_and_redo_on_the_model() {
    let mut h = with_a_square();
    h.view_push_pull(0, UP, 10.0);
    h.frame();
    h.view_click(NO_FACE, [1.0, 1.0, 10.0], UP, Snap::OnFace);
    h.frame();
    assert!(h.app.anchor.is_some());
    let undo = icons::ARROW_COUNTER_CLOCKWISE;
    let redo = icons::ARROW_CLOCKWISE;
    h.click(undo);
    h.frame();
    assert!(
        h.app.anchor.is_none(),
        "the half-drawn rectangle goes first"
    );
    h.click(undo);
    h.frame();
    assert_eq!(h.app.ops.len(), 1);
    h.click(redo);
    h.frame();
    assert_eq!(h.app.ops.len(), 2);
    h.click(icons::FILE_PLUS);
    h.frame();
    assert!(h.app.ops.is_empty(), "New empties the model");
    assert_eq!(h.app.redo.len(), 2, "which undo can bring back");
}

#[test]
fn a_step_freecad_refuses_is_named() {
    let mut h = with_a_square();
    h.view_push_pull(0, UP, 10.0);
    h.frame();
    h.reply::<Regenerate>(&failed(1, "there is no face 0"));
    h.frame();
    assert_eq!(
        status_line(&h),
        "Step 2 couldn't be done: there is no face 0. Undo to go back."
    );
    let mut h = fresh();
    rectangle(&mut h, [0.0; 3], [10.0, 10.0, 0.0]);
    h.reply::<Regenerate>(&failed(u32::MAX, "timed out"));
    h.frame();
    assert_eq!(status_line(&h), "Couldn't build the model: timed out");
    let mut h = fresh();
    rectangle(&mut h, [0.0; 3], [10.0, 10.0, 0.0]);
    h.fail::<Regenerate>(rpc_error::TIMEOUT);
    h.frame();
    assert!(status_line(&h).starts_with("Couldn't build the model: "));
}

#[test]
fn select_and_orbit_tools() {
    let mut h = with_a_square();
    h.click(&draw("Select"));
    h.frame();
    h.view_click(0, [5.0, 5.0, 0.0], UP, Snap::OnFace);
    h.frame();
    h.frame();
    assert_eq!(h.viewports()[0].selected, 0);
    h.click(&draw("Orbit"));
    h.frame();
    h.frame();
    assert!(h.has_label("Drag to orbit, shift-drag to pan, scroll to zoom."));
    assert_eq!(h.viewports()[0].tool, ViewTool::Orbit);
}

#[test]
fn the_view_toolbar_sends_one_camera_move_per_press() {
    let mut h = fresh();
    assert_eq!(h.viewports()[0].view, ViewCommand::default());
    h.click("Top");
    h.frame();
    h.frame();
    assert_eq!(
        h.viewports()[0].view,
        ViewCommand {
            view: StdView::Top,
            seq: 1
        }
    );
    h.click(icons::ARROWS_OUT);
    h.frame();
    h.frame();
    assert_eq!(h.viewports()[0].view.view, StdView::Fit);
    assert_eq!(h.viewports()[0].view.seq, 2);
    for name in ["Front", "Right", "Rear", "Bottom", "Left", icons::CUBE] {
        h.click(name);
        h.frame();
    }
    h.frame();
    assert_eq!(h.viewports()[0].view.view, StdView::Isometric);
}

#[test]
fn the_render_button_cycles_where_the_view_is_drawn() {
    let mut h = fresh();
    for (label, next) in [
        ("Render: Auto", Render::Local),
        ("Render: Browser", Render::Server),
        ("Render: Server", Render::Auto),
    ] {
        h.click(label);
        h.frame();
        h.frame();
        assert_eq!(h.viewports()[0].render, next);
    }
}

#[test]
fn saving_goes_to_the_home_folder_and_can_be_downloaded() {
    let mut h = fresh();
    h.click(icons::FLOPPY_DISK);
    h.frame();
    h.frame();
    assert_eq!(status_line(&h), "There is nothing to save yet.");

    let mut h = with_a_square();
    h.reply::<WhoAmI>(&Account {
        login_enabled: true,
        name: Some("ada".to_owned()),
        account_url: None,
    });
    h.frame();
    h.frame();
    assert_eq!(h.text_fields()[1], "/home/ada/model");
    h.click(icons::EXPORT);
    h.frame();
    h.frame();
    assert_eq!(h.outstanding::<ExportModel>(), 1);
    assert!(h.has_label("Saving…"));
    h.click(icons::EXPORT);
    h.frame();
    assert_eq!(h.outstanding::<ExportModel>(), 1, "one at a time");
    h.reply::<ExportModel>(&Exported {
        path: "/home/ada/model.step".to_owned(),
        bytes: 1234,
    });
    h.frame();
    h.frame();
    assert!(h.has_label("Saved /home/ada/model.step"));
    assert_eq!(
        h.open_urls(),
        vec![(
            "Download".to_owned(),
            "/files/home/ada/model.step".to_owned()
        )]
    );
    h.click(icons::ARROW_SQUARE_OUT);
    h.frame();
    h.fail::<ExportModel>(rpc_error::DENIED);
    h.frame();
    h.frame();
    assert!(
        h.labels()
            .iter()
            .any(|l| l.starts_with("Couldn't save /home/ada/model.stl"))
    );
}

#[test]
fn pad_and_pocket_push_the_faces_the_last_step_made() {
    let mut h = with_a_square();
    h.type_text(0, "25");
    h.frame();
    h.click("Pad");
    h.frame();
    assert_eq!(
        h.app.ops[1],
        CadOp::PushPull {
            after: 1,
            face: 0,
            distance: 25.0
        }
    );
    // Now the model on screen is not the whole model: nothing to pad until it is rebuilt.
    h.click("Pocket");
    h.frame();
    h.frame();
    assert_eq!(
        status_line(&h),
        "Pad and Pocket work on the sketch or shape just made."
    );
    let mut h = with_a_square();
    h.click("Pocket");
    h.frame();
    assert_eq!(
        h.app.ops[1],
        CadOp::PushPull {
            after: 1,
            face: 0,
            distance: -10.0
        },
        "10 mm when Measurements is empty"
    );
    let mut h = with_a_square();
    h.type_text(0, "far");
    h.frame();
    h.click("Pad");
    h.frame();
    assert_eq!(h.app.ops.len(), 1);
    assert!(h.app.measure_error);
}

// ── Sketcher ────────────────────────────────────────────────────────────────

/// Open a sketch on the ground and return the harness.
fn sketching() -> Harness<Modeller> {
    let mut h = fresh();
    h.click("Create sketch");
    h.frame();
    h.frame();
    assert!(h.has_label("Click a face, or the ground, to sketch on."));
    h.view_click(NO_FACE, [5.0, 5.0, 0.0], UP, Snap::Free);
    h.frame();
    h.frame();
    h
}

fn scene(h: &Harness<Modeller>) -> SketchScene {
    postcard::from_bytes(&h.viewports()[0].sketch).expect("a sketch is shown")
}

fn send(h: &mut Harness<Modeller>, e: SketchEvent) {
    h.view_sketch_event(&postcard::to_allocvec(&e).unwrap());
    h.frame();
    h.frame();
}

/// The sketch with one line in it, as the shell would report after the Line tool.
fn with_line(h: &mut Harness<Modeller>) {
    let mut s = scene(h).sketch;
    s.geos.push(Geo::new(Curve::Line([0.0, 0.0], [30.0, 40.0])));
    send(h, SketchEvent::Edited(s));
}

#[test]
fn a_sketch_opens_on_the_clicked_plane_facing_the_camera() {
    let h = sketching();
    let s = scene(&h);
    assert_eq!(s.sketch.plane.normal, [0.0, 0.0, 1.0]);
    assert_eq!(
        s.sketch.plane.origin,
        [0.0, 0.0, 0.0],
        "the ground, through the origin"
    );
    assert_eq!(s.sketch.plane.x_dir, [1.0, 0.0, 0.0]);
    assert_eq!(h.viewports()[0].tool, ViewTool::Line);
    assert_eq!(h.viewports()[0].view.view, StdView::Sketch);
    assert!(h.selectables().contains(&("Sketcher".to_owned(), true)));
    assert!(h.has_button("Coincident") && h.has_button("Angle"));
    assert!(!h.has_button("Pad"), "Part Design's toolbar is away");
    // A raised face: the plane goes through it.
    let mut h = fresh();
    h.click("Sketcher");
    h.frame();
    h.view_click(3, [0.0, 0.0, 50.0], UP, Snap::OnFace);
    h.frame();
    h.frame();
    assert_eq!(scene(&h).sketch.plane.origin, [0.0, 0.0, 50.0]);
}

#[test]
fn the_shells_edits_are_adopted_and_undoable() {
    let mut h = sketching();
    with_line(&mut h);
    assert_eq!(scene(&h).sketch.geos.len(), 1);
    h.click(icons::ARROW_COUNTER_CLOCKWISE);
    h.frame();
    h.frame();
    assert!(scene(&h).sketch.geos.is_empty(), "undo inside the sketch");
    h.click(icons::ARROW_CLOCKWISE);
    h.frame();
    h.frame();
    assert_eq!(scene(&h).sketch.geos.len(), 1);
    assert!(h.app.ops.is_empty(), "the model's steps are untouched");
    h.frame();
    let plane = scene(&h).sketch.plane;
    send(&mut h, SketchEvent::Edited(Sketch::new(plane)));
    h.click(icons::ARROW_COUNTER_CLOCKWISE);
    h.frame();
    h.frame();
    assert_eq!(scene(&h).sketch.geos.len(), 1);
}

#[test]
fn picking_toggles_the_selection_and_empty_space_clears_it() {
    let mut h = sketching();
    with_line(&mut h);
    send(&mut h, SketchEvent::Picked(Some(Item::Geo(0))));
    assert_eq!(scene(&h).selected, vec![Item::Geo(0)]);
    assert!(h.has_label("1 selected"));
    send(
        &mut h,
        SketchEvent::Picked(Some(Item::Point(PointRef::ORIGIN))),
    );
    assert_eq!(scene(&h).selected.len(), 2);
    send(&mut h, SketchEvent::Picked(Some(Item::Geo(0))));
    assert_eq!(scene(&h).selected, vec![Item::Point(PointRef::ORIGIN)]);
    send(&mut h, SketchEvent::Picked(None));
    assert!(scene(&h).selected.is_empty());
    // Garbage from the shell is ignored.
    h.view_sketch_event(&[0xff, 0xff]);
    h.frame();
}

#[test]
fn constraint_buttons_constrain_the_selection() {
    let mut h = sketching();
    with_line(&mut h);
    send(&mut h, SketchEvent::Picked(Some(Item::Geo(0))));
    h.click("Distance");
    h.frame();
    h.frame();
    let s = scene(&h);
    assert_eq!(s.sketch.constraints, vec![Constraint::Distance(0, 50.0)]);
    assert!(
        s.selected.is_empty(),
        "the selection is used up, as in FreeCAD"
    );

    // A wrong selection says what it needs.
    send(&mut h, SketchEvent::Picked(Some(Item::Geo(0))));
    h.click("Radius");
    h.frame();
    h.frame();
    assert_eq!(status_line(&h), "Select circles or arcs.");
    // The next gesture clears the complaint.
    send(&mut h, SketchEvent::Picked(None));
    assert_ne!(status_line(&h), "Select circles or arcs.");

    for name in [
        "Coincident",
        "Point on object",
        "Vertical",
        "Horizontal",
        "Parallel",
        "Perpendicular",
        "Tangent",
        "Equal",
        "Symmetric",
        "Lock",
        "Horizontal distance",
        "Vertical distance",
        "Distance",
        "Radius",
        "Diameter",
        "Angle",
    ] {
        assert!(h.has_button(name), "{name}");
    }
}

#[test]
fn a_dimension_takes_a_typed_value_in_millimetres_or_degrees() {
    let mut h = sketching();
    with_line(&mut h);
    send(&mut h, SketchEvent::Picked(Some(Item::Geo(0))));
    h.click("Distance");
    h.frame();
    send(&mut h, SketchEvent::Picked(Some(Item::Geo(0))));
    h.click("Angle");
    h.frame();
    h.frame();
    assert_eq!(scene(&h).sketch.constraints.len(), 2);

    send(&mut h, SketchEvent::Picked(Some(Item::Constraint(0))));
    h.type_text(0, "12cm");
    h.frame();
    h.click("Set");
    h.frame();
    h.frame();
    assert_eq!(
        scene(&h).sketch.constraints[0],
        Constraint::Distance(0, 120.0)
    );

    send(&mut h, SketchEvent::Picked(None));
    send(&mut h, SketchEvent::Picked(Some(Item::Constraint(1))));
    h.type_text(0, "45°");
    h.frame();
    h.click("Set");
    h.frame();
    h.frame();
    let Constraint::Angle(0, a) = scene(&h).sketch.constraints[1] else {
        panic!()
    };
    assert!((a - std::f64::consts::FRAC_PI_4).abs() < 1e-6);

    h.type_text(0, "lots");
    h.frame();
    h.click("Set");
    h.frame();
    h.frame();
    assert_eq!(status_line(&h), "Type a number.");
    send(&mut h, SketchEvent::Picked(None));
    h.click("Set");
    h.frame();
    h.frame();
    assert_eq!(
        status_line(&h),
        "Select one dimension, then type its value."
    );
}

#[test]
fn construction_offset_and_delete() {
    let mut h = sketching();
    h.click("Construction");
    h.frame();
    h.frame();
    assert_eq!(
        status_line(&h),
        "Select geometry to switch to or from construction."
    );
    h.click("Offset");
    h.frame();
    h.frame();
    assert_eq!(
        status_line(&h),
        "Select geometry and type the offset distance in Value."
    );
    h.click(icons::TRASH);
    h.frame();
    h.frame();
    assert_eq!(status_line(&h), "Select something to delete.");

    with_line(&mut h);
    send(&mut h, SketchEvent::Picked(Some(Item::Geo(0))));
    h.click("Construction");
    h.frame();
    h.frame();
    assert!(scene(&h).sketch.geos[0].construction);
    h.type_text(0, "5");
    h.frame();
    h.click("Offset");
    h.frame();
    h.frame();
    assert_eq!(scene(&h).sketch.geos.len(), 2);
    assert_eq!(
        h.viewports()[0].value,
        5.0,
        "the value is the fillet radius too"
    );
    h.click(icons::TRASH);
    h.frame();
    h.frame();
    let s = scene(&h);
    assert_eq!(s.sketch.geos.len(), 1);
    assert!(s.selected.is_empty());
}

#[test]
fn every_sketcher_tool_is_on_the_toolbar_with_a_hint() {
    let mut h = sketching();
    for (icon, name, tool) in [
        (icons::CURSOR, "Select", ViewTool::Select),
        (icons::DOT, "Point", ViewTool::Point),
        (icons::LINE_SEGMENT, "Line", ViewTool::Line),
        (icons::LINE_SEGMENTS, "Polyline", ViewTool::Polyline),
        (icons::CIRCLE_HALF, "Arc", ViewTool::Arc),
        (icons::CIRCLE, "Circle", ViewTool::Circle),
        (icons::RECTANGLE, "Rectangle", ViewTool::Rect),
        (icons::CORNERS_IN, "Fillet", ViewTool::Fillet),
        (icons::SCISSORS, "Trim", ViewTool::Trim),
        (
            icons::ARROWS_OUT_LINE_HORIZONTAL,
            "Extend",
            ViewTool::Extend,
        ),
        (icons::ARROWS_SPLIT, "Split", ViewTool::Split),
        (
            icons::CUBE_TRANSPARENT,
            "External geometry",
            ViewTool::External,
        ),
    ] {
        h.click(&format!("{icon} {name}"));
        h.frame();
        h.frame();
        assert_eq!(h.viewports()[0].tool, tool);
        let hint = h.app.hint();
        assert!(h.has_label(hint), "{name}: {hint}");
    }
    assert_eq!(
        h.app.sketch.as_ref().unwrap().hint(ViewTool::Orbit),
        "Drag to orbit, shift-drag to pan, scroll to zoom."
    );
}

#[test]
fn closing_a_sketch_makes_it_a_step_then_pad_extrudes_it() {
    let mut h = sketching();
    with_line(&mut h);
    h.click("View sketch");
    h.frame();
    h.frame();
    assert_eq!(h.viewports()[0].view.view, StdView::Sketch);
    h.click("✔ Close");
    h.frame();
    assert!(matches!(h.app.ops[..], [CadOp::Sketch(_)]));
    assert_eq!(h.outstanding::<Regenerate>(), 1);
    h.reply::<Regenerate>(&built(Model {
        new_faces: vec![0],
        ..model("/cad/mesh/s", 1, 0, 0)
    }));
    h.frame();
    h.frame();
    assert!(h.viewports()[0].sketch.is_empty(), "back in Part Design");
    h.type_text(0, "8");
    h.frame();
    h.click("Pad");
    h.frame();
    assert_eq!(
        h.app.ops[1],
        CadOp::PushPull {
            after: 1,
            face: 0,
            distance: 8.0
        }
    );
}

#[test]
fn editing_a_sketch_replaces_its_step_and_cancel_keeps_it() {
    let mut h = sketching();
    with_line(&mut h);
    h.click("Part Design");
    h.frame();
    assert_eq!(h.app.ops.len(), 1, "switching workbench closes the sketch");
    h.frame();
    h.click("Edit sketch");
    h.frame();
    h.frame();
    assert_eq!(scene(&h).sketch.geos.len(), 1);
    let mut s = scene(&h).sketch;
    s.geos.push(Geo::new(Curve::Circle {
        center: [0.0, 0.0],
        radius: 5.0,
    }));
    send(&mut h, SketchEvent::Edited(s));
    h.click("✖ Cancel");
    h.frame();
    h.frame();
    let CadOp::Sketch(kept) = &h.app.ops[0] else {
        panic!()
    };
    assert_eq!(kept.geos.len(), 1, "cancel keeps the step as it was");
    h.click("Edit sketch");
    h.frame();
    h.frame();
    let mut s = scene(&h).sketch;
    s.geos.push(Geo::new(Curve::Point([1.0, 1.0])));
    send(&mut h, SketchEvent::Edited(s));
    h.click("✔ Close");
    h.frame();
    assert_eq!(h.app.ops.len(), 1, "replaced, not added");
    let CadOp::Sketch(kept) = &h.app.ops[0] else {
        panic!()
    };
    assert_eq!(kept.geos.len(), 2);
}

// ── Helpers ─────────────────────────────────────────────────────────────────

#[test]
fn export_paths_take_the_formats_extension() {
    assert_eq!(export_path("/a/b", ExportFormat::Step), "/a/b.step");
    assert_eq!(
        export_path(" /a/b.STL ", ExportFormat::FreeCad),
        "/a/b.FCStd"
    );
    assert_eq!(export_path("/a/b.stp", ExportFormat::Stl), "/a/b.stl");
}

#[test]
fn download_urls_are_percent_encoded() {
    assert_eq!(
        download_url("/home/ada/my part #2.step"),
        "/files/home/ada/my%20part%20%232.step"
    );
}

#[test]
fn lengths_parse_like_sketchup_takes_them() {
    use measure::{length, pair};
    assert_eq!(length("250"), Some(250.0));
    assert_eq!(length(" -30 "), Some(-30.0));
    assert_eq!(length("+12.5mm"), Some(12.5));
    assert_eq!(length("2m"), Some(2000.0));
    assert_eq!(length("40 cm"), Some(400.0));
    assert_eq!(length(".5"), Some(0.5));
    for bad in [
        "",
        "abc",
        "1e5",
        "5 km",
        "1.2.3",
        "-",
        ".",
        "12x",
        "99999999999",
    ] {
        assert_eq!(length(bad), None, "{bad:?}");
    }
    assert_eq!(pair("200,300"), Some((200.0, 300.0)));
    assert_eq!(pair("2m; 30cm"), Some((2000.0, 300.0)));
    assert_eq!(pair("200"), None);
    assert_eq!(pair("a,3"), None);
}

#[test]
fn numbers_are_shown_without_float_formatting() {
    use measure::{mm, thousands};
    assert_eq!(thousands(0), "0");
    assert_eq!(thousands(952_000), "952 000");
    assert_eq!(thousands(1_000_005), "1 000 005");
    assert_eq!(mm(-30.4), "-30 mm");
    assert_eq!(mm(0.2), "0 mm");
    assert_eq!(mm(1234.6), "1 235 mm");
}
