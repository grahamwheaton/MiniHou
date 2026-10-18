//! Boundary Select: selects the boundary of the input selection group.
use std::sync::Arc;

use crate::{
    error::{Error, Result},
    geometry::{Attr, Geometry},
    nodes::{InputSpec, Inputs, NodeSpec},
    params::Params,
};

pub(super) const NODE: NodeSpec = NodeSpec {
    name: "boundary_select",
    inputs: &[InputSpec { name: "geo", multi: false, optional: false }],
    params: crate::params!("domain": "faces"; "group": ""; "output_group_name": "boundary"),
    cook,
};

fn cook(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let source = inputs.first().and_then(|v| v.first()).ok_or_else(|| Error::new("geo is not connected"))?;
    let domain = p.e("domain", &["faces", "edges", "points", "primitives"])?;
    let group_pattern = p.s("group")?;
    let output_group_name = p.s("output_group_name")?;
    crate::nodes::group::valid_name(&output_group_name)?;

    let mut out = (**source).clone();
    let e = crate::topo::edges(&out.topology, out.npoints);
    let offs = &out.topology.prim_offsets;

    match domain.as_str() {
        "faces" | "primitives" => {
            let nprims = out.nprims();
            let face_mask = if group_pattern.trim().is_empty() {
                vec![true; nprims]
            } else {
                crate::nodes::group::mask(&out, "prim", &group_pattern)?
            };

            let mut edge_faces = vec![Vec::new(); e.len()];
            for prim in 0..nprims {
                if !out.topology.is_closed(prim) || (offs[prim + 1] - offs[prim]) < 3 {
                    continue;
                }
                let s = offs[prim] as usize;
                let end = offs[prim + 1] as usize;
                for v in s..end {
                    let edge_id = e.of_vertex[v];
                    if edge_id >= 0 {
                        edge_faces[edge_id as usize].push(prim);
                    }
                }
            }

            let edge_mask: Vec<bool> = edge_faces
                .iter()
                .map(|faces| {
                    let n_total = faces.len();
                    let n_sel = faces.iter().filter(|&&f| face_mask[f]).count();
                    n_sel > 0 && (n_sel < n_total || n_total == 1)
                })
                .collect();

            crate::nodes::edge_tools::write_values(
                &mut out,
                &format!("edge_group_{output_group_name}"),
                &edge_mask.iter().map(|&b| b as u8 as f32).collect::<Vec<_>>(),
                true,
            )?;
            out.edge.insert(
                format!("group_{output_group_name}"),
                Arc::new(Attr::i32(1, edge_mask.iter().map(|&b| b as i32).collect())),
            );
        }
        "edges" => {
            let edge_mask = if group_pattern.trim().is_empty() {
                vec![true; e.len()]
            } else {
                crate::nodes::edge_tools::edge_mask(&out, &group_pattern)?
            };

            let mut pt_degree = vec![0usize; out.npoints];
            for (i, &sel) in edge_mask.iter().enumerate() {
                if sel {
                    pt_degree[e.points[2 * i] as usize] += 1;
                    pt_degree[e.points[2 * i + 1] as usize] += 1;
                }
            }

            let boundary_edges: Vec<bool> = edge_mask
                .iter()
                .enumerate()
                .map(|(i, &sel)| {
                    sel && (pt_degree[e.points[2 * i] as usize] == 1 || pt_degree[e.points[2 * i + 1] as usize] == 1)
                })
                .collect();

            crate::nodes::edge_tools::write_values(
                &mut out,
                &format!("edge_group_{output_group_name}"),
                &boundary_edges.iter().map(|&b| b as u8 as f32).collect::<Vec<_>>(),
                true,
            )?;
            out.edge.insert(
                format!("group_{output_group_name}"),
                Arc::new(Attr::i32(1, boundary_edges.iter().map(|&b| b as i32).collect())),
            );
            let pt_mask: Vec<i32> = pt_degree.into_iter().map(|d| (d == 1) as i32).collect();
            out.point.insert(format!("group_{output_group_name}"), Arc::new(Attr::i32(1, pt_mask)));
        }
        "points" => {
            let pt_mask = if group_pattern.trim().is_empty() {
                vec![true; out.npoints]
            } else {
                crate::nodes::groups::point_mask(&out, &group_pattern)?
            };

            let mut edge_faces_cnt = vec![0usize; e.len()];
            for prim in 0..out.nprims() {
                if !out.topology.is_closed(prim) || (offs[prim + 1] - offs[prim]) < 3 {
                    continue;
                }
                let s = offs[prim] as usize;
                let end = offs[prim + 1] as usize;
                for v in s..end {
                    let edge_id = e.of_vertex[v];
                    if edge_id >= 0 {
                        edge_faces_cnt[edge_id as usize] += 1;
                    }
                }
            }

            let mut pt_boundary = vec![false; out.npoints];
            for i in 0..e.len() {
                let a = e.points[2 * i] as usize;
                let b = e.points[2 * i + 1] as usize;
                if pt_mask[a] != pt_mask[b] {
                    if pt_mask[a] { pt_boundary[a] = true; }
                    if pt_mask[b] { pt_boundary[b] = true; }
                } else if pt_mask[a] && pt_mask[b] && edge_faces_cnt[i] <= 1 {
                    pt_boundary[a] = true;
                    pt_boundary[b] = true;
                }
            }

            out.point.insert(
                format!("group_{output_group_name}"),
                Arc::new(Attr::i32(1, pt_boundary.into_iter().map(|b| b as i32).collect())),
            );
        }
        _ => unreachable!(),
    }

    Ok(out)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
