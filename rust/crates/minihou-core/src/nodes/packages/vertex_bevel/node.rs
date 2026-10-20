//! Bevel manifold triangle/quad vertices with a shrinking corner grid.
use crate::{
    error::{Error, Result},
    geometry::{Attr, Geometry},
    nodes::{
        groups,
        model::{add, gather, len, lerp, make, mul, positions, range, sub},
        Inputs, NodeSpec, GEO,
    },
    params::Params,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

pub(super) const NODE: NodeSpec = NodeSpec {
    outputs: &["geometry"],
    cook_multi: None,
    name: "vertex_bevel",
    inputs: GEO,
    params: crate::params!("group": ""; "width": 0.1; "segments": 1; "profile": 0.5),
    cook,
};

type Edge = (usize, usize);
fn edge(a: usize, b: usize) -> Edge {
    (a.min(b), a.max(b))
}

fn arc_position(
    start: [f64; 3],
    end: [f64; 3],
    corner: [f64; 3],
    t: f64,
    profile: f64,
) -> [f64; 3] {
    let line = add(mul(start, 1.0 - t), mul(end, t));
    // At profile 0.5, place the midpoint of a right-angle arc at
    // (1 - sqrt(1/2)) along each edge, versus 1/2 for the chord.
    let bulge = 2.0 * (std::f64::consts::SQRT_2 - 1.0) * profile;
    add(line, mul(sub(corner, line), 4.0 * t * (1.0 - t) * bulge))
}

fn append_point(
    outpos: &mut Vec<[f64; 3]>,
    source_a: &mut Vec<usize>,
    source_b: &mut Vec<usize>,
    fractions: &mut Vec<f64>,
    position: [f64; 3],
    source: usize,
) -> usize {
    let id = outpos.len();
    outpos.push(position);
    source_a.push(source);
    source_b.push(source);
    fractions.push(0.0);
    id
}

#[derive(Clone, Copy)]
struct Corner {
    face: usize,
    vertex: usize,
    incoming: usize,
    outgoing: usize,
}

fn cook(inputs: &Inputs, params: &Params) -> Result<Geometry> {
    let g = NODE.one(inputs, 0)?;
    let width = params.f("width")?;
    if !width.is_finite() || width < 0.0 {
        return Err(Error::new("width must be finite and nonnegative"));
    }
    let segments = params.i("segments")?;
    if !(1..=16).contains(&segments) {
        return Err(Error::new("segments must be 1..16"));
    }
    let profile = params.f("profile")?;
    if !profile.is_finite() || !(0.0..=1.0).contains(&profile) {
        return Err(Error::new("profile must be 0..1"));
    }
    if width == 0.0 {
        return Ok((*g).clone());
    }
    let selected = groups::point_mask(&g, &params.s("group")?)?;
    let pos = positions(&g);
    let vp = &g.topology.vertex_point;
    let mut corners = vec![Vec::<Corner>::new(); g.npoints];
    let mut open_incident = vec![false; g.npoints];
    let mut edge_uses = BTreeMap::<Edge, Vec<(usize, usize, usize)>>::new();
    for face in 0..g.nprims() {
        let r = range(&g, face);
        if g.topology
            .closed
            .as_ref()
            .is_some_and(|closed| !closed[face])
        {
            for vertex in r {
                open_incident[vp[vertex] as usize] = true;
            }
            continue;
        }
        for vertex in r.clone() {
            let next = if vertex + 1 == r.end {
                r.start
            } else {
                vertex + 1
            };
            let prev = if vertex == r.start {
                r.end - 1
            } else {
                vertex - 1
            };
            let point = vp[vertex] as usize;
            let incoming = vp[prev] as usize;
            let outgoing = vp[next] as usize;
            corners[point].push(Corner {
                face,
                vertex,
                incoming,
                outgoing,
            });
            edge_uses
                .entry(edge(point, outgoing))
                .or_default()
                .push((face, point, outgoing));
        }
    }
    let mut accepted = BTreeMap::<usize, Vec<Corner>>::new();
    let mut skipped = Vec::new();
    for point in 0..g.npoints {
        if !selected[point] {
            continue;
        }
        let incident = &corners[point];
        let reason = if open_incident[point] || incident.len() < 3 {
            Some("open or low-valence neighbourhood")
        } else if incident
            .iter()
            .any(|c| !matches!(range(&g, c.face).len(), 3 | 4))
        {
            Some("face is not a triangle or quad")
        } else if incident
            .iter()
            .any(|c| c.incoming == point || c.outgoing == point || c.incoming == c.outgoing)
        {
            Some("degenerate face")
        } else {
            let neighbours: BTreeSet<_> = incident
                .iter()
                .flat_map(|c| [c.incoming, c.outgoing])
                .collect();
            if neighbours.len() != incident.len()
                || neighbours.iter().any(|&n| {
                    let uses = edge_uses.get(&edge(point, n));
                    uses.is_none_or(|u| {
                        u.len() != 2
                            || !u.iter().any(|&(_, a, b)| a == point && b == n)
                            || !u.iter().any(|&(_, a, b)| a == n && b == point)
                    })
                })
            {
                Some("open or nonmanifold neighbourhood")
            } else if neighbours
                .iter()
                .any(|&n| len(sub(pos[n], pos[point])) <= 1e-12)
            {
                Some("zero-length edge")
            } else {
                None
            }
        };
        if let Some(reason) = reason {
            skipped.push(serde_json::json!({"point": point, "reason": reason}));
            continue;
        }
        // Follow the opposite-wound cap boundary: each face's outgoing cut
        // meets the next face's incoming cut on the same original edge.
        let mut ring = Vec::with_capacity(incident.len());
        let mut at = incident[0];
        loop {
            if ring.iter().any(|c: &Corner| c.vertex == at.vertex) {
                break;
            }
            ring.push(at);
            let Some(next) = incident.iter().find(|c| c.outgoing == at.incoming) else {
                break;
            };
            at = *next;
        }
        if ring.len() != incident.len() || at.vertex != incident[0].vertex {
            skipped.push(serde_json::json!({"point": point, "reason": "disconnected vertex fan"}));
        } else {
            accepted.insert(point, ring);
        }
    }
    if accepted.is_empty() {
        let mut out = (*g).clone();
        if !skipped.is_empty() {
            out.detail
                .insert("vertex_bevel_skipped".into(), skipped.into());
        }
        return Ok(out);
    }
    let mut outpos = pos.clone();
    let mut source_a: Vec<usize> = (0..g.npoints).collect();
    let mut source_b = source_a.clone();
    let mut fractions = vec![0.0; g.npoints];
    let mut cut = BTreeMap::<(usize, usize), usize>::new();
    for (&point, ring) in &accepted {
        for c in ring {
            let neighbour = c.outgoing;
            let distance = len(sub(pos[neighbour], pos[point]));
            let other_width = if accepted.contains_key(&neighbour) {
                width
            } else {
                0.0
            };
            let fraction = (width / distance).min(if other_width > 0.0 { 0.49 } else { 0.9 });
            let id = outpos.len();
            outpos.push(add(
                pos[point],
                mul(sub(pos[neighbour], pos[point]), fraction),
            ));
            source_a.push(point);
            source_b.push(neighbour);
            fractions.push(fraction);
            cut.insert((point, neighbour), id);
        }
    }
    let mut faces = Vec::<Vec<usize>>::new();
    let mut prim_source = Vec::new();
    let mut vertex_source = Vec::new();
    // Each face-side arc runs from this corner's outgoing edge to the
    // next corner's outgoing edge (this face's incoming edge).
    let mut arcs = BTreeMap::<usize, Vec<usize>>::new();
    let mut grids = BTreeMap::<usize, Vec<Vec<Vec<usize>>>>::new();
    for (&point, ring) in &accepted {
        let n = ring.len();
        let mut layers = Vec::new();
        let mut boundary = Vec::with_capacity(n);
        let center = ring.iter().fold([0.0; 3], |sum, c| {
            add(sum, outpos[cut[&(point, c.outgoing)]])
        });
        let center = add(
            mul(center, (1.0 - profile * 0.5) / n as f64),
            mul(pos[point], profile * 0.5),
        );
        for i in 0..n {
            let start = outpos[cut[&(point, ring[i].outgoing)]];
            let end = outpos[cut[&(point, ring[(i + 1) % n].outgoing)]];
            let mut side = vec![cut[&(point, ring[i].outgoing)]];
            for k in 1..segments as usize {
                let t = k as f64 / segments as f64;
                let p = arc_position(start, end, pos[point], t, profile);
                side.push(append_point(
                    &mut outpos,
                    &mut source_a,
                    &mut source_b,
                    &mut fractions,
                    p,
                    point,
                ));
            }
            arcs.insert(ring[i].vertex, side.clone());
            boundary.push(side);
        }
        layers.push(boundary);
        for j in 1..=(segments as usize / 2) {
            let m = segments as usize - 2 * j;
            if m == 0 {
                break;
            }
            let mut layer = Vec::with_capacity(n);
            for i in 0..n {
                let start = outpos[cut[&(point, ring[i].outgoing)]];
                let end = outpos[cut[&(point, ring[(i + 1) % n].outgoing)]];
                let mut side = Vec::with_capacity(m);
                for k in 0..m {
                    let t = (j + k) as f64 / segments as f64;
                    let p = arc_position(start, end, pos[point], t, profile);
                    let p = add(
                        mul(p, 1.0 - j as f64 / (segments as f64 / 2.0)),
                        mul(center, j as f64 / (segments as f64 / 2.0)),
                    );
                    side.push(append_point(
                        &mut outpos,
                        &mut source_a,
                        &mut source_b,
                        &mut fractions,
                        p,
                        point,
                    ));
                }
                layer.push(side);
            }
            layers.push(layer);
        }
        grids.insert(point, layers);
    }
    for face in 0..g.nprims() {
        let r = range(&g, face);
        let mut made = Vec::new();
        for vertex in r.clone() {
            let point = vp[vertex] as usize;
            if accepted.contains_key(&point) {
                let prev = if vertex == r.start {
                    r.end - 1
                } else {
                    vertex - 1
                };
                let next = if vertex + 1 == r.end {
                    r.start
                } else {
                    vertex + 1
                };
                made.push(cut[&(point, vp[prev] as usize)]);
                vertex_source.push(vertex);
                if let Some(arc) = arcs.get(&vertex) {
                    for &id in arc.iter().skip(1).rev() {
                        made.push(id);
                        vertex_source.push(vertex);
                    }
                }
                made.push(cut[&(point, vp[next] as usize)]);
                vertex_source.push(vertex);
            } else {
                made.push(point);
                vertex_source.push(vertex);
            }
        }
        faces.push(made);
        prim_source.push(face);
    }
    for (&point, ring) in &accepted {
        let layers = &grids[&point];
        for j in 0..layers.len() {
            let outer = &layers[j];
            let m = outer[0].len();
            if m < 2 {
                break;
            }
            if m == 2 {
                let center = ring.iter().fold([0.0; 3], |sum, c| {
                    add(sum, outpos[cut[&(point, c.outgoing)]])
                });
                let center = add(
                    mul(center, (1.0 - profile * 0.5) / ring.len() as f64),
                    mul(pos[point], profile * 0.5),
                );
                let id = append_point(
                    &mut outpos,
                    &mut source_a,
                    &mut source_b,
                    &mut fractions,
                    center,
                    point,
                );
                for i in 0..ring.len() {
                    faces.push(vec![
                        outer[(i + ring.len() - 1) % ring.len()][1],
                        outer[i][0],
                        outer[i][1],
                        id,
                    ]);
                    prim_source.push(ring[i].face);
                    vertex_source.extend([ring[i].vertex; 4]);
                }
            } else {
                let inner = &layers[j + 1];
                for i in 0..ring.len() {
                    faces.push(vec![
                        outer[(i + ring.len() - 1) % ring.len()][m - 1],
                        outer[i][0],
                        outer[i][1],
                        inner[i][0],
                    ]);
                    prim_source.push(ring[i].face);
                    vertex_source.extend([ring[i].vertex; 4]);
                    for k in 1..m - 1 {
                        let next = if k == m - 2 {
                            inner[(i + 1) % ring.len()][0]
                        } else {
                            inner[i][k]
                        };
                        faces.push(vec![outer[i][k], outer[i][k + 1], next, inner[i][k - 1]]);
                        prim_source.push(ring[i].face);
                        vertex_source.extend([ring[i].vertex; 4]);
                    }
                }
            }
        }
        if segments % 2 == 1 {
            let inner = layers.last().unwrap();
            faces.push(inner.iter().map(|side| side[0]).collect());
            prim_source.push(ring[0].face);
            vertex_source.extend(ring.iter().map(|c| c.vertex));
        }
    }
    let mut point = lerp(&g.point, &source_a, &source_b, &fractions);
    point.insert(
        "P".into(),
        Arc::new(Attr::f32(
            3,
            outpos.iter().flat_map(|p| p.map(|x| x as f32)).collect(),
        )),
    );
    let closed = g.topology.closed.as_ref().map(|flags| {
        let mut flags = flags.clone();
        flags.resize(faces.len(), true);
        flags
    });
    let mut out = make(
        point,
        faces.iter().flatten().map(|&id| id as i32).collect(),
        &faces.iter().map(Vec::len).collect::<Vec<_>>(),
        gather(&g.vertex, &vertex_source),
        gather(&g.prim, &prim_source),
        g.detail.clone(),
        closed,
    )?;
    // Remove the original selected points, which no face uses after trimming.
    out = crate::nodes::model::compact(out)?;
    out.point.remove("N");
    out.vertex.remove("N");
    out.prim.remove("N");
    if !skipped.is_empty() {
        out.detail
            .insert("vertex_bevel_skipped".into(), skipped.into());
    }
    Ok(out)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
