use crate::nodes::pcg::*;
use crate::nodes::{InputSpec, Inputs, NodeSpec};
use crate::{error::{Error, Result}, geometry::{Attr, AttrData, Geometry}, params::Params};
use std::collections::HashMap;
use std::sync::Arc;

pub(super) const NODE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None,
    name: "pcg_copy_points",
    inputs: &[
        InputSpec {
            name: "source",
            multi: false,
            optional: false,
        },
        InputSpec {
            name: "targets",
            multi: false,
            optional: false,
        },
    ],
    params: crate::params!(
        "inherit_rotation": true;
        "inherit_scale": true
    ),
    cook: pcg_copy_points,
};

fn pcg_copy_points(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let source = NODE.one(inputs, 0)?;
    let targets = NODE.one(inputs, 1)?;
    let inherit_rot = p.b("inherit_rotation")?;
    let inherit_scale = p.b("inherit_scale")?;

    let s = read_pcg(&source);
    let t = read_pcg(&targets);
    let (ns, nt) = (source.npoints, targets.npoints);
    let total = ns * nt;

    let mut p_out = Vec::with_capacity(total * 3);
    let mut rot_out = Vec::with_capacity(total * 3);
    let mut scale_out = Vec::with_capacity(total * 3);
    let mut bmin_out = Vec::with_capacity(total * 3);
    let mut bmax_out = Vec::with_capacity(total * 3);
    let mut density_out = Vec::with_capacity(total);
    let mut seed_out = Vec::with_capacity(total);

    for ti in 0..nt {
        let rt = if inherit_rot {
            euler_xyz_to_matrix(t.rot[ti])
        } else {
            [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
        };
        let st = if inherit_scale {
            t.scale[ti]
        } else {
            [1.0, 1.0, 1.0]
        };

        for si in 0..ns {
            let sp = [
                s.p[si][0] * st[0],
                s.p[si][1] * st[1],
                s.p[si][2] * st[2],
            ];
            let pt = [
                t.p[ti][0] + rt[0][0] * sp[0] + rt[0][1] * sp[1] + rt[0][2] * sp[2],
                t.p[ti][1] + rt[1][0] * sp[0] + rt[1][1] * sp[1] + rt[1][2] * sp[2],
                t.p[ti][2] + rt[2][0] * sp[0] + rt[2][1] * sp[1] + rt[2][2] * sp[2],
            ];
            p_out.extend_from_slice(&[pt[0] as f32, pt[1] as f32, pt[2] as f32]);

            let rs = euler_xyz_to_matrix(s.rot[si]);
            let final_r = mat_mul(&rt, &rs);
            let euler = matrix_to_euler_xyz(&final_r);
            rot_out.extend_from_slice(&[euler[0] as f32, euler[1] as f32, euler[2] as f32]);

            scale_out.extend_from_slice(&[
                (s.scale[si][0] * st[0]) as f32,
                (s.scale[si][1] * st[1]) as f32,
                (s.scale[si][2] * st[2]) as f32,
            ]);

            bmin_out.extend_from_slice(&[
                s.bounds_min[si][0] as f32,
                s.bounds_min[si][1] as f32,
                s.bounds_min[si][2] as f32,
            ]);
            bmax_out.extend_from_slice(&[
                s.bounds_max[si][0] as f32,
                s.bounds_max[si][1] as f32,
                s.bounds_max[si][2] as f32,
            ]);

            density_out.push((s.density[si] * t.density[ti]) as f32);

            let s_val = (s.seed[si] as i64 as u64).wrapping_mul(0x9E3779B1);
            let mixed = hash32((s_val ^ (t.seed[ti] as i64 as u64)) as u32);
            seed_out.push((mixed & 0x7FFF_FFFF) as i32);
        }
    }

    let mut out = Geometry::points_only(p_out)?;
    // Copy other attributes from source
    for (name, attr) in &source.point {
        if name != "P"
            && name != "rot"
            && name != "scale"
            && name != "bounds_min"
            && name != "bounds_max"
            && name != "density"
            && name != "seed"
        {
            let w = attr.width;
            let mut gathered = Vec::with_capacity(total * w);
            for _ in 0..nt {
                match &attr.data {
                    AttrData::F32(v) => gathered.extend_from_slice(v),
                    AttrData::I32(v) => gathered.extend(v.iter().map(|&x| x as f32)),
                }
            }
            out.point
                .insert(name.clone(), Arc::new(Attr::f32(w, gathered)));
        }
    }
    out.point.insert("rot".into(), Arc::new(Attr::f32(3, rot_out)));
    out.point
        .insert("scale".into(), Arc::new(Attr::f32(3, scale_out)));
    out.point
        .insert("bounds_min".into(), Arc::new(Attr::f32(3, bmin_out)));
    out.point
        .insert("bounds_max".into(), Arc::new(Attr::f32(3, bmax_out)));
    out.point
        .insert("density".into(), Arc::new(Attr::f32(1, density_out)));
    out.point
        .insert("seed".into(), Arc::new(Attr::i32(1, seed_out)));
    Ok(out)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
