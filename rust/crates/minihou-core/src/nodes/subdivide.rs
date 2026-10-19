//! Catmull-Clark/linear refinement and semi-sharp creases, from subdivide.py.
use super::{GEO, Inputs, NodeSpec, edge_tools as edges, model::*};
use crate::{
    error::{Error, Result},
    geometry::{Attr, AttrData, AttrTable, Geometry},
    params::Params,
    topo,
};
use std::{collections::BTreeMap, sync::Arc};
pub const SUBDIVIDE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None,
    name: "subdivide",
    inputs: GEO,
    params: crate::params!("depth":1;"algorithm":"catmull_clark";"use_creases":true;"creases":"";"override_crease_weight":false;"crease_weight":1.;"outgrp_creases":false;"outgrp_creases_name":"creases"),
    cook: subdivide,
};
fn subdivide(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = SUBDIVIDE.one(inputs, 0)?;
    let depth = p.i("depth")?;
    if !(0..=6).contains(&depth) {
        return Err(Error::new("depth must be 0..6"));
    }
    if depth > 0 && g.nvertices().saturating_mul(4usize.pow((depth - 1) as u32)) > 20_000_000 {
        return Err(Error::new(
            "subdivide would exceed 20,000,000 faces; lower the depth",
        ));
    }
    polygons(&g, "subdivide")?;
    let smooth = p.e("algorithm", &["catmull_clark", "linear"])? == "catmull_clark";
    let mut out = (*g).clone();
    let mut use_creases = p.b("use_creases")?;
    let text = p.s("creases")?;
    if !text.trim().is_empty() {
        let mut w = edges::values(&g, "creaseweight")?;
        let mask = edges::edge_mask(&g, &text)?;
        for (x, b) in w.iter_mut().zip(mask) {
            if b {
                *x = if p.b("override_crease_weight")? {
                    p.f("crease_weight")? as f32
                } else {
                    if *x > 0. { *x } else { 1. }
                };
            }
        }
        edges::write_values(&mut out, "creaseweight", &w, false)?;
        use_creases = true;
    }
    for _ in 0..depth {
        out = once(&out, smooth, use_creases)?;
    }
    if p.b("outgrp_creases")? && !p.s("outgrp_creases_name")?.trim().is_empty() {
        let mask = edges::values(&out, "creaseweight")?
            .iter()
            .map(|&x| x > 0.)
            .collect::<Vec<_>>();
        outgroup(&mut out, p, "creases", "edge", &mask)?;
    }
    Ok(out)
}
fn midpoint_attrs(t: &AttrTable, pairs: &[i64]) -> AttrTable {
    t.iter()
        .map(|(name, a)| {
            let w = a.width;
            let attr = match &a.data {
                AttrData::F32(v) => Attr::f32(
                    w,
                    pairs
                        .chunks_exact(2)
                        .flat_map(|q| {
                            (0..w).map(move |k| {
                                (v[q[0] as usize * w + k] + v[q[1] as usize * w + k]) * 0.5
                            })
                        })
                        .collect(),
                ),
                AttrData::I32(v) => Attr::i32(
                    w,
                    pairs
                        .chunks_exact(2)
                        .flat_map(|q| {
                            (0..w).map(move |k| {
                                v[q[0] as usize * w + k].min(v[q[1] as usize * w + k])
                            })
                        })
                        .collect(),
                ),
            };
            (name.clone(), Arc::new(attr))
        })
        .collect()
}
fn once(input: &Geometry, smooth: bool, use_creases: bool) -> Result<Geometry> {
    let filtered;
    let g = if (0..input.nprims()).any(|i| range(input, i).len() < 3) {
        let src: Vec<_> = (0..input.nprims())
            .filter(|&i| range(input, i).len() >= 3)
            .collect();
        let faces: Vec<_> = src.iter().map(|&i| range(input, i).collect()).collect();
        filtered = compact(rebuild(input, &faces, &src, None)?)?;
        &filtered
    } else {
        input
    };
    let np = g.npoints;
    let nf = g.nprims();
    let nv = g.nvertices();
    let (pv, nx, pr) = corners(g);
    let vp = &g.topology.vertex_point;
    let e = topo::edges(&g.topology, np);
    let ne = e.len();
    let p = positions(g);
    let face_buckets: Vec<Vec<_>> = (0..nf)
        .map(|i| range(g, i).map(|v| vp[v] as usize).collect())
        .collect();
    let mut face = vec![[0.; 3]; nf];
    for i in 0..nf {
        for &q in &face_buckets[i] {
            face[i] = add(face[i], p[q]);
        }
        face[i] = mul(face[i], 1. / face_buckets[i].len().max(1) as f64);
    }
    let mid: Vec<_> = e
        .points
        .chunks_exact(2)
        .map(|q| mul(add(p[q[0] as usize], p[q[1] as usize]), 0.5))
        .collect();
    let mut edge = mid.clone();
    let mut vert = p.clone();
    let mut nfaces = vec![0usize; np];
    let mut fsum = vec![[0.; 3]; np];
    for v in 0..nv {
        let q = vp[v] as usize;
        nfaces[q] += 1;
        fsum[q] = add(fsum[q], face[pv[v]]);
    }
    let mut valence = vec![0usize; np];
    let mut rsum = vec![[0.; 3]; np];
    let mut bcount = vec![0usize; np];
    let mut bnb = vec![[0.; 3]; np];
    for i in 0..ne {
        let (a, b) = (e.points[2 * i] as usize, e.points[2 * i + 1] as usize);
        for q in [a, b] {
            valence[q] += 1;
            rsum[q] = add(rsum[q], mid[i]);
        }
        if e.count[i] == 1 {
            bcount[a] += 1;
            bcount[b] += 1;
            bnb[a] = add(bnb[a], p[b]);
            bnb[b] = add(bnb[b], p[a]);
        }
    }
    if smooth {
        let mut ef = vec![[0.; 3]; ne];
        for v in 0..nv {
            let i = e.of_vertex[v] as usize;
            ef[i] = add(ef[i], face[pv[v]]);
        }
        for i in 0..ne {
            if e.count[i] != 1 && e.count[i] <= 2 {
                edge[i] = mul(
                    add(
                        add(p[e.points[2 * i] as usize], p[e.points[2 * i + 1] as usize]),
                        ef[i],
                    ),
                    1. / (2 + e.count[i]).max(1) as f64,
                );
            }
        }
        for q in 0..np {
            let n = valence[q].max(1) as f64;
            let qq = mul(fsum[q], 1. / nfaces[q].max(1) as f64);
            let rr = mul(rsum[q], 1. / n);
            vert[q] = mul(add(add(qq, mul(rr, 2.)), mul(p[q], n - 3.)), 1. / n);
            if bcount[q] > 0 {
                vert[q] = if bcount[q] == 2 {
                    add(mul(p[q], 0.75), mul(bnb[q], 0.125))
                } else {
                    p[q]
                };
            }
            if nfaces[q] == 0 || (bcount[q] > 0 && nfaces[q] == 1) {
                vert[q] = p[q];
            }
        }
    }
    let sharp = edges::values(g, "creaseweight")?;
    let corner = if let Some(a) = g.point.get("cornerweight") {
        if a.width != 1 {
            return Err(Error::new(
                "cornerweight must be a finite nonnegative scalar point attribute",
            ));
        }
        match &a.data {
            AttrData::F32(v) => v.iter().map(|&x| x as f64).collect::<Vec<_>>(),
            AttrData::I32(v) => v.iter().map(|&x| x as f64).collect::<Vec<_>>(),
        }
    } else {
        vec![0.; np]
    };
    if corner.iter().any(|x| !x.is_finite() || *x < 0.) {
        return Err(Error::new(
            "cornerweight must be a finite nonnegative scalar point attribute",
        ));
    }
    if smooth && use_creases && (sharp.iter().any(|&x| x != 0.) || corner.iter().any(|&x| x != 0.))
    {
        let strength: Vec<_> = sharp.iter().map(|&x| (x as f64).clamp(0., 1.)).collect();
        let mut incident = vec![Vec::<(f64, usize)>::new(); np];
        for i in 0..ne {
            let w = strength[i];
            edge[i] = add(mul(edge[i], 1. - w), mul(mid[i], w));
            let w = if e.count[i] == 1 { 1. } else { w };
            if w > 0. {
                let (a, b) = (e.points[2 * i] as usize, e.points[2 * i + 1] as usize);
                incident[a].push((w, b));
                incident[b].push((w, a));
            }
        }
        for q in 0..np {
            let links = &mut incident[q];
            links.sort_by(|a, b| b.0.total_cmp(&a.0).then(b.1.cmp(&a.1)));
            if links.len() >= 2 {
                let w = links[1].0;
                let cp = add(
                    mul(p[q], 0.75),
                    mul(add(p[links[0].1], p[links[1].1]), 0.125),
                );
                if !(bcount[q] > 0 && nfaces[q] == 1) {
                    vert[q] = add(mul(vert[q], 1. - w), mul(cp, w));
                }
            }
            let w = corner[q]
                .min(1.)
                .max(if links.len() >= 3 { links[2].0 } else { 0. });
            vert[q] = add(mul(vert[q], 1. - w), mul(p[q], w));
        }
    }
    let mut point = concat(
        &concat(&g.point, &midpoint_attrs(&g.point, &e.points)),
        &mean(&g.point, &face_buckets),
    );
    point.insert(
        "P".into(),
        Arc::new(Attr::f32(
            3,
            vert.iter()
                .chain(&edge)
                .chain(&face)
                .flat_map(|q| q.map(|x| x as f32))
                .collect(),
        )),
    );
    let mut outvp = Vec::new();
    for v in 0..nv {
        outvp.extend([
            vp[v],
            (np + e.of_vertex[v] as usize) as i32,
            (np + ne + pv[v]) as i32,
            (np + e.of_vertex[pr[v]] as usize) as i32,
        ]);
    }
    let vb: Vec<Vec<_>> = (0..nf).map(|i| range(g, i).collect()).collect();
    let av = mean(&g.vertex, &vb);
    let mut vertex = AttrTable::new();
    for (name, a) in &g.vertex {
        let w = a.width;
        let attr = match &a.data {
            AttrData::F32(v) => {
                let avg = av[name].as_f32().unwrap();
                let mut data = Vec::new();
                for i in 0..nv {
                    data.extend_from_slice(&v[i * w..(i + 1) * w]);
                    for k in 0..w {
                        data.push((v[i * w + k] + v[nx[i] * w + k]) / 2.);
                    }
                    data.extend_from_slice(&avg[pv[i] * w..(pv[i] + 1) * w]);
                    for k in 0..w {
                        data.push((v[i * w + k] + v[pr[i] * w + k]) / 2.);
                    }
                }
                Attr::f32(w, data)
            }
            AttrData::I32(v) => Attr::i32(
                w,
                (0..nv)
                    .flat_map(|i| {
                        std::iter::repeat_n(i, 4)
                            .flat_map(move |j| v[j * w..(j + 1) * w].iter().copied())
                    })
                    .collect(),
            ),
        };
        vertex.insert(name.clone(), Arc::new(attr));
    }
    let mut out = make(
        point,
        outvp,
        &vec![4; nv],
        vertex,
        gather(&g.prim, &pv),
        g.detail.clone(),
        None,
    )?;
    if g.point.contains_key("cornerweight") {
        let mut data: Vec<_> = corner.iter().map(|&x| (x - 1.).max(0.) as f32).collect();
        data.resize(np + ne + nf, 0.);
        out.point
            .insert("cornerweight".into(), Arc::new(Attr::f32(1, data)));
    }
    let mut children = BTreeMap::new();
    for i in 0..ne {
        for q in [e.points[2 * i], e.points[2 * i + 1]] {
            let b = (np + i) as i64;
            children.insert((q.min(b), q.max(b)), i);
        }
    }
    let oe = topo::edges(&out.topology, out.npoints);
    for (name, a) in &g.vertex {
        if !name.starts_with("edge_group_") && name != "creaseweight" {
            continue;
        }
        let mut old = edges::values(g, name)?;
        if name == "creaseweight" {
            for x in &mut old {
                *x = (*x - 1.).max(0.);
            }
        }
        let data: Vec<_> = oe
            .points
            .chunks_exact(2)
            .map(|q| children.get(&(q[0], q[1])).map_or(0., |&i| old[i]))
            .collect();
        edges::write_values(&mut out, name, &data, matches!(a.data, AttrData::I32(_)))?;
    }
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn linear_quad_topology_and_integer_minima() {
        let mut g = Geometry::from_arrays(
            vec![0., 0., 0., 2., 0., 0., 2., 2., 0., 0., 2., 0.],
            vec![0, 1, 2, 3],
            vec![0, 4],
            None,
        )
        .unwrap();
        g.point
            .insert("group_a".into(), Arc::new(Attr::i32(1, vec![1, 1, 0, 0])));
        let out = once(&g, false, true).unwrap();
        assert_eq!((out.npoints, out.nprims()), (9, 4));
        assert_eq!(
            out.topology.vertex_point,
            vec![0, 4, 8, 5, 1, 6, 8, 4, 2, 7, 8, 6, 3, 5, 8, 7]
        );
        assert_eq!(
            out.point["group_a"].data,
            AttrData::I32(vec![1, 1, 0, 0, 1, 0, 0, 0, 0])
        );
        out.validate().unwrap();
    }
    #[test]
    fn sharpness_decays_only_on_children() {
        let mut g = Geometry::from_arrays(
            vec![0., 0., 0., 2., 0., 0., 2., 2., 0., 0., 2., 0.],
            vec![0, 1, 2, 3],
            vec![0, 4],
            None,
        )
        .unwrap();
        edges::write_values(&mut g, "creaseweight", &[3., 0., 0., 0.], false).unwrap();
        let out = once(&g, true, true).unwrap();
        let w = edges::values(&out, "creaseweight").unwrap();
        assert_eq!(w.iter().filter(|&&x| x == 2.).count(), 2);
        assert_eq!(w.iter().filter(|&&x| x != 0.).count(), 2);
    }
    #[test]
    fn guard_before_refinement() {
        let p = Params::resolve(SUBDIVIDE.params, json!({"depth":7}).as_object()).unwrap();
        let g = Geometry::points_only(vec![]).unwrap();
        assert!(subdivide(&[vec![Arc::new(g)]], &p).is_err());
    }
}
