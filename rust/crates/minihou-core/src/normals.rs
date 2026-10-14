//! Face and point normals (`minihou/nodes/normal.py` and its numba kernels).

use rayon::prelude::*;

use crate::geometry::Topology;

/// Newell normal of every primitive, float64, length = 2 x area (summing them area-weights). Degenerate primitives give 0.
pub fn prim_normals(p: &[f32], topo: &Topology) -> Vec<[f64; 3]> {
    prim_normals_raw(p, &topo.vertex_point, &topo.prim_offsets)
}

/// [`prim_normals`] on borrowed arrays.
pub fn prim_normals_raw(p: &[f32], vp: &[i32], offs: &[i32]) -> Vec<[f64; 3]> {
    (0..offs.len() - 1)
        .into_par_iter()
        .map(|prim| {
            let (s, e) = (offs[prim] as usize, offs[prim + 1] as usize);
            let mut n = [0.0f64; 3];
            for v in s..e {
                let w = if v + 1 < e { v + 1 } else { s };
                let (a, b) = (vp[v] as usize * 3, vp[w] as usize * 3);
                let (ax, ay, az) = (p[a] as f64, p[a + 1] as f64, p[a + 2] as f64);
                let (bx, by, bz) = (p[b] as f64, p[b + 1] as f64, p[b + 2] as f64);
                n[0] += (ay - by) * (az + bz);
                n[1] += (az - bz) * (ax + bx);
                n[2] += (ax - bx) * (ay + by);
            }
            n
        })
        .collect()
}

/// `normalize_rows` to float32: rows shorter than 1e-20 are left as they are (zero stays zero).
pub fn normalize_to_f32(rows: &[[f64; 3]]) -> Vec<f32> {
    rows.par_iter()
        .flat_map_iter(|r| {
            let mut len = (r[0] * r[0] + r[1] * r[1] + r[2] * r[2]).sqrt();
            if len < 1e-20 {
                len = 1.0;
            }
            [(r[0] / len) as f32, (r[1] / len) as f32, (r[2] / len) as f32]
        })
        .collect()
}

/// Unit point normals: the sum of the normals of the faces around each point (area-weighted), float32. Unused points get 0.
pub fn point_normals(p: &[f32], topo: &Topology, npoints: usize) -> Vec<f32> {
    point_normals_raw(p, &topo.vertex_point, &topo.prim_offsets, npoints)
}

/// [`point_normals`] on borrowed arrays.
pub fn point_normals_raw(p: &[f32], vp: &[i32], offs: &[i32], npoints: usize) -> Vec<f32> {
    let fn_ = prim_normals_raw(p, vp, offs);
    let mut acc = vec![[0.0f64; 3]; npoints];
    for (prim, n) in fn_.iter().enumerate() {
        for &q in &vp[offs[prim] as usize..offs[prim + 1] as usize] {
            let a = &mut acc[q as usize];
            a[0] += n[0];
            a[1] += n[1];
            a[2] += n[2];
        }
    }
    normalize_to_f32(&acc)
}
