use serde_json::Value;

use super::{
    apply3, cross3, det3, dot3, inverse_affine, length3, mul3, mul4, normalize3, transpose3,
    IDENTITY_3, Mat3, Mat4, Vec3,
};
use super::rotation::{
    axis_angle_matrix, euler_xyz, matrix_to_euler, matrix_to_euler_xyz, RotationOrder,
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
