//! PCG nodes: point samplers, point booleans, density tools and spawning (minihou/nodes/pcg.py).
use std::collections::HashMap;
use std::sync::Arc;

use super::{InputSpec, Inputs, NodeSpec};
use crate::error::{Error, Result};
use crate::geometry::{Attr, AttrData, Geometry};
use crate::params::Params;

const DEFAULT_EXTENT: f64 = 0.5;

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

#[inline]
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[inline]
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

#[inline]
fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

#[inline]
fn norm(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}

#[inline]
fn normalize(a: [f64; 3]) -> [f64; 3] {
    let l = norm(a);
    if l < 1e-20 {
        a
    } else {
        [a[0] / l, a[1] / l, a[2] / l]
    }
}

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

fn rotation_from_z_to(n: [f64; 3]) -> [[f64; 3]; 3] {
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

fn gather_attr(attr: &Attr, indices: &[usize]) -> Attr {
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

fn new_points_cloud(
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

struct OrientedBoxes {
    c: Vec<[f64; 3]>,
    a: Vec<[[f64; 3]; 3]>, // a[k] is axis k column vector
    h: Vec<[f64; 3]>,
}

fn compute_boxes(d: &PcgData) -> OrientedBoxes {
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

fn compute_aabbs(b: &OrientedBoxes) -> (Vec<[f64; 3]>, Vec<[f64; 3]>) {
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

fn obb_overlap_one(
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

fn overlapping_pairs(
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

fn covered_fraction(
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

fn fan_triangles(geo: &Geometry) -> Vec<[[f64; 3]; 3]> {
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

fn ray_tri_hit(
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

const PARITY_DIRS: [[f64; 3]; 3] = [
    [0.5774, 0.4122, 0.7046],
    [-0.3313, 0.8661, 0.3742],
    [0.7421, -0.5167, -0.4271],
];

fn point_inside_mesh(pt: [f64; 3], tris: &[[[f64; 3]; 3]], scale: f64) -> bool {
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

fn is_mesh(geo: &Geometry) -> bool {
    geo.nprims() > 0 && !geo.topology.has_open()
}

fn mesh_coverage(a: &PcgData, mesh: &Geometry, samples: usize) -> Vec<f64> {
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

fn coverage(a: &PcgData, other: &Geometry) -> (Vec<f64>, Vec<bool>) {
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




// 4. pcg_union
pub const PCG_UNION: NodeSpec = NodeSpec {
    name: "pcg_union",
    inputs: &[InputSpec {
        name: "inputs",
        multi: true,
        optional: false,
    }],
    params: crate::params!("density_function": "keep"),
    cook: pcg_union,
};

fn pcg_union(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let list = &inputs[0];
    if list.is_empty() {
        return new_points_cloud(Vec::new(), None, [0.5, 0.5, 0.5], None, None);
    }
    let density_function = p.e("density_function", &["keep", "maximum", "add"])?;

    let mut parts = Vec::new();
    let mut which = Vec::new();
    for (k, g) in list.iter().enumerate() {
        let d = read_pcg(g);
        let n = g.npoints;
        which.extend(vec![k; n]);
        let mut new_attrs = HashMap::new();
        new_attrs.insert(
            "rot".into(),
            NewAttrVal::F32(
                3,
                d.rot
                    .iter()
                    .flat_map(|r| [r[0] as f32, r[1] as f32, r[2] as f32])
                    .collect(),
            ),
        );
        new_attrs.insert(
            "scale".into(),
            NewAttrVal::F32(
                3,
                d.scale
                    .iter()
                    .flat_map(|s| [s[0] as f32, s[1] as f32, s[2] as f32])
                    .collect(),
            ),
        );
        new_attrs.insert(
            "bounds_min".into(),
            NewAttrVal::F32(
                3,
                d.bounds_min
                    .iter()
                    .flat_map(|b| [b[0] as f32, b[1] as f32, b[2] as f32])
                    .collect(),
            ),
        );
        new_attrs.insert(
            "bounds_max".into(),
            NewAttrVal::F32(
                3,
                d.bounds_max
                    .iter()
                    .flat_map(|b| [b[0] as f32, b[1] as f32, b[2] as f32])
                    .collect(),
            ),
        );
        new_attrs.insert(
            "density".into(),
            NewAttrVal::F32(1, d.density.iter().map(|&v| v as f32).collect()),
        );
        new_attrs.insert("seed".into(), NewAttrVal::I32(1, d.seed));
        parts.push(Arc::new(cloud(g, None, Some(new_attrs))?));
    }

    let out = if parts.len() > 1 {
        super::merge::merge(&[parts], &Params::resolve(&[], None)?)?
    } else {
        (*parts[0]).clone()
    };

    if density_function == "keep" || list.len() < 2 {
        return Ok(out);
    }

    let a = read_pcg(&out);
    let boxes = compute_boxes(&a);
    let (i_pairs, j_pairs) = overlapping_pairs(&boxes, None);

    let mut d = a.density.clone();
    for (&i, &j) in i_pairs.iter().zip(j_pairs.iter()) {
        if which[i] != which[j] {
            if density_function == "maximum" {
                d[i] = d[i].max(a.density[j]);
                d[j] = d[j].max(a.density[i]);
            } else {
                d[i] += a.density[j];
                d[j] += a.density[i];
            }
        }
    }
    let d_f32: Vec<f32> = d.iter().map(|&v| v.clamp(0.0, 1.0) as f32).collect();
    let mut attrs = HashMap::new();
    attrs.insert("density".into(), NewAttrVal::F32(1, d_f32));
    cloud(&out, None, Some(attrs))
}

// 5. pcg_difference
pub const PCG_DIFFERENCE: NodeSpec = NodeSpec {
    name: "pcg_difference",
    inputs: &[
        InputSpec {
            name: "points",
            multi: false,
            optional: false,
        },
        InputSpec {
            name: "subtract",
            multi: false,
            optional: false,
        },
    ],
    params: crate::params!("density_function": "binary"),
    cook: pcg_difference,
};

fn pcg_difference(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let points = PCG_DIFFERENCE.one(inputs, 0)?;
    let subtract = PCG_DIFFERENCE.one(inputs, 1)?;
    let density_function = p.e("density_function", &["binary", "subtract"])?;

    let a = read_pcg(&points);
    let (cov, touched) = coverage(&a, &subtract);

    if density_function == "binary" {
        let keep: Vec<bool> = touched.iter().map(|&t| !t).collect();
        cloud(&points, Some(&keep), None)
    } else {
        let mut d_out = Vec::with_capacity(points.npoints);
        let mut keep = Vec::with_capacity(points.npoints);
        for i in 0..points.npoints {
            let d = a.density[i] * (1.0 - cov[i]);
            keep.push(d > 1e-6);
            d_out.push(d as f32);
        }
        let mut attrs = HashMap::new();
        attrs.insert("density".into(), NewAttrVal::F32(1, d_out));
        cloud(&points, Some(&keep), Some(attrs))
    }
}

// 6. pcg_intersection
pub const PCG_INTERSECTION: NodeSpec = NodeSpec {
    name: "pcg_intersection",
    inputs: &[
        InputSpec {
            name: "points",
            multi: false,
            optional: false,
        },
        InputSpec {
            name: "other",
            multi: false,
            optional: false,
        },
    ],
    params: crate::params!("density_function": "binary"),
    cook: pcg_intersection,
};

fn pcg_intersection(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let points = PCG_INTERSECTION.one(inputs, 0)?;
    let other = PCG_INTERSECTION.one(inputs, 1)?;
    let density_function = p.e("density_function", &["binary", "multiply"])?;

    let a = read_pcg(&points);
    let (cov, touched) = coverage(&a, &other);

    if density_function == "binary" {
        cloud(&points, Some(&touched), None)
    } else {
        let mut d_out = Vec::with_capacity(points.npoints);
        let mut keep = Vec::with_capacity(points.npoints);
        for i in 0..points.npoints {
            let d = a.density[i] * cov[i];
            keep.push(d > 1e-6);
            d_out.push(d as f32);
        }
        let mut attrs = HashMap::new();
        attrs.insert("density".into(), NewAttrVal::F32(1, d_out));
        cloud(&points, Some(&keep), Some(attrs))
    }
}


// 9. pcg_distance
pub const PCG_DISTANCE: NodeSpec = NodeSpec {
    name: "pcg_distance",
    inputs: &[
        InputSpec {
            name: "points",
            multi: false,
            optional: false,
        },
        InputSpec {
            name: "target",
            multi: false,
            optional: false,
        },
    ],
    params: crate::params!(
        "max_distance": 5.0;
        "mode": "set";
        "invert": false
    ),
    cook: pcg_distance,
};

fn closest_point_on_tri(p: [f64; 3], a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> [f64; 3] {
    let ab = sub(b, a);
    let ac = sub(c, a);
    let ap = sub(p, a);
    let d1 = dot(ab, ap);
    let d2 = dot(ac, ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }

    let bp = sub(p, b);
    let d3 = dot(ab, bp);
    let d4 = dot(ac, bp);
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }

    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return [a[0] + ab[0] * v, a[1] + ab[1] * v, a[2] + ab[2] * v];
    }

    let cp = sub(p, c);
    let d5 = dot(ab, cp);
    let d6 = dot(ac, cp);
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }

    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return [a[0] + ac[0] * w, a[1] + ac[1] * w, a[2] + ac[2] * w];
    }

    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        let bc = sub(c, b);
        return [b[0] + bc[0] * w, b[1] + bc[1] * w, b[2] + bc[2] * w];
    }

    let denom = 1.0 / (va + vb + vc);
    let v = vb * denom;
    let w = vc * denom;
    [
        a[0] + ab[0] * v + ac[0] * w,
        a[1] + ab[1] * v + ac[1] * w,
        a[2] + ab[2] * v + ac[2] * w,
    ]
}

fn distance_to_target(points_p: &[[f64; 3]], target: &Geometry) -> Vec<f64> {
    if is_mesh(target) {
        let tris = fan_triangles(target);
        return points_p
            .iter()
            .map(|&p| {
                let mut best = f64::INFINITY;
                for tri in &tris {
                    let cp = closest_point_on_tri(p, tri[0], tri[1], tri[2]);
                    let d = norm(sub(cp, p));
                    if d < best {
                        best = d;
                    }
                }
                best
            })
            .collect();
    }
    if target.nprims() > 0 {
        let edges = crate::topo::edges(&target.topology, target.npoints);
        let pt = target.p();
        let mut best = vec![f64::INFINITY; points_p.len()];
        for pair in edges.points.chunks_exact(2) {
            let (i0, i1) = (pair[0] as usize, pair[1] as usize);
            let a = [
                pt[i0 * 3] as f64,
                pt[i0 * 3 + 1] as f64,
                pt[i0 * 3 + 2] as f64,
            ];
            let b = [
                pt[i1 * 3] as f64,
                pt[i1 * 3 + 1] as f64,
                pt[i1 * 3 + 2] as f64,
            ];
            let ab = sub(b, a);
            let l2 = dot(ab, ab).max(1e-300);
            for (p_idx, &p) in points_p.iter().enumerate() {
                let pa = sub(p, a);
                let t = (dot(pa, ab) / l2).clamp(0.0, 1.0);
                let q = [a[0] + t * ab[0], a[1] + t * ab[1], a[2] + t * ab[2]];
                let d = norm(sub(p, q));
                if d < best[p_idx] {
                    best[p_idx] = d;
                }
            }
        }
        return best;
    }
    let pt = target.p();
    points_p
        .iter()
        .map(|&p| {
            let mut best = f64::INFINITY;
            for c in pt.chunks_exact(3) {
                let d = norm(sub(p, [c[0] as f64, c[1] as f64, c[2] as f64]));
                if d < best {
                    best = d;
                }
            }
            best
        })
        .collect()
}

fn pcg_distance(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let points = PCG_DISTANCE.one(inputs, 0)?;
    let target = PCG_DISTANCE.one(inputs, 1)?;
    let max_dist = p.f("max_distance")?;
    let mode = p.e("mode", &["set", "multiply"])?;
    let invert = p.b("invert")?;

    let a = read_pcg(&points);
    let dist = if target.npoints == 0 {
        vec![f64::INFINITY; points.npoints]
    } else {
        distance_to_target(&a.p, &target)
    };

    let mut d_out = Vec::with_capacity(points.npoints);
    for i in 0..points.npoints {
        let mut v = (dist[i] / max_dist).clamp(0.0, 1.0);
        if invert {
            v = 1.0 - v;
        }
        let d = if mode == "set" { v } else { a.density[i] * v };
        d_out.push(d as f32);
    }

    let mut attrs = HashMap::new();
    attrs.insert("density".into(), NewAttrVal::F32(1, d_out));
    cloud(&points, None, Some(attrs))
}

// 10. pcg_copy_points
pub const PCG_COPY_POINTS: NodeSpec = NodeSpec {
    name: "pcg_copy_points",
    inputs: &[
        InputSpec {
            name: "source",
            multi: false,
            optional: false,
        },
        InputSpec {
            name: "targets",
            multi: false,
            optional: false,
        },
    ],
    params: crate::params!(
        "inherit_rotation": true;
        "inherit_scale": true
    ),
    cook: pcg_copy_points,
};

fn pcg_copy_points(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let source = PCG_COPY_POINTS.one(inputs, 0)?;
    let targets = PCG_COPY_POINTS.one(inputs, 1)?;
    let inherit_rot = p.b("inherit_rotation")?;
    let inherit_scale = p.b("inherit_scale")?;

    let s = read_pcg(&source);
    let t = read_pcg(&targets);
    let (ns, nt) = (source.npoints, targets.npoints);
    let total = ns * nt;

    let mut p_out = Vec::with_capacity(total * 3);
    let mut rot_out = Vec::with_capacity(total * 3);
    let mut scale_out = Vec::with_capacity(total * 3);
    let mut bmin_out = Vec::with_capacity(total * 3);
    let mut bmax_out = Vec::with_capacity(total * 3);
    let mut density_out = Vec::with_capacity(total);
    let mut seed_out = Vec::with_capacity(total);

    for ti in 0..nt {
        let rt = if inherit_rot {
            euler_xyz_to_matrix(t.rot[ti])
        } else {
            [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
        };
        let st = if inherit_scale {
            t.scale[ti]
        } else {
            [1.0, 1.0, 1.0]
        };

        for si in 0..ns {
            let sp = [
                s.p[si][0] * st[0],
                s.p[si][1] * st[1],
                s.p[si][2] * st[2],
            ];
            let pt = [
                t.p[ti][0] + rt[0][0] * sp[0] + rt[0][1] * sp[1] + rt[0][2] * sp[2],
                t.p[ti][1] + rt[1][0] * sp[0] + rt[1][1] * sp[1] + rt[1][2] * sp[2],
                t.p[ti][2] + rt[2][0] * sp[0] + rt[2][1] * sp[1] + rt[2][2] * sp[2],
            ];
            p_out.extend_from_slice(&[pt[0] as f32, pt[1] as f32, pt[2] as f32]);

            let rs = euler_xyz_to_matrix(s.rot[si]);
            let final_r = mat_mul(&rt, &rs);
            let euler = matrix_to_euler_xyz(&final_r);
            rot_out.extend_from_slice(&[euler[0] as f32, euler[1] as f32, euler[2] as f32]);

            scale_out.extend_from_slice(&[
                (s.scale[si][0] * st[0]) as f32,
                (s.scale[si][1] * st[1]) as f32,
                (s.scale[si][2] * st[2]) as f32,
            ]);

            bmin_out.extend_from_slice(&[
                s.bounds_min[si][0] as f32,
                s.bounds_min[si][1] as f32,
                s.bounds_min[si][2] as f32,
            ]);
            bmax_out.extend_from_slice(&[
                s.bounds_max[si][0] as f32,
                s.bounds_max[si][1] as f32,
                s.bounds_max[si][2] as f32,
            ]);

            density_out.push((s.density[si] * t.density[ti]) as f32);

            let s_val = (s.seed[si] as i64 as u64).wrapping_mul(0x9E3779B1);
            let mixed = hash32((s_val ^ (t.seed[ti] as i64 as u64)) as u32);
            seed_out.push((mixed & 0x7FFF_FFFF) as i32);
        }
    }

    let mut out = Geometry::points_only(p_out)?;
    // Copy other attributes from source
    for (name, attr) in &source.point {
        if name != "P"
            && name != "rot"
            && name != "scale"
            && name != "bounds_min"
            && name != "bounds_max"
            && name != "density"
            && name != "seed"
        {
            let w = attr.width;
            let mut gathered = Vec::with_capacity(total * w);
            for _ in 0..nt {
                match &attr.data {
                    AttrData::F32(v) => gathered.extend_from_slice(v),
                    AttrData::I32(v) => gathered.extend(v.iter().map(|&x| x as f32)),
                }
            }
            out.point
                .insert(name.clone(), Arc::new(Attr::f32(w, gathered)));
        }
    }
    out.point.insert("rot".into(), Arc::new(Attr::f32(3, rot_out)));
    out.point
        .insert("scale".into(), Arc::new(Attr::f32(3, scale_out)));
    out.point
        .insert("bounds_min".into(), Arc::new(Attr::f32(3, bmin_out)));
    out.point
        .insert("bounds_max".into(), Arc::new(Attr::f32(3, bmax_out)));
    out.point
        .insert("density".into(), Arc::new(Attr::f32(1, density_out)));
    out.point
        .insert("seed".into(), Arc::new(Attr::i32(1, seed_out)));
    Ok(out)
}

// 11. pcg_self_prune
pub const PCG_SELF_PRUNE: NodeSpec = NodeSpec {
    name: "pcg_self_prune",
    inputs: &[InputSpec {
        name: "points",
        multi: false,
        optional: false,
    }],
    params: crate::params!(
        "priority": "large_first";
        "seed": 0
    ),
    cook: pcg_self_prune,
};

fn pcg_self_prune(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let points = PCG_SELF_PRUNE.one(inputs, 0)?;
    let priority = p.e(
        "priority",
        &["large_first", "small_first", "density", "random"],
    )?;
    let seed = p.i("seed")?;

    let a = read_pcg(&points);
    let n = points.npoints;
    let boxes = compute_boxes(&a);
    let (i_pairs, j_pairs) = overlapping_pairs(&boxes, None);

    let mut vol = Vec::with_capacity(n);
    let mut tie = Vec::with_capacity(n);
    let mut max_vol = 1.0f64;
    for i in 0..n {
        let dx = (a.bounds_max[i][0] - a.bounds_min[i][0]).abs() * a.scale[i][0].abs();
        let dy = (a.bounds_max[i][1] - a.bounds_min[i][1]).abs() * a.scale[i][1].abs();
        let dz = (a.bounds_max[i][2] - a.bounds_min[i][2]).abs() * a.scale[i][2].abs();
        let v = dx * dy * dz;
        max_vol = max_vol.max(v);
        vol.push(v);
        tie.push(random01(a.seed[i], seed, 0) * 1e-6);
    }

    let pri: Vec<f64> = match priority.as_str() {
        "large_first" => vol
            .iter()
            .zip(tie.iter())
            .map(|(&v, &t)| v + t * max_vol)
            .collect(),
        "small_first" => vol
            .iter()
            .zip(tie.iter())
            .map(|(&v, &t)| -v + t * max_vol)
            .collect(),
        "density" => a
            .density
            .iter()
            .zip(tie.iter())
            .map(|(&d, &t)| d + t)
            .collect(),
        _ => tie,
    };

    let keep = greedy_independent(n, &i_pairs, &j_pairs, &pri);
    cloud(&points, Some(&keep), None)
}

// 12. pcg_projection
pub const PCG_PROJECTION: NodeSpec = NodeSpec {
    name: "pcg_projection",
    inputs: &[
        InputSpec {
            name: "points",
            multi: false,
            optional: false,
        },
        InputSpec {
            name: "surface",
            multi: false,
            optional: false,
        },
    ],
    params: crate::params!(
        "direction": [0.0, 0.0, -1.0];
        "align": true;
        "keep_misses": false
    ),
    cook: pcg_projection,
};

fn pcg_projection(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let points = PCG_PROJECTION.one(inputs, 0)?;
    let surface = PCG_PROJECTION.one(inputs, 1)?;
    let direction = p.v3("direction")?;
    let align = p.b("align")?;
    let keep_misses = p.b("keep_misses")?;

    let a = read_pcg(&points);
    let n = points.npoints;
    let tris = fan_triangles(&surface);

    let d_len = norm(direction);
    let d = if d_len < 1e-12 {
        [0.0, 0.0, -1.0]
    } else {
        [
            direction[0] / d_len,
            direction[1] / d_len,
            direction[2] / d_len,
        ]
    };
    let neg_d = [-d[0], -d[1], -d[2]];

    let mut hit = vec![false; n];
    let mut hit_p = a.p.clone();
    let mut hit_tri = vec![0usize; n];

    for i in 0..n {
        let orig = a.p[i];
        let mut t1 = f64::INFINITY;
        let mut tri1 = 0;
        let mut t2 = f64::INFINITY;
        let mut tri2 = 0;

        for (t_idx, tri) in tris.iter().enumerate() {
            if let Some(t) = ray_tri_hit(orig, d, tri[0], tri[1], tri[2]) {
                if t < t1 {
                    t1 = t;
                    tri1 = t_idx;
                }
            }
            if let Some(t) = ray_tri_hit(orig, neg_d, tri[0], tri[1], tri[2]) {
                if t < t2 {
                    t2 = t;
                    tri2 = t_idx;
                }
            }
        }

        let use_back = !t1.is_finite() || (t2.is_finite() && t2 < t1);
        let chosen_t = if use_back { -t2 } else { t1 };
        let chosen_tri = if use_back { tri2 } else { tri1 };

        if chosen_t.is_finite() {
            hit[i] = true;
            hit_p[i] = [
                orig[0] + d[0] * chosen_t,
                orig[1] + d[1] * chosen_t,
                orig[2] + d[2] * chosen_t,
            ];
            hit_tri[i] = chosen_tri;
        }
    }

    let mut new_p = Vec::with_capacity(n * 3);
    for pt in &hit_p {
        new_p.extend_from_slice(&[pt[0] as f32, pt[1] as f32, pt[2] as f32]);
    }

    let mut attrs = HashMap::new();
    attrs.insert("P".into(), NewAttrVal::F32(3, new_p));

    if align && hit.iter().any(|&h| h) {
        let mut new_rot = Vec::with_capacity(n * 3);
        let mut new_norm = Vec::with_capacity(n * 3);
        for i in 0..n {
            if hit[i] {
                let tri = tris[hit_tri[i]];
                let mut nrm = cross(sub(tri[1], tri[0]), sub(tri[2], tri[0]));
                nrm = normalize(nrm);
                if dot(nrm, neg_d) < 0.0 {
                    nrm = [-nrm[0], -nrm[1], -nrm[2]];
                }
                let r_align = rotation_from_z_to(nrm);
                let r_z = euler_xyz_to_matrix([0.0, 0.0, a.rot[i][2]]);
                let r_final = mat_mul(&r_align, &r_z);
                let euler = matrix_to_euler_xyz(&r_final);
                new_rot.extend_from_slice(&[euler[0] as f32, euler[1] as f32, euler[2] as f32]);
                new_norm.extend_from_slice(&[nrm[0] as f32, nrm[1] as f32, nrm[2] as f32]);
            } else {
                new_rot.extend_from_slice(&[
                    a.rot[i][0] as f32,
                    a.rot[i][1] as f32,
                    a.rot[i][2] as f32,
                ]);
                new_norm.extend_from_slice(&[0.0, 0.0, 1.0]);
            }
        }
        attrs.insert("rot".into(), NewAttrVal::F32(3, new_rot));
        attrs.insert("N".into(), NewAttrVal::F32(3, new_norm));
    }

    if keep_misses {
        cloud(&points, None, Some(attrs))
    } else {
        cloud(&points, Some(&hit), Some(attrs))
    }
}

// 14. pcg_spline_sampler
pub const PCG_SPLINE_SAMPLER: NodeSpec = NodeSpec {
    name: "pcg_spline_sampler",
    inputs: &[InputSpec {
        name: "curve",
        multi: false,
        optional: false,
    }],
    params: crate::params!(
        "mode": "on_curve";
        "spacing": 1.0;
        "extents": [0.5, 0.5, 0.5];
        "jitter": 0.0;
        "seed": 0
    ),
    cook: pcg_spline_sampler,
};

fn pcg_spline_sampler(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let curve = PCG_SPLINE_SAMPLER.one(inputs, 0)?;
    let mode = p.e("mode", &["on_curve", "interior"])?;
    let spacing = p.f("spacing")?;
    let extents = p.v3("extents")?;
    let jitter = p.f("jitter")?;
    let seed = p.i("seed")?;

    if curve.nprims() == 0 {
        return new_points_cloud(Vec::new(), None, extents, None, None);
    }

    if mode == "on_curve" {
        let cp = curve.p();
        let vp = &curve.topology.vertex_point;
        let mut out_x = Vec::new();
        let mut s_lists: Vec<Vec<[f64; 3]>> = Vec::new();

        for i in 0..curve.nprims() {
            let (start, end) = (
                curve.topology.prim_offsets[i] as usize,
                curve.topology.prim_offsets[i + 1] as usize,
            );
            if end - start < 2 {
                continue;
            }
            let closed = curve.topology.is_closed(i);
            let mut seg_lengths = Vec::new();
            let mut cum_s = vec![0.0];
            let mut total_l = 0.0;
            let count = if closed { end - start } else { end - start - 1 };
            for j in 0..count {
                let v0 = vp[start + j] as usize;
                let v1 = vp[start + (j + 1) % (end - start)] as usize;
                let p0 = [cp[v0 * 3] as f64, cp[v0 * 3 + 1] as f64, cp[v0 * 3 + 2] as f64];
                let p1 = [cp[v1 * 3] as f64, cp[v1 * 3 + 1] as f64, cp[v1 * 3 + 2] as f64];
                let l = norm(sub(p1, p0));
                seg_lengths.push(l);
                total_l += l;
                cum_s.push(total_l);
            }

            let mut targets = Vec::new();
            if total_l <= 0.0 {
                targets.push(0.0);
            } else {
                let mut s = 0.0;
                while s <= total_l + 1e-9 {
                    targets.push(s);
                    s += spacing;
                }
                if closed && targets.len() > 1 && (targets.last().unwrap() - total_l).abs() < 1e-9 {
                    targets.pop();
                }
            }

            let mut prim_pts = Vec::new();
            for &t_s in &targets {
                let s_clamped = t_s.clamp(0.0, total_l);
                let seg_idx = match cum_s.binary_search_by(|v| {
                    v.partial_cmp(&s_clamped).unwrap_or(std::cmp::Ordering::Equal)
                }) {
                    Ok(idx) => idx.min(seg_lengths.len().saturating_sub(1)),
                    Err(idx) => (idx - 1).min(seg_lengths.len().saturating_sub(1)),
                };
                let frac = if seg_lengths[seg_idx] > 1e-12 {
                    (s_clamped - cum_s[seg_idx]) / seg_lengths[seg_idx]
                } else {
                    0.0
                };
                let v0 = vp[start + seg_idx] as usize;
                let v1 = vp[start + (seg_idx + 1) % (end - start)] as usize;
                let p0 = [cp[v0 * 3] as f64, cp[v0 * 3 + 1] as f64, cp[v0 * 3 + 2] as f64];
                let p1 = [cp[v1 * 3] as f64, cp[v1 * 3 + 1] as f64, cp[v1 * 3 + 2] as f64];
                let pt = [
                    p0[0] + frac * (p1[0] - p0[0]),
                    p0[1] + frac * (p1[1] - p0[1]),
                    p0[2] + frac * (p1[2] - p0[2]),
                ];
                prim_pts.push(pt);
                out_x.push(pt);
            }
            s_lists.push(prim_pts);
        }

        let mut tg = vec![[1.0, 0.0, 0.0]; out_x.len()];
        let mut off = 0;
        for prim_pts in s_lists {
            let m = prim_pts.len();
            if m >= 2 {
                for k in 0..m {
                    let grad = if k == 0 {
                        sub(prim_pts[1], prim_pts[0])
                    } else if k == m - 1 {
                        sub(prim_pts[m - 1], prim_pts[m - 2])
                    } else {
                        let d = sub(prim_pts[k + 1], prim_pts[k - 1]);
                        [d[0] * 0.5, d[1] * 0.5, d[2] * 0.5]
                    };
                    let ln = norm(grad);
                    tg[off + k] = if ln > 0.0 {
                        normalize(grad)
                    } else {
                        [1.0, 0.0, 0.0]
                    };
                }
            }
            off += m;
        }

        let mut rot = Vec::with_capacity(out_x.len() * 3);
        let mut p_out = Vec::with_capacity(out_x.len() * 3);
        for i in 0..out_x.len() {
            let yaw = tg[i][1].atan2(tg[i][0]);
            let pitch = -(tg[i][2].clamp(-1.0, 1.0)).asin();
            rot.extend_from_slice(&[0.0f32, pitch as f32, yaw as f32]);
            p_out.extend_from_slice(&[out_x[i][0] as f32, out_x[i][1] as f32, out_x[i][2] as f32]);
        }
        return new_points_cloud(p_out, Some(rot), extents, None, None);
    }

    // mode == "interior"
    let cp = curve.p();
    let vp = &curve.topology.vertex_point;
    let mut all_out = Vec::new();

    for k_prim in 0..curve.nprims() {
        let (start, end) = (
            curve.topology.prim_offsets[k_prim] as usize,
            curve.topology.prim_offsets[k_prim + 1] as usize,
        );
        let m = end - start;
        if m < 3 {
            continue;
        }
        let mut q: Vec<[f64; 3]> = Vec::with_capacity(m);
        for v in &vp[start..end] {
            let idx = *v as usize;
            q.push([cp[idx * 3] as f64, cp[idx * 3 + 1] as f64, cp[idx * 3 + 2] as f64]);
        }
        let mut c = [0.0; 3];
        for pt in &q {
            c[0] += pt[0];
            c[1] += pt[1];
            c[2] += pt[2];
        }
        c = [c[0] / m as f64, c[1] / m as f64, c[2] / m as f64];

        let mut nrm = [0.0; 3];
        for i in 0..m {
            let next_i = (i + 1) % m;
            let d1 = sub(q[i], c);
            let d2 = sub(q[next_i], c);
            let cr = cross(d1, d2);
            nrm[0] += cr[0];
            nrm[1] += cr[1];
            nrm[2] += cr[2];
        }
        if norm(nrm) < 1e-12 {
            continue;
        }
        nrm = normalize(nrm);
        let mut u = cross(
            nrm,
            if nrm[0].abs() < 0.9 {
                [1.0, 0.0, 0.0]
            } else {
                [0.0, 1.0, 0.0]
            },
        );
        u = normalize(u);
        let mut w = cross(nrm, u);
        if nrm[2].abs() > 0.9 {
            u = [1.0, 0.0, 0.0];
            w = [0.0, nrm[2].signum(), 0.0];
        }

        let mut xy = Vec::with_capacity(m);
        let mut lo = [f64::INFINITY, f64::INFINITY];
        let mut hi = [f64::NEG_INFINITY, f64::NEG_INFINITY];
        for pt in &q {
            let diff = sub(*pt, c);
            let px = dot(diff, u);
            let py = dot(diff, w);
            lo[0] = lo[0].min(px);
            lo[1] = lo[1].min(py);
            hi[0] = hi[0].max(px);
            hi[1] = hi[1].max(py);
            xy.push([px, py]);
        }

        let mut gx = Vec::new();
        let mut x_val = lo[0] + spacing / 2.0;
        while x_val < hi[0] {
            gx.push(x_val);
            x_val += spacing;
        }
        let mut gy = Vec::new();
        let mut y_val = lo[1] + spacing / 2.0;
        while y_val < hi[1] {
            gy.push(y_val);
            y_val += spacing;
        }

        let mut grid_pts = Vec::new();
        for &px in &gx {
            for &py in &gy {
                grid_pts.push([px, py]);
            }
        }

        if jitter > 0.0 && !grid_pts.is_empty() {
            for g in &mut grid_pts {
                let seed_pos = [g[0] as f32, g[1] as f32, k_prim as f32];
                let s = position_seeds(&seed_pos)[0];
                let rx = random01(s, seed, 0);
                let ry = random01(s, seed, 1);
                g[0] += (rx - 0.5) * spacing * jitter;
                g[1] += (ry - 0.5) * spacing * jitter;
            }
        }

        for g in grid_pts {
            let mut inside = false;
            for e in 0..m {
                let prev_e = if e == 0 { m - 1 } else { e - 1 };
                let (a, b) = (xy[prev_e], xy[e]);
                let cross_edge = (a[1] > g[1]) != (b[1] > g[1]);
                let denom = if b[1] != a[1] { b[1] - a[1] } else { 1.0 };
                let x_hit = a[0] + (g[1] - a[1]) * (b[0] - a[0]) / denom;
                if cross_edge && (g[0] < x_hit) {
                    inside = !inside;
                }
            }
            if inside {
                all_out.push([
                    c[0] + g[0] * u[0] + g[1] * w[0],
                    c[1] + g[0] * u[1] + g[1] * w[1],
                    c[2] + g[0] * u[2] + g[1] * w[2],
                ]);
            }
        }
    }

    let mut p_out = Vec::with_capacity(all_out.len() * 3);
    for pt in &all_out {
        p_out.extend_from_slice(&[pt[0] as f32, pt[1] as f32, pt[2] as f32]);
    }
    new_points_cloud(p_out, None, extents, None, None)
}

// 15. pcg_spawner
pub const PCG_SPAWNER: NodeSpec = NodeSpec {
    name: "pcg_spawner",
    inputs: &[InputSpec {
        name: "points",
        multi: false,
        optional: false,
    }],
    params: crate::params!(
        "objects": "";
        "seed": 0
    ),
    cook: pcg_spawner,
};

fn pcg_spawner(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let points = PCG_SPAWNER.one(inputs, 0)?;
    let objects_str = p.s("objects")?;
    let seed = p.i("seed")?;

    let mut entries = Vec::new();
    for tok in objects_str.replace(',', " ").split_whitespace() {
        let (name, weight_str) = match tok.split_once(':') {
            Some((n, w)) => (n, w),
            None => (tok, "1.0"),
        };
        let weight: f64 = weight_str.parse().map_err(|_| {
            Error::new(format!("weight of {name:?} must be a number, e.g. {name}:2"))
        })?;
        if !name.is_empty() && weight > 0.0 {
            entries.push((name.to_string(), weight));
        }
    }
    if entries.is_empty() {
        return Err(Error::new("list the Blender objects to spawn, e.g. 'Tree:3 Rock:1'"));
    }

    let a = read_pcg(&points);
    let n = points.npoints;
    let total_w: f64 = entries.iter().map(|e| e.1).sum();
    let mut cdf = Vec::with_capacity(entries.len());
    let mut acc = 0.0;
    for e in &entries {
        acc += e.1;
        cdf.push(acc / total_w);
    }

    let mut instance = Vec::with_capacity(n);
    let mut variant = Vec::with_capacity(n);
    for i in 0..n {
        let r = random01(a.seed[i], seed, 0);
        let pick = match cdf.binary_search_by(|v| {
            v.partial_cmp(&r).unwrap_or(std::cmp::Ordering::Equal)
        }) {
            Ok(idx) => (idx + 1).min(entries.len() - 1),
            Err(idx) => idx.min(entries.len() - 1),
        };
        instance.push((pick + 1) as i32);
        variant.push(pick as i32);
    }

    let mut attrs = HashMap::new();
    attrs.insert(
        "rot".into(),
        NewAttrVal::F32(
            3,
            a.rot
                .iter()
                .flat_map(|r| [r[0] as f32, r[1] as f32, r[2] as f32])
                .collect(),
        ),
    );
    attrs.insert(
        "scale".into(),
        NewAttrVal::F32(
            3,
            a.scale
                .iter()
                .flat_map(|s| [s[0] as f32, s[1] as f32, s[2] as f32])
                .collect(),
        ),
    );
    attrs.insert("instance".into(), NewAttrVal::I32(1, instance));
    attrs.insert("variant".into(), NewAttrVal::I32(1, variant));

    let mut out = cloud(&points, None, Some(attrs))?;
    let names_json: Vec<serde_json::Value> = entries
        .iter()
        .map(|e| serde_json::json!(e.0))
        .collect();
    out.detail
        .insert("instance_objects".into(), serde_json::Value::Array(names_json));
    out.detail.remove("instance_bounds");
    Ok(out)
}

// 16. pcg_volume_sampler
pub const PCG_VOLUME_SAMPLER: NodeSpec = NodeSpec {
    name: "pcg_volume_sampler",
    inputs: &[InputSpec {
        name: "volume",
        multi: false,
        optional: false,
    }],
    params: crate::params!(
        "voxel_size": [1.0, 1.0, 1.0];
        "jitter": 0.0;
        "seed": 0
    ),
    cook: pcg_volume_sampler,
};

fn pcg_volume_sampler(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let volume = PCG_VOLUME_SAMPLER.one(inputs, 0)?;
    let voxel_size = p.v3("voxel_size")?;
    let jitter = p.f("jitter")?;
    let seed = p.i("seed")?;

    if volume.topology.has_open() {
        let n = volume.topology.closed_mask().iter().filter(|&&c| !c).count();
        return Err(Error::new(format!(
            "pcg_volume_sampler works on polygons, but the input has {n} open curve(s); remove them first, e.g. blast with group '@closed == 0'"
        )));
    }
    if volume.nprims() == 0 {
        return new_points_cloud(Vec::new(), None, [0.5, 0.5, 0.5], None, None);
    }

    let vp = volume.p();
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for c in vp.chunks_exact(3) {
        for k in 0..3 {
            lo[k] = lo[k].min(c[k] as f64);
            hi[k] = hi[k].max(c[k] as f64);
        }
    }

    let v = voxel_size;
    let dims = [
        ((hi[0] - lo[0]) / v[0]).floor().max(1.0) as usize,
        ((hi[1] - lo[1]) / v[1]).floor().max(1.0) as usize,
        ((hi[2] - lo[2]) / v[2]).floor().max(1.0) as usize,
    ];

    if dims[0] * dims[1] * dims[2] > 5_000_000 {
        return Err(Error::new(format!(
            "{} x {} x {} voxels is too many; use a larger voxel size",
            dims[0], dims[1], dims[2]
        )));
    }

    let start = [
        lo[0] + ((hi[0] - lo[0]) - dims[0] as f64 * v[0]) / 2.0 + v[0] / 2.0,
        lo[1] + ((hi[1] - lo[1]) - dims[1] as f64 * v[1]) / 2.0 + v[1] / 2.0,
        lo[2] + ((hi[2] - lo[2]) - dims[2] as f64 * v[2]) / 2.0 + v[2] / 2.0,
    ];

    let mut grid_pts = Vec::new();
    for ix in 0..dims[0] {
        let x = start[0] + ix as f64 * v[0];
        for iy in 0..dims[1] {
            let y = start[1] + iy as f64 * v[1];
            for iz in 0..dims[2] {
                let z = start[2] + iz as f64 * v[2];
                grid_pts.push([x, y, z]);
            }
        }
    }

    let tris = fan_triangles(&volume);
    let mut scale = 0.0f64;
    for k in 0..3 {
        scale = scale.max(hi[k] - lo[k]);
    }
    let mut inside_pts = Vec::new();
    for pt in grid_pts {
        if point_inside_mesh(pt, &tris, scale) {
            inside_pts.push(pt);
        }
    }

    if jitter > 0.0 && !inside_pts.is_empty() {
        for pt in &mut inside_pts {
            let pt_f32 = [pt[0] as f32, pt[1] as f32, pt[2] as f32];
            let s = position_seeds(&pt_f32)[0];
            for k in 0..3 {
                let r = random01(s, seed, k);
                pt[k] += (r - 0.5) * v[k] * jitter;
            }
        }
    }

    let mut p_out = Vec::with_capacity(inside_pts.len() * 3);
    for pt in inside_pts {
        p_out.extend_from_slice(&[pt[0] as f32, pt[1] as f32, pt[2] as f32]);
    }
    new_points_cloud(
        p_out,
        None,
        [v[0] / 2.0, v[1] / 2.0, v[2] / 2.0],
        None,
        None,
    )
}

// 17. pcg_debug
pub const PCG_DEBUG: NodeSpec = NodeSpec {
    name: "pcg_debug",
    inputs: &[InputSpec {
        name: "points",
        multi: false,
        optional: false,
    }],
    params: crate::params!("color_by_density": true),
    cook: pcg_debug,
};

fn pcg_debug(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let points = PCG_DEBUG.one(inputs, 0)?;
    let color_by_density = p.b("color_by_density")?;

    let a = read_pcg(&points);
    let n = points.npoints;

    // Unit cube corners (-1..1): 8 points
    let unit_corners: [[f64; 3]; 8] = [
        [-1.0, -1.0, -1.0],
        [1.0, -1.0, -1.0],
        [1.0, 1.0, -1.0],
        [-1.0, 1.0, -1.0],
        [-1.0, -1.0, 1.0],
        [1.0, -1.0, 1.0],
        [1.0, 1.0, 1.0],
        [-1.0, 1.0, 1.0],
    ];
    // 6 quads
    let unit_vp: [i32; 24] = [
        0, 3, 2, 1, // bottom
        4, 5, 6, 7, // top
        0, 1, 5, 4, // front
        2, 3, 7, 6, // back
        0, 4, 7, 3, // left
        1, 2, 6, 5, // right
    ];

    let boxes = compute_boxes(&a);
    let mut p_out = Vec::with_capacity(n * 8 * 3);
    let mut vp_out = Vec::with_capacity(n * 24);
    let mut offs_out = Vec::with_capacity(n * 6 + 1);
    offs_out.push(0);

    for i in 0..n {
        let base_pt = (i * 8) as i32;
        for c in &unit_corners {
            let pt = [
                boxes.c[i][0]
                    + boxes.a[i][0][0] * (c[0] * boxes.h[i][0])
                    + boxes.a[i][0][1] * (c[1] * boxes.h[i][1])
                    + boxes.a[i][0][2] * (c[2] * boxes.h[i][2]),
                boxes.c[i][1]
                    + boxes.a[i][1][0] * (c[0] * boxes.h[i][0])
                    + boxes.a[i][1][1] * (c[1] * boxes.h[i][1])
                    + boxes.a[i][1][2] * (c[2] * boxes.h[i][2]),
                boxes.c[i][2]
                    + boxes.a[i][2][0] * (c[0] * boxes.h[i][0])
                    + boxes.a[i][2][1] * (c[1] * boxes.h[i][1])
                    + boxes.a[i][2][2] * (c[2] * boxes.h[i][2]),
            ];
            p_out.extend_from_slice(&[pt[0] as f32, pt[1] as f32, pt[2] as f32]);
        }
        for face in 0..6 {
            for v_idx in 0..4 {
                vp_out.push(base_pt + unit_vp[face * 4 + v_idx]);
            }
            offs_out.push(vp_out.len() as i32);
        }
    }

    let mut out = Geometry::from_arrays(p_out, vp_out, offs_out, None)?;
    if color_by_density {
        let mut cd_out = Vec::with_capacity(n * 8 * 3);
        for i in 0..n {
            let d = a.density[i] as f32;
            let col = [
                0.4 * (1.0 - d) + d,
                0.02 * (1.0 - d) + d,
                0.02 * (1.0 - d) + d,
            ];
            for _ in 0..8 {
                cd_out.extend_from_slice(&col);
            }
        }
        out.point.insert("Cd".into(), Arc::new(Attr::f32(3, cd_out)));
    }
    let mut d_rep = Vec::with_capacity(n * 8);
    for i in 0..n {
        let d = a.density[i] as f32;
        d_rep.extend(vec![d; 8]);
    }
    out.point
        .insert("density".into(), Arc::new(Attr::f32(1, d_rep)));
    Ok(out)
}

// 18. pcg_surface_sampler
pub const PCG_SURFACE_SAMPLER: NodeSpec = NodeSpec {
    name: "pcg_surface_sampler",
    inputs: &[InputSpec {
        name: "surface",
        multi: false,
        optional: false,
    }],
    params: crate::params!(
        "points_per_m2": 1.0;
        "extents": [0.5, 0.5, 0.5];
        "looseness": 0.3;
        "align": false;
        "seed": 0
    ),
    cook: pcg_surface_sampler,
};

fn pairs_within(p: &[[f64; 3]], r: f64) -> (Vec<usize>, Vec<usize>) {
    let n = p.len();
    if n < 2 || r <= 0.0 {
        return (Vec::new(), Vec::new());
    }
    let r2 = r * r;
    let mut lo = p[0];
    for pt in p.iter().skip(1) {
        lo[0] = lo[0].min(pt[0]);
        lo[1] = lo[1].min(pt[1]);
        lo[2] = lo[2].min(pt[2]);
    }
    let inv_r = 1.0 / r;
    let mut cells: HashMap<(i64, i64, i64), Vec<usize>> = HashMap::new();
    for (i, pt) in p.iter().enumerate() {
        let cx = ((pt[0] - lo[0]) * inv_r).floor() as i64;
        let cy = ((pt[1] - lo[1]) * inv_r).floor() as i64;
        let cz = ((pt[2] - lo[2]) * inv_r).floor() as i64;
        cells.entry((cx, cy, cz)).or_default().push(i);
    }
    let mut i_pairs = Vec::new();
    let mut j_pairs = Vec::new();
    let forward_offsets: [(i64, i64, i64); 13] = [
        (0, 0, 1),
        (0, 1, -1),
        (0, 1, 0),
        (0, 1, 1),
        (1, -1, -1),
        (1, -1, 0),
        (1, -1, 1),
        (1, 0, -1),
        (1, 0, 0),
        (1, 0, 1),
        (1, 1, -1),
        (1, 1, 0),
        (1, 1, 1),
    ];
    for (&(cx, cy, cz), pts) in &cells {
        for a in 0..pts.len() {
            let i = pts[a];
            for b in (a + 1)..pts.len() {
                let j = pts[b];
                let (p1, p2) = (p[i], p[j]);
                let dx = p1[0] - p2[0];
                let dy = p1[1] - p2[1];
                let dz = p1[2] - p2[2];
                if dx * dx + dy * dy + dz * dz < r2 {
                    i_pairs.push(i);
                    j_pairs.push(j);
                }
            }
        }
        for &(dx, dy, dz) in &forward_offsets {
            if let Some(other_pts) = cells.get(&(cx + dx, cy + dy, cz + dz)) {
                for &i in pts {
                    for &j in other_pts {
                        let (p1, p2) = (p[i], p[j]);
                        let ddx = p1[0] - p2[0];
                        let ddy = p1[1] - p2[1];
                        let ddz = p1[2] - p2[2];
                        if ddx * ddx + ddy * ddy + ddz * ddz < r2 {
                            i_pairs.push(i);
                            j_pairs.push(j);
                        }
                    }
                }
            }
        }
    }
    (i_pairs, j_pairs)
}

fn pcg_surface_sampler(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let surface = PCG_SURFACE_SAMPLER.one(inputs, 0)?;
    let points_per_m2 = p.f("points_per_m2")?;
    let extents = p.v3("extents")?;
    let looseness = p.f("looseness")?;
    let align = p.b("align")?;
    let seed = p.i("seed")?;

    let topo = &surface.topology;
    let p_slice = surface.p();
    let nprims = surface.nprims();
    let vp = &topo.vertex_point;
    let offs = &topo.prim_offsets;

    let get_p = |pt: usize| -> [f64; 3] {
        [
            p_slice[pt * 3] as f64,
            p_slice[pt * 3 + 1] as f64,
            p_slice[pt * 3 + 2] as f64,
        ]
    };

    let mut area = 0.0f64;
    for prim in 0..nprims {
        let start = offs[prim] as usize;
        let end = offs[prim + 1] as usize;
        let k = end.saturating_sub(start);
        if k < 3 {
            continue;
        }
        let v0 = vp[start] as usize;
        let a = get_p(v0);
        for j in 0..(k - 2) {
            let v1 = vp[start + j + 1] as usize;
            let v2 = vp[start + j + 2] as usize;
            let b = get_p(v1);
            let c = get_p(v2);
            let cr = cross(
                [b[0] - a[0], b[1] - a[1], b[2] - a[2]],
                [c[0] - a[0], c[1] - a[1], c[2] - a[2]],
            );
            area += 0.5 * dot(cr, cr).sqrt();
        }
    }

    let target = (area * points_per_m2).round() as i64;
    if target <= 0 {
        return new_points_cloud(Vec::new(), None, extents, Some(Vec::new()), None);
    }
    let target = target as usize;
    let over = if looseness >= 1.0 { 1 } else { 6 };
    let pts = super::scatter::scatter_surface(&surface, target * over, seed as i64)?;

    let mut x: Vec<[f64; 3]> = pts
        .p()
        .chunks_exact(3)
        .map(|c| [c[0] as f64, c[1] as f64, c[2] as f64])
        .collect();
    let mut n_vec: Vec<[f64; 3]> = if let Some(a) = pts.point.get("N") {
        if let AttrData::F32(v) = &a.data {
            v.chunks_exact(3)
                .map(|c| [c[0] as f64, c[1] as f64, c[2] as f64])
                .collect()
        } else {
            vec![[0.0, 0.0, 1.0]; pts.npoints]
        }
    } else {
        vec![[0.0, 0.0, 1.0]; pts.npoints]
    };

    if over > 1 {
        let r = (1.0 - looseness) * 0.75 / points_per_m2.sqrt();
        let (i_pairs, j_pairs) = if r > 0.0 {
            pairs_within(&x, r)
        } else {
            (Vec::new(), Vec::new())
        };
        let pri: Vec<f64> = (0..x.len()).map(|i| random01(i as i32, seed, 1)).collect();
        let keep = greedy_independent(x.len(), &i_pairs, &j_pairs, &pri);
        let mut idx: Vec<usize> = (0..x.len()).filter(|&i| keep[i]).collect();
        idx.sort_by(|&a, &b| {
            pri[b]
                .partial_cmp(&pri[a])
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        idx.truncate(target);
        x = idx.iter().map(|&i| x[i]).collect();
        n_vec = idx.iter().map(|&i| n_vec[i]).collect();
    }

    let rot = if align {
        let mut r = Vec::with_capacity(n_vec.len() * 3);
        for &norm_v in &n_vec {
            let u = normalize(norm_v);
            let rot_mat = rotation_from_z_to(u);
            let euler = matrix_to_euler_xyz(&rot_mat);
            r.extend([euler[0] as f32, euler[1] as f32, euler[2] as f32]);
        }
        Some(r)
    } else {
        None
    };

    let p_flat: Vec<f32> = x
        .iter()
        .flat_map(|v| [v[0] as f32, v[1] as f32, v[2] as f32])
        .collect();
    let n_flat: Vec<f32> = n_vec
        .iter()
        .flat_map(|v| [v[0] as f32, v[1] as f32, v[2] as f32])
        .collect();
    new_points_cloud(p_flat, rot, extents, Some(n_flat), None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_points() -> Geometry {
        let p = vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 2.0, 0.0, 0.0];
        Geometry::points_only(p).unwrap()
    }

    #[test]
    fn test_pcg_spawner_and_debug() {
        let pts = sample_points();
        let p_spawn = Params::resolve(
            PCG_SPAWNER.params,
            serde_json::json!({"objects": "Tree:3 Rock:1"}).as_object(),
        )
        .unwrap();
        let s = pcg_spawner(&[vec![Arc::new(pts.clone())]], &p_spawn).unwrap();
        assert!(s.point.contains_key("instance"));
        assert!(s.point.contains_key("variant"));
        assert!(s.detail.contains_key("instance_objects"));

        let p_dbg = Params::resolve(PCG_DEBUG.params, None).unwrap();
        let dbg = pcg_debug(&[vec![Arc::new(pts)]], &p_dbg).unwrap();
        assert_eq!(dbg.npoints, 3 * 8);
        assert_eq!(dbg.nprims(), 3 * 6);
    }

    #[test]
    fn test_pcg_surface_sampler() {
        let p = vec![-10.0, -10.0, 0.0, 10.0, -10.0, 0.0, 10.0, 10.0, 0.0, -10.0, 10.0, 0.0];
        let vp = vec![0, 1, 2, 3];
        let offs = vec![0, 4];
        let quad = Geometry::from_arrays(p, vp, offs, None).unwrap();
        let params = Params::resolve(
            PCG_SURFACE_SAMPLER.params,
            serde_json::json!({
                "points_per_m2": 1.0,
                "extents": [0.8, 0.8, 1.0],
                "looseness": 0.0,
                "seed": 0
            }).as_object(),
        ).unwrap();
        let pts = pcg_surface_sampler(&[vec![Arc::new(quad)]], &params).unwrap();
        assert!((pts.npoints as i32 - 400).abs() <= 40);
        for name in ["rot", "scale", "bounds_min", "bounds_max", "density", "seed", "N"] {
            assert!(pts.point.contains_key(name));
        }
    }
}
