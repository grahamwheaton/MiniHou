//! Normal-projected uniform one-ring Laplacian, scaled by mean squared edge length.
use std::sync::Arc;

use crate::error::{Error, Result};
use crate::geometry::{Attr, Geometry};
use crate::nodes::{Inputs, NodeSpec, GEO};
use crate::params::Params;

pub(super) const NODE: NodeSpec = NodeSpec {
    outputs: &["geometry"],
    cook_multi: None,
    cook_sidecar: None,
    name: "curvature",
    inputs: GEO,
    params: crate::params!("name": "curvature"; "mode": "signed"),
    cook,
};

fn cook(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = NODE.one(inputs, 0)?;
    let name = p.s("name")?;
    crate::nodes::group::valid_name(&name)?;
    if name == "P" {
        return Err(Error::new("curvature cannot overwrite P"));
    }
    let mode = p.e("mode", &["signed", "convex", "concave"])?;
    let positions = geo.p();
    let normals = crate::normals::point_normals(positions, &geo.topology, geo.npoints);
    let mut neighbours = vec![Vec::new(); geo.npoints];
    for (a, b) in geo.topology.edge_pairs() {
        if a != b {
            neighbours[a as usize].push(b as usize);
            neighbours[b as usize].push(a as usize);
        }
    }
    let values = neighbours
        .iter()
        .enumerate()
        .map(|(i, ring)| {
            if ring.is_empty() {
                return 0.;
            }
            let inv = 1. / ring.len() as f64;
            let mut laplacian = [0.; 3];
            let mut mean_squared_length = 0.;
            for &j in ring {
                for k in 0..3 {
                    let displacement = positions[3 * i + k] as f64 - positions[3 * j + k] as f64;
                    laplacian[k] += displacement * inv;
                    mean_squared_length += displacement * displacement * inv;
                }
            }
            if mean_squared_length <= 1e-24 {
                return 0.;
            }
            let mut normal = [
                normals[3 * i] as f64,
                normals[3 * i + 1] as f64,
                normals[3 * i + 2] as f64,
            ];
            if normal.iter().all(|x| x.abs() < 1e-20) {
                normal = [0., 0., 1.];
            }
            let signed = 2.
                * laplacian
                    .iter()
                    .zip(normal)
                    .map(|(a, b)| a * b)
                    .sum::<f64>()
                / mean_squared_length;
            match mode.as_str() {
                "convex" => signed.max(0.) as f32,
                "concave" => (-signed).max(0.) as f32,
                _ => signed as f32,
            }
        })
        .collect();
    let mut out = (*geo).clone();
    out.point.insert(name, Arc::new(Attr::f32(1, values)));
    Ok(out)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
