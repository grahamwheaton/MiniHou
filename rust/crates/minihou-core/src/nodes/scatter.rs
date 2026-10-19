//! `scatter` (`minihou/nodes/scatter.py`): area-weighted point scattering on surfaces, or rejection sampling in volumes.

use std::sync::Arc;

use super::{np_random::Rng, Inputs, NodeSpec, GEO};
use crate::error::{Error, Result};
use crate::geometry::{Attr, Geometry};
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

fn scatter(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = SCATTER.one(inputs, 0)?;
    let count = p.i("count")?;
    if count < 0 {
        return Err(Error::new("count must be non-negative"));
    }
    let count = count as usize;
    let seed = p.i("seed")?;
    let mode = p.e("mode", &["surface", "volume"])?;
    let density_attribute = p.s("density_attribute")?;

    if count == 0 || geo.nprims() == 0 {
        return Ok(empty_scatter());
    }

    if mode == "volume" {
        if !density_attribute.is_empty() { return Err(Error::new("density_attribute is supported only in surface mode")); }
        return scatter_volume(&geo, count, seed);
    }

    scatter_surface_with_density(&geo, count, seed, &density_attribute)
}

pub(super) fn scatter_surface(geo: &Geometry, count: usize, seed: i64) -> Result<Geometry> {
    scatter_surface_with_density(geo, count, seed, "")
}

fn scatter_surface_with_density(geo: &Geometry, count: usize, seed: i64, density_attribute: &str) -> Result<Geometry> {
    let density = if density_attribute.is_empty() { None } else {
        let attr = geo.point.get(density_attribute).ok_or_else(|| Error::new(format!("scatter: missing point density attribute {density_attribute:?}")))?;
        if attr.width != 1 || attr.rows() != geo.npoints { return Err(Error::new("scatter: density attribute must have one scalar value per point")); }
        Some(super::blender_input::numbers(attr))
    };
    let topo = &geo.topology;
    let p_slice = geo.p();
    let nprims = geo.nprims();
    let vp = &topo.vertex_point;
    let offs = &topo.prim_offsets;

    struct Tri {
        a: [f64; 3],
        b: [f64; 3],
        c: [f64; 3],
        prim: i32,
        points: [usize; 3],
    }

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
            let area = 0.5 * (dot(cr, cr)).sqrt();
            triangles.push(Tri {
                a,
                b,
                c,
                prim: prim as i32,
                points: [v0, v1, v2],
            });
            areas.push(area);
        }
    }

    if triangles.is_empty() {
        return Ok(empty_scatter());
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

    let mut rng = Rng::new(&[(seed as u64) & 0xFFFFFFFF]);
    let r: Vec<f64> = (0..count).map(|_| rng.random()).collect();
    let r1: Vec<f64> = (0..count).map(|_| rng.random()).collect();
    let r2: Vec<f64> = (0..count).map(|_| rng.random()).collect();
    let accept: Option<Vec<f64>> = density.as_ref().map(|_| (0..count).map(|_| rng.random()).collect());

    let mut out_pos = Vec::with_capacity(count * 3);
    let mut out_n = Vec::with_capacity(count * 3);
    let mut out_prim = Vec::with_capacity(count);

    let ntri = triangles.len();
    for s in 0..count {
        let target = r[s] * total_area;
        let mut t = cdf.partition_point(|&val| val <= target);
        if t >= ntri {
            t = ntri - 1;
        }
        let tri = &triangles[t];
        let mut u = r1[s];
        let mut v = r2[s];
        if u + v > 1.0 {
            u = 1.0 - u;
            v = 1.0 - v;
        }
        if let (Some(density), Some(accept)) = (&density, &accept) {
            let [a,b,c] = tri.points.map(|point| {
                let x = density[point];
                if x.is_finite() { x.clamp(0.0, 1.0) } else { 0.0 }
            });
            let probability = a * (1.0 - u - v) + b * u + c * v;
            if accept[s] >= probability { continue; }
        }
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
    }

    let mut res = Geometry::points_only(out_pos)?;
    res.point.insert("N".into(), Arc::new(Attr::f32(3, out_n)));
    res.point.insert("primnum".into(), Arc::new(Attr::i32(1, out_prim)));
    Ok(res)
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
}
