//! Pure transform mathematics mirroring Houdini and the MiniHou transform engine.
//!
//! Provides `TrsParams` <-> 4x4 matrix conversion, matrix composition with
//! orientation frames (Global, Local, Normal, View, Cursor), continuity-preserving
//! Euler angle decomposition for supported rotation orders, and shear detection.
#![allow(dead_code)]

use serde_json::Value;

pub type Mat4 = [[f64; 4]; 4];
pub type Mat3 = [[f64; 3]; 3];
pub type Vec3 = [f64; 3];

pub const IDENTITY_4: Mat4 = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

pub const IDENTITY_3: Mat3 = [
    [1.0, 0.0, 0.0],
    [0.0, 1.0, 0.0],
    [0.0, 0.0, 1.0],
];

pub fn mul4(a: &Mat4, b: &Mat4) -> Mat4 {
    let mut r = [[0.0; 4]; 4];
    for i in 0..4 {
        for j in 0..4 {
            for k in 0..4 {
                r[i][j] += a[i][k] * b[k][j];
            }
        }
    }
    r
}

pub fn mul3(a: &Mat3, b: &Mat3) -> Mat3 {
    let mut r = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            for k in 0..3 {
                r[i][j] += a[i][k] * b[k][j];
            }
        }
    }
    r
}

pub fn transpose3(a: &Mat3) -> Mat3 {
    let mut r = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            r[i][j] = a[j][i];
        }
    }
    r
}

pub fn apply3(m: &Mat3, v: Vec3) -> Vec3 {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

pub fn det3(m: &Mat3) -> f64 {
    m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
}

pub fn inverse_affine(m: &Mat4) -> Option<Mat4> {
    let a = [[m[0][0], m[0][1], m[0][2]], [m[1][0], m[1][1], m[1][2]], [m[2][0], m[2][1], m[2][2]]];
    let det = det3(&a);
    if det.abs() < 1e-12 { return None; }
    let mut inv = IDENTITY_4;
    for (i, row) in inv.iter_mut().enumerate().take(3) { for (j, cell) in row.iter_mut().enumerate().take(3) {
        let r = (j + 1) % 3;
        let s = (j + 2) % 3;
        let c = (i + 1) % 3;
        let d = (i + 2) % 3;
        *cell = (a[r][c] * a[s][d] - a[r][d] * a[s][c]) / det;
    }}
    let t = apply3(&[[inv[0][0], inv[0][1], inv[0][2]],
        [inv[1][0], inv[1][1], inv[1][2]], [inv[2][0], inv[2][1], inv[2][2]]],
        [m[0][3], m[1][3], m[2][3]]);
    for i in 0..3 { inv[i][3] = -t[i]; }
    Some(inv)
}

pub fn dot3(a: Vec3, b: Vec3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub fn cross3(a: Vec3, b: Vec3) -> Vec3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

pub fn length3(v: Vec3) -> f64 {
    dot3(v, v).sqrt()
}

pub fn normalize3(v: Vec3) -> Vec3 {
    let len = length3(v);
    if len > 1e-12 {
        [v[0] / len, v[1] / len, v[2] / len]
    } else {
        [0.0, 0.0, 0.0]
    }
}

pub mod rotation;

pub use rotation::{
    axis_angle_matrix, euler_to_matrix, euler_xyz, matrix_to_euler, matrix_to_euler_xyz,
    unwrap_deg, RotationOrder, ALL_ROTATION_ORDERS,
};

/// Coordinate frame orientation basis (3 unit vectors in world space).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub axes: [Vec3; 3],
}

impl Frame {
    pub fn global() -> Self {
        Self {
            axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        }
    }

    pub fn from_axes(axes: [[f64; 3]; 3]) -> Self {
        Self {
            axes: [
                normalize3(axes[0]),
                normalize3(axes[1]),
                normalize3(axes[2]),
            ],
        }
    }

    pub fn from_f32(axes: [[f32; 3]; 3]) -> Self {
        Self::from_axes([
            [axes[0][0] as f64, axes[0][1] as f64, axes[0][2] as f64],
            [axes[1][0] as f64, axes[1][1] as f64, axes[1][2] as f64],
            [axes[2][0] as f64, axes[2][1] as f64, axes[2][2] as f64],
        ])
    }

    pub fn axis(&self, a: usize) -> Vec3 {
        self.axes[a]
    }

    pub fn from_rotation(r: &Mat3) -> Self {
        Self { axes: [
            [r[0][0], r[1][0], r[2][0]],
            [r[0][1], r[1][1], r[2][1]],
            [r[0][2], r[1][2], r[2][2]],
        ] }
    }

    pub fn from_normal(normal: Vec3, tangent_hint: Vec3) -> Self {
        let z = normalize3(normal);
        if length3(z) < 1e-9 { return Self::global(); }
        let mut x = [0, 1, 2].map(|i| tangent_hint[i] - dot3(tangent_hint, z) * z[i]);
        if length3(x) < 1e-9 {
            let fallback = if z[0].abs() < 0.8 { [1., 0., 0.] } else { [0., 1., 0.] };
            x = [0, 1, 2].map(|i| fallback[i] - dot3(fallback, z) * z[i]);
        }
        let x = normalize3(x);
        Self { axes: [x, cross3(z, x), z] }
    }

    /// Matrix with columns equal to frame axes.
    pub fn matrix(&self) -> Mat3 {
        [
            [self.axes[0][0], self.axes[1][0], self.axes[2][0]],
            [self.axes[0][1], self.axes[1][1], self.axes[2][1]],
            [self.axes[0][2], self.axes[1][2], self.axes[2][2]],
        ]
    }
}

/// Check whether the linear part of an affine 4x4 or 3x3 matrix has shear.
pub fn has_shear(m: &Mat4, eps: f64) -> bool {
    has_shear_3(
        &[
            [m[0][0], m[0][1], m[0][2]],
            [m[1][0], m[1][1], m[1][2]],
            [m[2][0], m[2][1], m[2][2]],
        ],
        eps,
    )
}

pub fn has_shear_3(m: &Mat3, eps: f64) -> bool {
    let c0 = [m[0][0], m[1][0], m[2][0]];
    let c1 = [m[0][1], m[1][1], m[2][1]];
    let c2 = [m[0][2], m[1][2], m[2][2]];
    let l0 = length3(c0);
    let l1 = length3(c1);
    let l2 = length3(c2);
    if l0 < 1e-12 || l1 < 1e-12 || l2 < 1e-12 {
        return false;
    }
    let d01 = dot3(c0, c1).abs() / (l0 * l1);
    let d02 = dot3(c0, c2).abs() / (l0 * l2);
    let d12 = dot3(c1, c2).abs() / (l1 * l2);
    d01 > eps || d02 > eps || d12 > eps
}

/// Parameters for a transform node.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrsParams {
    pub translate: Vec3,
    pub rotate: Vec3, // Euler XYZ degrees
    pub scale: Vec3,
    pub uniform_scale: f64,
    pub pivot: Vec3,
    pub axis_rotation: Vec3,
}

impl Default for TrsParams {
    fn default() -> Self {
        Self {
            translate: [0.0, 0.0, 0.0],
            rotate: [0.0, 0.0, 0.0],
            scale: [1.0, 1.0, 1.0],
            uniform_scale: 1.0,
            pivot: [0.0, 0.0, 0.0],
            axis_rotation: [0.0, 0.0, 0.0],
        }
    }
}

impl TrsParams {
    pub fn from_params(params: &Value) -> Self {
        let v3 = |key: &str, def: Vec3| -> Vec3 {
            params.get(key).and_then(Value::as_array).map_or(def, |arr| {
                [
                    arr.first().and_then(Value::as_f64).unwrap_or(def[0]),
                    arr.get(1).and_then(Value::as_f64).unwrap_or(def[1]),
                    arr.get(2).and_then(Value::as_f64).unwrap_or(def[2]),
                ]
            })
        };
        let uniform_scale = params
            .get("uniform_scale")
            .and_then(Value::as_f64)
            .unwrap_or(1.0);
        Self {
            translate: v3("translate", [0.0, 0.0, 0.0]),
            rotate: v3("rotate", [0.0, 0.0, 0.0]),
            scale: v3("scale", [1.0, 1.0, 1.0]),
            uniform_scale,
            pivot: v3("pivot", [0.0, 0.0, 0.0]),
            axis_rotation: v3("axis_rotation", [0.0, 0.0, 0.0]),
        }
    }

    pub fn effective_scale(&self) -> Vec3 {
        [
            self.scale[0] * self.uniform_scale,
            self.scale[1] * self.uniform_scale,
            self.scale[2] * self.uniform_scale,
        ]
    }

    pub fn world_rotation(&self) -> Mat3 {
        let b = euler_xyz(self.axis_rotation);
        mul3(&mul3(&b, &euler_xyz(self.rotate)), &transpose3(&b))
    }

    pub fn world_pivot(&self) -> Vec3 {
        let bt = apply3(&euler_xyz(self.axis_rotation), self.translate);
        [self.pivot[0] + bt[0], self.pivot[1] + bt[1], self.pivot[2] + bt[2]]
    }

    /// The pivot parameter can change without moving the geometry.
    pub fn with_pivot_preserving_matrix(mut self, pivot: Vec3) -> Self {
        let matrix = self.to_matrix();
        self.pivot = pivot;
        let linear_pivot = [0, 1, 2].map(|i| (0..3).map(|j| matrix[i][j] * pivot[j]).sum::<f64>());
        let world_offset = [0, 1, 2].map(|i| matrix[i][3] + linear_pivot[i] - pivot[i]);
        self.translate = apply3(&transpose3(&euler_xyz(self.axis_rotation)), world_offset);
        self
    }

    pub fn with_world_pivot_preserving_matrix(self, world_pivot: Vec3) -> Self {
        let Some(inverse) = inverse_affine(&self.to_matrix()) else { return self; };
        let input_pivot = [0, 1, 2].map(|i| inverse[i][3] +
            (0..3).map(|j| inverse[i][j] * world_pivot[j]).sum::<f64>());
        self.with_pivot_preserving_matrix(input_pivot)
    }

    pub fn decompose_like(&self, m: &Mat4) -> Self {
        let b = euler_xyz(self.axis_rotation);
        let bt = transpose3(&b);
        let l = [[m[0][0], m[0][1], m[0][2]], [m[1][0], m[1][1], m[1][2]], [m[2][0], m[2][1], m[2][2]]];
        let local = mul3(&mul3(&bt, &l), &b);
        let columns = [0, 1, 2].map(|j| [local[0][j], local[1][j], local[2][j]]);
        let mut scales = columns.map(length3);
        if det3(&local) < 0.0 { scales[0] = -scales[0]; }
        let mut rotation = IDENTITY_3;
        for i in 0..3 { for j in 0..3 {
            rotation[i][j] = if scales[j].abs() > 1e-12 { local[i][j] / scales[j] } else { IDENTITY_3[i][j] };
        }}
        let lp = apply3(&l, self.pivot);
        let offset = [0, 1, 2].map(|i| m[i][3] + lp[i] - self.pivot[i]);
        let translated = apply3(&bt, offset);
        let mut out = *self;
        out.translate = translated;
        out.rotate = matrix_to_euler_xyz(&rotation, self.rotate);
        out.scale = [0, 1, 2].map(|i| scales[i] / self.uniform_scale);
        out
    }

    /// Exact 4x4 matrix representation matching minihou engine's `transform` node.
    pub fn to_matrix(self) -> Mat4 {
        let s = self.effective_scale();
        let r = euler_xyz(self.rotate);
        let b = euler_xyz(self.axis_rotation);
        let bt_b = transpose3(&b);

        let mut rs = r;
        for row in rs.iter_mut() {
            for (j, x) in row.iter_mut().enumerate() {
                *x *= s[j];
            }
        }
        // m = B * (R * S) * B^T
        let m = mul3(&mul3(&b, &rs), &bt_b);
        let bt = apply3(&b, self.translate);
        let shift = [
            self.pivot[0] + bt[0] - (m[0][0] * self.pivot[0] + m[0][1] * self.pivot[1] + m[0][2] * self.pivot[2]),
            self.pivot[1] + bt[1] - (m[1][0] * self.pivot[0] + m[1][1] * self.pivot[1] + m[1][2] * self.pivot[2]),
            self.pivot[2] + bt[2] - (m[2][0] * self.pivot[0] + m[2][1] * self.pivot[1] + m[2][2] * self.pivot[2]),
        ];

        [
            [m[0][0], m[0][1], m[0][2], shift[0]],
            [m[1][0], m[1][1], m[1][2], shift[1]],
            [m[2][0], m[2][1], m[2][2], shift[2]],
            [0.0,     0.0,     0.0,     1.0],
        ]
    }

    /// Decompose a 4x4 affine matrix into `TrsParams` with the given `pivot`.
    pub fn from_matrix(
        m: &Mat4,
        pivot: Vec3,
        current_rotate: Vec3,
        order: RotationOrder,
    ) -> Self {
        let linear = [
            [m[0][0], m[0][1], m[0][2]],
            [m[1][0], m[1][1], m[1][2]],
            [m[2][0], m[2][1], m[2][2]],
        ];
        let translation = [m[0][3], m[1][3], m[2][3]];
        let lp = apply3(&linear, pivot);
        let translate = [
            translation[0] + lp[0] - pivot[0],
            translation[1] + lp[1] - pivot[1],
            translation[2] + lp[2] - pivot[2],
        ];

        let c0 = [linear[0][0], linear[1][0], linear[2][0]];
        let c1 = [linear[0][1], linear[1][1], linear[2][1]];
        let c2 = [linear[0][2], linear[1][2], linear[2][2]];
        let mut s0 = length3(c0);
        let mut s1 = length3(c1);
        let mut s2 = length3(c2);
        if s0 < 1e-12 { s0 = 1.0; }
        if s1 < 1e-12 { s1 = 1.0; }
        if s2 < 1e-12 { s2 = 1.0; }

        let det = det3(&linear);
        if det < 0.0 {
            s0 = -s0;
        }

        let r_mat = [
            [c0[0] / s0, c1[0] / s1, c2[0] / s2],
            [c0[1] / s0, c1[1] / s1, c2[1] / s2],
            [c0[2] / s0, c1[2] / s1, c2[2] / s2],
        ];
        let rotate = matrix_to_euler(&r_mat, current_rotate, order);

        Self {
            translate,
            rotate,
            scale: [s0, s1, s2],
            uniform_scale: 1.0,
            pivot,
            axis_rotation: [0.0, 0.0, 0.0],
        }
    }
}

/// Compose a delta transformation matrix with current parameters.
pub fn compose(
    current: &TrsParams,
    delta_matrix: &Mat4,
    _frame: Frame,
) -> TrsParams {
    let curr_mat = current.to_matrix();
    let composed = mul4(delta_matrix, &curr_mat);
    current.decompose_like(&composed)
}

/// Compose a translation along axis `a` of `frame` by `distance` units.
pub fn compose_translate_axis(current: &TrsParams, frame: Frame, axis: usize, distance: f64) -> TrsParams {
    let dir = frame.axis(axis);
    compose_translate(current, dir.map(|x| x * distance))
}

/// Compose a world translation displacement vector.
pub fn compose_translate(current: &TrsParams, delta_world: Vec3) -> TrsParams {
    let mut out = *current;
    let local = apply3(&transpose3(&euler_xyz(current.axis_rotation)), delta_world);
    for (item, delta) in out.translate.iter_mut().zip(local) { *item += delta; }
    out
}

/// Compose a rotation by `angle_deg` about axis `a` of `frame` through the pivot.
pub fn compose_rotate_axis(current: &TrsParams, frame: Frame, axis: usize, angle_deg: f64, is_local: bool) -> TrsParams {
    let world_axis = if is_local { Frame::from_rotation(&current.world_rotation()).axis(axis) } else { frame.axis(axis) };
    let delta = affine_about_pivot(axis_angle_matrix(world_axis, angle_deg), current.world_pivot());
    compose(current, &delta, frame)
}

/// Compose rotation by Euler angles vector through the pivot.
pub fn compose_rotate_euler(current: &TrsParams, frame: Frame, angles_deg: Vec3, is_local: bool) -> TrsParams {
    let basis = if is_local { Frame::from_rotation(&current.world_rotation()).matrix() } else { frame.matrix() };
    let delta_rotation = mul3(&mul3(&basis, &euler_xyz(angles_deg)), &transpose3(&basis));
    compose(current, &affine_about_pivot(delta_rotation, current.world_pivot()), frame)
}

pub fn affine_about_pivot(linear: Mat3, pivot: Vec3) -> Mat4 {
    let moved = apply3(&linear, pivot);
    [
        [linear[0][0], linear[0][1], linear[0][2], pivot[0] - moved[0]],
        [linear[1][0], linear[1][1], linear[1][2], pivot[1] - moved[1]],
        [linear[2][0], linear[2][1], linear[2][2], pivot[2] - moved[2]],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScaleResult {
    Direct(TrsParams),
    ShearNeedsSecondNode {
        frozen_current: TrsParams,
        second_node: TrsParams,
    },
}

/// Compose a scale about the pivot in `frame`.
///
/// If frame is local or object is unrotated, writes `scale` directly.
/// If composed matrix produces shear, returns `ShearNeedsSecondNode`.
pub fn compose_scale(
    current: &TrsParams,
    frame: Frame,
    factors: Vec3,
    is_local: bool,
    eps: f64,
) -> ScaleResult {
    if is_local {
        let mut out = *current;
        for (item, factor) in out.scale.iter_mut().zip(factors) { *item *= factor; }
        return ScaleResult::Direct(out);
    }

    let f = frame.matrix();
    let ft = transpose3(&f);
    let mut s_diag = IDENTITY_3;
    for i in 0..3 { s_diag[i][i] = factors[i]; }
    let s_world = mul3(&mul3(&f, &s_diag), &ft);
    let delta = affine_about_pivot(s_world, current.world_pivot());
    let composed = mul4(&delta, &current.to_matrix());
    let b = euler_xyz(current.axis_rotation);
    let linear = [[composed[0][0], composed[0][1], composed[0][2]],
        [composed[1][0], composed[1][1], composed[1][2]],
        [composed[2][0], composed[2][1], composed[2][2]]];
    let l_new = mul3(&mul3(&transpose3(&b), &linear), &b);

    if has_shear_3(&l_new, eps) {
        let world_pivot = current.world_pivot();
        let mut frame_rotation = frame.matrix();
        if det3(&frame_rotation) < 0.0 {
            for row in &mut frame_rotation { row[2] = -row[2]; }
        }
        ScaleResult::ShearNeedsSecondNode {
            frozen_current: *current,
            second_node: TrsParams {
                translate: [0.0, 0.0, 0.0],
                rotate: [0.0, 0.0, 0.0],
                scale: factors,
                uniform_scale: 1.0,
                pivot: world_pivot,
                axis_rotation: matrix_to_euler_xyz(&frame_rotation, [0.0; 3]),
            },
        }
    } else {
        ScaleResult::Direct(current.decompose_like(&composed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Lcg(u64);
    impl Lcg {
        fn new(seed: u64) -> Self { Self(seed) }
        fn next_f64(&mut self) -> f64 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1);
            ((self.0 >> 11) as f64) / ((1u64 << 53) as f64)
        }
        fn range(&mut self, min: f64, max: f64) -> f64 {
            min + self.next_f64() * (max - min)
        }
        fn next_u32(&mut self, n: u32) -> u32 {
            (self.next_f64() * (n as f64)) as u32
        }
    }

    fn mat4_diff(a: &Mat4, b: &Mat4) -> f64 {
        let mut max_d = 0.0f64;
        for i in 0..4 {
            for j in 0..4 {
                let d = (a[i][j] - b[i][j]).abs();
                if d > max_d { max_d = d; }
            }
        }
        max_d
    }

    fn mat3_diff(a: &Mat3, b: &Mat3) -> f64 {
        let mut max_d = 0.0f64;
        for i in 0..3 {
            for j in 0..3 {
                let d = (a[i][j] - b[i][j]).abs();
                if d > max_d { max_d = d; }
            }
        }
        max_d
    }

    #[test]
    fn test_euler_round_trip_all_orders() {
        let test_angles = [
            [0.0, 0.0, 0.0],
            [15.0, 30.0, 45.0],
            [-25.0, 40.0, -65.0],
            [120.0, -75.0, 200.0],
            [-185.0, 15.0, 310.0],
            // Gimbal pole cases
            [20.0, 90.0, 40.0],
            [-30.0, -90.0, 50.0],
            [90.0, 20.0, 40.0],
            [-90.0, -30.0, 50.0],
            [20.0, 40.0, 90.0],
            [-30.0, 50.0, -90.0],
        ];

        for &order in &ALL_ROTATION_ORDERS {
            for &angles in &test_angles {
                let m = euler_to_matrix(angles, order);
                let decomposed = matrix_to_euler(&m, [0.0, 0.0, 0.0], order);
                let m2 = euler_to_matrix(decomposed, order);
                assert!(mat3_diff(&m, &m2) < 1e-6, "Order {:?} failed for angles {:?}", order, angles);
            }

            // Pseudo-random round trip
            let mut rng = Lcg::new(42);
            for _ in 0..50 {
                let angles = [rng.range(-180.0, 180.0), rng.range(-89.0, 89.0), rng.range(-180.0, 180.0)];
                let m = euler_to_matrix(angles, order);
                let decomposed = matrix_to_euler(&m, angles, order);
                let m2 = euler_to_matrix(decomposed, order);
                assert!(mat3_diff(&m, &m2) < 1e-6, "Order {:?} failed for random angles {:?}", order, angles);
            }

            // Continuity test: angles should not jump by ~180 degrees on small rotation steps
            let mut current = matrix_to_euler(&euler_to_matrix([0.0, 20.0, 10.0], order), [0.0, 20.0, 10.0], order);
            for step in 1..360 {
                let target = [step as f64 * 0.5, 20.0, 10.0];
                let m = euler_to_matrix(target, order);
                let decomposed = matrix_to_euler(&m, current, order);
                let step_jump = ((decomposed[0] - current[0]).powi(2)
                    + (decomposed[1] - current[1]).powi(2)
                    + (decomposed[2] - current[2]).powi(2)).sqrt();
                assert!(step_jump < 2.0, "Order {:?} flipped at step {}: jump={}", order, step, step_jump);
                current = decomposed;
            }
        }
    }

    #[test]
    fn test_local_x_after_90_deg_z_moves_world_y() {
        let mut trs = TrsParams::default();
        // Rotate 90 degrees about Z
        trs = compose_rotate_axis(&trs, Frame::global(), 2, 90.0, false);
        assert!((trs.rotate[2] - 90.0).abs() < 1e-4);

        // Frame is Local: columns of R
        let r = euler_xyz(trs.rotate);
        let frame = Frame {
            axes: [
                [r[0][0], r[1][0], r[2][0]],
                [r[0][1], r[1][1], r[2][1]],
                [r[0][2], r[1][2], r[2][2]],
            ],
        };
        // Local X is world [0, 1, 0]
        assert!((frame.axis(0)[0] - 0.0).abs() < 1e-6);
        assert!((frame.axis(0)[1] - 1.0).abs() < 1e-6);
        assert!((frame.axis(0)[2] - 0.0).abs() < 1e-6);

        // Move 5 units along Local X
        trs = compose_translate_axis(&trs, frame, 0, 5.0);

        // Must move object along world Y
        assert!((trs.translate[0] - 0.0).abs() < 1e-6);
        assert!((trs.translate[1] - 5.0).abs() < 1e-6);
        assert!((trs.translate[2] - 0.0).abs() < 1e-6);

        // Verifying point [0, 0, 0] transformed by to_matrix() ends up at [0, 5, 0]
        let mat = trs.to_matrix();
        let p_out = [mat[0][3], mat[1][3], mat[2][3]];
        assert!((p_out[0] - 0.0).abs() < 1e-6);
        assert!((p_out[1] - 5.0).abs() < 1e-6);
        assert!((p_out[2] - 0.0).abs() < 1e-6);
    }

    #[test]
    fn axis_rotation_uniform_scale_and_rebased_pivot_compose_exactly() {
        let start = TrsParams {
            translate: [1.0, -0.4, 0.8], rotate: [15.0, 25.0, -40.0],
            scale: [1.5, 0.8, 2.0], uniform_scale: 1.3,
            pivot: [0.2, 0.3, -0.1], axis_rotation: [0.0, 0.0, 35.0],
        };
        let rebased = start.with_pivot_preserving_matrix([1.0, -0.5, 0.25]);
        assert!(mat4_diff(&rebased.to_matrix(), &start.to_matrix()) < 1e-10);
        let world_rebased = start.with_world_pivot_preserving_matrix([2., -1., 0.5]);
        assert!(mat4_diff(&world_rebased.to_matrix(), &start.to_matrix()) < 1e-10);
        for (actual, expected) in world_rebased.world_pivot().into_iter().zip([2., -1., 0.5]) {
            assert!((actual - expected).abs() < 1e-10);
        }
        let local = Frame::from_rotation(&rebased.world_rotation());
        let moved = compose_translate_axis(&rebased, local, 0, 2.0);
        let mut move_delta = IDENTITY_4;
        for (i, row) in move_delta.iter_mut().enumerate().take(3) { row[3] = local.axis(0)[i] * 2.0; }
        let expected = mul4(&move_delta, &start.to_matrix());
        assert!(mat4_diff(&moved.to_matrix(), &expected) < 1e-10);
        let rotated = compose_rotate_axis(&moved, Frame::global(), 1, 37.0, false);
        let expected = mul4(&affine_about_pivot(axis_angle_matrix([0., 1., 0.], 37.), moved.world_pivot()), &expected);
        assert!(mat4_diff(&rotated.to_matrix(), &expected) < 1e-9);
        let inverse = inverse_affine(&start.to_matrix()).unwrap();
        assert!(mat4_diff(&mul4(&start.to_matrix(), &inverse), &IDENTITY_4) < 1e-10);
    }

    #[test]
    fn view_scale_second_node_matches_matrix_delta() {
        let current = TrsParams { rotate: [20., 35., 10.], translate: [1., 2., -1.],
            pivot: [0.2, -0.3, 0.5], ..Default::default() };
        let frame = Frame::from_rotation(&euler_xyz([15., -20., 30.]));
        let factors = [2., 0.7, 1.3];
        let ScaleResult::ShearNeedsSecondNode { second_node, .. } =
            compose_scale(&current, frame, factors, false, 1e-6) else { panic!("expected shear"); };
        let mut diagonal = IDENTITY_3;
        for (i, row) in diagonal.iter_mut().enumerate() { row[i] = factors[i]; }
        let f = frame.matrix();
        let delta = affine_about_pivot(mul3(&mul3(&f, &diagonal), &transpose3(&f)), current.world_pivot());
        assert!(mat4_diff(&second_node.to_matrix(), &delta) < 1e-9);
        assert!(mat4_diff(&mul4(&second_node.to_matrix(), &current.to_matrix()),
            &mul4(&delta, &current.to_matrix())) < 1e-9);
    }

    #[test]
    fn test_global_local_global_consistency() {
        let mut trs = TrsParams::default();

        // 1. Move Global by [2, 1, 0]
        trs = compose_translate(&trs, [2.0, 1.0, 0.0]);

        // 2. Rotate Global by 45 deg about Z
        trs = compose_rotate_axis(&trs, Frame::global(), 2, 45.0, false);

        // 3. Switch to Local frame
        let r = euler_xyz(trs.rotate);
        let local_frame = Frame {
            axes: [
                [r[0][0], r[1][0], r[2][0]],
                [r[0][1], r[1][1], r[2][1]],
                [r[0][2], r[1][2], r[2][2]],
            ],
        };
        // Move along Local X by 3
        trs = compose_translate_axis(&trs, local_frame, 0, 3.0);
        // Rotate along Local Y by 30 deg
        trs = compose_rotate_axis(&trs, local_frame, 1, 30.0, true);

        // 4. Switch back to Global frame
        // Move Global by [0, 0, 4]
        trs = compose_translate(&trs, [0.0, 0.0, 4.0]);
        // Rotate Global by 60 deg about X
        trs = compose_rotate_axis(&trs, Frame::global(), 0, 60.0, false);

        // Compute ground truth 4x4 matrix
        let mut gt = IDENTITY_4;
        let trans1 = [
            [1.0, 0.0, 0.0, 2.0],
            [0.0, 1.0, 0.0, 1.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        gt = mul4(&trans1, &gt);

        let rotz45 = euler_xyz([0.0, 0.0, 45.0]);
        let cur_pivot0 = [gt[0][3], gt[1][3], gt[2][3]];
        let rotz45_4 = [
            [rotz45[0][0], rotz45[0][1], rotz45[0][2], cur_pivot0[0] - (rotz45[0][0]*cur_pivot0[0] + rotz45[0][1]*cur_pivot0[1] + rotz45[0][2]*cur_pivot0[2])],
            [rotz45[1][0], rotz45[1][1], rotz45[1][2], cur_pivot0[1] - (rotz45[1][0]*cur_pivot0[0] + rotz45[1][1]*cur_pivot0[1] + rotz45[1][2]*cur_pivot0[2])],
            [rotz45[2][0], rotz45[2][1], rotz45[2][2], cur_pivot0[2] - (rotz45[2][0]*cur_pivot0[0] + rotz45[2][1]*cur_pivot0[1] + rotz45[2][2]*cur_pivot0[2])],
            [0.0,          0.0,          0.0,          1.0],
        ];
        gt = mul4(&rotz45_4, &gt);

        // Local X is rotz45 * [3, 0, 0]
        let loc_x = apply3(&rotz45, [3.0, 0.0, 0.0]);
        let trans_loc = [
            [1.0, 0.0, 0.0, loc_x[0]],
            [0.0, 1.0, 0.0, loc_x[1]],
            [0.0, 0.0, 1.0, loc_x[2]],
            [0.0, 0.0, 0.0, 1.0],
        ];
        gt = mul4(&trans_loc, &gt);

        // Local Y axis in world space
        let loc_y_axis = apply3(&rotz45, [0.0, 1.0, 0.0]);
        let roty30_world = axis_angle_matrix(loc_y_axis, 30.0);
        let cur_pivot = [gt[0][3], gt[1][3], gt[2][3]];
        // rotate about cur_pivot
        let roty30_4 = [
            [roty30_world[0][0], roty30_world[0][1], roty30_world[0][2], cur_pivot[0] - (roty30_world[0][0]*cur_pivot[0] + roty30_world[0][1]*cur_pivot[1] + roty30_world[0][2]*cur_pivot[2])],
            [roty30_world[1][0], roty30_world[1][1], roty30_world[1][2], cur_pivot[1] - (roty30_world[1][0]*cur_pivot[0] + roty30_world[1][1]*cur_pivot[1] + roty30_world[1][2]*cur_pivot[2])],
            [roty30_world[2][0], roty30_world[2][1], roty30_world[2][2], cur_pivot[2] - (roty30_world[2][0]*cur_pivot[0] + roty30_world[2][1]*cur_pivot[1] + roty30_world[2][2]*cur_pivot[2])],
            [0.0, 0.0, 0.0, 1.0],
        ];
        gt = mul4(&roty30_4, &gt);

        let trans_g2 = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 4.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        gt = mul4(&trans_g2, &gt);

        let cur_pivot2 = [gt[0][3], gt[1][3], gt[2][3]];
        let rotx60 = euler_xyz([60.0, 0.0, 0.0]);
        let rotx60_4 = [
            [rotx60[0][0], rotx60[0][1], rotx60[0][2], cur_pivot2[0] - (rotx60[0][0]*cur_pivot2[0] + rotx60[0][1]*cur_pivot2[1] + rotx60[0][2]*cur_pivot2[2])],
            [rotx60[1][0], rotx60[1][1], rotx60[1][2], cur_pivot2[1] - (rotx60[1][0]*cur_pivot2[0] + rotx60[1][1]*cur_pivot2[1] + rotx60[1][2]*cur_pivot2[2])],
            [rotx60[2][0], rotx60[2][1], rotx60[2][2], cur_pivot2[2] - (rotx60[2][0]*cur_pivot2[0] + rotx60[2][1]*cur_pivot2[1] + rotx60[2][2]*cur_pivot2[2])],
            [0.0, 0.0, 0.0, 1.0],
        ];
        gt = mul4(&rotx60_4, &gt);

        let result_mat = trs.to_matrix();
        assert!(mat4_diff(&result_mat, &gt) < 1e-4, "Global-Local-Global mismatch: diff={}", mat4_diff(&result_mat, &gt));
    }

    #[test]
    fn test_property_random_20_operations() {
        for seed in [101, 202, 303, 404, 505] {
            let mut rng = Lcg::new(seed);
            let mut params = TrsParams::default();
            let mut ground_truth = IDENTITY_4;

            for step in 0..20 {
                let frame_type = step % 3;
                let (frame, is_local) = match frame_type {
                    0 => (Frame::global(), false),
                    1 => {
                        let r = euler_xyz(params.rotate);
                        (Frame {
                            axes: [
                                [r[0][0], r[1][0], r[2][0]],
                                [r[0][1], r[1][1], r[2][1]],
                                [r[0][2], r[1][2], r[2][2]],
                            ],
                        }, true)
                    }
                    _ => {
                        let f = Frame::from_axes([
                            [0.8, 0.6, 0.0],
                            [-0.6, 0.8, 0.0],
                            [0.0, 0.0, 1.0],
                        ]);
                        (f, false)
                    }
                };

                let op_type = rng.next_u32(3);
                match op_type {
                    0 => {
                        // Move
                        let axis = rng.next_u32(3) as usize;
                        let dist = rng.range(-2.0, 2.0);
                        params = compose_translate_axis(&params, frame, axis, dist);
                        let dir = frame.axis(axis);
                        let delta_m = [
                            [1.0, 0.0, 0.0, dir[0] * dist],
                            [0.0, 1.0, 0.0, dir[1] * dist],
                            [0.0, 0.0, 1.0, dir[2] * dist],
                            [0.0, 0.0, 0.0, 1.0],
                        ];
                        ground_truth = mul4(&delta_m, &ground_truth);
                    }
                    1 => {
                        // Rotate
                        let axis = rng.next_u32(3) as usize;
                        let angle = rng.range(-30.0, 30.0);
                        params = compose_rotate_axis(&params, frame, axis, angle, is_local);
                        let rot_axis = if is_local {
                            let r = [
                                [ground_truth[0][0], ground_truth[0][1], ground_truth[0][2]],
                                [ground_truth[1][0], ground_truth[1][1], ground_truth[1][2]],
                                [ground_truth[2][0], ground_truth[2][1], ground_truth[2][2]],
                            ];
                            let local_unit = match axis { 0 => [1.,0.,0.], 1 => [0.,1.,0.], _ => [0.,0.,1.] };
                            normalize3(apply3(&r, local_unit))
                        } else {
                            frame.axis(axis)
                        };
                        let r_delta = axis_angle_matrix(rot_axis, angle);
                        let cur_pivot = [ground_truth[0][3], ground_truth[1][3], ground_truth[2][3]];
                        let rot_m = [
                            [r_delta[0][0], r_delta[0][1], r_delta[0][2], cur_pivot[0] - (r_delta[0][0]*cur_pivot[0] + r_delta[0][1]*cur_pivot[1] + r_delta[0][2]*cur_pivot[2])],
                            [r_delta[1][0], r_delta[1][1], r_delta[1][2], cur_pivot[1] - (r_delta[1][0]*cur_pivot[0] + r_delta[1][1]*cur_pivot[1] + r_delta[1][2]*cur_pivot[2])],
                            [r_delta[2][0], r_delta[2][1], r_delta[2][2], cur_pivot[2] - (r_delta[2][0]*cur_pivot[0] + r_delta[2][1]*cur_pivot[1] + r_delta[2][2]*cur_pivot[2])],
                            [0.0, 0.0, 0.0, 1.0],
                        ];
                        ground_truth = mul4(&rot_m, &ground_truth);
                    }
                    _ => {
                        // Uniform scale
                        let s = rng.range(0.85, 1.15);
                        params = match compose_scale(&params, frame, [s, s, s], is_local, 1e-4) {
                            ScaleResult::Direct(p) => p,
                            _ => panic!("Uniform scale should not produce shear"),
                        };
                        let cur_pivot = [ground_truth[0][3], ground_truth[1][3], ground_truth[2][3]];
                        let scale_m = [
                            [s, 0.0, 0.0, cur_pivot[0] * (1.0 - s)],
                            [0.0, s, 0.0, cur_pivot[1] * (1.0 - s)],
                            [0.0, 0.0, s, cur_pivot[2] * (1.0 - s)],
                            [0.0, 0.0, 0.0, 1.0],
                        ];
                        ground_truth = mul4(&scale_m, &ground_truth);
                    }
                }
            }

            let result_mat = params.to_matrix();
            assert!(mat4_diff(&result_mat, &ground_truth) < 1e-4, "Random property test failed for seed {}: diff={}", seed, mat4_diff(&result_mat, &ground_truth));
        }
    }

    #[test]
    fn test_shear_detection_and_result() {
        let mut trs = TrsParams::default();
        // Unrotated object with non-uniform scale: no shear
        assert_eq!(
            compose_scale(&trs, Frame::global(), [2.0, 1.0, 1.0], false, 1e-4),
            ScaleResult::Direct(TrsParams { scale: [2.0, 1.0, 1.0], ..Default::default() })
        );

        // Rotate by 45 degrees about Z
        trs.rotate = [0.0, 0.0, 45.0];

        // Uniform scale on rotated object: no shear
        match compose_scale(&trs, Frame::global(), [2.0, 2.0, 2.0], false, 1e-4) {
            ScaleResult::Direct(p) => {
                assert!((p.scale[0] - 2.0).abs() < 1e-4);
                assert!((p.scale[1] - 2.0).abs() < 1e-4);
                assert!((p.scale[2] - 2.0).abs() < 1e-4);
                assert!((p.rotate[2] - 45.0).abs() < 1e-4);
            }
            _ => panic!("Uniform scale should never produce shear"),
        }

        // Local scale on rotated object: no shear
        match compose_scale(&trs, Frame::global(), [2.0, 1.0, 1.0], true, 1e-4) {
            ScaleResult::Direct(p) => {
                assert_eq!(p.scale, [2.0, 1.0, 1.0]);
                assert_eq!(p.rotate, [0.0, 0.0, 45.0]);
            }
            _ => panic!("Local scale should never produce shear"),
        }

        // Non-uniform scale on rotated object in Global frame: PRODUCES SHEAR!
        match compose_scale(&trs, Frame::global(), [2.0, 1.0, 1.0], false, 1e-4) {
            ScaleResult::ShearNeedsSecondNode { frozen_current, second_node } => {
                assert_eq!(frozen_current, trs);
                assert_eq!(second_node.scale, [2.0, 1.0, 1.0]);
                assert_eq!(second_node.rotate, [0.0, 0.0, 0.0]);
            }
            ScaleResult::Direct(_) => panic!("Non-uniform global scale on 45° rotated object must produce shear"),
        }
    }
}
