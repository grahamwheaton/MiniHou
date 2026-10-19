//! Polygon lofting and conservative planar profile offsets from profiles.py.
use super::{GEO, Inputs, NodeSpec, edge_tools as edges, model::*};
use crate::{
    error::{Error, Result},
    geometry::{Attr, Geometry},
    params::Params,
};
use std::{collections::BTreeSet, sync::Arc};
pub const SKIN_PROFILES: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None,
    name: "skin_profiles",
    inputs: GEO,
    params: crate::params!("group":"";"keep_profiles":false;"close_loft":false),
    cook: skin,
};
pub const POLYEXPAND2D: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None,
    name: "polyexpand2d",
    inputs: GEO,
    params: crate::params!("distance":0.1;"group":"";"mode":"single";"output_inside":true;"output_outside":true;"keep_input":false;"tolerance":0.000001;"miter_limit":10.;"outgrp_inside":false;"outgrp_inside_name":"inside";"outgrp_outside":false;"outgrp_outside_name":"outside";"outattr_edgedist":false;"outattr_edgedist_name":"edgedist";"outattr_edgespeed":false;"outattr_edgespeed_name":"edgespeed"),
    cook: expand,
};
fn skin(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = SKIN_PROFILES.one(inputs, 0)?;
    let sel = prim_mask(&g, &p.s("group")?)?;
    let ids: Vec<_> = (0..g.nprims()).filter(|&i| sel[i]).collect();
    let close = p.b("close_loft")?;
    if ids.len() < if close { 3 } else { 2 } {
        return Err(Error::new(
            "skin requires at least two profiles (three for close_loft)",
        ));
    }
    let size = range(&g, ids[0]).len();
    let closed = g.topology.is_closed(ids[0]);
    if size < if closed { 3 } else { 2 }
        || ids
            .iter()
            .any(|&i| range(&g, i).len() != size || g.topology.is_closed(i) != closed)
    {
        return Err(Error::new(
            "skin needs equal-count profiles with matching open/closed state",
        ));
    }
    if ids.len() * size > 1_000_000 {
        return Err(Error::new("skin limit: one million profile corners"));
    }
    for &i in &ids {
        if range(&g, i)
            .map(|v| g.topology.vertex_point[v])
            .collect::<BTreeSet<_>>()
            .len()
            != size
        {
            return Err(Error::new("skin profiles must not repeat points"));
        }
    }
    let keep = p.b("keep_profiles")?;
    let mut sources: Vec<_> = (0..g.nprims()).filter(|&i| !sel[i] || keep).collect();
    let mut faces: Vec<Vec<_>> = sources.iter().map(|&i| range(&g, i).collect()).collect();
    let mut flags: Vec<_> = sources.iter().map(|&i| g.topology.is_closed(i)).collect();
    let pos = positions(&g);
    for k in 0..if close { ids.len() } else { ids.len() - 1 } {
        let a = range(&g, ids[k]).start;
        let b = range(&g, ids[(k + 1) % ids.len()]).start;
        for j in 0..if closed { size } else { size - 1 } {
            let h = (j + 1) % size;
            let f = vec![a + j, a + h, b + h, b + j];
            let pts: Vec<_> = f
                .iter()
                .map(|&v| g.topology.vertex_point[v] as usize)
                .collect();
            let q: Vec<_> = pts.iter().map(|&i| pos[i]).collect();
            if pts.iter().collect::<BTreeSet<_>>().len() < 4
                || len(cross(sub(q[1], q[0]), sub(q[2], q[0])))
                    + len(cross(sub(q[2], q[0]), sub(q[3], q[0])))
                    < 1e-12
            {
                return Err(Error::new(
                    "skin would create a collapsed quad; check profile order/correspondence",
                ));
            }
            faces.push(f);
            sources.push(ids[k]);
            flags.push(true);
        }
    }
    edges::rebuild_edges(&g, &faces, &sources, Some(flags))
}
fn cross2(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}
fn sub2(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] - b[0], a[1] - b[1]]
}
fn dot2(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}
fn offset(p: &[[f64; 3]], distance: f64, tolerance: f64, miter: f64) -> Result<Vec<[f64; 3]>> {
    if p.len() < 3 || p.iter().flatten().any(|x| !x.is_finite()) {
        return Err(Error::new("offset requires at least three finite points"));
    }
    let n = p.len();
    let center = mul(p.iter().copied().fold([0.; 3], add), 1. / n as f64);
    let mut cov = [[0.; 3]; 3];
    for &q in p {
        let r = sub(q, center);
        for i in 0..3 {
            for j in 0..3 {
                cov[i][j] += r[i] * r[j];
            }
        }
    }
    let (eval, v) = eigen(cov);
    let mut order = [0, 1, 2];
    order.sort_by(|&a, &b| eval[b].total_cmp(&eval[a]));
    let axes = order.map(|i| [v[0][i], v[1][i], v[2][i]]);
    let mut scale = 1e-12f64;
    for k in 0..3 {
        let lo = p.iter().map(|q| q[k]).fold(f64::INFINITY, f64::min);
        let hi = p.iter().map(|q| q[k]).fold(f64::NEG_INFINITY, f64::max);
        scale = scale.max(hi - lo);
    }
    let eps = (tolerance * scale).max(1e-12);
    if eval[order[1]].max(0.).sqrt() < eps
        || p.iter().any(|&q| dot(sub(q, center), axes[2]).abs() > eps)
    {
        return Err(Error::new("offset profile is degenerate or nonplanar"));
    }
    let xy: Vec<_> = p
        .iter()
        .map(|&q| {
            let r = sub(q, center);
            [dot(r, axes[0]), dot(r, axes[1])]
        })
        .collect();
    let mut d = Vec::new();
    for i in 0..n {
        let r = sub2(xy[(i + 1) % n], xy[i]);
        let l = dot2(r, r).sqrt();
        if l < eps {
            return Err(Error::new(
                "offset profile contains repeated/zero-length edges",
            ));
        }
        d.push([r[0] / l, r[1] / l]);
    }
    let area = (0..n).map(|i| cross2(xy[i], xy[(i + 1) % n])).sum::<f64>() / 2.;
    let sign = if area > 0. { 1. } else { -1. };
    for i in 0..n {
        if cross2(d[(i + n - 1) % n], d[i]) * sign <= tolerance {
            return Err(Error::new(
                "initial polyexpand2d supports strictly convex loops without collinear corners",
            ));
        }
        if xy
            .iter()
            .any(|&q| cross2(d[i], sub2(q, xy[i])) * sign < -eps)
        {
            return Err(Error::new("offset profile must be simple and convex"));
        }
    }
    let normal: Vec<_> = d.iter().map(|r| [sign * r[1], -sign * r[0]]).collect();
    let mut result = Vec::new();
    for i in 0..n {
        let prev = (i + n - 1) % n;
        let delta = sub2(normal[i], normal[prev]).map(|x| x * distance);
        let along = cross2(delta, d[i]) / cross2(d[prev], d[i]);
        let q = [
            xy[i][0] + distance * normal[prev][0] + along * d[prev][0],
            xy[i][1] + distance * normal[prev][1] + along * d[prev][1],
        ];
        if distance != 0.
            && dot2(sub2(q, xy[i]), sub2(q, xy[i])).sqrt() > distance.abs() * miter + eps
        {
            return Err(Error::new(
                "offset exceeds miter_limit; reduce distance or use a less acute profile",
            ));
        }
        result.push(q);
    }
    if (0..n).any(|i| dot2(sub2(result[(i + 1) % n], result[i]), d[i]) <= eps) {
        return Err(Error::new("inward offset collapses or inverts the profile"));
    }
    Ok(result
        .iter()
        .map(|q| add(center, add(mul(axes[0], q[0]), mul(axes[1], q[1]))))
        .collect())
}
fn expand(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = POLYEXPAND2D.one(inputs, 0)?;
    let sel = prim_mask(&g, &p.s("group")?)?;
    let distance = p.f("distance")?;
    let mode = p.e("mode", &["single", "inside_outside"])?;
    let mut jobs = Vec::new();
    if mode == "single" {
        jobs.push((distance, if distance > 0. { 2 } else { 1 }));
    } else {
        if p.b("output_outside")? {
            jobs.push((distance.abs(), 2));
        }
        if p.b("output_inside")? {
            jobs.push((-distance.abs(), 1));
        }
    }
    if !sel.iter().any(|&x| x) || distance == 0. || jobs.is_empty() {
        return Ok((*g).clone());
    }
    let tolerance = p.f("tolerance")?;
    let miter = p.f("miter_limit")?;
    let keep = p.b("keep_input")?;
    let pos = positions(&g);
    let mut positions = pos.clone();
    let mut source: Vec<_> = (0..g.npoints).collect();
    let mut dist = vec![0.; g.npoints];
    let mut speed = vec![0.; g.npoints];
    let mut vertices = Vec::new();
    let mut vs = Vec::new();
    let mut ps = Vec::new();
    let mut sizes = Vec::new();
    let mut kinds = Vec::new();
    let mut flags = Vec::new();
    for i in 0..g.nprims() {
        let r = range(&g, i);
        if !sel[i] || keep {
            vertices.extend(r.clone().map(|v| g.topology.vertex_point[v]));
            vs.extend(r.clone());
            ps.push(i);
            sizes.push(r.len());
            kinds.push(0);
            flags.push(g.topology.is_closed(i));
        }
        if !sel[i] {
            continue;
        }
        if !g.topology.is_closed(i) {
            return Err(Error::new(
                "initial polyexpand2d requires closed convex profiles",
            ));
        }
        let ids: Vec<_> = r
            .clone()
            .map(|v| g.topology.vertex_point[v] as usize)
            .collect();
        let q: Vec<_> = ids.iter().map(|&i| pos[i]).collect();
        for &(d, kind) in &jobs {
            let moved = offset(&q, d, tolerance, miter)?;
            let count = positions.len();
            vertices.extend((count..count + moved.len()).map(|i| i as i32));
            for (a, b) in moved.iter().zip(&q) {
                speed.push(len(sub(*a, *b)) / d.abs().max(1e-30));
                dist.push(d);
            }
            positions.extend(moved);
            source.extend(&ids);
            vs.extend(r.clone());
            ps.push(i);
            sizes.push(r.len());
            kinds.push(kind);
            flags.push(true);
        }
    }
    let mut point = gather(&g.point, &source);
    point.insert(
        "P".into(),
        Arc::new(Attr::f32(
            3,
            positions.iter().flat_map(|q| q.map(|x| x as f32)).collect(),
        )),
    );
    let mut out = make(
        point,
        vertices,
        &sizes,
        gather(&g.vertex, &vs),
        gather(&g.prim, &ps),
        g.detail.clone(),
        Some(flags),
    )?;
    edges::remap(&g, &mut out, Some(&source))?;
    outgroup(
        &mut out,
        p,
        "inside",
        "prim",
        &kinds.iter().map(|&k| k == 1).collect::<Vec<_>>(),
    )?;
    outgroup(
        &mut out,
        p,
        "outside",
        "prim",
        &kinds.iter().map(|&k| k == 2).collect::<Vec<_>>(),
    )?;
    for (key, arr) in [("edgedist", dist), ("edgespeed", speed)] {
        if p.b(&format!("outattr_{key}"))? {
            let name = p.s(&format!("outattr_{key}_name"))?;
            if !name.trim().is_empty() {
                out.point.insert(
                    name.trim().into(),
                    Arc::new(Attr::f32(1, arr.iter().map(|&x| x as f32).collect())),
                );
            }
        }
    }
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn offset_winding_independent_and_collapse_rejected() {
        let q = vec![[0., 0., 0.], [2., 0., 0.], [2., 2., 0.], [0., 2., 0.]];
        let out = offset(&q, 0.25, 1e-6, 10.).unwrap();
        assert!(len(sub(out[0], [-0.25, -0.25, 0.])) < 1e-12);
        let mut rev = q.clone();
        rev.reverse();
        let mut r = offset(&rev, 0.25, 1e-6, 10.).unwrap();
        r.reverse();
        for (a, b) in out.iter().zip(r) {
            assert!(len(sub(*a, b)) < 1e-12);
        }
        assert!(offset(&q, -1., 1e-6, 10.).is_err());
    }
    #[test]
    fn skin_two_open_profiles() {
        let g = Geometry::from_arrays(
            vec![0., 0., 0., 1., 0., 0., 0., 1., 0., 1., 1., 0.],
            vec![0, 1, 2, 3],
            vec![0, 2, 4],
            Some(vec![false, false]),
        )
        .unwrap();
        let p = Params::resolve(SKIN_PROFILES.params, json!({}).as_object()).unwrap();
        let out = skin(&[vec![Arc::new(g)]], &p).unwrap();
        assert_eq!(out.topology.vertex_point, vec![0, 1, 3, 2]);
        assert_eq!(out.nprims(), 1);
        out.validate().unwrap();
    }
}
