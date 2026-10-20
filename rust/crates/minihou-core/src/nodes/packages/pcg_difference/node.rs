use crate::nodes::pcg::*;
use crate::nodes::{InputSpec, Inputs, NodeSpec};
use crate::{error::{Error, Result}, geometry::{Attr, AttrData, Geometry}, params::Params};
use std::collections::HashMap;
use std::sync::Arc;

pub(super) const NODE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "pcg_difference",
    inputs: &[
        InputSpec {
            name: "points",
            multi: false,
            optional: false,
        },
        InputSpec {
            name: "subtract",
            multi: false,
            optional: false,
        },
    ],
    params: crate::params!("density_function": "binary"),
    cook: pcg_difference,
};

fn pcg_difference(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let points = NODE.one(inputs, 0)?;
    let subtract = NODE.one(inputs, 1)?;
    let density_function = p.e("density_function", &["binary", "subtract"])?;

    let a = read_pcg(&points);
    let (cov, touched) = coverage(&a, &subtract);

    if density_function == "binary" {
        let keep: Vec<bool> = touched.iter().map(|&t| !t).collect();
        cloud(&points, Some(&keep), None)
    } else {
        let mut d_out = Vec::with_capacity(points.npoints);
        let mut keep = Vec::with_capacity(points.npoints);
        for i in 0..points.npoints {
            let d = a.density[i] * (1.0 - cov[i]);
            keep.push(d > 1e-6);
            d_out.push(d as f32);
        }
        let mut attrs = HashMap::new();
        attrs.insert("density".into(), NewAttrVal::F32(1, d_out));
        cloud(&points, Some(&keep), Some(attrs))
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
