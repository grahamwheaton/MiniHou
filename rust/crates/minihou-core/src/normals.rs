//! Face and point normals (`minihou/nodes/normal.py` and its numba kernels).

use std::sync::atomic::{AtomicU32, Ordering};

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
    let (start, prims) = point_prims(vp, offs, npoints);
    point_normals_with(p, vp, offs, &start, &prims)
}

/// The primitives around each point, one entry per corner, in primitive order: `prims[start[q]..start[q + 1]]` for point `q`
/// (a CSR table, `start` has `npoints + 1` entries). It depends only on the topology, so callers keep it while a deformer changes
/// the positions (`minihou.rust.point_normals` caches it on the topology).
pub fn point_prims(vp: &[i32], offs: &[i32], npoints: usize) -> (Vec<u32>, Vec<u32>) {
    let np = offs.len() - 1;
    let counts: Vec<AtomicU32> = (0..npoints).map(|_| AtomicU32::new(0)).collect();
    vp.par_iter().for_each(|&q| {
        counts[q as usize].fetch_add(1, Ordering::Relaxed);
    });
    let mut start = vec![0u32; npoints + 1];
    for i in 0..npoints {
        start[i + 1] = start[i] + counts[i].load(Ordering::Relaxed);
    }
    let fill: Vec<AtomicU32> = start[..npoints].par_iter().map(|&s| AtomicU32::new(s)).collect();
    drop(counts);
    let mut prims = vec![0u32; vp.len()];
    {
        let out = SharedMut(prims.as_mut_ptr());
        (0..np).into_par_iter().for_each(|prim| {
            let out = &out;
            for &q in &vp[offs[prim] as usize..offs[prim + 1] as usize] {
                let at = fill[q as usize].fetch_add(1, Ordering::Relaxed) as usize;
                // SAFETY: `fetch_add` gives every corner its own slot inside its point's bucket; buckets do not overlap.
                unsafe { *out.0.add(at) = prim as u32 };
            }
        });
    }
    // runs of points own contiguous parts of `prims`: put each bucket in primitive order
    let run = crate::limits::rayon_chunk_size(npoints);
    let mut runs = Vec::with_capacity(npoints / run + 1);
    let mut rest: &mut [u32] = &mut prims;
    for first in (0..npoints).step_by(run) {
        let last = (first + run).min(npoints);
        let (a, r) = rest.split_at_mut((start[last] - start[first]) as usize);
        runs.push((first, last, a));
        rest = r;
    }
    runs.into_par_iter().for_each(|(first, last, a)| {
        let base = start[first];
        for q in first..last {
            a[(start[q] - base) as usize..(start[q + 1] - base) as usize].sort_unstable();
        }
    });
    (start, prims)
}

/// Point normals from a [`point_prims`] table: each point sums the normals of the primitives around it in primitive order,
/// the same additions in the same order as a sequential pass over the primitives (the numba kernel), so the result is
/// identical, computed in parallel per point.
pub fn point_normals_with(p: &[f32], vp: &[i32], offs: &[i32], start: &[u32], prims: &[u32]) -> Vec<f32> {
    let fn_ = prim_normals_raw(p, vp, offs);
    let mut n = vec![0.0f32; (start.len() - 1) * 3];
    n.par_chunks_exact_mut(3).enumerate().for_each(|(q, o)| {
        let mut acc = [0.0f64; 3];
        for &prim in &prims[start[q] as usize..start[q + 1] as usize] {
            let f = &fn_[prim as usize];
            acc[0] += f[0];
            acc[1] += f[1];
            acc[2] += f[2];
        }
        let mut len = (acc[0] * acc[0] + acc[1] * acc[1] + acc[2] * acc[2]).sqrt();
        if len < 1e-20 {
            len = 1.0;
        }
        o.copy_from_slice(&[(acc[0] / len) as f32, (acc[1] / len) as f32, (acc[2] / len) as f32]);
    });
    n
}

use crate::topo::SharedMut;
