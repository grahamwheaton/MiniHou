use super::{mul3, normalize3, Mat3, Vec3};

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
    let (first, middle, last, pole) = match order {
        RotationOrder::Xyz => (0, 1, 2, m[2][0]),
        RotationOrder::Xzy => (0, 2, 1, m[1][0]),
        RotationOrder::Yxz => (1, 0, 2, m[2][1]),
        RotationOrder::Yzx => (1, 2, 0, m[0][1]),
        RotationOrder::Zxy => (2, 0, 1, m[1][2]),
        RotationOrder::Zyx => (2, 1, 0, m[0][2]),
    };
    if pole.abs() > 1.0 - 1e-10 {
        let alpha = current_deg[first];
        let mut best = None::<(f64, Vec3)>;
        for beta in [-90.0_f64, 90.0] {
            let mut a = [0.0; 3];
            a[first] = -alpha;
            let inverse_first = euler_to_matrix(a, order);
            a = [0.0; 3];
            a[middle] = -beta;
            let inverse_middle = euler_to_matrix(a, order);
            let c = mul3(&mul3(m, &inverse_first), &inverse_middle);
            let gamma = match last {
                0 => c[2][1].atan2(c[1][1]),
                1 => c[0][2].atan2(c[0][0]),
                _ => c[1][0].atan2(c[0][0]),
            }.to_degrees();
            let mut candidate = [0.0; 3];
            candidate[first] = alpha;
            candidate[middle] = unwrap_deg(beta, current_deg[middle]);
            candidate[last] = unwrap_deg(gamma, current_deg[last]);
            let reconstructed = euler_to_matrix(candidate, order);
            let error: f64 = (0..3).flat_map(|i| (0..3).map(move |j| (i, j)))
                .map(|(i, j)| (reconstructed[i][j] - m[i][j]).powi(2)).sum();
            if best.is_none_or(|(old, _)| error < old) { best = Some((error, candidate)); }
        }
        return best.unwrap().1;
    }
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
