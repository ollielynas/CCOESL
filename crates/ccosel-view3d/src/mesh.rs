//! A [`MeshData`] checked once on arrival, plus what picking and drawing derive from it.

use std::collections::HashMap;

use ccosel_proto::cad::MeshData;

use crate::math::{V3, add_scaled, cross, finite, len, normalize_or, scale, sub};

#[derive(Clone, Debug)]
pub struct Mesh {
    pub data: MeshData,
    /// Centre and radius of a sphere around every position, for framing the camera.
    pub center: V3,
    pub radius: f32,
    tri_normals: Vec<V3>,
}

impl Mesh {
    /// Accept a mesh off the wire, or say why not. After this, every index is in range and
    /// every coordinate is finite, so nothing downstream checks again.
    pub fn new(data: MeshData) -> Result<Self, &'static str> {
        if !data.is_consistent() {
            return Err("mesh indices out of range");
        }
        let all_finite = data.positions.iter().all(|p| finite(*p))
            && data.face_normals.iter().all(|n| finite(*n))
            && data.edges.iter().all(|e| finite(e[0]) && finite(e[1]))
            && data.vertices.iter().all(|v| finite(*v))
            && data.midpoints.iter().all(|v| finite(*v));
        if !all_finite {
            return Err("mesh has a non-finite coordinate");
        }

        let (center, radius) = bounds(&data.positions);
        let tri_normals = data
            .triangles
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| data.positions[i as usize]);
                normalize_or(cross(sub(b, a), sub(c, a)), [0.0, 0.0, 1.0])
            })
            .collect();
        Ok(Self {
            data,
            center,
            radius,
            tri_normals,
        })
    }

    pub fn triangle_count(&self) -> usize {
        self.data.triangles.len()
    }

    pub fn tri(&self, i: usize) -> [V3; 3] {
        self.data.triangles[i].map(|v| self.data.positions[v as usize])
    }

    /// The geometric normal of triangle `i`, from its winding.
    pub fn tri_normal(&self, i: usize) -> V3 {
        self.tri_normals[i]
    }

    pub fn face_count(&self) -> u32 {
        self.data.face_normals.len() as u32
    }

    /// The outward normal of a flat face, or `None` for a curved one (or no such face), which
    /// cannot be pushed or pulled.
    pub fn face_normal(&self, face: u32) -> Option<V3> {
        let n = *self.data.face_normals.get(face as usize)?;
        (len(n) > 0.5).then(|| normalize_or(n, n))
    }

    pub fn is_loose(&self, face: u32) -> bool {
        self.data.loose.get(face as usize).copied().unwrap_or(false)
    }

    /// Indices of the triangles making up `face`.
    pub fn face_tris(&self, face: u32) -> impl Iterator<Item = usize> + '_ {
        self.data
            .tri_face
            .iter()
            .enumerate()
            .filter(move |(_, f)| **f == face)
            .map(|(i, _)| i)
    }

    /// The outline of `face`: every triangle edge that no other triangle of the face shares,
    /// oriented as its triangle winds it.
    pub fn face_outline(&self, face: u32) -> Vec<[V3; 2]> {
        let mut count: HashMap<(u32, u32), (u32, u32, u32)> = HashMap::new();
        for t in self.face_tris(face) {
            let tri = self.data.triangles[t];
            for k in 0..3 {
                let (a, b) = (tri[k], tri[(k + 1) % 3]);
                let key = (a.min(b), a.max(b));
                count.entry(key).or_insert((a, b, 0)).2 += 1;
            }
        }
        let mut out: Vec<[V3; 2]> = count
            .into_values()
            .filter(|&(_, _, n)| n == 1)
            .map(|(a, b, _)| {
                [
                    self.data.positions[a as usize],
                    self.data.positions[b as usize],
                ]
            })
            .collect();
        // Map iteration order is arbitrary; a stable order keeps frames reproducible.
        out.sort_by(|x, y| x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal));
        out
    }

    /// The solid a push/pull of `face` by `distance` would add or remove, as triangles: the
    /// face moved along its normal, plus a wall along each outline edge. Drawn translucent
    /// while the user drags, and while the server rebuilds the real thing. Empty for a curved
    /// face or a zero distance.
    pub fn extrusion(&self, face: u32, distance: f32) -> Vec<[V3; 3]> {
        let Some(n) = self.face_normal(face) else {
            return Vec::new();
        };
        if distance == 0.0 || !distance.is_finite() {
            return Vec::new();
        }
        let off = scale(n, distance);
        let moved = |p: V3| add_scaled(p, off, 1.0);
        let mut out: Vec<[V3; 3]> = self
            .face_tris(face)
            .map(|t| self.tri(t).map(moved))
            .collect();
        for [a, b] in self.face_outline(face) {
            let (a2, b2) = (moved(a), moved(b));
            out.push([a, b, b2]);
            out.push([a, b2, a2]);
        }
        out
    }
}

fn bounds(points: &[V3]) -> (V3, f32) {
    let Some(first) = points.first() else {
        return ([0.0; 3], 0.0);
    };
    let mut lo = *first;
    let mut hi = *first;
    for p in points {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    let center = scale(crate::math::add(lo, hi), 0.5);
    (center, len(sub(hi, lo)) * 0.5)
}

/// Test fixtures, shared by this crate's tests: a box and a loose square.
#[cfg(test)]
pub(crate) mod fixtures {
    use ccosel_proto::cad::MeshData;

    /// An axis-aligned box from the origin to `(sx, sy, sz)`, one face per side, numbered
    /// bottom, top, front (-Y), back (+Y), left (-X), right (+X).
    pub fn cuboid(sx: f32, sy: f32, sz: f32) -> MeshData {
        let p = vec![
            [0.0, 0.0, 0.0],
            [sx, 0.0, 0.0],
            [sx, sy, 0.0],
            [0.0, sy, 0.0],
            [0.0, 0.0, sz],
            [sx, 0.0, sz],
            [sx, sy, sz],
            [0.0, sy, sz],
        ];
        // Each wound counter-clockwise seen from outside.
        let triangles = vec![
            [0, 2, 1],
            [0, 3, 2],
            [4, 5, 6],
            [4, 6, 7],
            [0, 1, 5],
            [0, 5, 4],
            [2, 3, 7],
            [2, 7, 6],
            [0, 4, 7],
            [0, 7, 3],
            [1, 2, 6],
            [1, 6, 5],
        ];
        let e = |a: usize, b: usize| [p[a], p[b]];
        let edges = vec![
            e(0, 1),
            e(1, 2),
            e(2, 3),
            e(3, 0),
            e(4, 5),
            e(5, 6),
            e(6, 7),
            e(7, 4),
            e(0, 4),
            e(1, 5),
            e(2, 6),
            e(3, 7),
        ];
        let midpoints = edges
            .iter()
            .map(|[a, b]| {
                [
                    (a[0] + b[0]) / 2.0,
                    (a[1] + b[1]) / 2.0,
                    (a[2] + b[2]) / 2.0,
                ]
            })
            .collect();
        MeshData {
            vertices: p.clone(),
            positions: p,
            triangles,
            tri_face: vec![0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5],
            face_normals: vec![
                [0.0, 0.0, -1.0],
                [0.0, 0.0, 1.0],
                [0.0, -1.0, 0.0],
                [0.0, 1.0, 0.0],
                [-1.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
            ],
            loose: vec![false; 6],
            edges,
            midpoints,
        }
    }

    /// A loose square on the ground, `side` across, facing up.
    pub fn square(side: f32) -> MeshData {
        let p = vec![
            [0.0, 0.0, 0.0],
            [side, 0.0, 0.0],
            [side, side, 0.0],
            [0.0, side, 0.0],
        ];
        MeshData {
            vertices: p.clone(),
            positions: p,
            triangles: vec![[0, 1, 2], [0, 2, 3]],
            tri_face: vec![0, 0],
            face_normals: vec![[0.0, 0.0, 1.0]],
            loose: vec![true],
            ..MeshData::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::{cuboid, square};
    use super::*;
    use crate::math::dot;

    #[test]
    fn bounds_enclose_the_mesh() {
        let m = Mesh::new(cuboid(2.0, 4.0, 4.0)).unwrap();
        assert_eq!(m.center, [1.0, 2.0, 2.0]);
        assert_eq!(m.radius, 3.0);
    }

    #[test]
    fn corrupt_meshes_are_refused() {
        let mut d = square(1.0);
        d.triangles[0][0] = 99;
        assert!(Mesh::new(d).is_err());
        let mut d = square(1.0);
        d.positions[1][2] = f32::NAN;
        assert!(Mesh::new(d).is_err());
    }

    #[test]
    fn triangle_normals_point_out_of_the_box() {
        let m = Mesh::new(cuboid(1.0, 1.0, 1.0)).unwrap();
        for t in 0..m.triangle_count() {
            let face = m.data.tri_face[t];
            let n = m.face_normal(face).unwrap();
            assert!(dot(m.tri_normal(t), n) > 0.99, "triangle {t}");
        }
    }

    #[test]
    fn a_square_outline_is_its_four_sides() {
        let m = Mesh::new(square(2.0)).unwrap();
        let outline = m.face_outline(0);
        assert_eq!(outline.len(), 4, "the shared diagonal is not outline");
    }

    #[test]
    fn extruding_a_square_makes_a_closed_prism() {
        let m = Mesh::new(square(2.0)).unwrap();
        let tris = m.extrusion(0, 3.0);
        // Cap (2) + four walls of two.
        assert_eq!(tris.len(), 10);
        assert!(tris[0].iter().all(|p| p[2] == 3.0), "the cap moved up");
        assert!(m.extrusion(0, 0.0).is_empty());
        assert!(m.extrusion(7, 1.0).is_empty(), "no such face");
    }

    #[test]
    fn curved_faces_have_no_normal() {
        let mut d = square(1.0);
        d.face_normals[0] = [0.0; 3];
        let m = Mesh::new(d).unwrap();
        assert_eq!(m.face_normal(0), None);
        assert!(m.extrusion(0, 1.0).is_empty());
    }
}
