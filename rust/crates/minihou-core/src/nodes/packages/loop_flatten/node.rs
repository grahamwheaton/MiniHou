//! Loop Flatten: project the selected points onto a plane (best-fit or average normal).

use crate::{
    error::Result,
    geometry::Geometry,
    nodes::{Inputs, NodeSpec, GEO},
    params::Params,
};

pub(super) const NODE: NodeSpec = NodeSpec {
    name: "loop_flatten",
    inputs: GEO,
    params: crate::params!(
        "group": "";
        "influence": 1.0;
        "plane": "best_fit";
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

fn cook(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = NODE.one(inputs, 0)?;
    let influence = (p.f("influence")? as f32).clamp(0.0, 1.0);
    let plane_mode = p.e("plane", &["best_fit", "normal"])?;
    let group = p.s("group")?;

    if g.npoints == 0 || influence == 0.0 {
        return Ok((*g).clone());
    }

    let trimmed = group.trim();
    let sel: Vec<bool> = if trimmed.is_empty() {
        vec![true; g.npoints]
    } else if g.vertex.contains_key(&format!("edge_group_{trimmed}")) || g.vertex.contains_key(trimmed) {
        let e = crate::topo::edges(&g.topology, g.npoints);
        let name = if g.vertex.contains_key(&format!("edge_group_{trimmed}")) {
            format!("edge_group_{trimmed}")
        } else {
            trimmed.to_string()
        };
        let mut pt_sel = vec![false; g.npoints];
        if let Some(attr) = g.vertex.get(&name) {
            for v in 0..g.nvertices() {
                let is_nonzero = match &attr.data {
                    crate::geometry::AttrData::F32(vec) => vec.get(v * attr.width).is_some_and(|&val| val != 0.0),
                    crate::geometry::AttrData::I32(vec) => vec.get(v * attr.width).is_some_and(|&val| val != 0),
                };
                if is_nonzero {
                    let edge_id = e.of_vertex[v];
                    if edge_id >= 0 && (edge_id as usize) < e.len() {
                        let u = e.points[2 * edge_id as usize] as usize;
                        let w = e.points[2 * edge_id as usize + 1] as usize;
                        if u < g.npoints { pt_sel[u] = true; }
                        if w < g.npoints { pt_sel[w] = true; }
                    }
                }
            }
        }
        pt_sel
    } else {
        crate::nodes::group::mask(&g, "point", trimmed).unwrap_or_else(|_| vec![false; g.npoints])
    };

    let selected_indices: Vec<usize> = sel
        .iter()
        .enumerate()
        .filter_map(|(i, &b)| if b { Some(i) } else { None })
        .collect();

    if selected_indices.len() < 3 {
        return Ok((*g).clone());
    }

    let p_slice = g.p();
    let count = selected_indices.len() as f64;

    // Centroid
    let mut center = [0.0f64; 3];
    for &idx in &selected_indices {
        center[0] += p_slice[idx * 3] as f64;
        center[1] += p_slice[idx * 3 + 1] as f64;
        center[2] += p_slice[idx * 3 + 2] as f64;
    }
    center[0] /= count;
    center[1] /= count;
    center[2] /= count;

    // Plane normal
    let normal: [f64; 3] = match plane_mode.as_str() {
        "normal" => {
            let has_n = g.point.get("N").filter(|a| a.width == 3).and_then(|a| a.as_f32()).is_some();
            let normals = if has_n {
                g.point["N"].as_f32().unwrap().to_vec()
            } else {
                crate::normals::point_normals(g.p(), &g.topology, g.npoints)
            };

            let mut sum_n = [0.0f64; 3];
            for &idx in &selected_indices {
                sum_n[0] += normals[idx * 3] as f64;
                sum_n[1] += normals[idx * 3 + 1] as f64;
                sum_n[2] += normals[idx * 3 + 2] as f64;
            }
            let len = (sum_n[0] * sum_n[0] + sum_n[1] * sum_n[1] + sum_n[2] * sum_n[2]).sqrt();
            if len > 1e-12 {
                [sum_n[0] / len, sum_n[1] / len, sum_n[2] / len]
            } else {
                [0.0, 0.0, 1.0]
            }
        }
        _ => {
            // "best_fit" from PCA covariance matrix
            let mut cov = [[0.0f64; 3]; 3];
            for &idx in &selected_indices {
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
            jacobi_smallest_eigenvector(cov)
        }
    };

    let mut new_p = p_slice.to_vec();
    let infl = influence as f64;
    for &idx in &selected_indices {
        let px = p_slice[idx * 3] as f64;
        let py = p_slice[idx * 3 + 1] as f64;
        let pz = p_slice[idx * 3 + 2] as f64;

        let dist_to_plane = (px - center[0]) * normal[0]
            + (py - center[1]) * normal[1]
            + (pz - center[2]) * normal[2];

        let proj_x = px - dist_to_plane * normal[0];
        let proj_y = py - dist_to_plane * normal[1];
        let proj_z = pz - dist_to_plane * normal[2];

        new_p[idx * 3] = (px * (1.0 - infl) + proj_x * infl) as f32;
        new_p[idx * 3 + 1] = (py * (1.0 - infl) + proj_y * infl) as f32;
        new_p[idx * 3 + 2] = (pz * (1.0 - infl) + proj_z * infl) as f32;
    }

    let mut out = (*g).clone();
    out.set_p(new_p);
    Ok(out)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
