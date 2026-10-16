//! Give selected surfaces thickness, matching minihou/nodes/solidify.py.
use super::{GEO, Inputs, NodeSpec, model::*};
use crate::{
    error::Result,
    geometry::{Attr, Geometry},
    normals,
    params::Params,
};
use std::{collections::BTreeMap, sync::Arc};
pub const SOLIDIFY: NodeSpec = NodeSpec {
    name: "solidify",
    inputs: GEO,
    params: crate::params!("thickness":0.1;"offset":-1.;"even":true;"rim":true;"group":"";"outgrp_top":false;"outgrp_top_name":"solidifytop";"outgrp_bottom":false;"outgrp_bottom_name":"solidifybottom";"outgrp_side":false;"outgrp_side_name":"solidifyside"),
    cook: solidify,
};
fn solidify(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = SOLIDIFY.one(inputs, 0)?;
    let mut thickness = p.f("thickness")?;
    let mut offset = p.f("offset")?;
    if thickness < 0. {
        thickness = -thickness;
        offset = -offset;
    }
    let mask = prim_mask(&g, &p.s("group")?)?;
    let sel: Vec<_> = (0..g.nprims())
        .map(|i| mask[i] && range(&g, i).len() >= 3 && g.topology.is_closed(i))
        .collect();
    if !sel.iter().any(|&b| b) || thickness == 0. {
        let mut out = (*g).clone();
        outgroup(&mut out, p, "top", "prim", &sel)?;
        outgroup(&mut out, p, "bottom", "prim", &vec![false; g.nprims()])?;
        outgroup(&mut out, p, "side", "prim", &vec![false; g.nprims()])?;
        return Ok(out);
    }
    let (pv, nx, _) = corners(&g);
    let vp = &g.topology.vertex_point;
    let pos = positions(&g);
    let pn = normals::prim_normals(g.p(), &g.topology);
    let fn_: Vec<_> = pn
        .iter()
        .map(|&v| mul(v, 1. / if len(v) > 0. { len(v) } else { 1. }))
        .collect();
    let mut vn = vec![[0.; 3]; g.npoints];
    let mut count = vec![0usize; g.npoints];
    for v in 0..g.nvertices() {
        if sel[pv[v]] {
            let q = vp[v] as usize;
            vn[q] = add(vn[q], pn[pv[v]]);
            count[q] += 1;
        }
    }
    for n in &mut vn {
        let l = len(*n);
        *n = mul(*n, 1. / if l > 1e-30 { l } else { 1. });
    }
    let mut scale = vec![1.; g.npoints];
    if p.b("even")? {
        let mut sum = vec![0.; g.npoints];
        for v in 0..g.nvertices() {
            if sel[pv[v]] {
                let q = vp[v] as usize;
                sum[q] += dot(vn[q], fn_[pv[v]]);
            }
        }
        for q in 0..g.npoints {
            scale[q] = 1. / (sum[q] / count[q].max(1) as f64).clamp(0.15, 1.);
        }
    }
    let src: Vec<_> = (0..g.npoints).filter(|&q| count[q] > 0).collect();
    let mut map = vec![0; g.npoints];
    for (i, &q) in src.iter().enumerate() {
        map[q] = g.npoints + i;
    }
    let dout = thickness * (1. + offset) / 2.;
    let din = thickness * (1. - offset) / 2.;
    let mut newp = Vec::new();
    for q in 0..g.npoints {
        let d = if count[q] > 0 {
            mul(vn[q], scale[q] * dout)
        } else {
            [0.; 3]
        };
        newp.extend(add(pos[q], d).map(|x| x as f32));
    }
    for &q in &src {
        newp.extend(sub(pos[q], mul(vn[q], scale[q] * din)).map(|x| x as f32));
    }
    let mut point = concat(&g.point, &gather(&g.point, &src));
    point.remove("N");
    point.insert("P".into(), Arc::new(Attr::f32(3, newp)));
    let mut vertices = vp.clone();
    let mut vs: Vec<_> = (0..g.nvertices()).collect();
    let mut ps: Vec<_> = (0..g.nprims()).collect();
    let mut sizes: Vec<_> = (0..g.nprims()).map(|i| range(&g, i).len()).collect();
    let mut closed = g.topology.closed_mask();
    let mut nb = 0;
    for i in 0..g.nprims() {
        if sel[i] {
            nb += 1;
            sizes.push(range(&g, i).len());
            ps.push(i);
            closed.push(true);
            for v in range(&g, i).rev() {
                vertices.push(map[vp[v] as usize] as i32);
                vs.push(v);
            }
        }
    }
    let mut cnt = BTreeMap::new();
    for v in 0..g.nvertices() {
        if sel[pv[v]] {
            let (a, b) = (vp[v], vp[nx[v]]);
            *cnt.entry((a.min(b), a.max(b))).or_insert(0usize) += 1;
        }
    }
    if p.b("rim")? {
        for v in 0..g.nvertices() {
            if !sel[pv[v]] {
                continue;
            }
            let (a, b) = (vp[v], vp[nx[v]]);
            if cnt[&(a.min(b), a.max(b))] != 1 {
                continue;
            }
            vertices.extend([a, map[a as usize] as i32, map[b as usize] as i32, b]);
            vs.extend([v, v, nx[v], nx[v]]);
            ps.push(pv[v]);
            sizes.push(4);
            closed.push(true);
        }
    }
    let mut out = make(
        point,
        vertices,
        &sizes,
        gather(&g.vertex, &vs),
        gather(&g.prim, &ps),
        g.detail.clone(),
        if g.topology.has_open() {
            Some(closed)
        } else {
            None
        },
    )?;
    let total = out.nprims();
    outgroup(
        &mut out,
        p,
        "top",
        "prim",
        &(0..total)
            .map(|i| i < g.nprims() && sel[i])
            .collect::<Vec<_>>(),
    )?;
    outgroup(
        &mut out,
        p,
        "bottom",
        "prim",
        &(0..total)
            .map(|i| i >= g.nprims() && i < g.nprims() + nb)
            .collect::<Vec<_>>(),
    )?;
    outgroup(
        &mut out,
        p,
        "side",
        "prim",
        &(0..total).map(|i| i >= g.nprims() + nb).collect::<Vec<_>>(),
    )?;
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::topo;
    #[test]
    fn shell_and_negative_thickness() {
        let g = Geometry::from_arrays(
            vec![0., 0., 0., 1., 0., 0., 1., 1., 0., 0., 1., 0.],
            vec![0, 1, 2, 3],
            vec![0, 4],
            None,
        )
        .unwrap();
        let p = Params::resolve(SOLIDIFY.params, None).unwrap();
        let o = solidify(&[vec![Arc::new(g)]], &p).unwrap();
        assert_eq!((o.npoints, o.nprims()), (8, 6));
        assert!(
            o.p()[12..]
                .chunks_exact(3)
                .all(|p| (p[2] + 0.1).abs() < 1e-6)
        );
        assert!(
            topo::edges(&o.topology, o.npoints)
                .count
                .iter()
                .all(|&c| c == 2)
        );
        o.validate().unwrap();
    }
}
