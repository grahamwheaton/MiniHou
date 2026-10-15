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

pub fn dot3(a: Vec3, b: Vec3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
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

/// Rotation orders for Euler angle representation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum RotationOrder {
    #[default]
    Xyz,
    Xzy,
    Yxz,
    Yzx,
    Zxy,
    Zyx,
}

pub const ALL_ROTATION_ORDERS: [RotationOrder; 6] = [
    RotationOrder::Xyz,
    RotationOrder::Xzy,
    RotationOrder::Yxz,
    RotationOrder::Yzx,
    RotationOrder::Zxy,
    RotationOrder::Zyx,
];

/// Rotation for Euler angles in degrees, in the specified order.
///
/// Order naming indicates application sequence:
/// - Xyz: X applied first, then Y, then Z -> `Rz * Ry * Rx`
/// - Xzy: X applied first, then Z, then Y -> `Ry * Rz * Rx`
/// - Yxz: Y applied first, then X, then Z -> `Rz * Rx * Ry`
/// - Yzx: Y applied first, then Z, then X -> `Rx * Rz * Ry`
/// - Zxy: Z applied first, then X, then Y -> `Ry * Rx * Rz`
/// - Zyx: Z applied first, then Y, then X -> `Rx * Ry * Rz`
pub fn euler_to_matrix(deg: Vec3, order: RotationOrder) -> Mat3 {
    let [rx, ry, rz] = deg.map(f64::to_radians);
    let (sx, cx) = rx.sin_cos();
    let (sy, cy) = ry.sin_cos();
    let (sz, cz) = rz.sin_cos();
    let x = [[1.0, 0.0, 0.0], [0.0, cx, -sx], [0.0, sx, cx]];
    let y = [[cy, 0.0, sy], [0.0, 1.0, 0.0], [-sy, 0.0, cy]];
    let z = [[cz, -sz, 0.0], [sz, cz, 0.0], [0.0, 0.0, 1.0]];
    match order {
        RotationOrder::Xyz => mul3(&mul3(&z, &y), &x),
        RotationOrder::Xzy => mul3(&mul3(&y, &z), &x),
        RotationOrder::Yxz => mul3(&mul3(&z, &x), &y),
        RotationOrder::Yzx => mul3(&mul3(&x, &z), &y),
        RotationOrder::Zxy => mul3(&mul3(&y, &x), &z),
        RotationOrder::Zyx => mul3(&mul3(&x, &y), &z),
    }
}

/// Rotation for Euler XYZ degrees, X applied first: `Rz @ Ry @ Rx`.
pub fn euler_xyz(deg: Vec3) -> Mat3 {
    euler_to_matrix(deg, RotationOrder::Xyz)
}

/// Axis-angle rotation matrix (angle in degrees, right-handed about unit `axis`).
pub fn axis_angle_matrix(axis: Vec3, angle_deg: f64) -> Mat3 {
    let u = normalize3(axis);
    let rad = angle_deg.to_radians();
    let (s, c) = rad.sin_cos();
    let c1 = 1.0 - c;
    [
        [
            c + u[0] * u[0] * c1,
            u[0] * u[1] * c1 - u[2] * s,
            u[0] * u[2] * c1 + u[1] * s,
        ],
        [
            u[1] * u[0] * c1 + u[2] * s,
            c + u[1] * u[1] * c1,
            u[1] * u[2] * c1 - u[0] * s,
        ],
        [
            u[2] * u[0] * c1 - u[1] * s,
            u[2] * u[1] * c1 + u[0] * s,
            c + u[2] * u[2] * c1,
        ],
    ]
}

/// Shift an angle in degrees by multiples of 360 to minimize distance to `target_deg`.
pub fn unwrap_deg(angle_deg: f64, target_deg: f64) -> f64 {
    angle_deg + ((target_deg - angle_deg) / 360.0).round() * 360.0
}

/// Decompose a 3x3 rotation matrix into Euler angles in `order`.
/// Chooses the solution closest to `current_deg` to prevent 180° flips.
pub fn matrix_to_euler(m: &Mat3, current_deg: Vec3, order: RotationOrder) -> Vec3 {
    let r = m;
    let (cand1, cand2) = match order {
        RotationOrder::Xyz => {
            let cy = (r[0][0] * r[0][0] + r[1][0] * r[1][0]).sqrt();
            if cy > 1e-9 {
                let y1 = (-r[2][0]).atan2(cy);
                let x1 = r[2][1].atan2(r[2][2]);
                let z1 = r[1][0].atan2(r[0][0]);

                let y2 = (-r[2][0]).atan2(-cy);
                let x2 = (-r[2][1]).atan2(-r[2][2]);
                let z2 = (-r[1][0]).atan2(-r[0][0]);
                ([x1, y1, z1], [x2, y2, z2])
            } else {
                let cur_x_rad = current_deg[0].to_radians();
                let (y1, z1) = if r[2][0] < 0.0 {
                    (std::f64::consts::FRAC_PI_2, cur_x_rad + r[1][2].atan2(r[1][1]))
                } else {
                    (-std::f64::consts::FRAC_PI_2, (-r[1][2]).atan2(r[1][1]) - cur_x_rad)
                };
                ([cur_x_rad, y1, z1], [cur_x_rad, y1, z1])
            }
        }
        RotationOrder::Xzy => {
            let cz = (r[0][0] * r[0][0] + r[2][0] * r[2][0]).sqrt();
            if cz > 1e-9 {
                let z1 = r[1][0].atan2(cz);
                let x1 = (-r[1][2]).atan2(r[1][1]);
                let y1 = (-r[2][0]).atan2(r[0][0]);

                let z2 = r[1][0].atan2(-cz);
                let x2 = r[1][2].atan2(-r[1][1]);
                let y2 = r[2][0].atan2(-r[0][0]);
                ([x1, y1, z1], [x2, y2, z2])
            } else {
                let cur_x_rad = current_deg[0].to_radians();
                let (z1, y1) = if r[1][0] > 0.0 {
                    (std::f64::consts::FRAC_PI_2, cur_x_rad + r[0][1].atan2(r[2][1]))
                } else {
                    (-std::f64::consts::FRAC_PI_2, (-r[0][1]).atan2(r[2][1]) - cur_x_rad)
                };
                ([cur_x_rad, y1, z1], [cur_x_rad, y1, z1])
            }
        }
        RotationOrder::Yxz => {
            let cx = (r[2][0] * r[2][0] + r[2][2] * r[2][2]).sqrt();
            if cx > 1e-9 {
                let x1 = r[2][1].atan2(cx);
                let y1 = (-r[2][0]).atan2(r[2][2]);
                let z1 = (-r[0][1]).atan2(r[1][1]);

                let x2 = r[2][1].atan2(-cx);
                let y2 = r[2][0].atan2(-r[2][2]);
                let z2 = r[0][1].atan2(-r[1][1]);
                ([x1, y1, z1], [x2, y2, z2])
            } else {
                let cur_y_rad = current_deg[1].to_radians();
                let (x1, z1) = if r[2][1] > 0.0 {
                    (std::f64::consts::FRAC_PI_2, r[0][2].atan2(r[0][0]) - cur_y_rad)
                } else {
                    (-std::f64::consts::FRAC_PI_2, cur_y_rad + (-r[0][2]).atan2(r[0][0]))
                };
                ([x1, cur_y_rad, z1], [x1, cur_y_rad, z1])
            }
        }
        RotationOrder::Yzx => {
            let cz = (r[1][1] * r[1][1] + r[2][1] * r[2][1]).sqrt();
            if cz > 1e-9 {
                let z1 = (-r[0][1]).atan2(cz);
                let x1 = r[2][1].atan2(r[1][1]);
                let y1 = r[0][2].atan2(r[0][0]);

                let z2 = (-r[0][1]).atan2(-cz);
                let x2 = (-r[2][1]).atan2(-r[1][1]);
                let y2 = (-r[0][2]).atan2(-r[0][0]);
                ([x1, y1, z1], [x2, y2, z2])
            } else {
                let cur_y_rad = current_deg[1].to_radians();
                let (z1, x1) = if r[0][1] < 0.0 {
                    (std::f64::consts::FRAC_PI_2, cur_y_rad + r[1][2].atan2(r[1][0]))
                } else {
                    (-std::f64::consts::FRAC_PI_2, (-r[1][2]).atan2(r[1][0]) - cur_y_rad)
                };
                ([x1, cur_y_rad, z1], [x1, cur_y_rad, z1])
            }
        }
        RotationOrder::Zxy => {
            let cx = (r[1][0] * r[1][0] + r[1][1] * r[1][1]).sqrt();
            if cx > 1e-9 {
                let x1 = (-r[1][2]).atan2(cx);
                let y1 = r[0][2].atan2(r[2][2]);
                let z1 = r[1][0].atan2(r[1][1]);

                let x2 = (-r[1][2]).atan2(-cx);
                let y2 = (-r[0][2]).atan2(-r[2][2]);
                let z2 = (-r[1][0]).atan2(-r[1][1]);
                ([x1, y1, z1], [x2, y2, z2])
            } else {
                let cur_z_rad = current_deg[2].to_radians();
                let (x1, y1) = if r[1][2] < 0.0 {
                    (std::f64::consts::FRAC_PI_2, cur_z_rad + r[0][1].atan2(r[0][0]))
                } else {
                    (-std::f64::consts::FRAC_PI_2, (-r[0][1]).atan2(r[0][0]) - cur_z_rad)
                };
                ([x1, y1, cur_z_rad], [x1, y1, cur_z_rad])
            }
        }
        RotationOrder::Zyx => {
            let cy = (r[0][0] * r[0][0] + r[0][1] * r[0][1]).sqrt();
            if cy > 1e-9 {
                let y1 = r[0][2].atan2(cy);
                let x1 = (-r[1][2]).atan2(r[2][2]);
                let z1 = (-r[0][1]).atan2(r[0][0]);

                let y2 = r[0][2].atan2(-cy);
                let x2 = r[1][2].atan2(-r[2][2]);
                let z2 = r[0][1].atan2(-r[0][0]);
                ([x1, y1, z1], [x2, y2, z2])
            } else {
                let cur_z_rad = current_deg[2].to_radians();
                let (y1, x1) = if r[0][2] > 0.0 {
                    (std::f64::consts::FRAC_PI_2, r[1][0].atan2(r[1][1]) - cur_z_rad)
                } else {
                    (-std::f64::consts::FRAC_PI_2, cur_z_rad + (-r[1][0]).atan2(r[1][1]))
                };
                ([x1, y1, cur_z_rad], [x1, y1, cur_z_rad])
            }
        }
    };

    let c1_deg = [
        unwrap_deg(cand1[0].to_degrees(), current_deg[0]),
        unwrap_deg(cand1[1].to_degrees(), current_deg[1]),
        unwrap_deg(cand1[2].to_degrees(), current_deg[2]),
    ];
    let c2_deg = [
        unwrap_deg(cand2[0].to_degrees(), current_deg[0]),
        unwrap_deg(cand2[1].to_degrees(), current_deg[1]),
        unwrap_deg(cand2[2].to_degrees(), current_deg[2]),
    ];

    let d1 = (c1_deg[0] - current_deg[0]).powi(2)
        + (c1_deg[1] - current_deg[1]).powi(2)
        + (c1_deg[2] - current_deg[2]).powi(2);
    let d2 = (c2_deg[0] - current_deg[0]).powi(2)
        + (c2_deg[1] - current_deg[1]).powi(2)
        + (c2_deg[2] - current_deg[2]).powi(2);

    if d1 <= d2 {
        c1_deg
    } else {
        c2_deg
    }
}

/// Decompose a 3x3 rotation matrix into Euler XYZ degrees.
pub fn matrix_to_euler_xyz(m: &Mat3, current_deg: Vec3) -> Vec3 {
    matrix_to_euler(m, current_deg, RotationOrder::Xyz)
}

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

    /// Exact 4x4 matrix representation matching minihou engine's `transform` node.
    pub fn to_matrix(&self) -> Mat4 {
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
    TrsParams::from_matrix(&composed, current.pivot, current.rotate, RotationOrder::Xyz)
}

/// Compose a translation along axis `a` of `frame` by `distance` units.
pub fn compose_translate_axis(current: &TrsParams, frame: Frame, axis: usize, distance: f64) -> TrsParams {
    let dir = frame.axis(axis);
    let mut out = *current;
    out.translate[0] += dir[0] * distance;
    out.translate[1] += dir[1] * distance;
    out.translate[2] += dir[2] * distance;
    out
}

/// Compose a world translation displacement vector.
pub fn compose_translate(current: &TrsParams, delta_world: Vec3) -> TrsParams {
    let mut out = *current;
    out.translate[0] += delta_world[0];
    out.translate[1] += delta_world[1];
    out.translate[2] += delta_world[2];
    out
}

/// Compose a rotation by `angle_deg` about axis `a` of `frame` through the pivot.
pub fn compose_rotate_axis(current: &TrsParams, frame: Frame, axis: usize, angle_deg: f64, is_local: bool) -> TrsParams {
    let mut out = *current;
    let r_current = euler_xyz(current.rotate);
    let r_new = if is_local {
        let local_axis = match axis { 0 => [1.0, 0.0, 0.0], 1 => [0.0, 1.0, 0.0], _ => [0.0, 0.0, 1.0] };
        let r_delta = axis_angle_matrix(local_axis, angle_deg);
        mul3(&r_current, &r_delta)
    } else {
        let r_delta = axis_angle_matrix(frame.axis(axis), angle_deg);
        mul3(&r_delta, &r_current)
    };
    out.rotate = matrix_to_euler_xyz(&r_new, current.rotate);
    out
}

/// Compose rotation by Euler angles vector through the pivot.
pub fn compose_rotate_euler(current: &TrsParams, frame: Frame, angles_deg: Vec3, is_local: bool) -> TrsParams {
    let mut out = *current;
    let r_current = euler_xyz(current.rotate);
    let r_new = if is_local {
        let r_delta = euler_xyz(angles_deg);
        mul3(&r_current, &r_delta)
    } else {
        let r_delta_local = euler_xyz(angles_deg);
        let f = frame.matrix();
        let ft = transpose3(&f);
        let r_delta_world = mul3(&mul3(&f, &r_delta_local), &ft);
        mul3(&r_delta_world, &r_current)
    };
    out.rotate = matrix_to_euler_xyz(&r_new, current.rotate);
    out
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
    let unrotated = current.rotate.iter().all(|a| a.abs() < 1e-6);
    if is_local || unrotated {
        let mut out = *current;
        for i in 0..3 {
            out.scale[i] *= factors[i];
        }
        return ScaleResult::Direct(out);
    }

    let f = frame.matrix();
    let ft = transpose3(&f);
    let mut s_diag = IDENTITY_3;
    for i in 0..3 { s_diag[i][i] = factors[i]; }
    let s_world = mul3(&mul3(&f, &s_diag), &ft);
    let r = euler_xyz(current.rotate);
    let mut s_curr = IDENTITY_3;
    let eff = current.effective_scale();
    for i in 0..3 { s_curr[i][i] = eff[i]; }
    let l_curr = mul3(&r, &s_curr);
    let l_new = mul3(&s_world, &l_curr);

    if has_shear_3(&l_new, eps) {
        let world_pivot = [
            current.pivot[0] + current.translate[0],
            current.pivot[1] + current.translate[1],
            current.pivot[2] + current.translate[2],
        ];
        ScaleResult::ShearNeedsSecondNode {
            frozen_current: *current,
            second_node: TrsParams {
                translate: [0.0, 0.0, 0.0],
                rotate: [0.0, 0.0, 0.0],
                scale: factors,
                uniform_scale: 1.0,
                pivot: world_pivot,
                axis_rotation: [0.0, 0.0, 0.0],
            },
        }
    } else {
        let c0 = [l_new[0][0], l_new[1][0], l_new[2][0]];
        let c1 = [l_new[0][1], l_new[1][1], l_new[2][1]];
        let c2 = [l_new[0][2], l_new[1][2], l_new[2][2]];
        let s0 = length3(c0);
        let s1 = length3(c1);
        let s2 = length3(c2);
        let r_new = [
            [c0[0]/s0, c1[0]/s1, c2[0]/s2],
            [c0[1]/s0, c1[1]/s1, c2[1]/s2],
            [c0[2]/s0, c1[2]/s1, c2[2]/s2],
        ];
        let mut out = *current;
        out.scale = [s0, s1, s2];
        out.uniform_scale = 1.0;
        out.rotate = matrix_to_euler_xyz(&r_new, current.rotate);
        ScaleResult::Direct(out)
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

