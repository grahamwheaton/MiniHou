use crate::error::{Error, Result};
use crate::geometry::{Attr, AttrData, Geometry};
use crate::nodes::{edge_tools, model, Inputs, NodeSpec, GEO};
use crate::params::Params;
use crate::{normals, topo};
use std::collections::BTreeMap;

pub(super) const NODE: NodeSpec = NodeSpec {
    name: "dissolve_flat_edges",
    inputs: GEO,
    params: crate::params!(
        "tolerance": 0.01;
        "group": "";
    ),
    cook: dissolve_flat_edges,
};

fn root(parent: &mut [usize], mut x: usize) -> usize {
    while parent[x] != x {
        parent[x] = parent[parent[x]];
        x = parent[x];
    }
    x
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn len(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}

fn unit(a: [f64; 3]) -> [f64; 3] {
    let l = len(a);
    if l < 1e-12 {
        [0.0, 0.0, 0.0]
    } else {
        [a[0] / l, a[1] / l, a[2] / l]
    }
}

fn dissolve_flat_edges(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = NODE.one(inputs, 0)?;
    let tolerance = p.f("tolerance")?;
    let group = p.s("group")?;

    model::polygons(&g, "dissolve_flat_edges")?;
    if g.nprims() <= 1 {
        return Ok((*g).clone());
    }

    let e = topo::edges(&g.topology, g.npoints);
    let chosen = if group.trim().is_empty() {
        vec![true; e.len()]
    } else {
        edge_tools::edge_mask(&g, &group)?
    };

    let (pv, nx, _) = model::corners(&g);
    let vp = &g.topology.vertex_point;
    let fn_ = normals::prim_normals(g.p(), &g.topology);

    let cos_tol = tolerance.to_radians().cos();

    let mut edge_verts: Vec<Vec<usize>> = vec![Vec::new(); e.len()];
    for v in 0..g.nvertices() {
        let eid = e.of_vertex[v];
        if eid >= 0 && (eid as usize) < e.len() {
            edge_verts[eid as usize].push(v);
        }
    }

    let mut dissolved = vec![false; e.len()];
    let mut parent: Vec<usize> = (0..g.nprims()).collect();

    for eid in 0..e.len() {
        if !chosen[eid] {
            continue;
        }
        if e.count[eid] != 2 {
            continue;
        }
        let vv = &edge_verts[eid];
        if vv.len() != 2 {
            continue;
        }
        let (u, v) = (vv[0], vv[1]);
        let (a, b) = (pv[u], pv[v]);
        if a == b {
            continue;
        }
        if vp[u] != vp[nx[v]] || vp[v] != vp[nx[u]] {
            continue;
        }
        let na = unit(fn_[a]);
        let nb = unit(fn_[b]);
        if len(fn_[a]) < 1e-12 || len(fn_[b]) < 1e-12 {
            continue;
        }
        let d = dot(na, nb);
        if d >= cos_tol - 1e-9 {
            dissolved[eid] = true;
            let ra = root(&mut parent, a);
            let rb = root(&mut parent, b);
            parent[rb] = ra;
        }
    }

    if !dissolved.iter().any(|&d| d) {
        return Ok((*g).clone());
    }

    let mut regions = BTreeMap::<usize, Vec<usize>>::new();
    for i in 0..g.nprims() {
        let r = root(&mut parent, i);
        regions.entry(r).or_default().push(i);
    }
    let mut regions: Vec<_> = regions.into_values().collect();
    regions.sort_by_key(|r| r[0]);

    let mut faces = Vec::new();
    let mut sources = Vec::new();

    for r in &regions {
        let vs: Vec<_> = (0..g.nvertices())
            .filter(|&v| r.contains(&pv[v]) && !dissolved[e.of_vertex[v] as usize])
            .collect();
        let f = if r.len() == 1 {
            vs
        } else {
            let mut by_start = BTreeMap::new();
            for &v in &vs {
                if by_start.insert(vp[v], v).is_some() {
                    return Err(Error::new(
                        "dissolve region has a hole, repeated corner or branching boundary",
                    ));
                }
            }
            let mut f = Vec::new();
            let mut at = vs.first().copied();
            while let Some(v) = at {
                if f.contains(&v) {
                    break;
                }
                f.push(v);
                at = by_start.get(&vp[nx[v]]).copied();
            }
            if f.len() < 3 || f.len() != vs.len() || at != f.first().copied() {
                return Err(Error::new(
                    "dissolve region must have one simple closed boundary",
                ));
            }
            f
        };
        faces.push(f);
        sources.push(r[0]);
    }

    let mut out = edge_tools::rebuild_edges(&g, &faces, &sources, None)?;

    for (name, a) in &g.prim {
        if name.starts_with("group_") {
            let w = a.width;
            let attr = match &a.data {
                AttrData::F32(v) => Attr::f32(
                    w,
                    regions
                        .iter()
                        .flat_map(|r| {
                            (0..w).map(move |k| {
                                r.iter()
                                    .map(|&i| v[i * w + k])
                                    .fold(f32::NEG_INFINITY, f32::max)
                            })
                        })
                        .collect(),
                ),
                AttrData::I32(v) => Attr::i32(
                    w,
                    regions
                        .iter()
                        .flat_map(|r| {
                            (0..w).map(move |k| r.iter().map(|&i| v[i * w + k]).max().unwrap_or(0))
                        })
                        .collect(),
                ),
            };
            out.prim.insert(name.clone(), std::sync::Arc::new(attr));
        }
    }

    out = model::compact(out)?;
    Ok(out)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
