use crate::nodes::pcg::*;
use crate::nodes::{InputSpec, Inputs, NodeSpec};
use crate::{error::{Error, Result}, geometry::{Attr, AttrData, Geometry}, params::Params};
use std::collections::HashMap;
use std::sync::Arc;

pub(super) const NODE: NodeSpec = NodeSpec {
    name: "pcg_spawner",
    inputs: &[InputSpec {
        name: "points",
        multi: false,
        optional: false,
    }],
    params: crate::params!(
        "objects": "";
        "seed": 0
    ),
    cook: pcg_spawner,
};

fn pcg_spawner(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let points = NODE.one(inputs, 0)?;
    let objects_str = p.s("objects")?;
    let seed = p.i("seed")?;

    let mut entries = Vec::new();
    for tok in objects_str.replace(',', " ").split_whitespace() {
        let (name, weight_str) = match tok.split_once(':') {
            Some((n, w)) => (n, w),
            None => (tok, "1.0"),
        };
        let weight: f64 = weight_str.parse().map_err(|_| {
            Error::new(format!("weight of {name:?} must be a number, e.g. {name}:2"))
        })?;
        if !name.is_empty() && weight > 0.0 {
            entries.push((name.to_string(), weight));
        }
    }
    if entries.is_empty() {
        return Err(Error::new("list the Blender objects to spawn, e.g. 'Tree:3 Rock:1'"));
    }

    let a = read_pcg(&points);
    let n = points.npoints;
    let total_w: f64 = entries.iter().map(|e| e.1).sum();
    let mut cdf = Vec::with_capacity(entries.len());
    let mut acc = 0.0;
    for e in &entries {
        acc += e.1;
        cdf.push(acc / total_w);
    }

    let mut instance = Vec::with_capacity(n);
    let mut variant = Vec::with_capacity(n);
    for i in 0..n {
        let r = random01(a.seed[i], seed, 0);
        let pick = match cdf.binary_search_by(|v| {
            v.partial_cmp(&r).unwrap_or(std::cmp::Ordering::Equal)
        }) {
            Ok(idx) => (idx + 1).min(entries.len() - 1),
            Err(idx) => idx.min(entries.len() - 1),
        };
        instance.push((pick + 1) as i32);
        variant.push(pick as i32);
    }

    let mut attrs = HashMap::new();
    attrs.insert(
        "rot".into(),
        NewAttrVal::F32(
            3,
            a.rot
                .iter()
                .flat_map(|r| [r[0] as f32, r[1] as f32, r[2] as f32])
                .collect(),
        ),
    );
    attrs.insert(
        "scale".into(),
        NewAttrVal::F32(
            3,
            a.scale
                .iter()
                .flat_map(|s| [s[0] as f32, s[1] as f32, s[2] as f32])
                .collect(),
        ),
    );
    attrs.insert("instance".into(), NewAttrVal::I32(1, instance));
    attrs.insert("variant".into(), NewAttrVal::I32(1, variant));

    let mut out = cloud(&points, None, Some(attrs))?;
    let names_json: Vec<serde_json::Value> = entries
        .iter()
        .map(|e| serde_json::json!(e.0))
        .collect();
    out.detail
        .insert("instance_objects".into(), serde_json::Value::Array(names_json));
    out.detail.remove("instance_bounds");
    Ok(out)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
