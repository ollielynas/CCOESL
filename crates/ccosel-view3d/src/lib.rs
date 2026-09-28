//! The 3D viewport's geometry, shared by the shell and the server.
//!
//! Everything here is plain `f32` arithmetic on a [`ccosel_proto::cad::MeshData`]: an orbit
//! camera, ray casting, SketchUp-style inference snapping, the push/pull drag, and a renderer
//! that turns a mesh and a camera into a [`ccosel_proto::scene2d::Scene2D`] of flat triangles
//! and visible edges.
//!
//! It depends on neither egui nor a GPU, for two reasons:
//!
//! - **One renderer, two places.** The shell runs [`render::render`] itself for an ordinary
//!   model, so orbiting costs no network. For a model too heavy for the browser, the server
//!   runs the very same function and sends the frame. The two can never draw differently.
//! - **Testable without a browser.** Picking and snapping are where a modeller feels right or
//!   wrong, and every rule here is checked by a unit test against a hand-built mesh.

pub mod camera;
pub mod math;
pub mod mesh;
pub mod pick;
pub mod render;

pub use camera::Camera;
pub use mesh::Mesh;
pub use pick::{Hit, PushPull, Resolved, raycast, resolve};
pub use render::{Style, render};
