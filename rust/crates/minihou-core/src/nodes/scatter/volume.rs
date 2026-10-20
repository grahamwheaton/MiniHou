use std::sync::Arc;
use super::surface::{cross, dot, empty_scatter};
use super::super::np_random::Rng;
use crate::error::{Error, Result};
use crate::geometry::{Attr, Geometry};

pub(in crate::nodes) fn scatter_volume(geo: &Geometry, count: usize, seed: i64) -> Result<Geometry> {
    if geo.topology.has_open() {
        return Err(Error::new("scatter in volume mode works on polygons, but the input has open curves"));
    }
    let p_slice = geo.p();
    if p_slice.is_empty() {
        return Ok(empty_scatter());
    }

    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for chunk in p_slice.chunks_exact(3) {
        for k in 0..3 {
            let v = chunk[k] as f64;
            lo[k] = lo[k].min(v);
            hi[k] = hi[k].max(v);
        }
    }
    let ext = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
    let max_ext = ext[0].max(ext[1]).max(ext[2]);
    if max_ext <= 0.0 {
        return Ok(empty_scatter());
    }

    // Collect triangles for raycasting
    let topo = &geo.topology;
    let vp = &topo.vertex_point;
    let offs = &topo.prim_offsets;
    let mut triangles = Vec::new();
    for prim in 0..geo.nprims() {
        let start = offs[prim] as usize;
        let end = offs[prim + 1] as usize;
        let k = end.saturating_sub(start);
        if k < 3 {
            continue;
        }
        let a = [p_slice[vp[start] as usize * 3] as f64, p_slice[vp[start] as usize * 3 + 1] as f64, p_slice[vp[start] as usize * 3 + 2] as f64];
        for j in 0..(k - 2) {
            let b = [p_slice[vp[start + j + 1] as usize * 3] as f64, p_slice[vp[start + j + 1] as usize * 3 + 1] as f64, p_slice[vp[start + j + 1] as usize * 3 + 2] as f64];
            let c = [p_slice[vp[start + j + 2] as usize * 3] as f64, p_slice[vp[start + j + 2] as usize * 3 + 1] as f64, p_slice[vp[start + j + 2] as usize * 3 + 2] as f64];
            triangles.push([a, b, c]);
        }
    }
    if triangles.is_empty() {
        return Ok(empty_scatter());
    }

    let dirs = [
        [0.5774, 0.4122, 0.7046],
        [-0.3313, 0.8661, 0.3742],
        [0.7421, -0.5167, -0.4271],
    ];
    let norm_dirs: Vec<[f64; 3]> = dirs
        .iter()
        .map(|d| {
            let l = dot(*d, *d).sqrt();
            [d[0] / l, d[1] / l, d[2] / l]
        })
        .collect();

    let mut rng = Rng::new(&[(seed as u64) & 0xFFFFFFFF, 11]);
    let mut found_pos = Vec::with_capacity(count * 3);
    let mut have = 0;
    let mut frac = 0.5f64;

    for _ in 0..64 {
        if have >= count {
            break;
        }
        let needed = (count - have) as f64;
        let n_batch = ((needed / frac.max(0.02) * 1.25) as usize).clamp(1024, 2_000_000);

        let mut batch_inside = 0;
        for _ in 0..n_batch {
            let cand = [
                lo[0] + rng.random() * ext[0],
                lo[1] + rng.random() * ext[1],
                lo[2] + rng.random() * ext[2],
            ];

            let mut votes = 0;
            for d in &norm_dirs {
                let mut crossings = 0;
                for t in &triangles {
                    let e1 = [t[1][0] - t[0][0], t[1][1] - t[0][1], t[1][2] - t[0][2]];
                    let e2 = [t[2][0] - t[0][0], t[2][1] - t[0][1], t[2][2] - t[0][2]];
                    let pv = cross(*d, e2);
                    let det = dot(e1, pv);
                    if det.abs() <= 1e-14 * (dot(e1, e1).sqrt() * dot(e2, e2).sqrt()).max(1e-300) {
                        continue;
                    }
                    let inv = 1.0 / det;
                    let tv = [cand[0] - t[0][0], cand[1] - t[0][1], cand[2] - t[0][2]];
                    let u = dot(tv, pv) * inv;
                    if u < -1e-12 || u > 1.0 + 1e-12 {
                        continue;
                    }
                    let qv = cross(tv, e1);
                    let v = dot(*d, qv) * inv;
                    if v < -1e-12 || u + v > 1.0 + 1e-12 {
                        continue;
                    }
                    let dist = dot(e2, qv) * inv;
                    if dist > 1e-7 * max_ext {
                        crossings ^= 1;
                    }
                }
                votes += crossings;
            }

            if votes >= 2 {
                batch_inside += 1;
                if have < count {
                    found_pos.push(cand[0] as f32);
                    found_pos.push(cand[1] as f32);
                    found_pos.push(cand[2] as f32);
                    have += 1;
                }
            }
        }
        frac = ((batch_inside as f64) / (n_batch as f64)).max(1e-3);
    }

    if found_pos.is_empty() {
        return Ok(empty_scatter());
    }

    let actual_count = found_pos.len() / 3;
    let mut res = Geometry::points_only(found_pos)?;
    res.point.insert("N".into(), Arc::new(Attr::f32(3, vec![0.0; actual_count * 3])));
    res.point.insert("primnum".into(), Arc::new(Attr::i32(1, vec![-1; actual_count])));
    Ok(res)
}
