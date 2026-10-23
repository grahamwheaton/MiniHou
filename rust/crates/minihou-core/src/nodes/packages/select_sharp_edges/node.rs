//! Select Sharp Edges: selects edges whose dihedral angle is above a threshold.
use std::sync::Arc;

use crate::{
    error::{Error, Result},
    geometry::{Attr, Geometry},
    nodes::{InputSpec, Inputs, NodeSpec},
    params::Params,
};

pub(super) const NODE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
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
    // Faces meeting along an edge are found by welded position, so meshes with split points (one point set per face) still
    // see their neighbours; a cube with unshared points otherwise has one face per edge and tags nothing.
    let pos = out.p();
    let key = |pt: i64| {
        let i = pt as usize * 3;
        [0, 1, 2].map(|k| (pos[i + k] + 0.0).to_bits())
    };
    let mut by_key: std::collections::HashMap<([u32; 3], [u32; 3]), Vec<usize>> = std::collections::HashMap::new();
    let ekey = |i: usize| {
        let (a, b) = (key(e.points[i * 2]), key(e.points[i * 2 + 1]));
        if a <= b { (a, b) } else { (b, a) }
    };
    for prim in 0..out.nprims() {
        if !out.topology.is_closed(prim) || (offs[prim + 1] - offs[prim]) < 3 {
            continue;
        }
        for v in offs[prim] as usize..offs[prim + 1] as usize {
            let edge_id = e.of_vertex[v];
            if edge_id >= 0 {
                let list = by_key.entry(ekey(edge_id as usize)).or_default();
                if !list.contains(&prim) {
                    list.push(prim);
                }
            }
        }
    }
    let edge_faces: Vec<Vec<usize>> = (0..e.len()).map(|i| by_key.get(&ekey(i)).cloned().unwrap_or_default()).collect();

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
