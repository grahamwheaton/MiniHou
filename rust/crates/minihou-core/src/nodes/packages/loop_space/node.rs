//! Loop Space: resample each selected loop to uniform arc-length spacing.

use crate::{
    error::Result,
    geometry::Geometry,
    nodes::{Inputs, NodeSpec, GEO},
    params::Params,
};

pub(super) const NODE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "loop_space",
    inputs: GEO,
    params: crate::params!(
        "group": "";
        "influence": 1.0;
        "interpolation": "linear";
    ),
    cook,
};

fn dist3(a: [f32; 3], b: [f32; 3]) -> f32 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    let dz = a[2] - b[2];
    (dx * dx + dy * dy + dz * dz).sqrt()
}

fn cook(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = NODE.one(inputs, 0)?;
    let influence = (p.f("influence")? as f32).clamp(0.0, 1.0);
    let _interpolation = p.e("interpolation", &["linear"])?;
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
        let n = lp.points.len();
        if n < 2 || (lp.is_closed && n < 3) {
            continue;
        }

        // Collect point coordinates
        let pts: Vec<[f32; 3]> = lp
            .points
            .iter()
            .map(|&idx| [p_slice[idx * 3], p_slice[idx * 3 + 1], p_slice[idx * 3 + 2]])
            .collect();

        if !lp.is_closed {
            // Open path: n vertices, n - 1 segments
            let mut cum_len = vec![0.0f32; n];
            for i in 0..n - 1 {
                cum_len[i + 1] = cum_len[i] + dist3(pts[i], pts[i + 1]);
            }
            let total_len = cum_len[n - 1];
            if total_len < 1e-12 {
                continue;
            }

            for k in 0..n {
                let target_d = (k as f32 / (n - 1) as f32) * total_len;
                let q = if k == 0 {
                    pts[0]
                } else if k == n - 1 {
                    pts[n - 1]
                } else {
                    // Find segment
                    let mut seg = 0;
                    while seg + 1 < n - 1 && cum_len[seg + 1] < target_d {
                        seg += 1;
                    }
                    let seg_len = cum_len[seg + 1] - cum_len[seg];
                    let u = if seg_len > 1e-12 {
                        (target_d - cum_len[seg]) / seg_len
                    } else {
                        0.0
                    };
                    [
                        pts[seg][0] * (1.0 - u) + pts[seg + 1][0] * u,
                        pts[seg][1] * (1.0 - u) + pts[seg + 1][1] * u,
                        pts[seg][2] * (1.0 - u) + pts[seg + 1][2] * u,
                    ]
                };

                let pt_idx = lp.points[k];
                for c in 0..3 {
                    new_p[pt_idx * 3 + c] = pts[k][c] * (1.0 - influence) + q[c] * influence;
                }
            }
        } else {
            // Closed loop: n vertices, n segments
            let mut cum_len = vec![0.0f32; n + 1];
            for i in 0..n {
                let next = (i + 1) % n;
                cum_len[i + 1] = cum_len[i] + dist3(pts[i], pts[next]);
            }
            let total_len = cum_len[n];
            if total_len < 1e-12 {
                continue;
            }

            for k in 0..n {
                let target_d = (k as f32 / n as f32) * total_len;
                let q = if k == 0 {
                    pts[0]
                } else {
                    let mut seg = 0;
                    while seg + 1 < n && cum_len[seg + 1] < target_d {
                        seg += 1;
                    }
                    let seg_len = cum_len[seg + 1] - cum_len[seg];
                    let u = if seg_len > 1e-12 {
                        (target_d - cum_len[seg]) / seg_len
                    } else {
                        0.0
                    };
                    let next_seg = (seg + 1) % n;
                    [
                        pts[seg][0] * (1.0 - u) + pts[next_seg][0] * u,
                        pts[seg][1] * (1.0 - u) + pts[next_seg][1] * u,
                        pts[seg][2] * (1.0 - u) + pts[next_seg][2] * u,
                    ]
                };

                let pt_idx = lp.points[k];
                for c in 0..3 {
                    new_p[pt_idx * 3 + c] = pts[k][c] * (1.0 - influence) + q[c] * influence;
                }
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
