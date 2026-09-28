//! What the pointer is over: ray casting, SketchUp-style inference snapping, and the
//! push/pull drag.

use ccosel_abi::view3d::{Anchor, NO_FACE, Snap};

use crate::camera::{Camera, Ray};
use crate::math::{V3, add_scaled, cross, dist2_2d, dot, normalize_or, sub};
use crate::mesh::Mesh;

/// How close, in pixels, the pointer must be to a corner, a midpoint or an axis to snap to it.
pub const SNAP_PX: f32 = 10.0;

/// The nearest face under a ray.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub face: u32,
    pub point: V3,
    /// The face's outward normal, or the triangle's where the face is curved.
    pub normal: V3,
    /// Distance along the ray.
    pub t: f32,
}

/// Möller–Trumbore, two-sided: a loose face is hit from behind as well as in front.
fn ray_triangle(ray: &Ray, [a, b, c]: [V3; 3]) -> Option<f32> {
    let e1 = sub(b, a);
    let e2 = sub(c, a);
    let p = cross(ray.dir, e2);
    let det = dot(e1, p);
    if det.abs() < 1e-12 {
        return None;
    }
    let inv = 1.0 / det;
    let s = sub(ray.origin, a);
    let u = dot(s, p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = cross(s, e1);
    let v = dot(ray.dir, q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = dot(e2, q) * inv;
    (t > 0.0).then_some(t)
}

pub fn raycast(mesh: &Mesh, ray: &Ray) -> Option<Hit> {
    let mut best: Option<(usize, f32)> = None;
    for i in 0..mesh.triangle_count() {
        let Some(t) = ray_triangle(ray, mesh.tri(i)) else {
            continue;
        };
        let better = match best {
            None => true,
            Some((j, bt)) => {
                // A loose face drawn onto a solid sits exactly on it. It is the one the user
                // sees (it is drawn on top), so it is the one the pointer is over.
                let tie = (t - bt).abs() <= 1e-4 * bt.max(1.0);
                if tie {
                    mesh.is_loose(mesh.data.tri_face[i]) && !mesh.is_loose(mesh.data.tri_face[j])
                } else {
                    t < bt
                }
            }
        };
        if better {
            best = Some((i, t));
        }
    }
    let (i, t) = best?;
    let face = mesh.data.tri_face[i];
    Some(Hit {
        face,
        point: ray.at(t),
        normal: mesh.face_normal(face).unwrap_or_else(|| mesh.tri_normal(i)),
        t,
    })
}

/// Where a click would land, and what it snapped to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Resolved {
    pub point: V3,
    /// The plane a shape started here would lie on, facing the viewer: the drawing plane
    /// when there is an anchor, else the face under the pointer, else the ground.
    pub normal: V3,
    /// The face under the pointer, or [`NO_FACE`].
    pub face: u32,
    pub snap: Snap,
}

/// The nearest visible corner or edge midpoint within [`SNAP_PX`] of `pointer`. Corners win a
/// near tie, as in SketchUp: they are what people aim for.
fn point_snap(
    mesh: &Mesh,
    cam: &Camera,
    w: f32,
    h: f32,
    pointer: [f32; 2],
    hit: Option<&Hit>,
) -> Option<(V3, Snap)> {
    let hit_depth = hit.map(|hit| cam.to_view(hit.point)[2]);
    let mut best: Option<(f32, V3, Snap)> = None;
    let candidates = mesh
        .data
        .vertices
        .iter()
        .map(|v| (*v, Snap::Endpoint, 3.0))
        .chain(
            mesh.data
                .midpoints
                .iter()
                .map(|v| (*v, Snap::Midpoint, 0.0)),
        );
    for (p, snap, bonus) in candidates {
        let Some(s) = cam.project(p, w, h) else {
            continue;
        };
        // A corner hidden behind the face under the pointer is not one the user can see.
        if let Some(d) = hit_depth
            && s[2] > d * 1.01 + cam.near()
        {
            continue;
        }
        let d = dist2_2d([s[0], s[1]], pointer).sqrt();
        if d > SNAP_PX {
            continue;
        }
        let score = d - bonus;
        if best.is_none_or(|(b, _, _)| score < b) {
            best = Some((score, p, snap));
        }
    }
    best.map(|(_, p, s)| (p, s))
}

fn toward_viewer(n: V3, p: V3, eye: V3) -> V3 {
    if dot(n, sub(eye, p)) < 0.0 {
        crate::math::scale(n, -1.0)
    } else {
        n
    }
}

/// The point on the line `origin + s·dir` closest to `ray`, as `s`. `None` when the two are
/// parallel, where every point is equally close.
fn closest_on_line(origin: V3, dir: V3, ray: &Ray) -> Option<f32> {
    let w0 = sub(origin, ray.origin);
    let b = dot(dir, ray.dir);
    let denom = dot(dir, dir) * dot(ray.dir, ray.dir) - b * b;
    if denom.abs() < 1e-9 {
        return None;
    }
    Some((b * dot(ray.dir, w0) - dot(ray.dir, ray.dir) * dot(dir, w0)) / denom)
}

fn ray_plane(ray: &Ray, point: V3, normal: V3) -> Option<V3> {
    let denom = dot(ray.dir, normal);
    if denom.abs() < 1e-9 {
        return None;
    }
    let t = dot(sub(point, ray.origin), normal) / denom;
    (t > 0.0).then(|| ray.at(t))
}

/// Resolve the pointer at `pointer` (pixels in a `w` × `h` viewport) to a point, applying
/// SketchUp's inferences in its order of strength: a corner or midpoint, then — once a shape
/// has an `anchor` — a line along the red, green or blue axis through it, then the face
/// under the pointer, then the ground.
///
/// With an anchor the result is always on the anchor's plane, since every point of a shape
/// must be. `None` when the pointer's ray never meets anything it could land on (pointing at
/// the sky edge-on to the drawing plane).
pub fn resolve(
    mesh: Option<&Mesh>,
    cam: &Camera,
    w: f32,
    h: f32,
    pointer: [f32; 2],
    anchor: Option<Anchor>,
) -> Option<Resolved> {
    let ray = cam.ray(pointer[0], pointer[1], w, h);
    let eye = cam.eye();
    let hit = mesh.and_then(|m| raycast(m, &ray));
    let face = hit.map_or(NO_FACE, |h| h.face);
    let snapped = mesh.and_then(|m| point_snap(m, cam, w, h, pointer, hit.as_ref()));

    if let Some(a) = anchor {
        let n = normalize_or(a.normal, [0.0, 0.0, 1.0]);
        let onto_plane = |p: V3| add_scaled(p, n, -dot(sub(p, a.point), n));
        if let Some((p, snap)) = snapped {
            return Some(Resolved {
                point: onto_plane(p),
                normal: n,
                face,
                snap,
            });
        }
        let axes = [
            ([1.0, 0.0, 0.0], Snap::AxisX),
            ([0.0, 1.0, 0.0], Snap::AxisY),
            ([0.0, 0.0, 1.0], Snap::AxisZ),
        ];
        let mut best: Option<(f32, V3, Snap)> = None;
        for (axis, snap) in axes {
            // Only axes lying in the drawing plane; the others would leave it.
            if dot(axis, n).abs() > 0.01 {
                continue;
            }
            let Some(s) = closest_on_line(a.point, axis, &ray) else {
                continue;
            };
            let p = add_scaled(a.point, axis, s);
            let Some(sp) = cam.project(p, w, h) else {
                continue;
            };
            let d = dist2_2d([sp[0], sp[1]], pointer).sqrt();
            if d <= SNAP_PX && best.is_none_or(|(b, _, _)| d < b) {
                best = Some((d, p, snap));
            }
        }
        if let Some((_, p, snap)) = best {
            return Some(Resolved {
                point: onto_plane(p),
                normal: n,
                face,
                snap,
            });
        }
        let p = ray_plane(&ray, a.point, n)?;
        return Some(Resolved {
            point: p,
            normal: n,
            face,
            snap: Snap::Free,
        });
    }

    if let Some((p, snap)) = snapped {
        let normal = hit.map_or([0.0, 0.0, 1.0], |h| toward_viewer(h.normal, h.point, eye));
        return Some(Resolved {
            point: p,
            normal,
            face,
            snap,
        });
    }
    if let Some(hit) = hit {
        return Some(Resolved {
            point: hit.point,
            normal: toward_viewer(hit.normal, hit.point, eye),
            face: hit.face,
            snap: Snap::OnFace,
        });
    }
    let p = ray_plane(&ray, [0.0; 3], [0.0, 0.0, 1.0])?;
    Some(Resolved {
        point: p,
        normal: toward_viewer([0.0, 0.0, 1.0], p, eye),
        face: NO_FACE,
        snap: Snap::Free,
    })
}

/// A push/pull in progress: `face` is being moved `distance` along its outward `normal`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PushPull {
    pub face: u32,
    pub normal: V3,
    /// Where on the face the drag started.
    pub origin: V3,
    pub distance: f32,
    /// Whether `distance` was snapped to a corner's height rather than following the pointer.
    pub snapped: bool,
}

impl PushPull {
    /// Start dragging the face `hit` landed on, if it is flat.
    pub fn start(mesh: &Mesh, hit: &Hit) -> Option<Self> {
        Some(Self {
            face: hit.face,
            normal: mesh.face_normal(hit.face)?,
            origin: hit.point,
            distance: 0.0,
            snapped: false,
        })
    }

    /// Follow the pointer: the distance is where the pointer's ray passes closest to the line
    /// the face moves along. If a corner of the model is under the pointer, snap to its height
    /// instead — how SketchUp lets you pull a face level with another.
    pub fn update(&mut self, mesh: &Mesh, cam: &Camera, w: f32, h: f32, pointer: [f32; 2]) {
        let ray = cam.ray(pointer[0], pointer[1], w, h);
        if let Some((p, Snap::Endpoint)) = point_snap(mesh, cam, w, h, pointer, None) {
            let d = dot(sub(p, self.origin), self.normal);
            if d.abs() > 1e-6 {
                self.distance = d;
                self.snapped = true;
                return;
            }
        }
        self.snapped = false;
        if let Some(s) = closest_on_line(self.origin, self.normal, &ray)
            && s.is_finite()
        {
            self.distance = s;
        }
    }
}

#[cfg(test)]
mod tests;
