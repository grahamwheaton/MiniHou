//! Separate adjacent polygons across sharp or selected edges.
use std::{collections::BTreeMap, sync::Arc};

use crate::{
    error::{Error, Result},
    geometry::{Geometry, MAX_ELEMENTS},
    nodes::{edge_tools, model, Inputs, NodeSpec, GEO},
    normals,
    params::Params,
    topo,
};

pub(super) const NODE: NodeSpec = NodeSpec {
    outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "edge_split", inputs: GEO,
    params: crate::params!("angle": 30.0; "mode": "sharp_edges"; "group": ""),
    cook,
};

fn root(parent: &mut [usize], mut x: usize) -> usize {
    while parent[x] != x {
        parent[x] = parent[parent[x]];
        x = parent[x];
    }
    x
}

fn endpoint(v: usize, point: usize, vp: &[i32], next: &[usize]) -> usize {
    if vp[v] as usize == point { v } else { next[v] }
}

fn angle_exceeds(a: [f64; 3], b: [f64; 3], degrees: f64) -> bool {
    let length = |n: [f64; 3]| (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    let scale = length(a) * length(b);
    scale > 1e-20 && ((a[0] * b[0] + a[1] * b[1] + a[2] * b[2]) / scale)
        .clamp(-1.0, 1.0).acos().to_degrees() > degrees
}

fn cook(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = NODE.one(inputs, 0)?;
    let angle = p.f("angle")?;
    if !angle.is_finite() || !(0.0..=180.0).contains(&angle) {
        return Err(Error::new("angle must be between 0 and 180 degrees"));
    }
    let mode = p.e("mode", &["sharp_edges", "selected_edges"])?;
    let group = p.s("group")?;
    let edges = topo::edges(&g.topology, g.npoints);
    let (face, next, _) = model::corners(&g);
    let vp = &g.topology.vertex_point;
    let mut uses = vec![Vec::<usize>::new(); edges.len()];
    for (v, &id) in edges.of_vertex.iter().enumerate() {
        if id >= 0 && g.topology.is_closed(face[v]) {
            uses[id as usize].push(v);
        }
    }
    let split = if mode == "selected_edges" {
        if group.trim().is_empty() { vec![false; edges.len()] }
        else { edge_tools::edge_mask(&g, &group)? }
    } else {
        let normal = normals::prim_normals(g.p(), &g.topology);
        uses.iter().map(|vertices| vertices.iter().enumerate().any(|(i, &a)| {
            vertices[i + 1..].iter().any(|&b| angle_exceeds(normal[face[a]], normal[face[b]], angle))
        })).collect()
    };
    if !split.iter().enumerate().any(|(i, &chosen)| chosen && uses[i].len() > 1) {
        return Ok((*g).clone());
    }

    // Each corner starts with its own point identity. An uncut shared edge welds
    // the two face corners at both endpoints. A cut edge forbids that weld, even
    // when another route around a closed vertex would otherwise rejoin them.
    let mut forbidden = vec![Vec::<(usize, usize)>::new(); g.npoints];
    for (id, vertices) in uses.iter().enumerate() {
        if !split[id] { continue; }
        let a = edges.points[2 * id] as usize;
        let b = edges.points[2 * id + 1] as usize;
        for (i, &u) in vertices.iter().enumerate() {
            for &v in &vertices[i + 1..] {
                forbidden[a].push((endpoint(u, a, vp, &next), endpoint(v, a, vp, &next)));
                forbidden[b].push((endpoint(u, b, vp, &next), endpoint(v, b, vp, &next)));
            }
        }
    }
    let mut parent: Vec<usize> = (0..g.nvertices()).collect();
    for (id, vertices) in uses.iter().enumerate() {
        if split[id] || vertices.len() != 2 { continue; }
        let a = edges.points[2 * id] as usize;
        let b = edges.points[2 * id + 1] as usize;
        for point in [a, b] {
            let left = root(&mut parent, endpoint(vertices[0], point, vp, &next));
            let right = root(&mut parent, endpoint(vertices[1], point, vp, &next));
            if left == right { continue; }
            let blocked = forbidden[point].iter().any(|&(x, y)| {
                let x = root(&mut parent, x);
                let y = root(&mut parent, y);
                (x == left && y == right) || (x == right && y == left)
            });
            if !blocked { parent[right] = left; }
        }
    }

    let mut source_points: Vec<usize> = (0..g.npoints).collect();
    let mut components = BTreeMap::<(usize, usize), usize>::new();
    let mut first = vec![false; g.npoints];
    let mut new_vp = vp.clone();
    for (v, &old) in vp.iter().enumerate() {
        let point = old as usize;
        if forbidden[point].is_empty() { continue; }
        let component = root(&mut parent, v);
        let new = *components.entry((point, component)).or_insert_with(|| {
            if !first[point] { first[point] = true; point }
            else { let id = source_points.len(); source_points.push(point); id }
        });
        if new >= MAX_ELEMENTS { return Err(Error::new("edge split exceeds the point limit")); }
        new_vp[v] = new as i32;
    }
    let mut out = (*g).clone();
    out.npoints = source_points.len();
    out.point = model::gather(&g.point, &source_points);
    let mut topology = (*g.topology).clone();
    topology.vertex_point = new_vp;
    topology.clear_edge_cache();
    out.topology = Arc::new(topology);

    // Edge-domain rows follow derived edge order, which changes after unwelding.
    let old_pairs: BTreeMap<_, _> = g.topology.edge_pairs().into_iter().enumerate()
        .map(|(i, pair)| (pair, i)).collect();
    let edge_sources: Vec<usize> = out.topology.edge_pairs().into_iter().map(|(a, b)| {
        let (a, b) = (source_points[a as usize] as i32, source_points[b as usize] as i32);
        old_pairs[&(a.min(b), a.max(b))]
    }).collect();
    out.edge = model::gather(&g.edge, &edge_sources);
    out.validate()?;
    Ok(out)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
