//! `transform` (`minihou/nodes/transform.py`), `normal` (`normal.py`) and `noise` (`noise.py`).

use std::sync::Arc;
use std::cmp::{Ordering, Reverse};
use std::collections::BinaryHeap;

use rayon::prelude::*;

use super::{Inputs, NodeSpec, GEO};
use crate::error::{Error, Result};
use crate::geometry::{Attr, Geometry};
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
    Ok(distance.into_iter().zip(selected).enumerate().map(|(i, (d, &sel))| {
        if sel { return 1.0; }
        if d >= radius { return 0.0; }
        let x = (1.0 - d/radius).clamp(0.0, 1.0);
        match falloff.as_str() {
            "smooth" => x*x*(3.0-2.0*x), "sphere" => (2.0*x-x*x).sqrt(), "root" => x.sqrt(),
            "inverse_square" => x*x/(2.0-x*x), "sharp" => x*x, "constant" => 1.0,
            "random" => x * ((i as u64).wrapping_mul(0x9e3779b97f4a7c15).rotate_left(17) as f64 / u64::MAX as f64),
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
