use crate::nodes::pcg::*;
use crate::nodes::{InputSpec, Inputs, NodeSpec};
use crate::{error::{Error, Result}, geometry::{Attr, AttrData, Geometry}, params::Params};
use std::collections::HashMap;
use std::sync::Arc;

pub(super) const NODE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None,
    name: "pcg_union",
    inputs: &[InputSpec {
        name: "inputs",
        multi: true,
        optional: false,
    }],
    params: crate::params!("density_function": "keep"),
    cook: pcg_union,
};

fn pcg_union(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let list = &inputs[0];
    if list.is_empty() {
        return new_points_cloud(Vec::new(), None, [0.5, 0.5, 0.5], None, None);
    }
    let density_function = p.e("density_function", &["keep", "maximum", "add"])?;

    let mut parts = Vec::new();
    let mut which = Vec::new();
    for (k, g) in list.iter().enumerate() {
        let d = read_pcg(g);
        let n = g.npoints;
        which.extend(vec![k; n]);
        let mut new_attrs = HashMap::new();
        new_attrs.insert(
            "rot".into(),
            NewAttrVal::F32(
                3,
                d.rot
                    .iter()
                    .flat_map(|r| [r[0] as f32, r[1] as f32, r[2] as f32])
                    .collect(),
            ),
        );
        new_attrs.insert(
            "scale".into(),
            NewAttrVal::F32(
                3,
                d.scale
                    .iter()
                    .flat_map(|s| [s[0] as f32, s[1] as f32, s[2] as f32])
                    .collect(),
            ),
        );
        new_attrs.insert(
            "bounds_min".into(),
            NewAttrVal::F32(
                3,
                d.bounds_min
                    .iter()
                    .flat_map(|b| [b[0] as f32, b[1] as f32, b[2] as f32])
                    .collect(),
            ),
        );
        new_attrs.insert(
            "bounds_max".into(),
            NewAttrVal::F32(
                3,
                d.bounds_max
                    .iter()
                    .flat_map(|b| [b[0] as f32, b[1] as f32, b[2] as f32])
                    .collect(),
            ),
        );
        new_attrs.insert(
            "density".into(),
            NewAttrVal::F32(1, d.density.iter().map(|&v| v as f32).collect()),
        );
        new_attrs.insert("seed".into(), NewAttrVal::I32(1, d.seed));
        parts.push(Arc::new(cloud(g, None, Some(new_attrs))?));
    }

    let out = if parts.len() > 1 {
        crate::nodes::merge::merge(&[parts], &Params::resolve(&[], None)?)?
    } else {
        (*parts[0]).clone()
    };

    if density_function == "keep" || list.len() < 2 {
        return Ok(out);
    }

    let a = read_pcg(&out);
    let boxes = compute_boxes(&a);
    let (i_pairs, j_pairs) = overlapping_pairs(&boxes, None);

    let mut d = a.density.clone();
    for (&i, &j) in i_pairs.iter().zip(j_pairs.iter()) {
        if which[i] != which[j] {
            if density_function == "maximum" {
                d[i] = d[i].max(a.density[j]);
                d[j] = d[j].max(a.density[i]);
            } else {
                d[i] += a.density[j];
                d[j] += a.density[i];
            }
        }
    }
    let d_f32: Vec<f32> = d.iter().map(|&v| v.clamp(0.0, 1.0) as f32).collect();
    let mut attrs = HashMap::new();
    attrs.insert("density".into(), NewAttrVal::F32(1, d_f32));
    cloud(&out, None, Some(attrs))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
