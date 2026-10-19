//! `scatter` (`minihou/nodes/scatter.py`): area-weighted point scattering on surfaces, or rejection sampling in volumes.

use std::{collections::BTreeMap, sync::Arc};

use super::{np_random::Rng, Inputs, NodeSpec, GEO};
use crate::error::{Error, Result};
use crate::geometry::{Attr, AttrData, Geometry};
use crate::params::Params;

pub const SCATTER: NodeSpec = NodeSpec {
    name: "scatter",
    inputs: GEO,
    params: crate::params!(
        "count": 1000;
        "seed": 0;
        "mode": "surface";
        "density_attribute": "";
        "group": "";
        "generate": "by_count";
        "scatter_on_voxels_independently": false;
        "force_total_count": false;
        "density_scale": 1.0;
        "override_prim_area": 0.0;
        "use_area_for_2d_volumes": true;
        "max_point_limit": 1000000;
        "global_seed": 0;
        "primitive_seed_attribute": "";
        "randomize_point_order": false;
        "relax_iterations": 0;
        "use_geometric_normals": false;
        "scale_radii_by": 1.0;
        "max_relax_radius": 0.0;
        "prim_num_attribute": false;
        "prim_num_attribute_name": "sourceprim";
        "prim_uvw_attribute": false;
        "prim_uvw_attribute_name": "sourceprimuv";
        "output_density_attribute": false;
        "output_density_attribute_name": "density";
        "output_radius_attribute": false;
        "output_radius_attribute_name": "pscale";
        "output_id_attribute": false;
        "output_id_attribute_name": "id";
        "output_radius_and_density_in_texture_space": false;
        "point_attributes": "*";
        "vertex_attributes": "N uv*";
        "primitive_attributes": "";
        "detail_attributes": "";
        "detail_as_detail": "";
    ),
    cook: scatter,
};

fn empty_scatter() -> Geometry {
    let mut g = Geometry::points_only(Vec::new()).unwrap();
    g.point.insert("N".into(), Arc::new(Attr::f32(3, Vec::new())));
    g.point.insert("primnum".into(), Arc::new(Attr::i32(1, Vec::new())));
    g
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

struct Tri {
    a: [f64; 3],
    b: [f64; 3],
    c: [f64; 3],
    prim: i32,
    points: [usize; 3],
    vertices: [usize; 3],
    physical_area: f64,
    texture_area: f64,
}

fn scatter(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = SCATTER.one(inputs, 0)?;
    let count = p.i("count")?;
    if count < 0 {
        return Err(Error::new("count must be non-negative"));
    }
    let count = count as usize;
    let seed = p.i("seed")?.wrapping_add(p.i("global_seed")?);
    let mode = p.e("mode", &["surface", "volume"])?;
    let density_attribute = p.s("density_attribute")?;

    if (count == 0 && !["by_density", "by_area", "in_texture_space"].contains(&p.s("generate")?.as_str())) || geo.nprims() == 0 {
        return Ok(empty_scatter());
    }

    if mode == "volume" {
        if !density_attribute.is_empty() { return Err(Error::new("density_attribute is supported only in surface mode")); }
        if !p.s("group")?.is_empty() { return Err(Error::new("scatter: volume group selection is not available for polygon interiors")); }
        if p.b("scatter_on_voxels_independently")? { return Err(Error::new("scatter: voxel sampling requires volume primitives")); }
        if p.i("relax_iterations")? != 0 { return Err(Error::new("scatter: volume relaxation requires volume primitives")); }
        let max = p.i("max_point_limit")?;
        if max < 0 { return Err(Error::new("scatter: max_point_limit must be non-negative")); }
        let generate = p.e("generate", &["by_count", "by_area", "by_density", "count_per_primitive", "in_texture_space"])?;
        if !["by_count", "by_density"].contains(&generate.as_str()) { return Err(Error::new("scatter: polygon interior mode supports By Count or By Density")); }
        let scale = p.f("density_scale")?;
        if !scale.is_finite() || scale < 0.0 { return Err(Error::new("scatter: density_scale must be finite and non-negative")); }
        let count = if generate == "by_density" && !p.b("force_total_count")? {
            let p = geo.p(); let mut lo = [f32::INFINITY;3]; let mut hi = [f32::NEG_INFINITY;3];
            for xyz in p.chunks_exact(3) { for k in 0..3 { lo[k] = lo[k].min(xyz[k]); hi[k] = hi[k].max(xyz[k]); } }
            ((0..3).map(|k| (hi[k]-lo[k]).max(0.0) as f64).product::<f64>() * scale).round() as usize
        } else { count };
        let mut out = scatter_volume(&geo, count.min(max as usize), seed)?;
        add_volume_output_attributes(&mut out, &geo, p, seed)?;
        return Ok(out);
    }

    scatter_surface_with_options(&geo, count, seed, &density_attribute, Some(p))
}

pub(super) fn scatter_surface(geo: &Geometry, count: usize, seed: i64) -> Result<Geometry> {
    scatter_surface_with_options(geo, count, seed, "", None)
}

fn scatter_surface_with_options(geo: &Geometry, count: usize, seed: i64, density_attribute: &str, options: Option<&Params>) -> Result<Geometry> {
    let (density, vertex_density, primitive_density) = if density_attribute.is_empty() { (None, None, None) } else if let Some(attr) = geo.point.get(density_attribute) {
        if attr.width != 1 || attr.rows() != geo.npoints { return Err(Error::new("scatter: density attribute must have one scalar value per point")); }
        (Some(super::blender_input::numbers(attr)), None, None)
    } else if let Some(attr) = geo.vertex.get(density_attribute) {
        if attr.width != 1 || attr.rows() != geo.topology.nvertices() { return Err(Error::new("scatter: density attribute must have one scalar value per vertex")); }
        (None, Some(super::blender_input::numbers(attr)), None)
    } else if let Some(attr) = geo.prim.get(density_attribute) {
        if attr.width != 1 || attr.rows() != geo.nprims() { return Err(Error::new("scatter: density attribute must have one scalar value per primitive")); }
        (None, None, Some(super::blender_input::numbers(attr)))
    } else if let Some(value) = geo.detail.get(density_attribute).and_then(|v| v.as_f64()) {
        (None, None, Some(vec![value; geo.nprims()]))
    } else { return Err(Error::new(format!("scatter: missing scalar density attribute {density_attribute:?}"))); };
    let group = options.map(|p| p.s("group")).transpose()?.unwrap_or_default();
    let selected = if group.is_empty() { vec![true; geo.nprims()] } else { super::group::mask(geo, "prim", &group)? };
    let generate = options.map(|p| p.e("generate", &["by_count", "by_area", "by_density", "count_per_primitive", "in_texture_space"])).transpose()?.unwrap_or_else(|| "by_count".into());
    let force_total_count = options.map(|p| p.b("force_total_count")).transpose()?.unwrap_or(false);
    let uv_attr = if generate == "in_texture_space" {
        let attr = geo.vertex.get("uv").filter(|a| a.width >= 2)
            .map(|a| (a, true)).or_else(|| geo.point.get("uv").filter(|a| a.width >= 2).map(|a| (a, false)))
            .ok_or_else(|| Error::new("scatter: In Texture Space requires a point or vertex uv attribute"))?;
        Some((super::blender_input::numbers(attr.0), attr.0.width, attr.1))
    } else { None };
    let density_scale = options.map(|p| p.f("density_scale")).transpose()?.unwrap_or(1.0);
    if !density_scale.is_finite() || density_scale < 0.0 { return Err(Error::new("scatter: density_scale must be finite and non-negative")); }
    let max_points = options.map(|p| p.i("max_point_limit")).transpose()?.unwrap_or(i32::MAX as i64);
    if max_points < 0 { return Err(Error::new("scatter: max_point_limit must be non-negative")); }
    let primitive_seed_name = options.map(|p| p.s("primitive_seed_attribute")).transpose()?.unwrap_or_default();
    let primitive_seeds = if primitive_seed_name.is_empty() { None } else {
        let attr = geo.prim.get(&primitive_seed_name).ok_or_else(|| Error::new(format!("scatter: missing primitive seed attribute {primitive_seed_name:?}")))?;
        if attr.width != 1 { return Err(Error::new("scatter: primitive seed attribute must be scalar")); }
        Some(super::blender_input::numbers(attr))
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
        .map(|a| super::blender_input::numbers(a));
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
            for attr in res.point.values_mut() { *attr = Arc::new(super::group::gather(attr, &order)); }
        }
    }
    Ok(res)
}

fn matches(name: &str, pattern: &str) -> bool {
    pattern.split_whitespace().any(|part| part == "*" || part == name ||
        part.strip_suffix('*').is_some_and(|prefix| name.starts_with(prefix)))
}

fn relax_samples(pos: &mut [f32], samples: &mut [(usize, [f64; 3])], tris: &[Tri], geo: &Geometry, iterations: usize, scale: f64, max_radius: f64, use_geometric_normals: bool) -> Result<()> {
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

fn sampled_attr(attr: &Attr, samples: &[(usize, [f64; 3])], tris: &[Tri], domain: &str) -> Attr {
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

fn add_output_attributes(out: &mut Geometry, geo: &Geometry, tris: &[Tri], samples: &[(usize, [f64; 3])], p: &Params, density_scale: f64, density: Option<&[f64]>, vertex_density: Option<&[f64]>, primitive_density: Option<&[f64]>) -> Result<()> {
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
        out.point.insert(name.clone(), Arc::new(super::group::gather(attr, &rows)));
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
    super::group::valid_name(&name)?;
    if out.point.contains_key(&name) { return Err(Error::new(format!("scatter: output attribute {name:?} already exists"))); }
    out.point.insert(name, Arc::new(attr));
    Ok(())
}

fn add_volume_output_attributes(out: &mut Geometry, geo: &Geometry, p: &Params, seed: i64) -> Result<()> {
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
        for attr in out.point.values_mut() { *attr = Arc::new(super::group::gather(attr, &order)); }
    }
    Ok(())
}

pub(super) fn scatter_volume(geo: &Geometry, count: usize, seed: i64) -> Result<Geometry> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relaxation_repels_across_shared_primitive_edge() {
        let g = Geometry::from_arrays(
            vec![0.,0.,0., 1.,0.,0., 0.,1.,0., 1.,1.,0.],
            vec![0,1,2, 1,3,2], vec![0,3,6], None,
        ).unwrap();
        let tris = [
            Tri { a:[0.,0.,0.], b:[1.,0.,0.], c:[0.,1.,0.], prim:0, points:[0,1,2], vertices:[0,1,2], physical_area:0.5, texture_area:0.5 },
            Tri { a:[1.,0.,0.], b:[1.,1.,0.], c:[0.,1.,0.], prim:1, points:[1,3,2], vertices:[3,4,5], physical_area:0.5, texture_area:0.5 },
        ];
        let mut pos = vec![0.49,0.49,0., 0.51,0.51,0.];
        let mut samples = vec![(0, [0.02,0.49,0.49]), (1, [0.49,0.02,0.49])];
        relax_samples(&mut pos, &mut samples, &tris, &g, 1, 1.0, 0.0, true).unwrap();
        let separation = ((pos[0]-pos[3]).powi(2)+(pos[1]-pos[4]).powi(2)).sqrt();
        assert!(separation > 0.04, "shared-edge samples should repel: {separation}");
    }
    use std::sync::Arc;

    #[test]
    fn scatter_grid_surface() {
        let g = Geometry::from_arrays(
            vec![
                0.0, 0.0, 0.0,
                1.0, 0.0, 0.0,
                1.0, 1.0, 0.0,
                0.0, 1.0, 0.0,
            ],
            vec![0, 1, 2, 3],
            vec![0, 4],
            None,
        ).unwrap();

        let p = Params::resolve(
            SCATTER.params,
            serde_json::json!({"count": 50, "seed": 1}).as_object(),
        ).unwrap();
        let res = (SCATTER.cook)(&[vec![Arc::new(g)]], &p).unwrap();
        assert_eq!(res.npoints, 50);
        assert_eq!(res.point.get("N").unwrap().rows(), 50);
        assert_eq!(res.point.get("primnum").unwrap().rows(), 50);
        assert_eq!(res.point.keys().map(String::as_str).collect::<Vec<_>>(), ["N", "P", "primnum"]);
        for chunk in res.p().chunks_exact(3) {
            assert!(chunk[0] >= 0.0 && chunk[0] <= 1.0);
            assert!(chunk[1] >= 0.0 && chunk[1] <= 1.0);
            assert_eq!(chunk[2], 0.0);
        }
    }

    #[test]
    fn scatter_empty_input() {
        let g = Geometry::points_only(vec![0.0, 0.0, 0.0]).unwrap();
        let p = Params::resolve(SCATTER.params, None).unwrap();
        let res = (SCATTER.cook)(&[vec![Arc::new(g)]], &p).unwrap();
        assert_eq!(res.npoints, 0);
    }

    #[test]
    fn scatter_density_zero_one_and_default() {
        let mut g = Geometry::from_arrays(
            vec![0.,0.,0., 1.,0.,0., 1.,1.,0., 0.,1.,0.],
            vec![0,1,2,3], vec![0,4], None).unwrap();
        let p = Params::resolve(SCATTER.params, serde_json::json!({"count":100,"seed":7}).as_object()).unwrap();
        let unweighted = (SCATTER.cook)(&[vec![Arc::new(g.clone())]], &p).unwrap();
        assert_eq!(unweighted.npoints, 100);
        let weighted = Params::resolve(SCATTER.params, serde_json::json!({"count":100,"seed":7,"density_attribute":"density"}).as_object()).unwrap();
        g.point.insert("density".into(), Arc::new(Attr::f32(1, vec![0.;4])));
        assert_eq!((SCATTER.cook)(&[vec![Arc::new(g.clone())]], &weighted).unwrap().npoints, 0);
        g.point.insert("density".into(), Arc::new(Attr::f32(1, vec![1.;4])));
        let full = (SCATTER.cook)(&[vec![Arc::new(g.clone())]], &weighted).unwrap();
        assert_eq!(full.npoints, 100);
        assert_eq!(full.p(), unweighted.p());
        assert_eq!((SCATTER.cook)(&[vec![Arc::new(g)]], &p).unwrap().p(), unweighted.p());
    }

    #[test]
    fn scatter_density_favors_dark_half() {
        let mut g = Geometry::from_arrays(
            vec![-1.,-1.,0., 0.,-1.,0., 1.,-1.,0., -1.,1.,0., 0.,1.,0., 1.,1.,0.],
            vec![0,1,4,3, 1,2,5,4], vec![0,4,8], None).unwrap();
        g.point.insert("density".into(), Arc::new(Attr::f32(1, vec![1.,0.,0.,1.,0.,0.])));
        let p = Params::resolve(SCATTER.params, serde_json::json!({"count":2000,"seed":11,"density_attribute":"density"}).as_object()).unwrap();
        let out = (SCATTER.cook)(&[vec![Arc::new(g)]], &p).unwrap();
        let left = out.p().chunks_exact(3).filter(|xyz| xyz[0] < 0.).count();
        assert!(left > out.npoints * 3 / 4, "left {left} of {}", out.npoints);
    }

    #[test]
    fn scatter_houdini_outputs_and_attribute_transfer() {
        let mut g = Geometry::from_arrays(vec![0.,0.,0., 1.,0.,0., 0.,1.,0.], vec![0,1,2], vec![0,3], None).unwrap();
        g.point.insert("Cd".into(), Arc::new(Attr::f32(1, vec![0.,1.,0.])));
        g.vertex.insert("uv".into(), Arc::new(Attr::f32(2, vec![0.,0., 1.,0., 0.,1.])));
        g.prim.insert("piece".into(), Arc::new(Attr::i32(1, vec![7])));
        g.detail.insert("quality".into(), serde_json::json!(4));
        let args = serde_json::json!({"count": 20, "point_attributes":"Cd", "vertex_attributes":"uv",
            "primitive_attributes":"piece", "detail_attributes":"quality", "prim_num_attribute":true,
            "prim_uvw_attribute":true, "output_density_attribute":true, "output_radius_attribute":true,
            "output_id_attribute":true, "randomize_point_order":true});
        let p = Params::resolve(SCATTER.params, args.as_object()).unwrap();
        let out = (SCATTER.cook)(&[vec![Arc::new(g)]], &p).unwrap();
        assert_eq!(out.npoints, 20);
        for name in ["sourceprim", "sourceprimuv", "density", "pscale", "id", "Cd", "uv", "piece", "quality"] { assert_eq!(out.point[name].rows(), 20, "{name}"); }
        assert!(out.point["sourceprim"].as_i32().unwrap().iter().all(|&v| v == 0));
        assert!(out.point["piece"].as_i32().unwrap().iter().all(|&v| v == 7));
        let mut ids = out.point["id"].as_i32().unwrap().to_vec(); ids.sort();
        assert_eq!(ids, (0..20).collect::<Vec<_>>());
        for (&cd, &uv) in out.point["Cd"].as_f32().unwrap().iter().zip(out.point["uv"].as_f32().unwrap().chunks_exact(2).map(|r| &r[0])) {
            assert!((cd-uv).abs()<1e-5);
        }
    }

    #[test]
    fn scatter_group_count_and_limit() {
        let g = Geometry::from_arrays(vec![0.,0.,0., 1.,0.,0., 1.,1.,0., 0.,1.,0., 2.,0.,0., 2.,1.,0.],
            vec![0,1,2,3, 1,4,5,2], vec![0,4,8], None).unwrap();
        let args = serde_json::json!({"group":"0", "count":40, "max_point_limit":12});
        let p = Params::resolve(SCATTER.params, args.as_object()).unwrap();
        let out = (SCATTER.cook)(&[vec![Arc::new(g)]], &p).unwrap();
        assert_eq!(out.npoints, 12);
        assert!(out.p().chunks_exact(3).all(|xyz| xyz[0] <= 1.0));
    }

    #[test]
    fn scatter_density_modes_and_texture_space() {
        let mut g = Geometry::from_arrays(vec![0.,0.,0., 2.,0.,0., 0.,2.,0.], vec![0,1,2], vec![0,3], None).unwrap();
        g.prim.insert("density".into(), Arc::new(Attr::f32(1, vec![0.0])));
        g.vertex.insert("uv".into(), Arc::new(Attr::f32(2, vec![0.,0., 1.,0., 0.,1.])));
        let run = |g: &Geometry, args: serde_json::Value| {
            let p = Params::resolve(SCATTER.params, args.as_object()).unwrap();
            (SCATTER.cook)(&[vec![Arc::new(g.clone())]], &p).unwrap()
        };
        assert_eq!(run(&g, serde_json::json!({"count":50,"density_attribute":"density"})).npoints, 0);
        g.prim.insert("density".into(), Arc::new(Attr::f32(1, vec![1.0])));
        assert_eq!(run(&g, serde_json::json!({"count":50,"density_attribute":"density"})).npoints, 50);
        let tex = run(&g, serde_json::json!({"generate":"in_texture_space","density_scale":40.0}));
        assert_eq!(tex.npoints, 20);
        assert!(tex.p().chunks_exact(3).all(|xyz| xyz[0] >= 0.0 && xyz[1] >= 0.0 && xyz[0]+xyz[1] <= 2.0));
        let relaxed = run(&g, serde_json::json!({"count":30,"relax_iterations":3}));
        assert_eq!(relaxed.npoints, 30);
        assert_ne!(relaxed.p(), run(&g, serde_json::json!({"count":30})).p());
    }

    #[test]
    fn scatter_volume_opt_in_output_names() {
        let g = Geometry::from_arrays(
            vec![0.,0.,0., 1.,0.,0., 1.,1.,0., 0.,1.,0., 0.,0.,1., 1.,0.,1., 1.,1.,1., 0.,1.,1.],
            vec![0,3,2,1, 4,5,6,7, 0,1,5,4, 1,2,6,5, 2,3,7,6, 3,0,4,7],
            vec![0,4,8,12,16,20,24], None).unwrap();
        let args = serde_json::json!({"mode":"volume", "count":10, "prim_num_attribute":true,
            "prim_uvw_attribute":true, "output_id_attribute":true, "output_density_attribute":true,
            "output_radius_attribute":true});
        let p = Params::resolve(SCATTER.params, args.as_object()).unwrap();
        let out = (SCATTER.cook)(&[vec![Arc::new(g)]], &p).unwrap();
        assert_eq!(out.npoints, 10);
        for name in ["sourceprim", "sourceprimuv", "id", "density", "pscale"] { assert_eq!(out.point[name].rows(), 10); }
        assert!(out.point["sourceprim"].as_i32().unwrap().iter().all(|&v| v == -1));
    }

    #[test]
    fn scatter_primitive_attribute_override_area() {
        // Two triangles: prim 0 has physical area 2.0, prim 1 has physical area 2.0
        let mut g = Geometry::from_arrays(
            vec![0.,0.,0., 2.,0.,0., 0.,2.,0., 10.,0.,0., 12.,0.,0., 10.,2.,0.],
            vec![0,1,2, 3,4,5],
            vec![0,3,6],
            None,
        ).unwrap();
        // Prim 0 has override area 0.001, Prim 1 has override area 10.0
        g.prim.insert("override_prim_area".into(), Arc::new(Attr::f32(1, vec![0.001, 10.0])));
        let p = Params::resolve(SCATTER.params, serde_json::json!({
            "count": 100,
            "generate": "by_count",
            "prim_num_attribute": true
        }).as_object()).unwrap();
        let out = (SCATTER.cook)(&[vec![Arc::new(g)]], &p).unwrap();
        assert_eq!(out.npoints, 100);
        let src_prims = out.point["sourceprim"].as_i32().unwrap();
        let count_p1 = src_prims.iter().filter(|&&p| p == 1).count();
        // Vast majority should be scattered on prim 1 due to area override
        assert!(count_p1 > 90);
    }
}
