//! The 3D viewport: what a guest asks the shell to show, and what the shell reports back.
//!
//! A guest cannot draw 3D. It has no GL context, it links no renderer, and it learns about
//! input a frame late. So the split is the one every other widget uses, taken further: the
//! **shell** owns the camera, the hover, the snapping and the live drag preview — everything
//! that has to track the pointer at display rate — and the **guest** owns the model. The guest
//! names a mesh by URL (the shell fetches it, so the bytes never pass through guest memory),
//! picks a tool, and receives finished gestures as [`ViewEvent`]s: "clicked here", "pushed
//! this face by that much".
//!
//! [`plane_basis`] and [`rect_corners`] live here, not in either side, because the shell draws
//! the rectangle the user is dragging out and the guest then builds the real one. Both must
//! produce the same four corners, so there is one definition.

use crate::decode::DecodeError;

/// "No face": the pointer is over empty space or the ground plane, or nothing is selected.
pub const NO_FACE: u32 = u32::MAX;

/// What the primary button does in a viewport. The middle button always orbits (pans with
/// shift) and the wheel always zooms, whatever the tool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ViewTool {
    /// Click reports the face under the pointer.
    Select = 0,
    /// Click reports a snapped point. With an anchor, the shell draws a rubber band from it.
    Line = 1,
    /// Click reports a snapped point. With an anchor, the shell draws the rectangle from it
    /// to the pointer, laid out by [`rect_corners`].
    Rect = 2,
    /// Drag a face along its normal. The shell draws the extrusion live and reports the
    /// distance once, on release.
    PushPull = 3,
    /// The primary button orbits too, for a trackpad with no middle button.
    Orbit = 4,
}

impl ViewTool {
    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Select),
            1 => Some(Self::Line),
            2 => Some(Self::Rect),
            3 => Some(Self::PushPull),
            4 => Some(Self::Orbit),
            _ => None,
        }
    }
}

/// A point on a plane: where a line or rectangle started, and the plane it is being drawn
/// on. Once a shape has a first point, the shell keeps later points on this plane.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Anchor {
    pub point: [f32; 3],
    pub normal: [f32; 3],
}

/// A face shown pushed or pulled by `distance` along its normal. The guest sets this while it
/// waits for the server to rebuild the model, so the result of a drag stays on screen instead
/// of snapping back until the new mesh arrives.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Extrude {
    pub face: u32,
    pub distance: f32,
}

/// Everything a `Viewport3d` command carries besides its id.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport<'a> {
    /// `x` or `y` of zero or less fills the available width or height.
    pub size: crate::Vec2,
    /// The mesh to show, as a URL the shell fetches, or empty for none.
    pub mesh: &'a str,
    pub tool: ViewTool,
    pub anchor: Option<Anchor>,
    /// Points already placed by the tool in progress (a line's earlier clicks), drawn as a
    /// polyline: little-endian `f32` triples, so an empty path costs one byte.
    pub path: &'a [u8],
    pub preview: Option<Extrude>,
    /// A face to draw as selected, or [`NO_FACE`].
    pub selected: u32,
    pub render: Render,
}

/// Where a viewport's frames are drawn. The picture is the same either way (one renderer runs
/// in both places); only where the work happens differs. Picking, snapping and the drag
/// preview always stay in the shell, so a click never waits on the network.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum Render {
    /// The shell decides: in the browser, unless the mesh is heavier than it draws smoothly.
    #[default]
    Auto = 0,
    /// Always in the browser. Orbiting costs no network at all.
    Local = 1,
    /// Always on the server, for a slow device. Each new camera position is a round trip; the
    /// shell keeps showing the last frame until the next arrives.
    Server = 2,
}

impl Render {
    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Auto),
            1 => Some(Self::Local),
            2 => Some(Self::Server),
            _ => None,
        }
    }
}

impl Viewport<'_> {
    /// The points of [`Viewport::path`].
    pub fn path_points(&self) -> impl Iterator<Item = [f32; 3]> + '_ {
        self.path.chunks_exact(12).map(|c| {
            let f = |i: usize| f32::from_le_bytes([c[i], c[i + 1], c[i + 2], c[i + 3]]);
            [f(0), f(4), f(8)]
        })
    }
}

/// Encode points for [`Viewport::path`].
pub fn encode_path(points: &[[f32; 3]]) -> alloc::vec::Vec<u8> {
    let mut out = alloc::vec::Vec::with_capacity(points.len() * 12);
    for p in points {
        for c in p {
            out.extend_from_slice(&c.to_le_bytes());
        }
    }
    out
}

/// A path is whole points or it is malformed.
pub(crate) fn check_path(path: &[u8]) -> Result<(), DecodeError> {
    if path.len().is_multiple_of(12) {
        Ok(())
    } else {
        Err(DecodeError::InvalidEnum)
    }
}

/// What the pointer landed on, strongest first. The shell draws the matching cue (a green dot
/// for an endpoint, a cyan one for a midpoint, a red/green/blue line for an axis) and reports
/// it with the point, so an app can say "Endpoint" in its status line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Snap {
    /// Nothing nearby: the point is where the pointer ray meets the drawing plane or ground.
    Free = 0,
    OnFace = 1,
    Endpoint = 2,
    Midpoint = 3,
    AxisX = 4,
    AxisY = 5,
    AxisZ = 6,
}

impl Snap {
    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Free),
            1 => Some(Self::OnFace),
            2 => Some(Self::Endpoint),
            3 => Some(Self::Midpoint),
            4 => Some(Self::AxisX),
            5 => Some(Self::AxisY),
            6 => Some(Self::AxisZ),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ViewAction {
    /// A click with the Select, Line or Rect tool.
    Click = 1,
    /// A push/pull drag finished: `face` moved `distance` along `normal`.
    PushPull = 2,
}

/// A finished gesture in a viewport, delivered as a `VIEWPORT` event before the guest's next
/// frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewEvent {
    /// The viewport's widget id.
    pub id: u64,
    pub action: ViewAction,
    pub snap: Snap,
    /// The face under the pointer (for a click) or the face dragged, or [`NO_FACE`].
    pub face: u32,
    pub point: [f32; 3],
    /// The face's outward normal, or the drawing plane's (the ground's is +Z).
    pub normal: [f32; 3],
    pub distance: f32,
}

const VIEW_EVENT_LEN: usize = 8 + 1 + 1 + 4 + 12 + 12 + 4;

pub fn encode_view_event(e: &ViewEvent) -> alloc::vec::Vec<u8> {
    let mut out = alloc::vec::Vec::with_capacity(VIEW_EVENT_LEN);
    out.extend_from_slice(&e.id.to_le_bytes());
    out.push(e.action as u8);
    out.push(e.snap as u8);
    out.extend_from_slice(&e.face.to_le_bytes());
    for c in e.point.iter().chain(&e.normal) {
        out.extend_from_slice(&c.to_le_bytes());
    }
    out.extend_from_slice(&e.distance.to_le_bytes());
    out
}

/// `None` if the payload is the wrong length or names an unknown action or snap. The guest
/// drops such an event rather than acting on half of it.
pub fn decode_view_event(p: &[u8]) -> Option<ViewEvent> {
    if p.len() != VIEW_EVENT_LEN {
        return None;
    }
    let f = |at: usize| f32::from_le_bytes([p[at], p[at + 1], p[at + 2], p[at + 3]]);
    let mut id = [0u8; 8];
    id.copy_from_slice(&p[..8]);
    let action = match p[8] {
        1 => ViewAction::Click,
        2 => ViewAction::PushPull,
        _ => return None,
    };
    Some(ViewEvent {
        id: u64::from_le_bytes(id),
        action,
        snap: Snap::from_u8(p[9])?,
        face: u32::from_le_bytes([p[10], p[11], p[12], p[13]]),
        point: [f(14), f(18), f(22)],
        normal: [f(26), f(30), f(34)],
        distance: f(38),
    })
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn add_scaled(a: [f32; 3], d: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] + d[0] * s, a[1] + d[1] * s, a[2] + d[2] * s]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// `core` has no `f32::sqrt` without `std`, and this crate takes no dependency to get one. A
/// bit-level first guess plus Newton steps is exact to the last bit or two, which is all a
/// direction needs.
fn sqrt(x: f32) -> f32 {
    if x <= 0.0 || x.is_nan() || x.is_infinite() {
        return if x > 0.0 { x } else { 0.0 };
    }
    let mut y = f32::from_bits((x.to_bits() >> 1) + 0x1fbd_1df5);
    for _ in 0..4 {
        y = 0.5 * (y + x / y);
    }
    y
}

fn normalized(a: [f32; 3]) -> [f32; 3] {
    let len = sqrt(dot(a, a));
    if len > 1e-12 {
        [a[0] / len, a[1] / len, a[2] / len]
    } else {
        [0.0, 0.0, 1.0]
    }
}

/// Two in-plane directions for a plane with normal `n`, with `u × v = n`.
///
/// A rectangle's sides follow these, so they are chosen the way a person expects: on a
/// horizontal plane, along X and Y; on any other plane, `u` is horizontal and `v` runs up the
/// slope.
pub fn plane_basis(n: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    let n = normalized(n);
    let u = if n[2].abs() > 0.999 {
        [1.0, 0.0, 0.0]
    } else {
        normalized(cross([0.0, 0.0, 1.0], n))
    };
    (u, cross(n, u))
}

/// The rectangle from `anchor` to `to` on the plane through `anchor` with normal `n`, sides
/// along [`plane_basis`], wound counter-clockwise seen from the side `n` points to. `to` need
/// not lie on the plane; it is projected.
pub fn rect_corners(anchor: [f32; 3], n: [f32; 3], to: [f32; 3]) -> [[f32; 3]; 4] {
    let (u, v) = plane_basis(n);
    let d = sub(to, anchor);
    let (du, dv) = (dot(d, u), dot(d, v));
    let a = anchor;
    let b = add_scaled(a, u, du);
    let c = add_scaled(b, v, dv);
    let e = add_scaled(a, v, dv);
    // Going u then v is counter-clockwise about u × v = n only when both steps have the same
    // sign; otherwise walk the other way round.
    if du * dv >= 0.0 {
        [a, b, c, e]
    } else {
        [a, e, c, b]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() < 1e-5)
    }

    fn winding_normal(c: &[[f32; 3]; 4]) -> [f32; 3] {
        cross(sub(c[1], c[0]), sub(c[2], c[0]))
    }

    #[test]
    fn basis_is_right_handed_for_any_normal() {
        for n in [
            [0.0, 0.0, 1.0],
            [0.0, 0.0, -1.0],
            [1.0, 0.0, 0.0],
            [0.0, -1.0, 0.0],
            [0.3, 0.4, 0.5],
        ] {
            let (u, v) = plane_basis(n);
            assert!(close(cross(u, v), normalized(n)), "normal {n:?}");
            assert!(dot(u, u) > 0.999 && dot(v, v) > 0.999);
        }
    }

    #[test]
    fn ground_rectangles_follow_the_axes() {
        let c = rect_corners([0.0; 3], [0.0, 0.0, 1.0], [2.0, 3.0, 0.0]);
        assert_eq!(
            c,
            [
                [0.0, 0.0, 0.0],
                [2.0, 0.0, 0.0],
                [2.0, 3.0, 0.0],
                [0.0, 3.0, 0.0]
            ]
        );
    }

    #[test]
    fn rectangles_wind_toward_the_normal_whichever_way_they_are_dragged() {
        let n = [0.0, 0.0, 1.0];
        for to in [
            [2.0, 3.0, 0.0],
            [-2.0, 3.0, 0.0],
            [2.0, -3.0, 0.0],
            [-2.0, -3.0, 0.0],
        ] {
            let c = rect_corners([0.0; 3], n, to);
            assert!(dot(winding_normal(&c), n) > 0.0, "dragged to {to:?}");
        }
    }

    #[test]
    fn points_off_the_plane_are_projected() {
        let c = rect_corners([0.0, 0.0, 5.0], [0.0, 0.0, 1.0], [1.0, 1.0, 9.0]);
        assert!(c.iter().all(|p| p[2] == 5.0));
    }

    #[test]
    fn sqrt_is_accurate() {
        for x in [1e-6_f32, 0.25, 1.0, 2.0, 9.0, 1234.5, 1e12] {
            let r = sqrt(x);
            assert!((r * r - x).abs() <= x * 1e-6, "sqrt({x}) = {r}");
        }
        assert_eq!(sqrt(0.0), 0.0);
        assert_eq!(sqrt(-1.0), 0.0);
        assert_eq!(sqrt(f32::NAN), 0.0);
    }

    #[test]
    fn view_events_round_trip() {
        let e = ViewEvent {
            id: 0x0102_0304_0506_0708,
            action: ViewAction::PushPull,
            snap: Snap::Midpoint,
            face: 7,
            point: [1.0, -2.0, 3.5],
            normal: [0.0, 0.0, 1.0],
            distance: -250.0,
        };
        assert_eq!(decode_view_event(&encode_view_event(&e)), Some(e));
    }

    #[test]
    fn malformed_view_events_are_dropped() {
        let e = ViewEvent {
            id: 1,
            action: ViewAction::Click,
            snap: Snap::Free,
            face: NO_FACE,
            point: [0.0; 3],
            normal: [0.0, 0.0, 1.0],
            distance: 0.0,
        };
        let good = encode_view_event(&e);
        assert_eq!(decode_view_event(&good[..good.len() - 1]), None);
        let mut bad_action = good.clone();
        bad_action[8] = 9;
        assert_eq!(decode_view_event(&bad_action), None);
        let mut bad_snap = good;
        bad_snap[9] = 200;
        assert_eq!(decode_view_event(&bad_snap), None);
    }

    #[test]
    fn paths_round_trip_and_reject_partial_points() {
        let pts = [[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]];
        let bytes = encode_path(&pts);
        let vp = Viewport {
            size: crate::Vec2::new(0.0, 0.0),
            mesh: "",
            tool: ViewTool::Line,
            anchor: None,
            path: &bytes,
            preview: None,
            selected: NO_FACE,
            render: Render::Auto,
        };
        assert_eq!(vp.path_points().collect::<alloc::vec::Vec<_>>(), pts);
        assert!(check_path(&bytes).is_ok());
        assert_eq!(check_path(&bytes[1..]), Err(DecodeError::InvalidEnum));
    }
}
