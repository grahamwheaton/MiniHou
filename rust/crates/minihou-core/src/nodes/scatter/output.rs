use std::sync::Arc;
use super::{Tri, surface::matches};
use super::super::np_random::Rng;
use crate::error::{Error, Result};
use crate::geometry::{Attr, AttrData, Geometry};
use crate::params::Params;

pub(super) fn sampled_attr(attr: &Attr, samples: &[(usize, [f64; 3])], tris: &[Tri], domain: &str) -> Attr {
    let rows = |tri: &Tri| if domain == "point" { tri.points } else { tri.vertices };
    match &attr.data {
        AttrData::F32(src) => {
            let mut dst = Vec::with_capacity(samples.len() * attr.width);
            for &(ti, weights) in samples {
                let ids = rows(&tris[ti]);
                for component in 0..attr.width {
                    dst.push((0..3).map(|j| weights[j] * src[ids[j] * attr.width + component] as f64).sum::<f64>() as f32);
                }
            }
            Attr::f32(attr.width, dst)
        }
        AttrData::I32(src) => {
            let mut dst = Vec::with_capacity(samples.len() * attr.width);
            for &(ti, weights) in samples {
                let ids = rows(&tris[ti]);
                let j = if weights[1] > weights[0] && weights[1] >= weights[2] { 1 } else if weights[2] > weights[0] && weights[2] > weights[1] { 2 } else { 0 };
                dst.extend_from_slice(&src[ids[j] * attr.width..(ids[j] + 1) * attr.width]);
            }
            Attr::i32(attr.width, dst)
        }
    }
}

pub(super) fn add_output_attributes(out: &mut Geometry, geo: &Geometry, tris: &[Tri], samples: &[(usize, [f64; 3])], p: &Params, density_scale: f64, density: Option<&[f64]>, vertex_density: Option<&[f64]>, primitive_density: Option<&[f64]>) -> Result<()> {
    let n = samples.len();
    for (table, key, domain) in [(&geo.point, "point_attributes", "point"), (&geo.vertex, "vertex_attributes", "vertex")] {
        let pattern = p.s(key)?;
        for (name, attr) in table {
            if name == "P" || (name != "N" && out.point.contains_key(name)) || !matches(name, &pattern) { continue; }
            out.point.insert(name.clone(), Arc::new(sampled_attr(attr, samples, tris, domain)));
        }
    }
    let pattern = p.s("primitive_attributes")?;
    for (name, attr) in &geo.prim {
        if out.point.contains_key(name) || !matches(name, &pattern) { continue; }
        let rows: Vec<_> = samples.iter().map(|&(ti, _)| tris[ti].prim as usize).collect();
        out.point.insert(name.clone(), Arc::new(super::super::group::gather(attr, &rows)));
    }
    for (key, as_detail) in [("detail_attributes", false), ("detail_as_detail", true)] {
        let pattern = p.s(key)?;
        for (name, value) in &geo.detail {
            if !matches(name, &pattern) { continue; }
            if as_detail { out.detail.insert(name.clone(), value.clone()); continue; }
            if out.point.contains_key(name) { continue; }
            if let Some(x) = value.as_i64() { out.point.insert(name.clone(), Arc::new(Attr::i32(1, vec![x as i32; n]))); }
            else if let Some(x) = value.as_f64() { out.point.insert(name.clone(), Arc::new(Attr::f32(1, vec![x as f32; n]))); }
            else if let Some(values) = value.as_array() {
                if !values.is_empty() && values.iter().all(|v| v.is_number()) {
                    let row: Vec<f32> = values.iter().map(|v| v.as_f64().unwrap() as f32).collect();
                    out.point.insert(name.clone(), Arc::new(Attr::f32(row.len(), (0..n).flat_map(|_| row.iter().copied()).collect())));
                }
            }
        }
    }
    for (enabled, key, attr) in [
        ("prim_num_attribute", "prim_num_attribute_name", Attr::i32(1, samples.iter().map(|&(ti, _)| tris[ti].prim).collect())),
        ("prim_uvw_attribute", "prim_uvw_attribute_name", Attr::f32(3, samples.iter().flat_map(|&(_, w)| [w[1] as f32, w[2] as f32, 0.0]).collect())),
        ("output_id_attribute", "output_id_attribute_name", Attr::i32(1, (0..n as i32).collect())),
    ] {
        if p.b(enabled)? { insert_named(out, p, key, attr)?; }
    }
    let physical_output = p.s("generate")? == "in_texture_space" && !p.b("output_radius_and_density_in_texture_space")?;
    let values: Vec<f32> = samples.iter().map(|&(ti, w)| {
        let weight = density.map(|d| (0..3).map(|j| w[j] * d[tris[ti].points[j]].clamp(0.0, 1.0)).sum::<f64>())
            .or_else(|| vertex_density.map(|d| (0..3).map(|j| w[j] * d[tris[ti].vertices[j]].clamp(0.0, 1.0)).sum::<f64>()))
            .or_else(|| primitive_density.map(|d| d[tris[ti].prim as usize].max(0.0))).unwrap_or(1.0);
        let texture_factor = if physical_output {
            tris[ti].texture_area / tris[ti].physical_area.max(1e-20)
        } else { 1.0 };
        (density_scale * weight * texture_factor) as f32
    }).collect();
    if p.b("output_density_attribute")? { insert_named(out, p, "output_density_attribute_name", Attr::f32(1, values.clone()))?; }
    if p.b("output_radius_attribute")? {
        let scale = p.f("scale_radii_by")?;
        let limit = p.f("max_relax_radius")?;
        if !scale.is_finite() || scale < 0.0 || !limit.is_finite() || limit < 0.0 { return Err(Error::new("scatter: radii must be finite and non-negative")); }
        let radii = values.iter().map(|&d| {
            let r = if d > 0.0 { 0.537 / (d as f64).sqrt() * scale } else { f64::INFINITY };
            if limit > 0.0 { r.min(limit) as f32 } else { r.min(f32::MAX as f64) as f32 }
        }).collect();
        insert_named(out, p, "output_radius_attribute_name", Attr::f32(1, radii))?;
    }
    Ok(())
}

fn insert_named(out: &mut Geometry, p: &Params, name_key: &str, attr: Attr) -> Result<()> {
    let name = p.s(name_key)?;
    super::super::group::valid_name(&name)?;
    if out.point.contains_key(&name) { return Err(Error::new(format!("scatter: output attribute {name:?} already exists"))); }
    out.point.insert(name, Arc::new(attr));
    Ok(())
}

pub(super) fn add_volume_output_attributes(out: &mut Geometry, geo: &Geometry, p: &Params, seed: i64) -> Result<()> {
    let n = out.npoints;
    for (enabled, key, attr) in [
        ("prim_num_attribute", "prim_num_attribute_name", Attr::i32(1, vec![-1; n])),
        ("prim_uvw_attribute", "prim_uvw_attribute_name", Attr::f32(3, vec![0.0; 3*n])),
        ("output_id_attribute", "output_id_attribute_name", Attr::i32(1, (0..n as i32).collect())),
    ] { if p.b(enabled)? { insert_named(out, p, key, attr)?; } }
    let lo = (0..3).map(|k| geo.p().chunks_exact(3).map(|xyz| xyz[k]).fold(f32::INFINITY, f32::min)).collect::<Vec<_>>();
    let hi = (0..3).map(|k| geo.p().chunks_exact(3).map(|xyz| xyz[k]).fold(f32::NEG_INFINITY, f32::max)).collect::<Vec<_>>();
    let volume = (0..3).map(|k| (hi[k]-lo[k]).max(0.0) as f64).product::<f64>();
    let density = if volume > 0.0 { n as f64/volume } else { 0.0 };
    if p.b("output_density_attribute")? { insert_named(out, p, "output_density_attribute_name", Attr::f32(1, vec![density as f32; n]))?; }
    if p.b("output_radius_attribute")? {
        let scale = p.f("scale_radii_by")?; let max = p.f("max_relax_radius")?;
        if !scale.is_finite() || scale < 0.0 || !max.is_finite() || max < 0.0 { return Err(Error::new("scatter: radii must be finite and non-negative")); }
        let radius = if density > 0.0 { 0.561 / density.cbrt() * scale } else { 0.0 };
        let radius = if max > 0.0 { radius.min(max) } else { radius };
        insert_named(out, p, "output_radius_attribute_name", Attr::f32(1, vec![radius as f32; n]))?;
    }
    let pattern = p.s("detail_as_detail")?;
    for (name, value) in &geo.detail { if matches(name, &pattern) { out.detail.insert(name.clone(), value.clone()); } }
    if p.b("randomize_point_order")? {
        let mut order: Vec<usize> = (0..n).collect();
        let mut rng = Rng::new(&[(seed as u64) & 0xffffffff, 0x5ca77e]);
        for i in (1..n).rev() { let j = (rng.random()*(i+1) as f64) as usize; order.swap(i,j.min(i)); }
        for attr in out.point.values_mut() { *attr = Arc::new(super::super::group::gather(attr, &order)); }
    }
    Ok(())
}
