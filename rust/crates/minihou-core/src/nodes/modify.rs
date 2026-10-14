//! `transform` (`minihou/nodes/transform.py`), `normal` (`normal.py`) and `noise` (`noise.py`).

use std::sync::Arc;

use rayon::prelude::*;

use super::{Inputs, NodeSpec, GEO};
use crate::error::{Error, Result};
use crate::geometry::{Attr, Geometry};
use crate::params::Params;
use crate::{math, noise, normals, xform};

pub const TRANSFORM: NodeSpec = NodeSpec {
    name: "transform",
    inputs: GEO,
    params: crate::params!(
        "translate": [0.0, 0.0, 0.0]; "rotate": [0.0, 0.0, 0.0]; "scale": [1.0, 1.0, 1.0]; "uniform_scale": 1.0;
        "pivot": [0.0, 0.0, 0.0]; "axis_rotation": [0.0, 0.0, 0.0];
        "group": "", unsupported;
        "proportional_enabled": false, unsupported;
        "proportional_radius": 1.0, unsupported;
        "proportional_falloff": "smooth", unsupported;
        "proportional_connected": false, unsupported;
        "proportional_projected": false, unsupported;
        "proportional_view_direction": [0.0, 0.0, 1.0], unsupported;
    ),
    cook: transform,
};

fn mat_f32(m: &math::M3) -> [[f32; 3]; 3] {
    m.map(|row| row.map(|x| x as f32))
}

fn transform(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = TRANSFORM.one(inputs, 0)?;
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
    out.set_p(xform::affine(geo.p(), m32, pv, shift));
    if let Some(n) = geo.point.get("N") {
        let n = n.as_f32().filter(|_| n.width == 3).ok_or_else(|| Error::new("point attribute N must be 3 floats"))?;
        // normals take the inverse scale: B (R / s) B^T, a zero scale counting as 1
        let nm = math::mul(&math::mul(&b, &scale_cols(&r, &|x| 1.0 / if x.abs() < 1e-12 { 1.0 } else { x })), &math::transpose(&b));
        let nn = xform::linear_normalized(n, mat_f32(&nm));
        out.point.insert("N".into(), Arc::new(Attr::f32(3, nn)));
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
