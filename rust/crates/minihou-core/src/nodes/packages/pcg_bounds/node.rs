use crate::nodes::pcg::{cloud, read_pcg, NewAttrVal};
use std::collections::HashMap;
use crate::nodes::{InputSpec, Inputs, NodeSpec};
use crate::{error::Result, geometry::Geometry, params::Params};

pub(super) const NODE: NodeSpec = NodeSpec {
    name: "pcg_bounds",
    inputs: &[InputSpec {
        name: "points",
        multi: false,
        optional: false,
    }],
    params: crate::params!(
        "mode": "set";
        "bounds_min": [-0.5, -0.5, -0.5];
        "bounds_max": [0.5, 0.5, 0.5];
        "factor": 1.0
    ),
    cook: pcg_bounds,
};

fn pcg_bounds(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let points = NODE.one(inputs, 0)?;
    let mode = p.e("mode", &["set", "scale"])?;
    let bmin = p.v3("bounds_min")?;
    let bmax = p.v3("bounds_max")?;
    let factor = p.f("factor")?;

    let a = read_pcg(&points);
    let n = points.npoints;
    let mut lo = Vec::with_capacity(n * 3);
    let mut hi = Vec::with_capacity(n * 3);

    if mode == "set" {
        for _ in 0..n {
            lo.extend_from_slice(&[bmin[0] as f32, bmin[1] as f32, bmin[2] as f32]);
            hi.extend_from_slice(&[bmax[0] as f32, bmax[1] as f32, bmax[2] as f32]);
        }
    } else {
        for i in 0..n {
            for c in 0..3 {
                let mid = (a.bounds_min[i][c] + a.bounds_max[i][c]) / 2.0;
                let l = mid + (a.bounds_min[i][c] - mid) * factor;
                let h = mid + (a.bounds_max[i][c] - mid) * factor;
                lo.push(l as f32);
                hi.push(h as f32);
            }
        }
    }

    let mut attrs = HashMap::new();
    attrs.insert("bounds_min".into(), NewAttrVal::F32(3, lo));
    attrs.insert("bounds_max".into(), NewAttrVal::F32(3, hi));
    cloud(&points, None, Some(attrs))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
