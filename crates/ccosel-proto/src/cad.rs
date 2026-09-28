//! The Modeller's methods: a model is a list of operations, and the server turns it into
//! geometry with FreeCAD.
//!
//! The app never holds a B-rep. Its whole document is a `Vec<CadOp>` — a few bytes per push
//! or rectangle — which is what crosses the LAN, what undo pops, and what the server replays.
//! Regenerating is a pure function of that list, so the list itself is the request key: two
//! windows showing the same model share one job, and polling it can never start a second.
//!
//! The geometry comes back as a URL, not in the reply. The *shell* fetches that URL and draws
//! the mesh, so the triangles never enter the app's memory or pass through the RPC cache.
//!
//! Units are millimetres, FreeCAD's own.

use alloc::string::String;
use alloc::vec::Vec;
use serde::{Deserialize, Serialize};

use crate::{Coalesce, Command, Effect, Method, Query, Rpc};

/// More operations than this is not a model a person drew by hand; the server refuses it
/// rather than tying up the FreeCAD worker.
pub const MAX_OPS: usize = 2048;

/// Likewise for the corners of one polygon.
pub const MAX_POINTS: usize = 512;

/// One step in building a model.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum CadOp {
    /// A flat face through `points`, in order and not repeating the first, facing `normal`.
    /// Drawn on the ground it stands alone; drawn on a solid's face it marks out a region
    /// that a later push/pull cuts into or builds out from.
    Polygon {
        points: Vec<[f32; 3]>,
        normal: [f32; 3],
    },
    /// Move face `face` of the model *as it stood after the first `after` operations* by
    /// `distance` along its outward normal: out to build, in (negative) to cut.
    ///
    /// Faces are numbered afresh every time the model is rebuilt, so a face number only means
    /// something alongside the model it was read from. `after` names that model; the server
    /// replays to it, finds the face, and works geometrically from there — which is why a
    /// face number never goes stale as more operations are added.
    PushPull {
        after: u32,
        face: u32,
        distance: f32,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RegenReq {
    pub ops: Vec<CadOp>,
}

/// What a finished rebuild produced.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Model {
    /// Where the shell fetches the mesh: content-addressed and immutable, so the browser
    /// caches it and an undo back to an earlier model costs no download.
    pub mesh: String,
    /// Triangles in the mesh. A viewport left to choose renders a heavy model on the server
    /// rather than in the browser; see `ccosel_abi::view3d::Render`.
    pub triangles: u32,
    pub faces: u32,
    pub solids: u32,
    /// Cubic millimetres, rounded. For the status line: an app shows it without formatting a
    /// float.
    pub volume_mm3: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum RegenResult {
    Ok(Model),
    /// Operation `op` could not be applied (the index into `ops`), with FreeCAD's reason.
    /// The app can undo back past it.
    Failed {
        op: u32,
        message: String,
    },
}

/// Where a rebuild has got to. A poll answers from the server's job table without waiting.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RegenStatus {
    pub finished: bool,
    pub elapsed_ms: u64,
    /// Present exactly when `finished`.
    pub result: Option<RegenResult>,
}

pub struct Regenerate;

impl Rpc for Regenerate {
    const METHOD: Method = Method::Regenerate;
    const COALESCE: Coalesce = Coalesce::ByArgs;
    // Honest: the same ops always mean the same job and the same model.
    const EFFECT: Effect = Effect::Idempotent;
    const DEADLINE_MS: u32 = 8_000;
    type Req<'a> = RegenReq;
    type Reply = RegenStatus;
}

impl Query for Regenerate {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExportFormat {
    /// ISO 10303, what other CAD programs open.
    Step,
    /// A triangle mesh, what slicers for 3D printers open.
    Stl,
    /// FreeCAD's own document, to carry on in desktop FreeCAD.
    FreeCad,
}

impl ExportFormat {
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Step => "step",
            Self::Stl => "stl",
            Self::FreeCad => "FCStd",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExportReq {
    pub ops: Vec<CadOp>,
    pub format: ExportFormat,
    /// A jail path to write, such as `/home/ada/bracket.step`. Written with the caller's
    /// permissions, exactly like `WriteFile`.
    pub path: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Exported {
    pub path: String,
    pub bytes: u64,
}

pub struct ExportModel;

impl Rpc for ExportModel {
    const METHOD: Method = Method::ExportModel;
    const COALESCE: Coalesce = Coalesce::None;
    // It writes a file.
    const EFFECT: Effect = Effect::Effectful;
    // Rebuilds from scratch if the model is not cached, then writes.
    const DEADLINE_MS: u32 = 60_000;
    type Req<'a> = ExportReq;
    type Reply = Exported;
}

impl Command for ExportModel {}

/// A tessellated model, as the server serves it at a [`Model::mesh`] URL and the shell draws
/// it. postcard, like everything else on the wire.
///
/// Triangles are wound counter-clockwise seen from outside. Faces are the model's B-rep faces,
/// numbered as `PushPull::face` expects; each triangle says which one it belongs to.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MeshData {
    pub positions: Vec<[f32; 3]>,
    pub triangles: Vec<[u32; 3]>,
    /// Per triangle.
    pub tri_face: Vec<u32>,
    /// Per face: its outward normal where it is flat, zero where it is curved (a curved face
    /// cannot be pushed or pulled).
    pub face_normals: Vec<[f32; 3]>,
    /// Per face: whether it stands alone rather than bounding a solid. Loose faces are drawn
    /// over a coplanar solid face, which is what a rectangle drawn onto a box is.
    pub loose: Vec<bool>,
    /// The model's edges, as line segments.
    pub edges: Vec<[[f32; 3]; 2]>,
    /// Corners and edge midpoints, which the pointer snaps to.
    pub vertices: Vec<[f32; 3]>,
    pub midpoints: Vec<[f32; 3]>,
}

impl MeshData {
    /// Whether every index points somewhere and every per-item list is the right length. The
    /// shell checks this before drawing anything, so a corrupt mesh fails once, cleanly,
    /// instead of indexing out of bounds every frame.
    pub fn is_consistent(&self) -> bool {
        let n = self.positions.len() as u32;
        let faces = self.face_normals.len() as u32;
        self.tri_face.len() == self.triangles.len()
            && self.loose.len() == self.face_normals.len()
            && self.triangles.iter().flatten().all(|&i| i < n)
            && self.tri_face.iter().all(|&f| f < faces)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square() -> MeshData {
        MeshData {
            positions: alloc::vec![[0.0; 3], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]],
            triangles: alloc::vec![[0, 1, 2], [0, 2, 3]],
            tri_face: alloc::vec![0, 0],
            face_normals: alloc::vec![[0.0, 0.0, 1.0]],
            loose: alloc::vec![true],
            ..MeshData::default()
        }
    }

    #[test]
    fn a_well_formed_mesh_is_consistent() {
        assert!(square().is_consistent());
        assert!(MeshData::default().is_consistent());
    }

    #[test]
    fn bad_indices_and_lengths_are_caught() {
        let mut m = square();
        m.triangles[1][2] = 4;
        assert!(!m.is_consistent());

        let mut m = square();
        m.tri_face[0] = 1;
        assert!(!m.is_consistent());

        let mut m = square();
        m.tri_face.pop();
        assert!(!m.is_consistent());

        let mut m = square();
        m.loose.push(false);
        assert!(!m.is_consistent());
    }

    #[test]
    fn ops_round_trip_through_postcard() {
        let req = RegenReq {
            ops: alloc::vec![
                CadOp::Polygon {
                    points: alloc::vec![[0.0; 3], [10.0, 0.0, 0.0], [10.0, 10.0, 0.0]],
                    normal: [0.0, 0.0, 1.0],
                },
                CadOp::PushPull {
                    after: 1,
                    face: 0,
                    distance: 25.0,
                },
            ],
        };
        let bytes = postcard::to_allocvec(&req).unwrap();
        assert_eq!(postcard::from_bytes::<RegenReq>(&bytes).unwrap(), req);
    }
}

/// An orbit camera: it looks at `target` from `distance` away, swung `azimuth` radians about
/// the Z axis (0 looks from +X) and raised `elevation` radians above the ground. Z is up, as
/// in FreeCAD and SketchUp.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CameraState {
    pub target: [f32; 3],
    pub distance: f32,
    pub azimuth: f32,
    pub elevation: f32,
    /// Vertical field of view, radians.
    pub fov_y: f32,
}

impl Default for CameraState {
    fn default() -> Self {
        Self {
            target: [0.0, 0.0, 0.0],
            distance: 5000.0,
            // Looking from the front-left and a little above: SketchUp's opening view.
            azimuth: -2.2,
            elevation: 0.45,
            fov_y: 0.6,
        }
    }
}

/// `POST /cad/render`: draw the mesh at `mesh` (a [`Model::mesh`] URL) from `camera`, into a
/// `width` × `height` pixel viewport. The reply body is a postcard [`crate::scene2d::Scene2D`].
///
/// A plain HTTP endpoint rather than an RPC method: the *shell* sends it, on the app's
/// behalf, when a viewport renders on the server. The app never sees a frame, just as it never
/// sees a mesh.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RenderReq {
    pub mesh: String,
    pub camera: CameraState,
    pub width: u16,
    pub height: u16,
}
