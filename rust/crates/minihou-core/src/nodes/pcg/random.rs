// -- hash & seed helpers --------------------------------------------------------------------------------------

#[inline]
pub fn hash32(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7FEB_352D);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846C_A68B);
    x ^= x >> 16;
    x
}

#[inline]
pub fn random01(seed: i32, salt: i64, k: usize) -> f64 {
    let s = (seed as u32) as u64;
    let salt_u = (salt as u64) & 0xFFFF_FFFF;
    let k_term = ((k as u64).wrapping_mul(0x9E37_79B9)) & 0xFFFF_FFFF;
    let inner = hash32((salt_u ^ k_term) as u32);
    let h = hash32((s as u32) ^ inner);
    h as f64 / 4294967296.0
}

pub fn position_seeds(p: &[f32]) -> Vec<i32> {
    p.chunks_exact(3)
        .map(|c| {
            let q0 = (c[0] as f64 * 1000.0).floor() as i64 as u32;
            let q1 = ((c[1] as f64 * 1000.0).floor() as i64)
                .wrapping_mul(3)
                .wrapping_add(0x51ED27) as u32;
            let q2 = ((c[2] as f64 * 1000.0).floor() as i64)
                .wrapping_mul(7)
                .wrapping_add(0x1B873593) as u32;
            let h = hash32(q0) ^ hash32(q1) ^ hash32(q2);
            (hash32(h) & 0x7FFF_FFFF) as i32
        })
        .collect()
}

// -- Euler XYZ & 3x3 helpers ----------------------------------------------------------------------------------

pub fn euler_xyz_to_matrix(e: [f64; 3]) -> [[f64; 3]; 3] {
    let (sx, cx) = e[0].sin_cos();
    let (sy, cy) = e[1].sin_cos();
    let (sz, cz) = e[2].sin_cos();
    [
        [cy * cz, sx * sy * cz - cx * sz, cx * sy * cz + sx * sz],
        [cy * sz, sx * sy * sz + cx * cz, cx * sy * sz - sx * cz],
        [-sy, sx * cy, cx * cy],
    ]
}

pub fn matrix_to_euler_xyz(r: &[[f64; 3]; 3]) -> [f64; 3] {
    let cy = (r[0][0] * r[0][0] + r[1][0] * r[1][0]).sqrt();
    let y = (-r[2][0]).atan2(cy);
    if cy > 1e-9 {
        let x = r[2][1].atan2(r[2][2]);
        let z = r[1][0].atan2(r[0][0]);
        [x, y, z]
    } else {
        let x = (-r[1][2]).atan2(r[1][1]);
        [x, y, 0.0]
    }
}

pub(crate) fn mat_mul(a: &[[f64; 3]; 3], b: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let mut out = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            out[i][j] = a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j];
        }
    }
    out
}

pub(crate) fn rotation_from_z_to(n: [f64; 3]) -> [[f64; 3]; 3] {
    let [nx, ny, c] = n;
    if c < -1.0 + 1e-9 {
        return [[1.0, 0.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, -1.0]];
    }
    let (x, y, s) = (-ny, nx, 1.0 / (1.0 + c));
    [
        [1.0 - s * y * y, s * x * y, y],
        [s * x * y, 1.0 - s * x * x, -x],
        [-y, x, 1.0 - s * (x * x + y * y)],
    ]
}
