//! A viewport in sketch mode, driven through a real offscreen `egui::Context`: solving what
//! the app sends, the drawing and editing tools, picking, dragging, and FreeCAD's solver
//! messages.
//!
//! The camera faces a new sketch with the origin in the middle of the view and X to the right,
//! so a click at the centre lands on the origin and one to the right of it on +X.

use ccosel_abi::event::{decode_batch, event_kind};
use ccosel_abi::view3d::{
    NO_FACE, Render, StdView, ViewCommand, ViewTool, Viewport, decode_sketch_event,
};
use ccosel_abi::{Cmd, Encoder, RespRecord, Vec2};
use ccosel_host::Replayer;
use ccosel_proto::sketch::{
    Constraint, Curve, Geo, Item, Plane, PointRef, Pos, Sketch, SketchEvent, SketchScene,
};

const VIEW: u64 = 9;

fn frame_buf(sketch: &Sketch, selected: &[Item], tool: ViewTool, value: f32, seq: u32) -> Vec<u8> {
    let scene = postcard::to_allocvec(&SketchScene {
        sketch: sketch.clone(),
        selected: selected.to_vec(),
    })
    .unwrap();
    let mut e = Encoder::new();
    e.push(&Cmd::Viewport3d {
        id: VIEW,
        view: Viewport {
            size: Vec2::new(400.0, 300.0),
            mesh: "",
            tool,
            anchor: None,
            path: &[],
            preview: None,
            selected: NO_FACE,
            render: Render::Local,
            sketch: &scene,
            value,
            view: ViewCommand {
                view: if seq > 0 { StdView::Top } else { StdView::None },
                seq,
            },
        },
    });
    e.as_slice().to_vec()
}

struct Rig {
    ctx: egui::Context,
    r: Replayer,
    shapes: Vec<egui::epaint::ClippedShape>,
    rect: egui::Rect,
    /// The app's copy, updated from every `Edited` as a real app would.
    sketch: Sketch,
    selected: Vec<Item>,
    tool: ViewTool,
    value: f32,
    picks: Vec<Option<Item>>,
    edits: usize,
}

impl Rig {
    fn new(sketch: Sketch, tool: ViewTool) -> Self {
        let mut rig = Self {
            ctx: egui::Context::default(),
            r: Replayer::new(),
            shapes: Vec::new(),
            rect: egui::Rect::NOTHING,
            sketch,
            selected: Vec::new(),
            tool,
            value: 0.0,
            picks: Vec::new(),
            edits: 0,
        };
        rig.frame(vec![]);
        rig
    }

    fn frame(&mut self, events: Vec<egui::Event>) {
        let buf = frame_buf(&self.sketch, &self.selected, self.tool, self.value, 0);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            events,
            ..Default::default()
        };
        let mut out: Vec<RespRecord> = Vec::new();
        let r = &mut self.r;
        let mut full = self.ctx.run_ui(input, |ui| {
            out = r.replay(ui, 1, &buf).expect("valid");
        });
        full.textures_delta.clear();
        self.shapes = full.shapes;
        let rec = out.iter().find(|x| x.local_id == VIEW).unwrap();
        self.rect = egui::Rect::from_min_max(
            egui::pos2(rec.rect[0], rec.rect[1]),
            egui::pos2(rec.rect[2], rec.rect[3]),
        );
        // Deliver what the shell said, as the real shell does before the app's next frame.
        if let Some(batch) = self.r.take_view_events() {
            for e in decode_batch(&batch).unwrap() {
                assert_eq!(e.kind, event_kind::SKETCH);
                let (id, body) = decode_sketch_event(e.payload).unwrap();
                assert_eq!(id, VIEW);
                match postcard::from_bytes::<SketchEvent>(body).unwrap() {
                    SketchEvent::Edited(s) => {
                        self.sketch = s;
                        self.edits += 1;
                    }
                    SketchEvent::Picked(p) => self.picks.push(p),
                }
            }
        }
    }

    fn at(&self, dx: f32, dy: f32) -> egui::Pos2 {
        self.rect.center() + egui::vec2(dx, dy)
    }

    fn click(&mut self, p: egui::Pos2) {
        self.frame(vec![egui::Event::PointerMoved(p)]);
        self.frame(vec![press(p, true)]);
        self.frame(vec![press(p, false)]);
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
}

fn press(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::default(),
    }
}

fn empty() -> Sketch {
    Sketch::new(Plane::GROUND)
}

fn line_sketch() -> Sketch {
    let mut s = empty();
    s.geos
        .push(Geo::new(Curve::Line([-300.0, -500.0], [300.0, -500.0])));
    s
}

#[test]
fn an_unsolved_sketch_comes_back_solved_and_then_settles() {
    let mut s = empty();
    s.geos.push(Geo::new(Curve::Line([0.0, 0.0], [100.0, 7.0])));
    s.constraints.push(Constraint::Horizontal(0));
    let rig = Rig::new(s, ViewTool::Select);
    assert_eq!(rig.edits, 1, "solved once");
    let Curve::Line(a, b) = rig.sketch.geos[0].curve else {
        panic!()
    };
    assert!((a[1] - b[1]).abs() < 1e-9);
    let mut rig = rig;
    rig.frame(vec![]);
    rig.frame(vec![]);
    assert_eq!(
        rig.edits, 1,
        "the solved sketch echoed back changes nothing"
    );
    assert!(rig.texts().iter().any(|t| t == "3 degrees of freedom"));
    assert!(
        rig.texts().iter().any(|t| t == "H"),
        "the constraint is labelled"
    );
}

#[test]
fn the_line_tool_draws_from_the_origin_level_and_attached() {
    let mut rig = Rig::new(empty(), ViewTool::Line);
    let (a, b) = (rig.at(0.0, 0.0), rig.at(60.0, 0.0));
    rig.click(a);
    assert_eq!(rig.edits, 0, "one click is only the start");
    rig.click(b);
    assert_eq!(rig.edits, 1);
    let s = &rig.sketch;
    assert_eq!(s.geos.len(), 1);
    assert!(s.constraints.contains(&Constraint::Coincident(
        PointRef::new(0, Pos::Start),
        PointRef::ORIGIN
    )));
    assert!(s.constraints.contains(&Constraint::Horizontal(0)));
    let Curve::Line(p, q) = s.geos[0].curve else {
        panic!()
    };
    assert!(q[0] > p[0] + 100.0, "to the right is +X: {p:?} {q:?}");
}

#[test]
fn rectangles_circles_arcs_and_points() {
    let mut rig = Rig::new(empty(), ViewTool::Rect);
    rig.click(rig.at(-40.0, -30.0));
    rig.click(rig.at(40.0, 30.0));
    assert_eq!(rig.sketch.geos.len(), 4);
    assert!(rig.sketch.constraints.len() >= 8);

    rig.tool = ViewTool::Circle;
    rig.click(rig.at(100.0, 0.0));
    rig.click(rig.at(120.0, 0.0));
    assert!(matches!(rig.sketch.geos[4].curve, Curve::Circle { .. }));

    rig.tool = ViewTool::Arc;
    rig.click(rig.at(-120.0, 80.0));
    rig.click(rig.at(-100.0, 80.0));
    rig.click(rig.at(-120.0, 60.0));
    assert!(matches!(rig.sketch.geos[5].curve, Curve::Arc { .. }));

    rig.tool = ViewTool::Point;
    rig.click(rig.at(150.0, 100.0));
    assert!(matches!(rig.sketch.geos[6].curve, Curve::Point(_)));
    assert_eq!(rig.edits, 4);
}

#[test]
fn a_polyline_closes_on_its_first_point() {
    let mut rig = Rig::new(empty(), ViewTool::Polyline);
    let pts = [
        rig.at(20.0, 20.0),
        rig.at(80.0, 20.0),
        rig.at(80.0, -40.0),
        rig.at(20.0, 20.0),
    ];
    for p in pts {
        rig.click(p);
    }
    assert_eq!(rig.sketch.geos.len(), 3);
    assert!(rig.sketch.constraints.contains(&Constraint::Coincident(
        PointRef::new(2, Pos::End),
        PointRef::new(0, Pos::Start)
    )));
    // A new polyline starts afresh; clicking its last point again ends it.
    let (a, b) = (rig.at(-100.0, 0.0), rig.at(-60.0, 50.0));
    rig.click(a);
    rig.click(b);
    rig.click(b);
    rig.click(rig.at(-150.0, -80.0));
    assert_eq!(rig.sketch.geos.len(), 4, "the third click finished it");
}

#[test]
fn escape_or_right_click_abandons_a_shape_half_drawn() {
    let mut rig = Rig::new(empty(), ViewTool::Line);
    rig.click(rig.at(10.0, 10.0));
    let p = rig.at(10.0, 10.0);
    rig.frame(vec![egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::default(),
    }]);
    let _ = p;
    rig.click(rig.at(90.0, 40.0));
    assert_eq!(rig.edits, 0, "the second click started a new line");
}

#[test]
fn select_picks_points_curves_labels_and_nothing() {
    let mut s = line_sketch();
    s.constraints.push(Constraint::Horizontal(0));
    let mut rig = Rig::new(s, ViewTool::Select);
    // Find the line on screen: it is level, half a metre below the origin.
    let mut hit = None;
    for dy in 0..140 {
        rig.click(rig.at(0.0, dy as f32));
        if let Some(Some(Item::Geo(0))) = rig.picks.last() {
            hit = Some(dy);
            break;
        }
    }
    let dy = hit.expect("the line is somewhere below the origin") as f32;
    rig.selected = vec![Item::Geo(0)];
    rig.frame(vec![]);
    rig.click(rig.at(-150.0, -120.0));
    assert_eq!(rig.picks.last(), Some(&None), "empty space clears");
    rig.click(rig.at(0.0, 0.0));
    assert_eq!(rig.picks.last(), Some(&Some(Item::Point(PointRef::ORIGIN))));
    // The line's end, far to the right on the same row.
    let mut end = None;
    for dx in 0..200 {
        rig.click(rig.at(dx as f32, dy));
        if let Some(Some(Item::Point(p))) = rig.picks.last() {
            end = Some(*p);
            break;
        }
    }
    assert_eq!(end, Some(PointRef::new(0, Pos::End)));
}

#[test]
fn dragging_a_point_moves_it_and_tells_the_app_on_release() {
    let mut rig = Rig::new(line_sketch(), ViewTool::Select);
    // Find the line's end by hovering.
    let mut from = None;
    'outer: for dy in 0..140 {
        for dx in (0..200).step_by(2) {
            rig.click(rig.at(dx as f32, dy as f32));
            if let Some(Some(Item::Point(p))) = rig.picks.last()
                && *p == PointRef::new(0, Pos::End)
            {
                from = Some(rig.at(dx as f32, dy as f32));
                break 'outer;
            }
        }
    }
    let from = from.expect("found the end");
    let before = rig.edits;
    rig.frame(vec![egui::Event::PointerMoved(from), press(from, true)]);
    for k in 1..=6 {
        rig.frame(vec![egui::Event::PointerMoved(
            from + egui::vec2(0.0, -8.0 * k as f32),
        )]);
    }
    assert_eq!(rig.edits, before, "nothing reported mid-drag");
    rig.frame(vec![press(from + egui::vec2(0.0, -48.0), false)]);
    assert_eq!(rig.edits, before + 1);
    let Curve::Line(_, b) = rig.sketch.geos[0].curve else {
        panic!()
    };
    assert!(b[1] > -500.0 + 100.0, "moved up: {b:?}");
}

#[test]
fn trim_split_extend_and_fillet_from_clicks() {
    let mut s = empty();
    // A plus sign, drawn at a size that fills the view.
    s.geos
        .push(Geo::new(Curve::Line([-1000.0, 0.0], [1000.0, 0.0])));
    s.geos
        .push(Geo::new(Curve::Line([0.0, -1000.0], [0.0, 1000.0])));
    let mut rig = Rig::new(s, ViewTool::Trim);
    // Right of the crossing, on the horizontal line.
    let right = rig.at(60.0, 0.0);
    rig.click(right);
    assert_eq!(rig.edits, 1);
    let Curve::Line(_, b) = rig.sketch.geos[0].curve else {
        panic!()
    };
    assert!(b[0].abs() < 1.0, "cut back to the crossing: {b:?}");

    rig.tool = ViewTool::Extend;
    rig.click(rig.at(-60.0, 0.0));
    assert!(
        rig.texts().iter().any(|t| t.contains("would not meet")),
        "{:?}",
        rig.texts()
    );

    rig.tool = ViewTool::Split;
    rig.click(rig.at(0.0, 50.0));
    assert_eq!(rig.sketch.geos.len(), 3);

    rig.tool = ViewTool::Trim;
    rig.click(rig.at(150.0, 150.0));
    assert!(rig.texts().iter().any(|t| t == "Click on a curve."));

    // Round the corner where the vertical line's lower piece meets the horizontal one.
    let mut corner = empty();
    corner
        .geos
        .push(Geo::new(Curve::Line([0.0, 0.0], [1000.0, 0.0])));
    corner
        .geos
        .push(Geo::new(Curve::Line([0.0, 0.0], [0.0, 1000.0])));
    let mut rig = Rig::new(corner, ViewTool::Fillet);
    rig.value = 200.0;
    rig.click(rig.at(0.0, 0.0));
    assert!(
        matches!(rig.sketch.geos.get(2).map(|g| g.curve), Some(Curve::Arc { radius, .. }) if (radius - 200.0).abs() < 1e-6)
    );
    rig.value = 1e6;
    rig.frame(vec![]);
    rig.click(rig.at(-100.0, -100.0));
    assert!(
        rig.texts().iter().any(|t| t.contains("corner")),
        "{:?}",
        rig.texts()
    );
}

#[test]
fn external_geometry_needs_a_model_edge() {
    let mut rig = Rig::new(empty(), ViewTool::External);
    rig.click(rig.at(0.0, 0.0));
    assert!(
        rig.texts()
            .iter()
            .any(|t| t == "Click an edge of the model.")
    );
}

#[test]
fn solver_messages_like_freecads() {
    let mut s = empty();
    s.geos.push(Geo::new(Curve::Line([0.0, 0.0], [100.0, 0.0])));
    s.constraints = vec![
        Constraint::Coincident(PointRef::new(0, Pos::Start), PointRef::ORIGIN),
        Constraint::Horizontal(0),
        Constraint::Distance(0, 100.0),
    ];
    let rig = Rig::new(s.clone(), ViewTool::Select);
    assert!(rig.texts().iter().any(|t| t == "Fully constrained"));
    assert!(rig.texts().iter().any(|t| t == "100 mm"));

    s.constraints.push(Constraint::Horizontal(0));
    let rig = Rig::new(s.clone(), ViewTool::Select);
    assert!(
        rig.texts().iter().any(|t| t == "Redundant constraints: 4"),
        "{:?}",
        rig.texts()
    );

    s.constraints.pop();
    s.constraints.push(Constraint::Vertical(0));
    let rig = Rig::new(s.clone(), ViewTool::Select);
    assert!(
        rig.texts()
            .iter()
            .any(|t| t.starts_with("Conflicting constraints:")),
        "{:?}",
        rig.texts()
    );

    s.constraints.pop();
    s.constraints.pop();
    let rig = Rig::new(s, ViewTool::Select);
    assert!(rig.texts().iter().any(|t| t == "1 degree of freedom"));
}

#[test]
fn a_sketch_that_does_not_make_sense_is_reported() {
    let mut s = empty();
    s.constraints.push(Constraint::Horizontal(5));
    let rig = Rig::new(s, ViewTool::Select);
    assert!(rig.texts().iter().any(|t| t.contains("refers to geometry")));
}

#[test]
fn a_standard_view_is_applied_once_per_request() {
    let ctx = egui::Context::default();
    let mut r = Replayer::new();
    for seq in [1, 1, 2] {
        let buf = frame_buf(&empty(), &[], ViewTool::Select, 0.0, seq);
        let mut full = ctx.run_ui(Default::default(), |ui| {
            r.replay(ui, 1, &buf).unwrap();
        });
        full.textures_delta.clear();
    }
}
