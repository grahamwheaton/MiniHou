use crate::nodes::pcg::*;
use crate::nodes::{InputSpec, Inputs, NodeSpec};
use crate::{error::{Error, Result}, geometry::{Attr, AttrData, Geometry}, params::Params};
use std::collections::HashMap;
use std::sync::Arc;

pub(super) const NODE: NodeSpec = NodeSpec {
    name: "pcg_self_prune",
    inputs: &[InputSpec {
        name: "points",
        multi: false,
        optional: false,
    }],
    params: crate::params!(
        "priority": "large_first";
        "seed": 0
    ),
    cook: pcg_self_prune,
};

fn pcg_self_prune(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let points = NODE.one(inputs, 0)?;
    let priority = p.e(
        "priority",
        &["large_first", "small_first", "density", "random"],
    )?;
    let seed = p.i("seed")?;

    let a = read_pcg(&points);
    let n = points.npoints;
    let boxes = compute_boxes(&a);
    let (i_pairs, j_pairs) = overlapping_pairs(&boxes, None);

    let mut vol = Vec::with_capacity(n);
    let mut tie = Vec::with_capacity(n);
    let mut max_vol = 1.0f64;
    for i in 0..n {
        let dx = (a.bounds_max[i][0] - a.bounds_min[i][0]).abs() * a.scale[i][0].abs();
        let dy = (a.bounds_max[i][1] - a.bounds_min[i][1]).abs() * a.scale[i][1].abs();
        let dz = (a.bounds_max[i][2] - a.bounds_min[i][2]).abs() * a.scale[i][2].abs();
        let v = dx * dy * dz;
        max_vol = max_vol.max(v);
        vol.push(v);
        tie.push(random01(a.seed[i], seed, 0) * 1e-6);
    }

    let pri: Vec<f64> = match priority.as_str() {
        "large_first" => vol
            .iter()
            .zip(tie.iter())
            .map(|(&v, &t)| v + t * max_vol)
            .collect(),
        "small_first" => vol
            .iter()
            .zip(tie.iter())
            .map(|(&v, &t)| -v + t * max_vol)
            .collect(),
        "density" => a
            .density
            .iter()
            .zip(tie.iter())
            .map(|(&d, &t)| d + t)
            .collect(),
        _ => tie,
    };

    let keep = greedy_independent(n, &i_pairs, &j_pairs, &pri);
    cloud(&points, Some(&keep), None)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
