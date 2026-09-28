//! The shell's half of `Viewport3d`: the camera, the pointer, and drawing.
//!
//! The guest names a mesh by URL and picks a tool; everything that follows the pointer happens
//! here, at display rate, with no network and no guest round trip:
//!
//! - **Camera.** Middle-drag orbits (shift pans), the wheel zooms toward the pointer, and the
//!   primary button orbits too with the Orbit or Select tool. The camera lives in
//!   [`ViewState`], never in the guest.
//! - **Pointer.** Every frame the pointer is resolved against the mesh with SketchUp's
//!   inferences (`ccosel_view3d::resolve`), and the cue is drawn: a coloured dot, a label.
//! - **Gestures.** A click, or a finished push/pull, becomes one `VIEWPORT` event for the
//!   guest. The push/pull's extrusion is drawn live during the drag.
//! - **Drawing.** `ccosel_view3d::render` turns mesh and camera into flat triangles, which
//!   become one `egui::Mesh`. Heavy meshes can be rendered by the server instead (`Render`);
//!   the shell then draws the server's frame and asks for a new one whenever the camera moves.
//!
//! This module never touches the network. It says what it wants — [`Replayer::take_mesh_fetches`],
//! [`Replayer::take_render_jobs`] — and the shell answers. That keeps it testable against an
//! offscreen `egui::Context`, like the rest of the host.
//!
//! [`Replayer::take_mesh_fetches`]: crate::Replayer::take_mesh_fetches
//! [`Replayer::take_render_jobs`]: crate::Replayer::take_render_jobs

use std::collections::HashMap;
use std::sync::Arc;

use ccosel_abi::event::event_kind;
use ccosel_abi::view3d::{
    Extrude, NO_FACE, Render, Snap, StdView, ViewAction, ViewEvent, ViewTool, Viewport,
    encode_sketch_event, encode_view_event, rect_corners,
};
use ccosel_proto::cad::{MeshData, RenderReq};
use ccosel_proto::scene2d::{Scene2D, unpack};
use ccosel_view3d::{Camera, Mesh, PushPull, Resolved, Style, raycast, render, resolve};

/// Above this many triangles, `Render::Auto` hands drawing to the server. A browser tab
/// sorts and paints this many flat triangles comfortably inside a frame; past it, orbiting
/// starts to stutter on a modest laptop, and a round trip per camera move is the better deal.
pub const AUTO_SERVER_TRIANGLES: usize = 60_000;

/// A mesh nobody has shown for this many frames is dropped. The browser's HTTP cache still
/// holds its bytes (mesh URLs are immutable), so an undo back to it costs a decode, not a
/// download.
const MESH_IDLE_FRAMES: u64 = 3_600;

/// How long a released push/pull keeps being drawn if the guest never takes it over with its
/// own preview, so an app that ignores the gesture does not leave a ghost on screen.
const RELEASED_FRAMES: u64 = 30;

const BACKGROUND: egui::Color32 = egui::Color32::from_rgb(0xee, 0xf1, 0xf4);

enum MeshSlot {
    Loading,
    Ready(Arc<Mesh>),
    Failed(String),
}

struct MeshEntry {
    slot: MeshSlot,
    last_used: u64,
}

/// A server render the shell has asked for, or one that has come back.
#[derive(Clone, Debug, PartialEq)]
pub struct RenderJob {
    /// Hand back to [`crate::Replayer::render_done`] with the reply.
    pub key: u64,
    pub req: RenderReq,
}

/// Everything the shell keeps for one viewport, across frames.
#[derive(Default)]
pub(crate) struct ViewState {
    camera: Camera,
    /// Framed on the first mesh it showed; later meshes keep the user's view.
    framed: bool,
    shown: Option<(String, Arc<Mesh>)>,
    /// The preview drawn last frame, kept while a new mesh loads so the result of a push/pull
    /// stays up until the geometry that replaces it does.
    last_preview: Option<Extrude>,
    /// A push/pull just released: drawn until the guest takes over with its own preview.
    released: Option<(Extrude, u64)>,
    drag: Option<PushPull>,
    hover: Option<Resolved>,
    local: Option<(u64, Arc<Scene2D>)>,
    server: ServerFrames,
    /// The last standard-view command obeyed.
    view_seq: u32,
    sketch: crate::sketch_view::SketchState,
}

#[derive(Default)]
struct ServerFrames {
    /// The frame on screen and the request that produced it.
    latest: Option<(RenderReq, Arc<Scene2D>)>,
    in_flight: Option<(u64, RenderReq)>,
    error: Option<String>,
}

/// Viewport state for one app, owned by its `Replayer`.
#[derive(Default)]
pub(crate) struct Views {
    views: HashMap<u64, ViewState>,
    meshes: HashMap<String, MeshEntry>,
    fetches: Vec<String>,
    jobs: Vec<RenderJob>,
    /// Render key -> viewport id, for routing a server frame back.
    job_view: HashMap<u64, u64>,
    next_key: u64,
    /// Finished gestures and sketch edits, as `(event kind, payload)`.
    events: Vec<(u32, Vec<u8>)>,
    frame: u64,
}

impl Views {
    pub(crate) fn begin_frame(&mut self) {
        self.frame += 1;
        let frame = self.frame;
        let shown: Vec<&str> = self
            .views
            .values()
            .filter_map(|v| v.shown.as_ref().map(|(u, _)| u.as_str()))
            .collect();
        let keep: Vec<String> = self
            .meshes
            .iter()
            .filter(|(url, e)| {
                frame - e.last_used < MESH_IDLE_FRAMES || shown.contains(&url.as_str())
            })
            .map(|(u, _)| u.clone())
            .collect();
        self.meshes.retain(|u, _| keep.contains(u));
    }

    pub(crate) fn take_fetches(&mut self) -> Vec<String> {
        std::mem::take(&mut self.fetches)
    }

    pub(crate) fn take_jobs(&mut self) -> Vec<RenderJob> {
        std::mem::take(&mut self.jobs)
    }

    pub(crate) fn take_events(&mut self) -> Vec<(u32, Vec<u8>)> {
        std::mem::take(&mut self.events)
    }

    pub(crate) fn mesh_done(&mut self, url: &str, bytes: Result<&[u8], String>) {
        let slot = match bytes.and_then(|b| {
            let data: MeshData =
                postcard::from_bytes(b).map_err(|e| format!("mesh {url} does not decode: {e}"))?;
            Mesh::new(data).map_err(|e| format!("mesh {url}: {e}"))
        }) {
            Ok(mesh) => MeshSlot::Ready(Arc::new(mesh)),
            Err(e) => MeshSlot::Failed(e),
        };
        let frame = self.frame;
        self.meshes.insert(
            url.to_owned(),
            MeshEntry {
                slot,
                last_used: frame,
            },
        );
    }

    pub(crate) fn render_done(&mut self, key: u64, bytes: Result<&[u8], String>) {
        let Some(view_id) = self.job_view.remove(&key) else {
            return;
        };
        let Some(view) = self.views.get_mut(&view_id) else {
            return;
        };
        let Some((k, req)) = view.server.in_flight.take_if(|(k, _)| *k == key) else {
            return;
        };
        debug_assert_eq!(k, key);
        match bytes.and_then(|b| {
            postcard::from_bytes::<Scene2D>(b).map_err(|e| format!("server frame: {e}"))
        }) {
            Ok(scene) if scene.is_consistent() => {
                view.server.latest = Some((req, Arc::new(scene)));
                view.server.error = None;
            }
            Ok(_) => view.server.error = Some("server frame is corrupt".to_owned()),
            Err(e) => view.server.error = Some(e),
        }
    }

    /// The mesh at `url`, asking the shell for it the first time it is wanted.
    fn mesh(&mut self, url: &str) -> Result<Option<Arc<Mesh>>, String> {
        let frame = self.frame;
        let entry = self.meshes.entry(url.to_owned()).or_insert_with(|| {
            self.fetches.push(url.to_owned());
            MeshEntry {
                slot: MeshSlot::Loading,
                last_used: frame,
            }
        });
        entry.last_used = frame;
        match &entry.slot {
            MeshSlot::Loading => Ok(None),
            MeshSlot::Ready(m) => Ok(Some(m.clone())),
            MeshSlot::Failed(e) => Err(e.clone()),
        }
    }

    /// Lay out, handle input for, and draw one viewport.
    pub(crate) fn show(
        &mut self,
        ui: &mut egui::Ui,
        id: u64,
        view: &Viewport<'_>,
        sense_id: egui::Id,
    ) -> egui::Response {
        let mut size = egui::vec2(view.size.x, view.size.y);
        if size.x.is_nan() || size.x <= 0.0 {
            size.x = ui.available_width();
        }
        if size.y.is_nan() || size.y <= 0.0 {
            let visible = ui.clip_rect().bottom() - ui.cursor().top();
            size.y = (visible - ui.spacing().item_spacing.y).max(200.0);
        }
        size = size.clamp(egui::vec2(40.0, 40.0), egui::vec2(8192.0, 8192.0));
        let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
        let r = ui.interact(rect, sense_id, egui::Sense::click_and_drag());

        let requested = if view.mesh.is_empty() {
            Ok(None)
        } else {
            self.mesh(view.mesh)
        };
        let frame = self.frame;
        let state = self.views.entry(id).or_default();
        let aspect = rect.width() / rect.height();
        let mut problem = None;
        match requested {
            Ok(Some(mesh)) => {
                if !state.framed && mesh.radius > 0.0 {
                    state.camera.fit(mesh.center, mesh.radius, aspect);
                    state.framed = true;
                }
                let changed = state.shown.as_ref().is_none_or(|(u, _)| u != view.mesh);
                if changed {
                    state.released = None;
                }
                state.shown = Some((view.mesh.to_owned(), mesh));
            }
            Ok(None) if view.mesh.is_empty() => state.shown = None,
            Ok(None) => {}
            Err(e) => problem = Some(e),
        }
        let loading =
            !view.mesh.is_empty() && state.shown.as_ref().is_none_or(|(u, _)| u != view.mesh);

        if view.view.seq != state.view_seq {
            state.view_seq = view.view.seq;
            let mesh = state.shown.as_ref().map(|(_, m)| m.clone());
            let plane = state.sketch_plane(view);
            apply_view(
                &mut state.camera,
                view.view.view,
                mesh.as_deref(),
                plane,
                aspect,
            );
        }
        let sketching = !view.sketch.is_empty();
        let events = state.input(ui, &r, rect, id, view, frame);
        self.events.extend(
            events
                .iter()
                .map(|e| (event_kind::VIEWPORT, encode_view_event(e))),
        );
        let state = self.views.get_mut(&id).expect("inserted above");

        // The preview: the guest's own, else the one kept while a new mesh loads, else a
        // push/pull the user has only just let go of.
        let preview = view
            .preview
            .or(if loading { state.last_preview } else { None })
            .or(state
                .released
                .filter(|(_, at)| frame - at < RELEASED_FRAMES)
                .map(|(p, _)| p));
        if view.preview.is_some() {
            state.last_preview = view.preview;
            state.released = None;
        } else if !loading {
            state.last_preview = None;
        }

        let mesh = state.shown.as_ref().map(|(_, m)| m.clone());
        let server = match view.render {
            Render::Local => false,
            Render::Server => true,
            Render::Auto => mesh
                .as_ref()
                .is_some_and(|m| m.triangle_count() > AUTO_SERVER_TRIANGLES),
        };

        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, BACKGROUND);
        let (w, h) = (rect.width().round() as u16, rect.height().round() as u16);
        let drag_preview = state.drag.map(|d| Extrude {
            face: d.face,
            distance: d.distance,
        });
        let extrusion = drag_preview
            .or(preview)
            .zip(mesh.as_ref())
            .map(|(p, m)| m.extrusion(p.face, p.distance))
            .unwrap_or_default();
        let hover_face = match view.tool {
            ViewTool::Select | ViewTool::PushPull if state.drag.is_none() => {
                state.hover.map_or(NO_FACE, |h| h.face)
            }
            _ => NO_FACE,
        };

        if server {
            let shown_url = state.shown.as_ref().map(|(u, _)| u.clone());
            if let Some(url) = shown_url {
                let want = RenderReq {
                    mesh: url,
                    camera: state.camera.state,
                    width: w,
                    height: h,
                };
                let have = state.server.latest.as_ref().map(|(r, _)| r);
                let asked = state.server.in_flight.as_ref().map(|(_, r)| r);
                // One frame in flight at a time; while the camera moves, the next request goes
                // out as soon as the previous frame lands, so the view keeps up at whatever
                // rate the link allows instead of queueing stale positions.
                if have != Some(&want) && asked.is_none() {
                    self.next_key += 1;
                    let key = self.next_key;
                    state.server.in_flight = Some((key, want.clone()));
                    self.job_view.insert(key, id);
                    self.jobs.push(RenderJob { key, req: want });
                }
            }
            if let Some((_, scene)) = &state.server.latest {
                paint_scene(&painter, rect.min, scene);
            }
            // What changes with the pointer is still drawn here, over the server's frame.
            let mut overlay = extrusion;
            if let Some(m) = &mesh {
                for f in [hover_face, view.selected] {
                    overlay.extend(m.face_tris(f).map(|t| m.tri(t)));
                }
            }
            let over = render(
                None,
                &state.camera,
                w,
                h,
                &Style {
                    extra: overlay,
                    axes: false,
                    ..Style::default()
                },
            );
            paint_scene(&painter, rect.min, &over);
            if let Some(e) = &state.server.error {
                problem.get_or_insert_with(|| e.clone());
            }
        } else {
            let style = Style {
                highlight: hover_face,
                selected: view.selected,
                extra: extrusion,
                axes: true,
            };
            let key = scene_key(&state.camera, w, h, mesh.as_deref(), &style);
            let scene = match &state.local {
                Some((k, s)) if *k == key => s.clone(),
                _ => {
                    let s = Arc::new(render(mesh.as_deref(), &state.camera, w, h, &style));
                    state.local = Some((key, s.clone()));
                    s
                }
            };
            paint_scene(&painter, rect.min, &scene);
        }

        if sketching {
            let mesh = state.shown.as_ref().map(|(_, m)| m.clone());
            let ViewState { sketch, camera, .. } = state;
            for e in sketch.show(ui, &r, rect, camera, mesh.as_deref(), view, frame) {
                if let Ok(body) = postcard::to_allocvec(&e) {
                    self.events
                        .push((event_kind::SKETCH, encode_sketch_event(id, &body)));
                }
            }
            if let Some(e) = &sketch.error {
                problem.get_or_insert_with(|| e.clone());
            }
        } else {
            state.paint_overlay(&painter, rect, view);
        }
        if let Some(p) = problem {
            painter.text(
                rect.left_bottom() + egui::vec2(8.0, -8.0),
                egui::Align2::LEFT_BOTTOM,
                p,
                egui::FontId::proportional(12.0),
                egui::Color32::from_rgb(0xb9, 0x1c, 0x1c),
            );
        } else if loading && mesh.is_none() {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Loading model…",
                egui::FontId::proportional(13.0),
                egui::Color32::from_gray(90),
            );
        }
        r
    }
}

impl ViewState {
    /// The plane of the sketch the app is showing, if it is showing one.
    fn sketch_plane(&self, view: &Viewport<'_>) -> Option<ccosel_proto::sketch::Plane> {
        if view.sketch.is_empty() {
            return None;
        }
        postcard::from_bytes::<ccosel_proto::sketch::SketchScene>(view.sketch)
            .ok()
            .map(|s| s.sketch.plane)
    }
}

/// FreeCAD's standard views. Z is up; "front" looks along +Y, as in FreeCAD.
fn apply_view(
    cam: &mut Camera,
    view: StdView,
    mesh: Option<&Mesh>,
    plane: Option<ccosel_proto::sketch::Plane>,
    aspect: f32,
) {
    use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, PI};
    let top = 1.55;
    let (azimuth, elevation) = match view {
        StdView::None => return,
        StdView::Fit => {
            if let Some(m) = mesh.filter(|m| m.radius > 0.0) {
                cam.fit(m.center, m.radius, aspect);
            }
            return;
        }
        StdView::Sketch => {
            if let Some(p) = plane {
                crate::sketch_view::face(cam, &p);
            }
            return;
        }
        StdView::Isometric => (-FRAC_PI_4, 0.6155),
        StdView::Front => (-FRAC_PI_2, 0.0),
        StdView::Rear => (FRAC_PI_2, 0.0),
        StdView::Right => (0.0, 0.0),
        StdView::Left => (PI, 0.0),
        StdView::Top => (-FRAC_PI_2, top),
        StdView::Bottom => (-FRAC_PI_2, -top),
    };
    cam.state.azimuth = azimuth;
    cam.state.elevation = elevation;
}

/// A cheap fingerprint of everything a local render depends on, so an idle viewport is not
/// re-rendered every frame.
fn scene_key(cam: &Camera, w: u16, h: u16, mesh: Option<&Mesh>, style: &Style) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let s = cam.state;
    for f in s
        .target
        .iter()
        .chain([&s.distance, &s.azimuth, &s.elevation, &s.fov_y])
    {
        f.to_bits().hash(&mut hasher);
    }
    (w, h, style.highlight, style.selected, style.axes).hash(&mut hasher);
    (mesh.map(|m| m as *const Mesh as usize)).hash(&mut hasher);
    for t in &style.extra {
        for p in t {
            for c in p {
                c.to_bits().hash(&mut hasher);
            }
        }
    }
    hasher.finish()
}

fn color(c: u32) -> egui::Color32 {
    let [r, g, b, a] = unpack(c);
    egui::Color32::from_rgba_unmultiplied(r, g, b, a)
}

/// Draw a frame at `origin`: one mesh for every triangle, then the edges over it.
fn paint_scene(painter: &egui::Painter, origin: egui::Pos2, scene: &Scene2D) {
    let at = |i: u32| {
        let p = scene.px(i);
        origin + egui::vec2(p[0], p[1])
    };
    let mut mesh = egui::Mesh::default();
    for (t, c) in scene.tris.iter().zip(&scene.tri_colors) {
        let base = mesh.vertices.len() as u32;
        for &i in t {
            mesh.colored_vertex(at(i), color(*c));
        }
        mesh.add_triangle(base, base + 1, base + 2);
    }
    painter.add(egui::Shape::mesh(mesh));
    for (l, c) in scene.lines.iter().zip(&scene.line_colors) {
        painter.line_segment([at(l[0]), at(l[1])], egui::Stroke::new(1.2, color(*c)));
    }
}

fn snap_cue(snap: Snap) -> Option<(&'static str, egui::Color32)> {
    Some(match snap {
        Snap::Free => return None,
        Snap::OnFace => ("On Face", egui::Color32::from_rgb(0x25, 0x63, 0xeb)),
        Snap::Endpoint => ("Endpoint", egui::Color32::from_rgb(0x16, 0xa3, 0x4a)),
        Snap::Midpoint => ("Midpoint", egui::Color32::from_rgb(0x06, 0xb6, 0xd4)),
        Snap::AxisX => ("On Red Axis", egui::Color32::from_rgb(0xdc, 0x26, 0x26)),
        Snap::AxisY => ("On Green Axis", egui::Color32::from_rgb(0x16, 0xa3, 0x4a)),
        Snap::AxisZ => ("On Blue Axis", egui::Color32::from_rgb(0x25, 0x63, 0xeb)),
    })
}

/// Millimetres, rounded, the way SketchUp's measurement box shows a length.
fn mm(v: f32) -> String {
    format!("{:.0} mm", v)
}

impl ViewState {
    fn resolve_at(
        &self,
        local: egui::Pos2,
        rect: egui::Rect,
        anchor: bool,
        view: &Viewport<'_>,
    ) -> Option<Resolved> {
        let mesh = self.shown.as_ref().map(|(_, m)| m.as_ref());
        resolve(
            mesh,
            &self.camera,
            rect.width(),
            rect.height(),
            [local.x, local.y],
            if anchor { view.anchor } else { None },
        )
    }

    fn input(
        &mut self,
        ui: &mut egui::Ui,
        r: &egui::Response,
        rect: egui::Rect,
        id: u64,
        view: &Viewport<'_>,
        frame: u64,
    ) -> Vec<ViewEvent> {
        let sketching = !view.sketch.is_empty();
        let mut events = Vec::new();
        let (w, h) = (rect.width(), rect.height());
        let pointer = r.hover_pos().or(r.interact_pointer_pos());
        let local = pointer.map(|p| p - rect.min.to_vec2());
        let drawing = matches!(view.tool, ViewTool::Line | ViewTool::Rect);

        self.hover = match local {
            _ if sketching => None,
            Some(p) if r.contains_pointer() || r.dragged() => {
                self.resolve_at(p, rect, drawing, view)
            }
            _ => None,
        };

        // Wheel: zoom toward what is under the pointer. Taken out of the input so the window
        // around the viewport does not scroll as well.
        if r.contains_pointer() {
            let dy = ui.input(|i| i.smooth_scroll_delta.y);
            if dy != 0.0 {
                ui.input_mut(|i| i.smooth_scroll_delta = egui::Vec2::ZERO);
                let toward = self.hover.map(|h| h.point);
                self.camera.zoom((-dy * 0.0025).exp(), toward);
            }
        }

        let delta = r.drag_delta();
        let shift = ui.input(|i| i.modifiers.shift);
        let orbit_with_primary = if sketching {
            view.tool == ViewTool::Orbit
        } else {
            matches!(view.tool, ViewTool::Orbit | ViewTool::Select)
        };
        if r.dragged_by(egui::PointerButton::Middle)
            || (r.dragged_by(egui::PointerButton::Primary) && orbit_with_primary)
        {
            if shift {
                self.camera.pan(delta.x, delta.y, h);
            } else {
                self.camera.orbit(delta.x, delta.y);
            }
        }

        if sketching {
            self.drag = None;
            return events;
        }
        let mesh = self.shown.as_ref().map(|(_, m)| m.clone());
        if view.tool == ViewTool::PushPull {
            if r.drag_started_by(egui::PointerButton::Primary)
                && let (Some(m), Some(p)) = (&mesh, r.interact_pointer_pos())
            {
                // Where the press was, not where the pointer is now: egui reports a drag
                // start only once the pointer has moved a few pixels.
                let press =
                    ui.input(|i| i.pointer.press_origin()).unwrap_or(p) - rect.min.to_vec2();
                let ray = self.camera.ray(press.x, press.y, w, h);
                self.drag = raycast(m, &ray).and_then(|hit| PushPull::start(m, &hit));
            }
            if let (Some(drag), Some(m), Some(p)) = (&mut self.drag, &mesh, local) {
                drag.update(m, &self.camera, w, h, [p.x, p.y]);
            }
            if r.drag_stopped_by(egui::PointerButton::Primary)
                && let Some(drag) = self.drag.take()
                && drag.distance.abs() > 1e-3
            {
                self.released = Some((
                    Extrude {
                        face: drag.face,
                        distance: drag.distance,
                    },
                    frame,
                ));
                events.push(ViewEvent {
                    id,
                    action: ViewAction::PushPull,
                    snap: if drag.snapped {
                        Snap::Endpoint
                    } else {
                        Snap::OnFace
                    },
                    face: drag.face,
                    point: drag.origin,
                    normal: drag.normal,
                    distance: drag.distance,
                });
            }
        } else {
            self.drag = None;
        }

        if r.clicked()
            && matches!(
                view.tool,
                ViewTool::Select | ViewTool::Line | ViewTool::Rect
            )
            && let Some(hit) = self.hover
        {
            events.push(ViewEvent {
                id,
                action: ViewAction::Click,
                snap: hit.snap,
                face: hit.face,
                point: hit.point,
                normal: hit.normal,
                distance: 0.0,
            });
        }
        events
    }

    /// What follows the pointer: the rubber band, the rectangle being drawn, the snap cue, and
    /// the push/pull distance. Drawn over the model, in pixels.
    fn paint_overlay(&self, painter: &egui::Painter, rect: egui::Rect, view: &Viewport<'_>) {
        let (w, h) = (rect.width(), rect.height());
        let to_screen = |p: [f32; 3]| {
            self.camera
                .project(p, w, h)
                .map(|s| rect.min + egui::vec2(s[0], s[1]))
        };
        let ink = egui::Stroke::new(1.6, egui::Color32::from_rgb(0x11, 0x18, 0x27));
        let mut chain: Vec<[f32; 3]> = view.path_points().collect();
        if let Some(h) = self.hover {
            match (view.tool, view.anchor) {
                (ViewTool::Line, Some(a)) => {
                    if chain.is_empty() {
                        chain.push(a.point);
                    }
                    chain.push(h.point);
                }
                (ViewTool::Rect, Some(a)) => {
                    let c = rect_corners(a.point, a.normal, h.point);
                    chain = vec![c[0], c[1], c[2], c[3], c[0]];
                }
                _ => {}
            }
        }
        for pair in chain.windows(2) {
            if let (Some(a), Some(b)) = (to_screen(pair[0]), to_screen(pair[1])) {
                painter.line_segment([a, b], ink);
            }
        }

        if let Some(drag) = self.drag {
            if let Some(p) = to_screen(ccosel_view3d::math::add_scaled(
                drag.origin,
                drag.normal,
                drag.distance,
            )) {
                painter.text(
                    p + egui::vec2(12.0, -12.0),
                    egui::Align2::LEFT_BOTTOM,
                    mm(drag.distance),
                    egui::FontId::proportional(12.0),
                    ink.color,
                );
            }
            return;
        }
        let Some(h) = self.hover else { return };
        let Some((label, color)) = snap_cue(h.snap) else {
            return;
        };
        if view.tool == ViewTool::Select || view.tool == ViewTool::Orbit {
            return;
        }
        if let Some(p) = to_screen(h.point) {
            painter.circle(p, 4.5, color, egui::Stroke::new(1.0, egui::Color32::WHITE));
            painter.text(
                p + egui::vec2(10.0, 10.0),
                egui::Align2::LEFT_TOP,
                label,
                egui::FontId::proportional(11.0),
                color,
            );
        }
    }
}
