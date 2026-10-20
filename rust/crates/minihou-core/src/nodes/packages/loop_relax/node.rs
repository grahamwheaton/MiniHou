//! Loop Relax: iterative 1D Laplacian smoothing along each loop.

use crate::{
    error::Result,
    geometry::Geometry,
    nodes::{Inputs, NodeSpec, GEO},
    params::Params,
};

pub(super) const NODE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "loop_relax",
    inputs: GEO,
    params: crate::params!(
        "group": "";
        "influence": 1.0;
        "iterations": 3;
    ),
    cook,
};

fn cook(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = NODE.one(inputs, 0)?;
    let influence = (p.f("influence")? as f32).clamp(0.0, 1.0);
    let iterations = p.i("iterations")?.max(0) as usize;
    let group = p.s("group")?;

    if g.npoints == 0 || influence == 0.0 || iterations == 0 {
        return Ok((*g).clone());
    }

    let loops = crate::loop_topology::extract_loops(&g, &group)?;
    if loops.is_empty() {
        return Ok((*g).clone());
    }

    let p_slice = g.p();
    let mut new_p = p_slice.to_vec();

    for lp in &loops {
        let n = lp.points.len();
        if n < 3 {
            continue;
        }

        let mut current_pts: Vec<[f32; 3]> = lp
            .points
            .iter()
            .map(|&idx| [p_slice[idx * 3], p_slice[idx * 3 + 1], p_slice[idx * 3 + 2]])
            .collect();

        if !lp.is_closed {
            // Open path: endpoints 0 and n - 1 are fixed
            for _ in 0..iterations {
                let mut next_pts = current_pts.clone();
                for i in 1..n - 1 {
                    next_pts[i] = [
                        0.5 * (current_pts[i - 1][0] + current_pts[i + 1][0]),
                        0.5 * (current_pts[i - 1][1] + current_pts[i + 1][1]),
                        0.5 * (current_pts[i - 1][2] + current_pts[i + 1][2]),
                    ];
                }
                current_pts = next_pts;
            }
        } else {
            // Closed loop: all vertices smoothed using cyclic neighbors
            for _ in 0..iterations {
                let mut next_pts = current_pts.clone();
                for i in 0..n {
                    let prev = if i == 0 { n - 1 } else { i - 1 };
                    let next = if i + 1 == n { 0 } else { i + 1 };
                    next_pts[i] = [
                        0.5 * (current_pts[prev][0] + current_pts[next][0]),
                        0.5 * (current_pts[prev][1] + current_pts[next][1]),
                        0.5 * (current_pts[prev][2] + current_pts[next][2]),
                    ];
                }
                current_pts = next_pts;
            }
        }

        for (k, &idx) in lp.points.iter().enumerate() {
            let orig = [p_slice[idx * 3], p_slice[idx * 3 + 1], p_slice[idx * 3 + 2]];
            let target = current_pts[k];
            for c in 0..3 {
                new_p[idx * 3 + c] = orig[c] * (1.0 - influence) + target[c] * influence;
            }
        }
    }

    let mut out = (*g).clone();
    out.set_p(new_p);
    Ok(out)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
