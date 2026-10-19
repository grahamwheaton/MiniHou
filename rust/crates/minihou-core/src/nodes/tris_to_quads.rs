//! `tris_to_quads` (Blender Tris to Quads, `mesh.tris_convert_to_quads`): merges adjacent triangle pairs into quads, best
//! pairs first. Spec: docs/RUST_MESH_NODES.md.
use std::collections::BTreeMap;

use super::mesh_common::out_group;
use super::{edge_tools, model, Inputs, NodeSpec, GEO};
use crate::error::{Error, Result};
use crate::geometry::Geometry;
use crate::params::Params;

pub const TRIS_TO_QUADS: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, name: "tris_to_quads", inputs: GEO, params: crate::params!(
    "group": ""; "face_threshold": 40.0; "shape_threshold": 40.0; "uvs": false; "vcols": false; "seam": false;
    "sharp": false; "materials": false; "output_group": true; "output_group_name": "quads"), cook: tris_to_quads };

type V3 = [f64; 3];
fn sub(a: V3, b: V3) -> V3 { [a[0] - b[0], a[1] - b[1], a[2] - b[2]] }
fn dot(a: V3, b: V3) -> f64 { a[0] * b[0] + a[1] * b[1] + a[2] * b[2] }
fn cross(a: V3, b: V3) -> V3 { [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]] }
fn norm(a: V3) -> f64 { dot(a, a).sqrt() }

struct Pair { score: f64, edge: (i32, i32), a: usize, b: usize, quad: [usize; 4], src: [usize; 4] }

/// True when attribute rows `x` and `y` (corners) are equal within a small tolerance; NaN rows compare equal to NaN.
fn same_row(a: &crate::geometry::Attr, x: usize, y: usize) -> bool {
    let w = a.width;
    match (a.as_f32(), a.as_i32()) {
        (Some(v), _) => (0..w).all(|c| { let (p, q) = (v[x * w + c], v[y * w + c]); (p - q).abs() <= 1e-6 || p.is_nan() && q.is_nan() }),
        (_, Some(v)) => (0..w).all(|c| v[x * w + c] == v[y * w + c]),
        _ => true,
    }
}

fn tris_to_quads(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = TRIS_TO_QUADS.one(inputs, 0)?;
    let (face_t, shape_t) = (p.f("face_threshold")?, p.f("shape_threshold")?);
    if !(0.0..=180.0).contains(&face_t) || !(0.0..=180.0).contains(&shape_t) {
        return Err(Error::new("tris_to_quads: face_threshold and shape_threshold must be between 0 and 180 degrees"));
    }
    let (uvs, vcols, seam, sharp, mats) = (p.b("uvs")?, p.b("vcols")?, p.b("seam")?, p.b("sharp")?, p.b("materials")?);
    model::polygons(&g, "tris_to_quads")?;
    let np = g.nprims();
    let sel = if p.s("group")?.trim().is_empty() { vec![true; np] } else { model::prim_mask(&g, &p.s("group")?)? };
    let vp = &g.topology.vertex_point;
    let pos = model::positions(&g);
    let seam_v = if seam { Some(edge_tools::values(&g, "edge_group_seam")?) } else { None };
    let sharp_v = if sharp { Some(edge_tools::values(&g, "edge_group_sharp")?) } else { None };
    let e = crate::topo::edges(&g.topology, g.npoints);
    let mat = g.prim.get("material_index").and_then(|a| a.as_i32()).map(<[i32]>::to_vec);
    let uv_attrs: Vec<_> = g.vertex.iter().filter(|(n, a)| n.starts_with("uv") && a.as_f32().is_some()).map(|(_, a)| a.clone()).collect();
    let cd = if vcols { g.vertex.get("Cd").cloned() } else { None };
    let tri = |f: usize| model::range(&g, f).len() == 3;
    // shared edge -> (face, corner) users
    let mut users: BTreeMap<(i32, i32), Vec<(usize, usize)>> = BTreeMap::new();
    for f in (0..np).filter(|&f| sel[f] && tri(f)) {
        let r = model::range(&g, f);
        for v in r.clone() {
            let (a, b) = (vp[v], vp[if v + 1 == r.end { r.start } else { v + 1 }]);
            users.entry((a.min(b), a.max(b))).or_default().push((f, v));
        }
    }
    let nrm: Vec<V3> = crate::normals::prim_normals(g.p(), &g.topology);
    let mut pairs: Vec<Pair> = vec![];
    for (&(lo, hi), u) in &users {
        // the edge must be manifold overall (exactly two faces in the whole mesh) and both users selected triangles
        let id = e.of_vertex[u[0].1];
        if u.len() != 2 || id < 0 || e.count[id as usize] != 2 { continue; }
        let ((fa, va), (fb, vb)) = if u[0].0 < u[1].0 { (u[0], u[1]) } else { (u[1], u[0]) };
        let next = |f: usize, v: usize| { let r = model::range(&g, f); if v + 1 == r.end { r.start } else { v + 1 } };
        let (a_from, a_to) = (vp[va], vp[next(fa, va)]);
        if a_from == vp[vb] { continue; } // same direction: inconsistent winding, leave alone
        let _ = (lo, hi);
        // A runs a->b, B runs b->a: quad [b, c, a, d] with corners A.b, A.c, B.a, B.d
        let (cb, cc) = (next(fa, va), next(fa, next(fa, va)));
        let (ca, cd_) = (vb, next(fb, vb)); // B: b->a starts at corner of b; so B's corner of a is the next one
        let (b_corner_a, b_corner_d) = (cd_, next(fb, cd_));
        let _ = ca;
        let quad_corners = [cb, cc, b_corner_a, b_corner_d];
        let quad: [usize; 4] = quad_corners.map(|c| vp[c] as usize);
        debug_assert_eq!(quad[0], a_to as usize);
        if quad.iter().enumerate().any(|(i, q)| quad[..i].contains(q)) { continue; }
        // compare flags
        if mats && mat.as_ref().is_some_and(|m| m[fa] != m[fb]) { continue; }
        let (a_ab, a_ba) = (va, next(fa, va)); // A corners of a and b
        let (b_ba, b_ab) = (vb, next(fb, vb)); // B corners of b and a
        let same_attr = |a: &crate::geometry::Attr| same_row(a, a_ab, b_ab) && same_row(a, a_ba, b_ba);
        if uvs && uv_attrs.iter().any(|a| !same_attr(a)) { continue; }
        if cd.as_ref().is_some_and(|a| !same_attr(a)) { continue; }
        if seam_v.as_ref().is_some_and(|s| s[id as usize] != 0.) || sharp_v.as_ref().is_some_and(|s| s[id as usize] != 0.) { continue; }
        // geometry: planarity, convexity, shape
        let q = quad.map(|i| pos[i]);
        let (na, nb) = (nrm[fa], nrm[fb]);
        let (la, lb) = (norm(na), norm(nb));
        if la < 1e-20 || lb < 1e-20 { continue; }
        let planar = (dot(na, nb) / (la * lb)).clamp(-1., 1.).acos().to_degrees();
        if planar > face_t + 1e-9 { continue; }
        let n = [na[0] + nb[0], na[1] + nb[1], na[2] + nb[2]];
        let mut shape = 0f64;
        let mut ok = true;
        for i in 0..4 {
            let (prev, cur, nxt) = (q[(i + 3) % 4], q[i], q[(i + 1) % 4]);
            let (u, w) = (sub(prev, cur), sub(nxt, cur));
            let (lu, lw) = (norm(u), norm(w));
            if lu < 1e-20 || lw < 1e-20 { ok = false; break; }
            // convex: turn direction agrees with the face normal
            if dot(cross(sub(cur, prev), sub(nxt, cur)), n) <= 0. { ok = false; break; }
            let ang = (dot(u, w) / (lu * lw)).clamp(-1., 1.).acos().to_degrees();
            shape = shape.max((ang - 90.).abs());
        }
        if !ok || shape > shape_t + 1e-9 { continue; }
        pairs.push(Pair { score: planar + shape, edge: (lo, hi), a: fa, b: fb, quad, src: quad_corners });
    }
    pairs.sort_by(|x, y| x.score.total_cmp(&y.score).then(x.edge.cmp(&y.edge)));
    let mut used = vec![false; np];
    let mut merged: BTreeMap<usize, Vec<usize>> = BTreeMap::new(); // lower face -> quad corners
    let mut dropped = vec![false; np];
    for pr in &pairs {
        if used[pr.a] || used[pr.b] { continue; }
        used[pr.a] = true;
        used[pr.b] = true;
        dropped[pr.b] = true;
        merged.insert(pr.a, pr.src.to_vec());
        let _ = pr.quad;
    }
    if merged.is_empty() {
        let mut out = (*g).clone();
        out_group(&mut out, p, &vec![false; np])?;
        return Ok(out);
    }
    let mut faces = vec![];
    let mut sources = vec![];
    let mut mask = vec![];
    for f in 0..np {
        if dropped[f] { continue; }
        match merged.get(&f) {
            Some(c) => { faces.push(c.clone()); mask.push(true); }
            None => { faces.push(model::range(&g, f).collect()); mask.push(false); }
        }
        sources.push(f);
    }
    let mut out = model::rebuild(&g, &faces, &sources, None)?;
    out.detail = g.detail.clone();
    super::mesh_common::refresh_corner_normals_for(&mut out, &mask);
    out_group(&mut out, p, &mask)?;
    out.validate()?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn tri_grid(n: usize) -> Geometry {
        let mut p = vec![];
        for y in 0..=n { for x in 0..=n { p.extend([x as f32, y as f32, 0.]); } }
        let id = |x: usize, y: usize| (y * (n + 1) + x) as i32;
        let mut vp = vec![];
        for y in 0..n { for x in 0..n {
            vp.extend([id(x, y), id(x + 1, y), id(x + 1, y + 1)]);
            vp.extend([id(x, y), id(x + 1, y + 1), id(x, y + 1)]);
        } }
        Geometry::from_arrays(p, vp, (0..=(2 * n * n) as i32).map(|i| i * 3).collect(), None).unwrap()
    }
    fn run(g: Geometry, kv: serde_json::Value) -> Geometry {
        let p = Params::resolve(TRIS_TO_QUADS.params, kv.as_object()).unwrap();
        tris_to_quads(&[vec![Arc::new(g)]], &p).unwrap()
    }
    #[test]
    fn triangulated_grid_becomes_all_quads() {
        let g = tri_grid(4);
        let out = run(g.clone(), serde_json::json!({}));
        assert_eq!(out.nprims(), 16);
        assert!(out.topology.prim_offsets.windows(2).all(|w| w[1] - w[0] == 4));
        assert_eq!(out.prim["group_quads"].as_i32().unwrap().iter().sum::<i32>(), 16);
        assert_eq!(super::super::grid_fill::tests::topo_stats(&out), (1, 16, 0));
        assert!(super::super::grid_fill::tests::consistent(&out));
        assert_eq!(out.topology, run(g, serde_json::json!({})).topology);
    }
    #[test]
    fn triangulated_cube_gives_six_quads() {
        let p = [[-1., -1., -1.], [1., -1., -1.], [1., 1., -1.], [-1., 1., -1.], [-1., -1., 1.], [1., -1., 1.], [1., 1., 1.], [-1., 1., 1.]];
        let f: [[i32; 4]; 6] = [[0, 3, 2, 1], [4, 5, 6, 7], [0, 1, 5, 4], [1, 2, 6, 5], [2, 3, 7, 6], [3, 0, 4, 7]];
        let vp: Vec<i32> = f.iter().flat_map(|q| [q[0], q[1], q[2], q[0], q[2], q[3]]).collect();
        let g = Geometry::from_arrays(p.iter().flatten().copied().collect(), vp, (0..=12).map(|i| i * 3).collect(), None).unwrap();
        let out = run(g, serde_json::json!({}));
        assert_eq!(out.nprims(), 6);
        assert_eq!(super::super::grid_fill::tests::topo_stats(&out), (2, 0, 0));
    }
    #[test]
    fn thresholds_group_and_empty() {
        let fold = |h: f32| Geometry::from_arrays(vec![0., 0., 0., 1., 0., 0., 1., 1., 0., 0., 1., h], vec![0, 1, 2, 0, 2, 3], vec![0, 3, 6], None).unwrap();
        assert_eq!(run(fold(0.), serde_json::json!({})).nprims(), 1);
        assert_eq!(run(fold(1.), serde_json::json!({})).nprims(), 2, "54 degree fold exceeds face_threshold 40");
        assert_eq!(run(fold(1.), serde_json::json!({"face_threshold": 90, "shape_threshold": 180})).nprims(), 1);
        assert_eq!(run(tri_grid(2), serde_json::json!({"group": "0"})).nprims(), 8, "a single triangle has no partner in the group");
        assert_eq!(run(Geometry::points_only(vec![]).unwrap(), serde_json::json!({})).nprims(), 0);
        // concave pair: the shared diagonal lies outside the quad
        let concave = Geometry::from_arrays(vec![0., 0., 0., 2., 0., 0., 0.5, 0.5, 0., 0., 2., 0.], vec![0, 1, 2, 0, 2, 3], vec![0, 3, 6], None).unwrap();
        assert_eq!(run(concave, serde_json::json!({"face_threshold": 180, "shape_threshold": 180})).nprims(), 2);
    }
    #[test]
    fn uv_and_material_boundaries_are_respected() {
        use crate::geometry::Attr;
        let mut g = Geometry::from_arrays(vec![0., 0., 0., 1., 0., 0., 1., 1., 0., 0., 1., 0.], vec![0, 1, 2, 0, 2, 3], vec![0, 3, 6], None).unwrap();
        g.vertex.insert("uv".into(), Arc::new(Attr::f32(2, vec![0., 0., 1., 0., 1., 1., 0.5, 0.5, 1., 1., 0., 1.])));
        g.prim.insert("material_index".into(), Arc::new(Attr::i32(1, vec![0, 1])));
        assert_eq!(run(g.clone(), serde_json::json!({})).nprims(), 1);
        assert_eq!(run(g.clone(), serde_json::json!({"uvs": true})).nprims(), 2);
        assert_eq!(run(g.clone(), serde_json::json!({"materials": true})).nprims(), 2);
        let merged = run(g, serde_json::json!({}));
        merged.validate().unwrap();
        assert!(merged.p().iter().all(|x| x.is_finite()));
    }
}
