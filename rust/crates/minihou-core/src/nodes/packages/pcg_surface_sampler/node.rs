use crate::nodes::pcg::*;
use crate::nodes::{InputSpec, Inputs, NodeSpec};
use crate::{error::{Error, Result}, geometry::{Attr, AttrData, Geometry}, params::Params};
use std::collections::HashMap;
use std::sync::Arc;

pub(super) const NODE: NodeSpec = NodeSpec {
    name: "pcg_surface_sampler",
    inputs: &[InputSpec {
        name: "surface",
        multi: false,
        optional: false,
    }],
    params: crate::params!(
        "points_per_m2": 1.0;
        "extents": [0.5, 0.5, 0.5];
        "looseness": 0.3;
        "align": false;
        "seed": 0
    ),
    cook: pcg_surface_sampler,
};

fn pairs_within(p: &[[f64; 3]], r: f64) -> (Vec<usize>, Vec<usize>) {
    let n = p.len();
    if n < 2 || r <= 0.0 {
        return (Vec::new(), Vec::new());
    }
    let r2 = r * r;
    let mut lo = p[0];
    for pt in p.iter().skip(1) {
        lo[0] = lo[0].min(pt[0]);
        lo[1] = lo[1].min(pt[1]);
        lo[2] = lo[2].min(pt[2]);
    }
    let inv_r = 1.0 / r;
    let mut cells: HashMap<(i64, i64, i64), Vec<usize>> = HashMap::new();
    for (i, pt) in p.iter().enumerate() {
        let cx = ((pt[0] - lo[0]) * inv_r).floor() as i64;
        let cy = ((pt[1] - lo[1]) * inv_r).floor() as i64;
        let cz = ((pt[2] - lo[2]) * inv_r).floor() as i64;
        cells.entry((cx, cy, cz)).or_default().push(i);
    }
    let mut i_pairs = Vec::new();
    let mut j_pairs = Vec::new();
    let forward_offsets: [(i64, i64, i64); 13] = [
        (0, 0, 1),
        (0, 1, -1),
        (0, 1, 0),
        (0, 1, 1),
        (1, -1, -1),
        (1, -1, 0),
        (1, -1, 1),
        (1, 0, -1),
        (1, 0, 0),
        (1, 0, 1),
        (1, 1, -1),
        (1, 1, 0),
        (1, 1, 1),
    ];
    for (&(cx, cy, cz), pts) in &cells {
        for a in 0..pts.len() {
            let i = pts[a];
            for b in (a + 1)..pts.len() {
                let j = pts[b];
                let (p1, p2) = (p[i], p[j]);
                let dx = p1[0] - p2[0];
                let dy = p1[1] - p2[1];
                let dz = p1[2] - p2[2];
                if dx * dx + dy * dy + dz * dz < r2 {
                    i_pairs.push(i);
                    j_pairs.push(j);
                }
            }
        }
        for &(dx, dy, dz) in &forward_offsets {
            if let Some(other_pts) = cells.get(&(cx + dx, cy + dy, cz + dz)) {
                for &i in pts {
                    for &j in other_pts {
                        let (p1, p2) = (p[i], p[j]);
                        let ddx = p1[0] - p2[0];
                        let ddy = p1[1] - p2[1];
                        let ddz = p1[2] - p2[2];
                        if ddx * ddx + ddy * ddy + ddz * ddz < r2 {
                            i_pairs.push(i);
                            j_pairs.push(j);
                        }
                    }
                }
            }
        }
    }
    (i_pairs, j_pairs)
}

fn pcg_surface_sampler(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let surface = NODE.one(inputs, 0)?;
    let points_per_m2 = p.f("points_per_m2")?;
    let extents = p.v3("extents")?;
    let looseness = p.f("looseness")?;
    let align = p.b("align")?;
    let seed = p.i("seed")?;

    let topo = &surface.topology;
    let p_slice = surface.p();
    let nprims = surface.nprims();
    let vp = &topo.vertex_point;
    let offs = &topo.prim_offsets;

    let get_p = |pt: usize| -> [f64; 3] {
        [
            p_slice[pt * 3] as f64,
            p_slice[pt * 3 + 1] as f64,
            p_slice[pt * 3 + 2] as f64,
        ]
    };

    let mut area = 0.0f64;
    for prim in 0..nprims {
        let start = offs[prim] as usize;
        let end = offs[prim + 1] as usize;
        let k = end.saturating_sub(start);
        if k < 3 {
            continue;
        }
        let v0 = vp[start] as usize;
        let a = get_p(v0);
        for j in 0..(k - 2) {
            let v1 = vp[start + j + 1] as usize;
            let v2 = vp[start + j + 2] as usize;
            let b = get_p(v1);
            let c = get_p(v2);
            let cr = cross(
                [b[0] - a[0], b[1] - a[1], b[2] - a[2]],
                [c[0] - a[0], c[1] - a[1], c[2] - a[2]],
            );
            area += 0.5 * dot(cr, cr).sqrt();
        }
    }

    let target = (area * points_per_m2).round() as i64;
    if target <= 0 {
        return new_points_cloud(Vec::new(), None, extents, Some(Vec::new()), None);
    }
    let target = target as usize;
    let over = if looseness >= 1.0 { 1 } else { 6 };
    let pts = crate::nodes::scatter::scatter_surface(&surface, target * over, seed as i64)?;

    let mut x: Vec<[f64; 3]> = pts
        .p()
        .chunks_exact(3)
        .map(|c| [c[0] as f64, c[1] as f64, c[2] as f64])
        .collect();
    let mut n_vec: Vec<[f64; 3]> = if let Some(a) = pts.point.get("N") {
        if let AttrData::F32(v) = &a.data {
            v.chunks_exact(3)
                .map(|c| [c[0] as f64, c[1] as f64, c[2] as f64])
                .collect()
        } else {
            vec![[0.0, 0.0, 1.0]; pts.npoints]
        }
    } else {
        vec![[0.0, 0.0, 1.0]; pts.npoints]
    };

    if over > 1 {
        let r = (1.0 - looseness) * 0.75 / points_per_m2.sqrt();
        let (i_pairs, j_pairs) = if r > 0.0 {
            pairs_within(&x, r)
        } else {
            (Vec::new(), Vec::new())
        };
        let pri: Vec<f64> = (0..x.len()).map(|i| random01(i as i32, seed, 1)).collect();
        let keep = greedy_independent(x.len(), &i_pairs, &j_pairs, &pri);
        let mut idx: Vec<usize> = (0..x.len()).filter(|&i| keep[i]).collect();
        idx.sort_by(|&a, &b| {
            pri[b]
                .partial_cmp(&pri[a])
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        idx.truncate(target);
        x = idx.iter().map(|&i| x[i]).collect();
        n_vec = idx.iter().map(|&i| n_vec[i]).collect();
    }

    let rot = if align {
        let mut r = Vec::with_capacity(n_vec.len() * 3);
        for &norm_v in &n_vec {
            let u = normalize(norm_v);
            let rot_mat = rotation_from_z_to(u);
            let euler = matrix_to_euler_xyz(&rot_mat);
            r.extend([euler[0] as f32, euler[1] as f32, euler[2] as f32]);
        }
        Some(r)
    } else {
        None
    };

    let p_flat: Vec<f32> = x
        .iter()
        .flat_map(|v| [v[0] as f32, v[1] as f32, v[2] as f32])
        .collect();
    let n_flat: Vec<f32> = n_vec
        .iter()
        .flat_map(|v| [v[0] as f32, v[1] as f32, v[2] as f32])
        .collect();
    new_points_cloud(p_flat, rot, extents, Some(n_flat), None)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
