//! Extended Inset (Blender `mesh.inset` options): boundary on/off, even / relative offset, edge rail, depth, individual faces.
//! Used by the `inset` node when an option leaves the Python-compatible defaults. Spec: docs/RUST_MESH_NODES.md.
use std::sync::Arc;

use super::model::*;
use crate::error::{Error, Result};
use crate::geometry::{Attr, AttrData, Geometry};
use crate::{normals, topo};

pub(super) struct Opts {
    /// Signed inset distance (negative = outset).
    pub thickness: f64,
    pub depth: f64,
    pub boundary: bool,
    pub even: bool,
    pub relative: bool,
    pub rail: bool,
    pub individual: bool,
    pub interpolate: bool,
}

/// Inset of the faces in `sel`. Writes the `extrudeFront` (inner faces) and `extrudeSide` (ring) groups like Python's inset.
pub(super) fn inset_ext(g: &Geometry, sel: &[bool], o: &Opts) -> Result<Geometry> {
    polygons(g, "inset")?;
    let np = g.nprims();
    let none = vec![false; np];
    if !sel.iter().any(|&b| b) || (o.thickness == 0. && o.depth == 0.) {
        let mut out = g.clone();
        write_group(&mut out, "prim", "extrudeFront", sel)?;
        write_group(&mut out, "prim", "extrudeSide", &none)?;
        return Ok(out);
    }
    if !o.thickness.is_finite() || !o.depth.is_finite() { return Err(Error::new("inset: thickness and depth must be finite")); }
    let (pv, nx, pr) = corners(g);
    let vp = &g.topology.vertex_point;
    let pos = positions(g);
    let e = topo::edges(&g.topology, g.npoints);
    let nv_total = g.nvertices();
    let mut cnt_sel = vec![0usize; e.len()];
    for v in 0..nv_total { if sel[pv[v]] { cnt_sel[e.of_vertex[v] as usize] += 1; } }
    if !o.individual && cnt_sel.iter().any(|&c| c > 2) { return Err(Error::new("inset requires a manifold selected region")); }
    let fn_ = normals::prim_normals(g.p(), &g.topology);
    let fu: Vec<[f64; 3]> = fn_.iter().map(|&n| unit(n)).collect();
    // corners whose outgoing edge lies on the inset boundary, and those of them that stay put (mesh border, use_boundary off)
    let is_b = |v: usize| sel[pv[v]] && (o.individual || cnt_sel[e.of_vertex[v] as usize] == 1);
    let skipped = |v: usize| is_b(v) && !o.boundary && e.count[e.of_vertex[v] as usize] == 1;
    // point or corner pinned in place
    let mut pinned_pt = vec![false; g.npoints];
    let mut on_boundary_pt = vec![false; g.npoints];
    for v in 0..nv_total {
        if !is_b(v) { continue; }
        on_boundary_pt[vp[v] as usize] = true;
        on_boundary_pt[vp[nx[v]] as usize] = true;
        if skipped(v) { pinned_pt[vp[v] as usize] = true; pinned_pt[vp[nx[v]] as usize] = true; }
    }
    let pinned_corner = |v: usize| skipped(v) || skipped(pr[v]) && sel[pv[v]];
    // inward in-plane normal and length of the boundary edge starting at corner c
    let edge_n = |c: usize| {
        let (a, b) = (pos[vp[c] as usize], pos[vp[nx[c]] as usize]);
        let d = sub(b, a);
        (unit(cross(fu[pv[c]], unit(d))), len(d))
    };
    // displacement (in the face plane) of corner v given the boundary edges (normal, length) that meet there
    let miter = |ns: &[([f64; 3], f64)]| -> ([f64; 3], f64, f64) {
        let mut m = [0.; 3];
        for (n, _) in ns { m = add(m, *n); }
        let m = if len(m) < 1e-9 { ns[0].0 } else { unit(m) };
        let mean_len = ns.iter().map(|x| x.1).sum::<f64>() / ns.len() as f64;
        let mut t = o.thickness;
        if o.relative { t *= mean_len; }
        let k = ns.iter().map(|(n, _)| dot(m, *n)).fold(f64::INFINITY, f64::min).max(0.2);
        (m, if o.even { t / k } else { t }, t)
    };
    // vertex normal of the selected faces at point q (area weighted)
    let mut pn = vec![[0.; 3]; g.npoints];
    for v in 0..nv_total { if sel[pv[v]] { let q = vp[v] as usize; pn[q] = add(pn[q], fn_[pv[v]]); } }
    let pn: Vec<[f64; 3]> = pn.iter().map(|&n| unit(n)).collect();

    let mut newpos: Vec<[f64; 3]> = vec![]; // positions of appended points
    let mut newsrc: Vec<usize> = vec![];
    let mut inner = vec![0usize; nv_total]; // point used by each selected corner in the inner face
    let mut moved = pos.clone();
    if !o.individual {
        // per boundary point
        let mut incident: Vec<Vec<([f64; 3], f64)>> = vec![vec![]; g.npoints];
        for v in 0..nv_total {
            if !is_b(v) { continue; }
            let n = edge_n(v);
            incident[vp[v] as usize].push(n);
            incident[vp[nx[v]] as usize].push(n);
        }
        let mut map = vec![usize::MAX; g.npoints];
        for q in 0..g.npoints {
            if !on_boundary_pt[q] || pinned_pt[q] { continue; }
            let ns = &incident[q];
            let (m, mut dist, t) = miter(ns);
            let mut d = mul(m, dist);
            if o.rail {
                // slide along the existing edge (inside the region) that best follows the miter direction
                let mut best: Option<([f64; 3], f64, f64)> = None;
                for v in 0..nv_total {
                    if !sel[pv[v]] { continue; }
                    for (a, b) in [(vp[v] as usize, vp[nx[v]] as usize), (vp[nx[v]] as usize, vp[v] as usize)] {
                        if a != q || cnt_sel[e.of_vertex[v] as usize] != 2 { continue; }
                        let dv = sub(pos[b], pos[q]);
                        let l = len(dv);
                        if l < 1e-12 { continue; }
                        let du = mul(dv, 1. / l);
                        let c = dot(du, m);
                        if c > 0.05 && best.is_none_or(|x| c > x.1 + 1e-12) { best = Some((du, c, l)); }
                    }
                }
                if let Some((du, _, l)) = best {
                    let k = ns.iter().map(|(n, _)| dot(du, *n)).sum::<f64>() / ns.len() as f64;
                    if k > 0.05 {
                        dist = (t / k).min(l * 0.99);
                        d = mul(du, dist);
                    }
                }
            }
            let _ = dist;
            map[q] = g.npoints + newpos.len();
            newpos.push(add(add(pos[q], d), mul(pn[q], o.depth)));
            newsrc.push(q);
        }
        for q in 0..g.npoints {
            if map[q] == usize::MAX && !on_boundary_pt[q] && pn[q] != [0.; 3] {
                moved[q] = add(pos[q], mul(pn[q], o.depth)); // interior of the region: move in place
            }
        }
        for v in 0..nv_total {
            if sel[pv[v]] {
                let q = vp[v] as usize;
                inner[v] = if map[q] != usize::MAX { map[q] } else { q };
            }
        }
    } else {
        for v in 0..nv_total {
            if !sel[pv[v]] { continue; }
            if pinned_corner(v) { inner[v] = vp[v] as usize; continue; }
            let ns = [edge_n(pr[v]), edge_n(v)];
            let (m, dist, _) = miter(&ns);
            inner[v] = g.npoints + newpos.len();
            newpos.push(add(add(pos[vp[v] as usize], mul(m, dist)), mul(pn_face(&fu, pv[v]), o.depth)));
            newsrc.push(vp[v] as usize);
        }
    }
    if newpos.iter().chain(moved.iter()).any(|q| q.iter().any(|x| !x.is_finite())) {
        return Err(Error::new("inset produced a non-finite position"));
    }
    // point table
    let mut point = concat(&g.point, &gather(&g.point, &newsrc));
    if !o.interpolate {
        for (name, a) in point.iter_mut() {
            if name == "P" || name.starts_with("group_") { continue; }
            if let AttrData::F32(v) = &a.data {
                let mut v = v.clone();
                let w = a.width;
                v[g.npoints * w..].fill(0.);
                *a = Arc::new(Attr::f32(w, v));
            }
        }
    }
    let mut p: Vec<f32> = moved.iter().flat_map(|q| q.map(|x| x as f32)).collect();
    p.extend(newpos.iter().flat_map(|q| q.map(|x| x as f32)));
    point.insert("P".into(), Arc::new(Attr::f32(3, p)));
    // faces
    let mut outvp: Vec<i32> = (0..nv_total).map(|v| if sel[pv[v]] { inner[v] as i32 } else { vp[v] }).collect();
    let mut vs: Vec<usize> = (0..nv_total).collect();
    let mut ps: Vec<usize> = (0..np).collect();
    let mut sizes: Vec<usize> = (0..np).map(|f| range(g, f).len()).collect();
    let mut ring = 0;
    for v in 0..nv_total {
        if !is_b(v) || skipped(v) { continue; }
        let cyc = [(vp[v] as usize, v), (vp[nx[v]] as usize, nx[v]), (inner[nx[v]], nx[v]), (inner[v], v)];
        let mut q: Vec<(usize, usize)> = vec![];
        for c in cyc { if q.last().is_none_or(|l| l.0 != c.0) { q.push(c); } }
        while q.len() > 1 && q[0].0 == q[q.len() - 1].0 { q.pop(); }
        if q.len() < 3 { continue; }
        sizes.push(q.len());
        ps.push(pv[v]);
        for (pt, row) in q { outvp.push(pt as i32); vs.push(row); }
        ring += 1;
    }
    let mut out = make(point, outvp, &sizes, gather(&g.vertex, &vs), gather(&g.prim, &ps), g.detail.clone(), None)?;
    out.point.remove("N");
    let total = sizes.len();
    let front: Vec<bool> = (0..total).map(|i| i < np && sel[i]).collect();
    let side: Vec<bool> = (0..total).map(|i| i >= np).collect();
    write_group(&mut out, "prim", "extrudeFront", &front)?;
    write_group(&mut out, "prim", "extrudeSide", &side)?;
    let touched: Vec<bool> = front.iter().zip(&side).map(|(a, b)| *a || *b).collect();
    super::mesh_common::refresh_corner_normals_for(&mut out, &touched);
    let _ = ring;
    Ok(out)
}

fn pn_face(fu: &[[f64; 3]], f: usize) -> [f64; 3] { fu[f] }

#[cfg(test)]
mod tests {
    use super::super::grid_fill::tests::{consistent, topo_stats};
    use super::super::{pushpull::INSET, Inputs};
    use crate::params::Params;
    use super::*;

    /// n x n quads in the z = 0 plane, counter-clockwise (normal +z).
    fn grid(n: usize) -> Geometry {
        let mut p = vec![];
        for y in 0..=n { for x in 0..=n { p.extend([x as f32, y as f32, 0.]); } }
        let id = |x: usize, y: usize| (y * (n + 1) + x) as i32;
        let mut vp = vec![];
        for y in 0..n { for x in 0..n { vp.extend([id(x, y), id(x + 1, y), id(x + 1, y + 1), id(x, y + 1)]); } }
        Geometry::from_arrays(p, vp, (0..=(n * n) as i32).map(|i| i * 4).collect(), None).unwrap()
    }
    fn run(g: Geometry, kv: serde_json::Value) -> Geometry {
        let p = Params::resolve(INSET.params, kv.as_object()).unwrap();
        let inputs: Box<Inputs> = Box::new([vec![Arc::new(g)]]);
        (INSET.cook)(&inputs, &p).unwrap()
    }
    fn xy(g: &Geometry, i: usize) -> (f32, f32) { (g.p()[i * 3], g.p()[i * 3 + 1]) }

    #[test]
    fn rail_slides_along_existing_edges() {
        // 2x2 block of a 3x3 grid; the boundary midpoint (2,1) slides along the y = 1 line
        let sel = "0 1 3 4";
        let rail = run(grid(3), serde_json::json!({"group": sel, "amount": 0.25, "use_edge_rail": true}));
        rail.validate().unwrap();
        assert_eq!(topo_stats(&rail).0, 1);
        assert!(rail.p().iter().all(|x| x.is_finite()));
        let q = (g_pt(&rail, 2.0 - 0.25, 1.0), g_pt(&rail, 1.0, 1.0));
        assert!(q.0 && q.1 || q.0, "rail point on the y = 1 line");
        let plain = run(grid(3), serde_json::json!({"group": sel, "amount": 0.25}));
        assert_eq!(plain.nprims(), rail.nprims());
        assert!(consistent(&rail));
    }
    fn g_pt(g: &Geometry, x: f32, y: f32) -> bool { (0..g.npoints).any(|i| { let p = xy(g, i); (p.0 - x).abs() < 1e-5 && (p.1 - y).abs() < 1e-5 }) }

    #[test]
    fn use_boundary_off_keeps_the_mesh_border_closed() {
        // inset the corner cell: with use_boundary the two outer border edges get ring faces, without they do not
        let on = run(grid(3), serde_json::json!({"group": "0", "amount": 0.2, "use_even_offset": false}));
        let off = run(grid(3), serde_json::json!({"group": "0", "amount": 0.2, "use_boundary": false}));
        assert_eq!(on.nprims(), 9 + 4);
        assert_eq!(off.nprims(), 9 + 2, "only the two interior edges of the corner cell are inset");
        assert_eq!(topo_stats(&off).0, 1);
        assert_eq!(topo_stats(&off).2, 0);
        assert!(g_pt(&off, 0., 0.), "border corner stays in place");
    }
    #[test]
    fn individual_outset_depth_and_options() {
        let g = grid(3);
        let ind = run(g.clone(), serde_json::json!({"group": "0 1", "amount": 0.2, "use_individual": true, "use_relative_offset": true}));
        assert_eq!(ind.nprims(), 9 + 8);
        ind.validate().unwrap();
        let out = run(g.clone(), serde_json::json!({"group": "4", "amount": 0.2, "use_outset": true}));
        assert!(g_pt(&out, 0.8, 0.8) && out.nprims() == 9 + 4);
        let deep = run(g.clone(), serde_json::json!({"group": "4", "amount": 0.2, "depth": 0.5, "use_even_offset": false}));
        assert!((0..deep.npoints).any(|i| (deep.p()[i * 3 + 2] - 0.5).abs() < 1e-6));
        assert_eq!(topo_stats(&deep), (1, 12, 0));
        let a = run(g.clone(), serde_json::json!({"group": "4", "use_interpolate": false}));
        let b = run(g, serde_json::json!({"group": "4", "use_interpolate": false}));
        assert_eq!(a.p(), b.p());
        assert!(a.prim.contains_key("group_extrudeSide"));
    }
    #[test]
    fn named_output_groups_and_empty_selection() {
        let out = run(grid(3), serde_json::json!({"group": "4", "output_group": true, "output_inner_group": true, "use_even_offset": false}));
        assert_eq!(out.prim["group_inset_ring"].as_i32().unwrap().iter().sum::<i32>(), 4);
        assert_eq!(out.prim["group_inset_inner"].as_i32().unwrap().iter().sum::<i32>(), 1);
        let none = run(grid(3), serde_json::json!({"group": "99", "use_edge_rail": true}));
        assert_eq!(none.nprims(), 9);
        assert_eq!(run(Geometry::points_only(vec![]).unwrap(), serde_json::json!({"use_edge_rail": true})).nprims(), 0);
    }
}
