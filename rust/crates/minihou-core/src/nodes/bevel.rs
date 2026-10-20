//! Polygon edge bevels from bevel.py (wedge splitting, terminating caps and segmented profiles).
use super::{
    InputSpec, Inputs, NodeSpec, edge_tools as edges, groups,
    model::*,
    modeling_tools::{pair_mask, parse_pairs, twins},
};
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
pub const POLYBEVEL: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "polybevel",
    inputs: &[
        InputSpec {
            name: "geo",
            multi: false,
            optional: false,
        },
        InputSpec {
            name: "profile",
            multi: false,
            optional: true,
        },
    ],
    params: crate::params!("distance":0.1;"divisions":1;"shape":"round";"convexity":1.;"limit_overlap":true;"exclude_edges":"";"min_angle":30.;"group":"";"group_type":"points";"edges":"";"output_group":"bevel";"outgrp_pointfilletpolys":false;"outgrp_pointfilletpolys_name":"pointfilletpolys";"outgrp_pointfilletedges":false;"outgrp_pointfilletedges_name":"pointfilletedges";"outgrp_edgefilletpolys":false;"outgrp_edgefilletpolys_name":"edgefilletpolys";"outgrp_offsetedges":false;"outgrp_offsetedges_name":"offsetedges";"outgrp_offsetpoints":false;"outgrp_offsetpoints_name":"offsetpoints";"outgrp_mergedpoints":false;"outgrp_mergedpoints_name":"mergedpoints"),
    cook: bevel,
};
fn u(v: [f64; 3]) -> [f64; 3] {
    mul(v, 1. / len(v).max(1e-12))
}
fn empty(g: &Geometry, p: &Params) -> Result<Geometry> {
    let mut out = g.clone();
    for (key, dom, n) in [
        ("pointfilletpolys", "prim", g.nprims()),
        ("edgefilletpolys", "prim", g.nprims()),
        ("offsetpoints", "point", g.npoints),
        ("mergedpoints", "point", g.npoints),
        (
            "pointfilletedges",
            "edge",
            topo::edges(&g.topology, g.npoints).len(),
        ),
        (
            "offsetedges",
            "edge",
            topo::edges(&g.topology, g.npoints).len(),
        ),
    ] {
        outgroup(&mut out, p, key, dom, &vec![false; n])?;
    }
    Ok(out)
}
fn bevel(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = POLYBEVEL.one(inputs, 0)?;
    polygons(&g, "polybevel")?;
    let distance = p.f("distance")?;
    if g.nprims() == 0 || distance <= 0. {
        return empty(&g, p);
    }
    let count = p.i("divisions")?;
    if !(1..=64).contains(&count) {
        return Err(Error::new("divisions must be 1..64"));
    }
    let (pv, nx, pr) = corners(&g);
    let vp = &g.topology.vertex_point;
    let tw = twins(&g);
    let pos = positions(&g);
    let n: Vec<_> = normals::prim_normals(g.p(), &g.topology)
        .iter()
        .map(|&v| u(v))
        .collect();
    let angle = p.f("min_angle")?;
    let mut bev: Vec<_> = (0..g.nvertices())
        .map(|i| {
            tw[i].is_some_and(|j| {
                dot(n[pv[i]], n[pv[j]])
                    <= angle.to_radians().cos() + if angle > 0. { 1e-6 } else { 2. }
            })
        })
        .collect();
    let picked = parse_pairs(&p.s("edges")?, g.npoints)?;
    if !picked.is_empty() {
        let wanted = pair_mask(&g, &picked);
        let e = topo::edges(&g.topology, g.npoints);
        for i in 0..g.nvertices() {
            bev[i] = tw[i].is_some() && wanted[e.of_vertex[i] as usize];
        }
    }
    let group = p.s("group")?;
    let ty = p.e("group_type", &["points", "edges", "primitives"])?;
    if ty == "points" {
        let sel = groups::point_mask(&g, &group)?;
        for i in 0..g.nvertices() {
            bev[i] &= sel[vp[i] as usize] && sel[vp[nx[i]] as usize];
        }
    } else if !group.trim().is_empty() && picked.is_empty() {
        if ty == "edges" {
            let m = edges::edge_mask(&g, &group)?;
            let e = topo::edges(&g.topology, g.npoints);
            for i in 0..g.nvertices() {
                bev[i] = tw[i].is_some() && m[e.of_vertex[i] as usize];
            }
        } else {
            let sel = prim_mask(&g, &group)?;
            for i in 0..g.nvertices() {
                bev[i] = tw[i].is_some_and(|j| sel[pv[i]] || sel[pv[j]]);
            }
        }
    }
    let exclude = p.s("exclude_edges")?;
    if !exclude.trim().is_empty() {
        let m = edges::edge_mask(&g, &exclude)?;
        let e = topo::edges(&g.topology, g.npoints);
        for i in 0..g.nvertices() {
            bev[i] &= !m[e.of_vertex[i] as usize];
        }
    }
    if !bev.iter().any(|&b| b) {
        return empty(&g, p);
    }
    let nv = g.nvertices();
    let mut degree = vec![0; g.npoints];
    for i in 0..nv {
        if bev[i] {
            degree[vp[i] as usize] += 1;
        }
    }
    let dangling: Vec<_> = degree.iter().map(|&n| n == 1).collect();
    let side: Vec<_> = (0..nv).map(|i| bev[i] || bev[pr[i]]).collect();
    let other_in: Vec<_> = (0..nv).map(|i| tw[pr[i]].unwrap_or(0)).collect();
    let other_out: Vec<_> = (0..nv).map(|i| nx[tw[i].unwrap_or(0)]).collect();
    let bout: Vec<_> = (0..nv)
        .map(|i| {
            bev[i]
                || tw[i].is_none()
                || (dangling[vp[i] as usize] && tw[i].is_some() && (side[i] || side[other_out[i]]))
        })
        .collect();
    let bin: Vec<_> = (0..nv)
        .map(|i| {
            bev[pr[i]]
                || tw[pr[i]].is_none()
                || (dangling[vp[i] as usize]
                    && tw[pr[i]].is_some()
                    && (side[i] || side[other_in[i]]))
        })
        .collect();
    let mut parent: Vec<_> = (0..nv).collect();
    for i in 0..nv {
        if !bin[i] {
            let a = edges::root(&mut parent, i);
            let b = edges::root(&mut parent, tw[pr[i]].unwrap());
            parent[a.max(b)] = a.min(b);
        }
    }
    let roots: Vec<_> = (0..nv).map(|i| edges::root(&mut parent, i)).collect();
    let unique: Vec<_> = roots
        .iter()
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let w: Vec<_> = roots
        .iter()
        .map(|r| unique.binary_search(r).unwrap())
        .collect();
    let nw = unique.len();
    let corner = unique;
    let home: Vec<_> = corner.iter().map(|&v| pos[vp[v] as usize]).collect();
    let mut outpos = home.clone();
    let mut bounds = vec![Vec::<([f64; 3], bool, usize)>::new(); nw];
    for i in 0..nv {
        if bin[i] {
            let h = pr[i];
            bounds[w[i]].push((
                sub(pos[vp[h] as usize], pos[vp[i] as usize]),
                bev[h],
                tw[h].map_or(h, |j| h.min(j)),
            ));
        }
    }
    for i in 0..nv {
        if bout[i] {
            bounds[w[i]].push((
                sub(pos[vp[nx[i]] as usize], pos[vp[i] as usize]),
                bev[i],
                tw[i].map_or(i, |j| i.min(j)),
            ));
        }
    }
    let ain: Vec<_> = (0..nv)
        .map(|i| u(sub(pos[vp[pr[i]] as usize], pos[vp[i] as usize])))
        .collect();
    let aout: Vec<_> = (0..nv)
        .map(|i| u(sub(pos[vp[nx[i]] as usize], pos[vp[i] as usize])))
        .collect();
    let mut idir = vec![[0.; 3]; nw];
    let mut ni = vec![0; nw];
    let mut span = vec![0.; nw];
    for i in 0..nv {
        if !bin[i] {
            idir[w[i]] = add(idir[w[i]], ain[i]);
            ni[w[i]] += 1;
        }
        span[w[i]] += dot(ain[i], aout[i]).clamp(-1., 1.).acos();
    }
    let slide = |c: [f64; 3], e: [f64; 3]| {
        mul(
            c,
            (distance / len(cross(c, e)).max(0.05)).min(4. * distance),
        )
    };
    for k in 0..nw {
        let b = &bounds[k];
        if b.len() != 2 || (!b[0].1 && !b[1].1) || b[0].2 == b[1].2 {
            continue;
        }
        let a = u(b[0].0);
        let bb = u(b[1].0);
        let off = if b[0].1 != b[1].1 {
            if b[0].1 { slide(bb, a) } else { slide(a, bb) }
        } else if ni[k] > 0 && len(idir[k]) > 1e-6 {
            slide(u(idir[k]), a)
        } else if len(add(a, bb)) > 0.2 {
            let sign = if span[k] <= std::f64::consts::PI + 1e-6 {
                1.
            } else {
                -1.
            };
            let half = ((1. - dot(a, bb)) / 2.).clamp(0., 1.).sqrt();
            mul(u(add(a, bb)), sign * distance / half.max(0.05))
        } else {
            mul(u(cross(n[pv[corner[k]]], aout[corner[k]])), distance)
        };
        outpos[k] = add(outpos[k], off);
    }
    if p.b("limit_overlap")? {
        for _ in 0..12 {
            let mut newn = vec![[0.; 3]; g.nprims()];
            for i in 0..nv {
                newn[pv[i]] = add(newn[pv[i]], cross(outpos[w[i]], outpos[w[nx[i]]]));
            }
            let folded: Vec<_> = (0..g.nprims())
                .map(|f| dot(newn[f], n[f]) <= 0.05 * len(newn[f]))
                .collect();
            let mut bad = vec![false; nw];
            for i in 0..nv {
                if folded[pv[i]] {
                    bad[w[i]] = true;
                }
            }
            let mut changed = false;
            for k in 0..nw {
                if bad[k] && len(sub(outpos[k], home[k])) > 0. {
                    outpos[k] = add(home[k], mul(sub(outpos[k], home[k]), 0.5));
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
    }
    let c1: Vec<_> = (0..nv)
        .filter(|&i| bev[i] && tw[i].is_some_and(|j| i < j))
        .collect();
    let qsrc: Vec<_> = c1
        .iter()
        .map(|&i| {
            let j = tw[i].unwrap();
            [nx[i], i, nx[j], j]
        })
        .collect();
    let mut succ = vec![None; nw];
    for &i in &c1 {
        let j = tw[i].unwrap();
        succ[w[nx[j]]] = Some(w[i]);
        succ[w[nx[i]]] = Some(w[j]);
    }
    let mut rings = Vec::new();
    let mut seen = BTreeSet::new();
    for start in 0..nw {
        if succ[start].is_none() || seen.contains(&start) {
            continue;
        }
        let mut at = start;
        let mut ring = Vec::new();
        while !ring.contains(&at) {
            ring.push(at);
            let Some(q) = succ[at] else {
                break;
            };
            at = q;
        }
        for &q in &ring {
            seen.insert(q);
        }
        if at == start && succ[*ring.last().unwrap()] == Some(start) && ring.len() >= 3 {
            rings.push(ring);
        }
    }
    // Python orders cycles by their smallest wedge; incomplete chains must not hide a later cycle.
    rings.sort_by_key(|r| r[0]);
    for q in 0..g.npoints {
        if !dangling[q] {
            continue;
        }
        let caps: Vec<_> = (0..nv)
            .filter(|&v| vp[v] as usize == q && !side[v])
            .collect();
        if caps.len() > 1 {
            if let Some(a) = (0..nw).find(|&k| succ[k].is_some() && vp[corner[k]] as usize == q) {
                let b = succ[a].unwrap();
                let c = w[caps[0]];
                if a != b && b != c && a != c {
                    rings.push(vec![a, b, c]);
                }
            }
        }
    }
    let mut outvp = Vec::new();
    let mut vs = Vec::new();
    let mut sizes = Vec::new();
    let mut ps: Vec<_> = (0..g.nprims()).collect();
    for f in 0..g.nprims() {
        let start = outvp.len();
        for v in range(&g, f) {
            if dangling[vp[v] as usize] && !side[v] {
                let a = if tw[pr[v]].is_some() && side[other_in[v]] {
                    w[other_in[v]]
                } else {
                    w[v]
                };
                let b = if tw[v].is_some() && side[other_out[v]] {
                    w[other_out[v]]
                } else {
                    w[v]
                };
                outvp.push(a as i32);
                vs.push(v);
                if a != b {
                    outvp.push(b as i32);
                    vs.push(v);
                }
            } else {
                outvp.push(w[v] as i32);
                vs.push(v);
            }
        }
        sizes.push(outvp.len() - start);
    }
    for (q, &i) in qsrc.iter().zip(&c1) {
        outvp.extend(q.map(|v| w[v] as i32));
        vs.extend(q);
        sizes.push(4);
        ps.push(pv[i]);
    }
    for ring in &rings {
        outvp.extend(ring.iter().map(|&k| k as i32));
        vs.extend(ring.iter().map(|&k| corner[k]));
        sizes.push(ring.len());
        ps.push(pv[corner[ring[0]]]);
    }
    let pointsrc: Vec<_> = corner.iter().map(|&v| vp[v] as usize).collect();
    let mut point = gather(&g.point, &pointsrc);
    point.insert(
        "P".into(),
        Arc::new(Attr::f32(
            3,
            outpos.iter().flat_map(|q| q.map(|x| x as f32)).collect(),
        )),
    );
    let mut out = make(
        point,
        outvp,
        &sizes,
        gather(&g.vertex, &vs),
        gather(&g.prim, &ps),
        g.detail.clone(),
        None,
    )?;
    out.point.remove("N");
    out.vertex.remove("N");
    out.prim.remove("N");
    let group = p.s("output_group")?;
    if !group.is_empty() {
        write_group(
            &mut out,
            "prim",
            &group,
            &(0..sizes.len())
                .map(|i| i >= g.nprims())
                .collect::<Vec<_>>(),
        )?;
    }
    outgroup(
        &mut out,
        p,
        "edgefilletpolys",
        "prim",
        &(0..sizes.len())
            .map(|i| i >= g.nprims() && i < g.nprims() + c1.len())
            .collect::<Vec<_>>(),
    )?;
    outgroup(
        &mut out,
        p,
        "pointfilletpolys",
        "prim",
        &(0..sizes.len())
            .map(|i| i >= g.nprims() + c1.len())
            .collect::<Vec<_>>(),
    )?;
    let offset: BTreeSet<_> = qsrc.iter().flat_map(|q| q.iter().map(|&v| w[v])).collect();
    outgroup(
        &mut out,
        p,
        "offsetpoints",
        "point",
        &(0..nw).map(|i| offset.contains(&i)).collect::<Vec<_>>(),
    )?;
    outgroup(&mut out, p, "mergedpoints", "point", &vec![false; nw])?;
    let pairs: Vec<_> = qsrc
        .iter()
        .flat_map(|q| [(w[q[0]], w[q[1]]), (w[q[2]], w[q[3]])])
        .collect();
    let mask = pair_mask(&out, &pairs);
    outgroup(&mut out, p, "offsetedges", "edge", &mask)?;
    let pairs: Vec<_> = rings
        .iter()
        .flat_map(|r| (0..r.len()).map(|i| (r[i], r[(i + 1) % r.len()])))
        .collect();
    let mask = pair_mask(&out, &pairs);
    outgroup(&mut out, p, "pointfilletedges", "edge", &mask)?;
    if count > 1 {
        let profile = inputs
            .get(1)
            .and_then(|v| v.first())
            .map(|g| profile_samples(g, count as usize))
            .transpose()?;
        out = segments(
            &out,
            g.nprims(),
            c1.len(),
            count as usize,
            &p.e("shape", &["round", "flat"])?,
            p.f("convexity")?,
            &home,
            p,
            profile.as_deref(),
        )?;
    }
    out = drop_degenerate(&out)?;
    if dangling.iter().any(|&b| b) {
        out = compact(out)?;
    }
    Ok(out)
}
fn drop_degenerate(g: &Geometry) -> Result<Geometry> {
    let mut faces = Vec::new();
    let mut src = Vec::new();
    for f in 0..g.nprims() {
        let r = range(g, f);
        let face: Vec<_> = r
            .clone()
            .filter(|&v| {
                let next = if v + 1 < r.end { v + 1 } else { r.start };
                g.topology.vertex_point[v] != g.topology.vertex_point[next] || r.len() == 1
            })
            .collect();
        if face.len() >= 3 {
            faces.push(face);
            src.push(f);
        }
    }
    rebuild(g, &faces, &src, None)
}
fn profile_samples(g: &Geometry, count: usize) -> Result<Vec<[f64; 2]>> {
    let f = (0..g.nprims())
        .find(|&i| range(g, i).len() >= 2)
        .ok_or_else(|| {
            Error::new("polybevel: cross-section needs a curve with at least two points")
        })?;
    let r = range(g, f);
    let pos = positions(g);
    let pts: Vec<_> = r
        .map(|v| pos[g.topology.vertex_point[v] as usize])
        .collect();
    let a = pts[0];
    let chord = sub(*pts.last().unwrap(), a);
    let l = len(chord);
    if l < 1e-12 {
        return Err(Error::new(
            "polybevel: the cross-section curve starts and ends at the same point",
        ));
    }
    let x = mul(chord, 1. / l);
    let q: Vec<_> = pts.iter().map(|&p| sub(p, a)).collect();
    let perp: Vec<_> = q.iter().map(|&p| sub(p, mul(x, dot(p, x)))).collect();
    let k = (0..perp.len())
        .max_by(|&i, &j| len(perp[i]).total_cmp(&len(perp[j])).then(j.cmp(&i)))
        .unwrap();
    let y = if len(perp[k]) > 1e-12 {
        mul(perp[k], 1. / len(perp[k]).max(1e-30))
    } else {
        [0.; 3]
    };
    let xy: Vec<_> = q.iter().map(|&p| [dot(p, x) / l, dot(p, y) / l]).collect();
    let mut arc = vec![0.];
    for i in 1..xy.len() {
        arc.push(
            arc[i - 1]
                + ((xy[i][0] - xy[i - 1][0]).powi(2) + (xy[i][1] - xy[i - 1][1]).powi(2)).sqrt(),
        );
    }
    let mut out = Vec::new();
    for j in 0..=count {
        let want = *arc.last().unwrap() * j as f64 / count as f64;
        let i = arc
            .partition_point(|&v| v <= want)
            .saturating_sub(1)
            .min(xy.len() - 2);
        let t = if arc[i + 1] > arc[i] {
            (want - arc[i]) / (arc[i + 1] - arc[i])
        } else {
            1.
        };
        out.push([
            xy[i][0] * (1. - t) + xy[i + 1][0] * t,
            xy[i][1] * (1. - t) + xy[i + 1][1] * t,
        ]);
    }
    Ok(out)
}
struct Refiner<'a> {
    pos: Vec<[f64; 3]>,
    ancestry: Vec<(usize, usize, f64)>,
    chains: BTreeMap<(usize, usize), Vec<usize>>,
    homes: &'a [[f64; 3]],
    shared: BTreeMap<(usize, usize), usize>,
    count: usize,
    shape: &'a str,
    convexity: f64,
    profile: Option<&'a [[f64; 2]]>,
}
fn pair(a: usize, b: usize) -> (usize, usize) {
    (a.min(b), a.max(b))
}
impl Refiner<'_> {
    fn add_point(&mut self, p: [f64; 3], a: usize, b: usize, t: f64) -> usize {
        let i = self.pos.len();
        self.pos.push(p);
        self.ancestry.push((a, b, t));
        i
    }
    fn chain(&mut self, a: usize, b: usize, axis: [f64; 3]) -> Vec<usize> {
        let key = pair(a, b);
        if let Some(c) = self.chains.get(&key) {
            let mut c = c.clone();
            if c[0] != a {
                c.reverse();
            }
            return c;
        }
        let left = self.pos[a];
        let right = self.pos[b];
        let mid = mul(add(left, right), 0.5);
        let dir = u(axis);
        let control = if self.shared[&key] > 1 {
            self.homes[a]
        } else {
            sub(self.homes[a], mul(dir, dot(sub(self.homes[a], mid), dir)))
        };
        let x = u(sub(left, control));
        let y = u(sub(right, control));
        let weight = ((1. - dot(x, y).clamp(-1., 1.)) / 2.).max(0.).sqrt();
        let mut out = vec![a];
        for j in 1..self.count {
            let t = j as f64 / self.count as f64;
            let linear = add(mul(left, 1. - t), mul(right, t));
            let den = ((1. - t).powi(2) + 2. * weight * t * (1. - t) + t * t).max(1e-12);
            let arc = mul(
                add(
                    add(
                        mul(left, (1. - t).powi(2)),
                        mul(control, 2. * weight * t * (1. - t)),
                    ),
                    mul(right, t * t),
                ),
                1. / den,
            );
            let q = if let Some(profile) = self.profile {
                let bulge = mul(u(sub(control, mid)), len(sub(right, left)));
                add(
                    left,
                    add(
                        mul(sub(right, left), profile[j][0]),
                        mul(bulge, profile[j][1]),
                    ),
                )
            } else if self.shape == "flat" {
                linear
            } else {
                add(linear, mul(sub(arc, linear), self.convexity))
            };
            out.push(self.add_point(q, a, b, t));
        }
        out.push(b);
        self.chains.insert(key, out.clone());
        out
    }
    fn patch(&mut self, face: &[usize]) -> Option<Vec<Vec<usize>>> {
        let count = self.count;
        let m = face.len();
        if m < 3 {
            return None;
        }
        let mut sides = Vec::new();
        for i in 0..m {
            let a = face[i];
            let b = face[(i + 1) % m];
            let mut seq = self.chains.get(&pair(a, b))?.clone();
            if seq.len() != count + 1 {
                return None;
            }
            if seq[0] != a {
                seq.reverse();
            }
            sides.push(seq);
        }
        let pts: Vec<Vec<_>> = sides
            .iter()
            .map(|s| s.iter().map(|&i| self.pos[i]).collect())
            .collect();
        let looppts: Vec<_> = pts
            .iter()
            .flat_map(|q| q[..q.len() - 1].iter().copied())
            .collect();
        let ws = if self.shape == "flat" {
            0.
        } else {
            self.convexity
        };
        let sphere = if ws > 0. && looppts.len() >= 6 {
            sphere_fit(&looppts)
        } else {
            None
        };
        let project = |q: [f64; 3], w: f64| {
            if let Some((center, radius)) = sphere {
                let d = sub(q, center);
                let on = add(center, mul(d, radius / len(d).max(1e-12)));
                add(q, mul(sub(on, q), ws * w))
            } else {
                q
            }
        };
        let along = |q: &[[f64; 3]], t: f64| {
            let x = t.clamp(0., 1.) * count as f64;
            let i = (x.floor() as usize).min(count - 1);
            let f = x - i as f64;
            add(mul(q[i], 1. - f), mul(q[i + 1], f))
        };
        let apex = project(
            mul(
                looppts.iter().copied().fold([0.; 3], add),
                1. / looppts.len() as f64,
            ),
            1.,
        );
        let mids: Vec<_> = pts.iter().map(|q| along(q, 0.5)).collect();
        let seam =
            |i: usize, y: f64| project(add(mul(mids[i], 1. - y), mul(apex, y)), (8. * y).min(1.));
        let h = count / 2;
        let odd = count % 2 == 1;
        let mut ids = BTreeMap::<(usize, usize, usize, usize), usize>::new();
        let mut grids = Vec::new();
        for i in 0..m {
            let prev = (i + m - 1) % m;
            let mut grid = vec![vec![0; h + 1]; h + 1];
            for a in 0..=h {
                for b in 0..=h {
                    let x = 2. * a as f64 / count as f64;
                    let y = 2. * b as f64 / count as f64;
                    let bottom = along(&pts[i], x / 2.);
                    let left = along(&pts[prev], 1. - y / 2.);
                    let right = seam(i, y);
                    let top = seam(prev, x);
                    let f = sub(
                        add(
                            add(mul(bottom, 1. - y), mul(top, y)),
                            add(mul(left, 1. - x), mul(right, x)),
                        ),
                        add(
                            add(
                                mul(pts[i][0], (1. - x) * (1. - y)),
                                mul(mids[i], x * (1. - y)),
                            ),
                            add(mul(mids[prev], (1. - x) * y), mul(apex, x * y)),
                        ),
                    );
                    let f = project(f, (8. * x.min(y)).min(1.));
                    grid[a][b] = if a == 0 && b == 0 {
                        face[i]
                    } else if b == 0 {
                        sides[i][a]
                    } else if a == 0 {
                        sides[prev][count - b]
                    } else {
                        let key = if !odd && a == h && b == h {
                            (0, 0, 0, 0)
                        } else if !odd && a == h {
                            (1, i, b, 0)
                        } else if !odd && b == h {
                            (1, prev, a, 0)
                        } else {
                            (2, i, a, b)
                        };
                        if let Some(&id) = ids.get(&key) {
                            id
                        } else {
                            let id = self.add_point(f, face[i], face[i], 0.);
                            ids.insert(key, id);
                            id
                        }
                    };
                }
            }
            grids.push(grid);
        }
        let mut out = Vec::new();
        for g in &grids {
            for a in 0..h {
                for b in 0..h {
                    out.push(vec![g[a][b], g[a + 1][b], g[a + 1][b + 1], g[a][b + 1]]);
                }
            }
        }
        if odd {
            for i in 0..m {
                let g = &grids[i];
                let next = &grids[(i + 1) % m];
                for v in 0..h {
                    out.push(vec![g[h][v], next[v][h], next[v + 1][h], g[h][v + 1]]);
                }
            }
            out.push(grids.iter().map(|g| g[h][h]).collect());
        }
        Some(out)
    }
}
fn sphere_fit(points: &[[f64; 3]]) -> Option<([f64; 3], f64)> {
    let rows: Vec<[f64; 4]> = points
        .iter()
        .map(|q| [2. * q[0], 2. * q[1], 2. * q[2], 1.])
        .collect();
    let rhs: Vec<_> = points.iter().map(|&q| dot(q, q)).collect();
    let sol = least_squares(&rows, &rhs, points.len().max(4) as f64 * f64::EPSILON);
    let center = [sol[0], sol[1], sol[2]];
    let r2 = sol[3] + dot(center, center);
    if r2 <= 0. {
        return None;
    }
    let radius = r2.sqrt();
    let miss = points
        .iter()
        .map(|&p| (len(sub(p, center)) - radius).abs())
        .fold(0., f64::max);
    let mut extent = 0.0f64;
    for k in 0..3 {
        extent = extent.max(
            points
                .iter()
                .map(|p| p[k])
                .fold(f64::NEG_INFINITY, f64::max)
                - points.iter().map(|p| p[k]).fold(f64::INFINITY, f64::min),
        );
    }
    if miss <= 0.2 * radius && radius <= 25. * extent.max(1e-12) {
        Some((center, radius))
    } else {
        None
    }
}
fn segments(
    g: &Geometry,
    first: usize,
    strips: usize,
    count: usize,
    shape: &str,
    convexity: f64,
    homes: &[[f64; 3]],
    p: &Params,
    profile: Option<&[[f64; 2]]>,
) -> Result<Geometry> {
    let faces: Vec<Vec<_>> = (0..g.nprims())
        .map(|i| {
            range(g, i)
                .map(|v| g.topology.vertex_point[v] as usize)
                .collect()
        })
        .collect();
    let mut shared = BTreeMap::new();
    for f in &faces[first..first + strips] {
        for (a, b) in [(f[0], f[3]), (f[1], f[2])] {
            *shared.entry(pair(a, b)).or_insert(0) += 1;
        }
    }
    let mut r = Refiner {
        pos: positions(g),
        ancestry: (0..g.npoints).map(|i| (i, i, 0.)).collect(),
        chains: BTreeMap::new(),
        homes,
        shared,
        count,
        shape,
        convexity,
        profile,
    };
    let mut refined = Vec::new();
    for f in &faces[first..first + strips] {
        let [a, b, c, d] = [f[0], f[1], f[2], f[3]];
        let axis = sub(r.pos[b], r.pos[a]);
        let left = r.chain(a, d, axis);
        let right = r.chain(b, c, axis);
        refined.push(
            (0..count)
                .map(|j| vec![left[j], right[j], right[j + 1], left[j + 1]])
                .collect::<Vec<_>>(),
        );
    }
    let mut newfaces = Vec::new();
    let mut parents = Vec::new();
    for (f, face) in faces.iter().enumerate() {
        let made = if f >= first && f < first + strips {
            refined[f - first].clone()
        } else {
            let mut boundary = Vec::new();
            for j in 0..face.len() {
                let a = face[j];
                let b = face[(j + 1) % face.len()];
                let mut seq = r.chains.get(&pair(a, b)).cloned().unwrap_or(vec![a, b]);
                if seq[0] != a {
                    seq.reverse();
                }
                boundary.extend(&seq[..seq.len() - 1]);
            }
            if f >= first + strips {
                if let Some(made) = r.patch(face) {
                    made
                } else {
                    let center = mul(
                        boundary.iter().map(|&i| r.pos[i]).fold([0.; 3], add),
                        1. / boundary.len() as f64,
                    );
                    let id = r.add_point(center, face[0], face[0], 0.);
                    (0..boundary.len())
                        .map(|i| vec![boundary[i], boundary[(i + 1) % boundary.len()], id])
                        .collect()
                }
            } else {
                vec![boundary]
            }
        };
        parents.extend(std::iter::repeat_n(f, made.len()));
        newfaces.extend(made);
    }
    let mut va = Vec::new();
    let mut vb = Vec::new();
    let mut vt = Vec::new();
    for (face, &parent) in newfaces.iter().zip(&parents) {
        let mapping: BTreeMap<_, _> = faces[parent]
            .iter()
            .copied()
            .zip(range(g, parent))
            .collect();
        let fallback = range(g, parent).start;
        for &q in face {
            let (a, b, t) = r.ancestry[q];
            va.push(*mapping.get(&a).unwrap_or(&fallback));
            vb.push(*mapping.get(&b).unwrap_or(&fallback));
            vt.push(t);
        }
    }
    let mut vertex = lerp(&g.vertex, &va, &vb, &vt);
    vertex.retain(|n, _| !n.starts_with("edge_group_"));
    let ia: Vec<_> = r.ancestry.iter().map(|r| r.0).collect();
    let ib: Vec<_> = r.ancestry.iter().map(|r| r.1).collect();
    let fr: Vec<_> = r.ancestry.iter().map(|r| r.2).collect();
    let mut point = lerp(&g.point, &ia, &ib, &fr);
    point.insert(
        "P".into(),
        Arc::new(Attr::f32(
            3,
            r.pos.iter().flat_map(|q| q.map(|x| x as f32)).collect(),
        )),
    );
    let mut out = make(
        point,
        newfaces.iter().flatten().map(|&x| x as i32).collect(),
        &newfaces.iter().map(Vec::len).collect::<Vec<_>>(),
        vertex,
        gather(&g.prim, &parents),
        g.detail.clone(),
        None,
    )?;
    let e = topo::edges(&g.topology, g.npoints);
    for name in g.vertex.keys().filter(|n| n.starts_with("edge_group_")) {
        let old = edges::values(g, name)?;
        let mut pairs = Vec::new();
        for (i, q) in e.points.chunks_exact(2).enumerate() {
            if old[i] > 0. {
                let a = q[0] as usize;
                let b = q[1] as usize;
                let seq = r.chains.get(&pair(a, b)).cloned().unwrap_or(vec![a, b]);
                pairs.extend(seq.windows(2).map(|p| (p[0], p[1])));
            }
        }
        let mask = pair_mask(&out, &pairs);
        edges::write_values(
            &mut out,
            name,
            &mask.iter().map(|&x| x as u8 as f32).collect::<Vec<_>>(),
            true,
        )?;
    }
    if p.b("outgrp_offsetpoints")? {
        let name = p.s("outgrp_offsetpoints_name")?;
        if !name.trim().is_empty() {
            let mut mask = groups::point_mask(g, name.trim())?;
            mask.resize(out.npoints, false);
            outgroup(&mut out, p, "offsetpoints", "point", &mask)?;
        }
    }
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn cube() -> Geometry {
        let spec = super::super::get("box").unwrap();
        let p = Params::resolve(spec.params, None).unwrap();
        // Inputs contains one slot per declared socket, including an unconnected
        // optional geo input on the box generator.
        (spec.cook)(&[vec![]], &p).unwrap()
    }
    #[test]
    fn chamfer_and_round_are_closed() {
        for count in [1, 2, 3, 4] {
            let g = cube();
            let p =
                Params::resolve(POLYBEVEL.params, json!({"divisions":count}).as_object()).unwrap();
            let out = bevel(&[vec![Arc::new(g)], vec![]], &p).unwrap();
            out.validate().unwrap();
            assert!(
                topo::edges(&out.topology, out.npoints)
                    .count
                    .iter()
                    .all(|&c| c == 2)
            );
            assert!(out.prim.contains_key("group_bevel"));
            assert!(!out.point.contains_key("N"));
        }
    }
    #[test]
    fn no_bevel_writes_empty_requested_groups() {
        let g = cube();
        let p = Params::resolve(
            POLYBEVEL.params,
            json!({"distance":0.,"outgrp_offsetpoints":true}).as_object(),
        )
        .unwrap();
        let out = bevel(&[vec![Arc::new(g.clone())], vec![]], &p).unwrap();
        assert_eq!(out.p(), g.p());
        assert_eq!(
            out.point["group_offsetpoints"].data,
            AttrData::I32(vec![0; g.npoints])
        );
    }
}
