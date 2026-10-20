use std::collections::HashMap;
use std::sync::Arc;

use crate::error::Result;
use crate::geometry::{Attr, AttrData, Geometry};

use super::random::position_seeds;
use super::DEFAULT_EXTENT;

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
