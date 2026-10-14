//! Per-point linear maps (`minihou/nodes/transform.py`'s matrix step), in float32 like the numpy code.

use rayon::prelude::*;

/// `(P - pivot) @ m.T + shift` for every point (x, y, z per point).
pub fn affine(p: &[f32], m: [[f32; 3]; 3], pivot: [f32; 3], shift: [f32; 3]) -> Vec<f32> {
    let mut out = vec![0.0f32; p.len()];
    out.par_chunks_exact_mut(3).zip(p.par_chunks_exact(3)).for_each(|(o, q)| {
        let d = [q[0] - pivot[0], q[1] - pivot[1], q[2] - pivot[2]];
        for k in 0..3 {
            o[k] = d[0] * m[k][0] + d[1] * m[k][1] + d[2] * m[k][2] + shift[k];
        }
    });
    out
}

/// `normalize_rows(v @ m.T)`: directions (normals) mapped by `m` and made unit length in float32 (rows shorter than 1e-20 are
/// left as they are).
pub fn linear_normalized(v: &[f32], m: [[f32; 3]; 3]) -> Vec<f32> {
    let mut out = vec![0.0f32; v.len()];
    out.par_chunks_exact_mut(3).zip(v.par_chunks_exact(3)).for_each(|(o, q)| {
        for k in 0..3 {
            o[k] = q[0] * m[k][0] + q[1] * m[k][1] + q[2] * m[k][2];
        }
        let mut len = (o[0] * o[0] + o[1] * o[1] + o[2] * o[2]).sqrt();
        if len < 1e-20 {
            len = 1.0;
        }
        o[0] /= len;
        o[1] /= len;
        o[2] /= len;
    });
    out
}
