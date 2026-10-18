//! Loop Curve: straighten interior points of an open chain onto the chord between endpoints.

use crate::{
    error::Result,
    geometry::Geometry,
    nodes::{Inputs, NodeSpec, GEO},
    params::Params,
};

pub(super) const NODE: NodeSpec = NodeSpec {
    name: "loop_curve",
    inputs: GEO,
    params: crate::params!(
        "group": "";
        "influence": 1.0;
    ),
    cook,
};

fn cook(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = NODE.one(inputs, 0)?;
    let influence = (p.f("influence")? as f32).clamp(0.0, 1.0);
    let group = p.s("group")?;

    if g.npoints == 0 || influence == 0.0 {
        return Ok((*g).clone());
    }

    let loops = crate::loop_topology::extract_loops(&g, &group)?;
    if loops.is_empty() {
        return Ok((*g).clone());
    }

    let p_slice = g.p();
    let mut new_p = p_slice.to_vec();

    for lp in &loops {
        // Curve operates on open chains with at least 3 points
        if lp.is_closed || lp.points.len() < 3 {
            continue;
        }

        let n = lp.points.len();
        let idx_0 = lp.points[0];
        let idx_end = lp.points[n - 1];

        let a = [
            p_slice[idx_0 * 3] as f64,
            p_slice[idx_0 * 3 + 1] as f64,
            p_slice[idx_0 * 3 + 2] as f64,
        ];
        let b = [
            p_slice[idx_end * 3] as f64,
            p_slice[idx_end * 3 + 1] as f64,
            p_slice[idx_end * 3 + 2] as f64,
        ];

        let chord = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let len_sq = chord[0] * chord[0] + chord[1] * chord[1] + chord[2] * chord[2];

        if len_sq < 1e-12 {
            for &idx in &lp.points[1..n - 1] {
                let orig = [
                    p_slice[idx * 3] as f64,
                    p_slice[idx * 3 + 1] as f64,
                    p_slice[idx * 3 + 2] as f64,
                ];
                let infl = influence as f64;
                for c in 0..3 {
                    new_p[idx * 3 + c] = (orig[c] * (1.0 - infl) + a[c] * infl) as f32;
                }
            }
            continue;
        }

        let infl = influence as f64;
        for &idx in &lp.points[1..n - 1] {
            let px = p_slice[idx * 3] as f64;
            let py = p_slice[idx * 3 + 1] as f64;
            let pz = p_slice[idx * 3 + 2] as f64;

            let dot = (px - a[0]) * chord[0] + (py - a[1]) * chord[1] + (pz - a[2]) * chord[2];
            let t = (dot / len_sq).clamp(0.0, 1.0);

            let qx = a[0] + t * chord[0];
            let qy = a[1] + t * chord[1];
            let qz = a[2] + t * chord[2];

            new_p[idx * 3] = (px * (1.0 - infl) + qx * infl) as f32;
            new_p[idx * 3 + 1] = (py * (1.0 - infl) + qy * infl) as f32;
            new_p[idx * 3 + 2] = (pz * (1.0 - infl) + qz * infl) as f32;
        }
    }

    let mut out = (*g).clone();
    out.set_p(new_p);
    Ok(out)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
