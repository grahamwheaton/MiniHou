//! `fuse` (`minihou/nodes/primitives.py`): point welding and snapping to target geometry.

use std::collections::BTreeMap;
use std::sync::Arc;

use super::{model, InputSpec, Inputs, NodeSpec};
use crate::error::Result;
use crate::geometry::{Attr, Geometry, Topology};
use crate::params::Params;

pub const FUSE: NodeSpec = NodeSpec {
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
        "outgrp_fused": false;
        "outgrp_fused_name": "fusedpoints";
        "outgrp_unique": false;
        "outgrp_unique_name": "uniquepoints";
    ),
    cook: fuse,
};

pub(super) fn fuse_points(
    geo: &Geometry,
    distance: f64,
    counts_out: Option<&mut Vec<usize>>,
) -> Result<Geometry> {
    if geo.npoints == 0 {
        return Ok(geo.clone());
    }
    let d = distance.max(1e-12);
    let p_slice = geo.p();
    let mut seen: BTreeMap<[i64; 3], usize> = BTreeMap::new();
    let mut keep_idx = Vec::new();
    let mut counts = Vec::new();
    let mut remap = Vec::with_capacity(geo.npoints);

    for i in 0..geo.npoints {
        let k = [
            (p_slice[i * 3] as f64 / d + 0.5).floor() as i64,
            (p_slice[i * 3 + 1] as f64 / d + 0.5).floor() as i64,
            (p_slice[i * 3 + 2] as f64 / d + 0.5).floor() as i64,
        ];
        if let Some(&new_id) = seen.get(&k) {
            counts[new_id] += 1;
            remap.push(new_id as i32);
        } else {
            let new_id = keep_idx.len();
            seen.insert(k, new_id);
            keep_idx.push(i);
            counts.push(1);
            remap.push(new_id as i32);
        }
    }

    let new_npoints = keep_idx.len();
    let new_vp: Vec<i32> = geo
        .topology
        .vertex_point
        .iter()
        .map(|&v| remap[v as usize])
        .collect();

    let mut out = Geometry { packed: Default::default(),
        npoints: new_npoints,
        topology: Arc::new(Topology {
            vertex_point: new_vp,
            prim_offsets: geo.topology.prim_offsets.clone(),
            closed: geo.topology.closed.clone(),
        }),
        point: model::gather(&geo.point, &keep_idx),
        vertex: geo.vertex.clone(),
        prim: geo.prim.clone(),
        detail: geo.detail.clone(),
    };
    out.validate()?;

    if let Some(c_out) = counts_out {
        *c_out = counts;
    }

    Ok(out)
}

fn fuse(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = FUSE.one(inputs, 0)?;
    let targets = inputs.get(1).and_then(|t| t.first());
    let distance = p.f("distance")?;

    if let Some(tgt) = targets {
        let mut out = (*geo).clone();
        let hit = if geo.npoints > 0 && tgt.npoints > 0 {
            let src_p = tgt.p();
            let mut hit_mask = vec![false; geo.npoints];
            let mut positions = geo.p().to_vec();
            let d_thresh = distance.max(1e-12);
            let d_thresh_sq = d_thresh * d_thresh;

            for i in 0..geo.npoints {
                let q = [
                    positions[i * 3] as f64,
                    positions[i * 3 + 1] as f64,
                    positions[i * 3 + 2] as f64,
                ];
                let mut best_dist_sq = f64::INFINITY;
                let mut best_idx = 0;
                for j in 0..tgt.npoints {
                    let tp = [
                        src_p[j * 3] as f64,
                        src_p[j * 3 + 1] as f64,
                        src_p[j * 3 + 2] as f64,
                    ];
                    let dsq = (q[0] - tp[0]) * (q[0] - tp[0])
                        + (q[1] - tp[1]) * (q[1] - tp[1])
                        + (q[2] - tp[2]) * (q[2] - tp[2]);
                    if dsq < best_dist_sq {
                        best_dist_sq = dsq;
                        best_idx = j;
                    }
                }
                if best_dist_sq <= d_thresh_sq {
                    hit_mask[i] = true;
                    positions[i * 3] = src_p[best_idx * 3];
                    positions[i * 3 + 1] = src_p[best_idx * 3 + 1];
                    positions[i * 3 + 2] = src_p[best_idx * 3 + 2];
                }
            }
            out.set_p(positions);
            hit_mask
        } else {
            vec![false; geo.npoints]
        };

        model::outgroup(&mut out, p, "fused", "point", &hit)?;
        let unique: Vec<bool> = hit.iter().map(|&h| !h).collect();
        model::outgroup(&mut out, p, "unique", "point", &unique)?;
        return Ok(out);
    }

    let mut counts = Vec::new();
    let mut out = fuse_points(&geo, distance, Some(&mut counts))?;
    let fused_mask: Vec<bool> = counts.iter().map(|&c| c > 1).collect();
    let unique_mask: Vec<bool> = counts.iter().map(|&c| c == 1).collect();
    model::outgroup(&mut out, p, "fused", "point", &fused_mask)?;
    model::outgroup(&mut out, p, "unique", "point", &unique_mask)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuse_duplicate_points() {
        let g = Geometry::points_only(vec![
            0.0, 0.0, 0.0,
            0.0001, 0.0, 0.0,
            1.0, 0.0, 0.0,
        ])
        .unwrap();
        let p = Params::resolve(
            FUSE.params,
            serde_json::json!({"distance": 0.01}).as_object(),
        )
        .unwrap();
        let out = fuse(&[vec![Arc::new(g)], vec![]], &p).unwrap();
        assert_eq!(out.npoints, 2);
    }

    #[test]
    fn fuse_outgroups() {
        let g = Geometry::points_only(vec![
            0.0, 0.0, 0.0,
            0.0, 0.0, 0.0,
            1.0, 0.0, 0.0,
        ])
        .unwrap();
        let p = Params::resolve(
            FUSE.params,
            serde_json::json!({
                "distance": 0.01,
                "outgrp_fused": true,
                "outgrp_unique": true,
            })
            .as_object(),
        )
        .unwrap();
        let out = fuse(&[vec![Arc::new(g)], vec![]], &p).unwrap();
        assert_eq!(out.npoints, 2);
        assert_eq!(
            out.point.get("group_fusedpoints").unwrap().data,
            crate::geometry::AttrData::I32(vec![1, 0])
        );
        assert_eq!(
            out.point.get("group_uniquepoints").unwrap().data,
            crate::geometry::AttrData::I32(vec![0, 1])
        );
    }
}
