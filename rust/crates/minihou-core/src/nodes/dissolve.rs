//! Dissolve node: merge neighbouring coplanar faces into n-gons.
//! Port of Python's `dissolve` from `minihou/nodes/surface.py`.

use std::collections::{BTreeMap, BTreeSet};

use super::{GEO, Inputs, NodeSpec};
use crate::boolean_kernel as k;
use crate::error::Result;
use crate::geometry::Geometry;
use crate::params::Params;

pub const DISSOLVE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "dissolve",
    inputs: GEO,
    params: crate::params!(
        "group": "";
        "angle": 1.0
    ),
    cook: dissolve_node,
};

fn dot3(a: [f64; 3], b: [f64; 3]) -> f64 {
    ((a[0] as f32) * (b[0] as f32)
        + (a[1] as f32) * (b[1] as f32)
        + (a[2] as f32) * (b[2] as f32)) as f64
}

fn find(parent: &mut [usize], mut x: usize) -> usize {
    while parent[x] != x {
        parent[x] = parent[parent[x]];
        x = parent[x];
    }
    x
}

fn flat_groups(geo: &Geometry, sel: &[bool], cos_tol: f64) -> Vec<usize> {
    let nprims = geo.nprims();
    if nprims == 0 {
        return Vec::new();
    }
    let raw_pn = crate::normals::prim_normals(geo.p(), &geo.topology);
    let pn: Vec<[f64; 3]> = raw_pn
        .into_iter()
        .map(|n| {
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            let s = if len < 1e-20 { 1.0 } else { len };
            [
                (n[0] / s) as f32 as f64,
                (n[1] / s) as f32 as f64,
                (n[2] / s) as f32 as f64,
            ]
        })
        .collect();

    let mut pv = vec![0usize; geo.nvertices()];
    for p in 0..nprims {
        let s = geo.topology.prim_offsets[p] as usize;
        let e = geo.topology.prim_offsets[p + 1] as usize;
        for v in s..e {
            pv[v] = p;
        }
    }

    let tw = super::modeling_tools::twins(geo);

    struct EdgeCandidate {
        a: usize,
        b: usize,
        dot: f64,
    }

    let mut candidates = Vec::new();
    for he in 0..geo.nvertices() {
        if let Some(tw_he) = tw[he] {
            let a = pv[he];
            let b = pv[tw_he];
            if a < b && sel[a] && sel[b] {
                let dot = dot3(pn[a], pn[b]);
                if dot >= cos_tol {
                    candidates.push(EdgeCandidate { a, b, dot });
                }
            }
        }
    }

    // Stable sort descending by dot (flattest pairs first)
    candidates.sort_by(|x, y| y.dot.total_cmp(&x.dot));

    let mut parent: Vec<usize> = (0..nprims).collect();
    for c in candidates {
        let ra = find(&mut parent, c.a);
        let rb = find(&mut parent, c.b);
        if ra == rb {
            continue;
        }
        if dot3(pn[ra], pn[rb]) < cos_tol {
            continue;
        }
        let (lo, hi) = if ra < rb { (ra, rb) } else { (rb, ra) };
        parent[hi] = lo;
    }

    (0..nprims).map(|i| find(&mut parent, i)).collect()
}

fn outline(loops: Vec<Vec<usize>>, v: &[usize], p: &[[f64; 3]]) -> Option<Vec<Vec<usize>>> {
    if loops.len() == 1 {
        return Some(loops);
    }
    let mut n = [0.; 3];
    for lp in &loops {
        for k_idx in 0..lp.len() {
            n = k::add(
                n,
                k::cross(p[v[lp[k_idx]]], p[v[lp[(k_idx + 1) % lp.len()]]]),
            );
        }
    }
    if k::len(n) < 1e-300 {
        return None;
    }
    let (u, w) = k::basis(k::unit(n));
    let xy: Vec<_> = v
        .iter()
        .map(|&i| [k::dot(p[i], u), k::dot(p[i], w)])
        .collect();
    let areas: Vec<_> = loops.iter().map(|q| k::area2(q, &xy)).collect();
    let outer = (0..areas.len()).max_by(|&a, &b| {
        areas[a]
            .total_cmp(&areas[b])
            .then(b.cmp(&a))
    })?;
    if areas[outer] <= 0. || areas.iter().filter(|&&a| a > 0.).count() != 1 {
        return None;
    }
    let holes = loops
        .iter()
        .enumerate()
        .filter(|&(i, _)| i != outer)
        .map(|(_, l)| l.iter().rev().copied().collect())
        .collect();
    k::holes_into(loops[outer].clone(), holes, &xy).ok()
}

fn merge_pieces(
    v: &[usize],
    sizes: &[usize],
    group: &[usize],
    p: &[[f64; 3]],
) -> Vec<(Vec<usize>, usize)> {
    let mut first = Vec::with_capacity(sizes.len());
    let mut poly_of = Vec::with_capacity(v.len());
    let mut next = Vec::with_capacity(v.len());
    let mut f = 0;
    for (q, &n) in sizes.iter().enumerate() {
        first.push(f);
        for k_idx in 0..n {
            poly_of.push(q);
            next.push(f + (k_idx + 1) % n);
        }
        f += n;
    }

    let mut key = BTreeMap::new();
    for c in 0..v.len() {
        key.insert((group[poly_of[c]], v[c], v[next[c]]), c);
    }
    let mut comp = k::Dsu::new(sizes.len());
    for c in 0..v.len() {
        if let Some(&o) = key.get(&(group[poly_of[c]], v[next[c]], v[c])) {
            comp.join(poly_of[c], poly_of[o]);
        }
    }
    let mut components: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for q in 0..sizes.len() {
        components.entry(comp.root(q)).or_default().push(q);
    }
    let mut out = vec![];
    for (root, polys) in components {
        let g = group[polys[0]];
        if polys.len() == 1 {
            let q = polys[0];
            out.push(((first[q]..first[q] + sizes[q]).collect(), g));
            continue;
        }
        let mut succ = BTreeMap::new();
        let mut order = vec![];
        let mut valid = true;
        for &q in &polys {
            for c in first[q]..first[q] + sizes[q] {
                let opposite = key.get(&(g, v[next[c]], v[c])).copied();
                if opposite.is_none_or(|o| comp.root(poly_of[o]) != root) {
                    if succ.insert(v[c], c).is_some() {
                        valid = false;
                        break;
                    }
                    order.push(v[c]);
                }
            }
            if !valid {
                break;
            }
        }
        let mut loops = vec![];
        let mut seen = BTreeSet::new();
        if valid && !succ.is_empty() {
            for start in order {
                if seen.contains(&start) {
                    continue;
                }
                let mut cur = start;
                let mut lp = vec![];
                while seen.insert(cur) {
                    if let Some(&c) = succ.get(&cur) {
                        lp.push(c);
                        cur = v[next[c]];
                    } else {
                        valid = false;
                        break;
                    }
                }
                if cur != start {
                    valid = false;
                    break;
                }
                loops.push(lp);
            }
        } else {
            valid = false;
        }
        let merged = if valid { outline(loops, v, p) } else { None };
        if let Some(merged) = merged {
            out.extend(merged.into_iter().map(|q| (q, g)));
        } else {
            for q in polys {
                out.push(((first[q]..first[q] + sizes[q]).collect(), g));
            }
        }
    }
    out
}

pub fn dissolve(geo: &Geometry, group: &str, angle: f64) -> Result<Geometry> {
    super::model::polygons(geo, "dissolve")?;
    if geo.nprims() < 2 {
        return Ok(geo.clone());
    }
    let sel = super::group::mask(geo, "prim", group)?;
    let cos_tol = angle.to_radians().cos();
    let rep = flat_groups(geo, &sel, cos_tol);
    if (0..geo.nprims()).all(|i| rep[i] == i) {
        return Ok(geo.clone());
    }

    let sizes: Vec<usize> = geo
        .topology
        .prim_offsets
        .windows(2)
        .map(|w| (w[1] - w[0]) as usize)
        .collect();
    let vp: Vec<usize> = geo
        .topology
        .vertex_point
        .iter()
        .map(|&p| p as usize)
        .collect();
    let pos: Vec<[f64; 3]> = geo
        .p()
        .chunks_exact(3)
        .map(|c| [c[0] as f64, c[1] as f64, c[2] as f64])
        .collect();

    let merged = merge_pieces(&vp, &sizes, &rep, &pos);
    let mut order: Vec<usize> = (0..merged.len()).collect();
    order.sort_by(|&i, &j| merged[i].1.cmp(&merged[j].1));

    let mut cidx = Vec::new();
    let mut msz = Vec::with_capacity(order.len());
    let mut mrep = Vec::with_capacity(order.len());
    for &o in &order {
        let (corners, g) = &merged[o];
        msz.push(corners.len());
        mrep.push(*g);
        cidx.extend(corners.iter().copied());
    }

    let new_vp: Vec<i32> = cidx.iter().map(|&c| geo.topology.vertex_point[c]).collect();
    let mut offsets = Vec::with_capacity(msz.len() + 1);
    offsets.push(0i32);
    for s in &msz {
        offsets.push(offsets.last().unwrap() + *s as i32);
    }

    let vertex = super::model::gather(&geo.vertex, &cidx);
    let prim = super::model::gather(&geo.prim, &mrep);

    let mut out = Geometry::from_arrays(geo.p().to_vec(), new_vp, offsets, None)?;
    for (name, attr) in &geo.point {
        if name != "P" {
            out.point.insert(name.clone(), attr.clone());
        }
    }
    out.vertex = vertex;
    out.prim = prim;
    out.detail = geo.detail.clone();
    Ok(out)
}

fn dissolve_node(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = DISSOLVE.one(inputs, 0)?;
    let group = p.s("group")?;
    let angle = p.f("angle")?;
    dissolve(&g, &group, angle)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use super::*;
    use crate::geometry::{Attr, Geometry};
    use crate::params::Params;

    fn total_polygon_area(geo: &Geometry) -> f64 {
        let p = geo.p();
        let vp = &geo.topology.vertex_point;
        let offs = &geo.topology.prim_offsets;
        let mut total = 0.0;
        for prim in 0..geo.nprims() {
            let s = offs[prim] as usize;
            let e = offs[prim + 1] as usize;
            if e - s < 3 {
                continue;
            }
            let p0 = [
                p[vp[s] as usize * 3] as f64,
                p[vp[s] as usize * 3 + 1] as f64,
                p[vp[s] as usize * 3 + 2] as f64,
            ];
            let mut c = [0.0; 3];
            for i in 1..(e - s - 1) {
                let v1 = [
                    p[vp[s + i] as usize * 3] as f64 - p0[0],
                    p[vp[s + i] as usize * 3 + 1] as f64 - p0[1],
                    p[vp[s + i] as usize * 3 + 2] as f64 - p0[2],
                ];
                let v2 = [
                    p[vp[s + i + 1] as usize * 3] as f64 - p0[0],
                    p[vp[s + i + 1] as usize * 3 + 1] as f64 - p0[1],
                    p[vp[s + i + 1] as usize * 3 + 2] as f64 - p0[2],
                ];
                let cr = [
                    v1[1] * v2[2] - v1[2] * v2[1],
                    v1[2] * v2[0] - v1[0] * v2[2],
                    v1[0] * v2[1] - v1[1] * v2[0],
                ];
                c[0] += cr[0];
                c[1] += cr[1];
                c[2] += cr[2];
            }
            total += 0.5 * (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt();
        }
        total
    }

    #[test]
    fn test_triangulated_box_becomes_six_quads() {
        let box_p = Params::resolve(
            super::super::generate::BOX.params,
            serde_json::json!({"size": [1.0, 2.0, 3.0]}).as_object(),
        )
        .unwrap();
        let box_geo = (super::super::generate::BOX.cook)(&[vec![]], &box_p).unwrap();
        let div_p = Params::resolve(super::super::model::DIVIDE.params, None).unwrap();
        let tri = (super::super::model::DIVIDE.cook)(&[vec![Arc::new(box_geo.clone())]], &div_p).unwrap();
        assert_eq!(tri.nprims(), 12);

        let out = dissolve(&tri, "", 1.0).unwrap();
        assert_eq!(out.nprims(), 6);
        for w in out.topology.prim_offsets.windows(2) {
            assert_eq!(w[1] - w[0], 4);
        }
        let tw = super::super::modeling_tools::twins(&out);
        assert!(tw.iter().all(Option::is_some));
        assert!((total_polygon_area(&out) - total_polygon_area(&box_geo)).abs() < 1e-5);
    }

    #[test]
    fn test_subdivided_faces_merge_back() {
        let box_p = Params::resolve(
            super::super::generate::BOX.params,
            serde_json::json!({"size": [1.0, 1.0, 1.0]}).as_object(),
        )
        .unwrap();
        let box_geo = (super::super::generate::BOX.cook)(&[vec![]], &box_p).unwrap();
        let sub_p = Params::resolve(
            super::super::subdivide::SUBDIVIDE.params,
            serde_json::json!({"depth": 1, "algorithm": "linear"}).as_object(),
        )
        .unwrap();
        let fine = (super::super::subdivide::SUBDIVIDE.cook)(&[vec![Arc::new(box_geo)]], &sub_p).unwrap();
        assert!(fine.nprims() > 6);

        let out = dissolve(&fine, "", 1.0).unwrap();
        assert_eq!(out.nprims(), 6);
        let tw = super::super::modeling_tools::twins(&out);
        assert!(tw.iter().all(Option::is_some));
        assert!((total_polygon_area(&out) - 6.0).abs() < 1e-5);
    }

    #[test]
    fn test_group_limits_the_merge_and_curved_surfaces_stay_in_patches() {
        let box_p = Params::resolve(
            super::super::generate::BOX.params,
            serde_json::json!({"size": [1.0, 1.0, 1.0]}).as_object(),
        )
        .unwrap();
        let box_geo = (super::super::generate::BOX.cook)(&[vec![]], &box_p).unwrap();
        let div_p = Params::resolve(super::super::model::DIVIDE.params, None).unwrap();
        let tri = (super::super::model::DIVIDE.cook)(&[vec![Arc::new(box_geo)]], &div_p).unwrap();

        let part = dissolve(&tri, "0-5", 1.0).unwrap();
        assert!(part.nprims() > 6);
        assert!(part.nprims() < 12);

        let sphere_p = Params::resolve(
            super::super::generate::SPHERE.params,
            serde_json::json!({"rows": 40, "columns": 64}).as_object(),
        )
        .unwrap();
        let sphere_geo = (super::super::generate::SPHERE.cook)(&[], &sphere_p).unwrap();
        let s = (super::super::model::DIVIDE.cook)(&[vec![Arc::new(sphere_geo.clone())]], &div_p).unwrap();

        let out = dissolve(&s, "", 6.0).unwrap();
        assert!(out.nprims() < s.nprims());
        assert!(out.nprims() > 100);
        let tw = super::super::modeling_tools::twins(&out);
        assert!(tw.iter().all(Option::is_some));
        let s_area = total_polygon_area(&sphere_geo);
        assert!((total_polygon_area(&out) - s_area).abs() < 0.05 * s_area);

        assert_eq!(dissolve(&tri, "", 0.0).unwrap().nprims(), 6);
    }

    #[test]
    fn test_attributes_follow_the_faces() {
        let box_p = Params::resolve(
            super::super::generate::BOX.params,
            serde_json::json!({"size": [1.0, 1.0, 1.0]}).as_object(),
        )
        .unwrap();
        let mut box_geo = (super::super::generate::BOX.cook)(&[vec![]], &box_p).unwrap();
        let id_vals: Vec<i32> = (0..box_geo.nprims()).map(|i| i as i32 * 10).collect();
        box_geo.prim.insert("id".into(), Arc::new(Attr::i32(1, id_vals)));

        let div_p = Params::resolve(super::super::model::DIVIDE.params, None).unwrap();
        let tri = (super::super::model::DIVIDE.cook)(&[vec![Arc::new(box_geo)]], &div_p).unwrap();

        let out = dissolve(&tri, "", 1.0).unwrap();
        let mut ids: Vec<i32> = match &out.prim.get("id").unwrap().data {
            crate::geometry::AttrData::I32(v) => v.clone(),
            _ => panic!("expected i32 id"),
        };
        ids.sort();
        assert_eq!(ids, vec![0, 10, 20, 30, 40, 50]);

        for key in ["uv", "N"] {
            if let Some(attr) = out.vertex.get(key) {
                assert_eq!(attr.rows(), out.topology.nvertices());
            }
        }
    }

    #[test]
    fn test_flat_grid_with_hole_keeps_its_area() {
        let grid_p = Params::resolve(
            super::super::generate::GRID.params,
            serde_json::json!({"columns": 7, "rows": 7, "size_x": 6.0, "size_y": 6.0}).as_object(),
        )
        .unwrap();
        let grid_geo = (super::super::generate::GRID.cook)(&[], &grid_p).unwrap();
        let div_p = Params::resolve(super::super::model::DIVIDE.params, None).unwrap();
        let g = (super::super::model::DIVIDE.cook)(&[vec![Arc::new(grid_geo)]], &div_p).unwrap();

        let blast_p = Params::resolve(
            super::super::group::BLAST.params,
            serde_json::json!({"group": "34-37"}).as_object(),
        )
        .unwrap();
        let centre = (super::super::group::BLAST.cook)(&[vec![Arc::new(g)]], &blast_p).unwrap();

        let out = dissolve(&centre, "", 1.0).unwrap();
        assert!(out.nprims() < centre.nprims());
        let centre_area = total_polygon_area(&centre);
        let out_area = total_polygon_area(&out);
        assert!((out_area - centre_area).abs() < 1e-4);
        assert!((out_area - 36.0 * (1.0 - 4.0 / 72.0)).abs() < 1e-4);
    }

    #[test]
    fn test_open_curves_are_refused() {
        let line_p = Params::resolve(
            crate::nodes::registry().iter().find(|spec| spec.name == "line").unwrap().params,
            serde_json::json!({"points": 5}).as_object(),
        )
        .unwrap();
        let line_geo = (crate::nodes::registry().iter().find(|spec| spec.name == "line").unwrap().cook)(&[], &line_p).unwrap();
        assert!(dissolve(&line_geo, "", 1.0).is_err());
    }
}
