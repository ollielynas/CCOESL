//! Small vector helpers. `[f32; 3]` throughout, so the types match `MeshData` and the ABI
//! with no conversion.

pub type V3 = [f32; 3];

pub fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

pub fn scale(a: V3, s: f32) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}

/// `a + d * s`.
pub fn add_scaled(a: V3, d: V3, s: f32) -> V3 {
    [a[0] + d[0] * s, a[1] + d[1] * s, a[2] + d[2] * s]
}

pub fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

pub fn len(a: V3) -> f32 {
    dot(a, a).sqrt()
}

/// `a` scaled to unit length, or `fallback` if it has none.
pub fn normalize_or(a: V3, fallback: V3) -> V3 {
    let l = len(a);
    if l > 1e-12 && l.is_finite() {
        scale(a, 1.0 / l)
    } else {
        fallback
    }
}

pub fn lerp(a: V3, b: V3, t: f32) -> V3 {
    add_scaled(a, sub(b, a), t)
}

pub fn finite(a: V3) -> bool {
    a.iter().all(|c| c.is_finite())
}

/// Squared distance between two screen points.
pub fn dist2_2d(a: [f32; 2], b: [f32; 2]) -> f32 {
    let (dx, dy) = (a[0] - b[0], a[1] - b[1]);
    dx * dx + dy * dy
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basics() {
        assert_eq!(cross([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]), [0.0, 0.0, 1.0]);
        assert_eq!(dot([1.0, 2.0, 3.0], [4.0, 5.0, 6.0]), 32.0);
        assert_eq!(normalize_or([0.0, 3.0, 4.0], [0.0; 3]), [0.0, 0.6, 0.8]);
        assert_eq!(normalize_or([0.0; 3], [1.0, 0.0, 0.0]), [1.0, 0.0, 0.0]);
        assert_eq!(
            normalize_or([f32::NAN, 0.0, 0.0], [1.0, 0.0, 0.0]),
            [1.0, 0.0, 0.0]
        );
        assert_eq!(lerp([0.0; 3], [2.0, 4.0, 6.0], 0.5), [1.0, 2.0, 3.0]);
        assert!(!finite([0.0, f32::INFINITY, 0.0]));
    }
}
