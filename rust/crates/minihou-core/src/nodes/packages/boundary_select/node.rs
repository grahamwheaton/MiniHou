//! Boundary Select: selects the boundary of the input selection group.
use std::sync::Arc;

use crate::{
    error::{Error, Result},
    geometry::{Attr, Geometry},
    nodes::{InputSpec, Inputs, NodeSpec},
    params::Params,
};

pub(super) const NODE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "boundary_select",
    inputs: &[InputSpec { name: "geo", multi: false, optional: false }],
    params: crate::params!("domain": "edges"; "group": ""; "output_group_name": "boundary"),
    cook,
};

fn cook(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let source = inputs.first().and_then(|v| v.first()).ok_or_else(|| Error::new("geo is not connected"))?;
    let domain = p.e("domain", &["edges", "faces", "points", "primitives"])?;
    let group_pattern = p.s("group")?;
    let output_group_name = p.s("output_group_name").unwrap_or_else(|_| "boundary".into());
    let output_group_name = if output_group_name.trim().is_empty() { "boundary" } else { output_group_name.trim() };
    crate::nodes::group::valid_name(output_group_name)?;

    let mut out = (**source).clone();
    let e = crate::topo::edges(&out.topology, out.npoints);
    let offs = &out.topology.prim_offsets;
    let nprims = out.nprims();

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

    let edge_mask: Vec<bool> = match domain.as_str() {
        "faces" | "primitives" => {
            let face_mask = if group_pattern.trim().is_empty() {
                vec![true; nprims]
            } else {
                crate::nodes::group::mask(&out, "prim", &group_pattern)?
            };

            edge_faces
                .iter()
                .map(|faces| {
                    let n_total = faces.len();
                    let n_sel = faces.iter().filter(|&&f| face_mask[f]).count();
                    n_sel > 0 && (n_sel < n_total || n_total == 1)
                })
                .collect()
        }
        "edges" => {
            let sel_edges = if group_pattern.trim().is_empty() {
                vec![true; e.len()]
            } else {
                crate::nodes::edge_tools::edge_mask(&out, &group_pattern)?
            };

            let mut pt_degree = vec![0usize; out.npoints];
            for (i, &sel) in sel_edges.iter().enumerate() {
                if sel {
                    pt_degree[e.points[2 * i] as usize] += 1;
                    pt_degree[e.points[2 * i + 1] as usize] += 1;
                }
            }

            sel_edges
                .iter()
                .enumerate()
                .map(|(i, &sel)| {
                    if !sel {
                        return false;
                    }
                    let is_mesh_boundary = edge_faces[i].len() <= 1;
                    let is_endpoint = pt_degree[e.points[2 * i] as usize] == 1 || pt_degree[e.points[2 * i + 1] as usize] == 1;
                    is_mesh_boundary || is_endpoint
                })
                .collect()
        }
        "points" => {
            let pt_mask = if group_pattern.trim().is_empty() {
                vec![true; out.npoints]
            } else {
                crate::nodes::groups::point_mask(&out, &group_pattern)?
            };

            (0..e.len())
                .map(|i| {
                    let a = e.points[2 * i] as usize;
                    let b = e.points[2 * i + 1] as usize;
                    let in_a = pt_mask[a];
                    let in_b = pt_mask[b];
                    (in_a && in_b && edge_faces[i].len() <= 1) || (in_a != in_b && (in_a || in_b))
                })
                .collect()
        }
        _ => unreachable!(),
    };

    let edge_attr = Arc::new(Attr::i32(1, edge_mask.iter().map(|&b| b as i32).collect()));
    let vertex_values: Vec<f32> = edge_mask.iter().map(|&b| b as u8 as f32).collect();
    store_group(&mut out, output_group_name, &edge_attr, &vertex_values)?;

    Ok(out)
}

fn store_group(out: &mut Geometry, name: &str, edge_attr: &Arc<Attr>, values: &[f32]) -> Result<()> {
    for key in [format!("group_{name}"), format!("edge_group_{name}")] {
        crate::nodes::edge_tools::write_values(out, &key, values, true)?;
        out.edge.insert(key, edge_attr.clone());
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
