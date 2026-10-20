use std::{collections::BTreeMap, sync::Arc};
use super::{Tri, add_output_attributes, sampled_attr};
use super::super::np_random::Rng;
use crate::error::{Error, Result};
use crate::geometry::{Attr, Geometry};
use crate::params::Params;

pub(super) fn empty_scatter() -> Geometry {
    let mut g = Geometry::points_only(Vec::new()).unwrap();
    g.point.insert("N".into(), Arc::new(Attr::f32(3, Vec::new())));
    g.point.insert("primnum".into(), Arc::new(Attr::i32(1, Vec::new())));
    g
}

pub(super) fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub(super) fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

pub(in crate::nodes) fn scatter_surface(geo: &Geometry, count: usize, seed: i64) -> Result<Geometry> {
    scatter_surface_with_options(geo, count, seed, "", None)
}

pub(super) fn scatter_surface_with_options(geo: &Geometry, count: usize, seed: i64, density_attribute: &str, options: Option<&Params>) -> Result<Geometry> {
    let (density, vertex_density, primitive_density) = if density_attribute.is_empty() { (None, None, None) } else if let Some(attr) = geo.point.get(density_attribute) {
        if attr.width != 1 || attr.rows() != geo.npoints { return Err(Error::new("scatter: density attribute must have one scalar value per point")); }
        (Some(super::super::blender_input::numbers(attr)), None, None)
    } else if let Some(attr) = geo.vertex.get(density_attribute) {
        if attr.width != 1 || attr.rows() != geo.topology.nvertices() { return Err(Error::new("scatter: density attribute must have one scalar value per vertex")); }
        (None, Some(super::super::blender_input::numbers(attr)), None)
    } else if let Some(attr) = geo.prim.get(density_attribute) {
        if attr.width != 1 || attr.rows() != geo.nprims() { return Err(Error::new("scatter: density attribute must have one scalar value per primitive")); }
        (None, None, Some(super::super::blender_input::numbers(attr)))
    } else if let Some(value) = geo.detail.get(density_attribute).and_then(|v| v.as_f64()) {
        (None, None, Some(vec![value; geo.nprims()]))
    } else { return Err(Error::new(format!("scatter: missing scalar density attribute {density_attribute:?}"))); };
    let group = options.map(|p| p.s("group")).transpose()?.unwrap_or_default();
    let selected = if group.is_empty() { vec![true; geo.nprims()] } else { super::super::group::mask(geo, "prim", &group)? };
    let generate = options.map(|p| p.e("generate", &["by_count", "by_area", "by_density", "count_per_primitive", "in_texture_space"])).transpose()?.unwrap_or_else(|| "by_count".into());
    let force_total_count = options.map(|p| p.b("force_total_count")).transpose()?.unwrap_or(false);
    let uv_attr = if generate == "in_texture_space" {
        let attr = geo.vertex.get("uv").filter(|a| a.width >= 2)
            .map(|a| (a, true)).or_else(|| geo.point.get("uv").filter(|a| a.width >= 2).map(|a| (a, false)))
            .ok_or_else(|| Error::new("scatter: In Texture Space requires a point or vertex uv attribute"))?;
        Some((super::super::blender_input::numbers(attr.0), attr.0.width, attr.1))
    } else { None };
    let density_scale = options.map(|p| p.f("density_scale")).transpose()?.unwrap_or(1.0);
    if !density_scale.is_finite() || density_scale < 0.0 { return Err(Error::new("scatter: density_scale must be finite and non-negative")); }
    let max_points = options.map(|p| p.i("max_point_limit")).transpose()?.unwrap_or(i32::MAX as i64);
    if max_points < 0 { return Err(Error::new("scatter: max_point_limit must be non-negative")); }
    let primitive_seed_name = options.map(|p| p.s("primitive_seed_attribute")).transpose()?.unwrap_or_default();
    let primitive_seeds = if primitive_seed_name.is_empty() { None } else {
        let attr = geo.prim.get(&primitive_seed_name).ok_or_else(|| Error::new(format!("scatter: missing primitive seed attribute {primitive_seed_name:?}")))?;
        if attr.width != 1 { return Err(Error::new("scatter: primitive seed attribute must be scalar")); }
        Some(super::super::blender_input::numbers(attr))
    };
    let topo = &geo.topology;
    let p_slice = geo.p();
    let nprims = geo.nprims();
    let vp = &topo.vertex_point;
    let offs = &topo.prim_offsets;

    let mut triangles = Vec::new();
    let mut areas = Vec::new();

    let get_p = |pt: usize| -> [f64; 3] {
        [
            p_slice[pt * 3] as f64,
            p_slice[pt * 3 + 1] as f64,
            p_slice[pt * 3 + 2] as f64,
        ]
    };

    for prim in 0..nprims {
        if !selected[prim] { continue; }
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
            let physical_area = 0.5 * dot(cr, cr).sqrt();
            let texture_area = if let Some((values, width, is_vertex)) = &uv_attr {
                let ids = if *is_vertex { [start, start+j+1, start+j+2] } else { [v0,v1,v2] };
                let get_uv = |id: usize| [values[id* *width], values[id* *width+1]];
                let [a,b,c] = ids.map(get_uv);
                0.5 * ((b[0]-a[0])*(c[1]-a[1])-(b[1]-a[1])*(c[0]-a[0])).abs()
            } else { physical_area };
            let area = if uv_attr.is_some() { texture_area } else { physical_area };
            triangles.push(Tri {
                a,
                b,
                c,
                prim: prim as i32,
                points: [v0, v1, v2],
                vertices: [start, start + j + 1, start + j + 2],
                physical_area,
                texture_area,
            });
            areas.push(area);
        }
    }

    if triangles.is_empty() {
        return Ok(empty_scatter());
    }

    let override_area = options.map(|p| p.f("override_prim_area")).transpose()?.unwrap_or(0.0);
    if !override_area.is_finite() || override_area < 0.0 { return Err(Error::new("scatter: override_prim_area must be finite and non-negative")); }
    let prim_override = geo.prim.get("override_prim_area")
        .or_else(|| geo.prim.get("area"))
        .filter(|a| a.width == 1)
        .map(|a| super::super::blender_input::numbers(a));
    if override_area > 0.0 || prim_override.is_some() {
        let mut prim_areas = vec![0.0; nprims];
        for (tri, &area) in triangles.iter().zip(&areas) { prim_areas[tri.prim as usize] += area; }
        for (tri, area) in triangles.iter().zip(&mut areas) {
            let p_idx = tri.prim as usize;
            let target_area = prim_override.as_ref()
                .and_then(|po| po.get(p_idx).copied())
                .filter(|&v| v > 0.0)
                .unwrap_or(override_area);
            if target_area > 0.0 {
                let total = prim_areas[p_idx];
                *area = if total > 0.0 { *area * target_area / total } else { 0.0 };
            }
        }
    }
    if force_total_count && (density.is_some() || vertex_density.is_some() || primitive_density.is_some()) {
        for (tri, area) in triangles.iter().zip(&mut areas) {
            let mean = if let Some(values) = &density { tri.points.iter().map(|&id| values[id].max(0.0)).sum::<f64>() / 3.0 }
                else if let Some(values) = &vertex_density { tri.vertices.iter().map(|&id| values[id].max(0.0)).sum::<f64>() / 3.0 }
                else { primitive_density.as_ref().unwrap()[tri.prim as usize].max(0.0) };
            *area *= mean;
        }
    }

    let mut cdf = Vec::with_capacity(areas.len());
    let mut accum = 0.0;
    for &a in &areas {
        accum += a;
        cdf.push(accum);
    }
    let total_area = accum;
    if total_area <= 0.0 {
        return Ok(empty_scatter());
    }

    let per_prim_count = count;
    let active_prims: Vec<usize> = (0..nprims).filter(|&prim| triangles.iter().zip(&areas).any(|(tri, &a)| tri.prim as usize == prim && a > 0.0)).collect();
    let count = match generate.as_str() {
        "by_density" | "in_texture_space" if force_total_count => count,
        "by_density" | "by_area" => (total_area * density_scale).round().max(0.0) as usize,
        "in_texture_space" => (total_area * density_scale).round().max(0.0) as usize,
        "count_per_primitive" => count.saturating_mul(active_prims.len()),
        _ => count,
    }.min(max_points as usize);
    if count == 0 { return Ok(empty_scatter()); }

    let mut rng = Rng::new(&[(seed as u64) & 0xFFFFFFFF]);
    let r: Vec<f64> = (0..count).map(|_| rng.random()).collect();
    let r1: Vec<f64> = (0..count).map(|_| rng.random()).collect();
    let r2: Vec<f64> = (0..count).map(|_| rng.random()).collect();
    let accept: Option<Vec<f64>> = (density.is_some() || vertex_density.is_some() || primitive_density.is_some()).then(|| (0..count).map(|_| rng.random()).collect());

    let mut out_pos = Vec::with_capacity(count * 3);
    let mut out_n = Vec::with_capacity(count * 3);
    let mut out_prim = Vec::with_capacity(count);
    let mut samples = Vec::with_capacity(count);

    let ntri = triangles.len();
    for s in 0..count {
        let target = if generate == "count_per_primitive" {
            let prim = active_prims[(s / per_prim_count).min(active_prims.len()-1)];
            let first = triangles.partition_point(|tri| (tri.prim as usize) < prim);
            let end = triangles.partition_point(|tri| (tri.prim as usize) <= prim);
            let start_area = if first == 0 { 0.0 } else { cdf[first-1] };
            start_area + r[s] * (cdf[end-1] - start_area)
        } else { r[s] * total_area };
        let mut t = cdf.partition_point(|&val| val <= target);
        if t >= ntri {
            t = ntri - 1;
        }
        let tri = &triangles[t];
        let (mut u, mut v) = if let Some(seeds) = &primitive_seeds {
            let mut local = Rng::new(&[(seed as u64) & 0xffffffff, seeds[tri.prim as usize] as u64, s as u64]);
            (local.random(), local.random())
        } else { (r1[s], r2[s]) };
        if u + v > 1.0 {
            u = 1.0 - u;
            v = 1.0 - v;
        }
        if !force_total_count { if let Some(accept) = &accept {
            let probability = if let Some(density) = &density {
                let [a,b,c] = tri.points.map(|point| {
                    let x = density[point]; if x.is_finite() { x.clamp(0.0, 1.0) } else { 0.0 }
                });
                a * (1.0 - u - v) + b * u + c * v
            } else if let Some(values) = &vertex_density {
                let [a,b,c] = tri.vertices.map(|id| values[id].clamp(0.0, 1.0));
                a * (1.0-u-v) + b*u + c*v
            } else { primitive_density.as_ref().unwrap()[tri.prim as usize].clamp(0.0, 1.0) };
            if accept[s] >= (probability * density_scale).clamp(0.0, 1.0) { continue; }
        }}
        let e1 = [tri.b[0] - tri.a[0], tri.b[1] - tri.a[1], tri.b[2] - tri.a[2]];
        let e2 = [tri.c[0] - tri.a[0], tri.c[1] - tri.a[1], tri.c[2] - tri.a[2]];
        let px = tri.a[0] + e1[0] * u + e2[0] * v;
        let py = tri.a[1] + e1[1] * u + e2[1] * v;
        let pz = tri.a[2] + e1[2] * u + e2[2] * v;
        out_pos.push(px as f32);
        out_pos.push(py as f32);
        out_pos.push(pz as f32);

        let cr = cross(e1, e2);
        let mut ln = (dot(cr, cr)).sqrt();
        if ln < 1e-20 {
            ln = 1.0;
        }
        out_n.push((cr[0] / ln) as f32);
        out_n.push((cr[1] / ln) as f32);
        out_n.push((cr[2] / ln) as f32);

        out_prim.push(tri.prim);
        samples.push((t, [1.0 - u - v, u, v]));
    }

    if let Some(p) = options {
        let iterations = p.i("relax_iterations")?;
        if iterations < 0 || iterations > 100 { return Err(Error::new("scatter: relax_iterations must be between 0 and 100")); }
        if iterations > 0 {
            relax_samples(&mut out_pos, &mut samples, &triangles, geo, iterations as usize, p.f("scale_radii_by")?, p.f("max_relax_radius")?, p.b("use_geometric_normals")?)?;
            for (i, &(ti, _)) in samples.iter().enumerate() {
                let tri = &triangles[ti];
                out_prim[i] = tri.prim;
                let e1 = [0,1,2].map(|k| tri.b[k]-tri.a[k]);
                let e2 = [0,1,2].map(|k| tri.c[k]-tri.a[k]);
                let normal = cross(e1, e2);
                let length = dot(normal, normal).sqrt().max(1e-20);
                for k in 0..3 { out_n[3*i+k] = (normal[k]/length) as f32; }
            }
        }
    }
    let mut res = Geometry::points_only(out_pos)?;
    res.point.insert("N".into(), Arc::new(Attr::f32(3, out_n)));
    res.point.insert("primnum".into(), Arc::new(Attr::i32(1, out_prim)));
    if let Some(p) = options {
        let output_density_scale = if ["by_count", "count_per_primitive"].contains(&generate.as_str()) || force_total_count {
            count as f64 / total_area
        } else { density_scale };
        add_output_attributes(&mut res, geo, &triangles, &samples, p, output_density_scale, density.as_deref(), vertex_density.as_deref(), primitive_density.as_deref())?;
        if p.b("randomize_point_order")? {
            let mut order: Vec<usize> = (0..res.npoints).collect();
            let mut shuffle = Rng::new(&[(seed as u64) & 0xffffffff, 0x5ca77e]);
            for i in (1..order.len()).rev() { let j = (shuffle.random() * (i + 1) as f64) as usize; order.swap(i, j.min(i)); }
            for attr in res.point.values_mut() { *attr = Arc::new(super::super::group::gather(attr, &order)); }
        }
    }
    Ok(res)
}

pub(super) fn matches(name: &str, pattern: &str) -> bool {
    pattern.split_whitespace().any(|part| part == "*" || part == name ||
        part.strip_suffix('*').is_some_and(|prefix| name.starts_with(prefix)))
}

pub(super) fn relax_samples(pos: &mut [f32], samples: &mut [(usize, [f64; 3])], tris: &[Tri], geo: &Geometry, iterations: usize, scale: f64, max_radius: f64, use_geometric_normals: bool) -> Result<()> {
    if !scale.is_finite() || scale < 0.0 || !max_radius.is_finite() || max_radius < 0.0 { return Err(Error::new("scatter: relaxation radii must be finite and non-negative")); }
    if samples.len() > 10_000 { return Err(Error::new("scatter: relaxation supports at most 10000 points")); }
    let n = samples.len();
    if n < 2 || scale == 0.0 { return Ok(()); }
    let source_normals = if use_geometric_normals { None } else {
        geo.vertex.get("N").map(|a| (a, "vertex")).or_else(|| geo.point.get("N").map(|a| (a, "point")))
            .and_then(|(a, domain)| sampled_attr(a, samples, tris, domain).as_f32().map(|values| values.to_vec()))
    };
    let area: f64 = tris.iter().map(|t| {
        let e1 = [t.b[0]-t.a[0],t.b[1]-t.a[1],t.b[2]-t.a[2]];
        let e2 = [t.c[0]-t.a[0],t.c[1]-t.a[1],t.c[2]-t.a[2]];
        0.5 * dot(cross(e1,e2), cross(e1,e2)).sqrt()
    }).sum();
    let radius = (0.537 * (area / n as f64).sqrt() * scale).min(if max_radius > 0.0 { max_radius } else { f64::INFINITY });
    // Shared source edges let points repel and travel between triangulated faces.
    let mut edge_tris: BTreeMap<(usize, usize), Vec<usize>> = BTreeMap::new();
    for (ti, tri) in tris.iter().enumerate() {
        for (a, b) in [(0, 1), (1, 2), (2, 0)] {
            let (lo, hi) = (tri.points[a].min(tri.points[b]), tri.points[a].max(tri.points[b]));
            edge_tris.entry((lo, hi)).or_default().push(ti);
        }
    }
    let mut neighbours: Vec<Vec<usize>> = (0..tris.len()).map(|i| vec![i]).collect();
    for touching in edge_tris.values() {
        for &i in touching { neighbours[i].extend(touching.iter().copied()); }
    }
    for adjacent in &mut neighbours { adjacent.sort_unstable(); adjacent.dedup(); }
    for _ in 0..iterations {
        let mut delta = vec![[0.0;3]; n];
        for i in 0..n { for j in i+1..n {
            if neighbours[samples[i].0].binary_search(&samples[j].0).is_err() { continue; }
            let mut d = [pos[3*i] as f64-pos[3*j] as f64,pos[3*i+1] as f64-pos[3*j+1] as f64,pos[3*i+2] as f64-pos[3*j+2] as f64];
            if let Some(normals) = &source_normals {
                if normals.len() >= 3*n {
                    let normal = [0,1,2].map(|k| normals[3*i+k] as f64 + normals[3*j+k] as f64);
                    let len = dot(normal,normal).sqrt();
                    if len > 1e-12 { let projection = dot(d,normal)/(len*len); for k in 0..3 { d[k] -= projection*normal[k]; } }
                }
            }
            let len = dot(d,d).sqrt();
            if len > 1e-9 && len < 2.0*radius {
                let push = (2.0*radius-len)*0.125/len;
                for k in 0..3 { delta[i][k] += d[k]*push; delta[j][k] -= d[k]*push; }
            }
        }}
        for i in 0..n {
            let target = [0,1,2].map(|k| pos[3*i+k] as f64 + delta[i][k]);
            let mut best = None;
            for &ti in &neighbours[samples[i].0] {
                let tri = &tris[ti];
                let e1 = [0,1,2].map(|k| tri.b[k]-tri.a[k]);
                let e2 = [0,1,2].map(|k| tri.c[k]-tri.a[k]);
                let v = [0,1,2].map(|k| target[k]-tri.a[k]);
                let (aa,ab,bb) = (dot(e1,e1),dot(e1,e2),dot(e2,e2));
                let det = aa*bb-ab*ab;
                if det <= 1e-20 { continue; }
                let mut u = (dot(v,e1)*bb-dot(v,e2)*ab)/det;
                let mut w = (dot(v,e2)*aa-dot(v,e1)*ab)/det;
                u = u.max(0.0); w = w.max(0.0);
                if u+w > 1.0 { let sum = u+w; u /= sum; w /= sum; }
                let projected = [0,1,2].map(|k| tri.a[k]+u*e1[k]+w*e2[k]);
                let distance = (0..3).map(|k| (target[k]-projected[k]).powi(2)).sum::<f64>();
                if best.as_ref().is_none_or(|(d, ..): &(f64, usize, [f64; 3], [f64; 3])| distance < *d) {
                    best = Some((distance, ti, [1.0-u-w,u,w], projected));
                }
            }
            if let Some((_, ti, weights, projected)) = best {
                samples[i] = (ti, weights);
                for k in 0..3 { pos[3*i+k] = projected[k] as f32; }
            }
        }
    }
    Ok(())
}

