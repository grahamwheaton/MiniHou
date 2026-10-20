//! Ripple: concentric sinusoidal displacement from a centre.
use crate::{
    error::{Error, Result},
    geometry::Geometry,
    nodes::{GEO, Inputs, NodeSpec},
    params::Params,
};

pub(super) const NODE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "ripple",
    inputs: GEO,
    params: crate::params!(
        "center": [0.0, 0.0, 0.0];
        "decay": 0.0;
        "frequency": 1.0;
        "offset": 0.0;
        "strength": 1.0;
    ),
    cook,
};

fn point_normals(geo: &Geometry) -> Vec<f32> {
    if let Some(attr) = geo.point.get("N") {
        if let Some(n) = attr.as_f32() {
            if n.len() == geo.npoints * 3 {
                return n.to_vec();
            }
        }
    }
    if !geo.topology.prim_offsets.is_empty() {
        let n = crate::normals::point_normals(geo.p(), &geo.topology, geo.npoints);
        if n.len() == geo.npoints * 3 {
            return n;
        }
    }
    let mut n = Vec::with_capacity(geo.npoints * 3);
    for _ in 0..geo.npoints {
        n.extend_from_slice(&[0.0, 1.0, 0.0]);
    }
    n
}

fn cook(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let source = inputs[0].first().ok_or_else(|| Error::new("geo is not connected"))?;
    let strength = p.f("strength")? as f32;
    if strength == 0.0 {
        return Ok((**source).clone());
    }
    let center = p.v3("center")?.map(|v| v as f32);
    let decay = p.f("decay")? as f32;
    let frequency = p.f("frequency")? as f32;
    let offset = p.f("offset")? as f32;
    let mut out = (**source).clone();
    let normals = point_normals(&out);
    let mut points = out.p().to_vec();
    for (xyz, n) in points.chunks_exact_mut(3).zip(normals.chunks_exact(3)) {
        let dx = xyz[0] - center[0];
        let dy = xyz[1] - center[1];
        let dz = xyz[2] - center[2];
        let dist = (dx * dx + dy * dy + dz * dz).sqrt();
        let factor = if decay > 0.0 { (-decay * dist).exp() } else { 1.0 };
        let val = (dist * frequency + offset).sin() * factor;
        let n_len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        let (nx, ny, nz) = if n_len > 1e-6 {
            (n[0] / n_len, n[1] / n_len, n[2] / n_len)
        } else {
            (0.0, 1.0, 0.0)
        };
        let disp = strength * val;
        xyz[0] += nx * disp;
        xyz[1] += ny * disp;
        xyz[2] += nz * disp;
    }
    out.set_p(points);
    Ok(out)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
