//! The orbit camera, and the mapping between world space and viewport pixels.
//!
//! Projection is done straight from the camera's basis rather than through a 4×4 matrix: a
//! point's depth, and whether it is in front of the camera at all, fall out directly, and
//! there is no matrix layout to get transposed. Pixels are measured from the viewport's
//! top-left corner, `y` down, like egui's.

use ccosel_proto::cad::CameraState;

use crate::math::{V3, add_scaled, cross, dot, lerp, normalize_or, sub};

/// Radians of orbit per pixel dragged.
const ORBIT_PER_PX: f32 = 0.008;

/// Just short of straight up or down, so "up" on screen is always defined.
const MAX_ELEVATION: f32 = 1.55;

/// A ray from the eye through a pixel. `dir` is unit length.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ray {
    pub origin: V3,
    pub dir: V3,
}

impl Ray {
    pub fn at(&self, t: f32) -> V3 {
        add_scaled(self.origin, self.dir, t)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub state: CameraState,
}

impl Default for Camera {
    fn default() -> Self {
        Self::new(CameraState::default())
    }
}

impl Camera {
    /// A camera from a state that may have come off the wire: non-finite or out-of-range
    /// values are replaced, so no later arithmetic sees a NaN.
    pub fn new(mut state: CameraState) -> Self {
        let d = CameraState::default();
        if !state.target.iter().all(|c| c.is_finite()) {
            state.target = d.target;
        }
        if !(state.distance.is_finite() && state.distance > 0.0) {
            state.distance = d.distance;
        }
        if !state.azimuth.is_finite() {
            state.azimuth = d.azimuth;
        }
        state.elevation = if state.elevation.is_finite() {
            state.elevation.clamp(-MAX_ELEVATION, MAX_ELEVATION)
        } else {
            d.elevation
        };
        if !(state.fov_y.is_finite() && state.fov_y > 0.05 && state.fov_y < 3.0) {
            state.fov_y = d.fov_y;
        }
        Self { state }
    }

    /// Unit vector from the target towards the eye.
    fn back(&self) -> V3 {
        let (se, ce) = self.state.elevation.sin_cos();
        let (sa, ca) = self.state.azimuth.sin_cos();
        [ce * ca, ce * sa, se]
    }

    pub fn eye(&self) -> V3 {
        add_scaled(self.state.target, self.back(), self.state.distance)
    }

    /// `(right, up, forward)`, unit and right-handed, with world Z as up.
    pub fn basis(&self) -> (V3, V3, V3) {
        let f = crate::math::scale(self.back(), -1.0);
        let r = normalize_or(cross(f, [0.0, 0.0, 1.0]), [1.0, 0.0, 0.0]);
        let u = cross(r, f);
        (r, u, f)
    }

    /// Anything nearer the eye than this is clipped. Scaled with the distance, so zooming in
    /// on a small part keeps working.
    pub fn near(&self) -> f32 {
        (self.state.distance * 0.002).max(1e-4)
    }

    fn tan_half(&self) -> f32 {
        (self.state.fov_y * 0.5).tan()
    }

    /// World point to view coordinates: `[right, up, depth]`, depth along the view direction.
    pub fn to_view(&self, p: V3) -> V3 {
        let (r, u, f) = self.basis();
        let d = sub(p, self.eye());
        [dot(d, r), dot(d, u), dot(d, f)]
    }

    /// View coordinates to pixels. The caller must have checked depth against [`near`].
    ///
    /// [`near`]: Camera::near
    pub fn view_to_screen(&self, v: V3, w: f32, h: f32) -> [f32; 2] {
        let k = (h * 0.5) / (v[2] * self.tan_half());
        [w * 0.5 + v[0] * k, h * 0.5 - v[1] * k]
    }

    /// Pixel position and depth of `p`, or `None` if it is behind the near plane.
    pub fn project(&self, p: V3, w: f32, h: f32) -> Option<[f32; 3]> {
        let v = self.to_view(p);
        // Written so a NaN depth also counts as "not in front".
        if v[2].is_nan() || v[2] < self.near() {
            return None;
        }
        let s = self.view_to_screen(v, w, h);
        Some([s[0], s[1], v[2]])
    }

    /// The ray from the eye through pixel `(x, y)` of a `w` × `h` viewport.
    pub fn ray(&self, x: f32, y: f32, w: f32, h: f32) -> Ray {
        let (r, u, f) = self.basis();
        let t = self.tan_half();
        let half = (h * 0.5).max(1e-6);
        let sx = (x - w * 0.5) / half * t;
        let sy = -(y - h * 0.5) / half * t;
        let dir = normalize_or(add_scaled(add_scaled(f, r, sx), u, sy), f);
        Ray {
            origin: self.eye(),
            dir,
        }
    }

    /// Swing round the target by a pointer drag of `(dx, dy)` pixels. Dragging right turns
    /// the model right; dragging down looks from higher up.
    pub fn orbit(&mut self, dx: f32, dy: f32) {
        self.state.azimuth -= dx * ORBIT_PER_PX;
        self.state.elevation =
            (self.state.elevation + dy * ORBIT_PER_PX).clamp(-MAX_ELEVATION, MAX_ELEVATION);
    }

    /// Slide the view so the model follows a drag of `(dx, dy)` pixels in an `h`-pixel-tall
    /// viewport: a point at the target's depth stays under the pointer.
    pub fn pan(&mut self, dx: f32, dy: f32, h: f32) {
        let (r, u, _) = self.basis();
        let world_per_px = 2.0 * self.state.distance * self.tan_half() / h.max(1.0);
        let t = add_scaled(self.state.target, r, -dx * world_per_px);
        self.state.target = add_scaled(t, u, dy * world_per_px);
    }

    /// Move `factor` of the way closer (below 1) or further (above 1), keeping `toward` — the
    /// point under the pointer — where it is on screen, the way SketchUp's wheel zoom does.
    pub fn zoom(&mut self, factor: f32, toward: Option<V3>) {
        if !(factor.is_finite() && factor > 0.0) {
            return;
        }
        let distance = (self.state.distance * factor).clamp(1e-3, 1e9);
        let applied = distance / self.state.distance;
        if let Some(p) = toward.filter(|p| crate::math::finite(*p)) {
            self.state.target = lerp(p, self.state.target, applied);
        }
        self.state.distance = distance;
    }

    /// Frame a sphere of `radius` about `center` in a viewport of the given aspect ratio
    /// (width over height), keeping the direction of view.
    pub fn fit(&mut self, center: V3, radius: f32, aspect: f32) {
        let half_v = self.state.fov_y * 0.5;
        let half_h = (half_v.tan() * aspect.max(0.1)).atan();
        let half = half_v.min(half_h);
        self.state.target = center;
        self.state.distance = (radius.max(1e-3) / half.sin()) * 1.1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::{dist2_2d, len};

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3 * (1.0 + b.abs())
    }

    fn looking_down_y() -> Camera {
        // Eye on -Y, looking along +Y at the origin, level.
        Camera::new(CameraState {
            target: [0.0; 3],
            distance: 10.0,
            azimuth: -std::f32::consts::FRAC_PI_2,
            elevation: 0.0,
            fov_y: std::f32::consts::FRAC_PI_2,
        })
    }

    #[test]
    fn the_target_projects_to_the_centre() {
        let c = Camera::default();
        let p = c.project(c.state.target, 400.0, 300.0).unwrap();
        assert!(close(p[0], 200.0) && close(p[1], 150.0));
        assert!(close(p[2], c.state.distance));
    }

    #[test]
    fn right_is_right_and_up_is_up() {
        let c = looking_down_y();
        let right = c.project([1.0, 0.0, 0.0], 200.0, 200.0).unwrap();
        let up = c.project([0.0, 0.0, 1.0], 200.0, 200.0).unwrap();
        assert!(right[0] > 100.0 && close(right[1], 100.0));
        assert!(up[1] < 100.0 && close(up[0], 100.0));
        // 90° field of view, distance 10: one unit up is a tenth of the half-height.
        assert!(close(up[1], 90.0));
    }

    #[test]
    fn points_behind_the_eye_do_not_project() {
        let c = looking_down_y();
        assert!(c.project([0.0, -20.0, 0.0], 100.0, 100.0).is_none());
    }

    #[test]
    fn a_ray_through_a_projected_point_passes_through_it() {
        let c = Camera::default();
        let p = [120.0, -40.0, 75.0];
        let s = c.project(p, 640.0, 480.0).unwrap();
        let ray = c.ray(s[0], s[1], 640.0, 480.0);
        let t = dot(sub(p, ray.origin), ray.dir);
        assert!(len(sub(ray.at(t), p)) < 1e-2);
    }

    #[test]
    fn orbit_keeps_distance_and_clamps_elevation() {
        let mut c = Camera::default();
        let before = len(sub(c.eye(), c.state.target));
        c.orbit(50.0, 10_000.0);
        assert!(close(len(sub(c.eye(), c.state.target)), before));
        assert!(c.state.elevation <= MAX_ELEVATION);
    }

    #[test]
    fn pan_keeps_the_target_under_the_pointer() {
        let mut c = Camera::default();
        let before = c.project(c.state.target, 400.0, 300.0).unwrap();
        let old_target = c.state.target;
        c.pan(30.0, -20.0, 300.0);
        let moved = c.project(old_target, 400.0, 300.0).unwrap();
        assert!(dist2_2d([moved[0], moved[1]], [before[0] + 30.0, before[1] - 20.0]) < 0.1);
    }

    #[test]
    fn zoom_toward_a_point_keeps_it_still_on_screen() {
        let mut c = Camera::default();
        let p = [300.0, 200.0, 0.0];
        let before = c.project(p, 400.0, 300.0).unwrap();
        c.zoom(0.5, Some(p));
        let after = c.project(p, 400.0, 300.0).unwrap();
        assert!(dist2_2d([before[0], before[1]], [after[0], after[1]]) < 0.1);
        assert!(close(
            c.state.distance,
            CameraState::default().distance * 0.5
        ));
    }

    #[test]
    fn nonsense_zoom_is_ignored() {
        let mut c = Camera::default();
        c.zoom(f32::NAN, None);
        c.zoom(-1.0, None);
        assert_eq!(c, Camera::default());
    }

    #[test]
    fn fit_puts_the_whole_sphere_on_screen() {
        let mut c = Camera::default();
        c.fit([10.0, 10.0, 10.0], 100.0, 2.0);
        for p in [
            [110.0, 10.0, 10.0],
            [10.0, 10.0, 110.0],
            [-90.0, 10.0, 10.0],
        ] {
            let s = c.project(p, 400.0, 200.0).unwrap();
            assert!((0.0..=400.0).contains(&s[0]) && (0.0..=200.0).contains(&s[1]));
        }
    }

    #[test]
    fn wire_state_is_sanitised() {
        let c = Camera::new(CameraState {
            target: [f32::NAN, 0.0, 0.0],
            distance: -5.0,
            azimuth: f32::INFINITY,
            elevation: 9.0,
            fov_y: 0.0,
        });
        let d = CameraState::default();
        assert_eq!(c.state.target, d.target);
        assert_eq!(c.state.distance, d.distance);
        assert_eq!(c.state.azimuth, d.azimuth);
        assert_eq!(c.state.elevation, MAX_ELEVATION);
        assert_eq!(c.state.fov_y, d.fov_y);
    }
}
