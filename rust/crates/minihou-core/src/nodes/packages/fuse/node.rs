use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use crate::error::{Error, Result};
use crate::geometry::{Attr, Geometry, Topology};
use crate::nodes::{groups, model, InputSpec, Inputs, NodeSpec};
use crate::params::Params;

pub(super) const NODE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "fuse",
    inputs: &[
        InputSpec {
            name: "geo",
            multi: false,
            optional: false,
        },
        InputSpec {
            name: "targets",
            multi: false,
            optional: true,
        },
    ],
    params: crate::params!(
        "distance": 0.001;
        "group": "";
        "snap_to": "near_points";
        "fuse_points": true;
        "output_positions": "first";
        "remove_degenerate": true;
        "remove_unused_points": true;
        "outgrp_fused": false;
        "outgrp_fused_name": "fusedpoints";
        "outgrp_unique": false;
        "outgrp_unique_name": "uniquepoints";
    ),
    cook,
};

struct UnionFind {
    parent: Vec<usize>,
}

impl UnionFind {
    fn new(n: usize) -> Self {
        Self {
            parent: (0..n).collect(),
        }
    }

    fn find(&mut self, i: usize) -> usize {
        let mut root = i;
        while self.parent[root] != root {
            root = self.parent[root];
        }
        let mut curr = i;
        while curr != root {
            let next = self.parent[curr];
            self.parent[curr] = root;
            curr = next;
        }
        root
    }

    fn union(&mut self, i: usize, j: usize) {
        let ri = self.find(i);
        let rj = self.find(j);
        if ri != rj {
            if ri < rj {
                self.parent[rj] = ri;
            } else {
                self.parent[ri] = rj;
            }
        }
    }
}

fn drop_degenerate(g: &Geometry) -> Result<Geometry> {
    if g.nprims() == 0 {
        return Ok(g.clone());
    }
    let topo = &g.topology;
    let nprims = g.nprims();
    let vp = &topo.vertex_point;
    let offs = &topo.prim_offsets;

    let mut kept_vp = Vec::new();
    let mut kept_sizes = Vec::new();
    let mut kept_vert_indices = Vec::new();
    let mut kept_prims = Vec::new();
    let mut kept_closed = Vec::new();

    for prim in 0..nprims {
        let start = offs[prim] as usize;
        let end = offs[prim + 1] as usize;
        let count = end.saturating_sub(start);
        if count == 0 {
            continue;
        }
        let is_closed = topo.is_closed(prim);

        let mut corners = Vec::new();
        let mut vert_ix = Vec::new();
        for j in 0..count {
            let pt = vp[start + j];
            if j + 1 < count && pt == vp[start + j + 1] {
                continue;
            }
            corners.push(pt);
            vert_ix.push(start + j);
        }

        if is_closed && corners.len() > 1 && corners.first() == corners.last() {
            corners.pop();
            vert_ix.pop();
        }

        let min_corners = if is_closed { 3 } else { 2 };
        if corners.len() < min_corners {
            continue;
        }

        kept_vp.extend(corners);
        kept_vert_indices.extend(vert_ix);
        kept_sizes.push(count);
        kept_prims.push(prim);
        kept_closed.push(is_closed);
    }

    let closed_opt = if topo.has_open() {
        Some(kept_closed)
    } else {
        None
    };

    model::make(
        g.point.clone(),
        kept_vp,
        &kept_sizes,
        model::gather(&g.vertex, &kept_vert_indices),
        model::gather(&g.prim, &kept_prims),
        g.detail.clone(),
        closed_opt,
    )
}

fn cook(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = NODE.one(inputs, 0)?;
    if geo.npoints == 0 {
        return Ok((*geo).clone());
    }

    let distance = p.f("distance")?;
    let group = p.s("group")?;
    let snap_to = p.e("snap_to", &["near_points", "grid"])?;
    let fuse_pts = p.b("fuse_points")?;
    let output_pos = p.e("output_positions", &["first", "average"])?;
    let remove_deg = p.b("remove_degenerate")?;
    let remove_unused = p.b("remove_unused_points")?;
    let outgrp_fused = p.b("outgrp_fused")?;
    let outgrp_fused_name = p.s("outgrp_fused_name")?;
    let outgrp_unique = p.b("outgrp_unique")?;
    let outgrp_unique_name = p.s("outgrp_unique_name")?;

    let p_slice = geo.p();
    let npoints = geo.npoints;
    let in_group = if group.trim().is_empty() {
        vec![true; npoints]
    } else {
        groups::point_mask(&geo, &group)?
    };

    let d = distance.max(1e-12);

    // Target snapping mode
    if let Some(targets) = inputs.get(1).and_then(|t| t.first()).filter(|g| g.npoints > 0) {
        let tgt_p = targets.p();
        let tgt_npoints = targets.npoints;

        let mut hit = vec![false; npoints];
        let mut new_p = p_slice.to_vec();

        // Spatial grid for targets
        let mut target_grid: HashMap<[i64; 3], Vec<usize>> = HashMap::new();
        for i in 0..tgt_npoints {
            let key = [
                (tgt_p[i * 3] as f64 / d).floor() as i64,
                (tgt_p[i * 3 + 1] as f64 / d).floor() as i64,
                (tgt_p[i * 3 + 2] as f64 / d).floor() as i64,
            ];
            target_grid.entry(key).or_default().push(i);
        }

        for i in 0..npoints {
            if !in_group[i] {
                continue;
            }
            let pos = [
                p_slice[i * 3] as f64,
                p_slice[i * 3 + 1] as f64,
                p_slice[i * 3 + 2] as f64,
            ];
            let cell = [
                (pos[0] / d).floor() as i64,
                (pos[1] / d).floor() as i64,
                (pos[2] / d).floor() as i64,
            ];

            let mut nearest_tgt = None;
            let mut nearest_dist_sq = d * d;

            for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        let neighbor_cell = [cell[0] + dx, cell[1] + dy, cell[2] + dz];
                        if let Some(list) = target_grid.get(&neighbor_cell) {
                            for &ti in list {
                                let tpos = [
                                    tgt_p[ti * 3] as f64,
                                    tgt_p[ti * 3 + 1] as f64,
                                    tgt_p[ti * 3 + 2] as f64,
                                ];
                                let dist_sq = (pos[0] - tpos[0]).powi(2)
                                    + (pos[1] - tpos[1]).powi(2)
                                    + (pos[2] - tpos[2]).powi(2);
                                if dist_sq <= nearest_dist_sq {
                                    nearest_dist_sq = dist_sq;
                                    nearest_tgt = Some(ti);
                                }
                            }
                        }
                    }
                }
            }

            if let Some(ti) = nearest_tgt {
                hit[i] = true;
                if output_pos == "average" {
                    new_p[i * 3] = ((pos[0] + tgt_p[ti * 3] as f64) * 0.5) as f32;
                    new_p[i * 3 + 1] = ((pos[1] + tgt_p[ti * 3 + 1] as f64) * 0.5) as f32;
                    new_p[i * 3 + 2] = ((pos[2] + tgt_p[ti * 3 + 2] as f64) * 0.5) as f32;
                } else {
                    new_p[i * 3] = tgt_p[ti * 3];
                    new_p[i * 3 + 1] = tgt_p[ti * 3 + 1];
                    new_p[i * 3 + 2] = tgt_p[ti * 3 + 2];
                }
            }
        }

        let mut out = (*geo).clone();
        out.set_p(new_p);

        if fuse_pts {
            let mut counts = Vec::new();
            out = crate::nodes::primitives::fuse_points(&out, d, Some(&mut counts))?;
            if remove_deg && out.nprims() > 0 {
                out = drop_degenerate(&out)?;
            }
            if remove_unused && out.nprims() > 0 {
                out = model::compact(out)?;
            }
        }

        if outgrp_fused && !outgrp_fused_name.trim().is_empty() {
            let m: Vec<bool> = hit.iter().map(|&h| h).collect();
            if m.len() == out.npoints {
                model::write_group(&mut out, "point", outgrp_fused_name.trim(), &m)?;
            }
        }
        if outgrp_unique && !outgrp_unique_name.trim().is_empty() {
            let m: Vec<bool> = hit.iter().map(|&h| !h).collect();
            if m.len() == out.npoints {
                model::write_group(&mut out, "point", outgrp_unique_name.trim(), &m)?;
            }
        }

        return Ok(out);
    }

    // Self-fuse mode
    let mut uf = UnionFind::new(npoints);

    if snap_to == "grid" {
        let mut grid_buckets: HashMap<[i64; 3], usize> = HashMap::new();
        for i in 0..npoints {
            if !in_group[i] {
                continue;
            }
            let key = [
                (p_slice[i * 3] as f64 / d + 0.5).floor() as i64,
                (p_slice[i * 3 + 1] as f64 / d + 0.5).floor() as i64,
                (p_slice[i * 3 + 2] as f64 / d + 0.5).floor() as i64,
            ];
            if let Some(&first) = grid_buckets.get(&key) {
                uf.union(first, i);
            } else {
                grid_buckets.insert(key, i);
            }
        }
    } else {
        // "near_points"
        let mut spatial_grid: HashMap<[i64; 3], Vec<usize>> = HashMap::new();
        for i in 0..npoints {
            if !in_group[i] {
                continue;
            }
            let key = [
                (p_slice[i * 3] as f64 / d).floor() as i64,
                (p_slice[i * 3 + 1] as f64 / d).floor() as i64,
                (p_slice[i * 3 + 2] as f64 / d).floor() as i64,
            ];
            spatial_grid.entry(key).or_default().push(i);
        }

        let dist_sq_thresh = d * d;

        for i in 0..npoints {
            if !in_group[i] {
                continue;
            }
            let pos = [
                p_slice[i * 3] as f64,
                p_slice[i * 3 + 1] as f64,
                p_slice[i * 3 + 2] as f64,
            ];
            let cell = [
                (pos[0] / d).floor() as i64,
                (pos[1] / d).floor() as i64,
                (pos[2] / d).floor() as i64,
            ];

            for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        let neighbor_cell = [cell[0] + dx, cell[1] + dy, cell[2] + dz];
                        if let Some(list) = spatial_grid.get(&neighbor_cell) {
                            for &j in list {
                                if j <= i {
                                    continue;
                                }
                                let jpos = [
                                    p_slice[j * 3] as f64,
                                    p_slice[j * 3 + 1] as f64,
                                    p_slice[j * 3 + 2] as f64,
                                ];
                                let dist_sq = (pos[0] - jpos[0]).powi(2)
                                    + (pos[1] - jpos[1]).powi(2)
                                    + (pos[2] - jpos[2]).powi(2);
                                if dist_sq <= dist_sq_thresh {
                                    uf.union(i, j);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Collect clusters
    let mut clusters: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in 0..npoints {
        let root = uf.find(i);
        clusters.entry(root).or_default().push(i);
    }

    // Compute cluster positions
    let mut cluster_pos: BTreeMap<usize, [f32; 3]> = BTreeMap::new();
    for (&root, pts) in &clusters {
        if output_pos == "average" {
            let mut sum = [0.0f64; 3];
            for &pt in pts {
                sum[0] += p_slice[pt * 3] as f64;
                sum[1] += p_slice[pt * 3 + 1] as f64;
                sum[2] += p_slice[pt * 3 + 2] as f64;
            }
            let count = pts.len() as f64;
            cluster_pos.insert(root, [
                (sum[0] / count) as f32,
                (sum[1] / count) as f32,
                (sum[2] / count) as f32,
            ]);
        } else {
            cluster_pos.insert(root, [
                p_slice[root * 3],
                p_slice[root * 3 + 1],
                p_slice[root * 3 + 2],
            ]);
        }
    }

    if !fuse_pts {
        // Snap positions only
        let mut new_p = p_slice.to_vec();
        let mut is_fused = vec![false; npoints];
        for (&root, pts) in &clusters {
            let pos = cluster_pos[&root];
            let fused = pts.len() > 1;
            for &pt in pts {
                new_p[pt * 3] = pos[0];
                new_p[pt * 3 + 1] = pos[1];
                new_p[pt * 3 + 2] = pos[2];
                is_fused[pt] = fused;
            }
        }
        let mut out = (*geo).clone();
        out.set_p(new_p);
        if outgrp_fused && !outgrp_fused_name.trim().is_empty() {
            model::write_group(&mut out, "point", outgrp_fused_name.trim(), &is_fused)?;
        }
        if outgrp_unique && !outgrp_unique_name.trim().is_empty() {
            let unique: Vec<bool> = is_fused.iter().map(|&f| !f).collect();
            model::write_group(&mut out, "point", outgrp_unique_name.trim(), &unique)?;
        }
        return Ok(out);
    }

    // Consolidate points
    let mut survivors: Vec<usize> = Vec::new();
    let mut remap = vec![0i32; npoints];
    let mut cluster_sizes = Vec::new();

    for (&root, pts) in &clusters {
        let new_idx = survivors.len();
        survivors.push(root);
        cluster_sizes.push(pts.len());
        for &pt in pts {
            remap[pt] = new_idx as i32;
        }
    }

    let new_npoints = survivors.len();
    let mut new_p = Vec::with_capacity(new_npoints * 3);
    for &root in &survivors {
        let pos = cluster_pos[&root];
        new_p.extend_from_slice(&pos);
    }

    let new_vp: Vec<i32> = geo
        .topology
        .vertex_point
        .iter()
        .map(|&v| remap[v as usize])
        .collect();

    let mut out = Geometry {
        npoints: new_npoints,
        topology: Arc::new(Topology {
            vertex_point: new_vp,
            prim_offsets: geo.topology.prim_offsets.clone(),
            closed: geo.topology.closed.clone(),
        ..Default::default()}),
        point: model::gather(&geo.point, &survivors),
        vertex: geo.vertex.clone(),
        prim: geo.prim.clone(),
        detail: geo.detail.clone(),
        edge: geo.edge.clone(),
        packed: geo.packed.clone(),
    };
    out.set_p(new_p);

    if remove_deg && out.nprims() > 0 {
        out = drop_degenerate(&out)?;
    }
    if remove_unused && out.nprims() > 0 {
        out = model::compact(out)?;
    }

    if outgrp_fused && !outgrp_fused_name.trim().is_empty() {
        let is_fused: Vec<bool> = cluster_sizes.iter().map(|&s| s > 1).collect();
        if is_fused.len() == out.npoints {
            model::write_group(&mut out, "point", outgrp_fused_name.trim(), &is_fused)?;
        }
    }
    if outgrp_unique && !outgrp_unique_name.trim().is_empty() {
        let unique: Vec<bool> = cluster_sizes.iter().map(|&s| s == 1).collect();
        if unique.len() == out.npoints {
            model::write_group(&mut out, "point", outgrp_unique_name.trim(), &unique)?;
        }
    }

    Ok(out)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
