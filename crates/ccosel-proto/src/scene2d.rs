//! A rendered viewport frame as flat 2D primitives: what `ccosel-view3d` produces from a mesh
//! and a camera, and what the shell draws with `egui::Painter`.
//!
//! The same frame comes from either side. Rendered locally, it never leaves the shell;
//! rendered on the server (for a model too heavy to draw well in a browser tab), it arrives as
//! the body of `POST /cad/render`. Either way the shell draws it identically, so switching
//! modes changes where the work happens, not what the user sees.
//!
//! Coordinates are in eighths of a pixel from the viewport's top-left corner, as integers:
//! postcard writes those as short varints, where `f32`s would cost four bytes each.

use alloc::vec::Vec;
use serde::{Deserialize, Serialize};

/// Units per pixel in [`Scene2D::points`].
pub const SUBPIXELS: f32 = 8.0;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Scene2D {
    pub width: u16,
    pub height: u16,
    /// Screen positions, in [`SUBPIXELS`] per pixel.
    pub points: Vec<[i32; 2]>,
    /// Filled triangles, indexing `points`, **already in drawing order**: back to front.
    pub tris: Vec<[u32; 3]>,
    /// Per triangle: `0xAABBGGRR`, as [`rgba`] builds it.
    pub tri_colors: Vec<u32>,
    /// Visible edge segments, indexing `points`, drawn over every triangle. Hidden parts of
    /// edges have already been cut away.
    pub lines: Vec<[u32; 2]>,
    /// Per line, packed like `tri_colors`: black for model edges, red/green/blue for the axes.
    pub line_colors: Vec<u32>,
}

/// Pack a colour the way [`Scene2D::tri_colors`] stores it.
pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> u32 {
    u32::from_le_bytes([r, g, b, a])
}

/// Unpack a colour from [`Scene2D::tri_colors`].
pub const fn unpack(c: u32) -> [u8; 4] {
    c.to_le_bytes()
}

impl Scene2D {
    /// A point in pixels.
    pub fn px(&self, i: u32) -> [f32; 2] {
        let p = self.points[i as usize];
        [p[0] as f32 / SUBPIXELS, p[1] as f32 / SUBPIXELS]
    }

    /// Whether every index points somewhere and there is a colour per triangle. The shell
    /// checks a frame from the server before drawing it, so a corrupt one fails once instead
    /// of indexing out of bounds.
    pub fn is_consistent(&self) -> bool {
        let n = self.points.len() as u32;
        self.tri_colors.len() == self.tris.len()
            && self.line_colors.len() == self.lines.len()
            && self.tris.iter().flatten().all(|&i| i < n)
            && self.lines.iter().flatten().all(|&i| i < n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn triangle() -> Scene2D {
        Scene2D {
            width: 100,
            height: 80,
            points: alloc::vec![[0, 0], [800, 0], [0, 640]],
            tris: alloc::vec![[0, 1, 2]],
            tri_colors: alloc::vec![rgba(200, 200, 200, 255)],
            lines: alloc::vec![[0, 1]],
            line_colors: alloc::vec![rgba(0, 0, 0, 255)],
        }
    }

    #[test]
    fn round_trips_through_postcard() {
        let s = triangle();
        let bytes = postcard::to_allocvec(&s).unwrap();
        assert_eq!(postcard::from_bytes::<Scene2D>(&bytes).unwrap(), s);
    }

    #[test]
    fn points_read_back_in_pixels() {
        let s = triangle();
        assert_eq!(s.px(1), [100.0, 0.0]);
        assert_eq!(s.px(2), [0.0, 80.0]);
    }

    #[test]
    fn colours_pack_and_unpack() {
        assert_eq!(unpack(rgba(1, 2, 3, 4)), [1, 2, 3, 4]);
    }

    #[test]
    fn bad_indices_are_caught() {
        assert!(triangle().is_consistent());
        let mut s = triangle();
        s.tris[0][2] = 3;
        assert!(!s.is_consistent());
        let mut s = triangle();
        s.lines[0][1] = 9;
        assert!(!s.is_consistent());
        let mut s = triangle();
        s.tri_colors.clear();
        assert!(!s.is_consistent());
        let mut s = triangle();
        s.line_colors.clear();
        assert!(!s.is_consistent());
    }
}
