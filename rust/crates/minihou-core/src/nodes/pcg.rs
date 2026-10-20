//! PCG nodes: point samplers, point booleans, density tools and spawning (minihou/nodes/pcg.py).
use std::collections::HashMap;
use std::sync::Arc;

use super::{InputSpec, Inputs, NodeSpec};
use crate::error::{Error, Result};
use crate::geometry::{Attr, AttrData, Geometry};
use crate::params::Params;

pub(crate) const DEFAULT_EXTENT: f64 = 0.5;

pub(crate) mod random;

pub use random::{euler_xyz_to_matrix, hash32, matrix_to_euler_xyz, position_seeds, random01};
pub(crate) use random::{mat_mul, rotation_from_z_to};

// -- Euler XYZ & 3x3 helpers ----------------------------------------------------------------------------------

#[inline]
pub(crate) fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[inline]
pub(crate) fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

#[inline]
pub(crate) fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

#[inline]
pub(crate) fn norm(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}

#[inline]
pub(crate) fn normalize(a: [f64; 3]) -> [f64; 3] {
    let l = norm(a);
    if l < 1e-20 {
        a
    } else {
        [a[0] / l, a[1] / l, a[2] / l]
    }
}


// -- PCG point attributes -------------------------------------------------------------------------------------

pub(crate) struct PcgData {
    pub(crate) p: Vec<[f64; 3]>,
    pub(crate) rot: Vec<[f64; 3]>,
    pub(crate) scale: Vec<[f64; 3]>,
    pub(crate) bounds_min: Vec<[f64; 3]>,
    pub(crate) bounds_max: Vec<[f64; 3]>,
    pub(crate) density: Vec<f64>,
    pub(crate) seed: Vec<i32>,
}

pub(crate) fn read_pcg(geo: &Geometry) -> PcgData {
    let n = geo.npoints;
    let pos: Vec<[f64; 3]> = geo
        .p()
        .chunks_exact(3)
        .map(|c| [c[0] as f64, c[1] as f64, c[2] as f64])
        .collect();

    let vec_attr = |name: &str, default: [f64; 3]| -> Vec<[f64; 3]> {
        if let Some(attr) = geo.point.get(name) {
            if attr.width == 3 {
                match &attr.data {
                    AttrData::F32(v) => {
                        return v
                            .chunks_exact(3)
                            .map(|c| [c[0] as f64, c[1] as f64, c[2] as f64])
                            .collect();
                    }
                    AttrData::I32(v) => {
                        return v
                            .chunks_exact(3)
                            .map(|c| [c[0] as f64, c[1] as f64, c[2] as f64])
                            .collect();
                    }
                }
            }
        }
        vec![default; n]
    };

    let density = if let Some(attr) = geo.point.get("density") {
        match &attr.data {
            AttrData::F32(v) => v.chunks(attr.width).map(|r| r[0] as f64).collect(),
            AttrData::I32(v) => v.chunks(attr.width).map(|r| r[0] as f64).collect(),
        }
    } else {
        vec![1.0; n]
    };

    let seed = if let Some(attr) = geo.point.get("seed") {
        match &attr.data {
            AttrData::I32(v) => v.chunks(attr.width).map(|r| r[0]).collect(),
            AttrData::F32(v) => v.chunks(attr.width).map(|r| r[0] as i32).collect(),
        }
    } else {
        position_seeds(geo.p())
    };

    PcgData {
        p: pos,
        rot: vec_attr("rot", [0.0, 0.0, 0.0]),
        scale: vec_attr("scale", [1.0, 1.0, 1.0]),
        bounds_min: vec_attr("bounds_min", [-DEFAULT_EXTENT; 3]),
        bounds_max: vec_attr("bounds_max", [DEFAULT_EXTENT; 3]),
        density,
        seed,
    }
}

pub(crate) fn gather_attr(attr: &Attr, indices: &[usize]) -> Attr {
    let w = attr.width;
    let data = match &attr.data {
        AttrData::F32(v) => AttrData::F32(
            indices
                .iter()
                .flat_map(|&i| v[i * w..(i + 1) * w].iter().copied())
                .collect(),
        ),
        AttrData::I32(v) => AttrData::I32(
            indices
                .iter()
                .flat_map(|&i| v[i * w..(i + 1) * w].iter().copied())
                .collect(),
        ),
    };
    Attr { width: w, data }
}

pub(crate) enum NewAttrVal {
    F32(usize, Vec<f32>),
    I32(usize, Vec<i32>),
}

pub(crate) fn cloud(
    geo: &Geometry,
    keep: Option<&[bool]>,
    new_attrs: Option<HashMap<String, NewAttrVal>>,
) -> Result<Geometry> {
    let idx: Vec<usize> = match keep {
        Some(mask) => mask
            .iter()
            .enumerate()
            .filter_map(|(i, &k)| if k { Some(i) } else { None })
            .collect(),
        None => (0..geo.npoints).collect(),
    };

    let p_src = geo.p();
    let mut p_out = Vec::with_capacity(idx.len() * 3);
    for &i in &idx {
        p_out.extend_from_slice(&p_src[i * 3..i * 3 + 3]);
    }

    let mut out = Geometry::points_only(p_out)?;
    for (name, attr) in &geo.point {
        if name != "P" {
            out.point
                .insert(name.clone(), Arc::new(gather_attr(attr, &idx)));
        }
    }
    out.detail = geo.detail.clone();

    if let Some(attrs) = new_attrs {
        for (name, val) in attrs {
            let attr = match val {
                NewAttrVal::F32(w, v) => {
                    let collected = if v.len() == geo.npoints * w {
                        idx.iter()
                            .flat_map(|&i| v[i * w..(i + 1) * w].iter().copied())
                            .collect()
                    } else {
                        v
                    };
                    Attr {
                        width: w,
                        data: AttrData::F32(collected),
                    }
                }
                NewAttrVal::I32(w, v) => {
                    let collected = if v.len() == geo.npoints * w {
                        idx.iter()
                            .flat_map(|&i| v[i * w..(i + 1) * w].iter().copied())
                            .collect()
                    } else {
                        v
                    };
                    Attr {
                        width: w,
                        data: AttrData::I32(collected),
                    }
                }
            };
            if name == "P" {
                if let AttrData::F32(pts) = attr.data {
                    out.set_p(pts);
                }
            } else {
                out.point.insert(name, Arc::new(attr));
            }
        }
    }
    Ok(out)
}

pub(crate) fn new_points_cloud(
    p: Vec<f32>,
    rot: Option<Vec<f32>>,
    extents: [f64; 3],
    n_attr: Option<Vec<f32>>,
    density: Option<Vec<f32>>,
) -> Result<Geometry> {
    let n = p.len() / 3;
    let seeds = position_seeds(&p);
    let mut out = Geometry::points_only(p)?;

    let r_vec = rot.unwrap_or_else(|| vec![0.0f32; n * 3]);
    let s_vec = vec![1.0f32; n * 3];
    let bmin = extents
        .iter()
        .map(|&e| -e as f32)
        .cycle()
        .take(n * 3)
        .collect();
    let bmax = extents
        .iter()
        .map(|&e| e as f32)
        .cycle()
        .take(n * 3)
        .collect();
    let d_vec = density.unwrap_or_else(|| vec![1.0f32; n]);

    out.point.insert("rot".into(), Arc::new(Attr::f32(3, r_vec)));
    out.point
        .insert("scale".into(), Arc::new(Attr::f32(3, s_vec)));
    out.point
        .insert("bounds_min".into(), Arc::new(Attr::f32(3, bmin)));
    out.point
        .insert("bounds_max".into(), Arc::new(Attr::f32(3, bmax)));
    out.point
        .insert("density".into(), Arc::new(Attr::f32(1, d_vec)));
    out.point
        .insert("seed".into(), Arc::new(Attr::i32(1, seeds)));
    if let Some(nrm) = n_attr {
        out.point.insert("N".into(), Arc::new(Attr::f32(3, nrm)));
    }
    Ok(out)
}

// -- Oriented bounding boxes & SAT ----------------------------------------------------------------------------

pub(crate) struct OrientedBoxes {
    pub(crate) c: Vec<[f64; 3]>,
    pub(crate) a: Vec<[[f64; 3]; 3]>, // a[k] is axis k column vector
    pub(crate) h: Vec<[f64; 3]>,
}

pub(crate) fn compute_boxes(d: &PcgData) -> OrientedBoxes {
    let n = d.p.len();
    let mut c = Vec::with_capacity(n);
    let mut a = Vec::with_capacity(n);
    let mut h = Vec::with_capacity(n);

    for i in 0..n {
        let r = euler_xyz_to_matrix(d.rot[i]);
        let s = d.scale[i];
        let lo = [
            d.bounds_min[i][0] * s[0],
            d.bounds_min[i][1] * s[1],
            d.bounds_min[i][2] * s[2],
        ];
        let hi = [
            d.bounds_max[i][0] * s[0],
            d.bounds_max[i][1] * s[1],
            d.bounds_max[i][2] * s[2],
        ];
        let half = [
            (hi[0] - lo[0]).abs() / 2.0,
            (hi[1] - lo[1]).abs() / 2.0,
            (hi[2] - lo[2]).abs() / 2.0,
        ];
        let mid = [
            (lo[0] + hi[0]) / 2.0,
            (lo[1] + hi[1]) / 2.0,
            (lo[2] + hi[2]) / 2.0,
        ];
        let centre = [
            d.p[i][0] + r[0][0] * mid[0] + r[0][1] * mid[1] + r[0][2] * mid[2],
            d.p[i][1] + r[1][0] * mid[0] + r[1][1] * mid[1] + r[1][2] * mid[2],
            d.p[i][2] + r[2][0] * mid[0] + r[2][1] * mid[1] + r[2][2] * mid[2],
        ];
        let flip = [
            if s[0] == 0.0 { 1.0 } else { s[0].signum() },
            if s[1] == 0.0 { 1.0 } else { s[1].signum() },
            if s[2] == 0.0 { 1.0 } else { s[2].signum() },
        ];
        let mut axes = [[0.0; 3]; 3];
        for row in 0..3 {
            for col in 0..3 {
                axes[row][col] = r[row][col] * flip[col];
            }
        }
        c.push(centre);
        a.push(axes);
        h.push(half);
    }
    OrientedBoxes { c, a, h }
}

pub(crate) fn compute_aabbs(b: &OrientedBoxes) -> (Vec<[f64; 3]>, Vec<[f64; 3]>) {
    let n = b.c.len();
    let mut lo = Vec::with_capacity(n);
    let mut hi = Vec::with_capacity(n);
    for i in 0..n {
        let mut ext = [0.0; 3];
        for k in 0..3 {
            ext[k] = b.a[i][k][0].abs() * b.h[i][0]
                + b.a[i][k][1].abs() * b.h[i][1]
                + b.a[i][k][2].abs() * b.h[i][2];
        }
        lo.push([b.c[i][0] - ext[0], b.c[i][1] - ext[1], b.c[i][2] - ext[2]]);
        hi.push([b.c[i][0] + ext[0], b.c[i][1] + ext[1], b.c[i][2] + ext[2]]);
    }
    (lo, hi)
}

pub(crate) fn obb_overlap_one(
    ca: [f64; 3],
    aa: [[f64; 3]; 3],
    ha: [f64; 3],
    cb: [f64; 3],
    ab: [[f64; 3]; 3],
    hb: [f64; 3],
    tol: f64,
) -> bool {
    let mut r = [[0.0; 3]; 3];
    let mut ar = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            let dot = aa[0][i] * ab[0][j] + aa[1][i] * ab[1][j] + aa[2][i] * ab[2][j];
            r[i][j] = dot;
            ar[i][j] = dot.abs() + 1e-12;
        }
    }
    let d = [cb[0] - ca[0], cb[1] - ca[1], cb[2] - ca[2]];
    let mut t = [0.0; 3];
    for i in 0..3 {
        t[i] = aa[0][i] * d[0] + aa[1][i] * d[1] + aa[2][i] * d[2];
    }
    let max_ha = ha[0].max(ha[1]).max(ha[2]);
    let max_hb = hb[0].max(hb[1]).max(hb[2]);
    let scale = max_ha.max(max_hb).max(1e-12) * tol;

    for i in 0..3 {
        let rb = hb[0] * ar[i][0] + hb[1] * ar[i][1] + hb[2] * ar[i][2];
        if t[i].abs() >= ha[i] + rb - scale {
            return false;
        }
    }

    for j in 0..3 {
        let ra = ha[0] * ar[0][j] + ha[1] * ar[1][j] + ha[2] * ar[2][j];
        let t_proj = (t[0] * r[0][j] + t[1] * r[1][j] + t[2] * r[2][j]).abs();
        if t_proj >= ra + hb[j] - scale {
            return false;
        }
    }

    for i in 0..3 {
        let i1 = (i + 1) % 3;
        let i2 = (i + 2) % 3;
        for j in 0..3 {
            let j1 = (j + 1) % 3;
            let j2 = (j + 2) % 3;
            let ra = ha[i1] * ar[i2][j] + ha[i2] * ar[i1][j];
            let rb = hb[j1] * ar[i][j2] + hb[j2] * ar[i][j1];
            let dist = (t[i2] * r[i1][j] - t[i1] * r[i2][j]).abs();
            if dist > ra + rb {
                return false;
            }
        }
    }
    true
}

pub(crate) fn overlapping_pairs(
    ba: &OrientedBoxes,
    bb: Option<&OrientedBoxes>,
) -> (Vec<usize>, Vec<usize>) {
    let is_same = bb.is_none();
    let bb = bb.unwrap_or(ba);
    let (la, ha) = compute_aabbs(ba);
    let (lb, hb) = if is_same {
        (la.clone(), ha.clone())
    } else {
        compute_aabbs(bb)
    };

    let mut out_i = Vec::new();
    let mut out_j = Vec::new();

    for i in 0..ba.c.len() {
        let start_j = if is_same { i + 1 } else { 0 };
        for j in start_j..bb.c.len() {
            if la[i][0] <= hb[j][0]
                && lb[j][0] <= ha[i][0]
                && la[i][1] <= hb[j][1]
                && lb[j][1] <= ha[i][1]
                && la[i][2] <= hb[j][2]
                && lb[j][2] <= ha[i][2]
            {
                if obb_overlap_one(ba.c[i], ba.a[i], ba.h[i], bb.c[j], bb.a[j], bb.h[j], 1e-9) {
                    out_i.push(i);
                    out_j.push(j);
                }
            }
        }
    }
    (out_i, out_j)
}

pub(crate) fn covered_fraction(
    ba: &OrientedBoxes,
    bb: &OrientedBoxes,
    pairs_i: &[usize],
    pairs_j: &[usize],
    samples: usize,
) -> Vec<f64> {
    let n = ba.c.len();
    if pairs_i.is_empty() {
        return vec![0.0; n];
    }
    let mut grid = Vec::new();
    for ix in 0..samples {
        let gx = (ix as f64 + 0.5) / samples as f64 * 2.0 - 1.0;
        for iy in 0..samples {
            let gy = (iy as f64 + 0.5) / samples as f64 * 2.0 - 1.0;
            for iz in 0..samples {
                let gz = (iz as f64 + 0.5) / samples as f64 * 2.0 - 1.0;
                grid.push([gx, gy, gz]);
            }
        }
    }
    let total_samples = grid.len();
    let mut covered = vec![vec![false; total_samples]; n];

    for (&i, &j) in pairs_i.iter().zip(pairs_j.iter()) {
        for (k, g) in grid.iter().enumerate() {
            if covered[i][k] {
                continue;
            }
            let pt = [
                ba.c[i][0]
                    + ba.a[i][0][0] * (g[0] * ba.h[i][0])
                    + ba.a[i][0][1] * (g[1] * ba.h[i][1])
                    + ba.a[i][0][2] * (g[2] * ba.h[i][2]),
                ba.c[i][1]
                    + ba.a[i][1][0] * (g[0] * ba.h[i][0])
                    + ba.a[i][1][1] * (g[1] * ba.h[i][1])
                    + ba.a[i][1][2] * (g[2] * ba.h[i][2]),
                ba.c[i][2]
                    + ba.a[i][2][0] * (g[0] * ba.h[i][0])
                    + ba.a[i][2][1] * (g[1] * ba.h[i][1])
                    + ba.a[i][2][2] * (g[2] * ba.h[i][2]),
            ];
            let diff = [pt[0] - bb.c[j][0], pt[1] - bb.c[j][1], pt[2] - bb.c[j][2]];
            let mut inside = true;
            for c in 0..3 {
                let loc =
                    bb.a[j][0][c] * diff[0] + bb.a[j][1][c] * diff[1] + bb.a[j][2][c] * diff[2];
                if loc.abs() > bb.h[j][c] + 1e-12 {
                    inside = false;
                    break;
                }
            }
            if inside {
                covered[i][k] = true;
            }
        }
    }

    covered
        .into_iter()
        .map(|row| row.iter().filter(|&&b| b).count() as f64 / total_samples as f64)
        .collect()
}

pub fn greedy_independent(
    n: usize,
    edges_i: &[usize],
    edges_j: &[usize],
    priority: &[f64],
) -> Vec<bool> {
    let mut adj = vec![Vec::new(); n];
    for (&i, &j) in edges_i.iter().zip(edges_j.iter()) {
        adj[i].push(j);
        adj[j].push(i);
    }
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| {
        priority[b]
            .partial_cmp(&priority[a])
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.cmp(&b))
    });

    let mut kept = vec![false; n];
    let mut dropped = vec![false; n];
    for v in order {
        if !dropped[v] {
            kept[v] = true;
            for &u in &adj[v] {
                dropped[u] = true;
            }
        }
    }
    kept
}

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

// =============================================================================================================
// NODE IMPLEMENTATIONS
// =============================================================================================================
