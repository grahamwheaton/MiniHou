//! `recalc_normals` (Blender Recalculate Normals, `mesh.normals_make_consistent`): consistent winding per connected
//! component, then outside (or inside). Spec: docs/RUST_MESH_NODES.md.
use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use super::mesh_common::out_group;
use super::{model, Inputs, NodeSpec, GEO};
use crate::error::Result;
use crate::geometry::{Attr, AttrData, AttrTable, Geometry};
use crate::normals;
use crate::params::Params;

pub const RECALC_NORMALS: NodeSpec = NodeSpec { name: "recalc_normals", inputs: GEO, params: crate::params!(
    "group": ""; "inside": false; "output_group": false; "output_group_name": "flipped"), cook: recalc };

fn negate_rows(t: &mut AttrTable, name: &str, mask: &[bool]) {
    let Some(a) = t.get(name).filter(|a| a.rows() == mask.len()) else { return };
    let mut a = (**a).clone();
    let w = a.width;
    match &mut a.data {
        AttrData::F32(v) => v.chunks_mut(w).zip(mask).filter(|x| *x.1).for_each(|(r, _)| r.iter_mut().for_each(|x| *x = -*x)),
        AttrData::I32(v) => v.chunks_mut(w).zip(mask).filter(|x| *x.1).for_each(|(r, _)| r.iter_mut().for_each(|x| *x = -*x)),
    }
    t.insert(name.into(), Arc::new(a));
}

fn recalc(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = RECALC_NORMALS.one(inputs, 0)?;
    let inside = p.b("inside")?;
    model::polygons(&g, "recalc_normals")?;
    let np = g.nprims();
    let sel = if p.s("group")?.trim().is_empty() { vec![true; np] } else { model::prim_mask(&g, &p.s("group")?)? };
    let vp = &g.topology.vertex_point;
    let pos = model::positions(&g);
    // undirected edge -> faces using it, with whether the face runs it low -> high
    let mut edges: BTreeMap<(i32, i32), Vec<(usize, bool)>> = BTreeMap::new();
    for f in (0..np).filter(|&f| sel[f]) {
        let r = model::range(&g, f);
        for v in r.clone() {
            let (a, b) = (vp[v], vp[if v + 1 == r.end { r.start } else { v + 1 }]);
            if a != b { edges.entry((a.min(b), a.max(b))).or_default().push((f, a < b)); }
        }
    }
    let mut flip = vec![false; np];
    let mut comp = vec![usize::MAX; np];
    let mut ncomp = 0;
    let mut comps: Vec<Vec<usize>> = vec![];
    let nbrs = |f: usize| -> Vec<(usize, bool)> {
        // (neighbour, does it need the opposite flag relative to f) for manifold edges only
        let r = model::range(&g, f);
        let mut out = vec![];
        for v in r.clone() {
            let (a, b) = (vp[v], vp[if v + 1 == r.end { r.start } else { v + 1 }]);
            if a == b { continue; }
            let users = &edges[&(a.min(b), a.max(b))];
            if users.len() != 2 { continue; }
            let me_fwd = a < b;
            if let Some(&(o, o_fwd)) = users.iter().find(|u| u.0 != f) {
                // same original direction => the neighbour must be flipped relative to us
                out.push((o, o_fwd == me_fwd));
            }
        }
        out
    };
    for s in (0..np).filter(|&f| sel[f]) {
        if comp[s] != usize::MAX { continue; }
        comp[s] = ncomp;
        let mut members = vec![s];
        let mut q = VecDeque::from([s]);
        while let Some(f) = q.pop_front() {
            for (o, differ) in nbrs(f) {
                if comp[o] == usize::MAX {
                    comp[o] = ncomp;
                    flip[o] = flip[f] ^ differ;
                    members.push(o);
                    q.push_back(o);
                }
            }
        }
        comps.push(members);
        ncomp += 1;
    }
    // orient each component
    let fnorm = normals::prim_normals(g.p(), &g.topology); // Newell vector, original winding
    for members in &comps {
        let sign = |f: usize| if flip[f] { -1. } else { 1. };
        let mut center = [0.; 3];
        let mut cnt: f64 = 0.;
        for &f in members { for v in model::range(&g, f) { let q = pos[vp[v] as usize]; for c in 0..3 { center[c] += q[c]; } cnt += 1.; } }
        for c in center.iter_mut() { *c /= cnt.max(1.); }
        let closed = members.iter().all(|&f| {
            let r = model::range(&g, f);
            r.clone().all(|v| { let (a, b) = (vp[v], vp[if v + 1 == r.end { r.start } else { v + 1 }]); edges[&(a.min(b), a.max(b))].len() == 2 })
        });
        let mut want_out: Option<bool> = None; // true if the consistent orientation already points outward
        let mut scale: f64 = 0.;
        for &f in members { for v in model::range(&g, f) { for c in 0..3 { scale = f64::max(scale, (pos[vp[v] as usize][c] - center[c]).abs()); } } }
        let tiny = 1e-9 * scale.max(1e-12).powi(3);
        if closed {
            let vol: f64 = members.iter().map(|&f| {
                let q = pos[vp[model::range(&g, f).start] as usize];
                sign(f) * (0..3).map(|c| (q[c] - center[c]) * fnorm[f][c]).sum::<f64>() / 6.
            }).sum();
            if vol.abs() > tiny { want_out = Some(vol > 0.); }
        } else {
            // Blender's heuristic: the vertex farthest from the centre must have an outward-facing neighbourhood
            let mut far = (-1., usize::MAX);
            for &f in members { for v in model::range(&g, f) {
                let pt = vp[v] as usize;
                let d: f64 = (0..3).map(|c| (pos[pt][c] - center[c]).powi(2)).sum();
                if d > far.0 + 1e-12 || (d - far.0).abs() <= 1e-12 && pt < far.1 { far = (d, pt); }
            } }
            let mut acc = 0.;
            for &f in members {
                if model::range(&g, f).any(|v| vp[v] as usize == far.1) {
                    acc += sign(f) * (0..3).map(|c| (pos[far.1][c] - center[c]) * fnorm[f][c]).sum::<f64>();
                }
            }
            if acc.abs() > 1e-9 * scale.max(1e-12).powi(3) { want_out = Some(acc > 0.); }
        }
        let out_sign = match want_out {
            Some(o) => o != inside,
            None => {
                // ambiguous (flat or degenerate): keep the majority of the existing orientation, area-weighted
                let vote: f64 = members.iter().map(|&f| sign(f) * (fnorm[f][0].abs() + fnorm[f][1].abs() + fnorm[f][2].abs())).sum();
                vote >= 0. // true = keep consistent orientation as is
            }
        };
        if !out_sign { for &f in members { flip[f] = !flip[f]; } }
    }
    let mut out = if flip.iter().any(|&x| x) {
        let faces: Vec<Vec<usize>> = (0..np).map(|f| { let mut r: Vec<usize> = model::range(&g, f).collect(); if flip[f] { r.reverse(); } r }).collect();
        let mut o = model::rebuild(&g, &faces, &(0..np).collect::<Vec<_>>(), g.topology.closed.clone())?;
        // flipped corners: the vertex rows were gathered with the faces, so only the stored normals need to change
        let mut cf = vec![false; o.nvertices()];
        for f in (0..np).filter(|&f| flip[f]) { for v in model::range(&o, f) { cf[v] = true; } }
        negate_rows(&mut o.vertex, "N", &cf);
        negate_rows(&mut o.prim, "N", &flip);
        if o.point.get("N").is_some_and(|a| a.width == 3 && a.as_f32().is_some()) {
            let fresh = normals::point_normals(o.p(), &o.topology, o.npoints);
            let mut v = o.point["N"].as_f32().unwrap().to_vec();
            let mut touched = vec![false; o.npoints];
            for f in (0..np).filter(|&f| flip[f]) { for c in model::range(&o, f) { touched[o.topology.vertex_point[c] as usize] = true; } }
            for (i, t) in touched.iter().enumerate() { if *t { v[i * 3..i * 3 + 3].copy_from_slice(&fresh[i * 3..i * 3 + 3]); } }
            o.point.insert("N".into(), Arc::new(Attr::f32(3, v)));
        }
        o
    } else { (*g).clone() };
    out_group(&mut out, p, &flip)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::grid_fill::tests::consistent;

    fn cube(flips: &[usize]) -> Geometry {
        let p = [[-1., -1., -1.], [1., -1., -1.], [1., 1., -1.], [-1., 1., -1.], [-1., -1., 1.], [1., -1., 1.], [1., 1., 1.], [-1., 1., 1.]];
        // outward winding
        let f: [[i32; 4]; 6] = [[0, 3, 2, 1], [4, 5, 6, 7], [0, 1, 5, 4], [1, 2, 6, 5], [2, 3, 7, 6], [3, 0, 4, 7]];
        let mut vp = vec![];
        for (i, q) in f.iter().enumerate() { let mut q = q.to_vec(); if flips.contains(&i) { q.reverse(); } vp.extend(q); }
        Geometry::from_arrays(p.iter().flatten().copied().collect(), vp, (0..=6).map(|i| i * 4).collect(), None).unwrap()
    }
    fn run(g: Geometry, kv: serde_json::Value) -> Geometry {
        let p = Params::resolve(RECALC_NORMALS.params, kv.as_object()).unwrap();
        recalc(&[vec![Arc::new(g)]], &p).unwrap()
    }
    fn volume(g: &Geometry) -> f64 {
        let n = normals::prim_normals(g.p(), &g.topology);
        (0..g.nprims()).map(|f| { let q = g.topology.vertex_point[g.topology.prim_offsets[f] as usize] as usize * 3; (0..3).map(|c| g.p()[q + c] as f64 * n[f][c]).sum::<f64>() / 6. }).sum()
    }
    #[test]
    fn cube_with_random_flips_becomes_outward() {
        for flips in [vec![], vec![0], vec![1, 3, 4], vec![0, 1, 2, 3, 4, 5]] {
            let out = run(cube(&flips), serde_json::json!({}));
            assert!(consistent(&out));
            assert!((volume(&out) - 8.).abs() < 1e-4, "{flips:?} {}", volume(&out));
        }
        let inward = run(cube(&[]), serde_json::json!({"inside": true}));
        assert!((volume(&inward) + 8.).abs() < 1e-4);
    }
    #[test]
    fn output_group_marks_flipped_faces() {
        let out = run(cube(&[2, 5]), serde_json::json!({"output_group": true}));
        assert_eq!(out.prim["group_flipped"].as_i32().unwrap(), &[0, 0, 1, 0, 0, 1]);
    }
    #[test]
    fn two_components_one_inside_out() {
        let a = cube(&[]);
        let mut b = cube(&[0, 1, 2, 3, 4, 5]);
        b.set_p(b.p().iter().map(|x| x + 10.).collect());
        let mut vp = a.topology.vertex_point.clone();
        vp.extend(b.topology.vertex_point.iter().map(|x| x + 8));
        let mut offs = a.topology.prim_offsets.clone();
        offs.extend(b.topology.prim_offsets[1..].iter().map(|x| x + 24));
        let merged = Geometry::from_arrays([a.p(), b.p()].concat(), vp, offs, None).unwrap();
        let out = run(merged, serde_json::json!({}));
        assert!((volume(&out) - 16.).abs() < 1e-3);
    }
    #[test]
    fn open_plane_keeps_majority_orientation_and_group_limits_scope() {
        let plane = |down: bool| {
            let mut q = vec![0, 1, 4, 3, 1, 2, 5, 4];
            if down { q.chunks_mut(4).for_each(|c| c.reverse()); }
            Geometry::from_arrays(vec![0., 0., 0., 1., 0., 0., 2., 0., 0., 0., 1., 0., 1., 1., 0., 2., 1., 0.], q, vec![0, 4, 8], None).unwrap()
        };
        let up = run(plane(false), serde_json::json!({}));
        assert_eq!(up.topology.vertex_point, plane(false).topology.vertex_point);
        let down = run(plane(true), serde_json::json!({}));
        assert_eq!(down.topology.vertex_point, plane(true).topology.vertex_point);
        let mixed = Geometry::from_arrays(plane(false).p().to_vec(), vec![0, 1, 4, 3, 5, 4, 1, 2], vec![0, 4, 8], None).unwrap();
        let fixed = run(mixed, serde_json::json!({}));
        assert!(consistent(&fixed));
    }
    #[test]
    fn torus_is_made_outward_and_empty_is_fine() {
        let tp = Params::resolve(super::super::generate::TORUS.params, None).unwrap();
        let t = (super::super::generate::TORUS.cook)(&[vec![]], &tp).unwrap();
        let out = run(t.clone(), serde_json::json!({}));
        assert!(consistent(&out));
        assert!(volume(&out) > 0.);
        let rev = run(run(t, serde_json::json!({"inside": true})), serde_json::json!({}));
        assert!(volume(&rev) > 0.);
        assert_eq!(run(Geometry::points_only(vec![]).unwrap(), serde_json::json!({})).nprims(), 0);
    }
}
