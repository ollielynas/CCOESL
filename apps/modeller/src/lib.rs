//! The Modeller: FreeCAD's Part Design and Sketcher, with SketchUp's push/pull, and FreeCAD
//! doing the geometry.
//!
//! Sketch on the ground or on any face — lines, arcs, circles, held in place by FreeCAD's
//! constraints — then pad or pocket the sketch into a solid. Or draw a rectangle straight onto
//! a face and push or pull it, SketchUp style. Save as a FreeCAD document, STEP or STL.
//!
//! The top panel is laid out like FreeCAD's: the File, Edit, workbench and View toolbars on
//! the first row, the workbench's own toolbars below, and the Sketcher's constraint toolbar
//! while a sketch is open.
//!
//! **The model is the list of steps.** This app never holds geometry: its whole document is
//! `ops`, a `Vec<CadOp>`. The server replays it in FreeCAD (`Regenerate`, polled like the
//! Compiler's builds) and answers with the URL of a mesh, which the *shell* fetches and draws.
//! Undo pops a step; going back to a model seen before costs nothing.
//!
//! **The shell does everything that follows the pointer**: orbiting, snapping, the rubber
//! band, the live extrusion, and in a sketch the drawing tools and the solver while dragging.
//! This app hears only finished gestures and edits, and turns each into a step.

use ccosel_proto::account::WhoAmI;
use ccosel_proto::cad::{
    CadOp, ExportFormat, ExportModel, ExportReq, Exported, Model, RegenReq, RegenResult, Regenerate,
};
use ccosel_sdk::icons;
use ccosel_sdk::view3d::{
    Anchor, Extrude, NO_FACE, Render, StdView, ViewAction, ViewCommand, ViewEvent, ViewTool,
    plane_basis, rect_corners,
};
use ccosel_sdk::{App, CallId, Poll, REPAINT_ON_INPUT_ONLY, Text, Ui, Vec2, View3d};

mod measure;
mod sketcher;

use sketcher::SketchEdit;

/// How often to ask whether a rebuild has finished. FreeCAD rebuilds a hand-drawn model in
/// well under a second, so this is about responsiveness, not load.
const POLL_MS: u32 = 150;

/// Part Design's own drawing tools: SketchUp's quick way to a solid, beside the Sketcher.
const DRAW_TOOLS: [(&str, &str, ViewTool); 5] = [
    (icons::CURSOR, "Select", ViewTool::Select),
    (icons::LINE_SEGMENTS, "Line", ViewTool::Line),
    (icons::RECTANGLE, "Rectangle", ViewTool::Rect),
    (icons::ARROW_FAT_LINE_UP, "Push/Pull", ViewTool::PushPull),
    (icons::HAND_POINTING, "Orbit", ViewTool::Orbit),
];

/// FreeCAD's View toolbar.
const VIEWS: [(&str, &str, StdView); 8] = [
    (icons::ARROWS_OUT, "Fit all", StdView::Fit),
    (icons::CUBE, "Isometric", StdView::Isometric),
    ("Front", "Front view", StdView::Front),
    ("Top", "Top view", StdView::Top),
    ("Right", "Right view", StdView::Right),
    ("Rear", "Rear view", StdView::Rear),
    ("Bottom", "Bottom view", StdView::Bottom),
    ("Left", "Left view", StdView::Left),
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
    /// What a rebuild in progress is waiting on, from the server: installing FreeCAD, say.
    note: String,
    /// The op list changed this frame: ask for its model before the frame ends, rather than
    /// a frame late.
    changed: bool,
    last: Option<Last>,
    measure: Text,
    measure_error: bool,
    /// Where saves and exports go, without an extension. Filled in with the user's home.
    file: Text,
    file_set: bool,
    export: Option<(CallId, String)>,
    exported: Option<Result<Exported, String>>,
    render: Render,
    /// The sketch open in the Sketcher, if one is.
    sketch: Option<SketchEdit>,
    /// "Create sketch" was pressed: the next click picks the plane.
    choosing_plane: bool,
    /// The last standard view asked for; its `seq` goes up with each press.
    view: ViewCommand,
    /// A complaint to show in the status line, such as a constraint that needs another
    /// selection.
    message: Option<String>,
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
            note: String::new(),
            changed: false,
            last: None,
            measure: Text::default(),
            measure_error: false,
            file: Text::new("/model"),
            file_set: false,
            export: None,
            exported: None,
            render: Render::Auto,
            sketch: None,
            choosing_plane: false,
            view: ViewCommand::default(),
            message: None,
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

/// An icon button with its FreeCAD name as the tooltip.
fn tool_button(ui: &mut Ui<'_>, icon: &str, tip: &str) -> bool {
    let clicked = ui.button(icon).clicked();
    ui.tooltip(tip);
    clicked
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

    /// Start again with an empty model. Undo brings the old one back, step by step.
    fn new_model(&mut self) {
        while !self.ops.is_empty() {
            self.undo();
        }
        self.sketch = None;
        self.choosing_plane = false;
    }

    fn show_view(&mut self, view: StdView) {
        self.view = ViewCommand {
            view,
            seq: self.view.seq.wrapping_add(1),
        };
    }

    /// The faces the model's last step made, if the model on screen is the whole model: what
    /// Pad and Pocket act on.
    fn new_faces(&self) -> Option<(usize, &[u32])> {
        match &self.shown {
            Some((m, n)) if *n == self.ops.len() && !m.new_faces.is_empty() => {
                Some((*n, &m.new_faces))
            }
            _ => None,
        }
    }

    /// FreeCAD's Pad (`out`) or Pocket: push or pull every face the last step made by the
    /// length in Measurements (10 mm if it is empty).
    fn pad(&mut self, out: bool) {
        let Some((after, faces)) = self.new_faces() else {
            self.message = Some("Pad and Pocket work on the sketch or shape just made.".to_owned());
            return;
        };
        let faces = faces.to_vec();
        let text = self.measure.as_str().trim().to_owned();
        let Some(length) = (if text.is_empty() {
            Some(10.0)
        } else {
            measure::length(&text).map(f32::abs)
        }) else {
            self.measure_error = true;
            return;
        };
        let d = if out { length } else { -length };
        for face in faces {
            self.push(CadOp::PushPull {
                after: after as u32,
                face,
                distance: d,
            });
        }
        self.last = Some(Last::PushPull);
    }

    /// Turn one finished gesture from the shell into a step, or into drawing progress.
    fn gesture(&mut self, e: &ViewEvent) {
        if self.choosing_plane {
            if e.action == ViewAction::Click {
                self.choosing_plane = false;
                self.sketch = Some(SketchEdit::on_plane(e.point, e.normal, None));
                self.tool = ViewTool::Line;
                self.show_view(StdView::Sketch);
            }
            return;
        }
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
        if !self.building {
            self.note.clear();
        }
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
                    self.note.clone_from(&status.note);
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

    /// Row one, as in FreeCAD: File, Edit, the workbench, and View.
    fn main_toolbar(&mut self, ui: &mut Ui<'_>) {
        ui.horizontal(|ui| {
            if tool_button(ui, icons::FILE_PLUS, "New") {
                self.new_model();
            }
            if tool_button(ui, icons::FLOPPY_DISK, "Save as a FreeCAD document") {
                self.save(ui, ExportFormat::FreeCad);
            }
            if tool_button(ui, icons::EXPORT, "Export as STEP") {
                self.save(ui, ExportFormat::Step);
            }
            if tool_button(ui, icons::ARROW_SQUARE_OUT, "Export as STL") {
                self.save(ui, ExportFormat::Stl);
            }
            ui.separator();
            if tool_button(ui, icons::ARROW_COUNTER_CLOCKWISE, "Undo") {
                match &mut self.sketch {
                    Some(s) => s.undo(),
                    None => self.undo(),
                }
            }
            if tool_button(ui, icons::ARROW_CLOCKWISE, "Redo") {
                match &mut self.sketch {
                    Some(s) => s.redo(),
                    None => self.redo(),
                }
            }
            ui.separator();
            let sketching = self.sketch.is_some();
            if ui.selectable(!sketching, "Part Design").clicked() && sketching {
                self.close_sketch();
            }
            if ui.selectable(sketching, "Sketcher").clicked() && !sketching {
                self.start_sketch();
            }
            ui.separator();
            for (icon, tip, view) in VIEWS {
                if tool_button(ui, icon, tip) {
                    self.show_view(view);
                }
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

    /// Row two in Part Design: sketches and their features, then the draw tools.
    fn part_design_toolbar(&mut self, ui: &mut Ui<'_>) {
        ui.horizontal(|ui| {
            if ui.button("Create sketch").clicked() {
                self.start_sketch();
            }
            ui.tooltip("Pick a face, or the ground, to sketch on.");
            let last_sketch = self
                .ops
                .iter()
                .rposition(|op| matches!(op, CadOp::Sketch(_)));
            if let Some(i) = last_sketch
                && ui.button("Edit sketch").clicked()
                && let CadOp::Sketch(sk) = &self.ops[i]
            {
                self.sketch = Some(SketchEdit::editing(sk.clone(), i));
                self.tool = ViewTool::Select;
                self.show_view(StdView::Sketch);
            }
            if ui.button("Pad").clicked() {
                self.pad(true);
            }
            ui.tooltip(
                "Pull the sketch just made out into a solid, by the length in Measurements.",
            );
            if ui.button("Pocket").clicked() {
                self.pad(false);
            }
            ui.tooltip("Push the sketch just made into the solid under it, cutting it away.");
            ui.separator();
            for (icon, name, tool) in DRAW_TOOLS {
                let mut label = String::from(icon);
                label.push(' ');
                label.push_str(name);
                if ui.selectable(self.tool == tool, &label).clicked() && self.tool != tool {
                    self.tool = tool;
                    self.cancel_drawing();
                    self.choosing_plane = false;
                }
            }
        });
    }

    fn start_sketch(&mut self) {
        self.choosing_plane = true;
        self.cancel_drawing();
        self.tool = ViewTool::Select;
    }

    /// Leave the Sketcher, keeping the sketch as a step (or replacing the one edited).
    fn close_sketch(&mut self) {
        let Some(edit) = self.sketch.take() else {
            return;
        };
        let op = CadOp::Sketch(edit.sketch);
        match edit.replaces {
            Some(i) if i < self.ops.len() => {
                self.ops[i] = op;
                self.redo.clear();
                self.exported = None;
                self.changed = true;
            }
            _ => self.push(op),
        }
        self.tool = ViewTool::Select;
        self.last = None;
    }

    fn hint(&self) -> &'static str {
        if self.choosing_plane {
            return "Click a face, or the ground, to sketch on.";
        }
        if let Some(s) = &self.sketch {
            return s.hint(self.tool);
        }
        match (self.tool, self.anchor.is_some()) {
            (ViewTool::Select, _) => "Click a face to select it. Drag to orbit.",
            (ViewTool::Line, false) => "Click to start an outline.",
            (ViewTool::Line, true) => "Click each corner; click the first again to close it.",
            (ViewTool::Rect, false) => "Click the first corner, on the ground or a face.",
            (ViewTool::Rect, true) => "Click the opposite corner.",
            (ViewTool::PushPull, _) => "Drag a face to push or pull it.",
            _ => "Drag to orbit, shift-drag to pan, scroll to zoom.",
        }
    }

    fn status(&self) -> String {
        if let Some(m) = &self.message {
            return m.clone();
        }
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
            return if self.note.is_empty() {
                "Building…".to_owned()
            } else {
                self.note.clone()
            };
        }
        match &self.shown {
            None => "Draw a rectangle on the ground, or create a sketch, to start.".to_owned(),
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
                "A distance after a push/pull or for Pad and Pocket (250, -30, 2m), or \
                 width,height after a rectangle.",
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

    fn save(&mut self, ui: &mut Ui<'_>, format: ExportFormat) {
        if self.export.is_some() {
            return;
        }
        if self.ops.is_empty() {
            self.message = Some("There is nothing to save yet.".to_owned());
            return;
        }
        let path = export_path(self.file.as_str(), format);
        let call = ui.rpc().send::<ExportModel>(&ExportReq {
            ops: self.ops.clone(),
            format,
            path: path.clone(),
        });
        self.export = Some((call, path));
        self.exported = None;
    }

    fn file_row(&mut self, ui: &mut Ui<'_>) {
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
            ui.label("File");
            ui.text_edit(&mut self.file);
            ui.tooltip("Where Save and Export write, without the extension.");
            if self.export.is_some() {
                ui.label("Saving…");
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
        self.main_toolbar(ui);
        let mut scene = Vec::new();
        let mut value = 0.0;
        // Each section that comes and goes gets its own id scope, so a text field in one never
        // inherits the id — and with it the shell's buffer — of a field in the other.
        match &mut self.sketch {
            Some(s) => {
                let tool = &mut self.tool;
                let out = ui.push_id("sketcher", |ui| s.toolbars(ui, tool));
                if let Some(msg) = out.message {
                    self.message = Some(msg);
                }
                if out.close {
                    self.close_sketch();
                } else if out.cancel {
                    self.sketch = None;
                    self.tool = ViewTool::Select;
                } else if out.face {
                    self.show_view(StdView::Sketch);
                }
            }
            None => ui.push_id("part design", |ui| self.part_design_toolbar(ui)),
        }
        ui.horizontal(|ui| {
            ui.label(self.hint());
            ui.separator();
            ui.label(&self.status());
        });
        if self.sketch.is_none() {
            ui.push_id("measurements", |ui| self.measurements(ui));
        }
        ui.push_id("file", |ui| self.file_row(ui));
        if let Some(s) = &self.sketch {
            scene = s.scene();
            value = s.value();
        }

        let mesh = self.shown.as_ref().map(|(m, _)| m.mesh.clone());
        let sketching = self.sketch.is_some();
        let view = ui.viewport3d(&View3d {
            size: Vec2::new(0.0, 0.0),
            mesh: mesh.as_deref().unwrap_or(""),
            tool: self.tool,
            anchor: if sketching { None } else { self.anchor },
            path: if sketching { &[] } else { &self.path },
            preview: self.preview,
            selected: self.selected,
            render: self.render,
            sketch: &scene,
            value,
            view: self.view,
        });
        if !view.events.is_empty() || !view.sketch.is_empty() {
            self.message = None;
        }
        for e in &view.events {
            self.gesture(e);
        }
        if let Some(s) = &mut self.sketch {
            for bytes in &view.sketch {
                s.event(bytes);
            }
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
