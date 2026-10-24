//! Deterministic hemisphere ray sampling against the input mesh.
use std::sync::Arc;

use crate::error::{Error, Result};
use crate::geometry::{Attr, Geometry};
use crate::nodes::{ray::RayMesh, Inputs, NodeSpec, GEO};
use crate::params::Params;

pub(super) const NODE: NodeSpec = NodeSpec {
    outputs: &["geometry"],
    cook_multi: None,
    cook_sidecar: None,
    name: "ambient_occlusion",
    inputs: GEO,
    params: crate::params!("domain": "points"; "name": "ao"; "samples": 32; "max_distance": 10.0),
    cook,
};

type V = [f64; 3];
fn dot(a: V, b: V) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V, b: V) -> V {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn unit(a: V) -> V {
    let len = dot(a, a).sqrt();
    if len.is_finite() && len > 1e-20 {
        a.map(|v| v / len)
    } else {
        [0., 0., 1.]
    }
}
fn point(p: &[f32], i: usize) -> V {
    [p[3 * i] as f64, p[3 * i + 1] as f64, p[3 * i + 2] as f64]
}

fn cook(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = NODE.one(inputs, 0)?;
    let domain = p.e("domain", &["points", "faces"])?;
    let name = p.s("name")?;
    crate::nodes::group::valid_name(&name)?;
    if name == "P" {
        return Err(Error::new("ambient occlusion cannot overwrite P"));
    }
    let samples = p.i("samples")?;
    if !(1..=4096).contains(&samples) {
        return Err(Error::new("samples must be between 1 and 4096"));
    }
    let count = if domain == "points" {
        geo.npoints
    } else {
        geo.nprims()
    };
    if count.saturating_mul(samples as usize) > 20_000_000 {
        return Err(Error::new(
            "ambient occlusion ray count exceeds safety limit",
        ));
    }
    let max_distance = p.f("max_distance")?;
    if !max_distance.is_finite() || max_distance <= 0. {
        return Err(Error::new("max_distance must be positive and finite"));
    }

    let positions = geo.p();
    let (centres, normals): (Vec<V>, Vec<V>) = if domain == "points" {
        let authored = geo
            .point
            .get("N")
            .filter(|a| a.width == 3)
            .and_then(|a| a.as_f32());
        (
            (0..geo.npoints).map(|i| point(positions, i)).collect(),
            (0..geo.npoints)
                .map(|i| authored.map_or([0., 0., 1.], |n| unit(point(n, i))))
                .collect(),
        )
    } else {
        let face_normals = crate::normals::prim_normals(positions, &geo.topology);
        let mut centres = Vec::with_capacity(geo.nprims());
        for prim in 0..geo.nprims() {
            let range = geo.topology.prim_offsets[prim] as usize
                ..geo.topology.prim_offsets[prim + 1] as usize;
            let mut centre = [0.; 3];
            let n = range.len();
            for v in range {
                let q = point(positions, geo.topology.vertex_point[v] as usize);
                for k in 0..3 {
                    centre[k] += q[k];
                }
            }
            if n > 0 {
                for c in &mut centre {
                    *c /= n as f64;
                }
            }
            centres.push(centre);
        }
        (centres, face_normals.into_iter().map(unit).collect())
    };
    let mesh = RayMesh::new(&geo);
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    for i in 0..geo.npoints {
        let q = point(positions, i);
        for k in 0..3 {
            min[k] = min[k].min(q[k]);
            max[k] = max[k].max(q[k]);
        }
    }
    let diagonal = if geo.npoints == 0 {
        1.
    } else {
        dot(
            [max[0] - min[0], max[1] - min[1], max[2] - min[2]],
            [max[0] - min[0], max[1] - min[1], max[2] - min[2]],
        )
        .sqrt()
    };
    let bias = (diagonal * 1e-6).max(1e-8);
    let golden_angle = std::f64::consts::PI * (3. - 5_f64.sqrt());
    let values = centres
        .iter()
        .zip(normals.iter())
        .enumerate()
        .map(|(i, (&centre, &normal))| {
            let reference = if normal[0].abs() < 0.9 {
                [1., 0., 0.]
            } else {
                [0., 1., 0.]
            };
            let tangent = unit(cross(normal, reference));
            let bitangent = cross(normal, tangent);
            // Fixed per-element phase prevents a regular ray pattern while remaining reproducible.
            let phase = ((i as u64).wrapping_mul(0x9e3779b97f4a7c15) >> 32) as f64
                * (2. * std::f64::consts::PI / 4294967296.);
            let mut unoccluded = 0;
            for j in 0..samples as usize {
                let z = (j as f64 + 0.5) / samples as f64;
                let radius = (1. - z * z).sqrt();
                let angle = golden_angle * j as f64 + phase;
                let direction: V = std::array::from_fn(|k| {
                    normal[k] * z + radius * (tangent[k] * angle.cos() + bitangent[k] * angle.sin())
                });
                let origin: V =
                    std::array::from_fn(|k| centre[k] + normal[k] * bias + direction[k] * bias);
                if mesh
                    .first_hit_distance(origin, direction, max_distance)
                    .is_none()
                {
                    unoccluded += 1;
                }
            }
            unoccluded as f32 / samples as f32
        })
        .collect();
    let mut out = (*geo).clone();
    let table = if domain == "points" {
        &mut out.point
    } else {
        &mut out.prim
    };
    table.insert(name, Arc::new(Attr::f32(1, values)));
    Ok(out)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
