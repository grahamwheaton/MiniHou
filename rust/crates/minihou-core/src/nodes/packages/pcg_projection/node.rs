use crate::nodes::pcg::*;
use crate::nodes::{InputSpec, Inputs, NodeSpec};
use crate::{error::{Error, Result}, geometry::{Attr, AttrData, Geometry}, params::Params};
use std::collections::HashMap;
use std::sync::Arc;

pub(super) const NODE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "pcg_projection",
    inputs: &[
        InputSpec {
            name: "points",
            multi: false,
            optional: false,
        },
        InputSpec {
            name: "surface",
            multi: false,
            optional: false,
        },
    ],
    params: crate::params!(
        "direction": [0.0, 0.0, -1.0];
        "align": true;
        "keep_misses": false
    ),
    cook: pcg_projection,
};

fn pcg_projection(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let points = NODE.one(inputs, 0)?;
    let surface = NODE.one(inputs, 1)?;
    let direction = p.v3("direction")?;
    let align = p.b("align")?;
    let keep_misses = p.b("keep_misses")?;

    let a = read_pcg(&points);
    let n = points.npoints;
    let tris = fan_triangles(&surface);

    let d_len = norm(direction);
    let d = if d_len < 1e-12 {
        [0.0, 0.0, -1.0]
    } else {
        [
            direction[0] / d_len,
            direction[1] / d_len,
            direction[2] / d_len,
        ]
    };
    let neg_d = [-d[0], -d[1], -d[2]];

    let mut hit = vec![false; n];
    let mut hit_p = a.p.clone();
    let mut hit_tri = vec![0usize; n];

    for i in 0..n {
        let orig = a.p[i];
        let mut t1 = f64::INFINITY;
        let mut tri1 = 0;
        let mut t2 = f64::INFINITY;
        let mut tri2 = 0;

        for (t_idx, tri) in tris.iter().enumerate() {
            if let Some(t) = ray_tri_hit(orig, d, tri[0], tri[1], tri[2]) {
                if t < t1 {
                    t1 = t;
                    tri1 = t_idx;
                }
            }
            if let Some(t) = ray_tri_hit(orig, neg_d, tri[0], tri[1], tri[2]) {
                if t < t2 {
                    t2 = t;
                    tri2 = t_idx;
                }
            }
        }

        let use_back = !t1.is_finite() || (t2.is_finite() && t2 < t1);
        let chosen_t = if use_back { -t2 } else { t1 };
        let chosen_tri = if use_back { tri2 } else { tri1 };

        if chosen_t.is_finite() {
            hit[i] = true;
            hit_p[i] = [
                orig[0] + d[0] * chosen_t,
                orig[1] + d[1] * chosen_t,
                orig[2] + d[2] * chosen_t,
            ];
            hit_tri[i] = chosen_tri;
        }
    }

    let mut new_p = Vec::with_capacity(n * 3);
    for pt in &hit_p {
        new_p.extend_from_slice(&[pt[0] as f32, pt[1] as f32, pt[2] as f32]);
    }

    let mut attrs = HashMap::new();
    attrs.insert("P".into(), NewAttrVal::F32(3, new_p));

    if align && hit.iter().any(|&h| h) {
        let mut new_rot = Vec::with_capacity(n * 3);
        let mut new_norm = Vec::with_capacity(n * 3);
        for i in 0..n {
            if hit[i] {
                let tri = tris[hit_tri[i]];
                let mut nrm = cross(sub(tri[1], tri[0]), sub(tri[2], tri[0]));
                nrm = normalize(nrm);
                if dot(nrm, neg_d) < 0.0 {
                    nrm = [-nrm[0], -nrm[1], -nrm[2]];
                }
                let r_align = rotation_from_z_to(nrm);
                let r_z = euler_xyz_to_matrix([0.0, 0.0, a.rot[i][2]]);
                let r_final = mat_mul(&r_align, &r_z);
                let euler = matrix_to_euler_xyz(&r_final);
                new_rot.extend_from_slice(&[euler[0] as f32, euler[1] as f32, euler[2] as f32]);
                new_norm.extend_from_slice(&[nrm[0] as f32, nrm[1] as f32, nrm[2] as f32]);
            } else {
                new_rot.extend_from_slice(&[
                    a.rot[i][0] as f32,
                    a.rot[i][1] as f32,
                    a.rot[i][2] as f32,
                ]);
                new_norm.extend_from_slice(&[0.0, 0.0, 1.0]);
            }
        }
        attrs.insert("rot".into(), NewAttrVal::F32(3, new_rot));
        attrs.insert("N".into(), NewAttrVal::F32(3, new_norm));
    }

    if keep_misses {
        cloud(&points, None, Some(attrs))
    } else {
        cloud(&points, Some(&hit), Some(attrs))
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
