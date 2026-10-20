use crate::nodes::pcg::*;
use crate::nodes::{InputSpec, Inputs, NodeSpec};
use crate::{error::{Error, Result}, geometry::{Attr, AttrData, Geometry}, params::Params};
use std::collections::HashMap;
use std::sync::Arc;

pub(super) const NODE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "pcg_volume_sampler",
    inputs: &[InputSpec {
        name: "volume",
        multi: false,
        optional: false,
    }],
    params: crate::params!(
        "voxel_size": [1.0, 1.0, 1.0];
        "jitter": 0.0;
        "seed": 0
    ),
    cook: pcg_volume_sampler,
};

fn pcg_volume_sampler(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let volume = NODE.one(inputs, 0)?;
    let voxel_size = p.v3("voxel_size")?;
    let jitter = p.f("jitter")?;
    let seed = p.i("seed")?;

    if volume.topology.has_open() {
        let n = volume.topology.closed_mask().iter().filter(|&&c| !c).count();
        return Err(Error::new(format!(
            "pcg_volume_sampler works on polygons, but the input has {n} open curve(s); remove them first, e.g. blast with group '@closed == 0'"
        )));
    }
    if volume.nprims() == 0 {
        return new_points_cloud(Vec::new(), None, [0.5, 0.5, 0.5], None, None);
    }

    let vp = volume.p();
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for c in vp.chunks_exact(3) {
        for k in 0..3 {
            lo[k] = lo[k].min(c[k] as f64);
            hi[k] = hi[k].max(c[k] as f64);
        }
    }

    let v = voxel_size;
    let dims = [
        ((hi[0] - lo[0]) / v[0]).floor().max(1.0) as usize,
        ((hi[1] - lo[1]) / v[1]).floor().max(1.0) as usize,
        ((hi[2] - lo[2]) / v[2]).floor().max(1.0) as usize,
    ];

    if dims[0] * dims[1] * dims[2] > 5_000_000 {
        return Err(Error::new(format!(
            "{} x {} x {} voxels is too many; use a larger voxel size",
            dims[0], dims[1], dims[2]
        )));
    }

    let start = [
        lo[0] + ((hi[0] - lo[0]) - dims[0] as f64 * v[0]) / 2.0 + v[0] / 2.0,
        lo[1] + ((hi[1] - lo[1]) - dims[1] as f64 * v[1]) / 2.0 + v[1] / 2.0,
        lo[2] + ((hi[2] - lo[2]) - dims[2] as f64 * v[2]) / 2.0 + v[2] / 2.0,
    ];

    let mut grid_pts = Vec::new();
    for ix in 0..dims[0] {
        let x = start[0] + ix as f64 * v[0];
        for iy in 0..dims[1] {
            let y = start[1] + iy as f64 * v[1];
            for iz in 0..dims[2] {
                let z = start[2] + iz as f64 * v[2];
                grid_pts.push([x, y, z]);
            }
        }
    }

    let tris = fan_triangles(&volume);
    let mut scale = 0.0f64;
    for k in 0..3 {
        scale = scale.max(hi[k] - lo[k]);
    }
    let mut inside_pts = Vec::new();
    for pt in grid_pts {
        if point_inside_mesh(pt, &tris, scale) {
            inside_pts.push(pt);
        }
    }

    if jitter > 0.0 && !inside_pts.is_empty() {
        for pt in &mut inside_pts {
            let pt_f32 = [pt[0] as f32, pt[1] as f32, pt[2] as f32];
            let s = position_seeds(&pt_f32)[0];
            for k in 0..3 {
                let r = random01(s, seed, k);
                pt[k] += (r - 0.5) * v[k] * jitter;
            }
        }
    }

    let mut p_out = Vec::with_capacity(inside_pts.len() * 3);
    for pt in inside_pts {
        p_out.extend_from_slice(&[pt[0] as f32, pt[1] as f32, pt[2] as f32]);
    }
    new_points_cloud(
        p_out,
        None,
        [v[0] / 2.0, v[1] / 2.0, v[2] / 2.0],
        None,
        None,
    )
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
