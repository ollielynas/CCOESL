//! The Sketcher workbench: a sketch being edited, its selection, and FreeCAD's Sketcher
//! toolbars — geometry, editing tools, and constraints.
//!
//! The shell draws the sketch, runs the drawing tools against the pointer and solves; this
//! side holds the sketch, the selection, and what the constraint buttons make of it.

use std::f64::consts::PI;

use ccosel_proto::sketch::{Item, Plane, Sketch, SketchEvent, SketchScene};
use ccosel_sdk::view3d::{ViewTool, plane_basis};
use ccosel_sdk::{Text, Ui, icons};
use ccosel_sketch::build::{Kind, constraints_for};
use ccosel_sketch::edit;

use crate::measure;

/// The Sketcher's geometry toolbar, in FreeCAD's order.
const GEOMETRY: [(&str, &str, ViewTool); 6] = [
    (icons::DOT, "Point", ViewTool::Point),
    (icons::LINE_SEGMENT, "Line", ViewTool::Line),
    (icons::LINE_SEGMENTS, "Polyline", ViewTool::Polyline),
    (icons::CIRCLE_HALF, "Arc", ViewTool::Arc),
    (icons::CIRCLE, "Circle", ViewTool::Circle),
    (icons::RECTANGLE, "Rectangle", ViewTool::Rect),
];

/// Its editing tools.
const EDITING: [(&str, &str, ViewTool); 5] = [
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
];

/// Its constraint toolbar, in FreeCAD's order and with FreeCAD's names.
const CONSTRAINTS: [(&str, Kind); 16] = [
    ("Coincident", Kind::Coincident),
    ("Point on object", Kind::PointOnObject),
    ("Vertical", Kind::Vertical),
    ("Horizontal", Kind::Horizontal),
    ("Parallel", Kind::Parallel),
    ("Perpendicular", Kind::Perpendicular),
    ("Tangent", Kind::Tangent),
    ("Equal", Kind::Equal),
    ("Symmetric", Kind::Symmetric),
    ("Lock", Kind::Lock),
    ("Horizontal distance", Kind::DistanceX),
    ("Vertical distance", Kind::DistanceY),
    ("Distance", Kind::Distance),
    ("Radius", Kind::Radius),
    ("Diameter", Kind::Diameter),
    ("Angle", Kind::Angle),
];

/// What the toolbars asked of the rest of the app this frame.
#[derive(Debug, Default)]
pub struct Out {
    pub close: bool,
    pub cancel: bool,
    /// Turn the view to face the sketch.
    pub face: bool,
    pub message: Option<String>,
}

pub struct SketchEdit {
    pub sketch: Sketch,
    /// The step this sketch was opened from, to be replaced when it closes.
    pub replaces: Option<usize>,
    selected: Vec<Item>,
    undo: Vec<Sketch>,
    redo: Vec<Sketch>,
    /// For a dimension's new value, the Fillet radius, and the Offset distance.
    value: Text,
    /// Frames in which an edit from the shell is the solver finishing a change made here,
    /// not a new step for undo.
    settling: u8,
}

impl SketchEdit {
    fn new(sketch: Sketch, replaces: Option<usize>) -> Self {
        Self {
            sketch,
            replaces,
            selected: Vec::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            value: Text::default(),
            settling: 0,
        }
    }

    /// A new sketch on the plane through `point` facing `normal`: a face that was clicked,
    /// or the ground. Its origin is where the world origin projects onto the plane, so a
    /// sketch on the ground shares the model's coordinates.
    pub fn on_plane(point: [f32; 3], normal: [f32; 3], replaces: Option<usize>) -> Self {
        let n = normal.map(f64::from);
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        let n = if len > 1e-9 {
            n.map(|c| c / len)
        } else {
            [0.0, 0.0, 1.0]
        };
        let d = point[0] as f64 * n[0] + point[1] as f64 * n[1] + point[2] as f64 * n[2];
        let (u, _) = plane_basis(normal);
        Self::new(
            Sketch::new(Plane {
                origin: n.map(|c| c * d),
                x_dir: u.map(f64::from),
                normal: n,
            }),
            replaces,
        )
    }

    /// An existing sketch, the model's step `i`.
    pub fn editing(sketch: Sketch, i: usize) -> Self {
        Self::new(sketch, Some(i))
    }

    /// What the shell is sent: the sketch and what is selected in it.
    pub fn scene(&self) -> Vec<u8> {
        postcard::to_allocvec(&SketchScene {
            sketch: self.sketch.clone(),
            selected: self.selected.clone(),
        })
        .unwrap_or_default()
    }

    /// The Fillet radius, from the value box: 0 lets the shell choose one to suit the view.
    pub fn value(&self) -> f32 {
        measure::length(self.value.as_str()).map_or(0.0, f32::abs)
    }

    /// Change the sketch from here: a step for undo, and the solver's answer expected.
    fn change(&mut self, next: Sketch) {
        self.undo.push(std::mem::replace(&mut self.sketch, next));
        self.redo.clear();
        self.settling = 2;
        self.prune();
    }

    pub fn undo(&mut self) {
        if let Some(prev) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.sketch, prev));
            self.settling = 2;
            self.prune();
        }
    }

    pub fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.sketch, next));
            self.settling = 2;
            self.prune();
        }
    }

    /// Drop selected items the sketch no longer has.
    fn prune(&mut self) {
        let geos = self.sketch.geos.len() as i32;
        let cons = self.sketch.constraints.len() as u32;
        self.selected.retain(|item| match *item {
            Item::Geo(g) => g < geos,
            Item::Point(p) => p.geo < geos,
            Item::Constraint(c) => c < cons,
        });
    }

    /// Take in one sketch event from the shell.
    pub fn event(&mut self, bytes: &[u8]) {
        let Ok(e) = postcard::from_bytes::<SketchEvent>(bytes) else {
            return;
        };
        match e {
            SketchEvent::Edited(s) => {
                if self.settling > 0 {
                    self.sketch = s;
                    self.prune();
                } else {
                    self.change(s);
                    self.settling = 0;
                }
            }
            SketchEvent::Picked(Some(item)) => {
                // FreeCAD toggles: clicking a selected thing deselects it.
                if let Some(i) = self.selected.iter().position(|s| *s == item) {
                    self.selected.remove(i);
                } else {
                    self.selected.push(item);
                }
            }
            SketchEvent::Picked(None) => self.selected.clear(),
        }
    }

    pub fn hint(&self, tool: ViewTool) -> &'static str {
        match tool {
            ViewTool::Point => "Click to place a point.",
            ViewTool::Line => "Click the start, then the end.",
            ViewTool::Polyline => {
                "Click each corner; click the first again, or the last twice, to finish."
            }
            ViewTool::Arc => "Click the centre, then the start, then the end (counter-clockwise).",
            ViewTool::Circle => "Click the centre, then a point on the circle.",
            ViewTool::Rect => "Click one corner, then the opposite corner.",
            ViewTool::Fillet => "Click a corner to round it, with the radius in Value.",
            ViewTool::Trim => "Click the part of a curve to cut away.",
            ViewTool::Extend => "Click near the end of a line or arc to lengthen it.",
            ViewTool::Split => "Click a line or arc where it should be cut in two.",
            ViewTool::External => "Click an edge of the model to copy it into the sketch.",
            ViewTool::Orbit => "Drag to orbit, shift-drag to pan, scroll to zoom.",
            _ => "Select geometry and constraints, then constrain it. Drag to move it.",
        }
    }

    /// Set the one selected dimension to what the value box says: millimetres, or degrees
    /// for an angle.
    fn set_value(&mut self) -> Result<(), &'static str> {
        let need = "Select one dimension, then type its value.";
        let [Item::Constraint(i)] = self.selected[..] else {
            return Err(need);
        };
        let c = *self.sketch.constraints.get(i as usize).ok_or(need)?;
        c.value().ok_or(need)?;
        let text = self.value.as_str().trim().trim_end_matches('°').to_owned();
        let is_angle = matches!(
            c,
            ccosel_proto::sketch::Constraint::Angle(..)
                | ccosel_proto::sketch::Constraint::AngleBetween(..)
        );
        let v = measure::length(&text).ok_or("Type a number.")?;
        let v = if is_angle {
            f64::from(v) * PI / 180.0
        } else {
            f64::from(v)
        };
        let mut next = self.sketch.clone();
        next.constraints[i as usize] = c.with_value(v);
        self.change(next);
        Ok(())
    }

    fn selected_geos(&self) -> Vec<i32> {
        self.selected
            .iter()
            .filter_map(|i| match *i {
                Item::Geo(g) if g >= 0 => Some(g),
                _ => None,
            })
            .collect()
    }

    /// FreeCAD's Sketcher toolbars: two rows of tools, one of constraints, and the value box.
    pub fn toolbars(&mut self, ui: &mut Ui<'_>, tool: &mut ViewTool) -> Out {
        self.settling = self.settling.saturating_sub(1);
        let mut out = Out::default();
        ui.horizontal(|ui| {
            if ui.button("✔ Close").clicked() {
                out.close = true;
            }
            ui.tooltip("Leave the sketch, keeping it as a step of the model.");
            if ui.button("✖ Cancel").clicked() {
                out.cancel = true;
            }
            ui.tooltip("Leave the sketch without keeping the changes.");
            if ui.button("View sketch").clicked() {
                out.face = true;
            }
            ui.tooltip("Look straight at the sketch.");
            ui.separator();
            let mut pick = |ui: &mut Ui<'_>, icon: &str, name: &str, t: ViewTool| {
                let mut label = String::from(icon);
                label.push(' ');
                label.push_str(name);
                if ui.selectable(*tool == t, &label).clicked() {
                    *tool = t;
                }
            };
            pick(ui, icons::CURSOR, "Select", ViewTool::Select);
            for (icon, name, t) in GEOMETRY {
                pick(ui, icon, name, t);
            }
        });
        ui.horizontal(|ui| {
            for (icon, name, t) in EDITING {
                let mut label = String::from(icon);
                label.push(' ');
                label.push_str(name);
                if ui.selectable(*tool == t, &label).clicked() {
                    *tool = t;
                }
            }
            ui.separator();
            if ui.button("Construction").clicked() {
                let geos = self.selected_geos();
                if geos.is_empty() {
                    out.message =
                        Some("Select geometry to switch to or from construction.".to_owned());
                } else {
                    let mut next = self.sketch.clone();
                    edit::toggle_construction(&mut next, &geos);
                    self.change(next);
                }
            }
            ui.tooltip("Switch the selected geometry between normal and construction.");
            if ui.button("Offset").clicked() {
                let geos = self.selected_geos();
                match measure::length(self.value.as_str()) {
                    Some(d) if !geos.is_empty() => {
                        let mut next = self.sketch.clone();
                        edit::offset(&mut next, &geos, f64::from(d));
                        self.change(next);
                    }
                    _ => {
                        out.message = Some(
                            "Select geometry and type the offset distance in Value.".to_owned(),
                        );
                    }
                }
            }
            ui.tooltip("Copy the selected geometry the distance in Value to one side.");
            if ui.button(icons::TRASH).clicked() {
                if self.selected.is_empty() {
                    out.message = Some("Select something to delete.".to_owned());
                } else {
                    let mut next = self.sketch.clone();
                    edit::delete(&mut next, &self.selected);
                    self.selected.clear();
                    self.change(next);
                }
            }
            ui.tooltip("Delete what is selected.");
        });
        ui.horizontal(|ui| {
            for (name, kind) in CONSTRAINTS {
                if ui.button(name).clicked() {
                    match constraints_for(kind, &self.selected, &self.sketch) {
                        Ok(cons) => {
                            let mut next = self.sketch.clone();
                            next.constraints.extend(cons);
                            self.selected.clear();
                            self.change(next);
                        }
                        Err(e) => out.message = Some(e.to_owned()),
                    }
                }
                ui.tooltip(kind.needs());
            }
        });
        ui.horizontal(|ui| {
            ui.label("Value");
            ui.text_edit(&mut self.value);
            ui.tooltip("A dimension's value (mm, or degrees for an angle), the Fillet radius, or the Offset distance.");
            if ui.button("Set").clicked()
                && let Err(e) = self.set_value()
            {
                out.message = Some(e.to_owned());
            }
            let n = self.selected.len();
            if n > 0 {
                let mut s = measure::itoa(n as u64);
                s.push_str(" selected");
                ui.label(&s);
            }
        });
        out
    }
}
