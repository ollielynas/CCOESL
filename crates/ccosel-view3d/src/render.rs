//! Mesh + camera → [`Scene2D`]: flat-shaded triangles in drawing order, and the parts of the
//! model's edges that are not hidden.
//!
//! Faces use the painter's algorithm: clip to the near plane, drop the back faces of solids,
//! sort far to near. That is exact for a convex solid and good for the boxy, axis-aligned
//! models push/pull makes. Two cases are handled on purpose:
//!
//! - A **loose face drawn onto a solid** lies exactly in the solid's face. It is always ordered
//!   after the coplanar solid triangles it overlaps, so it never disappears into them.
//! - **Edges** are not sorted at all. A small software depth buffer is filled with the faces,
//!   and each edge is sampled against it, so a hidden edge is cut away exactly where it passes
//!   behind something — the thing a painter's sort cannot get right for lines.
//!
//! The same function runs in the shell and on the server, so the two render modes match.

use std::collections::HashMap;

use ccosel_abi::view3d::NO_FACE;
use ccosel_proto::scene2d::{SUBPIXELS, Scene2D, rgba};

use crate::camera::Camera;
use crate::math::{V3, dot, lerp, normalize_or, sub};
use crate::mesh::Mesh;

/// What to draw besides the model itself.
#[derive(Clone, Debug)]
pub struct Style {
    /// The face under the pointer, tinted, or [`NO_FACE`].
    pub highlight: u32,
    /// The selected face, tinted more strongly, or [`NO_FACE`].
    pub selected: u32,
    /// Translucent triangles sorted in with the model: a push/pull's extrusion.
    pub extra: Vec<[V3; 3]>,
    /// Draw the red, green and blue axes through the origin.
    pub axes: bool,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            highlight: NO_FACE,
            selected: NO_FACE,
            extra: Vec::new(),
            axes: true,
        }
    }
}

/// SketchUp's palette: warm white fronts, blue-grey backs, black edges.
const FRONT: [f32; 3] = [0.94, 0.93, 0.90];
const BACK: [f32; 3] = [0.62, 0.67, 0.80];
const HIGHLIGHT: [f32; 3] = [0.70, 0.80, 1.0];
const SELECTED: [f32; 3] = [0.45, 0.62, 1.0];
const EXTRA: u32 = rgba(90, 140, 255, 110);
const EDGE: u32 = rgba(20, 20, 20, 255);
const AXIS_X: u32 = rgba(220, 40, 40, 255);
const AXIS_Y: u32 = rgba(30, 160, 60, 255);
const AXIS_Z: u32 = rgba(40, 80, 220, 255);

/// The longest side of the depth buffer edges are tested against, in cells. Enough to place
/// an edge's hidden part to within a couple of pixels, small enough to redo every frame of an
/// orbit.
const DEPTH_CELLS: f32 = 480.0;

/// A triangle ready to draw.
struct Tri {
    /// Screen positions and view depths.
    pts: [[f32; 3]; 3],
    /// Mesh vertex indices, when it was not clipped, so shared corners share a point.
    verts: Option<[u32; 3]>,
    color: u32,
    /// Far-to-near sort key: the centroid's depth.
    key: f32,
    /// A loose face that sits on some solid face; drawn after that face.
    loose_face: Option<u32>,
    /// Whether it is solid geometry for the edge depth test (the preview is not).
    occludes: bool,
    plane: Option<(V3, f32)>,
}

/// Clip a polygon in view space to `z >= near`. Sutherland–Hodgman against one plane.
fn clip_near(poly: &[V3], near: f32) -> Vec<V3> {
    let mut out = Vec::with_capacity(poly.len() + 1);
    for i in 0..poly.len() {
        let a = poly[i];
        let b = poly[(i + 1) % poly.len()];
        let (ina, inb) = (a[2] >= near, b[2] >= near);
        if ina {
            out.push(a);
        }
        if ina != inb {
            let t = (near - a[2]) / (b[2] - a[2]);
            out.push(lerp(a, b, t));
        }
    }
    out
}

fn shade(base: [f32; 3], lambert: f32) -> u32 {
    let k = 0.62 + 0.38 * lambert.clamp(0.0, 1.0);
    let c = |v: f32| (v * k * 255.0).round().clamp(0.0, 255.0) as u8;
    rgba(c(base[0]), c(base[1]), c(base[2]), 255)
}

/// The light comes from over the viewer's left shoulder, fixed to the camera, so every face
/// keeps a readable shade from every angle.
fn lambert(normal_view: V3) -> f32 {
    let light = normalize_or([-0.45, 0.6, -0.65], [0.0, 0.0, -1.0]);
    dot(normal_view, light)
}

fn to_view_dir(cam: &Camera, n: V3) -> V3 {
    let (r, u, f) = cam.basis();
    [dot(n, r), dot(n, u), dot(n, f)]
}

/// Draw `mesh` (if any) from `cam` into a `w` × `h` pixel frame.
pub fn render(mesh: Option<&Mesh>, cam: &Camera, w: u16, h: u16, style: &Style) -> Scene2D {
    let (wf, hf) = (f32::from(w.max(1)), f32::from(h.max(1)));
    let near = cam.near();
    let eye = cam.eye();
    let mut tris: Vec<Tri> = Vec::new();

    let mut push_poly = |view: [V3; 3],
                         verts: Option<[u32; 3]>,
                         color: u32,
                         loose_face: Option<u32>,
                         occludes: bool,
                         plane: Option<(V3, f32)>| {
        let poly = clip_near(&view, near);
        if poly.len() < 3 {
            return;
        }
        let clipped = poly.len() != 3 || poly.iter().zip(view.iter()).any(|(a, b)| a != b);
        let screen: Vec<[f32; 3]> = poly
            .iter()
            .map(|v| {
                let s = cam.view_to_screen(*v, wf, hf);
                [s[0], s[1], v[2]]
            })
            .collect();
        for k in 1..screen.len() - 1 {
            let pts = [screen[0], screen[k], screen[k + 1]];
            tris.push(Tri {
                pts,
                verts: if clipped { None } else { verts },
                color,
                key: (pts[0][2] + pts[1][2] + pts[2][2]) / 3.0,
                loose_face,
                occludes,
                plane,
            });
        }
    };

    if let Some(mesh) = mesh {
        for i in 0..mesh.triangle_count() {
            let face = mesh.data.tri_face[i];
            let loose = mesh.is_loose(face);
            let world = mesh.tri(i);
            let n = mesh.tri_normal(i);
            let front = dot(n, sub(eye, world[0])) > 0.0;
            // A solid's back faces are always behind its front ones: not drawing them is both
            // faster and one less thing for the sort to get wrong.
            if !front && !loose {
                continue;
            }
            let base = if face == style.selected {
                SELECTED
            } else if face == style.highlight {
                HIGHLIGHT
            } else if front {
                FRONT
            } else {
                BACK
            };
            // The side the viewer sees, so a loose face's back is lit like its front.
            let nv = to_view_dir(
                cam,
                if front {
                    n
                } else {
                    crate::math::scale(n, -1.0)
                },
            );
            let color = shade(base, lambert(nv));
            let view = world.map(|p| cam.to_view(p));
            push_poly(
                view,
                Some(mesh.data.triangles[i]),
                color,
                loose.then_some(face),
                true,
                Some((n, dot(n, world[0]))),
            );
        }
    }
    for t in &style.extra {
        push_poly(t.map(|p| cam.to_view(p)), None, EXTRA, None, false, None);
    }

    order_loose_after_their_solid(&mut tris, mesh);
    // Far first. `total_cmp` so a stray NaN cannot make the sort panic or misbehave.
    tris.sort_by(|a, b| {
        b.key
            .total_cmp(&a.key)
            .then(a.loose_face.is_some().cmp(&b.loose_face.is_some()))
    });

    let mut scene = Scene2D {
        width: w,
        height: h,
        ..Scene2D::default()
    };
    let mut shared: HashMap<u32, u32> = HashMap::new();
    let point = |scene: &mut Scene2D, p: [f32; 2]| -> u32 {
        let q = |v: f32| {
            (v * SUBPIXELS)
                .round()
                .clamp(-(1 << 28) as f32, (1 << 28) as f32) as i32
        };
        scene.points.push([q(p[0]), q(p[1])]);
        (scene.points.len() - 1) as u32
    };
    for t in &tris {
        let mut idx = [0u32; 3];
        for k in 0..3 {
            let p = [t.pts[k][0], t.pts[k][1]];
            idx[k] = match t.verts {
                Some(v) => *shared.entry(v[k]).or_insert_with(|| point(&mut scene, p)),
                None => point(&mut scene, p),
            };
        }
        scene.tris.push(idx);
        scene.tri_colors.push(t.color);
    }

    let depth = DepthBuffer::build(&tris, wf, hf);
    let mut segments: Vec<([V3; 2], u32)> = Vec::new();
    if let Some(mesh) = mesh {
        segments.extend(mesh.data.edges.iter().map(|e| (*e, EDGE)));
    }
    if style.axes {
        let reach = mesh
            .map_or(0.0, |m| m.radius * 20.0)
            .max(cam.state.distance * 20.0);
        segments.push(([[0.0; 3], [reach, 0.0, 0.0]], AXIS_X));
        segments.push(([[0.0; 3], [0.0, reach, 0.0]], AXIS_Y));
        segments.push(([[0.0; 3], [0.0, 0.0, reach]], AXIS_Z));
    }
    for (seg, color) in segments {
        for [a, b] in depth.visible_parts(cam, seg, wf, hf) {
            let (i, j) = (point(&mut scene, a), point(&mut scene, b));
            scene.lines.push([i, j]);
            scene.line_colors.push(color);
        }
    }
    scene
}

/// A screen-space box: min x, max x, min y, max y.
type BBox = (f32, f32, f32, f32);

/// Give each loose triangle a sort key no nearer than the nearest coplanar solid triangle it
/// overlaps on screen, so the solid one is drawn first and the loose one lands on top.
fn order_loose_after_their_solid(tris: &mut [Tri], mesh: Option<&Mesh>) {
    let scale = mesh.map_or(1.0, |m| m.radius.max(1e-3));
    let bbox = |t: &Tri| {
        let xs = t.pts.map(|p| p[0]);
        let ys = t.pts.map(|p| p[1]);
        (
            xs.iter().copied().fold(f32::INFINITY, f32::min),
            xs.iter().copied().fold(f32::NEG_INFINITY, f32::max),
            ys.iter().copied().fold(f32::INFINITY, f32::min),
            ys.iter().copied().fold(f32::NEG_INFINITY, f32::max),
        )
    };
    let solids: Vec<((V3, f32), BBox, f32)> = tris
        .iter()
        .filter(|t| t.loose_face.is_none() && t.occludes)
        .filter_map(|t| Some((t.plane?, bbox(t), t.key)))
        .collect();
    for t in tris.iter_mut().filter(|t| t.loose_face.is_some()) {
        let Some((n, d)) = t.plane else { continue };
        let b = bbox(t);
        for ((sn, sd), sb, key) in &solids {
            let coplanar = dot(n, *sn).abs() > 0.999 && (d - sd * dot(n, *sn)).abs() < 1e-4 * scale;
            let overlap = b.0 <= sb.1 && sb.0 <= b.1 && b.2 <= sb.3 && sb.2 <= b.3;
            if coplanar && overlap && *key < t.key {
                t.key = *key;
            }
        }
    }
}

/// Inverse depth (`1/z`, so nearer is larger and empty is 0) of the frontmost face at each
/// cell of a coarse grid over the viewport. `1/z` because it interpolates linearly in screen
/// space, which depth itself does not under perspective.
struct DepthBuffer {
    cells: Vec<f32>,
    cw: usize,
    ch: usize,
    /// Cells per pixel.
    scale: f32,
}

impl DepthBuffer {
    fn build(tris: &[Tri], w: f32, h: f32) -> Self {
        let scale = (DEPTH_CELLS / w.max(h)).min(1.0);
        let cw = ((w * scale).ceil() as usize).max(1);
        let ch = ((h * scale).ceil() as usize).max(1);
        let mut cells = vec![0.0f32; cw * ch];
        for t in tris.iter().filter(|t| t.occludes) {
            let p = t.pts.map(|p| [p[0] * scale, p[1] * scale, 1.0 / p[2]]);
            let area = edge(p[0], p[1], p[2]);
            if area.abs() < 1e-9 {
                continue;
            }
            let min_x = p
                .iter()
                .map(|p| p[0])
                .fold(f32::INFINITY, f32::min)
                .floor()
                .max(0.0);
            let max_x = p
                .iter()
                .map(|p| p[0])
                .fold(f32::NEG_INFINITY, f32::max)
                .ceil();
            let min_y = p
                .iter()
                .map(|p| p[1])
                .fold(f32::INFINITY, f32::min)
                .floor()
                .max(0.0);
            let max_y = p
                .iter()
                .map(|p| p[1])
                .fold(f32::NEG_INFINITY, f32::max)
                .ceil();
            let max_x = max_x.min(cw as f32 - 1.0);
            let max_y = max_y.min(ch as f32 - 1.0);
            if !(min_x <= max_x && min_y <= max_y) {
                continue;
            }
            for y in min_y as usize..=max_y as usize {
                for x in min_x as usize..=max_x as usize {
                    let c = [x as f32 + 0.5, y as f32 + 0.5, 0.0];
                    let w0 = edge(p[1], p[2], c) / area;
                    let w1 = edge(p[2], p[0], c) / area;
                    let w2 = 1.0 - w0 - w1;
                    if w0 < -1e-4 || w1 < -1e-4 || w2 < -1e-4 {
                        continue;
                    }
                    let inv = w0 * p[0][2] + w1 * p[1][2] + w2 * p[2][2];
                    let cell = &mut cells[y * cw + x];
                    if inv > *cell {
                        *cell = inv;
                    }
                }
            }
        }
        Self {
            cells,
            cw,
            ch,
            scale,
        }
    }

    /// The farthest face around a pixel: the edge a face meets is at that face's depth, and
    /// looking one cell either side is what keeps rounding from hiding it.
    fn farthest_near(&self, x: f32, y: f32) -> f32 {
        let (cx, cy) = ((x * self.scale) as isize, (y * self.scale) as isize);
        let mut far = f32::INFINITY;
        for dy in -1..=1 {
            for dx in -1..=1 {
                let (x, y) = (cx + dx, cy + dy);
                if x < 0 || y < 0 || x >= self.cw as isize || y >= self.ch as isize {
                    return 0.0;
                }
                far = far.min(self.cells[y as usize * self.cw + x as usize]);
            }
        }
        far
    }

    /// The pieces of the world segment `seg` that are in front of the near plane and not
    /// behind any face, in pixels.
    fn visible_parts(&self, cam: &Camera, seg: [V3; 2], w: f32, h: f32) -> Vec<[[f32; 2]; 2]> {
        let near = cam.near();
        let (mut a, mut b) = (cam.to_view(seg[0]), cam.to_view(seg[1]));
        if a[2] < near && b[2] < near {
            return Vec::new();
        }
        if a[2] < near {
            a = lerp(a, b, (near - a[2]) / (b[2] - a[2]));
        } else if b[2] < near {
            b = lerp(b, a, (near - b[2]) / (a[2] - b[2]));
        }
        let (sa, sb) = (cam.view_to_screen(a, w, h), cam.view_to_screen(b, w, h));
        // Off-screen parts need no test; bound the work for an axis reaching the horizon.
        let (sa, sb, ia, ib) = match clip_to_rect(sa, sb, 1.0 / a[2], 1.0 / b[2], w, h) {
            Some(c) => c,
            None => return Vec::new(),
        };
        let len = ((sb[0] - sa[0]).powi(2) + (sb[1] - sa[1]).powi(2)).sqrt();
        let steps = ((len / 2.0).ceil() as usize).clamp(1, 4096);
        let at = |t: f32| [sa[0] + (sb[0] - sa[0]) * t, sa[1] + (sb[1] - sa[1]) * t];
        let mut out = Vec::new();
        let mut run: Option<f32> = None;
        for k in 0..steps {
            let t = (k as f32 + 0.5) / steps as f32;
            let p = at(t);
            let inv = ia + (ib - ia) * t;
            let visible = inv >= self.farthest_near(p[0], p[1]) * 0.995;
            match (visible, run) {
                (true, None) => run = Some(k as f32 / steps as f32),
                (false, Some(start)) => {
                    out.push([at(start), at(k as f32 / steps as f32)]);
                    run = None;
                }
                _ => {}
            }
        }
        if let Some(start) = run {
            out.push([at(start), sb]);
        }
        out
    }
}

/// Twice the signed area of the triangle `a b c`, in screen space.
fn edge(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> f32 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

type Clipped = ([f32; 2], [f32; 2], f32, f32);

/// Liang–Barsky: the part of the screen segment `a b` inside the viewport, carrying the
/// inverse depths `ia`, `ib` along (they are linear in screen space).
fn clip_to_rect(a: [f32; 2], b: [f32; 2], ia: f32, ib: f32, w: f32, h: f32) -> Option<Clipped> {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    for (p, q) in [(-dx, a[0]), (dx, w - a[0]), (-dy, a[1]), (dy, h - a[1])] {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
            continue;
        }
        let r = q / p;
        if p < 0.0 {
            t0 = t0.max(r);
        } else {
            t1 = t1.min(r);
        }
        if t0 > t1 {
            return None;
        }
    }
    let at = |t: f32| [a[0] + dx * t, a[1] + dy * t];
    Some((at(t0), at(t1), ia + (ib - ia) * t0, ia + (ib - ia) * t1))
}

#[cfg(test)]
mod tests;
