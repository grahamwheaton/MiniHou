//! `transform` (`minihou/nodes/transform.py`), `normal` (`normal.py`) and `noise` (`noise.py`).

use std::sync::Arc;
use std::cmp::{Ordering, Reverse};
use std::collections::BinaryHeap;

use rayon::prelude::*;

use super::{InputSpec, Inputs, NodeSpec, GEO};
use crate::error::{Error, Result};
use crate::geometry::{Attr, AttrData, AttrTable, Geometry, Topology};
use crate::params::Params;
use crate::{math, noise, normals, topo, xform};
use super::groups;

pub const TRANSFORM: NodeSpec = NodeSpec {
    name: "transform",
    inputs: GEO,
    params: crate::params!(
        "translate": [0.0, 0.0, 0.0]; "rotate": [0.0, 0.0, 0.0]; "scale": [1.0, 1.0, 1.0]; "uniform_scale": 1.0;
        "pivot": [0.0, 0.0, 0.0]; "axis_rotation": [0.0, 0.0, 0.0];
        "group": "";
        "proportional_enabled": false;
        "proportional_radius": 1.0;
        "proportional_falloff": "smooth";
        "proportional_connected": false;
        "proportional_projected": false;
        "proportional_view_direction": [0.0, 0.0, 1.0];
    ),
    cook: transform,
};

fn mat_f32(m: &math::M3) -> [[f32; 3]; 3] {
    m.map(|row| row.map(|x| x as f32))
}

fn transform(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = TRANSFORM.one(inputs, 0)?;
    let selected = groups::point_mask(&geo, &p.s("group")?)?;
    if p.b("proportional_enabled")? {
        return proportional_transform(&geo, p, &selected);
    }
    let us = p.f("uniform_scale")?;
    let s = p.v3("scale")?.map(|x| x * us);
    let r = math::euler_xyz(p.v3("rotate")?);
    let b = math::euler_xyz(p.v3("axis_rotation")?);
    let pivot = p.v3("pivot")?;
    let translate = p.v3("translate")?;
    let scale_cols = |m: &math::M3, f: &dyn Fn(f64) -> f64| -> math::M3 {
        let mut o = *m;
        for row in o.iter_mut() {
            for (j, x) in row.iter_mut().enumerate() {
                *x *= f(s[j]);
            }
        }
        o
    };
    // M = B (R * s) B^T: scale, then rotate, in the axis_rotation frame
    let m = math::mul(&math::mul(&b, &scale_cols(&r, &|x| x)), &math::transpose(&b));
    let m32 = mat_f32(&m);
    let bt = math::apply(&b, translate);
    let shift: [f32; 3] = [0, 1, 2].map(|k| (pivot[k] + bt[k]) as f32);
    let pv: [f32; 3] = pivot.map(|x| x as f32);
    let mut out = (*geo).clone();
    let mut positions = xform::affine(geo.p(), m32, pv, shift);
    for (i, &use_point) in selected.iter().enumerate() {
        if !use_point { positions[3*i..3*i+3].copy_from_slice(&geo.p()[3*i..3*i+3]); }
    }
    out.set_p(positions);
    super::packed::transform_instances(&mut out, &m, &selected);
    if let Some(n) = geo.point.get("N") {
        let n = n.as_f32().filter(|_| n.width == 3).ok_or_else(|| Error::new("point attribute N must be 3 floats"))?;
        // normals take the inverse scale: B (R / s) B^T, a zero scale counting as 1
        let nm = math::mul(&math::mul(&b, &scale_cols(&r, &|x| 1.0 / if x.abs() < 1e-12 { 1.0 } else { x })), &math::transpose(&b));
        let mut nn = xform::linear_normalized(n, mat_f32(&nm));
        for (i, &use_point) in selected.iter().enumerate() {
            if !use_point { nn[3*i..3*i+3].copy_from_slice(&n[3*i..3*i+3]); }
        }
        out.point.insert("N".into(), Arc::new(Attr::f32(3, nn)));
    }
    Ok(out)
}

#[derive(Copy, Clone, PartialEq)]
struct Distance(f64, usize);
impl Eq for Distance {}
impl PartialOrd for Distance { fn partial_cmp(&self, other: &Self) -> Option<Ordering> { Some(self.cmp(other)) } }
impl Ord for Distance { fn cmp(&self, other: &Self) -> Ordering { self.0.total_cmp(&other.0).then(self.1.cmp(&other.1)) } }

fn proportional_weights(geo: &Geometry, selected: &[bool], p: &Params) -> Result<Vec<f64>> {
    let radius = p.f("proportional_radius")?;
    if !radius.is_finite() || radius <= 0.0 { return Err(Error::new("proportional radius must be positive and finite")); }
    let falloff = p.e("proportional_falloff", &["smooth", "sphere", "root", "inverse_square", "sharp", "linear", "constant", "random"])?;
    if falloff == "random" { return Err(Error::new("random proportional falloff is not supported by the Rust backend")); }
    let pos: Vec<[f64; 3]> = geo.p().as_chunks::<3>().0.iter().map(|q| [q[0] as f64, q[1] as f64, q[2] as f64]).collect();
    if pos.iter().flatten().any(|x| !x.is_finite()) { return Err(Error::new("proportional editing requires finite positions")); }
    let connected = p.b("proportional_connected")?;
    let projected = p.b("proportional_projected")? && !connected;
    let mut metric = pos.clone();
    if projected {
        let mut view = p.v3("proportional_view_direction")?;
        let len = view.iter().map(|v| v*v).sum::<f64>().sqrt();
        if !len.is_finite() || len < 1e-12 { return Err(Error::new("projected proportional editing requires a nonzero view direction")); }
        for v in &mut view { *v /= len; }
        for q in &mut metric {
            let dot: f64 = (0..3).map(|k| q[k]*view[k]).sum();
            for k in 0..3 { q[k] -= dot*view[k]; }
        }
    }
    let mut distance: Vec<f64> = selected.iter().map(|&b| if b { 0.0 } else { f64::INFINITY }).collect();
    if connected {
        let mut adj = vec![Vec::<(usize, f64)>::new(); geo.npoints];
        let edges = topo::edges(&geo.topology, geo.npoints);
        for pair in edges.points.as_chunks::<2>().0 {
            let (a,b) = (pair[0] as usize, pair[1] as usize);
            let len = (0..3).map(|k| (pos[a][k]-pos[b][k]).powi(2)).sum::<f64>().sqrt();
            adj[a].push((b, len)); adj[b].push((a, len));
        }
        let mut queue = BinaryHeap::new();
        for (i, &b) in selected.iter().enumerate() { if b { queue.push(Reverse(Distance(0.0, i))); } }
        while let Some(Reverse(Distance(d, i))) = queue.pop() {
            if d != distance[i] || d >= radius { continue; }
            for &(j, len) in &adj[i] {
                let candidate = d + len;
                if candidate < radius && candidate < distance[j] { distance[j] = candidate; queue.push(Reverse(Distance(candidate, j))); }
            }
        }
    } else {
        let seeds: Vec<_> = metric.iter().zip(selected).filter_map(|(q, &b)| b.then_some(q)).collect();
        if !seeds.is_empty() {
            for (i, q) in metric.iter().enumerate() {
                distance[i] = seeds.iter().map(|seed| (0..3).map(|k| (q[k]-seed[k]).powi(2)).sum::<f64>()).fold(f64::INFINITY, f64::min).sqrt();
            }
        }
    }
    Ok(distance.into_iter().zip(selected).map(|(d, &sel)| {
        if sel { return 1.0; }
        if d >= radius { return 0.0; }
        let x = (1.0 - d/radius).clamp(0.0, 1.0);
        match falloff.as_str() {
            "smooth" => x*x*(3.0-2.0*x), "sphere" => (2.0*x-x*x).sqrt(), "root" => x.sqrt(),
            "inverse_square" => x*x/(2.0-x*x), "sharp" => x*x, "constant" => 1.0,
            _ => x,
        }
    }).collect())
}

fn proportional_transform(geo: &Geometry, p: &Params, selected: &[bool]) -> Result<Geometry> {
    let weights = proportional_weights(geo, selected, p)?;
    let b = math::euler_xyz(p.v3("axis_rotation")?);
    let bt = math::transpose(&b);
    let translate = math::apply(&b, p.v3("translate")?);
    let pivot = p.v3("pivot")?;
    let angles = p.v3("rotate")?;
    let uniform_scale = p.f("uniform_scale")?;
    let scale = p.v3("scale")?.map(|x| x * uniform_scale);
    let mut out = geo.clone();
    let mut positions = geo.p().to_vec();
    for (i, &w) in weights.iter().enumerate() {
        if w == 0.0 { continue; }
        let q = [0,1,2].map(|k| geo.p()[3*i+k] as f64 - pivot[k]);
        let local = math::apply(&bt, q);
        let local_scale = scale.map(|s| 1.0 + w*(s-1.0));
        let scaled = [0,1,2].map(|k| local[k]*local_scale[k]);
        let rotated = math::apply(&math::euler_xyz(angles.map(|a| a*w)), scaled);
        let world = math::apply(&b, rotated);
        for k in 0..3 { positions[3*i+k] = (world[k] + pivot[k] + w*translate[k]) as f32; }
    }
    out.set_p(positions);
    if let Some(n) = geo.point.get("N") {
        let normal = n.as_f32().filter(|_| n.width == 3).ok_or_else(|| Error::new("point attribute N must be 3 floats"))?;
        let mut result = normal.to_vec();
        for (i, &w) in weights.iter().enumerate() {
            if w == 0.0 { continue; }
            let local = math::apply(&bt, [0,1,2].map(|k| normal[3*i+k] as f64));
            let divided = [0,1,2].map(|k| {
                let s = 1.0 + w*(scale[k]-1.0);
                local[k] / if s.abs() < 1e-12 { 1.0 } else { s }
            });
            let rotated = math::apply(&math::euler_xyz(angles.map(|a| a*w)), divided);
            let world = math::apply(&b, rotated);
            let len = world.iter().map(|x| x*x).sum::<f64>().sqrt();
            for k in 0..3 { result[3*i+k] = (world[k] / if len > 1e-12 { len } else { 1.0 }) as f32; }
        }
        out.point.insert("N".into(), Arc::new(Attr::f32(3, result)));
    }
    Ok(out)
}

pub const NORMAL: NodeSpec = NodeSpec {
    name: "normal",
    inputs: GEO,
    params: crate::params!("domain": "point"; "cusp_angle": 60.0;),
    cook: normal,
};

fn normal(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = NORMAL.one(inputs, 0)?;
    let domain = p.e("domain", &["point", "prim", "vertex"])?;
    let mut out = (*geo).clone();
    match domain.as_str() {
        "point" => {
            let n = normals::point_normals(geo.p(), &geo.topology, geo.npoints);
            out.point.insert("N".into(), Arc::new(Attr::f32(3, n)));
        }
        "prim" => {
            let n = normals::normalize_to_f32(&normals::prim_normals(geo.p(), &geo.topology));
            out.prim.insert("N".into(), Arc::new(Attr::f32(3, n)));
        }
        _ => return Err(Error::new("domain 'vertex' (cusped vertex normals) is not supported by the Rust backend yet")),
    }
    Ok(out)
}

pub const NOISE: NodeSpec = NodeSpec {
    name: "noise",
    inputs: GEO,
    params: crate::params!(
        "attrib": "P"; "mode": "scalar"; "amplitude": [0.0, 0.0, 1.0]; "frequency": 1.0; "offset": [0.0, 0.0, 0.0];
        "octaves": 1; "lacunarity": 2.0; "gain": 0.5; "seed": 0;
        "group": "", unsupported;
    ),
    cook: noise_node,
};

fn noise_node(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = NOISE.one(inputs, 0)?;
    let attrib = p.s("attrib")?;
    if attrib.is_empty() {
        return Err(Error::new("'attrib' must not be empty"));
    }
    let mode = p.e("mode", &["scalar", "vector"])?;
    let amp = p.v3("amplitude")?.map(|x| x as f32);
    let freq = p.f("frequency")? as f32;
    let offset = p.v3("offset")?.map(|x| x as f32);
    let octaves = p.i("octaves")?;
    if !(1..=12).contains(&octaves) {
        return Err(Error::new("octaves must be between 1 and 12"));
    }
    let (lac, gain, seed) = (p.f("lacunarity")?, p.f("gain")?, p.i("seed")?);
    let src = geo.p();
    let mut out = (*geo).clone();
    if mode == "scalar" && attrib != "P" {
        let pos: Vec<f32> = src.par_chunks_exact(3).flat_map_iter(|q| [0, 1, 2].map(|k| q[k] * freq + offset[k])).collect();
        let v = noise::fbm3(&pos, seed, octaves as u32, lac, gain);
        out.point.insert(attrib, Arc::new(Attr::f32(1, v.iter().map(|x| x * amp[0]).collect())));
        return Ok(out);
    }
    let add = attrib == "P";
    let v = noise::noise_vec(src, freq, offset, amp, seed, octaves as u32, lac, gain, mode == "vector", add);
    if add {
        out.set_p(v);
    } else {
        out.point.insert(attrib, Arc::new(Attr::f32(3, v)));
    }
    Ok(out)
}

pub const MATCH_SIZE: NodeSpec = NodeSpec {
    name: "match_size",
    inputs: &[
        InputSpec { name: "geo", multi: false, optional: false },
        InputSpec { name: "target", multi: false, optional: true },
    ],
    params: crate::params!(
        "translate": true; "center": [0.0, 0.0, 0.0]; "justify_z": "center";
        "scale": false; "size": [1.0, 1.0, 1.0]; "uniform": true;
    ),
    cook: match_size,
};

fn match_size(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = MATCH_SIZE.one(inputs, 0)?;
    if geo.npoints == 0 {
        return Ok((*geo).clone());
    }
    let target = inputs[1].first().filter(|t| t.npoints > 0);
    let normals = geo.point.get("N").and_then(|a| (a.width == 3).then(|| a.as_f32()).flatten());
    let out = match_size_points(
        geo.p(),
        target.map(|t| t.p()),
        p.b("translate")?,
        p.v3("center")?,
        &p.e("justify_z", &["center", "min", "max"])?,
        p.b("scale")?,
        p.v3("size")?,
        p.b("uniform")?,
        normals,
    );
    let mut g = (*geo).clone();
    g.set_p(out.p);
    if let Some(n) = out.n {
        g.point.insert("N".into(), Arc::new(Attr::f32(3, n)));
    }
    Ok(g)
}

/// New positions and, when the scale is not uniform, rescaled and renormalised normals
/// (`minihou/nodes/attrib_tools.py`), all in float64 like the numpy body, back to float32.
pub struct MatchSizeOut {
    pub p: Vec<f32>,
    pub n: Option<Vec<f32>>,
}

/// `match_size` on raw arrays, shared by the node cook above and the Python fast path. `target` is the second input's
/// nonempty points (its bounding box replaces `center`/`size`); `normals` the point `N` when it is xyz, else `None`.
#[allow(clippy::too_many_arguments)]
pub fn match_size_points(
    points: &[f32],
    target: Option<&[f32]>,
    translate: bool,
    center: [f64; 3],
    justify_z: &str,
    scale: bool,
    size: [f64; 3],
    uniform: bool,
    normals: Option<&[f32]>,
) -> MatchSizeOut {
    let (mut center, mut size) = (center, size);
    if let Some(t) = target {
        let (tlo, thi) = (axis_extremes(t, false), axis_extremes(t, true));
        size = [thi[0] - tlo[0], thi[1] - tlo[1], thi[2] - tlo[2]];
        center = [(tlo[0] + thi[0]) / 2.0, (tlo[1] + thi[1]) / 2.0,
                  if justify_z == "center" { (tlo[2] + thi[2]) / 2.0 } else if justify_z == "min" { tlo[2] } else { thi[2] }];
    }
    let (lo, hi) = (axis_extremes(points, false), axis_extremes(points, true));
    let mid = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0, (lo[2] + hi[2]) / 2.0];
    let mut s = [1.0f64; 3];
    if scale {
        let ext = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
        let ok = [ext[0] > 1e-12, ext[1] > 1e-12, ext[2] > 1e-12];
        for k in 0..3 {
            s[k] = if ok[k] { size[k] / ext[k] } else { 1.0 };
        }
        if uniform {
            let vals: Vec<f64> = (0..3).filter(|&k| ok[k]).map(|k| s[k]).collect();
            let u = if vals.is_empty() { 1.0 } else { numpy_min(&vals) };
            s = [u, u, u];
        }
    }
    let anchor = [mid[0], mid[1], match justify_z { "min" => lo[2], "max" => hi[2], _ => mid[2] }];
    let tgt = if translate { center } else { anchor };
    let mut p = vec![0.0f32; points.len()];
    for (i, q) in points.as_chunks::<3>().0.iter().enumerate() {
        for k in 0..3 {
            p[3 * i + k] = ((q[k] as f64 - anchor[k]) * s[k] + tgt[k]) as f32;
        }
    }
    // numpy's `np.allclose(s, s[0])`: only a real per-axis scale change rescales N
    let s0 = s[0];
    let close = (0..3).all(|k| (s[k] - s0).abs() <= 1e-8 + 1e-5 * s0.abs());
    let n = normals.and_then(|nrm| {
        if close {
            return None;
        }
        let mut result = vec![0.0f32; nrm.len()];
        for (i, row) in nrm.as_chunks::<3>().0.iter().enumerate() {
            let v = [row[0] as f64 / s[0], row[1] as f64 / s[1], row[2] as f64 / s[2]];
            let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
            let len = if len < 1e-20 { 1.0 } else { len };
            for k in 0..3 {
                result[3 * i + k] = (v[k] / len) as f32;
            }
        }
        Some(result)
    });
    MatchSizeOut { p, n }
}

/// Per-axis minimum or maximum of flat xyz points, NaN-propagating like `np.ndarray.min(0)` / `.max(0)`.
fn axis_extremes(points: &[f32], want_max: bool) -> [f64; 3] {
    let mut out = [if want_max { f64::NEG_INFINITY } else { f64::INFINITY }; 3];
    let mut nan = [false; 3];
    for q in points.as_chunks::<3>().0 {
        for k in 0..3 {
            let v = q[k] as f64;
            if v.is_nan() {
                nan[k] = true;
            } else if (want_max && v > out[k]) || (!want_max && v < out[k]) {
                out[k] = v;
            }
        }
    }
    for k in 0..3 {
        if nan[k] {
            out[k] = f64::NAN;
        }
    }
    out
}

/// `np.min` over a list: NaN if any element is NaN, else the smallest.
fn numpy_min(v: &[f64]) -> f64 {
    let mut m = f64::INFINITY;
    for &x in v {
        if x.is_nan() {
            return f64::NAN;
        }
        if x < m {
            m = x;
        }
    }
    m
}

pub const DELETE: NodeSpec = NodeSpec {
    name: "delete",
    inputs: GEO,
    params: crate::params!("expr": "@P.z < 0"; "invert": false;),
    cook: delete_node,
};

fn delete_node(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = DELETE.one(inputs, 0)?;
    let expr = p.s("expr")?;
    let invert = p.b("invert")?;

    if geo.npoints == 0 {
        return Ok((*geo).clone());
    }

    let mut mask = delete_mask(&geo, &expr)?;
    if invert {
        for m in &mut mask {
            *m = !*m;
        }
    }

    if !mask.iter().any(|&m| m) {
        return Ok((*geo).clone());
    }

    let keep: Vec<bool> = mask.iter().map(|&m| !m).collect();
    keep_points(&geo, &keep)
}

fn delete_mask(geo: &Geometry, expr: &str) -> Result<Vec<bool>> {
    if expr.contains(';') || expr.contains('\n') {
        return Err(Error::new("the delete expression must be a single expression, e.g. @P.z < 0"));
    }
    let text = expr.trim();
    if text.is_empty() || text == "0" {
        return Ok(vec![false; geo.npoints]);
    }
    if text == "1" {
        return Ok(vec![true; geo.npoints]);
    }
    let trimmed = text.trim_start_matches('(').trim_end_matches(')').trim();
    if let Some(axis) = trimmed.strip_prefix("@P.") {
        let (k, rest) = match axis.chars().next() {
            Some('x') => (0, &axis[1..]),
            Some('y') => (1, &axis[1..]),
            Some('z') => (2, &axis[1..]),
            _ => return Err(Error::new("delete expression is not supported by the Rust backend")),
        };
        let rest = rest.trim();
        let (op, rhs) = [">=", "<=", "==", "!=", ">", "<"].into_iter()
            .find_map(|op| rest.strip_prefix(op).map(|rhs| (op, rhs.trim())))
            .ok_or_else(|| Error::new("delete expression is not supported by the Rust backend"))?;
        let rhs = rhs.trim_matches(|c| c == '(' || c == ')').trim();
        let value: f32 = rhs.parse().map_err(|_| Error::new("delete expression is not supported by the Rust backend"))?;
        return Ok(geo.p().as_chunks::<3>().0.iter().map(|q| match op {
            ">=" => q[k] >= value,
            "<=" => q[k] <= value,
            "==" => q[k] == value,
            "!=" => q[k] != value,
            ">" => q[k] > value,
            _ => q[k] < value,
        }).collect());
    }
    Err(Error::new("delete expression is not supported by the Rust backend"))
}

pub(super) fn keep_points(geo: &Geometry, keep: &[bool]) -> Result<Geometry> {
    let npoints = geo.npoints;
    let topo = &geo.topology;
    let nverts = topo.nvertices();
    let nprims = topo.nprims();
    let closed = topo.closed_mask();

    let mut remap = vec![-1i32; npoints];
    let mut new_point_count = 0;
    for (i, &k) in keep.iter().enumerate() {
        if k {
            remap[i] = new_point_count as i32;
            new_point_count += 1;
        }
    }


    let vkeep_pt: Vec<bool> = topo.vertex_point.iter().map(|&v| keep[v as usize]).collect();

    let mut dropped_per_prim = vec![0usize; nprims];
    let mut vert_prim = Vec::with_capacity(nverts);
    for (p, drop_count) in dropped_per_prim.iter_mut().enumerate() {
        let start = topo.prim_offsets[p] as usize;
        let end = topo.prim_offsets[p + 1] as usize;
        vert_prim.extend(std::iter::repeat_n(p, end - start));
        for &vk in &vkeep_pt[start..end] {
            if !vk {
                *drop_count += 1;
            }
        }
    }

    let mut vkeep = vec![false; nverts];
    for i in 0..nverts {
        let p = vert_prim[i];
        vkeep[i] = if closed[p] {
            dropped_per_prim[p] == 0
        } else {
            vkeep_pt[i]
        };
    }

    let mut first = vec![false; nverts];
    for p in 0..nprims {
        let start = topo.prim_offsets[p] as usize;
        let end = topo.prim_offsets[p + 1] as usize;
        if end > start {
            first[start] = true;
        }
    }

    let mut starts_run = vec![false; nverts];
    for i in 0..nverts {
        if vkeep[i] {
            let is_first = first[i] || i == 0 || !vkeep[i - 1];
            starts_run[i] = is_first;
        }
    }

    let mut run = vec![-1i32; nverts];
    let mut run_count = 0;
    for i in 0..nverts {
        if starts_run[i] {
            run_count += 1;
        }
        if vkeep[i] {
            run[i] = (run_count - 1) as i32;
        }
    }

    let mut run_size = vec![0i32; run_count];
    for i in 0..nverts {
        if vkeep[i] {
            run_size[run[i] as usize] += 1;
        }
    }

    let mut run_prim = Vec::with_capacity(run_count);
    for i in 0..nverts {
        if starts_run[i] {
            run_prim.push(vert_prim[i]);
        }
    }

    let mut good = vec![false; run_count];
    for r in 0..run_count {
        let p = run_prim[r];
        good[r] = if closed[p] {
            run_size[r] > 0
        } else {
            run_size[r] >= 2
        };
    }

    for i in 0..nverts {
        if vkeep[i] {
            let r = run[i] as usize;
            vkeep[i] = good[r];
        }
    }

    let new_vp: Vec<i32> = vkeep.iter()
        .zip(topo.vertex_point.iter())
        .filter_map(|(&k, &v)| k.then_some(remap[v as usize]))
        .collect();

    let mut new_offs = Vec::with_capacity(run_count + 1);
    new_offs.push(0);
    let mut acc = 0;
    for r in 0..run_count {
        if good[r] {
            acc += run_size[r];
            new_offs.push(acc);
        }
    }

    let src_prim: Vec<usize> = good.iter().enumerate().filter_map(|(r, &g)| g.then_some(run_prim[r])).collect();

    let new_closed = if topo.closed.is_none() {
        None
    } else {
        Some(src_prim.iter().map(|&p| closed[p]).collect())
    };

    Ok(Geometry { packed: Default::default(), edge: Default::default(),
        npoints: new_point_count,
        topology: Arc::new(Topology {
            vertex_point: new_vp,
            prim_offsets: new_offs,
            closed: new_closed,
        }),
        point: slice_table_mask(&geo.point, keep),
        vertex: slice_table_mask(&geo.vertex, &vkeep),
        prim: slice_table_indices(&geo.prim, &src_prim),
        detail: geo.detail.clone(),
    })
}

fn slice_table_mask(table: &AttrTable, mask: &[bool]) -> AttrTable {
    let mut out = AttrTable::new();
    for (name, attr) in table {
        let w = attr.width;
        let data = match &attr.data {
            AttrData::F32(v) => {
                let mut d = Vec::new();
                for (i, &m) in mask.iter().enumerate() {
                    if m {
                        d.extend_from_slice(&v[i * w..(i + 1) * w]);
                    }
                }
                AttrData::F32(d)
            }
            AttrData::I32(v) => {
                let mut d = Vec::new();
                for (i, &m) in mask.iter().enumerate() {
                    if m {
                        d.extend_from_slice(&v[i * w..(i + 1) * w]);
                    }
                }
                AttrData::I32(d)
            }
        };
        out.insert(name.clone(), Arc::new(Attr { width: w, data }));
    }
    out
}

fn slice_table_indices(table: &AttrTable, indices: &[usize]) -> AttrTable {
    let mut out = AttrTable::new();
    for (name, attr) in table {
        let w = attr.width;
        let data = match &attr.data {
            AttrData::F32(v) => {
                let mut d = Vec::with_capacity(indices.len() * w);
                for &idx in indices {
                    d.extend_from_slice(&v[idx * w..(idx + 1) * w]);
                }
                AttrData::F32(d)
            }
            AttrData::I32(v) => {
                let mut d = Vec::with_capacity(indices.len() * w);
                for &idx in indices {
                    d.extend_from_slice(&v[idx * w..(idx + 1) * w]);
                }
                AttrData::I32(d)
            }
        };
        out.insert(name.clone(), Arc::new(Attr { width: w, data }));
    }
    out
}

