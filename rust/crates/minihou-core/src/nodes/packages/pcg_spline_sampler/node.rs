use crate::nodes::pcg::*;
use crate::nodes::{InputSpec, Inputs, NodeSpec};
use crate::{error::{Error, Result}, geometry::{Attr, AttrData, Geometry}, params::Params};
use std::collections::HashMap;
use std::sync::Arc;

pub(super) const NODE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None,
    name: "pcg_spline_sampler",
    inputs: &[InputSpec {
        name: "curve",
        multi: false,
        optional: false,
    }],
    params: crate::params!(
        "mode": "on_curve";
        "spacing": 1.0;
        "extents": [0.5, 0.5, 0.5];
        "jitter": 0.0;
        "seed": 0
    ),
    cook: pcg_spline_sampler,
};

fn pcg_spline_sampler(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let curve = NODE.one(inputs, 0)?;
    let mode = p.e("mode", &["on_curve", "interior"])?;
    let spacing = p.f("spacing")?;
    let extents = p.v3("extents")?;
    let jitter = p.f("jitter")?;
    let seed = p.i("seed")?;

    if curve.nprims() == 0 {
        return new_points_cloud(Vec::new(), None, extents, None, None);
    }

    if mode == "on_curve" {
        let cp = curve.p();
        let vp = &curve.topology.vertex_point;
        let mut out_x = Vec::new();
        let mut s_lists: Vec<Vec<[f64; 3]>> = Vec::new();

        for i in 0..curve.nprims() {
            let (start, end) = (
                curve.topology.prim_offsets[i] as usize,
                curve.topology.prim_offsets[i + 1] as usize,
            );
            if end - start < 2 {
                continue;
            }
            let closed = curve.topology.is_closed(i);
            let mut seg_lengths = Vec::new();
            let mut cum_s = vec![0.0];
            let mut total_l = 0.0;
            let count = if closed { end - start } else { end - start - 1 };
            for j in 0..count {
                let v0 = vp[start + j] as usize;
                let v1 = vp[start + (j + 1) % (end - start)] as usize;
                let p0 = [cp[v0 * 3] as f64, cp[v0 * 3 + 1] as f64, cp[v0 * 3 + 2] as f64];
                let p1 = [cp[v1 * 3] as f64, cp[v1 * 3 + 1] as f64, cp[v1 * 3 + 2] as f64];
                let l = norm(sub(p1, p0));
                seg_lengths.push(l);
                total_l += l;
                cum_s.push(total_l);
            }

            let mut targets = Vec::new();
            if total_l <= 0.0 {
                targets.push(0.0);
            } else {
                let mut s = 0.0;
                while s <= total_l + 1e-9 {
                    targets.push(s);
                    s += spacing;
                }
                if closed && targets.len() > 1 && (targets.last().unwrap() - total_l).abs() < 1e-9 {
                    targets.pop();
                }
            }

            let mut prim_pts = Vec::new();
            for &t_s in &targets {
                let s_clamped = t_s.clamp(0.0, total_l);
                let seg_idx = match cum_s.binary_search_by(|v| {
                    v.partial_cmp(&s_clamped).unwrap_or(std::cmp::Ordering::Equal)
                }) {
                    Ok(idx) => idx.min(seg_lengths.len().saturating_sub(1)),
                    Err(idx) => (idx - 1).min(seg_lengths.len().saturating_sub(1)),
                };
                let frac = if seg_lengths[seg_idx] > 1e-12 {
                    (s_clamped - cum_s[seg_idx]) / seg_lengths[seg_idx]
                } else {
                    0.0
                };
                let v0 = vp[start + seg_idx] as usize;
                let v1 = vp[start + (seg_idx + 1) % (end - start)] as usize;
                let p0 = [cp[v0 * 3] as f64, cp[v0 * 3 + 1] as f64, cp[v0 * 3 + 2] as f64];
                let p1 = [cp[v1 * 3] as f64, cp[v1 * 3 + 1] as f64, cp[v1 * 3 + 2] as f64];
                let pt = [
                    p0[0] + frac * (p1[0] - p0[0]),
                    p0[1] + frac * (p1[1] - p0[1]),
                    p0[2] + frac * (p1[2] - p0[2]),
                ];
                prim_pts.push(pt);
                out_x.push(pt);
            }
            s_lists.push(prim_pts);
        }

        let mut tg = vec![[1.0, 0.0, 0.0]; out_x.len()];
        let mut off = 0;
        for prim_pts in s_lists {
            let m = prim_pts.len();
            if m >= 2 {
                for k in 0..m {
                    let grad = if k == 0 {
                        sub(prim_pts[1], prim_pts[0])
                    } else if k == m - 1 {
                        sub(prim_pts[m - 1], prim_pts[m - 2])
                    } else {
                        let d = sub(prim_pts[k + 1], prim_pts[k - 1]);
                        [d[0] * 0.5, d[1] * 0.5, d[2] * 0.5]
                    };
                    let ln = norm(grad);
                    tg[off + k] = if ln > 0.0 {
                        normalize(grad)
                    } else {
                        [1.0, 0.0, 0.0]
                    };
                }
            }
            off += m;
        }

        let mut rot = Vec::with_capacity(out_x.len() * 3);
        let mut p_out = Vec::with_capacity(out_x.len() * 3);
        for i in 0..out_x.len() {
            let yaw = tg[i][1].atan2(tg[i][0]);
            let pitch = -(tg[i][2].clamp(-1.0, 1.0)).asin();
            rot.extend_from_slice(&[0.0f32, pitch as f32, yaw as f32]);
            p_out.extend_from_slice(&[out_x[i][0] as f32, out_x[i][1] as f32, out_x[i][2] as f32]);
        }
        return new_points_cloud(p_out, Some(rot), extents, None, None);
    }

    // mode == "interior"
    let cp = curve.p();
    let vp = &curve.topology.vertex_point;
    let mut all_out = Vec::new();

    for k_prim in 0..curve.nprims() {
        let (start, end) = (
            curve.topology.prim_offsets[k_prim] as usize,
            curve.topology.prim_offsets[k_prim + 1] as usize,
        );
        let m = end - start;
        if m < 3 {
            continue;
        }
        let mut q: Vec<[f64; 3]> = Vec::with_capacity(m);
        for v in &vp[start..end] {
            let idx = *v as usize;
            q.push([cp[idx * 3] as f64, cp[idx * 3 + 1] as f64, cp[idx * 3 + 2] as f64]);
        }
        let mut c = [0.0; 3];
        for pt in &q {
            c[0] += pt[0];
            c[1] += pt[1];
            c[2] += pt[2];
        }
        c = [c[0] / m as f64, c[1] / m as f64, c[2] / m as f64];

        let mut nrm = [0.0; 3];
        for i in 0..m {
            let next_i = (i + 1) % m;
            let d1 = sub(q[i], c);
            let d2 = sub(q[next_i], c);
            let cr = cross(d1, d2);
            nrm[0] += cr[0];
            nrm[1] += cr[1];
            nrm[2] += cr[2];
        }
        if norm(nrm) < 1e-12 {
            continue;
        }
        nrm = normalize(nrm);
        let mut u = cross(
            nrm,
            if nrm[0].abs() < 0.9 {
                [1.0, 0.0, 0.0]
            } else {
                [0.0, 1.0, 0.0]
            },
        );
        u = normalize(u);
        let mut w = cross(nrm, u);
        if nrm[2].abs() > 0.9 {
            u = [1.0, 0.0, 0.0];
            w = [0.0, nrm[2].signum(), 0.0];
        }

        let mut xy = Vec::with_capacity(m);
        let mut lo = [f64::INFINITY, f64::INFINITY];
        let mut hi = [f64::NEG_INFINITY, f64::NEG_INFINITY];
        for pt in &q {
            let diff = sub(*pt, c);
            let px = dot(diff, u);
            let py = dot(diff, w);
            lo[0] = lo[0].min(px);
            lo[1] = lo[1].min(py);
            hi[0] = hi[0].max(px);
            hi[1] = hi[1].max(py);
            xy.push([px, py]);
        }

        let mut gx = Vec::new();
        let mut x_val = lo[0] + spacing / 2.0;
        while x_val < hi[0] {
            gx.push(x_val);
            x_val += spacing;
        }
        let mut gy = Vec::new();
        let mut y_val = lo[1] + spacing / 2.0;
        while y_val < hi[1] {
            gy.push(y_val);
            y_val += spacing;
        }

        let mut grid_pts = Vec::new();
        for &px in &gx {
            for &py in &gy {
                grid_pts.push([px, py]);
            }
        }

        if jitter > 0.0 && !grid_pts.is_empty() {
            for g in &mut grid_pts {
                let seed_pos = [g[0] as f32, g[1] as f32, k_prim as f32];
                let s = position_seeds(&seed_pos)[0];
                let rx = random01(s, seed, 0);
                let ry = random01(s, seed, 1);
                g[0] += (rx - 0.5) * spacing * jitter;
                g[1] += (ry - 0.5) * spacing * jitter;
            }
        }

        for g in grid_pts {
            let mut inside = false;
            for e in 0..m {
                let prev_e = if e == 0 { m - 1 } else { e - 1 };
                let (a, b) = (xy[prev_e], xy[e]);
                let cross_edge = (a[1] > g[1]) != (b[1] > g[1]);
                let denom = if b[1] != a[1] { b[1] - a[1] } else { 1.0 };
                let x_hit = a[0] + (g[1] - a[1]) * (b[0] - a[0]) / denom;
                if cross_edge && (g[0] < x_hit) {
                    inside = !inside;
                }
            }
            if inside {
                all_out.push([
                    c[0] + g[0] * u[0] + g[1] * w[0],
                    c[1] + g[0] * u[1] + g[1] * w[1],
                    c[2] + g[0] * u[2] + g[1] * w[2],
                ]);
            }
        }
    }

    let mut p_out = Vec::with_capacity(all_out.len() * 3);
    for pt in &all_out {
        p_out.extend_from_slice(&[pt[0] as f32, pt[1] as f32, pt[2] as f32]);
    }
    new_points_cloud(p_out, None, extents, None, None)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
