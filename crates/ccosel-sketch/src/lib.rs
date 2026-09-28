//! FreeCAD-style sketching, shared by the shell and apps.
//!
//! The shell uses it to solve a sketch while the user drags, and to run the drawing and
//! editing tools against the pointer; an app uses it to turn a selection into constraints.
//! One implementation, so what the user sees while editing is what the app then holds. The
//! data it works on is `ccosel_proto::sketch`, which the server hands to FreeCAD's own
//! Sketcher, the final word.

pub mod build;
pub mod edit;
pub mod geom;
pub mod solve;

pub use build::{Kind, constraints_for};
pub use solve::{Grab, Report, drag, measure, solve};
