use ccosel_proto::account::Account;
use ccosel_proto::cad::RegenStatus;
use ccosel_sdk::testing::{Harness, rpc_error};
use ccosel_sdk::view3d::Snap;

use super::*;

const UP: [f32; 3] = [0.0, 0.0, 1.0];

fn model(mesh: &str, faces: u32, solids: u32, volume: u64) -> Model {
    Model {
        mesh: mesh.to_owned(),
        triangles: faces * 2,
        faces,
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

fn status_line(h: &Harness<Modeller>) -> String {
    h.labels()[1].clone()
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
    h.reply::<Regenerate>(&built(model("/cad/mesh/a", 1, 0, 0)));
    h.frame();
    h
}

#[test]
fn it_opens_ready_to_draw_a_rectangle() {
    let h = fresh();
    assert_eq!(h.app.tool, ViewTool::Rect);
    assert!(h.has_label("Click the first corner, on the ground or a face."));
    assert!(h.has_label("Draw a rectangle on the ground to start."));
    let view = &h.viewports()[0];
    assert_eq!(view.mesh, "");
    assert_eq!(view.tool, ViewTool::Rect);
    assert_eq!(view.anchor, None);
    assert_eq!(h.outstanding::<Regenerate>(), 0, "nothing to build yet");
    assert!(!h.has_button("STEP"), "nothing to save yet");
    assert_eq!(h.app.wants_repaint_after_ms(), REPAINT_ON_INPUT_ONLY);
}

#[test]
fn two_clicks_draw_a_rectangle_and_the_server_builds_it() {
    let mut h = fresh();
    h.view_click(NO_FACE, [10.0, 20.0, 0.0], UP, Snap::Free);
    h.frame();
    // Gestures are handled once the frame's widgets are down; the next frame shows them.
    h.frame();
    assert!(h.has_label("Click the opposite corner."));
    assert_eq!(
        h.viewports()[0].anchor,
        Some(Anchor {
            point: [10.0, 20.0, 0.0],
            normal: UP
        })
    );

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
    assert_eq!(h.app.anchor, None, "ready for the next shape");
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
fn a_rebuild_in_progress_is_polled_until_it_finishes() {
    let mut h = fresh();
    rectangle(&mut h, [0.0; 3], [10.0, 10.0, 0.0]);
    h.reply::<Regenerate>(&running());
    h.frame();
    assert!(h.has_label("Building…"));
    // The unfinished answer was dropped, so the next frame asks again.
    h.frame();
    assert_eq!(h.outstanding::<Regenerate>(), 1);
    h.reply::<Regenerate>(&built(model("/cad/mesh/b", 1, 0, 0)));
    h.frame();
    assert_eq!(h.viewports()[0].mesh, "/cad/mesh/b");
    assert_eq!(h.app.wants_repaint_after_ms(), REPAINT_ON_INPUT_ONLY);
}

#[test]
fn a_push_pull_becomes_a_step_on_the_shown_model_and_previews_until_rebuilt() {
    let mut h = with_a_square();
    h.click("Push/Pull");
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
    h.frame();
    assert!(h.has_label("100 mm"));

    h.reply::<Regenerate>(&built(model("/cad/mesh/cube", 6, 1, 1_000_000)));
    h.frame();
    assert_eq!(h.viewports()[0].preview, None);
    assert_eq!(h.viewports()[0].mesh, "/cad/mesh/cube");
    assert_eq!(status_line(&h), "6 faces · 1 solid · 1 000 000 mm³");
}

#[test]
fn a_push_pull_while_a_rebuild_is_pending_refers_to_the_model_on_screen() {
    let mut h = with_a_square();
    h.view_push_pull(0, UP, 50.0);
    h.frame();
    // The second push arrives before the first has been rebuilt: its face is still one of the
    // one-step model's.
    h.view_push_pull(0, UP, 20.0);
    h.frame();
    assert_eq!(
        h.app.ops[2],
        CadOp::PushPull {
            after: 1,
            face: 0,
            distance: 20.0
        }
    );
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
    assert_eq!(
        h.viewports()[0].preview,
        Some(Extrude {
            face: 0,
            distance: 2000.0
        })
    );
}

#[test]
fn typing_width_and_height_remakes_the_last_rectangle_the_same_way_round() {
    let mut h = fresh();
    // Dragged towards -X and +Y.
    rectangle(&mut h, [0.0; 3], [-10.0, 10.0, 0.0]);
    h.type_text(0, "300,200");
    h.frame();
    h.click("Apply");
    h.frame();
    let CadOp::Polygon { points, .. } = &h.app.ops[0] else {
        panic!("a polygon")
    };
    let xs: Vec<f32> = points.iter().map(|p| p[0]).collect();
    let ys: Vec<f32> = points.iter().map(|p| p[1]).collect();
    assert_eq!(xs.iter().copied().fold(f32::INFINITY, f32::min), -300.0);
    assert_eq!(ys.iter().copied().fold(f32::NEG_INFINITY, f32::max), 200.0);

    // Again, now that it is 300 × 200: still the same corner, still the same direction.
    h.type_text(0, "1m;50");
    h.frame();
    h.click("Apply");
    h.frame();
    let CadOp::Polygon { points, .. } = &h.app.ops[0] else {
        panic!("a polygon")
    };
    assert!(points.contains(&[-1000.0, 50.0, 0.0]));
}

#[test]
fn a_measurement_that_makes_no_sense_says_so() {
    let mut h = fresh();
    h.type_text(0, "12");
    h.frame();
    h.click("Apply");
    h.frame();
    assert!(h.has_label("Type a length, or width,height for a rectangle."));

    rectangle(&mut h, [0.0; 3], [10.0, 10.0, 0.0]);
    h.frame();
    assert!(!h.has_label("Type a length, or width,height for a rectangle."));
    h.type_text(0, "abc");
    h.frame();
    h.click("Apply");
    h.frame();
    assert!(h.has_label("Type a length, or width,height for a rectangle."));
}

#[test]
fn the_line_tool_closes_an_outline_on_its_first_corner() {
    let mut h = fresh();
    h.click("Line");
    h.frame();
    assert!(h.has_label("Click to start an outline."));
    let corners = [[0.0, 0.0, 0.0], [50.0, 0.0, 0.0], [50.0, 30.0, 0.0]];
    for c in corners {
        h.view_click(NO_FACE, c, UP, Snap::Free);
        h.frame();
    }
    // A double click on a corner does not add it twice.
    h.view_click(NO_FACE, corners[2], UP, Snap::Endpoint);
    h.frame();
    h.frame();
    assert_eq!(h.viewports()[0].path, corners.to_vec());
    assert!(h.has_label("Click each corner; click the first again to close it."));

    h.view_click(NO_FACE, corners[0], UP, Snap::Endpoint);
    h.frame();
    assert_eq!(
        h.app.ops,
        vec![CadOp::Polygon {
            points: corners.to_vec(),
            normal: UP
        }]
    );
    h.frame();
    assert!(h.viewports()[0].path.is_empty());
}

#[test]
fn undo_cancels_a_half_drawn_shape_then_steps_back_and_redo_steps_forward() {
    let mut h = with_a_square();
    h.view_push_pull(0, UP, 10.0);
    h.frame();
    h.view_click(NO_FACE, [1.0, 1.0, 10.0], UP, Snap::OnFace);
    h.frame();
    assert!(h.app.anchor.is_some());

    h.click("Undo");
    h.frame();
    assert!(
        h.app.anchor.is_none(),
        "the half-drawn rectangle goes first"
    );
    assert_eq!(h.app.ops.len(), 2);

    h.click("Undo");
    h.frame();
    assert_eq!(h.app.ops.len(), 1);
    h.frame();
    assert_eq!(h.viewports()[0].preview, None);
    h.click("Redo");
    h.frame();
    assert_eq!(h.app.ops.len(), 2);
    h.click("Redo");
    h.frame();
    assert_eq!(h.app.ops.len(), 2, "nothing more to redo");

    // Undo everything: an empty model.
    h.click("Undo");
    h.frame();
    h.click("Undo");
    h.frame();
    h.frame();
    assert!(h.app.ops.is_empty());
    assert!(h.has_label("Draw a rectangle on the ground to start."));

    // Something new after an undo forgets what was undone.
    h.click("Redo");
    h.frame();
    h.click("Undo");
    h.frame();
    rectangle(&mut h, [0.0; 3], [3.0, 3.0, 0.0]);
    assert!(h.app.redo.is_empty());
}

#[test]
fn a_step_freecad_refuses_is_named_in_the_status_line() {
    let mut h = with_a_square();
    h.view_push_pull(0, UP, 10.0);
    h.frame();
    h.reply::<Regenerate>(&failed(1, "there is no face 0"));
    h.frame();
    h.frame();
    assert_eq!(
        status_line(&h),
        "Step 2 couldn't be done: there is no face 0. Undo to go back."
    );
    assert_eq!(h.viewports()[0].preview, None);
    assert_eq!(
        h.viewports()[0].mesh,
        "/cad/mesh/a",
        "the last good model stays up"
    );
}

#[test]
fn a_model_that_cannot_be_built_at_all_says_why() {
    let mut h = fresh();
    rectangle(&mut h, [0.0; 3], [10.0, 10.0, 0.0]);
    h.reply::<Regenerate>(&failed(u32::MAX, "the CAD worker failed: timed out"));
    h.frame();
    assert_eq!(
        status_line(&h),
        "Couldn't build the model: the CAD worker failed: timed out"
    );

    let mut h = fresh();
    rectangle(&mut h, [0.0; 3], [10.0, 10.0, 0.0]);
    h.fail::<Regenerate>(rpc_error::TIMEOUT);
    h.frame();
    assert!(status_line(&h).starts_with("Couldn't build the model: "));
}

#[test]
fn select_reports_the_face_to_the_viewport() {
    let mut h = with_a_square();
    h.click("Select");
    h.frame();
    assert!(h.has_label("Click a face to select it. Drag to orbit."));
    h.view_click(0, [5.0, 5.0, 0.0], UP, Snap::OnFace);
    h.frame();
    h.frame();
    assert_eq!(h.viewports()[0].selected, 0);
    h.click("Orbit");
    h.frame();
    h.frame();
    assert!(h.has_label("Drag to orbit, shift-drag to pan, scroll to zoom."));
    assert_eq!(h.viewports()[0].tool, ViewTool::Orbit);
}

#[test]
fn switching_tools_abandons_a_half_drawn_shape() {
    let mut h = fresh();
    h.view_click(NO_FACE, [0.0; 3], UP, Snap::Free);
    h.frame();
    h.click("Line");
    h.frame();
    h.frame();
    assert_eq!(h.viewports()[0].anchor, None);
    // Clicking the tool already chosen changes nothing.
    h.view_click(NO_FACE, [0.0; 3], UP, Snap::Free);
    h.frame();
    h.click("Line");
    h.frame();
    assert!(h.app.anchor.is_some());
}

#[test]
fn the_render_button_cycles_where_the_view_is_drawn() {
    let mut h = fresh();
    assert_eq!(h.viewports()[0].render, Render::Auto);
    h.click("Render: Auto");
    h.frame();
    h.frame();
    assert_eq!(h.viewports()[0].render, Render::Local);
    h.click("Render: Browser");
    h.frame();
    h.frame();
    assert_eq!(h.viewports()[0].render, Render::Server);
    h.click("Render: Server");
    h.frame();
    h.frame();
    assert_eq!(h.viewports()[0].render, Render::Auto);
}

#[test]
fn exports_go_to_the_home_folder_and_can_be_downloaded() {
    let mut h = with_a_square();
    h.reply::<WhoAmI>(&Account {
        login_enabled: true,
        name: Some("ada".to_owned()),
        account_url: None,
    });
    h.frame();
    h.frame();
    assert_eq!(h.text_fields()[1], "/home/ada/model");

    h.click("STEP");
    h.frame();
    h.frame();
    assert_eq!(h.outstanding::<ExportModel>(), 1);
    assert!(h.has_label("Saving…"));
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

    // A new step makes the saved file out of date: the link goes.
    h.click("Push/Pull");
    h.frame();
    h.view_push_pull(0, UP, 5.0);
    h.frame();
    h.frame();
    assert!(h.open_urls().is_empty());
}

#[test]
fn a_failed_export_says_so() {
    let mut h = with_a_square();
    h.reply::<WhoAmI>(&Account {
        login_enabled: false,
        name: None,
        account_url: None,
    });
    h.frame();
    assert_eq!(
        h.text_fields()[1],
        "/model",
        "no home folder when nobody is signed in"
    );
    h.click("STL");
    h.frame();
    h.fail::<ExportModel>(rpc_error::DENIED);
    h.frame();
    h.frame();
    assert!(
        h.labels()
            .iter()
            .any(|l| l.starts_with("Couldn't save /model.stl"))
    );
}

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

#[test]
fn the_server_installing_freecad_is_shown_while_the_first_model_waits() {
    let mut h = fresh();
    rectangle(&mut h, [0.0; 3], [10.0, 10.0, 0.0]);
    h.reply::<Regenerate>(&RegenStatus {
        note: "Installing FreeCAD on the server: downloading, 42% of 820 MB".to_owned(),
        ..running()
    });
    h.frame();
    h.frame();
    assert_eq!(
        status_line(&h),
        "Installing FreeCAD on the server: downloading, 42% of 820 MB"
    );
    h.reply::<Regenerate>(&built(model("/cad/mesh/a", 1, 0, 0)));
    h.frame();
    h.frame();
    assert_eq!(status_line(&h), "1 face");
}
