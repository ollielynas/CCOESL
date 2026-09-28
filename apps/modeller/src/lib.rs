//! The Modeller: SketchUp-style push/pull modelling, with FreeCAD doing the geometry.
//!
//! Draw a rectangle (or any flat outline, with the Line tool) on the ground or on a face,
//! then push or pull it into a solid. Pulling out of a solid adds to it; pushing into one
//! cuts a pocket. Type into the Measurements box to make the last step exact. Save as STEP,
//! STL or a FreeCAD document.
//!
//! **The model is the list of steps.** This app never holds geometry: its whole document is
//! `ops`, a `Vec<CadOp>` of a few bytes per step. The server replays it in FreeCAD
//! (`Regenerate`, polled like the Compiler's builds) and answers with the URL of a mesh, which
//! the *shell* fetches and draws. Undo pops a step; redo pushes it back. Undoing to a model
//! seen before costs nothing, because the server and the browser both remember it.
//!
//! **The shell does everything that follows the pointer**: orbiting, snapping, the rubber
//! band, the live extrusion. This app only hears about finished gestures — a click at a
//! snapped point, a face pushed by a distance — and turns each into a step.

use ccosel_proto::account::WhoAmI;
use ccosel_proto::cad::{
    CadOp, ExportFormat, ExportModel, ExportReq, Exported, Model, RegenReq, RegenResult, Regenerate,
};
use ccosel_sdk::view3d::{
    Anchor, Extrude, NO_FACE, Render, ViewAction, ViewEvent, ViewTool, plane_basis, rect_corners,
};
use ccosel_sdk::{App, CallId, Poll, REPAINT_ON_INPUT_ONLY, Text, Ui, Vec2, View3d};

mod measure;

/// How often to ask whether a rebuild has finished. FreeCAD rebuilds a hand-drawn model in
/// well under a second, so this is about responsiveness, not load.
const POLL_MS: u32 = 150;

const TOOLS: [(&str, ViewTool); 5] = [
    ("Select", ViewTool::Select),
    ("Line", ViewTool::Line),
    ("Rectangle", ViewTool::Rect),
    ("Push/Pull", ViewTool::PushPull),
    ("Orbit", ViewTool::Orbit),
];

/// The last step, as the Measurements box needs to remake it with an exact size.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Last {
    /// A rectangle from `anchor` on the plane `normal`, `du` × `dv` along `plane_basis`.
    Rect {
        anchor: [f32; 3],
        normal: [f32; 3],
        du: f32,
        dv: f32,
    },
    PushPull,
}

pub struct Modeller {
    ops: Vec<CadOp>,
    redo: Vec<CadOp>,
    tool: ViewTool,
    /// The plane and first point of the shape being drawn.
    anchor: Option<Anchor>,
    /// The Line tool's corners so far.
    path: Vec<[f32; 3]>,
    selected: u32,
    /// The last model the server built, and how many steps built it. Face numbers in the
    /// shell's gestures are that model's.
    shown: Option<(Model, usize)>,
    /// A push/pull the server has not rebuilt yet, drawn by the shell in the meantime.
    preview: Option<Extrude>,
    /// A step FreeCAD refused (its index, or `u32::MAX` for none in particular) and why.
    failure: Option<(u32, String)>,
    building: bool,
    /// The op list changed this frame: ask for its model before the frame ends, rather than
    /// a frame late.
    changed: bool,
    last: Option<Last>,
    measure: Text,
    measure_error: bool,
    /// Where exports go, without an extension. Filled in with the user's home folder.
    file: Text,
    file_set: bool,
    export: Option<(CallId, String)>,
    exported: Option<Result<Exported, String>>,
    render: Render,
}

impl Default for Modeller {
    fn default() -> Self {
        Self {
            ops: Vec::new(),
            redo: Vec::new(),
            tool: ViewTool::Rect,
            anchor: None,
            path: Vec::new(),
            selected: NO_FACE,
            shown: None,
            preview: None,
            failure: None,
            building: false,
            changed: false,
            last: None,
            measure: Text::default(),
            measure_error: false,
            file: Text::new("/model"),
            file_set: false,
            export: None,
            exported: None,
            render: Render::Auto,
        }
    }
}

fn close(a: [f32; 3], b: [f32; 3]) -> bool {
    let d = (0..3).map(|i| (a[i] - b[i]) * (a[i] - b[i])).sum::<f32>();
    d < 1e-6
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn sign(v: f32) -> f32 {
    if v < 0.0 { -1.0 } else { 1.0 }
}

/// `/files/<path>`, each segment percent-encoded, for a download link.
pub fn download_url(path: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut url = String::from("/files");
    for segment in path.split('/').filter(|s| !s.is_empty()) {
        url.push('/');
        for &b in segment.as_bytes() {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
                url.push(b as char);
            } else {
                url.push('%');
                url.push(HEX[usize::from(b >> 4)] as char);
                url.push(HEX[usize::from(b & 0xF)] as char);
            }
        }
    }
    url
}

/// `base` with the format's extension, replacing one the user typed.
fn export_path(base: &str, format: ExportFormat) -> String {
    let mut base = base.trim().to_owned();
    for known in [".step", ".stp", ".stl", ".fcstd"] {
        if base.to_ascii_lowercase().ends_with(known) {
            base.truncate(base.len() - known.len());
        }
    }
    base.push('.');
    base.push_str(format.extension());
    base
}

impl Modeller {
    /// Add a step. Anything undone is gone once something new is done, as everywhere.
    fn push(&mut self, op: CadOp) {
        self.ops.push(op);
        self.redo.clear();
        self.exported = None;
        self.measure_error = false;
        self.changed = true;
    }

    fn cancel_drawing(&mut self) {
        self.anchor = None;
        self.path.clear();
    }

    fn undo(&mut self) {
        if self.anchor.is_some() {
            // Esc-like: a shape half drawn goes first.
            self.cancel_drawing();
            return;
        }
        if let Some(op) = self.ops.pop() {
            self.changed = true;
            self.redo.push(op);
            self.last = None;
            self.selected = NO_FACE;
            self.preview = None;
            self.exported = None;
        }
    }

    fn redo(&mut self) {
        if let Some(op) = self.redo.pop() {
            self.changed = true;
            self.ops.push(op);
            self.last = None;
            self.exported = None;
        }
    }

    /// Turn one finished gesture from the shell into a step, or into drawing progress.
    fn gesture(&mut self, e: &ViewEvent) {
        match (e.action, self.tool) {
            (ViewAction::PushPull, _) => {
                let after = self.shown.as_ref().map_or(0, |(_, n)| *n) as u32;
                self.push(CadOp::PushPull {
                    after,
                    face: e.face,
                    distance: e.distance,
                });
                self.preview = Some(Extrude {
                    face: e.face,
                    distance: e.distance,
                });
                self.last = Some(Last::PushPull);
            }
            (ViewAction::Click, ViewTool::Select) => self.selected = e.face,
            (ViewAction::Click, ViewTool::Rect) => match self.anchor {
                None => {
                    self.anchor = Some(Anchor {
                        point: e.point,
                        normal: e.normal,
                    });
                }
                Some(a) => {
                    let (u, v) = plane_basis(a.normal);
                    let d = [
                        e.point[0] - a.point[0],
                        e.point[1] - a.point[1],
                        e.point[2] - a.point[2],
                    ];
                    let (du, dv) = (dot(d, u), dot(d, v));
                    // A click on the first corner again is a slip, not a rectangle.
                    if du.abs() < 1e-3 || dv.abs() < 1e-3 {
                        return;
                    }
                    self.push(CadOp::Polygon {
                        points: rect_corners(a.point, a.normal, e.point).to_vec(),
                        normal: a.normal,
                    });
                    self.last = Some(Last::Rect {
                        anchor: a.point,
                        normal: a.normal,
                        du,
                        dv,
                    });
                    self.cancel_drawing();
                }
            },
            (ViewAction::Click, ViewTool::Line) => match self.anchor {
                None => {
                    self.anchor = Some(Anchor {
                        point: e.point,
                        normal: e.normal,
                    });
                    self.path = vec![e.point];
                }
                Some(a) => {
                    if self.path.len() >= 3 && close(e.point, self.path[0]) {
                        let points = std::mem::take(&mut self.path);
                        self.push(CadOp::Polygon {
                            points,
                            normal: a.normal,
                        });
                        self.last = None;
                        self.cancel_drawing();
                    } else if !self.path.last().is_some_and(|p| close(*p, e.point)) {
                        self.path.push(e.point);
                    }
                }
            },
            _ => {}
        }
    }

    /// Remake the last step at the size typed into the Measurements box.
    fn apply_measure(&mut self) {
        let text = self.measure.as_str().to_owned();
        let applied = match (self.last, self.ops.last_mut()) {
            (Some(Last::PushPull), Some(CadOp::PushPull { face, distance, .. })) => {
                measure::length(&text).map(|d| {
                    *distance = d;
                    self.preview = Some(Extrude {
                        face: *face,
                        distance: d,
                    });
                })
            }
            (
                Some(Last::Rect {
                    anchor,
                    normal,
                    du,
                    dv,
                }),
                Some(CadOp::Polygon { points, .. }),
            ) => measure::pair(&text)
                .filter(|(w, h)| *w != 0.0 && *h != 0.0)
                .map(|(w, h)| {
                    let (u, v) = plane_basis(normal);
                    let (w, h) = (w.abs() * sign(du), h.abs() * sign(dv));
                    let to = [
                        anchor[0] + u[0] * w + v[0] * h,
                        anchor[1] + u[1] * w + v[1] * h,
                        anchor[2] + u[2] * w + v[2] * h,
                    ];
                    *points = rect_corners(anchor, normal, to).to_vec();
                    self.last = Some(Last::Rect {
                        anchor,
                        normal,
                        du: w,
                        dv: h,
                    });
                }),
            _ => None,
        };
        self.measure_error = applied.is_none();
        if applied.is_some() {
            self.changed = true;
            self.redo.clear();
            self.exported = None;
        }
    }

    /// Ask for the current op list's model, and take it when it is ready.
    fn poll(&mut self, ui: &mut Ui<'_>) {
        self.building = false;
        if self.ops.is_empty() {
            self.shown = None;
            self.preview = None;
            self.failure = None;
            return;
        }
        let req = RegenReq {
            ops: self.ops.clone(),
        };
        match ui.rpc().get::<Regenerate>(&req) {
            Poll::Pending => self.building = true,
            Poll::Failed(e) => {
                self.failure = Some((u32::MAX, e.message().to_owned()));
                self.preview = None;
            }
            Poll::Ready(status) => match &status.result {
                None => {
                    self.building = true;
                    ui.rpc().invalidate::<Regenerate>(&req);
                }
                Some(RegenResult::Ok(model)) => {
                    let n = self.ops.len();
                    if self
                        .shown
                        .as_ref()
                        .is_none_or(|(m, k)| m != model || *k != n)
                    {
                        self.shown = Some((model.clone(), n));
                        self.preview = None;
                        self.selected = NO_FACE;
                    }
                    self.failure = None;
                }
                Some(RegenResult::Failed { op, message }) => {
                    self.failure = Some((*op, message.clone()));
                    self.preview = None;
                }
            },
        }
    }

    fn toolbar(&mut self, ui: &mut Ui<'_>) {
        ui.horizontal(|ui| {
            for (name, tool) in TOOLS {
                if ui.selectable(self.tool == tool, name).clicked() && self.tool != tool {
                    self.tool = tool;
                    self.cancel_drawing();
                }
            }
            ui.separator();
            if ui.button("Undo").clicked() {
                self.undo();
            }
            if ui.button("Redo").clicked() {
                self.redo();
            }
            ui.separator();
            let label = match self.render {
                Render::Auto => "Render: Auto",
                Render::Local => "Render: Browser",
                Render::Server => "Render: Server",
            };
            if ui.button(label).clicked() {
                self.render = match self.render {
                    Render::Auto => Render::Local,
                    Render::Local => Render::Server,
                    Render::Server => Render::Auto,
                };
            }
            ui.tooltip(
                "Where the view is drawn. Auto uses the server for very large models; \
                 the server is also kinder to slow computers.",
            );
        });
    }

    fn hint(&self) -> &'static str {
        match (self.tool, self.anchor.is_some()) {
            (ViewTool::Select, _) => "Click a face to select it. Drag to orbit.",
            (ViewTool::Line, false) => "Click to start an outline.",
            (ViewTool::Line, true) => "Click each corner; click the first again to close it.",
            (ViewTool::Rect, false) => "Click the first corner, on the ground or a face.",
            (ViewTool::Rect, true) => "Click the opposite corner.",
            (ViewTool::PushPull, _) => "Drag a face to push or pull it.",
            (ViewTool::Orbit, _) => "Drag to orbit, shift-drag to pan, scroll to zoom.",
        }
    }

    fn status(&self) -> String {
        if let Some((op, message)) = &self.failure {
            return if *op == u32::MAX {
                format!("Couldn't build the model: {message}")
            } else {
                format!(
                    "Step {} couldn't be done: {message}. Undo to go back.",
                    measure::itoa(u64::from(*op) + 1)
                )
            };
        }
        if self.building {
            return "Building…".to_owned();
        }
        match &self.shown {
            None => "Draw a rectangle on the ground to start.".to_owned(),
            Some((m, _)) => {
                let mut s = measure::itoa(u64::from(m.faces));
                s.push_str(if m.faces == 1 { " face" } else { " faces" });
                if m.solids > 0 {
                    s.push_str(" · ");
                    s.push_str(&measure::itoa(u64::from(m.solids)));
                    s.push_str(if m.solids == 1 {
                        " solid · "
                    } else {
                        " solids · "
                    });
                    s.push_str(&measure::thousands(m.volume_mm3));
                    s.push_str(" mm³");
                }
                s
            }
        }
    }

    fn measurements(&mut self, ui: &mut Ui<'_>) {
        ui.horizontal(|ui| {
            ui.label("Measurements");
            ui.text_edit(&mut self.measure);
            ui.tooltip(
                "A distance after a push/pull (250, -30, 2m), or width,height after a rectangle.",
            );
            if ui.button("Apply").clicked() {
                self.apply_measure();
            }
            if self.measure_error {
                ui.label("Type a length, or width,height for a rectangle.");
            } else if let Some(Last::PushPull) = self.last
                && let Some(CadOp::PushPull { distance, .. }) = self.ops.last()
            {
                ui.label(&measure::mm(*distance));
            }
        });
    }

    fn exports(&mut self, ui: &mut Ui<'_>) {
        if !self.file_set
            && let Poll::Ready(account) = ui.rpc().get::<WhoAmI>(&())
        {
            if let Some(name) = &account.name {
                self.file.set(&format!("/home/{name}/model"));
            }
            self.file_set = true;
        }
        if let Some((call, path)) = self.export.clone() {
            match ui.rpc().outcome::<ExportModel>(call) {
                Poll::Pending => {}
                Poll::Ready(done) => {
                    self.exported = Some(Ok((*done).clone()));
                    self.export = None;
                }
                Poll::Failed(e) => {
                    self.exported = Some(Err(format!("Couldn't save {path}: {}", e.message())));
                    self.export = None;
                }
            }
        }
        ui.horizontal(|ui| {
            ui.label("Save as");
            ui.text_edit(&mut self.file);
            if self.export.is_some() {
                ui.label("Saving…");
                return;
            }
            if self.ops.is_empty() {
                return;
            }
            for (label, format) in [
                ("STEP", ExportFormat::Step),
                ("STL", ExportFormat::Stl),
                ("FreeCAD", ExportFormat::FreeCad),
            ] {
                if ui.button(label).clicked() {
                    let path = export_path(self.file.as_str(), format);
                    let call = ui.rpc().send::<ExportModel>(&ExportReq {
                        ops: self.ops.clone(),
                        format,
                        path: path.clone(),
                    });
                    self.export = Some((call, path));
                    self.exported = None;
                }
            }
            match &self.exported {
                Some(Ok(done)) => {
                    let mut msg = String::from("Saved ");
                    msg.push_str(&done.path);
                    ui.label(&msg);
                    ui.open_url("Download", &download_url(&done.path));
                }
                Some(Err(e)) => ui.label(e),
                None => {}
            }
        });
    }
}

impl App for Modeller {
    fn update(&mut self, ui: &mut Ui<'_>) {
        self.poll(ui);
        self.toolbar(ui);
        ui.horizontal(|ui| {
            ui.label(self.hint());
            ui.separator();
            ui.label(&self.status());
        });
        self.measurements(ui);
        self.exports(ui);

        let mesh = self.shown.as_ref().map(|(m, _)| m.mesh.clone());
        let view = ui.viewport3d(&View3d {
            size: Vec2::new(0.0, 0.0),
            mesh: mesh.as_deref().unwrap_or(""),
            tool: self.tool,
            anchor: self.anchor,
            path: &self.path,
            preview: self.preview,
            selected: self.selected,
            render: self.render,
        });
        for e in &view.events {
            self.gesture(e);
        }
        if std::mem::take(&mut self.changed) {
            self.poll(ui);
        }
    }

    fn wants_repaint_after_ms(&self) -> u32 {
        if self.building {
            POLL_MS
        } else {
            REPAINT_ON_INPUT_ONLY
        }
    }
}

#[cfg(test)]
mod tests;

ccosel_sdk::ccosel_app!(Modeller);
