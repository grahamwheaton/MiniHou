use crate::nodes::pcg::*;
use crate::nodes::{InputSpec, Inputs, NodeSpec};
use crate::{error::{Error, Result}, geometry::{Attr, AttrData, Geometry}, params::Params};
use std::collections::HashMap;
use std::sync::Arc;

pub(super) const NODE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "pcg_debug",
    inputs: &[InputSpec {
        name: "points",
        multi: false,
        optional: false,
    }],
    params: crate::params!("color_by_density": true),
    cook: pcg_debug,
};

fn pcg_debug(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let points = NODE.one(inputs, 0)?;
    let color_by_density = p.b("color_by_density")?;

    let a = read_pcg(&points);
    let n = points.npoints;

    // Unit cube corners (-1..1): 8 points
    let unit_corners: [[f64; 3]; 8] = [
        [-1.0, -1.0, -1.0],
        [1.0, -1.0, -1.0],
        [1.0, 1.0, -1.0],
        [-1.0, 1.0, -1.0],
        [-1.0, -1.0, 1.0],
        [1.0, -1.0, 1.0],
        [1.0, 1.0, 1.0],
        [-1.0, 1.0, 1.0],
    ];
    // 6 quads
    let unit_vp: [i32; 24] = [
        0, 3, 2, 1, // bottom
        4, 5, 6, 7, // top
        0, 1, 5, 4, // front
        2, 3, 7, 6, // back
        0, 4, 7, 3, // left
        1, 2, 6, 5, // right
    ];

    let boxes = compute_boxes(&a);
    let mut p_out = Vec::with_capacity(n * 8 * 3);
    let mut vp_out = Vec::with_capacity(n * 24);
    let mut offs_out = Vec::with_capacity(n * 6 + 1);
    offs_out.push(0);

    for i in 0..n {
        let base_pt = (i * 8) as i32;
        for c in &unit_corners {
            let pt = [
                boxes.c[i][0]
                    + boxes.a[i][0][0] * (c[0] * boxes.h[i][0])
                    + boxes.a[i][0][1] * (c[1] * boxes.h[i][1])
                    + boxes.a[i][0][2] * (c[2] * boxes.h[i][2]),
                boxes.c[i][1]
                    + boxes.a[i][1][0] * (c[0] * boxes.h[i][0])
                    + boxes.a[i][1][1] * (c[1] * boxes.h[i][1])
                    + boxes.a[i][1][2] * (c[2] * boxes.h[i][2]),
                boxes.c[i][2]
                    + boxes.a[i][2][0] * (c[0] * boxes.h[i][0])
                    + boxes.a[i][2][1] * (c[1] * boxes.h[i][1])
                    + boxes.a[i][2][2] * (c[2] * boxes.h[i][2]),
            ];
            p_out.extend_from_slice(&[pt[0] as f32, pt[1] as f32, pt[2] as f32]);
        }
        for face in 0..6 {
            for v_idx in 0..4 {
                vp_out.push(base_pt + unit_vp[face * 4 + v_idx]);
            }
            offs_out.push(vp_out.len() as i32);
        }
    }

    let mut out = Geometry::from_arrays(p_out, vp_out, offs_out, None)?;
    if color_by_density {
        let mut cd_out = Vec::with_capacity(n * 8 * 3);
        for i in 0..n {
            let d = a.density[i] as f32;
            let col = [
                0.4 * (1.0 - d) + d,
                0.02 * (1.0 - d) + d,
                0.02 * (1.0 - d) + d,
            ];
            for _ in 0..8 {
                cd_out.extend_from_slice(&col);
            }
        }
        out.point.insert("Cd".into(), Arc::new(Attr::f32(3, cd_out)));
    }
    let mut d_rep = Vec::with_capacity(n * 8);
    for i in 0..n {
        let d = a.density[i] as f32;
        d_rep.extend(vec![d; 8]);
    }
    out.point
        .insert("density".into(), Arc::new(Attr::f32(1, d_rep)));
    Ok(out)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
