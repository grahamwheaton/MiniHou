//! Modeling tools, including quad-strip loop cuts, from modeling_tools.py.
use super::{GEO, Inputs, NodeSpec, edge_tools as edges, groups, model::*};
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
pub const SOFT_PEAK: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None,
    name: "soft_peak",
    inputs: GEO,
    params: crate::params!("distance":0.3;"radius":1.;"falloff":"smooth";"group":""),
    cook: soft_peak,
};
pub const GROUP_RANGE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None,
    name: "group_range",
    inputs: GEO,
    params: crate::params!("name":"group1";"domain":"primitives";"start":0;"end":-1;"select":1;"every":1;"offset":0;"invert":false;"combine":"replace"),
    cook: group_range,
};
pub const EDGE_COLLAPSE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None,
    name: "edge_collapse",
    inputs: GEO,
    params: crate::params!("group":"";"outgrp_collapsed":false;"outgrp_collapsed_name":"collapsedpoints"),
    cook: edge_collapse,
};
pub const EDGE_FLIP: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None,
    name: "edge_flip",
    inputs: GEO,
    params: crate::params!("group":"";"outgrp_flipped":false;"outgrp_flipped_name":"flippededges"),
    cook: edge_flip,
};
pub const EDGE_CUSP: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None,
    name: "edge_cusp",
    inputs: GEO,
    params: crate::params!("group":"";"cusp_angle":180.),
    cook: edge_cusp,
};
pub const POLYSPLIT: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None,
    name: "polysplit",
    inputs: GEO,
    params: crate::params!("mode":"edge_loop";"split_edges":"";"loops":1;"position":0.5;"factor":0.;"smoothness":0.;"falloff":"inverse_square";"even":false;"flipped":false;"clamp":true;"correct_uvs":true;"split_points":"";"outgrp_points":false;"outgrp_points_name":"splitpoints";"outgrp_edges":false;"outgrp_edges_name":"splitedges"),
    cook: polysplit,
};
// Native keymap calls this tool loop_cut; Python's node type is polysplit.
pub const LOOP_CUT: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None,
    name: "loop_cut",
    inputs: GEO,
    params: POLYSPLIT.params,
    cook: polysplit,
};
pub const EDGE_SLIDE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None,
    name: "edge_slide",
    inputs: GEO,
    params: crate::params!("group":"";"factor":0.;"even":false;"flipped":false;"clamp":true;"mirror_editing":true;"correct_uvs":true;"output_group":false;"output_group_name":"slid_edges"),
    cook: edge_slide,
};
pub(super) fn twins(g: &Geometry) -> Vec<Option<usize>> {
    let (_, nx, _) = corners(g);
    let vp = &g.topology.vertex_point;
    let mut directed = BTreeMap::<(i32, i32), Vec<usize>>::new();
    for v in 0..g.nvertices() {
        directed.entry((vp[v], vp[nx[v]])).or_default().push(v);
    }
    let mut tw = vec![None; g.nvertices()];
    for v in 0..g.nvertices() {
        if directed[&(vp[v], vp[nx[v]])].len() == 1 {
            if let Some(opp) = directed.get(&(vp[nx[v]], vp[v])) {
                if opp.len() == 1 {
                    tw[v] = Some(opp[0]);
                }
            }
        }
    }
    tw
}
pub(super) fn pair_mask(g: &Geometry, pairs: &[(usize, usize)]) -> Vec<bool> {
    let e = topo::edges(&g.topology, g.npoints);
    let pairs: BTreeSet<_> = pairs.iter().map(|&(a, b)| (a.min(b), a.max(b))).collect();
    e.points
        .chunks_exact(2)
        .map(|p| pairs.contains(&(p[0] as usize, p[1] as usize)))
        .collect()
}
pub(super) fn parse_pairs(text: &str, np: usize) -> Result<Vec<(usize, usize)>> {
    let mut pairs = BTreeSet::new();
    for token in text
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|s| !s.is_empty())
    {
        let (a, b) = token
            .split_once('-')
            .ok_or_else(|| Error::new("write point pairs like 12-13"))?;
        let a: usize = a.parse().map_err(|_| Error::new("invalid point pair"))?;
        let b: usize = b.parse().map_err(|_| Error::new("invalid point pair"))?;
        if a >= np || b >= np {
            return Err(Error::new("point pair is outside the geometry"));
        }
        if a != b {
            pairs.insert((a.min(b), a.max(b)));
        }
    }
    Ok(pairs.into_iter().collect())
}
fn soft_peak(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = SOFT_PEAK.one(inputs, 0)?;
    let text = p.s("group")?;
    let sel = groups::point_mask(&g, &text)?;
    if !sel.iter().any(|&b| b) {
        return Ok((*g).clone());
    }
    let radius = p.f("radius")?;
    if radius <= 0. {
        return Err(Error::new("radius must be positive"));
    }
    let falloff = p.e("falloff", &["smooth", "linear", "constant"])?;
    let n = if let Some(a) = g.point.get("N") {
        a.as_f32()
            .filter(|_| a.width == 3)
            .ok_or_else(|| Error::new("N must have three components"))?
            .to_vec()
    } else {
        normals::point_normals(g.p(), &g.topology, g.npoints)
    };
    let pos = positions(&g);
    let sources: Vec<_> = pos
        .iter()
        .zip(&sel)
        .filter(|(_, s)| **s)
        .map(|(&q, _)| q)
        .collect();
    let mut out = (*g).clone();
    let mut pts = Vec::new();
    for (i, &q) in pos.iter().enumerate() {
        let d = if text.trim().is_empty() {
            0.
        } else {
            sources
                .iter()
                .map(|&s| len(sub(q, s)))
                .fold(f64::INFINITY, f64::min)
        };
        let u = (1. - d / radius).clamp(0., 1.);
        let w = match falloff.as_str() {
            "smooth" => u * u * (3. - 2. * u),
            "linear" => u,
            _ => (d <= radius) as u8 as f64,
        };
        for k in 0..3 {
            pts.push((q[k] + n[3 * i + k] as f64 * w * p.f("distance")?) as f32);
        }
    }
    out.set_p(pts);
    Ok(out)
}
fn group_range(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = GROUP_RANGE.one(inputs, 0)?;
    let point = p.e("domain", &["primitives", "points"])? == "points";
    let n = if point { g.npoints } else { g.nprims() };
    let start = p.i("start")?;
    let end = p.i("end")?;
    let last = if end < 0 {
        n as i64 - 1
    } else {
        end.min(n as i64 - 1)
    };
    let select = p.i("select")?;
    let every = p.i("every")?;
    if every < 1 || select < 1 || start < 0 {
        return Err(Error::new("invalid group range"));
    }
    let offset = p.i("offset")?;
    let invert = p.b("invert")?;
    let mut mask: Vec<_> = (0..n)
        .map(|i| {
            let i = i as i64;
            (i >= start && i <= last && (i - start + offset).rem_euclid(every) < select.min(every))
                ^ invert
        })
        .collect();
    let name = p.s("name")?;
    group_name(&name)?;
    let old = if point {
        g.point.get(&format!("group_{name}"))
    } else {
        g.prim.get(&format!("group_{name}"))
    };
    let combine = p.e("combine", &["replace", "union", "intersect", "subtract"])?;
    for (i, m) in mask.iter_mut().enumerate() {
        let old = old.is_some_and(|a| match &a.data {
            AttrData::F32(v) => v[i * a.width] != 0.,
            AttrData::I32(v) => v[i * a.width] != 0,
        });
        *m = match combine.as_str() {
            "union" => old || *m,
            "intersect" => old && *m,
            "subtract" => old && !*m,
            _ => *m,
        };
    }
    let mut out = (*g).clone();
    write_group(&mut out, if point { "point" } else { "prim" }, &name, &mask)?;
    Ok(out)
}
fn edge_collapse(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = EDGE_COLLAPSE.one(inputs, 0)?;
    let text = p.s("group")?;
    if text.trim().is_empty() {
        return Ok((*g).clone());
    }
    polygons(&g, "edge_collapse")?;
    let e = topo::edges(&g.topology, g.npoints);
    let sel = edges::edge_mask(&g, &text)?;
    if !sel.iter().any(|&b| b) {
        return Ok((*g).clone());
    }
    let mut parent: Vec<_> = (0..g.npoints).collect();
    for (i, q) in e.points.chunks_exact(2).enumerate() {
        if sel[i] {
            let a = edges::root(&mut parent, q[0] as usize);
            let b = edges::root(&mut parent, q[1] as usize);
            parent[a.max(b)] = a.min(b);
        }
    }
    let roots: Vec<_> = (0..g.npoints)
        .map(|i| edges::root(&mut parent, i))
        .collect();
    let unique: Vec<_> = roots
        .iter()
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let labels: Vec<_> = roots
        .iter()
        .map(|r| unique.binary_search(r).unwrap())
        .collect();
    let mut buckets = vec![Vec::new(); unique.len()];
    for (i, &r) in labels.iter().enumerate() {
        buckets[r].push(i);
    }
    let point = mean(&g.point, &buckets);
    let mut faces = Vec::new();
    let mut ps = Vec::new();
    for i in 0..g.nprims() {
        let r = range(&g, i);
        let mut f = Vec::new();
        for v in r.clone() {
            let next = if v + 1 < r.end { v + 1 } else { r.start };
            if labels[g.topology.vertex_point[v] as usize]
                != labels[g.topology.vertex_point[next] as usize]
                || r.len() == 1
            {
                f.push(v);
            }
        }
        if f.len() >= 3 {
            faces.push(f);
            ps.push(i);
        }
    }
    let ix: Vec<_> = faces.iter().flatten().copied().collect();
    let mut out = make(
        point,
        ix.iter()
            .map(|&v| labels[g.topology.vertex_point[v] as usize] as i32)
            .collect(),
        &faces.iter().map(Vec::len).collect::<Vec<_>>(),
        gather(&g.vertex, &ix),
        gather(&g.prim, &ps),
        g.detail.clone(),
        None,
    )?;
    outgroup(
        &mut out,
        p,
        "collapsed",
        "point",
        &buckets.iter().map(|b| b.len() > 1).collect::<Vec<_>>(),
    )?;
    Ok(out)
}
fn edge_flip(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = EDGE_FLIP.one(inputs, 0)?;
    let text = p.s("group")?;
    if text.trim().is_empty() {
        return Ok((*g).clone());
    }
    polygons(&g, "edge_flip")?;
    let e = topo::edges(&g.topology, g.npoints);
    let sel = edges::edge_mask(&g, &text)?;
    let tw = twins(&g);
    let (pv, nx, pr) = corners(&g);
    let mut used = vec![false; g.nprims()];
    let mut replace = BTreeMap::new();
    let mut pairs = Vec::new();
    for i in 0..g.nvertices() {
        let Some(j) = tw[i] else {
            continue;
        };
        if j < i || e.of_vertex[i] < 0 || !sel[e.of_vertex[i] as usize] {
            continue;
        }
        let (a, b) = (pv[i], pv[j]);
        if a == b || range(&g, a).len() != 3 || range(&g, b).len() != 3 || used[a] || used[b] {
            continue;
        }
        used[a] = true;
        used[b] = true;
        let (c, d) = (pr[i], pr[j]);
        if g.topology.vertex_point[c] == g.topology.vertex_point[d] {
            continue;
        }
        replace.insert(a, vec![i, d, c]);
        replace.insert(b, vec![d, nx[i], c]);
        pairs.push((
            g.topology.vertex_point[c] as usize,
            g.topology.vertex_point[d] as usize,
        ));
    }
    if replace.is_empty() {
        return Ok((*g).clone());
    }
    let faces: Vec<_> = (0..g.nprims())
        .map(|i| replace.remove(&i).unwrap_or_else(|| range(&g, i).collect()))
        .collect();
    let mut out = edges::rebuild_edges(
        &g,
        &faces,
        &(0..g.nprims()).collect::<Vec<_>>(),
        g.topology.closed.clone(),
    )?;
    let mask = pair_mask(&out, &pairs);
    outgroup(&mut out, p, "flipped", "edge", &mask)?;
    Ok(out)
}
fn edge_cusp(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = EDGE_CUSP.one(inputs, 0)?;
    polygons(&g, "edge_cusp")?;
    let text = p.s("group")?;
    let e = topo::edges(&g.topology, g.npoints);
    let hard = if text.trim().is_empty() {
        vec![false; e.len()]
    } else {
        edges::edge_mask(&g, &text)?
    };
    let fn_ = normals::prim_normals(g.p(), &g.topology);
    let un: Vec<_> = fn_.iter().map(|&v| unit32(v)).collect();
    let (pv, nx, _) = corners(&g);
    let tw = twins(&g);
    let mut parent: Vec<_> = (0..g.nvertices()).collect();
    let cos = p.f("cusp_angle")?.to_radians().cos();
    for i in 0..g.nvertices() {
        let Some(j) = tw[i] else {
            continue;
        };
        if e.of_vertex[i] < 0
            || hard[e.of_vertex[i] as usize]
            || dot(un[pv[i]], un[pv[j]]) < cos - 1e-9
        {
            continue;
        }
        for (a, b) in [(i, nx[j]), (j, nx[i])] {
            let a = edges::root(&mut parent, a);
            let b = edges::root(&mut parent, b);
            parent[a.max(b)] = a.min(b);
        }
    }
    let labels: Vec<_> = (0..g.nvertices())
        .map(|i| edges::root(&mut parent, i))
        .collect();
    let mut acc = vec![[0.; 3]; g.nvertices()];
    for v in 0..g.nvertices() {
        acc[labels[v]] = add(acc[labels[v]], fn_[pv[v]]);
    }
    let mut out = (*g).clone();
    out.vertex.insert(
        "N".into(),
        Arc::new(Attr::f32(
            3,
            labels
                .iter()
                .flat_map(|&i| unit32(acc[i]).map(|x| x as f32))
                .collect(),
        )),
    );
    out.point.remove("N");
    out.prim.remove("N");
    Ok(out)
}
pub const TWIST: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None,
    name: "twist",
    inputs: GEO,
    params: crate::params!("strength":90.;"origin":[0.,0.,0.];"direction":[0.,1.,0.];"group":""),
    cook: twist,
};
pub const LINEAR_TAPER: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None,
    name: "linear_taper",
    inputs: GEO,
    params: crate::params!("taper_x":0.5;"taper_y":0.5;"length":1.;"dropoff":1.;"origin":[0.,0.,0.];"direction":[0.,1.,0.];"up":[0.,0.,1.];"symmetric":false;"group":""),
    cook: taper,
};
pub const SQUASH_STRETCH: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None,
    name: "squash_stretch",
    inputs: GEO,
    params: crate::params!("amount":1.5;"preserve_volume":true;"center":[0.,0.,0.];"direction":[0.,1.,0.];"group":""),
    cook: squash,
};
fn direction(v: [f64; 3]) -> Result<[f64; 3]> {
    if len(v) < 1e-12 {
        return Err(Error::new("direction must not be the zero vector"));
    }
    Ok(unit(v))
}
fn twist(i: &Inputs, p: &Params) -> Result<Geometry> {
    let g = TWIST.one(i, 0)?;
    let d = direction(p.v3("direction")?)?;
    let o = p.v3("origin")?;
    let strength = p.f("strength")?.to_radians();
    apply(&g, &p.s("group")?, |q| {
        let r = sub(q, o);
        let t = dot(r, d);
        let (s, c) = (strength * t).sin_cos();
        add(
            o,
            add(add(mul(r, c), mul(cross(d, r), s)), mul(d, t * (1. - c))),
        )
    })
}
fn taper(i: &Inputs, p: &Params) -> Result<Geometry> {
    let g = LINEAR_TAPER.one(i, 0)?;
    let d = direction(p.v3("direction")?)?;
    let up = p.v3("up")?;
    let mut y = sub(up, mul(d, dot(up, d)));
    if len(y) < 1e-9 {
        y = cross(
            d,
            if d[0].abs() < 0.9 {
                [1., 0., 0.]
            } else {
                [0., 1., 0.]
            },
        );
    }
    y = unit(y);
    let x = cross(y, d);
    let o = p.v3("origin")?;
    let length = p.f("length")?;
    let dropoff = p.f("dropoff")?;
    if length < 1e-8 || dropoff < 0.01 {
        return Err(Error::new("invalid taper length or dropoff"));
    }
    let symmetric = p.b("symmetric")?;
    let tx = p.f("taper_x")?;
    let ty = p.f("taper_y")?;
    apply(&g, &p.s("group")?, |q| {
        let r = sub(q, o);
        let t = dot(r, d);
        let u = ((if symmetric { t.abs() } else { t }) / length)
            .max(0.)
            .powf(dropoff);
        add(
            o,
            add(
                add(
                    mul(x, dot(r, x) * (1. + (tx - 1.) * u)),
                    mul(y, dot(r, y) * (1. + (ty - 1.) * u)),
                ),
                mul(d, t),
            ),
        )
    })
}
fn squash(i: &Inputs, p: &Params) -> Result<Geometry> {
    let g = SQUASH_STRETCH.one(i, 0)?;
    let d = direction(p.v3("direction")?)?;
    let c = p.v3("center")?;
    let amount = p.f("amount")?;
    if amount < 0.01 {
        return Err(Error::new("amount must be at least 0.01"));
    }
    let side = if p.b("preserve_volume")? {
        1. / amount.sqrt()
    } else {
        1.
    };
    apply(&g, &p.s("group")?, |q| {
        let r = sub(q, c);
        let a = mul(d, dot(r, d));
        add(c, add(mul(a, amount), mul(sub(r, a), side)))
    })
}
fn normal_map(j: [[f64; 3]; 3], n: [f64; 3]) -> [f64; 3] {
    let transpose: [[f64; 3]; 3] = std::array::from_fn(|i| std::array::from_fn(|k| j[k][i]));
    unit32(least_squares(&transpose, &n, 1e-15))
}

fn apply<F: Fn([f64; 3]) -> [f64; 3]>(g: &Geometry, text: &str, map: F) -> Result<Geometry> {
    let pos = positions(g);
    if pos.iter().flatten().any(|x| !x.is_finite()) {
        return Err(Error::new("deformation requires finite positions"));
    }
    let sel = groups::point_mask(g, text)?;
    if !sel.iter().any(|&b| b) {
        return Ok(g.clone());
    }
    let mut out = g.clone();
    let mut newp = Vec::new();
    for (i, &q) in pos.iter().enumerate() {
        let q = if sel[i] { map(q) } else { q };
        if q.iter().any(|x| !x.is_finite()) {
            return Err(Error::new("deformation produced nonfinite positions"));
        }
        newp.extend(q.map(|x| x as f32));
    }
    out.set_p(newp);
    if ![&g.point, &g.vertex, &g.prim]
        .iter()
        .any(|t| t.contains_key("N"))
    {
        return Ok(out);
    }
    let mut extent = 1.0f64;
    for k in 0..3 {
        let lo = pos.iter().map(|q| q[k]).fold(f64::INFINITY, f64::min);
        let hi = pos.iter().map(|q| q[k]).fold(f64::NEG_INFINITY, f64::max);
        extent = extent.max(hi - lo);
    }
    let h = extent * 1e-5;
    let mut jac = vec![[[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]]; g.npoints];
    for (i, &q) in pos.iter().enumerate() {
        if sel[i] {
            for k in 0..3 {
                let mut a = q;
                let mut b = q;
                a[k] += h;
                b[k] -= h;
                let col = mul(sub(map(a), map(b)), 1. / (2. * h));
                for j in 0..3 {
                    jac[i][j][k] = col[j];
                }
            }
        }
    }
    for domain in ["point", "vertex", "prim"] {
        let table = g.table(domain);
        let Some(a) = table.get("N") else {
            continue;
        };
        let data = a
            .as_f32()
            .filter(|_| a.width == 3)
            .ok_or_else(|| Error::new("N must have three components"))?;
        let mut result = data.to_vec();
        for i in 0..a.rows() {
            let (j, active) = match domain {
                "point" => (jac[i], sel[i]),
                "vertex" => {
                    let q = g.topology.vertex_point[i] as usize;
                    (jac[q], sel[q])
                }
                _ => {
                    let mut j = [[0.; 3]; 3];
                    let mut active = false;
                    let r = range(g, i);
                    for v in r.clone() {
                        let q = g.topology.vertex_point[v] as usize;
                        active |= sel[q];
                        for x in 0..3 {
                            for y in 0..3 {
                                j[x][y] += jac[q][x][y] / r.len().max(1) as f64;
                            }
                        }
                    }
                    (j, active)
                }
            };
            if active {
                let n = normal_map(
                    j,
                    [
                        data[3 * i] as f64,
                        data[3 * i + 1] as f64,
                        data[3 * i + 2] as f64,
                    ],
                );
                result[3 * i..3 * i + 3].copy_from_slice(&n.map(|x| x as f32));
            }
        }
        let attr = Arc::new(Attr::f32(3, result));
        match domain {
            "point" => {
                out.point.insert("N".into(), attr);
            }
            "vertex" => {
                out.vertex.insert("N".into(), attr);
            }
            _ => {
                out.prim.insert("N".into(), attr);
            }
        }
    }
    Ok(out)
}
type Pair = (usize, usize);
fn key(a: usize, b: usize) -> Pair {
    (a.min(b), a.max(b))
}
fn walk(
    owners: &BTreeMap<Pair, Vec<usize>>,
    quads: &BTreeMap<usize, Vec<usize>>,
    mut p: usize,
    mut f: usize,
    mut t: usize,
    start: Pair,
) -> Vec<(usize, Pair, Pair)> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    loop {
        if !seen.insert(p) {
            break;
        }
        let Some(q) = quads.get(&p) else {
            break;
        };
        let Some(k) = q.iter().position(|&v| v == f) else {
            break;
        };
        let (a, b) = if q[(k + 1) % 4] == t {
            (q[(k + 3) % 4], q[(k + 2) % 4])
        } else if q[(k + 3) % 4] == t {
            (q[(k + 1) % 4], q[(k + 2) % 4])
        } else {
            break;
        };
        out.push((p, (f, t), (a, b)));
        let edge = key(a, b);
        if edge == start {
            break;
        }
        let nb: Vec<_> = owners[&edge].iter().copied().filter(|&i| i != p).collect();
        if nb.len() != 1 {
            break;
        }
        p = nb[0];
        f = a;
        t = b;
    }
    out
}
/// Fewest/most loops one polysplit (and the viewport Loop Cut count) allows, as in Python's `Int("loops", min=1, max=64)`.
pub const LOOP_CUT_MAX: usize = 64;
/// Where the new points sit along each ring edge, measured from its "from" end: `position` for a single loop, else even.
pub fn loop_cut_fractions(loops: usize, position: f64) -> Vec<f64> {
    if loops <= 1 {
        vec![position]
    } else {
        (0..loops).map(|j| (j + 1) as f64 / (loops + 1) as f64).collect()
    }
}
/// Signed displacement of all cuts by one interval. Zero preserves polysplit's Python semantics.
pub fn loop_slide_fractions(loops: usize, position: f64, factor: f64, clamp: bool) -> Vec<f64> {
    if factor == 0. { return loop_cut_fractions(loops, position); }
    let f = if clamp { factor.clamp(-1., 1.) } else { factor };
    let base = if loops == 1 { 0.5 } else { 0. };
    (0..loops).map(|j| if loops == 1 { base + 0.5 * f } else { (j + 1) as f64 / (loops + 1) as f64 + f / (loops + 1) as f64 }).collect()
}
/// Edge/quad adjacency polysplit walks. Shared with the viewport Loop Cut preview so hover and cook agree exactly.
#[derive(Clone, Debug, Default)]
pub struct LoopTopology {
    owners: BTreeMap<Pair, Vec<usize>>,
    quads: BTreeMap<usize, Vec<usize>>,
}
/// The ring edges a set of seeds cuts (undirected key -> from/to; fractions count from `from`)
/// and the quads that become strips (quad -> entry edge, opposite edge).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LoopRing {
    pub dirs: BTreeMap<Pair, Pair>,
    pub quads: BTreeMap<usize, (Pair, Pair)>,
}
impl LoopTopology {
    pub fn new(g: &Geometry) -> Self {
        Self::with_edges(g, &topo::edges(&g.topology, g.npoints))
    }
    fn with_edges(g: &Geometry, e: &topo::Edges) -> Self {
        let (pv, _, _) = corners(g);
        let vp = &g.topology.vertex_point;
        let mut owners = BTreeMap::<Pair, Vec<usize>>::new();
        for v in 0..g.nvertices() {
            if e.of_vertex[v] < 0 {
                continue;
            }
            let i = e.of_vertex[v] as usize;
            owners
                .entry((e.points[2 * i] as usize, e.points[2 * i + 1] as usize))
                .or_default()
                .push(pv[v]);
        }
        let quads = (0..g.nprims())
            .filter(|&i| range(g, i).len() == 4)
            .map(|i| (i, range(g, i).map(|v| vp[v] as usize).collect::<Vec<_>>()))
            .collect();
        Self { owners, quads }
    }
    /// Every edge as a sorted point pair.
    pub fn edges(&self) -> impl Iterator<Item = Pair> + '_ {
        self.owners.keys().copied()
    }
    /// Walk each seed's quad strip on both sides until a non-quad, a boundary, a non-manifold edge or the start.
    /// Seeds are sorted point pairs; an edge with no faces or more than two is skipped (like Python's `_walk`).
    pub fn ring(&self, seeds: impl IntoIterator<Item = Pair>) -> LoopRing {
        let mut out = LoopRing::default();
        for (a, b) in seeds {
            let k = key(a, b);
            let Some(owners) = self.owners.get(&k) else {
                continue;
            };
            if out.dirs.contains_key(&k) || !(1..=2).contains(&owners.len()) {
                continue;
            }
            out.dirs.insert(k, k);
            for &face in owners {
                for (q, entry, opp) in walk(&self.owners, &self.quads, face, k.0, k.1, k) {
                    if out.quads.contains_key(&q) {
                        continue;
                    }
                    out.quads.insert(q, (entry, opp));
                    for (f, t) in [entry, opp] {
                        out.dirs.entry(key(f, t)).or_insert((f, t));
                    }
                }
            }
        }
        out
    }
}
impl LoopRing {
    /// The new points on edge a-b listed from a (indices 0..n into that edge's points) and their fractions from a.
    pub fn along(&self, fractions: &[f64], a: usize, b: usize) -> (Vec<usize>, Vec<f64>) {
        let mut ids: Vec<_> = (0..fractions.len()).collect();
        let mut fs = fractions.to_vec();
        if self.dirs.get(&key(a, b)).is_some_and(|d| d.0 != a) {
            ids.reverse();
            fs.reverse();
            for x in &mut fs {
                *x = 1. - *x;
            }
        }
        (ids, fs)
    }
    /// World-space segments of the new edges across each split quad: exactly where polysplit puts them.
    pub fn cut_lines(&self, p: &[f32], fractions: &[f64]) -> Vec<([f32; 3], [f32; 3])> {
        let at = |a: usize, b: usize, f: f64| {
            [0, 1, 2].map(|k| (p[3 * a + k] as f64 * (1. - f) + p[3 * b + k] as f64 * f) as f32)
        };
        let mut out = Vec::new();
        for &(entry, opp) in self.quads.values() {
            let (_, fa) = self.along(fractions, entry.0, entry.1);
            let (_, fd) = self.along(fractions, opp.0, opp.1);
            for (x, y) in fa.iter().zip(&fd) {
                out.push((at(entry.0, entry.1, *x), at(opp.0, opp.1, *y)));
            }
        }
        out
    }
}
/// Slide selected edge chains along the unselected face rails. Each face supplies a rail
/// on its side of an oriented chain, so open borders have one available direction.
fn slide_selected_edges(g: &Geometry, selected: &BTreeSet<Pair>, factor: f64, even: bool,
    flipped: bool, clamp: bool, correct_uvs: bool) -> Geometry {
    let mut out = g.clone();
    if selected.is_empty() || factor == 0. { return out; }
    let mut adjacency = BTreeMap::<usize, Vec<usize>>::new();
    for &(a,b) in selected { adjacency.entry(a).or_default().push(b); adjacency.entry(b).or_default().push(a); }
    for n in adjacency.values_mut() { n.sort_unstable(); }
    let mut oriented = Vec::new();
    let mut visited = BTreeSet::new();
    while visited.len() < selected.len() {
        let start = adjacency.iter().find(|(v, n)| n.len() == 1 && n.iter().any(|&u| !visited.contains(&key(**v,u))))
            .map(|(v,_)| *v).or_else(|| adjacency.iter().find(|(v,n)| n.iter().any(|&u| !visited.contains(&key(**v,u)))).map(|(v,_)| *v));
        let Some(mut current) = start else { break; };
        loop {
            let Some(next) = adjacency[&current].iter().copied().find(|&u| !visited.contains(&key(current,u))) else { break; };
            visited.insert(key(current,next)); oriented.push((current,next)); current = next;
        }
    }
    let vp = &g.topology.vertex_point;
    let mut rails = BTreeMap::<usize,[Option<usize>;2]>::new();
    for &(a,b) in &oriented {
        for face in 0..g.nprims() {
            if !g.topology.is_closed(face) { continue; }
            let r = range(g,face);
            if r.len() < 3 { continue; }
            let corners: Vec<_> = r.collect();
            for i in 0..corners.len() {
                let x = vp[corners[i]] as usize;
                let y = vp[corners[(i+1)%corners.len()]] as usize;
                let (side, u, v, before, after) = if (x,y)==(a,b) {
                    (0,a,b,vp[corners[(i+corners.len()-1)%corners.len()]] as usize,vp[corners[(i+2)%corners.len()]] as usize)
                } else if (x,y)==(b,a) {
                    (1,a,b,vp[corners[(i+2)%corners.len()]] as usize,vp[corners[(i+corners.len()-1)%corners.len()]] as usize)
                } else { continue; };
                if !selected.contains(&key(u,before)) { rails.entry(u).or_default()[side].get_or_insert(before); }
                if !selected.contains(&key(v,after)) { rails.entry(v).or_default()[side].get_or_insert(after); }
            }
        }
    }
    let side = (factor.is_sign_negative() as usize) ^ (flipped as usize);
    let reference = rails.iter().find_map(|(&v,r)| r[side].map(|n| (v,n)))
        .map(|(v,n)| (0..3).map(|k| (g.p()[3*v+k]-g.p()[3*n+k]).powi(2)).sum::<f32>().sqrt() as f64).unwrap_or(0.);
    let mut targets = BTreeMap::<usize,(usize,f64)>::new();
    for (&v,r) in &rails {
        let Some(n) = r[side] else { continue; };
        let distance=(0..3).map(|k| (g.p()[3*v+k]-g.p()[3*n+k]).powi(2)).sum::<f32>().sqrt() as f64;
        if distance <= 1e-12 { continue; }
        let amount=factor.abs() * if even && reference > 0. { reference/distance } else { 1. };
        targets.insert(v,(n,if clamp { amount.clamp(0.,1.) } else { amount }));
    }
    let mut positions=g.p().to_vec();
    for (&v,&(n,t)) in &targets { for k in 0..3 { positions[3*v+k]=(g.p()[3*v+k] as f64*(1.-t)+g.p()[3*n+k] as f64*t) as f32; } }
    out.set_p(positions);
    for (name,attr) in &g.point {
        if name=="P" || name.starts_with("group_") { continue; }
        if let AttrData::F32(data)=&attr.data {
            let mut data=data.clone();
            for (&v,&(n,t)) in &targets { for k in 0..attr.width { data[v*attr.width+k]=(attr.as_f32().unwrap()[v*attr.width+k] as f64*(1.-t)+attr.as_f32().unwrap()[n*attr.width+k] as f64*t) as f32; } }
            out.point.insert(name.clone(),Arc::new(Attr::f32(attr.width,data)));
        }
    }
    if correct_uvs {
        let (_,next,prev)=corners(g);
        for (name,attr) in &g.vertex {
            if name.starts_with("edge_group_") { continue; }
            if let AttrData::F32(data)=&attr.data {
                let mut changed=data.clone();
                for c in 0..g.nvertices() {
                    let v=vp[c] as usize;
                    let Some(&(n,t))=targets.get(&v) else { continue; };
                    let rail=if vp[next[c]] as usize==n { Some(next[c]) } else if vp[prev[c]] as usize==n { Some(prev[c]) } else { None };
                    if let Some(q)=rail { for k in 0..attr.width { changed[c*attr.width+k]=(data[c*attr.width+k] as f64*(1.-t)+data[q*attr.width+k] as f64*t) as f32; } }
                }
                out.vertex.insert(name.clone(),Arc::new(Attr::f32(attr.width,changed)));
            }
        }
    }
    out
}
/// Geometry used by the viewport preview. The node calls the same kernel after parsing its group.
pub fn edge_slide_preview(g: &Geometry, selected: &[(usize,usize)], factor: f64, even: bool,
    flipped: bool, clamp: bool, correct_uvs: bool) -> Geometry {
    slide_selected_edges(g,&selected.iter().map(|&(a,b)|key(a,b)).collect(),factor,even,flipped,clamp,correct_uvs)
}
/// Cook-backed preview for the modal, including Even and Flip on irregular rings.
pub fn loop_cut_preview(g: &Geometry, seed: Pair, count: usize, factor: f64, even: bool,
    flipped: bool, clamp: bool) -> Result<Vec<([f32;3],[f32;3])>> {
    let value=serde_json::json!({"split_edges":format!("p{}-{}",seed.0,seed.1),"loops":count,
        "factor":factor,"even":even,"flipped":flipped,"clamp":clamp,"outgrp_edges":true});
    let params=Params::resolve(POLYSPLIT.params,value.as_object())?;
    let result=polysplit(&[vec![Arc::new(g.clone())]],&params)?;
    let edges=topo::edges(&result.topology,result.npoints);
    let mask=edges::edge_mask(&result,"splitedges")?;
    let positions=result.p();
    Ok(edges.points.chunks_exact(2).enumerate().filter(|(i,_)|mask[*i]).map(|(_,pair)| {
        let point=|id:i64| [0,1,2].map(|k|positions[3*id as usize+k]);
        (point(pair[0]),point(pair[1]))
    }).collect())
}
fn edge_slide(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g=EDGE_SLIDE.one(inputs,0)?;
    polygons(&g,"edge_slide")?;
    let group=p.s("group")?;
    if group.trim().is_empty() { return Ok((*g).clone()); }
    let edges=topo::edges(&g.topology,g.npoints);
    let mask=edges::edge_mask(&g,&group)?;
    let selected: BTreeSet<_>=edges.points.chunks_exact(2).enumerate().filter(|(i,_)|mask[*i])
        .map(|(_,v)|key(v[0] as usize,v[1] as usize)).collect();
    let factor=p.f("factor")?;
    if !(-1. ..=1.).contains(&factor) { return Err(Error::new("factor must be -1..1")); }
    let mut out=slide_selected_edges(&g,&selected,factor,p.b("even")?,p.b("flipped")?,p.b("clamp")?,p.b("correct_uvs")?);
    // Mirror editing is reserved for the editor's symmetry context; without one, geometry is unchanged.
    let _=p.b("mirror_editing")?;
    if p.b("output_group")? { let name=p.s("output_group_name")?; group_name(&name)?;
        let mask=pair_mask(&out,&selected.iter().copied().collect::<Vec<_>>());
        write_group(&mut out,"edge",&name,&mask)?;
    }
    Ok(out)
}
fn polysplit(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = POLYSPLIT.one(inputs, 0)?;
    polygons(&g, "polysplit")?;
    if p.e("mode", &["edge_loop", "points"])? == "points" {
        return split_points(&g, p);
    }
    let text = p.s("split_edges")?;
    if text.trim().is_empty() {
        return Ok((*g).clone());
    }
    let e = topo::edges(&g.topology, g.npoints);
    let sel = edges::edge_mask(&g, &text)?;
    let (_, nx, _) = corners(&g);
    let vp = &g.topology.vertex_point;
    let lt = LoopTopology::with_edges(&g, &e);
    let quads = &lt.quads;
    let seeds = (0..e.len())
        .filter(|&i| sel[i])
        .map(|i| (e.points[2 * i] as usize, e.points[2 * i + 1] as usize));
    let loop_ring = lt.ring(seeds);
    let (ring, split) = (&loop_ring.dirs, &loop_ring.quads);
    if ring.is_empty() {
        return Ok((*g).clone());
    }
    let loops = p.i("loops")?;
    if loops < 1 || loops as i64 > LOOP_CUT_MAX as i64 {
        return Err(Error::new("loops must be 1..64"));
    }
    let n = loops as usize;
    let position = p.f("position")?;
    if !(0.0..=1.0).contains(&position) {
        return Err(Error::new("position must be 0..1"));
    }
    let factor=p.f("factor")?;
    if !(-1. ..=1.).contains(&factor) { return Err(Error::new("factor must be -1..1")); }
    let smoothness=p.f("smoothness")?;
    if !(-1. ..=1.).contains(&smoothness) { return Err(Error::new("smoothness must be -1..1")); }
    let falloff=p.e("falloff", &["inverse_square","linear","smooth"])?;
    let fractions = loop_cut_fractions(n,if factor==0. { position } else { 0.5 });
    let keys: Vec<_> = ring.keys().copied().collect();
    let mut ia = Vec::new();
    let mut ib = Vec::new();
    let mut fr = Vec::new();
    for k in &keys {
        let (a, b) = ring[k];
        for &f in &fractions {
            ia.push(a);
            ib.push(b);
            fr.push(f);
        }
    }
    let point = concat(&g.point, &lerp(&g.point, &ia, &ib, &fr));
    let along = |a: usize, b: usize| {
        let ki = keys.binary_search(&key(a, b)).unwrap();
        let (js, fs) = loop_ring.along(&fractions, a, b);
        let ids: Vec<_> = js.into_iter().map(|j| g.npoints + ki * n + j).collect();
        (ids, fs)
    };
    type Row = (usize, usize, f64, usize);
    let mut faces = Vec::<Vec<Row>>::new();
    let mut sources = Vec::new();
    let mut cut_pairs = Vec::new();
    for i in 0..g.nprims() {
        let r = range(&g, i);
        if let Some(&(entry, _)) = split.get(&i) {
            let q = &quads[&i];
            let k = q.iter().position(|&v| v == entry.0).unwrap();
            let forward = q[(k + 1) % 4] == entry.1;
            let order: Vec<_> = (0..4)
                .map(|s| {
                    if forward {
                        (k + s) % 4
                    } else {
                        (k + 4 - s) % 4
                    }
                })
                .collect();
            let ra: Vec<_> = order.iter().map(|&o| r.start + o).collect();
            let pa: Vec<_> = order.iter().map(|&o| q[o]).collect();
            let (ab, fa) = along(pa[0], pa[1]);
            let (dc, fd) = along(pa[3], pa[2]);
            let mut top = vec![(ra[0], ra[0], 0., pa[0])];
            let mut bot = vec![(ra[3], ra[3], 0., pa[3])];
            top.extend(ab.iter().zip(&fa).map(|(&id, &f)| (ra[0], ra[1], f, id)));
            bot.extend(dc.iter().zip(&fd).map(|(&id, &f)| (ra[3], ra[2], f, id)));
            top.push((ra[1], ra[1], 0., pa[1]));
            bot.push((ra[2], ra[2], 0., pa[2]));
            for j in 0..n + 1 {
                let mut f = vec![top[j], top[j + 1], bot[j + 1], bot[j]];
                if !forward {
                    f.reverse();
                }
                faces.push(f);
                sources.push(i);
            }
            for j in 1..n + 1 {
                cut_pairs.push((top[j].3, bot[j].3));
            }
            continue;
        }
        let mut face = Vec::new();
        for c in r {
            let a = vp[c] as usize;
            let b = vp[nx[c]] as usize;
            face.push((c, c, 0., a));
            if ring.contains_key(&key(a, b)) {
                let (ids, fs) = along(a, b);
                face.extend(ids.iter().zip(&fs).map(|(&id, &f)| (c, nx[c], f, id)));
            }
        }
        faces.push(face);
        sources.push(i);
    }
    let rows: Vec<_> = faces.iter().flatten().copied().collect();
    let ia: Vec<_> = rows.iter().map(|r| r.0).collect();
    let ib: Vec<_> = rows.iter().map(|r| r.1).collect();
    let ft: Vec<_> = rows.iter().map(|r| r.2).collect();
    let closed = g
        .topology
        .closed
        .as_ref()
        .map(|c| sources.iter().map(|&i| c[i]).collect());
    let mut out = make(
        point,
        rows.iter().map(|r| r.3 as i32).collect(),
        &faces.iter().map(Vec::len).collect::<Vec<_>>(),
        lerp(&g.vertex, &ia, &ib, &ft),
        gather(&g.prim, &sources),
        g.detail.clone(),
        closed,
    )?;
    edges::remap(&g, &mut out, None)?;
    let np = out.npoints;
    outgroup(
        &mut out,
        p,
        "points",
        "point",
        &(0..np).map(|i| i >= g.npoints).collect::<Vec<_>>(),
    )?;
    let mask = pair_mask(&out, &cut_pairs);
    outgroup(&mut out, p, "edges", "edge", &mask)?;
    if smoothness != 0. {
        let mut neighbours=BTreeMap::<usize,Vec<usize>>::new();
        for &(a,b) in &cut_pairs { neighbours.entry(a).or_default().push(b); neighbours.entry(b).or_default().push(a); }
        let old=out.p().to_vec(); let mut new=old.clone();
        let weight=match falloff.as_str() { "linear"=>smoothness, "smooth"=>smoothness.abs().powi(2)*(3.-2.*smoothness.abs())*smoothness.signum(), _=>smoothness.abs().powi(2)*smoothness.signum() };
        for (&v,n) in &neighbours { if n.len()!=2 { continue; }
            for k in 0..3 { let mid=(old[3*n[0]+k] as f64+old[3*n[1]+k] as f64)*0.5;
                new[3*v+k]=(old[3*v+k] as f64+(mid-old[3*v+k] as f64)*weight) as f32; }
        }
        out.set_p(new);
    }
    if factor != 0. {
        out=slide_selected_edges(&out,&cut_pairs.iter().map(|&(a,b)|key(a,b)).collect(),factor,
            p.b("even")?,p.b("flipped")?,p.b("clamp")?,p.b("correct_uvs")?);
    }
    Ok(out)
}
fn split_points(g: &Geometry, p: &Params) -> Result<Geometry> {
    let pairs = parse_pairs(&p.s("split_points")?, g.npoints)?;
    if pairs.is_empty() {
        return Ok(g.clone());
    }
    let mut faces: Vec<Vec<_>> = (0..g.nprims()).map(|i| range(g, i).collect()).collect();
    let mut sources: Vec<_> = (0..g.nprims()).collect();
    let mut cuts = Vec::new();
    for (a, b) in pairs {
        let mut found = false;
        for fi in 0..faces.len() {
            let f = &faces[fi];
            let ia = f
                .iter()
                .position(|&v| g.topology.vertex_point[v] as usize == a);
            let ib = f
                .iter()
                .position(|&v| g.topology.vertex_point[v] as usize == b);
            if let (Some(i), Some(j)) = (ia, ib) {
                if f.len() <= 3 {
                    continue;
                }
                let (i, j) = (i.min(j), i.max(j));
                if j - i < 2 || f.len() - (j - i) < 2 {
                    continue;
                }
                let f1 = f[i..=j].to_vec();
                let f2 = f[j..].iter().chain(&f[..=i]).copied().collect();
                faces[fi] = f1;
                faces.push(f2);
                sources.push(sources[fi]);
                cuts.push((a, b));
                found = true;
                break;
            }
        }
        if !found {
            return Err(Error::new(format!(
                "polysplit: no face has both points {a} and {b} (not neighbours)"
            )));
        }
    }
    let closed = g
        .topology
        .closed
        .as_ref()
        .map(|c| sources.iter().map(|&i| c[i]).collect());
    let mut out = edges::rebuild_edges(g, &faces, &sources, closed)?;
    let mask = pair_mask(&out, &cuts);
    outgroup(&mut out, p, "edges", "edge", &mask)?;
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn quad() -> Geometry {
        Geometry::from_arrays(
            vec![0., 0., 0., 2., 0., 0., 2., 2., 0., 0., 2., 0.],
            vec![0, 1, 2, 3],
            vec![0, 4],
            None,
        )
        .unwrap()
    }
    #[test]
    fn loop_cut_exact_points_and_winding() {
        let p = Params::resolve(
            POLYSPLIT.params,
            json!({"split_edges":"p0-1","outgrp_points":true,"outgrp_edges":true}).as_object(),
        )
        .unwrap();
        let o = polysplit(&[vec![Arc::new(quad())]], &p).unwrap();
        assert_eq!((o.npoints, o.nprims()), (6, 2));
        assert_eq!(&o.p()[12..], &[1., 0., 0., 1., 2., 0.]);
        assert!(
            normals::prim_normals(o.p(), &o.topology)
                .iter()
                .all(|n| n[2] > 0.)
        );
        assert_eq!(
            o.point["group_splitpoints"].data,
            AttrData::I32(vec![0, 0, 0, 0, 1, 1])
        );
    }
    fn cube() -> Geometry {
        let p = vec![
            0., 0., 0., 1., 0., 0., 1., 1., 0., 0., 1., 0., 0., 0., 1., 1., 0., 1., 1., 1., 1., 0., 1., 1.,
        ];
        let f = vec![0, 3, 2, 1, 4, 5, 6, 7, 0, 1, 5, 4, 1, 2, 6, 5, 2, 3, 7, 6, 3, 0, 4, 7];
        Geometry::from_arrays(p, f, vec![0, 4, 8, 12, 16, 20, 24], None).unwrap()
    }
    fn cut(g: Geometry, params: serde_json::Value) -> Geometry {
        let p = Params::resolve(POLYSPLIT.params, params.as_object()).unwrap();
        polysplit(&[vec![Arc::new(g)]], &p).unwrap()
    }
    #[test]
    fn loop_cut_box_ring_is_watertight_for_each_count() {
        for loops in 1..4 {
            let o = cut(cube(), json!({"split_edges":"p0-1","loops":loops}));
            assert_eq!((o.nprims(), o.npoints), (6 + 4 * loops, 8 + 4 * loops));
            assert!((0..o.nprims()).all(|i| range(&o, i).len() == 4));
            let lt = LoopTopology::new(&o);
            assert!(lt.owners.values().all(|f| f.len() == 2), "closed manifold");
        }
    }
    #[test]
    fn loop_cut_position_counts_from_lower_point_and_ignores_pair_direction() {
        let a = cut(cube(), json!({"split_edges":"p0-1","position":0.25}));
        let b = cut(cube(), json!({"split_edges":"p1-0","position":0.25}));
        assert_eq!(a.p(), b.p(), "python sorts p-pairs; direction does not matter");
        assert_eq!(&a.p()[24..27], &[0.25, 0., 0.]);
        // multiple loops ignore position and space evenly
        let c = cut(cube(), json!({"split_edges":"p0-1","loops":3,"position":0.9}));
        assert_eq!(&c.p()[24..33], &[0.25, 0., 0., 0.5, 0., 0., 0.75, 0., 0.]);
    }
    #[test]
    fn loop_preview_lines_are_the_cooked_edges() {
        for (loops, position) in [(1, 0.3), (2, 0.5), (5, 0.5)] {
            let g = cube();
            let ring = LoopTopology::new(&g).ring([(0, 1)]);
            let lines = ring.cut_lines(g.p(), &loop_cut_fractions(loops, position));
            assert_eq!(lines.len(), 4 * loops);
            let o = cut(g, json!({"split_edges":"p0-1","loops":loops,"position":position,"outgrp_edges":true}));
            let pts: Vec<[f32; 3]> = o.p().chunks_exact(3).map(|q| [q[0], q[1], q[2]]).collect();
            let near = |a: [f32; 3], b: [f32; 3]| (0..3).all(|k| (a[k] - b[k]).abs() < 1e-5);
            let id = |x: [f32; 3]| pts.iter().position(|&q| near(q, x)).expect("preview end is a cooked point");
            let lt = LoopTopology::new(&o);
            for (a, b) in lines {
                assert!(lt.owners.contains_key(&key(id(a), id(b))), "preview segment is a cooked edge");
            }
        }
    }
    #[test]
    fn loop_ring_stops_at_triangles_boundaries_and_nonmanifold_edges() {
        // two quads in a row then a triangle: 0-1-2-3, 1-4-5-2, 4-6-5
        let p = vec![0., 0., 0., 1., 0., 0., 1., 1., 0., 0., 1., 0., 2., 0., 0., 2., 1., 0., 3., 0.5, 0.];
        let g = Geometry::from_arrays(p, vec![0, 1, 2, 3, 1, 4, 5, 2, 4, 6, 5], vec![0, 4, 8, 11], None).unwrap();
        let lt = LoopTopology::new(&g);
        let ring = lt.ring([(0, 3)]);
        assert_eq!(ring.quads.len(), 2);
        assert!(ring.dirs.contains_key(&(4, 5)), "the triangle's edge is cut too (no cracks)");
        assert_eq!(lt.ring([(0, 1)]).quads.len(), 1, "boundary stops the strip");
        assert!(lt.ring([(4, 6)]).quads.is_empty(), "a triangle-only edge has no strip");
        assert!(lt.ring([(7, 9)]).dirs.is_empty(), "unknown edges are ignored");
        let o = cut(g, json!({"split_edges":"p0-3"}));
        assert_eq!((o.npoints, o.nprims()), (10, 5));
        assert_eq!(range(&o, 4).len(), 4, "triangle gains the new point");
    }
    #[test]
    fn loop_fractions_match_python() {
        assert_eq!(loop_cut_fractions(1, 0.3), vec![0.3]);
        assert_eq!(loop_cut_fractions(3, 0.3), vec![0.25, 0.5, 0.75]);
    }
    fn slide(g: Geometry, params: serde_json::Value) -> Geometry {
        let p=Params::resolve(EDGE_SLIDE.params,params.as_object()).unwrap();
        edge_slide(&[vec![Arc::new(g)]],&p).unwrap()
    }
    #[test] fn edge_slide_boundary_flip_and_empty_group() {
        let g=quad();
        assert_eq!(slide(g.clone(),json!({})).p(),g.p());
        let positive=slide(g.clone(),json!({"group":"p0-1","factor":0.5}));
        assert_eq!(&positive.p()[..6],&[0.,1.,0.,2.,1.,0.]);
        assert_eq!(slide(g.clone(),json!({"group":"p0-1","factor":-0.5})).p(),g.p());
        assert_eq!(slide(g,json!({"group":"p0-1","factor":-0.5,"flipped":true})).p(),positive.p());
        assert_eq!(positive.topology.vertex_point,quad().topology.vertex_point);
    }
    #[test] fn edge_slide_on_cube_and_polysplit_are_same() {
        let centre=cut(cube(),json!({"split_edges":"p0-1","outgrp_edges":true}));
        let via_slide=slide(centre.clone(),json!({"group":"splitedges","factor":0.4}));
        let direct=cut(cube(),json!({"split_edges":"p0-1","factor":0.4}));
        assert_eq!(via_slide.p(),direct.p());
        assert_eq!(via_slide.topology.vertex_point,direct.topology.vertex_point);
        assert_eq!(slide(centre.clone(),json!({"group":"splitedges","factor":0.4})).p(),via_slide.p());
        let negative=slide(centre,json!({"group":"splitedges","factor":-0.4}));
        assert_ne!(negative.p(),via_slide.p());
    }
    #[test] fn edge_slide_interpolates_point_attributes_and_keeps_uv_seams() {
        let mut g=Geometry::from_arrays(vec![0.,0.,0., 1.,0.,0., 2.,0.,0., 0.,1.,0., 1.,1.,0., 2.,1.,0.],
            vec![0,1,4,3, 1,2,5,4],vec![0,4,8],None).unwrap();
        g.point.insert("weight".into(),Arc::new(Attr::f32(1,vec![0.,1.,2.,0.,1.,2.])));
        g.vertex.insert("uv".into(),Arc::new(Attr::f32(2,vec![0.,0., 1.,0., 1.,1., 0.,1.,
            10.,0., 11.,0., 11.,1., 10.,1.])));
        let o=slide(g.clone(),json!({"group":"p1-4","factor":0.5}));
        assert_eq!(o.topology.vertex_point,g.topology.vertex_point);
        assert_eq!(o.point["weight"].as_f32().unwrap()[1],0.5);
        let uv=o.vertex["uv"].as_f32().unwrap();
        assert_eq!(uv[2],0.5);
        assert_eq!(uv[8],10.);
        let off=slide(g,json!({"group":"p1-4","factor":0.5,"correct_uvs":false}));
        assert_eq!(off.vertex["uv"].as_f32().unwrap()[2],1.);
    }
    #[test] fn edge_slide_even_and_clamp_on_unequal_rails() {
        let g=Geometry::from_arrays(vec![0.,0.,0., 2.,0.,0., 2.,2.,0., 0.,4.,0.],vec![0,1,2,3],vec![0,4],None).unwrap();
        let plain=slide(g.clone(),json!({"group":"p0-1","factor":0.5}));
        let even=slide(g.clone(),json!({"group":"p0-1","factor":0.5,"even":true}));
        assert_eq!(plain.p()[4],1.);
        assert_eq!(even.p()[4],2.);
        let capped=slide(g.clone(),json!({"group":"p0-1","factor":1.,"even":true}));
        let free=slide(g,json!({"group":"p0-1","factor":1.,"even":true,"clamp":false}));
        assert_eq!(capped.p()[4],2.);
        assert_eq!(free.p()[4],4.);
    }
    #[test] fn multiple_loop_cuts_slide_together_and_preview_matches_cook() {
        let g=cube();
        for count in [1,3] { for factor in [-0.5,0.,0.5] {
            let o=cut(g.clone(),json!({"split_edges":"p0-1","loops":count,"factor":factor,"outgrp_edges":true}));
            let lines=loop_cut_preview(&g,(0,1),count,factor,false,false,true).unwrap();
            assert_eq!(lines.len(),4*count);
            let coords: BTreeSet<_>=o.p().chunks_exact(3).map(|q|format!("{:.4},{:.4},{:.4}",q[0],q[1],q[2])).collect();
            for (a,b) in lines { for p in [a,b] { assert!(coords.contains(&format!("{:.4},{:.4},{:.4}",p[0],p[1],p[2]))); } }
        } }
    }
    #[test]
    fn twist_and_singular_taper_normals() {
        let mut g = quad();
        g.point.insert(
            "N".into(),
            Arc::new(Attr::f32(
                3,
                vec![0., 0., 1., 0., 0., 1., 0., 0., 1., 0., 0., 1.],
            )),
        );
        let p = Params::resolve(TWIST.params, None).unwrap();
        let out = twist(&[vec![Arc::new(g.clone())]], &p).unwrap();
        assert!(out.p().iter().all(|x| x.is_finite()));
        let p = Params::resolve(LINEAR_TAPER.params, None).unwrap();
        let out = taper(&[vec![Arc::new(g)]], &p).unwrap();
        out.validate().unwrap();
        assert!(
            out.point["N"]
                .as_f32()
                .unwrap()
                .iter()
                .all(|x| x.is_finite())
        );
    }
    #[test]
    fn group_range_negative_offset_uses_python_modulo() {
        let p = Params::resolve(
            GROUP_RANGE.params,
            json!({"domain":"points","every":3,"offset":-1}).as_object(),
        )
        .unwrap();
        let out = group_range(&[vec![Arc::new(quad())]], &p).unwrap();
        assert_eq!(
            out.point["group_group1"].data,
            AttrData::I32(vec![0, 1, 0, 0])
        );
    }
}
