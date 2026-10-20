// ------------------------------------------------------------------------------------------------------------- math

pub(super) type V3 = [f64; 3];
pub(super) type M3 = [[f64; 3]; 3];

pub(super) fn add(a: V3, b: V3) -> V3 { [a[0] + b[0], a[1] + b[1], a[2] + b[2]] }
pub(super) fn sub(a: V3, b: V3) -> V3 { [a[0] - b[0], a[1] - b[1], a[2] - b[2]] }
pub(super) fn mul(a: V3, s: f64) -> V3 { [a[0] * s, a[1] * s, a[2] * s] }
pub(super) fn dot(a: V3, b: V3) -> f64 { a[0] * b[0] + a[1] * b[1] + a[2] * b[2] }
pub(super) fn cross(a: V3, b: V3) -> V3 { [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]] }
pub(super) fn norm(a: V3) -> f64 { dot(a, a).sqrt() }
pub(super) fn unit(a: V3) -> V3 { let n = norm(a); if n > 1e-300 { mul(a, 1.0 / n) } else { [0.0; 3] } }
pub(super) fn mv(m: &M3, v: V3) -> V3 { [dot(m[0], v), dot(m[1], v), dot(m[2], v)] }
pub(super) fn mt(m: &M3) -> M3 { [[m[0][0], m[1][0], m[2][0]], [m[0][1], m[1][1], m[2][1]], [m[0][2], m[1][2], m[2][2]]] }
pub(super) fn mm(a: &M3, b: &M3) -> M3 {
    let bt = mt(b);
    [[dot(a[0], bt[0]), dot(a[0], bt[1]), dot(a[0], bt[2])],
     [dot(a[1], bt[0]), dot(a[1], bt[1]), dot(a[1], bt[2])],
     [dot(a[2], bt[0]), dot(a[2], bt[1]), dot(a[2], bt[2])]]
}
pub(super) fn madd(a: &M3, b: &M3) -> M3 { std::array::from_fn(|i| add(a[i], b[i])) }
pub(super) fn mscale(a: &M3, s: f64) -> M3 { std::array::from_fn(|i| mul(a[i], s)) }
pub(super) const IDENTITY: M3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
pub(super) const ZERO_M: M3 = [[0.0; 3]; 3];
pub(super) fn skew(a: V3) -> M3 { [[0.0, -a[2], a[1]], [a[2], 0.0, -a[0]], [-a[1], a[0], 0.0]] }
pub(super) fn inverse(m: &M3) -> M3 {
    let c0 = cross(m[1], m[2]);
    let det = dot(m[0], c0);
    if det.abs() < 1e-300 {
        return ZERO_M;
    }
    let c1 = cross(m[2], m[0]);
    let c2 = cross(m[0], m[1]);
    mscale(&mt(&[c0, c1, c2]), 1.0 / det)
}

/// Unit quaternion (w, x, y, z).
pub(super) type Q = [f64; 4];
pub(super) fn qmul(a: Q, b: Q) -> Q {
    [a[0] * b[0] - a[1] * b[1] - a[2] * b[2] - a[3] * b[3],
     a[0] * b[1] + a[1] * b[0] + a[2] * b[3] - a[3] * b[2],
     a[0] * b[2] - a[1] * b[3] + a[2] * b[0] + a[3] * b[1],
     a[0] * b[3] + a[1] * b[2] - a[2] * b[1] + a[3] * b[0]]
}
pub(super) fn qconj(q: Q) -> Q { [q[0], -q[1], -q[2], -q[3]] }
pub(super) fn qnorm(q: Q) -> Q { let n = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt(); [q[0] / n, q[1] / n, q[2] / n, q[3] / n] }
pub(super) fn qmat(q: Q) -> M3 {
    let [w, x, y, z] = q;
    [[1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y - w * z), 2.0 * (x * z + w * y)],
     [2.0 * (x * y + w * z), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z - w * x)],
     [2.0 * (x * z - w * y), 2.0 * (y * z + w * x), 1.0 - 2.0 * (x * x + y * y)]]
}
/// Rotate `q` by the small rotation vector `d` (world frame).
pub(super) fn qrotate(q: Q, d: V3) -> Q {
    let half = [0.0, d[0] * 0.5, d[1] * 0.5, d[2] * 0.5];
    let dq = qmul(half, q);
    qnorm([q[0] + dq[0], q[1] + dq[1], q[2] + dq[2], q[3] + dq[3]])
}
/// Rotation vector of a unit quaternion (shortest arc).
pub(super) fn qlog(q: Q) -> V3 {
    let q = if q[0] < 0.0 { [-q[0], -q[1], -q[2], -q[3]] } else { q };
    let s = (q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    if s < 1e-12 {
        return [2.0 * q[1], 2.0 * q[2], 2.0 * q[3]];
    }
    let angle = 2.0 * s.atan2(q[0]);
    mul([q[1], q[2], q[3]], angle / s)
}
