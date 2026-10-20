//! Derived-edge selections and conservative polygon operations from edge_tools.py.
use super::{GEO, InputSpec, Inputs, NodeSpec, groups as node_groups, model::*};
use crate::{
    error::{Error, Result},
    geometry::{Attr, AttrData, Geometry},
    normals,
    params::Params,
    topo,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
mod common;
mod groups;
mod dissolve;
pub(super) use common::{values, write_values, wildcard, remap};
pub(crate) use common::{edge_mask, rebuild_edges};
pub(super) use dissolve::root;
use groups::{crease, edge_group, promote};
use dissolve::dissolve;
pub const CREASE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "crease",
    inputs: GEO,
    params: crate::params!("domain":"edges";"group":"";"sharpness":2.),
    cook: crease,
};
pub const EDGE_GROUP: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "edge_group",
    inputs: &[
        InputSpec {
            name: "geo",
            multi: false,
            optional: false,
        },
        InputSpec {
            name: "bounds",
            multi: false,
            optional: true,
        },
    ],
    params: crate::params!("name":"edges";"method":"pattern";"pattern":"*";"combine":"replace"),
    cook: edge_group,
};
pub const EDGE_GROUP_PROMOTE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "edge_group_promote",
    inputs: GEO,
    params: crate::params!("pattern":"*";"name":"edge_points";"to":"points"),
    cook: promote,
};
pub const DISSOLVE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "dissolve_edges",
    inputs: GEO,
    params: crate::params!("group":"";"angle":0.1;"preserve_seams":true),
    cook: dissolve,
};
pub const POLYBRIDGE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "polybridge",
    inputs: GEO,
    params: crate::params!("source":"";"destination":"";"segments":1;"twist":0;"reverse":false;"outgrp_bridge":false;"outgrp_bridge_name":"bridgepolys"),
    cook: polybridge,
};
pub(super) fn chain(g: &Geometry, selected: &[bool]) -> Result<(Vec<usize>, bool)> {
    let e = topo::edges(&g.topology, g.npoints);
    let mut adj = BTreeMap::<usize, Vec<usize>>::new();
    for (i, p) in e.points.chunks_exact(2).enumerate() {
        if selected[i] {
            let (a, b) = (p[0] as usize, p[1] as usize);
            if a == b {
                return Err(Error::new("zero-length topological edge"));
            }
            adj.entry(a).or_default().push(b);
            adj.entry(b).or_default().push(a);
        }
    }
    if adj.is_empty() || adj.values().any(|nb| nb.len() > 2) {
        return Err(Error::new(
            "select one nonempty, nonbranching edge chain or loop",
        ));
    }
    let ends: Vec<_> = adj
        .iter()
        .filter(|(_, nb)| nb.len() == 1)
        .map(|(&p, _)| p)
        .collect();
    if ends.len() != 0 && ends.len() != 2 {
        return Err(Error::new("edge chain has invalid endpoints"));
    }
    let start = ends
        .first()
        .copied()
        .unwrap_or(*adj.first_key_value().unwrap().0);
    let mut prev = None;
    let mut at = start;
    let mut path = Vec::new();
    while !path.contains(&at) {
        path.push(at);
        let next = adj[&at].iter().copied().find(|&q| Some(q) != prev);
        match next {
            Some(q) => {
                prev = Some(at);
                at = q;
            }
            None => break,
        }
    }
    if path.len() != adj.len() {
        return Err(Error::new("select one connected edge chain or loop"));
    }
    Ok((path, ends.is_empty()))
}
fn orient(
    g: &Geometry,
    path: &mut Vec<usize>,
    closed: bool,
    reverse_boundary: bool,
) -> Result<bool> {
    let (pv, nx, _) = corners(g);
    let vp = &g.topology.vertex_point;
    let mut signs = Vec::new();
    for i in 0..if closed { path.len() } else { path.len() - 1 } {
        let a = path[i] as i32;
        let b = path[(i + 1) % path.len()] as i32;
        let hits: Vec<_> = (0..g.nvertices())
            .filter(|&v| {
                g.topology.is_closed(pv[v])
                    && ((vp[v] == a && vp[nx[v]] == b) || (vp[v] == b && vp[nx[v]] == a))
            })
            .collect();
        if hits.len() > 1 {
            return Err(Error::new(
                "bridge requires boundary edges, not polygon interior edges",
            ));
        }
        if let Some(&v) = hits.first() {
            signs.push(vp[v] == a);
        }
    }
    if signs.iter().any(|&b| b != signs[0]) {
        return Err(Error::new("boundary winding is inconsistent"));
    }
    if signs.first() == Some(&reverse_boundary) {
        path.reverse();
    }
    Ok(!signs.is_empty())
}
fn polybridge(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = POLYBRIDGE.one(inputs, 0)?;
    let src = p.s("source")?;
    let dst = p.s("destination")?;
    if src.trim().is_empty() || dst.trim().is_empty() {
        return Err(Error::new(
            "specify both source and destination edge selections",
        ));
    }
    let am = edge_mask(&g, &src)?;
    let bm = edge_mask(&g, &dst)?;
    let (mut a, ca) = chain(&g, &am)?;
    let (mut b, cb) = chain(&g, &bm)?;
    if ca != cb || a.len() != b.len() || a.iter().any(|q| b.contains(q)) {
        return Err(Error::new(
            "bridge needs disjoint profiles of equal point count and matching open/closed state",
        ));
    }
    orient(&g, &mut a, ca, true)?;
    let oriented = orient(&g, &mut b, cb, false)?;
    if p.b("reverse")? {
        if oriented {
            return Err(Error::new(
                "reverse would conflict with polygon boundary winding",
            ));
        }
        b.reverse();
    }
    let twist = p.i("twist")?;
    if twist != 0 && !ca {
        return Err(Error::new("twist requires closed loops"));
    }
    if ca {
        let l = b.len();
        b.rotate_left(twist.rem_euclid(l as i64) as usize);
    }
    let segments = p.i("segments")?;
    if segments < 1 || segments > 256 {
        return Err(Error::new("segments must be 1..256"));
    }
    let segments = segments as usize;
    let n = a.len();
    if n * segments > 1_000_000 {
        return Err(Error::new("bridge limit: one million profile samples"));
    }
    let (pv, _, pr) = corners(&g);
    let vp = &g.topology.vertex_point;
    let e = topo::edges(&g.topology, g.npoints);
    let profile_corners = |path: &[usize], m: &[bool]| {
        let mut hit: Vec<_> = e
            .of_vertex
            .iter()
            .map(|&i| i >= 0 && m[i as usize])
            .collect();
        let old = hit.clone();
        for v in 0..g.nvertices() {
            hit[v] |= old[pr[v]] && g.topology.is_closed(pv[v]);
        }
        path.iter()
            .map(|&q| {
                (0..g.nvertices())
                    .find(|&v| vp[v] as usize == q && hit[v])
                    .or_else(|| (0..g.nvertices()).find(|&v| vp[v] as usize == q))
                    .unwrap()
            })
            .collect::<Vec<_>>()
    };
    let va = profile_corners(&a, &am);
    let vb = profile_corners(&b, &bm);
    let mut point = g.point.clone();
    let mut rings = vec![a.clone()];
    for k in 1..segments {
        rings.push((g.npoints + (k - 1) * n..g.npoints + k * n).collect());
        point = concat(
            &point,
            &lerp(&g.point, &a, &b, &vec![k as f64 / segments as f64; n]),
        );
    }
    rings.push(b.clone());
    let mut outvp = vp.clone();
    let mut sizes: Vec<_> = (0..g.nprims()).map(|i| range(&g, i).len()).collect();
    let mut ia = Vec::new();
    let mut ib = Vec::new();
    let mut fr = Vec::new();
    let mut ps = Vec::new();
    let positions = point["P"].as_f32().unwrap();
    let pos = |q: usize| {
        [
            positions[3 * q] as f64,
            positions[3 * q + 1] as f64,
            positions[3 * q + 2] as f64,
        ]
    };
    for k in 0..segments {
        for j in 0..if ca { n } else { n - 1 } {
            let h = (j + 1) % n;
            let f = [rings[k][j], rings[k][h], rings[k + 1][h], rings[k + 1][j]];
            let q = f.map(pos);
            if len(cross(sub(q[1], q[0]), sub(q[2], q[0])))
                + len(cross(sub(q[2], q[0]), sub(q[3], q[0])))
                < 1e-12
            {
                return Err(Error::new("bridge contains a collapsed face"));
            }
            outvp.extend(f.map(|q| q as i32));
            sizes.push(4);
            ia.extend([va[j], va[h], va[h], va[j]]);
            ib.extend([vb[j], vb[h], vb[h], vb[j]]);
            fr.extend([
                k as f64 / segments as f64,
                k as f64 / segments as f64,
                (k + 1) as f64 / segments as f64,
                (k + 1) as f64 / segments as f64,
            ]);
            ps.push(pv[va[j]]);
        }
    }
    let mut closed = g.topology.closed_mask();
    closed.extend(std::iter::repeat_n(true, ps.len()));
    let mut out = make(
        point,
        outvp,
        &sizes,
        concat(&g.vertex, &lerp(&g.vertex, &ia, &ib, &fr)),
        concat(&g.prim, &gather(&g.prim, &ps)),
        g.detail.clone(),
        Some(closed),
    )?;
    remap(&g, &mut out, None)?;
    outgroup(
        &mut out,
        p,
        "bridge",
        "prim",
        &(0..sizes.len())
            .map(|i| i >= g.nprims())
            .collect::<Vec<_>>(),
    )?;
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn polybridge_uses_two_group_create_edge_loops_on_box() {
        let box_params = Params::resolve(super::super::generate::BOX.params, None).unwrap();
        let box_geo = (super::super::generate::BOX.cook)(&[vec![]], &box_params).unwrap();
        // Open the two caps so the selected loops are bridgeable boundaries.
        let keep = super::super::group::centroids(&box_geo).chunks_exact(3)
            .map(|p| p[2].abs() < 0.25).collect::<Vec<_>>();
        let mut g = super::super::group::keep_prims(&box_geo, &keep, false).unwrap();
        let edges = topo::edges(&g.topology, g.npoints);
        for (name, z) in [("group1", -0.5), ("group2", 0.5)] {
            let pattern = edges.points.chunks_exact(2).filter_map(|pair| {
                let a = pair[0] as usize;
                let b = pair[1] as usize;
                (g.p()[3*a+2] == z && g.p()[3*b+2] == z)
                    .then(|| format!("{}-{}", a, b))
            }).collect::<Vec<_>>().join(" ");
            let params = Params::resolve(super::super::group::GROUP_CREATE.params,
                json!({"name":name,"domain":"edges","pattern":pattern}).as_object()).unwrap();
            g = (super::super::group::GROUP_CREATE.cook)(&[vec![Arc::new(g)],vec![]], &params).unwrap();
            assert_eq!(g.edge[&format!("group_{name}")].as_i32().unwrap().iter().filter(|&&v| v != 0).count(), 4);
        }
        let params = Params::resolve(POLYBRIDGE.params,
            json!({"source":"group1","destination":"group2"}).as_object()).unwrap();
        let out = polybridge(&[vec![Arc::new(g)]], &params).unwrap();
        assert_eq!(out.nprims(), 8);
        out.validate().unwrap();
    }
    #[test]
    fn edge_groups_and_crease_live_on_outgoing_corners() {
        let g = Geometry::from_arrays(
            vec![0., 0., 0., 1., 0., 0., 1., 1., 0., 0., 1., 0.],
            vec![0, 1, 2, 0, 2, 3],
            vec![0, 3, 6],
            None,
        )
        .unwrap();
        let p = Params::resolve(
            CREASE.params,
            json!({"group":"p0-2","sharpness":3}).as_object(),
        )
        .unwrap();
        let out = crease(&[vec![Arc::new(g)]], &p).unwrap();
        assert_eq!(
            values(&out, "creaseweight").unwrap(),
            vec![0., 3., 0., 0., 0.]
        );
        assert_eq!(
            out.vertex["creaseweight"].data,
            AttrData::F32(vec![0., 0., 3., 3., 0., 0.])
        );
    }
    #[test]
    fn dissolve_rejects_material_seams() {
        let mut g = Geometry::from_arrays(
            vec![0., 0., 0., 1., 0., 0., 1., 1., 0., 0., 1., 0.],
            vec![0, 1, 2, 0, 2, 3],
            vec![0, 3, 6],
            None,
        )
        .unwrap();
        let p = Params::resolve(DISSOLVE.params, json!({"group":"p0-2"}).as_object()).unwrap();
        let out = dissolve(&[vec![Arc::new(g.clone())]], &p).unwrap();
        assert_eq!(out.nprims(), 1);
        assert_eq!(out.nvertices(), 4);
        g.prim
            .insert("material_id".into(), Arc::new(Attr::i32(1, vec![1, 2])));
        assert!(dissolve(&[vec![Arc::new(g)]], &p).is_err());
    }
}
