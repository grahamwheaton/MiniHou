//! Derived-edge selections and conservative polygon operations from edge_tools.py.
use super::{GEO, InputSpec, Inputs, NodeSpec, groups, model::*};
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
pub(super) fn values(g: &Geometry, name: &str) -> Result<Vec<f32>> {
    let e = topo::edges(&g.topology, g.npoints);
    let mut out = vec![0.0f32; e.len()];
    if let Some(a) = g.vertex.get(name) {
        if a.width != 1 {
            return Err(Error::new(format!(
                "{name} must be a finite scalar vertex attribute"
            )));
        }
        for v in 0..g.nvertices() {
            let x = match &a.data {
                AttrData::F32(a) => a[v],
                AttrData::I32(a) => a[v] as f32,
            };
            if !x.is_finite() || name == "creaseweight" && x < 0. {
                return Err(Error::new(format!("invalid {name}")));
            }
            let i = e.of_vertex[v];
            if i >= 0 {
                out[i as usize] = out[i as usize].max(x);
            }
        }
    }
    Ok(out)
}
pub(super) fn write_values(
    g: &mut Geometry,
    name: &str,
    data: &[f32],
    integer: bool,
) -> Result<()> {
    let e = topo::edges(&g.topology, g.npoints);
    if data.len() != e.len() {
        return Err(Error::new(
            "edge attribute length does not match derived edges",
        ));
    }
    let v: Vec<_> = e
        .of_vertex
        .iter()
        .map(|&i| if i < 0 { 0. } else { data[i as usize] })
        .collect();
    g.vertex.insert(
        name.into(),
        Arc::new(if integer {
            Attr::i32(1, v.iter().map(|&x| x as i32).collect())
        } else {
            Attr::f32(1, v)
        }),
    );
    Ok(())
}
pub(super) fn wildcard(p: &[u8], n: &[u8]) -> bool {
    match p {
        [] => n.is_empty(),
        [b'*', rest @ ..] => (0..=n.len()).any(|i| wildcard(rest, &n[i..])),
        [b'?', rest @ ..] => !n.is_empty() && wildcard(rest, &n[1..]),
        [b'[', rest @ ..] => {
            let skip = usize::from(rest.first() == Some(&b'!'));
            let first = skip + usize::from(rest.get(skip) == Some(&b']'));
            if let Some(end) = rest[first..]
                .iter()
                .position(|&c| c == b']')
                .map(|i| i + first)
            {
                let Some(&c) = n.first() else {
                    return false;
                };
                let mut hit = false;
                let mut i = skip;
                while i < end {
                    if i + 2 < end && rest[i + 1] == b'-' {
                        hit |= rest[i] <= c && c <= rest[i + 2];
                        i += 3;
                    } else {
                        hit |= rest[i] == c;
                        i += 1;
                    }
                }
                (hit != (skip == 1)) && wildcard(&rest[end + 1..], &n[1..])
            } else {
                n.first() == Some(&b'[') && wildcard(rest, &n[1..])
            }
        }
        [c, rest @ ..] => n.first() == Some(c) && wildcard(rest, &n[1..]),
    }
}
pub(crate) fn edge_mask(g: &Geometry, pattern: &str) -> Result<Vec<bool>> {
    let e = topo::edges(&g.topology, g.npoints);
    let tokens: Vec<_> = pattern
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|s| !s.is_empty())
        .collect();
    if tokens.is_empty() {
        return Ok(vec![true; e.len()]);
    }
    let mut out = vec![tokens[0].starts_with('^'); e.len()];
    for token in tokens {
        let exclude = token.starts_with('^');
        let token = token.strip_prefix('^').unwrap_or(token);
        let mut hit = vec![false; e.len()];
        if let Some(pair) = token
            .strip_prefix('p')
            .and_then(|s| s.split_once('-'))
            .filter(|(a, b)| {
                a.chars().all(|c| c.is_ascii_digit()) && b.chars().all(|c| c.is_ascii_digit())
            })
        {
            let a: i64 = pair
                .0
                .parse()
                .map_err(|_| Error::new("invalid edge pair"))?;
            let b: i64 = pair
                .1
                .parse()
                .map_err(|_| Error::new("invalid edge pair"))?;
            for i in 0..e.len() {
                hit[i] = e.points[2 * i] == a.min(b) && e.points[2 * i + 1] == a.max(b);
            }
            if !hit.iter().any(|&x| x) {
                return Err(Error::new(format!("edge {token:?} does not exist")));
            }
        } else if token.as_bytes().first().is_some_and(u8::is_ascii_digit) {
            let (span, step) = token.split_once(':').unwrap_or((token, "1"));
            let step: usize = step
                .parse()
                .map_err(|_| Error::new("edge range stride must be a positive integer"))?;
            if step == 0 {
                return Err(Error::new("edge range stride must be a positive integer"));
            }
            let (a, b) = span.split_once('-').unwrap_or((span, span));
            let a: usize = a.parse().map_err(|_| Error::new("invalid edge range"))?;
            let b: usize = b.parse().map_err(|_| Error::new("invalid edge range"))?;
            if a < e.len() {
                for i in (a..=b.min(e.len() - 1)).step_by(step) {
                    hit[i] = true;
                }
            }
        } else if token == "*" {
            hit.fill(true);
        } else {
            let mut found = false;
            for key in g.vertex.keys().filter(|k| k.starts_with("edge_group_")) {
                if wildcard(token.as_bytes(), key[11..].as_bytes()) {
                    found = true;
                    for (b, x) in hit.iter_mut().zip(values(g, key)?) {
                        *b |= x != 0.;
                    }
                }
            }
            if !found && !token.contains(['*', '?', '[']) {
                return Err(Error::new(format!("no edge group {token:?}")));
            }
        }
        for (o, b) in out.iter_mut().zip(hit) {
            if exclude {
                *o &= !b;
            } else {
                *o |= b;
            }
        }
    }
    Ok(out)
}
pub(super) fn remap(
    before: &Geometry,
    after: &mut Geometry,
    sources: Option<&[usize]>,
) -> Result<()> {
    let a = topo::edges(&before.topology, before.npoints);
    let b = topo::edges(&after.topology, after.npoints);
    let lookup: BTreeMap<_, _> = a
        .points
        .chunks_exact(2)
        .enumerate()
        .map(|(i, p)| ((p[0], p[1]), i))
        .collect();
    for (name, attr) in &before.vertex {
        if !name.starts_with("edge_group_") && name != "creaseweight" {
            continue;
        }
        let old = values(before, name)?;
        let data = b
            .points
            .chunks_exact(2)
            .map(|p| {
                let (mut a, mut b) = (p[0], p[1]);
                if let Some(src) = sources {
                    a = src[a as usize] as i64;
                    b = src[b as usize] as i64;
                }
                lookup.get(&(a.min(b), a.max(b))).map_or(0., |&i| old[i])
            })
            .collect::<Vec<_>>();
        write_values(after, name, &data, matches!(attr.data, AttrData::I32(_)))?;
    }
    Ok(())
}
pub(crate) fn rebuild_edges(
    g: &Geometry,
    faces: &[Vec<usize>],
    sources: &[usize],
    closed: Option<Vec<bool>>,
) -> Result<Geometry> {
    let mut out = rebuild(g, faces, sources, closed)?;
    remap(g, &mut out, None)?;
    Ok(out)
}
pub const CREASE: NodeSpec = NodeSpec {
    name: "crease",
    inputs: GEO,
    params: crate::params!("domain":"edges";"group":"";"sharpness":2.),
    cook: crease,
};
pub const EDGE_GROUP: NodeSpec = NodeSpec {
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
pub const EDGE_GROUP_PROMOTE: NodeSpec = NodeSpec {
    name: "edge_group_promote",
    inputs: GEO,
    params: crate::params!("pattern":"*";"name":"edge_points";"to":"points"),
    cook: promote,
};
pub const DISSOLVE: NodeSpec = NodeSpec {
    name: "dissolve_edges",
    inputs: GEO,
    params: crate::params!("group":"";"angle":0.1;"preserve_seams":true),
    cook: dissolve,
};
pub const POLYBRIDGE: NodeSpec = NodeSpec {
    name: "polybridge",
    inputs: GEO,
    params: crate::params!("source":"";"destination":"";"segments":1;"twist":0;"reverse":false;"outgrp_bridge":false;"outgrp_bridge_name":"bridgepolys"),
    cook: polybridge,
};
fn crease(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = CREASE.one(inputs, 0)?;
    let mut out = (*g).clone();
    let text = p.s("group")?;
    let sharp = p.f("sharpness")? as f32;
    match p.e("domain", &["edges", "points"])?.as_str() {
        "edges" => {
            let mut a = values(&g, "creaseweight")?;
            for (x, m) in a.iter_mut().zip(edge_mask(&g, &text)?) {
                if m {
                    *x = sharp;
                }
            }
            write_values(&mut out, "creaseweight", &a, false)?;
        }
        _ => {
            let mut a = vec![0.; g.npoints];
            if let Some(old) = g.point.get("cornerweight") {
                if old.width != 1 {
                    return Err(Error::new("cornerweight must be a scalar point attribute"));
                }
                a = match &old.data {
                    AttrData::F32(v) => v.clone(),
                    AttrData::I32(v) => v.iter().map(|&x| x as f32).collect(),
                };
            }
            for (x, m) in a.iter_mut().zip(groups::point_mask(&g, &text)?) {
                if m {
                    *x = sharp;
                }
            }
            out.point
                .insert("cornerweight".into(), Arc::new(Attr::f32(1, a)));
        }
    }
    Ok(out)
}
fn edge_group(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = EDGE_GROUP.one(inputs, 0)?;
    let name = p.s("name")?;
    group_name(&name)?;
    let pattern = p.s("pattern")?;
    let e = topo::edges(&g.topology, g.npoints);
    let method = p.e(
        "method",
        &[
            "pattern",
            "boundary",
            "points",
            "primitives",
            "bounding_object",
        ],
    )?;
    let mut mask = match method.as_str() {
        "pattern" => edge_mask(&g, &pattern)?,
        "boundary" => e.count.iter().map(|&x| x == 1).collect(),
        "points" => {
            let sel = groups::point_mask(&g, &pattern)?;
            e.points
                .chunks_exact(2)
                .map(|p| sel[p[0] as usize] && sel[p[1] as usize])
                .collect()
        }
        "primitives" => {
            let sel = prim_mask(&g, &pattern)?;
            let (pv, _, _) = corners(&g);
            let mut m = vec![false; e.len()];
            for v in 0..g.nvertices() {
                if e.of_vertex[v] >= 0 && sel[pv[v]] {
                    m[e.of_vertex[v] as usize] = true;
                }
            }
            m
        }
        _ => {
            let b = inputs
                .get(1)
                .and_then(|v| v.first())
                .filter(|b| b.npoints > 0)
                .ok_or_else(|| {
                    Error::new("edge_group: connect the bounding object to the second input")
                })?;
            let mut lo = [f64::INFINITY; 3];
            let mut hi = [f64::NEG_INFINITY; 3];
            for q in positions(b) {
                for k in 0..3 {
                    lo[k] = lo[k].min(q[k]);
                    hi[k] = hi[k].max(q[k]);
                }
            }
            let inside: Vec<_> = positions(&g)
                .iter()
                .map(|q| (0..3).all(|k| q[k] >= lo[k] - 1e-9 && q[k] <= hi[k] + 1e-9))
                .collect();
            e.points
                .chunks_exact(2)
                .map(|p| inside[p[0] as usize] && inside[p[1] as usize])
                .collect()
        }
    };
    let key = format!("edge_group_{name}");
    let old = values(&g, &key)?;
    let combine = p.e("combine", &["replace", "union", "intersect", "subtract"])?;
    for (m, x) in mask.iter_mut().zip(old) {
        *m = match combine.as_str() {
            "union" => *m || x != 0.,
            "intersect" => *m && x != 0.,
            "subtract" => x != 0. && !*m,
            _ => *m,
        };
    }
    let mut out = (*g).clone();
    write_values(
        &mut out,
        &key,
        &mask.iter().map(|&b| b as u8 as f32).collect::<Vec<_>>(),
        true,
    )?;
    Ok(out)
}
fn promote(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = EDGE_GROUP_PROMOTE.one(inputs, 0)?;
    let m = edge_mask(&g, &p.s("pattern")?)?;
    let e = topo::edges(&g.topology, g.npoints);
    let points = p.e("to", &["points", "primitives"])? == "points";
    let mut sel = vec![false; if points { g.npoints } else { g.nprims() }];
    if points {
        for i in 0..e.len() {
            if m[i] {
                sel[e.points[2 * i] as usize] = true;
                sel[e.points[2 * i + 1] as usize] = true;
            }
        }
    } else {
        let (pv, _, _) = corners(&g);
        for v in 0..g.nvertices() {
            if e.of_vertex[v] >= 0 && m[e.of_vertex[v] as usize] {
                sel[pv[v]] = true;
            }
        }
    }
    let mut out = (*g).clone();
    write_group(
        &mut out,
        if points { "point" } else { "prim" },
        &p.s("name")?,
        &sel,
    )?;
    Ok(out)
}
pub(super) fn root(parent: &mut [usize], mut i: usize) -> usize {
    while parent[i] != i {
        parent[i] = parent[parent[i]];
        i = parent[i];
    }
    i
}
fn dissolve(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = DISSOLVE.one(inputs, 0)?;
    let pattern = p.s("group")?;
    if pattern.trim().is_empty() {
        return Ok((*g).clone());
    }
    polygons(&g, "dissolve")?;
    let e = topo::edges(&g.topology, g.npoints);
    let chosen = edge_mask(&g, &pattern)?;
    let (pv, nx, _) = corners(&g);
    let vp = &g.topology.vertex_point;
    let fn_ = normals::prim_normals(g.p(), &g.topology);
    let mut parent: Vec<_> = (0..g.nprims()).collect();
    for eid in 0..e.len() {
        if !chosen[eid] {
            continue;
        }
        if e.count[eid] != 2 {
            return Err(Error::new("dissolve requires manifold interior edges"));
        }
        let vv: Vec<_> = (0..g.nvertices())
            .filter(|&v| e.of_vertex[v] == eid as i64)
            .collect();
        let (u, v) = (vv[0], vv[1]);
        let (a, b) = (pv[u], pv[v]);
        if a == b || vp[u] != vp[nx[v]] || vp[v] != vp[nx[u]] {
            return Err(Error::new(
                "dissolve requires distinct consistently wound faces",
            ));
        }
        if len(fn_[a]).min(len(fn_[b])) < 1e-12
            || dot(unit(fn_[a]), unit(fn_[b])) < p.f("angle")?.to_radians().cos() - 1e-9
        {
            return Err(Error::new(
                "dissolve face angle exceeds limit or face is degenerate",
            ));
        }
        if p.b("preserve_seams")? {
            for (name, attr) in &g.prim {
                if !name.starts_with("group_") && !equal_rows(attr, a, b, 0.) {
                    return Err(Error::new(format!(
                        "dissolve crosses primitive attribute seam: {name}"
                    )));
                }
            }
            for (name, attr) in &g.vertex {
                if !name.starts_with("edge_group_")
                    && name != "creaseweight"
                    && (!equal_rows(attr, u, nx[v], 1e-6) || !equal_rows(attr, v, nx[u], 1e-6))
                {
                    return Err(Error::new(format!(
                        "dissolve crosses corner attribute seam: {name}"
                    )));
                }
            }
        }
        let ra = root(&mut parent, a);
        let rb = root(&mut parent, b);
        parent[rb] = ra;
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
            .filter(|&v| r.contains(&pv[v]) && !chosen[e.of_vertex[v] as usize])
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
    let mut out = rebuild_edges(&g, &faces, &sources, None)?;
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
            out.prim.insert(name.clone(), Arc::new(attr));
        }
    }
    Ok(out)
}
fn equal_rows(a: &Attr, i: usize, j: usize, tol: f64) -> bool {
    let w = a.width;
    (0..w).all(|k| match &a.data {
        AttrData::F32(v) => (v[i * w + k] as f64 - v[j * w + k] as f64).abs() <= tol,
        AttrData::I32(v) => v[i * w + k] == v[j * w + k],
    })
}
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
