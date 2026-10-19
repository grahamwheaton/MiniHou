use crate::nodes::pcg::{cloud, euler_xyz_to_matrix, mat_mul, matrix_to_euler_xyz, random01, read_pcg, NewAttrVal};
use std::collections::HashMap;
use crate::nodes::{InputSpec, Inputs, NodeSpec};
use crate::{error::Result, geometry::Geometry, params::Params};

pub(super) const NODE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None,
    name: "pcg_transform",
    inputs: &[InputSpec {
        name: "points",
        multi: false,
        optional: false,
    }],
    params: crate::params!(
        "offset_min": [0.0, 0.0, 0.0];
        "offset_max": [0.0, 0.0, 0.0];
        "rotation_min": [0.0, 0.0, 0.0];
        "rotation_max": [0.0, 0.0, 360.0];
        "scale_min": [1.0, 1.0, 1.0];
        "scale_max": [1.0, 1.0, 1.0];
        "uniform_scale": true;
        "absolute_rotation": false;
        "seed": 0
    ),
    cook: pcg_transform,
};

fn pcg_transform(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let points = NODE.one(inputs, 0)?;
    let off_min = p.v3("offset_min")?;
    let off_max = p.v3("offset_max")?;
    let rot_min = p.v3("rotation_min")?;
    let rot_max = p.v3("rotation_max")?;
    let sc_min = p.v3("scale_min")?;
    let sc_max = p.v3("scale_max")?;
    let uniform_scale = p.b("uniform_scale")?;
    let absolute_rotation = p.b("absolute_rotation")?;
    let seed = p.i("seed")?;

    let a = read_pcg(&points);
    let n = points.npoints;

    let mut new_p = Vec::with_capacity(n * 3);
    let mut new_rot = Vec::with_capacity(n * 3);
    let mut new_scale = Vec::with_capacity(n * 3);

    for i in 0..n {
        let pt_seed = a.seed[i];
        let r: [f64; 9] = std::array::from_fn(|k| random01(pt_seed, seed, k));
        let off = [
            off_min[0] + (off_max[0] - off_min[0]) * r[0],
            off_min[1] + (off_max[1] - off_min[1]) * r[1],
            off_min[2] + (off_max[2] - off_min[2]) * r[2],
        ];
        let rot_deg = [
            rot_min[0] + (rot_max[0] - rot_min[0]) * r[3],
            rot_min[1] + (rot_max[1] - rot_min[1]) * r[4],
            rot_min[2] + (rot_max[2] - rot_min[2]) * r[5],
        ];
        let rot_rad = [
            rot_deg[0].to_radians(),
            rot_deg[1].to_radians(),
            rot_deg[2].to_radians(),
        ];
        let sc_val = [
            sc_min[0] + (sc_max[0] - sc_min[0]) * r[6],
            sc_min[1] + (sc_max[1] - sc_min[1]) * r[7],
            sc_min[2] + (sc_max[2] - sc_min[2]) * r[8],
        ];
        let sc = if uniform_scale {
            [sc_val[0], sc_val[0], sc_val[0]]
        } else {
            sc_val
        };

        new_p.extend_from_slice(&[
            (a.p[i][0] + off[0]) as f32,
            (a.p[i][1] + off[1]) as f32,
            (a.p[i][2] + off[2]) as f32,
        ]);

        let final_rot = if absolute_rotation {
            rot_rad
        } else {
            let r_base = euler_xyz_to_matrix(a.rot[i]);
            let r_add = euler_xyz_to_matrix(rot_rad);
            matrix_to_euler_xyz(&mat_mul(&r_base, &r_add))
        };
        new_rot.extend_from_slice(&[
            final_rot[0] as f32,
            final_rot[1] as f32,
            final_rot[2] as f32,
        ]);

        new_scale.extend_from_slice(&[
            (a.scale[i][0] * sc[0]) as f32,
            (a.scale[i][1] * sc[1]) as f32,
            (a.scale[i][2] * sc[2]) as f32,
        ]);
    }

    let mut attrs = HashMap::new();
    attrs.insert("P".into(), NewAttrVal::F32(3, new_p));
    attrs.insert("rot".into(), NewAttrVal::F32(3, new_rot));
    attrs.insert("scale".into(), NewAttrVal::F32(3, new_scale));
    cloud(&points, None, Some(attrs))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
