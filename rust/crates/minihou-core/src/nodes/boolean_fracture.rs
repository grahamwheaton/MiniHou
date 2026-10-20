//! Sequential cutter fracture using the surface Boolean (fracture_tools.py).
use super::{
    InputSpec, Inputs, NodeSpec, attrib_tools::piece_labels, boolean, group::keep_prims, model::*,
};
use crate::{
    error::Result,
    geometry::{Attr, AttrData, Geometry},
    params::Params,
    topo,
};
use std::sync::Arc;
pub const BOOLEAN_FRACTURE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "boolean_fracture",
    inputs: &[
        InputSpec {
            name: "geo",
            multi: false,
            optional: false,
        },
        InputSpec {
            name: "cutters",
            multi: false,
            optional: false,
        },
    ],
    params: crate::params!("keep":"both";"mode":"each_cutter";"piece_attrib":"piece";"cutter_attrib":"cutter";
        "outgrp_inside":true;"outgrp_inside_name":"inside";"outgrp_outside":false;"outgrp_outside_name":"outside";
        "outgrp_cutter":false;"outgrp_cutter_name":"cutterpieces";"outgrp_remainder":false;"outgrp_remainder_name":"remainder";
        "outgrp_cutpoints":false;"outgrp_cutpoints_name":"cutpoints";"outgrp_cutedges":false;"outgrp_cutedges_name":"cutedges"),
    cook: cook,
};
fn flags(g: &Geometry, name: &str) -> Vec<bool> {
    g.prim
        .get(name)
        .map(|a| match &a.data {
            AttrData::I32(v) => v.iter().map(|&v| v != 0).collect(),
            AttrData::F32(v) => v.iter().map(|&v| v != 0.).collect(),
        })
        .unwrap_or_else(|| vec![false; g.nprims()])
}
fn bool_mark(a: &Geometry, b: &Geometry, op: &str) -> Result<Geometry> {
    let settings = serde_json::json!({"outgrp_from_a":false,"outgrp_from_b_name":"vfcutter"});
    let params = Params::resolve(boolean::BOOLEAN.params, settings.as_object())?;
    let mut g = boolean::boolean(a, b, op, Some(&params))?;
    let mark = flags(&g, "group_vfcutter");
    g.prim.remove("group_vfcutter");
    let old = flags(&g, "vf_inside");
    let inside = mark
        .iter()
        .zip(old)
        .map(|(&a, b)| i32::from(a || b))
        .collect();
    g.prim
        .insert("vf_inside".into(), Arc::new(Attr::i32(1, inside)));
    Ok(g)
}
fn cook(i: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = BOOLEAN_FRACTURE.one(i, 0)?;
    let cutters = BOOLEAN_FRACTURE.one(i, 1)?;
    polygons(&geo, "boolean_fracture input")?;
    polygons(&cutters, "boolean_fracture cutters")?;
    let keep = p.e("keep", &["both", "inside", "outside"])?;
    let mode = p.e("mode", &["each_cutter", "union"])?;
    if geo.nprims() == 0 || cutters.nprims() == 0 {
        return Ok((*geo).clone());
    }
    let bodies = if mode == "union" {
        vec![(*cutters).clone()]
    } else {
        let labels = piece_labels(&cutters).1;
        let max = labels.iter().copied().max().unwrap_or(-1);
        (0..=max)
            .map(|k| {
                keep_prims(
                    &cutters,
                    &labels.iter().map(|&l| l == k).collect::<Vec<_>>(),
                    true,
                )
            })
            .collect::<Result<Vec<_>>>()?
    };
    let mut rem = (*geo).clone();
    rem.prim.insert(
        "vf_inside".into(),
        Arc::new(Attr::i32(1, vec![0; rem.nprims()])),
    );
    let mut parts = vec![];
    for (k, body) in bodies.iter().enumerate() {
        if rem.nprims() == 0 {
            break;
        }
        let cut = bool_mark(&rem, body, "intersect")?;
        if cut.nprims() > 0 {
            parts.push((cut, k as i32, true));
        }
        rem = bool_mark(&rem, body, "subtract")?;
    }
    if rem.nprims() > 0 {
        parts.push((rem, -1, false));
    }
    let mut out = vec![];
    let ca = p.s("cutter_attrib")?;
    for (mut g, k, is_cutter) in parts {
        if keep == "inside" && !is_cutter || keep == "outside" && is_cutter {
            continue;
        }
        let inside = flags(&g, "vf_inside");
        g.prim.remove("vf_inside");
        if !ca.is_empty() {
            g.prim
                .insert(ca.clone(), Arc::new(Attr::i32(1, vec![k; g.nprims()])));
        }
        let e = topo::edges(&g.topology, g.npoints);
        let (pv, _, _) = corners(&g);
        let mut sides = vec![0u8; e.len()];
        for (v, &edge) in e.of_vertex.iter().enumerate() {
            if edge >= 0 {
                sides[edge as usize] |= if inside[pv[v]] { 1 } else { 2 };
            }
        }
        let seam: Vec<_> = sides.iter().map(|&s| s == 3).collect();
        let mut points = vec![false; g.npoints];
        for (pair, &s) in e.points.chunks_exact(2).zip(&seam) {
            if s {
                points[pair[0] as usize] = true;
                points[pair[1] as usize] = true;
            }
        }
        outgroup(&mut g, p, "inside", "prim", &inside)?;
        outgroup(
            &mut g,
            p,
            "outside",
            "prim",
            &inside.iter().map(|v| !v).collect::<Vec<_>>(),
        )?;
        let n = g.nprims();
        outgroup(&mut g, p, "cutter", "prim", &vec![is_cutter; n])?;
        outgroup(&mut g, p, "remainder", "prim", &vec![!is_cutter; n])?;
        outgroup(&mut g, p, "cutpoints", "point", &points)?;
        outgroup(&mut g, p, "cutedges", "edge", &seam)?;
        out.push(g);
    }
    let mut iter = out.into_iter();
    let Some(mut res) = iter.next() else {
        return Geometry::points_only(vec![]);
    };
    for g in iter {
        res = merge_pair(&res, &g)?;
    }
    let pa = p.s("piece_attrib")?;
    if !pa.is_empty() {
        let labels = piece_labels(&res).1;
        res.prim.insert(pa, Arc::new(Attr::i32(1, labels)));
    }
    Ok(res)
}
fn merge_pair(a: &Geometry, b: &Geometry) -> Result<Geometry> {
    let (a, b) = boolean::unify(a, b)?;
    let mut vp = a.topology.vertex_point.clone();
    vp.extend(
        b.topology
            .vertex_point
            .iter()
            .map(|&v| v + a.npoints as i32),
    );
    let sizes = (0..a.nprims())
        .map(|p| range(&a, p).len())
        .chain((0..b.nprims()).map(|p| range(&b, p).len()))
        .collect::<Vec<_>>();
    let mut detail = a.detail.clone();
    for (k, v) in &b.detail {
        detail.entry(k.clone()).or_insert_with(|| v.clone());
    }
    let mut sources = serde_json::Map::new();
    for g in [&a, &b] {
        if let Some(src) = g.detail.get("usd_materials").and_then(|v| v.as_object()) {
            for (n, v) in src {
                sources.entry(n.clone()).or_insert_with(|| v.clone());
            }
        }
    }
    if !sources.is_empty() {
        detail.insert("usd_materials".into(), sources.into());
    }
    let combine = boolean::combine;
    let mut out = make(
        combine(&[(a.point.clone(), a.npoints), (b.point.clone(), b.npoints)])?,
        vp,
        &sizes,
        boolean::combine_vertices(&[
            (a.vertex.clone(), a.nvertices()),
            (b.vertex.clone(), b.nvertices()),
        ])?,
        combine(&[(a.prim.clone(), a.nprims()), (b.prim.clone(), b.nprims())])?,
        detail,
        None,
    )?;
    // merge.py's instance dictionary remap. Boolean itself preserves A's detail.
    let lists: Vec<Vec<serde_json::Value>> = [&a, &b]
        .into_iter()
        .map(|g| {
            g.detail
                .get("instance_objects")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default()
        })
        .collect();
    if lists.iter().filter(|l| !l.is_empty()).count() == 2 {
        let mut names = vec![];
        for l in &lists {
            for n in l {
                if !names.contains(n) {
                    names.push(n.clone());
                }
            }
        }
        let mut indices = vec![];
        for (g, l) in [&a, &b].into_iter().zip(&lists) {
            let values = g.point.get("instance");
            for i in 0..g.npoints {
                let v = values
                    .map(|a| match &a.data {
                        AttrData::I32(v) => v[i * a.width] as i64,
                        AttrData::F32(v) => v[i * a.width] as i64,
                    })
                    .unwrap_or(0)
                    .clamp(0, l.len() as i64) as usize;
                indices.push(if v == 0 {
                    0
                } else {
                    names.iter().position(|n| *n == l[v - 1]).unwrap() as i32 + 1
                });
            }
        }
        out.point
            .insert("instance".into(), Arc::new(Attr::i32(1, indices)));
        let mut bounds = vec![serde_json::Value::Null; names.len()];
        for (g, l) in [&a, &b].into_iter().zip(&lists) {
            if let Some(known) = g
                .detail
                .get("instance_bounds")
                .and_then(|v| v.as_array())
                .filter(|v| v.len() == l.len())
            {
                for (n, v) in l.iter().zip(known) {
                    let j = names.iter().position(|x| x == n).unwrap();
                    if bounds[j].is_null() && !v.is_null() {
                        bounds[j] = v.clone();
                    }
                }
            }
        }
        out.detail.insert("instance_objects".into(), names.into());
        if bounds.iter().any(|v| !v.is_null()) {
            out.detail.insert("instance_bounds".into(), bounds.into());
        } else {
            out.detail.remove("instance_bounds");
        }
    }
    Ok(out)
}
