use crate::geometry::Geometry;

use super::boxes::{compute_boxes, covered_fraction, overlapping_pairs};
use super::cloud::{read_pcg, PcgData};
use super::{cross, dot, norm, normalize, sub};

// -- Ray-triangle intersection and inside mesh test -----------------------------------------------------------

pub(crate) fn fan_triangles(geo: &Geometry) -> Vec<[[f64; 3]; 3]> {
    let p = geo.p();
    let vp = &geo.topology.vertex_point;
    let mut tris = Vec::new();
    for r in geo.topology.prim_offsets.windows(2) {
        let (start, end) = (r[0] as usize, r[1] as usize);
        if end - start >= 3 {
            let p0 = [
                p[vp[start] as usize * 3] as f64,
                p[vp[start] as usize * 3 + 1] as f64,
                p[vp[start] as usize * 3 + 2] as f64,
            ];
            for k in start + 1..end - 1 {
                let p1 = [
                    p[vp[k] as usize * 3] as f64,
                    p[vp[k] as usize * 3 + 1] as f64,
                    p[vp[k] as usize * 3 + 2] as f64,
                ];
                let p2 = [
                    p[vp[k + 1] as usize * 3] as f64,
                    p[vp[k + 1] as usize * 3 + 1] as f64,
                    p[vp[k + 1] as usize * 3 + 2] as f64,
                ];
                tris.push([p0, p1, p2]);
            }
        }
    }
    tris
}

pub(crate) fn ray_tri_hit(
    orig: [f64; 3],
    dir: [f64; 3],
    v0: [f64; 3],
    v1: [f64; 3],
    v2: [f64; 3],
) -> Option<f64> {
    let e1 = sub(v1, v0);
    let e2 = sub(v2, v0);
    let pv = cross(dir, e2);
    let det = dot(e1, pv);
    let norm_e = norm(e1) * norm(e2);
    if det.abs() <= 1e-14 * norm_e.max(1e-300) {
        return None;
    }
    let inv = 1.0 / det;
    let tv = sub(orig, v0);
    let u = dot(tv, pv) * inv;
    if !(-1e-12..=1.0 + 1e-12).contains(&u) {
        return None;
    }
    let qv = cross(tv, e1);
    let v = dot(dir, qv) * inv;
    if v < -1e-12 || u + v > 1.0 + 1e-12 {
        return None;
    }
    let t = dot(e2, qv) * inv;
    if t > 0.0 {
        Some(t)
    } else {
        None
    }
}

pub(crate) const PARITY_DIRS: [[f64; 3]; 3] = [
    [0.5774, 0.4122, 0.7046],
    [-0.3313, 0.8661, 0.3742],
    [0.7421, -0.5167, -0.4271],
];

pub(crate) fn point_inside_mesh(pt: [f64; 3], tris: &[[[f64; 3]; 3]], scale: f64) -> bool {
    let eps = 1e-7 * scale.max(1e-12);
    let mut votes = 0;
    for &raw_dir in &PARITY_DIRS {
        let d = normalize(raw_dir);
        let mut parity = 0;
        let mut o = pt;
        for _ in 0..1024 {
            let mut nearest_t = f64::INFINITY;
            for tri in tris {
                if let Some(t) = ray_tri_hit(o, d, tri[0], tri[1], tri[2]) {
                    if t < nearest_t {
                        nearest_t = t;
                    }
                }
            }
            if nearest_t.is_finite() {
                parity ^= 1;
                o = [
                    o[0] + d[0] * (nearest_t + eps),
                    o[1] + d[1] * (nearest_t + eps),
                    o[2] + d[2] * (nearest_t + eps),
                ];
            } else {
                break;
            }
        }
        votes += parity;
    }
    votes >= 2
}

pub(crate) fn is_mesh(geo: &Geometry) -> bool {
    geo.nprims() > 0 && !geo.topology.has_open()
}

pub(crate) fn mesh_coverage(a: &PcgData, mesh: &Geometry, samples: usize) -> Vec<f64> {
    if a.p.is_empty() {
        return Vec::new();
    }
    let tris = fan_triangles(mesh);
    let boxes = compute_boxes(a);
    let scale = if mesh.npoints > 0 {
        let mut max_ext = 0.0f64;
        for k in 0..3 {
            let min_v = mesh
                .p()
                .chunks_exact(3)
                .map(|c| c[k] as f64)
                .fold(f64::INFINITY, f64::min);
            let max_v = mesh
                .p()
                .chunks_exact(3)
                .map(|c| c[k] as f64)
                .fold(f64::NEG_INFINITY, f64::max);
            max_ext = max_ext.max(max_v - min_v);
        }
        max_ext
    } else {
        1.0
    };

    let mut grid = Vec::new();
    if samples <= 1 {
        grid.push([0.0, 0.0, 0.0]);
    } else {
        for ix in 0..samples {
            let gx = (ix as f64) / (samples - 1) as f64 * 2.0 - 1.0;
            for iy in 0..samples {
                let gy = (iy as f64) / (samples - 1) as f64 * 2.0 - 1.0;
                for iz in 0..samples {
                    let gz = (iz as f64) / (samples - 1) as f64 * 2.0 - 1.0;
                    grid.push([gx, gy, gz]);
                }
            }
        }
    }
    let total_pts = grid.len();
    let mut out = Vec::with_capacity(a.p.len());

    for i in 0..a.p.len() {
        let mut inside_count = 0;
        for g in &grid {
            let pt = [
                boxes.c[i][0]
                    + boxes.a[i][0][0] * (g[0] * boxes.h[i][0])
                    + boxes.a[i][0][1] * (g[1] * boxes.h[i][1])
                    + boxes.a[i][0][2] * (g[2] * boxes.h[i][2]),
                boxes.c[i][1]
                    + boxes.a[i][1][0] * (g[0] * boxes.h[i][0])
                    + boxes.a[i][1][1] * (g[1] * boxes.h[i][1])
                    + boxes.a[i][1][2] * (g[2] * boxes.h[i][2]),
                boxes.c[i][2]
                    + boxes.a[i][2][0] * (g[0] * boxes.h[i][0])
                    + boxes.a[i][2][1] * (g[1] * boxes.h[i][1])
                    + boxes.a[i][2][2] * (g[2] * boxes.h[i][2]),
            ];
            if point_inside_mesh(pt, &tris, scale) {
                inside_count += 1;
            }
        }
        out.push(inside_count as f64 / total_pts as f64);
    }
    out
}

pub(crate) fn coverage(a: &PcgData, other: &Geometry) -> (Vec<f64>, Vec<bool>) {
    if is_mesh(other) {
        let frac = mesh_coverage(a, other, 3);
        let touched = frac.iter().map(|&f| f > 0.0).collect();
        return (frac, touched);
    }
    let b = read_pcg(other);
    let ba = compute_boxes(a);
    let bb = compute_boxes(&b);
    let (i, j) = overlapping_pairs(&ba, Some(&bb));
    let mut touched = vec![false; a.p.len()];
    for &idx in &i {
        touched[idx] = true;
    }
    let frac = covered_fraction(&ba, &bb, &i, &j, 3);
    let cov = (0..a.p.len())
        .map(|k| if touched[k] { frac[k].max(1e-3) } else { 0.0 })
        .collect();
    (cov, touched)
}
