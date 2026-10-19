//! Loop Circle: fit a planar circle to a selected loop and place its points at equal angle.

use crate::{
    error::Result,
    geometry::Geometry,
    nodes::{Inputs, NodeSpec, GEO},
    params::Params,
};

pub(super) const NODE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None,
    name: "loop_circle",
    inputs: GEO,
    params: crate::params!(
        "fit_inside": false;
        "group": "";
        "influence": 1.0;
    ),
    cook,
};

fn jacobi_smallest_eigenvector(m: [[f64; 3]; 3]) -> [f64; 3] {
    let mut a = m;
    let mut v = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

    for _ in 0..50 {
        let mut max_val = 0.0f64;
        let mut p = 0;
        let mut q = 1;
        for i in 0..3 {
            for j in (i + 1)..3 {
                let val = a[i][j].abs();
                if val > max_val {
                    max_val = val;
                    p = i;
                    q = j;
                }
            }
        }

        if max_val < 1e-15 {
            break;
        }

        let app = a[p][p];
        let aqq = a[q][q];
        let apq = a[p][q];

        let theta = (aqq - app) / (2.0 * apq);
        let t = if theta >= 0.0 {
            1.0 / (theta + (theta * theta + 1.0).sqrt())
        } else {
            -1.0 / (-theta + (theta * theta + 1.0).sqrt())
        };
        let c = 1.0 / (t * t + 1.0).sqrt();
        let s = t * c;
        let tau = s / (1.0 + c);

        a[p][p] = app - t * apq;
        a[q][q] = aqq + t * apq;
        a[p][q] = 0.0;
        a[q][p] = 0.0;

        for r in 0..3 {
            if r != p && r != q {
                let arp = a[r][p];
                let arq = a[r][q];
                a[r][p] = arp - s * (arq + tau * arp);
                a[p][r] = a[r][p];
                a[r][q] = arq + s * (arp - tau * arq);
                a[q][r] = a[r][q];
            }
        }

        for r in 0..3 {
            let vrp = v[r][p];
            let vrq = v[r][q];
            v[r][p] = vrp - s * (vrq + tau * vrp);
            v[r][q] = vrq + s * (vrp - tau * vrq);
        }
    }

    let mut min_idx = 0;
    let mut min_eig = a[0][0];
    for i in 1..3 {
        if a[i][i] < min_eig {
            min_eig = a[i][i];
            min_idx = i;
        }
    }

    let mut n = [v[0][min_idx], v[1][min_idx], v[2][min_idx]];
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if len > 1e-12 {
        n[0] /= len;
        n[1] /= len;
        n[2] /= len;
    } else {
        n = [0.0, 0.0, 1.0];
    }
    n
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn norm(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}

fn normalize(a: [f64; 3]) -> [f64; 3] {
    let l = norm(a);
    if l > 1e-12 {
        [a[0] / l, a[1] / l, a[2] / l]
    } else {
        [0.0, 0.0, 0.0]
    }
}

fn cook(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = NODE.one(inputs, 0)?;
    let influence = (p.f("influence")? as f32).clamp(0.0, 1.0);
    let fit_inside = p.b("fit_inside")?;
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
        if n < 3 {
            continue;
        }

        // 1. Compute centroid
        let mut center = [0.0f64; 3];
        for &idx in &lp.points {
            center[0] += p_slice[idx * 3] as f64;
            center[1] += p_slice[idx * 3 + 1] as f64;
            center[2] += p_slice[idx * 3 + 2] as f64;
        }
        let fn_len = n as f64;
        center[0] /= fn_len;
        center[1] /= fn_len;
        center[2] /= fn_len;

        // 2. Compute covariance matrix and PCA plane normal
        let mut cov = [[0.0f64; 3]; 3];
        for &idx in &lp.points {
            let dx = p_slice[idx * 3] as f64 - center[0];
            let dy = p_slice[idx * 3 + 1] as f64 - center[1];
            let dz = p_slice[idx * 3 + 2] as f64 - center[2];
            let d = [dx, dy, dz];
            for i in 0..3 {
                for j in 0..3 {
                    cov[i][j] += d[i] * d[j];
                }
            }
        }
        let mut normal = jacobi_smallest_eigenvector(cov);

        // 3. Align normal with loop winding
        let mut area_sum = [0.0f64; 3];
        let seg_count = if lp.is_closed { n } else { n - 1 };
        for i in 0..seg_count {
            let idx_a = lp.points[i];
            let idx_b = lp.points[(i + 1) % n];
            let pa = [
                p_slice[idx_a * 3] as f64 - center[0],
                p_slice[idx_a * 3 + 1] as f64 - center[1],
                p_slice[idx_a * 3 + 2] as f64 - center[2],
            ];
            let pb = [
                p_slice[idx_b * 3] as f64 - center[0],
                p_slice[idx_b * 3 + 1] as f64 - center[1],
                p_slice[idx_b * 3 + 2] as f64 - center[2],
            ];
            let c = cross(pa, pb);
            area_sum[0] += c[0];
            area_sum[1] += c[1];
            area_sum[2] += c[2];
        }
        if dot(area_sum, normal) < 0.0 {
            normal = [-normal[0], -normal[1], -normal[2]];
        }

        // 4. Project points onto plane and measure radial distances
        let mut proj = Vec::with_capacity(n);
        let mut dists = Vec::with_capacity(n);
        for &idx in &lp.points {
            let px = p_slice[idx * 3] as f64;
            let py = p_slice[idx * 3 + 1] as f64;
            let pz = p_slice[idx * 3 + 2] as f64;

            let dist_to_plane = (px - center[0]) * normal[0]
                + (py - center[1]) * normal[1]
                + (pz - center[2]) * normal[2];

            let vx = px - dist_to_plane * normal[0] - center[0];
            let vy = py - dist_to_plane * normal[1] - center[1];
            let vz = pz - dist_to_plane * normal[2] - center[2];
            let r = (vx * vx + vy * vy + vz * vz).sqrt();
            proj.push([vx, vy, vz]);
            dists.push(r);
        }

        let radius = if fit_inside {
            dists.iter().copied().fold(f64::INFINITY, f64::min)
        } else {
            dists.iter().sum::<f64>() / fn_len
        };

        if radius < 1e-12 {
            continue;
        }

        // 5. Build orthonormal basis (u, v) on the circle plane
        let mut u = [0.0f64; 3];
        for v_cand in &proj {
            let len = norm(*v_cand);
            if len > 1e-12 {
                u = [v_cand[0] / len, v_cand[1] / len, v_cand[2] / len];
                break;
            }
        }
        if norm(u) < 1e-12 {
            let perp = if normal[0].abs() < 0.9 {
                [1.0, 0.0, 0.0]
            } else {
                [0.0, 1.0, 0.0]
            };
            u = normalize(cross(normal, perp));
        }
        let v = normalize(cross(normal, u));

        // 6. Compute target circular positions
        let total_angle = if lp.is_closed {
            std::f64::consts::TAU
        } else {
            let last_v = proj[n - 1];
            let mut phi = dot(last_v, v).atan2(dot(last_v, u));
            if phi < 0.0 {
                phi += std::f64::consts::TAU;
            }
            phi
        };

        let step = if lp.is_closed {
            total_angle / fn_len
        } else {
            total_angle / (n - 1) as f64
        };

        for (k, &idx) in lp.points.iter().enumerate() {
            let angle = k as f64 * step;
            let qx = center[0] + radius * (angle.cos() * u[0] + angle.sin() * v[0]);
            let qy = center[1] + radius * (angle.cos() * u[1] + angle.sin() * v[1]);
            let qz = center[2] + radius * (angle.cos() * u[2] + angle.sin() * v[2]);

            let orig_x = p_slice[idx * 3] as f64;
            let orig_y = p_slice[idx * 3 + 1] as f64;
            let orig_z = p_slice[idx * 3 + 2] as f64;

            let infl = influence as f64;
            new_p[idx * 3] = (orig_x * (1.0 - infl) + qx * infl) as f32;
            new_p[idx * 3 + 1] = (orig_y * (1.0 - infl) + qy * infl) as f32;
            new_p[idx * 3 + 2] = (orig_z * (1.0 - infl) + qz * infl) as f32;
        }
    }

    let mut out = (*g).clone();
    out.set_p(new_p);
    Ok(out)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
