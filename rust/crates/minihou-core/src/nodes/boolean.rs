//! Polygon Boolean node, including source polygon reconstruction and attributes.
use super::{InputSpec, Inputs, NodeSpec, model::*};
use crate::{
    boolean_kernel as k,
    error::{Error, Result},
    geometry::{Attr, AttrData, AttrTable, Geometry},
    normals,
    params::Params,
    topo,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
pub const BOOLEAN: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "boolean",
    inputs: &[
        InputSpec {
            name: "a",
            multi: false,
            optional: false,
        },
        InputSpec {
            name: "b",
            multi: false,
            optional: false,
        },
    ],
    params: crate::params!("operation":"union";"outgrp_from_a":true;"outgrp_from_a_name":"from_a";"outgrp_from_b":true;"outgrp_from_b_name":"from_b";"outgrp_seam":false;"outgrp_seam_name":"intersection";"outgrp_seampoints":false;"outgrp_seampoints_name":"intersectionpoints"),
    cook: cook,
};
fn cook(i: &Inputs, p: &Params) -> Result<Geometry> {
    boolean(
        BOOLEAN.one(i, 0)?.as_ref(),
        BOOLEAN.one(i, 1)?.as_ref(),
        &p.e("operation", &["union", "subtract", "intersect"])?,
        Some(p),
    )
}
/// (point triples, corner triples, source primitives), in Python _triangles order.
pub(super) fn triangles(g: &Geometry) -> (Vec<[usize; 3]>, Vec<[usize; 3]>, Vec<usize>) {
    let ns = normals::prim_normals(g.p(), &g.topology);
    let pos = positions(g);
    let vp = &g.topology.vertex_point;
    let mut tv = vec![];
    let mut tp = vec![];
    let mut ears = vec![];
    for (p, &n) in ns.iter().enumerate() {
        let r = range(g, p);
        let m = r.len();
        if m < 3 {
            continue;
        }
        let q: Vec<_> = r.clone().map(|v| pos[vp[v] as usize]).collect();
        let bad = (0..m).any(|i| {
            let a = sub(q[i], q[(i + m - 1) % m]);
            let b = sub(q[(i + 1) % m], q[i]);
            dot(cross(a, b), n) <= 1e-9 * len(a) * len(b) * len(n)
        });
        if m > 3 && bad {
            ears.push((p, r, q, n));
        } else {
            for i in 1..m - 1 {
                tv.push([r.start, r.start + i, r.start + i + 1]);
                tp.push(p);
            }
        }
    }
    for (p, r, q, n) in ears {
        let (u, w) = k::basis(k::unit(n));
        let xy: Vec<_> = q.iter().map(|&p| [dot(p, u), dot(p, w)]).collect();
        for t in k::earclip(&(0..q.len()).collect::<Vec<_>>(), &xy) {
            tv.push(t.map(|i| i + r.start));
            tp.push(p);
        }
    }
    (
        tv.iter().map(|t| t.map(|v| vp[v] as usize)).collect(),
        tv,
        tp,
    )
}
fn planar(g: &Geometry) -> Vec<bool> {
    let pos = positions(g);
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for p in &pos {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    let size = (0..3).map(|k| hi[k] - lo[k]).fold(0., f64::max).max(1e-12);
    let ns = normals::prim_normals(g.p(), &g.topology);
    (0..g.nprims())
        .map(|p| {
            let q: Vec<_> = range(g, p)
                .map(|v| pos[g.topology.vertex_point[v] as usize])
                .collect();
            let c = mul(
                q.iter().copied().fold([0.; 3], add),
                1. / q.len().max(1) as f64,
            );
            let n = k::unit(ns[p]);
            q.iter().all(|&x| dot(sub(x, c), n).abs() <= 1e-6 * size)
        })
        .collect()
}
pub(super) fn combine(parts: &[(AttrTable, usize)]) -> Result<AttrTable> {
    let mut names = BTreeSet::new();
    for (t, _) in parts {
        names.extend(t.keys().cloned());
    }
    let mut out = AttrTable::new();
    for name in names {
        let reference = parts.iter().find_map(|(t, _)| t.get(&name)).unwrap();
        let w = reference.width;
        let have: Vec<_> = parts.iter().filter_map(|(t, _)| t.get(&name)).collect();
        if have.iter().any(|a| a.width != w) {
            return Err(Error::new(format!(
                "attribute {name:?} has mismatched shapes across inputs"
            )));
        }
        // Geometry stores f32/i32. Mixed numeric inputs promote to the float storage type.
        let floats = have.iter().any(|a| matches!(a.data, AttrData::F32(_)));
        let n = parts.iter().map(|(_, n)| n * w).sum();
        let mut a = if floats {
            Attr::f32(w, Vec::with_capacity(n))
        } else {
            Attr::i32(w, Vec::with_capacity(n))
        };
        for (t, count) in parts {
            match &mut a.data {
                AttrData::F32(dst) => {
                    if let Some(src) = t.get(&name) {
                        match &src.data {
                            AttrData::F32(v) => dst.extend(v),
                            AttrData::I32(v) => dst.extend(v.iter().map(|&x| x as f32)),
                        }
                    } else {
                        dst.resize(dst.len() + count * w, 0.);
                    }
                }
                AttrData::I32(dst) => {
                    if let Some(src) = t.get(&name) {
                        if let AttrData::I32(v) = &src.data {
                            dst.extend(v);
                        }
                    } else {
                        dst.resize(dst.len() + count * w, 0);
                    }
                }
            }
        }
        out.insert(name, Arc::new(a));
    }
    Ok(out)
}
pub(super) fn combine_vertices(parts: &[(AttrTable, usize)]) -> Result<AttrTable> {
    let mut out = combine(parts)?;
    for (name, attr) in &mut out {
        if matches!(name.as_str(), "N" | "Cd" | "P") || !matches!(attr.width, 2 | 3) { continue; }
        let Some(values) = attr.as_f32() else { continue; };
        let mut values = values.to_vec();
        let mut start = 0;
        for (table, count) in parts {
            if !table.contains_key(name) {
                values[start..start + count * attr.width].fill(f32::NAN);
            }
            start += count * attr.width;
        }
        *attr = Arc::new(Attr::f32(attr.width, values));
    }
    Ok(out)
}
fn blend(table: &AttrTable, idx: &[[usize; 3]], weights: &[[f64; 3]]) -> AttrTable {
    table
        .iter()
        .map(|(name, a)| {
            let w = a.width;
            let data = match &a.data {
                AttrData::F32(v) => AttrData::F32(
                    idx.iter()
                        .zip(weights)
                        .flat_map(|(ix, b)| {
                            (0..w).map(move |c| {
                                (0..3).map(|k| v[ix[k] * w + c] as f64 * b[k]).sum::<f64>() as f32
                            })
                        })
                        .collect(),
                ),
                AttrData::I32(v) => AttrData::I32(
                    idx.iter()
                        .zip(weights)
                        .flat_map(|(ix, b)| {
                            let mut best = 0;
                            for k in 1..3 {
                                if b[k] > b[best] {
                                    best = k;
                                }
                            }
                            (0..w).map(move |c| v[ix[best] * w + c])
                        })
                        .collect(),
                ),
            };
            (name.clone(), Arc::new(Attr { width: w, data }))
        })
        .collect()
}
/// Merge connected pieces of one source primitive; retain corner indices for UVs.
fn merge_pieces(
    v: &[usize],
    sizes: &[usize],
    group: &[usize],
    p: &[[f64; 3]],
) -> Vec<(Vec<usize>, usize)> {
    let mut first = vec![];
    let mut poly_of = vec![];
    let mut next = vec![];
    let mut f = 0;
    for (q, &n) in sizes.iter().enumerate() {
        first.push(f);
        for k in 0..n {
            poly_of.push(q);
            next.push(f + (k + 1) % n);
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
fn outline(loops: Vec<Vec<usize>>, v: &[usize], p: &[[f64; 3]]) -> Option<Vec<Vec<usize>>> {
    if loops.len() == 1 {
        return Some(loops);
    }
    let mut n = [0.; 3];
    for lp in &loops {
        for k in 0..lp.len() {
            n = add(n, cross(p[v[lp[k]]], p[v[lp[(k + 1) % lp.len()]]]));
        }
    }
    if len(n) < 1e-300 {
        return None;
    }
    let (u, w) = k::basis(k::unit(n));
    let xy: Vec<_> = v.iter().map(|&i| [dot(p[i], u), dot(p[i], w)]).collect();
    let areas: Vec<_> = loops.iter().map(|q| k::area2(q, &xy)).collect();
    let outer = (0..areas.len()).max_by(|&a, &b| areas[a].total_cmp(&areas[b]).then(b.cmp(&a)))?;
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
pub(super) fn unify(a: &Geometry, b: &Geometry) -> Result<(Geometry, Geometry)> {
    let mut gs = super::material_tables::reconcile(&[a.clone(), b.clone()])?;
    super::model::promote_corner_uv_sets(&mut gs);
    let b = gs.pop().unwrap();
    Ok((gs.pop().unwrap(), b))
}
pub(super) fn boolean(
    a: &Geometry,
    b: &Geometry,
    op: &str,
    params: Option<&Params>,
) -> Result<Geometry> {
    polygons(a, "boolean input A")?;
    polygons(b, "boolean input B")?;
    let (a, b) = unify(a, b)?;
    if a.nprims() == 0 || b.nprims() == 0 {
        return match op {
            "union" => Ok(if b.nprims() == 0 { a } else { b }),
            "subtract" => Ok(a),
            _ => Geometry::points_only(vec![]),
        };
    }
    if a.p().iter().chain(b.p()).any(|x| !x.is_finite()) {
        return Err(Error::new("boolean requires finite point positions"));
    }
    let (ta, va, pa) = triangles(&a);
    let (tb, vb, pb) = triangles(&b);
    let posa = positions(&a);
    let posb = positions(&b);
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for p in posa.iter().chain(&posb) {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    let ext = sub(hi, lo);
    let eps = 1e-6 * len(ext);
    let size = ext.into_iter().fold(1e-12, f64::max);
    let nb = normals::point_normals(b.p(), &b.topology, b.npoints);
    let grown: Vec<_> = posb
        .iter()
        .enumerate()
        .map(|(i, &p)| {
            add(
                p,
                mul(
                    [nb[i * 3] as f64, nb[i * 3 + 1] as f64, nb[i * 3 + 2] as f64],
                    eps,
                ),
            )
        })
        .collect();
    let c = mul(
        grown.iter().copied().fold([0.; 3], add),
        1. / grown.len() as f64,
    );
    let axis = k::unit([0.5369, 0.2875, 0.7932]);
    let angle = 0.15 * eps / size;
    let sin = angle.sin();
    let cos = angle.cos();
    // Rodrigues in matrix form, matching numpy's I + sin*K + (1-cos)*K@K.
    let kk = [
        [0., -axis[2], axis[1]],
        [axis[2], 0., -axis[0]],
        [-axis[1], axis[0], 0.],
    ];
    let r: crate::math::M3 = std::array::from_fn(|i| {
        std::array::from_fn(|j| {
            (if i == j { 1. } else { 0. })
                + sin * kk[i][j]
                + (1. - cos) * (0..3).map(|k| kk[i][k] * kk[k][j]).sum::<f64>()
        })
    });
    let grown: Vec<_> = grown
        .into_iter()
        .map(|p| {
            add(
                add(crate::math::apply(&r, sub(p, c)), c),
                mul([0.0731, -0.1123, 0.0917], eps),
            )
        })
        .collect();
    let mut result = k::boolean(&posa, &ta, &grown, &tb, op, 3. * eps)?;
    let na = a.npoints;
    let nb = b.npoints;
    result.p[na..na + nb].copy_from_slice(&posb);
    let pos: Vec<_> = result.remap.iter().map(|&i| result.p[i]).collect();
    let mut parts = vec![(a.point.clone(), na), (b.point.clone(), nb)];
    let mut cut_order = vec![];
    for (side, g) in [&a, &b].into_iter().enumerate() {
        let ix: Vec<_> = result
            .keys
            .iter()
            .enumerate()
            .filter(|(_, key)| key[0] == side)
            .map(|(i, _)| i)
            .collect();
        let ia = ix.iter().map(|&i| result.keys[i][1]).collect::<Vec<_>>();
        let ib = ix.iter().map(|&i| result.keys[i][2]).collect::<Vec<_>>();
        let fr = ix.iter().map(|&i| result.edge_t[i]).collect::<Vec<_>>();
        parts.push((lerp(&g.point, &ia, &ib, &fr), ix.len()));
        cut_order.extend(ix);
    }
    let point = combine(&parts)?;
    let mut inv: Vec<_> = (0..na + nb + cut_order.len()).collect();
    for (j, &i) in cut_order.iter().enumerate() {
        inv[na + nb + i] = na + nb + j;
    }
    let mut point = gather(&point, &inv);
    point.insert(
        "P".into(),
        Arc::new(Attr::f32(
            3,
            pos.iter().flat_map(|p| p.map(|x| x as f32)).collect(),
        )),
    );
    let mut vout = vec![];
    let mut sizes_out = vec![];
    let mut vertex_parts = vec![];
    let mut prim_parts = vec![];
    let mut side_out = vec![];
    for (side, g, tv, tp, base, off) in [(0, &a, &va, &pa, 0, 0), (1, &b, &vb, &pb, na, ta.len())] {
        let mine: Vec<_> = result.pieces.iter().filter(|q| q.owner == side).collect();
        if mine.is_empty() {
            continue;
        }
        let mut ntri = vec![0; g.nprims()];
        let mut nwhole = vec![0; g.nprims()];
        for &p in tp {
            ntri[p] += 1;
        }
        for q in &mine {
            if q.whole {
                nwhole[tp[q.src - off]] += 1;
            }
        }
        let intact: Vec<_> = (0..g.nprims())
            .map(|p| ntri[p] > 0 && ntri[p] == nwhole[p])
            .collect();
        let prims: Vec<_> = (0..g.nprims()).filter(|&p| intact[p]).collect();
        if !prims.is_empty() {
            let mut vv = vec![];
            for &p in &prims {
                let mut r: Vec<_> = range(g, p).collect();
                if side == 1 && op == "subtract" {
                    r.reverse();
                }
                sizes_out.push(r.len());
                side_out.push(side);
                vout.extend(
                    r.iter()
                        .map(|&v| g.topology.vertex_point[v] as usize + base),
                );
                vv.extend(r);
            }
            vertex_parts.push((gather(&g.vertex, &vv), vv.len()));
            prim_parts.push((gather(&g.prim, &prims), prims.len()));
        }
        let cut: Vec<_> = mine
            .into_iter()
            .filter(|q| !intact[tp[q.src - off]])
            .collect();
        if cut.is_empty() {
            continue;
        }
        let sizes: Vec<_> = cut.iter().map(|q| q.v.len()).collect();
        let v: Vec<_> = cut.iter().flat_map(|q| q.v.iter().copied()).collect();
        let bary: Vec<_> = cut.iter().flat_map(|q| q.bary.iter().copied()).collect();
        let idx: Vec<_> = cut
            .iter()
            .flat_map(|q| std::iter::repeat_n(tv[q.src - off], q.v.len()))
            .collect();
        let attrs = blend(&g.vertex, &idx, &bary);
        let flat = planar(g);
        let groups: Vec<_> = cut
            .iter()
            .map(|q| {
                let tri = q.src - off;
                let p = tp[tri];
                if flat[p] { p } else { g.nprims() + tri }
            })
            .collect();
        let merged = merge_pieces(&v, &sizes, &groups, &pos);
        let mut ci = vec![];
        let mut pi = vec![];
        for (corners, grp) in merged {
            sizes_out.push(corners.len());
            side_out.push(side);
            vout.extend(corners.iter().map(|&c| v[c]));
            ci.extend(corners);
            pi.push(if grp < g.nprims() {
                grp
            } else {
                tp[grp - g.nprims()]
            });
        }
        vertex_parts.push((gather(&attrs, &ci), ci.len()));
        prim_parts.push((gather(&g.prim, &pi), pi.len()));
    }
    if sizes_out.is_empty() {
        return Geometry::points_only(vec![]);
    }
    let mut out = make(
        point,
        vout.iter().map(|&i| i as i32).collect(),
        &sizes_out,
        combine_vertices(&vertex_parts)?,
        combine(&prim_parts)?,
        a.detail.clone(),
        None,
    )?;
    super::material_tables::merge_usd_sources(&[a, b], &mut out)?;
    let defaults = Params::resolve(BOOLEAN.params, None)?;
    let p = params.unwrap_or(&defaults);
    outgroup(
        &mut out,
        p,
        "from_a",
        "prim",
        &side_out.iter().map(|&s| s == 0).collect::<Vec<_>>(),
    )?;
    outgroup(
        &mut out,
        p,
        "from_b",
        "prim",
        &side_out.iter().map(|&s| s == 1).collect::<Vec<_>>(),
    )?;
    if p.b("outgrp_seam")? || p.b("outgrp_seampoints")? {
        let e = topo::edges(&out.topology, out.npoints);
        let (pv, _, _) = corners(&out);
        let mut sides = vec![0u8; e.len()];
        for (v, &edge) in e.of_vertex.iter().enumerate() {
            if edge >= 0 {
                sides[edge as usize] |= 1 << side_out[pv[v]];
            }
        }
        let seam: Vec<_> = sides.iter().map(|&s| s == 3).collect();
        let mut points = vec![false; out.npoints];
        for (edge, &s) in e.points.chunks_exact(2).zip(&seam) {
            if s {
                points[edge[0] as usize] = true;
                points[edge[1] as usize] = true;
            }
        }
        outgroup(&mut out, p, "seam", "edge", &seam)?;
        outgroup(&mut out, p, "seampoints", "point", &points)?;
    }
    out.point.remove("N");
    out.vertex.remove("N");
    out.prim.remove("N");
    compact(out)
}
