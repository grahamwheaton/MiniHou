//! Select Sharp Edges: selects edges whose dihedral angle is above a threshold.
use std::sync::Arc;

use crate::{
    error::{Error, Result},
    geometry::{Attr, Geometry},
    nodes::{InputSpec, Inputs, NodeSpec},
    params::Params,
};

pub(super) const NODE: NodeSpec = NodeSpec {
    name: "select_sharp_edges",
    inputs: &[InputSpec { name: "geo", multi: false, optional: false }],
    params: crate::params!("angle": 30.0; "output_group_name": "sharp_edges"; "domain": "edges"; "group": ""),
    cook,
};

fn cook(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let source = inputs.first().and_then(|v| v.first()).ok_or_else(|| Error::new("geo is not connected"))?;
    p.e("domain", &["edges"])?;
    let angle = p.f("angle")?;
    if !angle.is_finite() || angle < 0.0 {
        return Err(Error::new("angle must be a non-negative finite number"));
    }
    let output_group_name = p.s("output_group_name")?;
    let output_group_name = if output_group_name.trim().is_empty() { "sharp_edges" } else { output_group_name.trim() };
    crate::nodes::group::valid_name(output_group_name)?;
    let group_pattern = p.s("group").unwrap_or_default();

    let mut out = (**source).clone();
    let filter_mask = if group_pattern.trim().is_empty() {
        None
    } else {
        Some(crate::nodes::edge_tools::edge_mask(&out, &group_pattern)?)
    };

    let fn_ = crate::normals::prim_normals(out.p(), &out.topology);
    let un: Vec<[f64; 3]> = fn_
        .iter()
        .map(|n| {
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            if len < 1e-20 {
                [0.0, 0.0, 0.0]
            } else {
                [n[0] / len, n[1] / len, n[2] / len]
            }
        })
        .collect();

    let e = crate::topo::edges(&out.topology, out.npoints);
    let offs = &out.topology.prim_offsets;
    let mut edge_faces = vec![Vec::new(); e.len()];
    for prim in 0..out.nprims() {
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

    let mut sharp_mask = vec![false; e.len()];
    for (i, faces) in edge_faces.iter().enumerate() {
        if filter_mask.as_ref().is_some_and(|m| !m[i]) {
            continue;
        }
        if faces.len() < 2 {
            continue;
        }
        if faces.len() == 2 {
            let n1 = un[faces[0]];
            let n2 = un[faces[1]];
            let dot = (n1[0] * n2[0] + n1[1] * n2[1] + n1[2] * n2[2]).clamp(-1.0, 1.0);
            let deg = dot.acos().to_degrees();
            if deg > angle {
                sharp_mask[i] = true;
            }
        } else {
            for a in 0..faces.len() {
                for b in (a + 1)..faces.len() {
                    let n1 = un[faces[a]];
                    let n2 = un[faces[b]];
                    let dot = (n1[0] * n2[0] + n1[1] * n2[1] + n1[2] * n2[2]).clamp(-1.0, 1.0);
                    let deg = dot.acos().to_degrees();
                    if deg > angle {
                        sharp_mask[i] = true;
                        break;
                    }
                }
                if sharp_mask[i] {
                    break;
                }
            }
        }
    }

    crate::nodes::edge_tools::write_values(
        &mut out,
        &format!("edge_group_{output_group_name}"),
        &sharp_mask.iter().map(|&b| b as u8 as f32).collect::<Vec<_>>(),
        true,
    )?;
    if output_group_name != "sharp" {
        let _ = crate::nodes::edge_tools::write_values(
            &mut out,
            "edge_group_sharp",
            &sharp_mask.iter().map(|&b| b as u8 as f32).collect::<Vec<_>>(),
            true,
        );
    }
    out.edge.insert(
        format!("group_{output_group_name}"),
        Arc::new(Attr::i32(1, sharp_mask.iter().map(|&b| b as i32).collect())),
    );

    Ok(out)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
