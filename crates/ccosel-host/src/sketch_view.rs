//! A viewport in sketch mode: FreeCAD's Sketcher, drawn and driven by the shell.
//!
//! The app owns the sketch and the selection and sends both every frame (`SketchScene`). The
//! shell keeps a working copy it can change at display rate: it solves what the app sent,
//! runs the drawing and editing tools against the pointer, and re-solves while a point is
//! dragged. Every finished change goes back to the app as `SketchEvent::Edited` with the whole
//! sketch; until the app sends that sketch back, the shell keeps showing its own copy, so
//! nothing flickers across the one-frame round trip.

use std::f64::consts::PI;
use std::hash::{Hash, Hasher};

use ccosel_abi::view3d::{ViewTool, Viewport};
use ccosel_proto::sketch::{
    Constraint, Curve, Item, Plane, PointRef, Pos, Sketch, SketchEvent, SketchScene,
};
use ccosel_sketch::edit::{self, Attach};
use ccosel_sketch::geom::{self, P2};
use ccosel_sketch::{Grab, Report};
use ccosel_view3d::{Camera, Mesh};

/// How close, in pixels, the pointer must be to snap to or pick something.
const PICK_PX: f32 = 8.0;

const INK: egui::Color32 = egui::Color32::from_rgb(0x11, 0x18, 0x27);
const DONE: egui::Color32 = egui::Color32::from_rgb(0x15, 0x80, 0x3d);
const CONSTRUCTION: egui::Color32 = egui::Color32::from_rgb(0x1d, 0x4e, 0xd8);
const EXTERNAL: egui::Color32 = egui::Color32::from_rgb(0xa2, 0x1c, 0xaf);
const SELECTED: egui::Color32 = egui::Color32::from_rgb(0xea, 0x58, 0x0c);
const HOVER: egui::Color32 = egui::Color32::from_rgb(0x0e, 0xa5, 0xe9);
const BAD: egui::Color32 = egui::Color32::from_rgb(0xdc, 0x26, 0x26);
const WARN: egui::Color32 = egui::Color32::from_rgb(0xd9, 0x77, 0x06);
const AXIS_X: egui::Color32 = egui::Color32::from_rgb(0xdc, 0x26, 0x26);
const AXIS_Y: egui::Color32 = egui::Color32::from_rgb(0x16, 0xa3, 0x4a);

/// A tool part way through: the clicks it has had so far.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Pending {
    #[default]
    Idle,
    /// The first point of a line or rectangle, or the centre of a circle or arc.
    First(P2, Attach),
    /// An arc's centre and start.
    ArcStart(P2, Attach, P2, Attach),
    /// A polyline under way: its first point, and the last line drawn.
    Chain { first: PointRef, last: i32 },
}

/// What the pointer is over, in sketch coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Hover {
    at: P2,
    attach: Attach,
}

#[derive(Default)]
pub(crate) struct SketchState {
    /// A fingerprint of the bytes the app last sent, so they are decoded only when they change.
    sent: u64,
    /// The app's selection.
    selected: Vec<Item>,
    /// What is shown and edited.
    working: Option<Sketch>,
    /// An edit reported to the app and not yet echoed back.
    awaiting: Option<Sketch>,
    report: Report,
    tool: Pending,
    drag: Option<(Grab, P2)>,
    hover: Option<Hover>,
    /// A tool's complaint ("that radius is too big"), and the frame it was made in.
    message: Option<(String, u64)>,
    /// Where each constraint's label was drawn last frame, for picking.
    labels: Vec<(egui::Rect, u32)>,
    /// The plane last shown, to face a new sketch when it opens.
    plane: Option<Plane>,
    pub(crate) error: Option<String>,
}

fn fingerprint(bytes: &[u8]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut h);
    h.finish()
}

/// Whether two sketches are the same to within rounding, so solving an already solved sketch
/// never ping-pongs an edit back and forth with the app.
fn same(a: &Sketch, b: &Sketch) -> bool {
    if a.constraints != b.constraints || a.geos.len() != b.geos.len() {
        return false;
    }
    let close = |x: &[f64], y: &[f64]| x.iter().zip(y).all(|(p, q)| (p - q).abs() < 1e-7);
    a.geos.iter().zip(&b.geos).all(|(g, h)| {
        g.construction == h.construction
            && g.fixed == h.fixed
            && match (g.curve, h.curve) {
                (Curve::Point(p), Curve::Point(q)) => close(&p, &q),
                (Curve::Line(a1, b1), Curve::Line(a2, b2)) => close(&a1, &a2) && close(&b1, &b2),
                (
                    Curve::Circle {
                        center: c1,
                        radius: r1,
                    },
                    Curve::Circle {
                        center: c2,
                        radius: r2,
                    },
                ) => close(&c1, &c2) && close(&[r1], &[r2]),
                (
                    Curve::Arc {
                        center: c1,
                        radius: r1,
                        start: s1,
                        end: e1,
                    },
                    Curve::Arc {
                        center: c2,
                        radius: r2,
                        start: s2,
                        end: e2,
                    },
                ) => close(&c1, &c2) && close(&[r1, s1, e1], &[r2, s2, e2]),
                _ => false,
            }
    })
}

/// A world point as `f32`, for the camera.
fn to_f32(p: [f64; 3]) -> [f32; 3] {
    p.map(|c| c as f32)
}

/// Point a camera straight at a plane, keeping its target on it and its distance.
pub(crate) fn face(cam: &mut Camera, plane: &Plane) {
    let n = plane.normal;
    // Looking straight down or up, the azimuth only turns the picture: choose it so the
    // sketch's X runs to the right, as in FreeCAD. Otherwise look along the normal.
    cam.state.azimuth = if n[2].abs() > 0.99 {
        let x = plane.x_dir;
        (-x[0]).atan2(x[1]) as f32
    } else {
        n[1].atan2(n[0]) as f32
    };
    cam.state.elevation = (n[2].clamp(-1.0, 1.0).asin() as f32).clamp(-1.55, 1.55);
    let t = plane.to_sketch(cam.state.target.map(f64::from));
    cam.state.target = to_f32(plane.to_world(t));
}

fn fmt_len(v: f64) -> String {
    let s = format!("{v:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    s.to_owned()
}

/// The text FreeCAD would show for a constraint, if it shows any.
fn label(c: &Constraint) -> Option<String> {
    Some(match *c {
        Constraint::Coincident(..) => return None,
        Constraint::PointOnObject(..) => "on".to_owned(),
        Constraint::Horizontal(_) | Constraint::HorizontalPoints(..) => "H".to_owned(),
        Constraint::Vertical(_) | Constraint::VerticalPoints(..) => "V".to_owned(),
        Constraint::Parallel(..) => "//".to_owned(),
        Constraint::Perpendicular(..) => "_|_".to_owned(),
        Constraint::Tangent(..) => "T".to_owned(),
        Constraint::Equal(..) => "=".to_owned(),
        Constraint::Symmetric(..) | Constraint::SymmetricPoint(..) => "><".to_owned(),
        Constraint::Distance(_, v)
        | Constraint::DistancePoints(_, _, v)
        | Constraint::DistancePointLine(_, _, v) => format!("{} mm", fmt_len(v)),
        Constraint::DistanceX(_, _, v) => format!("X {}", fmt_len(v)),
        Constraint::DistanceY(_, _, v) => format!("Y {}", fmt_len(v)),
        Constraint::Radius(_, v) => format!("R {}", fmt_len(v)),
        Constraint::Diameter(_, v) => format!("Ø {}", fmt_len(v)),
        Constraint::Angle(_, v) | Constraint::AngleBetween(_, _, v) => {
            format!("{}°", fmt_len(v * 180.0 / PI))
        }
    })
}

/// Where on the sketch a constraint's label belongs.
fn anchor(sk: &Sketch, c: &Constraint) -> Option<P2> {
    let mid_of = |g: i32| -> Option<P2> {
        Some(match geom::curve(sk, g)? {
            Curve::Line(a, b) if g >= 0 => geom::lerp(a, b, 0.5),
            Curve::Line(..) => [0.0, 0.0],
            Curve::Point(p) => p,
            Curve::Circle { center, radius } => geom::polar(center, radius, PI / 4.0),
            Curve::Arc {
                center,
                radius,
                start,
                end,
            } => geom::polar(center, radius, (start + end) / 2.0),
        })
    };
    let pt = |r: PointRef| geom::point(sk, r);
    match *c {
        Constraint::DistanceX(a, b, _) | Constraint::DistanceY(a, b, _)
            if a == PointRef::ORIGIN =>
        {
            pt(b)
        }
        Constraint::HorizontalPoints(a, b)
        | Constraint::VerticalPoints(a, b)
        | Constraint::DistancePoints(a, b, _)
        | Constraint::DistanceX(a, b, _)
        | Constraint::DistanceY(a, b, _)
        | Constraint::Symmetric(a, b, _)
        | Constraint::SymmetricPoint(a, b, _) => Some(geom::lerp(pt(a)?, pt(b)?, 0.5)),
        Constraint::PointOnObject(a, _) => pt(a),
        Constraint::DistancePointLine(a, g, _) => {
            let p = pt(a)?;
            let foot = edit::closest_on(sk, g, p).unwrap_or(p);
            Some(geom::lerp(p, foot, 0.5))
        }
        _ => mid_of(*c.geos().first()?),
    }
}

impl SketchState {
    /// Take in what the app sent. Returns an event if solving it changed it.
    fn receive(&mut self, bytes: &[u8]) -> Option<SketchEvent> {
        let print = fingerprint(bytes);
        if print == self.sent {
            return None;
        }
        self.sent = print;
        let scene: SketchScene = match postcard::from_bytes(bytes) {
            Ok(s) => s,
            Err(_) => {
                self.error = Some("the sketch did not decode".to_owned());
                return None;
            }
        };
        if !scene.sketch.is_valid() {
            self.error = Some("the sketch refers to geometry it does not have".to_owned());
            return None;
        }
        self.error = None;
        self.selected = scene.selected;
        // The app has caught up with an edit, or overridden it; either way its word goes.
        self.awaiting = None;
        let mut solved = scene.sketch.clone();
        self.report = ccosel_sketch::solve(&mut solved);
        let changed = self.report.converged && !same(&solved, &scene.sketch);
        self.working = Some(solved.clone());
        if changed {
            self.awaiting = Some(solved.clone());
            return Some(SketchEvent::Edited(solved));
        }
        None
    }

    /// The sketch as edited by a tool: solve it, show it, tell the app.
    fn commit(&mut self, mut sk: Sketch) -> SketchEvent {
        self.report = ccosel_sketch::solve(&mut sk);
        self.working = Some(sk.clone());
        self.awaiting = Some(sk.clone());
        SketchEvent::Edited(sk)
    }

    fn fail(&mut self, msg: &str, frame: u64) {
        self.message = Some((msg.to_owned(), frame));
    }

    /// One frame of a viewport in sketch mode. Returns the sketch events it produced.
    #[allow(clippy::too_many_arguments)] // the viewport's whole context, passed once
    pub(crate) fn show(
        &mut self,
        ui: &egui::Ui,
        r: &egui::Response,
        rect: egui::Rect,
        cam: &mut Camera,
        mesh: Option<&Mesh>,
        view: &Viewport<'_>,
        frame: u64,
    ) -> Vec<SketchEvent> {
        let mut out: Vec<SketchEvent> = self.receive(view.sketch).into_iter().collect();
        let Some(mut sk) = self.working.clone() else {
            return out;
        };
        if self.plane != Some(sk.plane) {
            if self.plane.is_none() {
                face(cam, &sk.plane);
            }
            self.plane = Some(sk.plane);
        }
        let (w, h) = (rect.width(), rect.height());
        let plane = sk.plane;

        // The pointer on the sketch plane, and how many sketch units a pixel is there.
        let pointer = r
            .hover_pos()
            .or(r.interact_pointer_pos())
            .map(|p| p - rect.min.to_vec2());
        let on_plane_at = |p: egui::Vec2| -> Option<(P2, f64)> {
            let ray = cam.ray(p.x, p.y, w, h);
            let n = plane.normal.map(|c| c as f32);
            let o = plane.origin.map(|c| c as f32);
            let denom = ray.dir[0] * n[0] + ray.dir[1] * n[1] + ray.dir[2] * n[2];
            if denom.abs() < 1e-9 {
                return None;
            }
            let t = ((o[0] - ray.origin[0]) * n[0]
                + (o[1] - ray.origin[1]) * n[1]
                + (o[2] - ray.origin[2]) * n[2])
                / denom;
            (t > 0.0).then(|| {
                let world = ray.at(t);
                let depth = cam.to_view(world)[2];
                let per_px = 2.0 * depth * (cam.state.fov_y * 0.5).tan() / h;
                (
                    plane.to_sketch(world.map(f64::from)),
                    f64::from(per_px * PICK_PX),
                )
            })
        };
        let on_plane = pointer.and_then(|p| on_plane_at(p.to_vec2()));
        let hover_at = |sk: &Sketch, (p, tol): (P2, f64)| -> Hover {
            if let Some(q) = geom::nearest_point(sk, p, tol) {
                Hover {
                    at: geom::point(sk, q).unwrap_or(p),
                    attach: Attach::Point(q),
                }
            } else if let Some(g) = geom::nearest_curve(sk, p, tol) {
                Hover {
                    at: edit::closest_on(sk, g, p).unwrap_or(p),
                    attach: Attach::Curve(g),
                }
            } else {
                Hover {
                    at: p,
                    attach: Attach::Nothing,
                }
            }
        };

        self.hover = on_plane.map(|at| hover_at(&sk, at));

        let cancel = r.secondary_clicked()
            || (r.contains_pointer() && ui.input(|i| i.key_pressed(egui::Key::Escape)));
        if cancel {
            // A polyline in progress keeps what has been drawn of it; only the next segment
            // is abandoned.
            self.tool = Pending::Idle;
        }

        // Dragging in Select: a point or curve follows the pointer, solved live.
        if view.tool == ViewTool::Select {
            // What is grabbed is what was under the pointer when the button went down, not
            // where egui decided it was a drag, a few pixels on.
            let pressed_at = ui
                .input(|i| i.pointer.press_origin())
                .map(|p| p - rect.min)
                .and_then(on_plane_at);
            if r.drag_started_by(egui::PointerButton::Primary)
                && let Some((p, tol)) = pressed_at
            {
                let h = hover_at(&sk, (p, tol));
                self.drag = match h.attach {
                    Attach::Point(q) if q.geo >= 0 => Some((Grab::Point(q), p)),
                    Attach::Curve(g) if g >= 0 => Some((Grab::Edge(g), p)),
                    _ => None,
                };
            }
            if let (Some((grab, last)), Some((p, _))) = (self.drag, on_plane)
                && r.dragged_by(egui::PointerButton::Primary)
            {
                self.report = ccosel_sketch::drag(&mut sk, grab, last, p);
                self.working = Some(sk.clone());
                self.drag = Some((grab, p));
            }
            if r.drag_stopped_by(egui::PointerButton::Primary) && self.drag.take().is_some() {
                out.push(self.commit(sk.clone()));
            }
        } else {
            self.drag = None;
        }

        if r.clicked()
            && let (Some(hover), Some((p, tol))) = (self.hover, on_plane)
            && let Some(e) = self.click(
                &mut sk, view, hover, p, tol, cam, mesh, pointer, rect, frame,
            )
        {
            out.push(e);
        }

        self.paint(ui, rect, cam, view, frame);
        out
    }

    #[allow(clippy::too_many_arguments)] // everything a click needs, in one place
    fn click(
        &mut self,
        sk: &mut Sketch,
        view: &Viewport<'_>,
        hover: Hover,
        raw: P2,
        tol: f64,
        cam: &Camera,
        mesh: Option<&Mesh>,
        pointer: Option<egui::Pos2>,
        rect: egui::Rect,
        frame: u64,
    ) -> Option<SketchEvent> {
        let at = hover.at;
        let attach = hover.attach;
        match (view.tool, self.tool) {
            (ViewTool::Select | ViewTool::Orbit | ViewTool::PushPull, _) => {
                let label = pointer.and_then(|p| {
                    let screen = rect.min + p.to_vec2();
                    self.labels
                        .iter()
                        .find(|(r, _)| r.expand(2.0).contains(screen))
                        .map(|(_, c)| Item::Constraint(*c))
                });
                let item = label.or(match attach {
                    Attach::Point(q) => Some(Item::Point(q)),
                    Attach::Curve(g) => Some(Item::Geo(g)),
                    Attach::Nothing => None,
                });
                return Some(SketchEvent::Picked(item));
            }
            (ViewTool::Point, _) => {
                let g = edit::add_point(sk, at);
                edit::attach(sk, PointRef::new(g, Pos::Start), attach);
            }
            (ViewTool::Line | ViewTool::Rect | ViewTool::Circle, Pending::Idle)
            | (ViewTool::Arc, Pending::Idle) => {
                self.tool = Pending::First(at, attach);
                return None;
            }
            (ViewTool::Line, Pending::First(a, a_at)) => {
                if geom::dist(a, at) < 1e-9 {
                    return None;
                }
                let g = edit::add_line(sk, a, at, false);
                edit::attach(sk, PointRef::new(g, Pos::Start), a_at);
                edit::attach(sk, PointRef::new(g, Pos::End), attach);
                edit::auto_level(sk, g);
                self.tool = Pending::Idle;
            }
            (ViewTool::Rect, Pending::First(a, a_at)) => {
                if (a[0] - at[0]).abs() < 1e-9 || (a[1] - at[1]).abs() < 1e-9 {
                    return None;
                }
                let ids = edit::add_rectangle(sk, a, at, false);
                edit::attach(sk, PointRef::new(ids[0], Pos::Start), a_at);
                edit::attach(sk, PointRef::new(ids[2], Pos::Start), attach);
                self.tool = Pending::Idle;
            }
            (ViewTool::Circle, Pending::First(c, c_at)) => {
                let r = geom::dist(c, at);
                if r < 1e-9 {
                    return None;
                }
                let g = edit::add_circle(sk, c, r, false);
                edit::attach(sk, PointRef::new(g, Pos::Mid), c_at);
                if let Attach::Curve(other) = attach {
                    sk.constraints.push(Constraint::Tangent(g, other));
                }
                self.tool = Pending::Idle;
            }
            (ViewTool::Arc, Pending::First(c, c_at)) => {
                if geom::dist(c, at) < 1e-9 {
                    return None;
                }
                self.tool = Pending::ArcStart(c, c_at, at, attach);
                return None;
            }
            (ViewTool::Arc, Pending::ArcStart(c, c_at, s, s_at)) => {
                let g = edit::add_arc(sk, c, s, at, false);
                edit::attach(sk, PointRef::new(g, Pos::Mid), c_at);
                edit::attach(sk, PointRef::new(g, Pos::Start), s_at);
                // The end lies where the pointer points, at the start's radius.
                if let Attach::Point(q) = attach
                    && let Some(e) = geom::point(sk, PointRef::new(g, Pos::End))
                    && geom::point(sk, q).is_some_and(|qp| geom::dist(qp, e) < tol)
                {
                    edit::attach(sk, PointRef::new(g, Pos::End), attach);
                }
                self.tool = Pending::Idle;
            }
            (ViewTool::Polyline, Pending::Idle) => {
                self.tool = Pending::First(at, attach);
                return None;
            }
            (ViewTool::Polyline, Pending::First(a, a_at)) => {
                if geom::dist(a, at) < 1e-9 {
                    return None;
                }
                let g = edit::add_line(sk, a, at, false);
                edit::attach(sk, PointRef::new(g, Pos::Start), a_at);
                edit::attach(sk, PointRef::new(g, Pos::End), attach);
                edit::auto_level(sk, g);
                self.tool = Pending::Chain {
                    first: PointRef::new(g, Pos::Start),
                    last: g,
                };
            }
            (ViewTool::Polyline, Pending::Chain { first, last }) => {
                let from = geom::point(sk, PointRef::new(last, Pos::End))?;
                if geom::dist(from, at) < tol {
                    // Clicking the last point again finishes the polyline.
                    self.tool = Pending::Idle;
                    return None;
                }
                let closing = geom::point(sk, first).is_some_and(|f| geom::dist(f, at) < tol);
                let g = edit::add_line(sk, from, at, false);
                sk.constraints.push(Constraint::Coincident(
                    PointRef::new(last, Pos::End),
                    PointRef::new(g, Pos::Start),
                ));
                if closing {
                    sk.constraints
                        .push(Constraint::Coincident(PointRef::new(g, Pos::End), first));
                    self.tool = Pending::Idle;
                } else {
                    edit::attach(sk, PointRef::new(g, Pos::End), attach);
                    self.tool = Pending::Chain { first, last: g };
                }
                edit::auto_level(sk, g);
            }
            (ViewTool::Trim | ViewTool::Extend | ViewTool::Split, _) => {
                let Some(g) = geom::nearest_curve(sk, raw, tol).filter(|g| *g >= 0) else {
                    self.fail("Click on a curve.", frame);
                    return None;
                };
                let result = match view.tool {
                    ViewTool::Trim => edit::trim(sk, g, raw),
                    ViewTool::Extend => edit::extend(sk, g, raw),
                    _ => edit::split(sk, g, raw),
                };
                if let Err(e) = result {
                    self.fail(e, frame);
                    return None;
                }
            }
            (ViewTool::Fillet, _) => {
                let radius = if view.value > 0.0 {
                    f64::from(view.value)
                } else {
                    tol * 3.0
                };
                if let Err(e) = edit::fillet(sk, raw, radius, tol) {
                    self.fail(e, frame);
                    return None;
                }
            }
            (ViewTool::External, _) => {
                let p = pointer?;
                let Some(seg) = mesh.and_then(|m| nearest_edge(m, cam, rect, p)) else {
                    self.fail("Click an edge of the model.", frame);
                    return None;
                };
                let a = sk.plane.to_sketch(seg[0].map(f64::from));
                let b = sk.plane.to_sketch(seg[1].map(f64::from));
                if geom::dist(a, b) < 1e-9 {
                    self.fail("That edge is end-on to the sketch.", frame);
                    return None;
                }
                edit::add_external(sk, a, b);
            }
            _ => return None,
        }
        Some(self.commit(sk.clone()))
    }

    fn paint(
        &mut self,
        ui: &egui::Ui,
        rect: egui::Rect,
        cam: &Camera,
        view: &Viewport<'_>,
        frame: u64,
    ) {
        let Some(sk) = self.working.as_ref() else {
            return;
        };
        let painter = ui.painter_at(rect);
        let (w, h) = (rect.width(), rect.height());
        let to_screen = |p: P2| {
            cam.project(to_f32(sk.plane.to_world(p)), w, h)
                .map(|s| rect.min + egui::vec2(s[0], s[1]))
        };
        let poly = |pts: &[P2], stroke: egui::Stroke| {
            for pair in pts.windows(2) {
                if let (Some(a), Some(b)) = (to_screen(pair[0]), to_screen(pair[1])) {
                    painter.line_segment([a, b], stroke);
                }
            }
        };

        // The sketch's own axes, as FreeCAD draws them.
        let reach = f64::from(cam.state.distance) * 4.0;
        poly(
            &[[-reach, 0.0], [reach, 0.0]],
            egui::Stroke::new(1.0, AXIS_X),
        );
        poly(
            &[[0.0, -reach], [0.0, reach]],
            egui::Stroke::new(1.0, AXIS_Y),
        );

        let selected = |item: Item| self.selected.contains(&item);
        let hovered = |item: Item| {
            self.hover.is_some_and(|h| match (h.attach, item) {
                (Attach::Point(q), Item::Point(r)) => q == r,
                (Attach::Curve(g), Item::Geo(k)) => g == k,
                _ => false,
            })
        };
        let base = if self.report.fully_constrained() {
            DONE
        } else {
            INK
        };
        for (i, g) in sk.geos.iter().enumerate() {
            let item = Item::Geo(i as i32);
            let color = if selected(item) {
                SELECTED
            } else if hovered(item) {
                HOVER
            } else if g.fixed {
                EXTERNAL
            } else if g.construction {
                CONSTRUCTION
            } else {
                base
            };
            let stroke = egui::Stroke::new(if g.construction { 1.0 } else { 2.0 }, color);
            match g.curve {
                Curve::Line(a, b) => poly(&[a, b], stroke),
                Curve::Circle { center, radius } => {
                    let pts: Vec<P2> = (0..=64)
                        .map(|k| geom::polar(center, radius, k as f64 / 64.0 * 2.0 * PI))
                        .collect();
                    poly(&pts, stroke);
                }
                Curve::Arc {
                    center,
                    radius,
                    start,
                    end,
                } => {
                    let pts: Vec<P2> = (0..=48)
                        .map(|k| {
                            geom::polar(center, radius, start + (end - start) * k as f64 / 48.0)
                        })
                        .collect();
                    poly(&pts, stroke);
                }
                Curve::Point(_) => {}
            }
            for &pos in geom::points_of(&g.curve) {
                let r = PointRef::new(i as i32, pos);
                if let Some(s) = geom::point(sk, r).and_then(to_screen) {
                    let (radius, color) = if selected(Item::Point(r)) {
                        (4.5, SELECTED)
                    } else if hovered(Item::Point(r)) {
                        (4.5, HOVER)
                    } else {
                        (2.5, color)
                    };
                    painter.circle_filled(s, radius, color);
                }
            }
        }
        if let Some(o) = to_screen([0.0, 0.0]) {
            let c = if selected(Item::Point(PointRef::ORIGIN))
                || hovered(Item::Point(PointRef::ORIGIN))
            {
                SELECTED
            } else {
                INK
            };
            painter.circle_stroke(o, 3.0, egui::Stroke::new(1.0, c));
        }

        // Constraint labels: pickable, red when in conflict, amber when redundant.
        self.labels.clear();
        let mut stacked: std::collections::HashMap<(i64, i64), u32> = Default::default();
        for (i, c) in sk.constraints.iter().enumerate() {
            let (Some(text), Some(at)) = (label(c), anchor(sk, c)) else {
                continue;
            };
            let Some(s) = to_screen(at) else {
                continue;
            };
            let key = ((s.x / 6.0) as i64, (s.y / 6.0) as i64);
            let k = stacked.entry(key).or_default();
            let pos = s + egui::vec2(8.0, -10.0 - 14.0 * *k as f32);
            *k += 1;
            let color = if self.report.conflicting.contains(&i) {
                BAD
            } else if self.report.redundant.contains(&i) {
                WARN
            } else if selected(Item::Constraint(i as u32)) {
                SELECTED
            } else {
                CONSTRUCTION
            };
            let galley = painter.layout_no_wrap(text, egui::FontId::proportional(11.0), color);
            let r = egui::Rect::from_min_size(pos, galley.size()).expand(2.0);
            painter.rect_filled(r, 2.0, egui::Color32::from_white_alpha(220));
            painter.galley(pos, galley, color);
            self.labels.push((r, i as u32));
        }

        // What the tool in progress would make.
        if let Some(hv) = self.hover {
            let rubber = egui::Stroke::new(1.2, HOVER);
            match (view.tool, self.tool) {
                (ViewTool::Line | ViewTool::Polyline, Pending::First(a, _)) => {
                    poly(&[a, hv.at], rubber)
                }
                (ViewTool::Polyline, Pending::Chain { last, .. }) => {
                    if let Some(a) = geom::point(sk, PointRef::new(last, Pos::End)) {
                        poly(&[a, hv.at], rubber);
                    }
                }
                (ViewTool::Rect, Pending::First(a, _)) => {
                    let b = hv.at;
                    poly(&[a, [b[0], a[1]], b, [a[0], b[1]], a], rubber);
                }
                (ViewTool::Circle | ViewTool::Arc, Pending::First(c, _)) => {
                    let r = geom::dist(c, hv.at);
                    let pts: Vec<P2> = (0..=64)
                        .map(|k| geom::polar(c, r, k as f64 / 64.0 * 2.0 * PI))
                        .collect();
                    poly(&pts, rubber);
                }
                (ViewTool::Arc, Pending::ArcStart(c, _, s, _)) => {
                    let r = geom::dist(c, s);
                    let a0 = (s[1] - c[1]).atan2(s[0] - c[0]);
                    let a1 = (hv.at[1] - c[1]).atan2(hv.at[0] - c[0]);
                    let span = geom::sweep_to(a0, a1);
                    let pts: Vec<P2> = (0..=48)
                        .map(|k| geom::polar(c, r, a0 + span * k as f64 / 48.0))
                        .collect();
                    poly(&pts, rubber);
                }
                _ => {}
            }
            let drawing = !matches!(
                view.tool,
                ViewTool::Select | ViewTool::Orbit | ViewTool::PushPull
            );
            if drawing && let Some(s) = to_screen(hv.at) {
                match hv.attach {
                    Attach::Point(_) => {
                        painter.circle_stroke(s, 6.0, egui::Stroke::new(1.5, HOVER));
                    }
                    Attach::Curve(_) => {
                        let d = 4.0;
                        painter.line_segment(
                            [s + egui::vec2(-d, -d), s + egui::vec2(d, d)],
                            egui::Stroke::new(1.5, HOVER),
                        );
                        painter.line_segment(
                            [s + egui::vec2(-d, d), s + egui::vec2(d, -d)],
                            egui::Stroke::new(1.5, HOVER),
                        );
                    }
                    Attach::Nothing => {}
                }
            }
        }

        // FreeCAD's solver messages.
        let (text, color) = if let Some(e) = &self.error {
            (format!("Sketch problem: {e}"), BAD)
        } else if !self.report.conflicting.is_empty() {
            (
                format!(
                    "Conflicting constraints: {}",
                    list(&self.report.conflicting)
                ),
                BAD,
            )
        } else if !self.report.redundant.is_empty() {
            (
                format!("Redundant constraints: {}", list(&self.report.redundant)),
                WARN,
            )
        } else if self.report.fully_constrained() {
            ("Fully constrained".to_owned(), DONE)
        } else if self.report.dof == 1 {
            ("1 degree of freedom".to_owned(), INK)
        } else {
            (format!("{} degrees of freedom", self.report.dof), INK)
        };
        painter.text(
            rect.left_top() + egui::vec2(8.0, 8.0),
            egui::Align2::LEFT_TOP,
            text,
            egui::FontId::proportional(12.0),
            color,
        );
        if let Some((msg, at)) = &self.message
            && frame - at < 180
        {
            painter.text(
                rect.left_top() + egui::vec2(8.0, 24.0),
                egui::Align2::LEFT_TOP,
                msg,
                egui::FontId::proportional(12.0),
                WARN,
            );
        }
    }
}

/// Constraint indices as FreeCAD lists them, from 1.
fn list(ix: &[usize]) -> String {
    ix.iter()
        .map(|i| (i + 1).to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

/// The model edge nearest pointer `p` (in the viewport), within the pick distance.
fn nearest_edge(
    mesh: &Mesh,
    cam: &Camera,
    rect: egui::Rect,
    p: egui::Pos2,
) -> Option<[[f32; 3]; 2]> {
    let (w, h) = (rect.width(), rect.height());
    let mut best: Option<(f32, [[f32; 3]; 2])> = None;
    for seg in &mesh.data.edges {
        let (Some(a), Some(b)) = (cam.project(seg[0], w, h), cam.project(seg[1], w, h)) else {
            continue;
        };
        let (a, b) = (egui::pos2(a[0], a[1]), egui::pos2(b[0], b[1]));
        let ab = b - a;
        let t = if ab.length_sq() > 0.0 {
            ((p - a).dot(ab) / ab.length_sq()).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let d = (a + ab * t - p).length();
        if d <= PICK_PX && best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, *seg));
        }
    }
    best.map(|(_, s)| s)
}
