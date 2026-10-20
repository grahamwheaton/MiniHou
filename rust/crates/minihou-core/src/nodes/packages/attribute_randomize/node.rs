use crate::nodes::{Inputs, NodeSpec, GEO};
use crate::error::{Error, Result};
use crate::geometry::{Attr, AttrData, Geometry};
use crate::params::{Params};
use crate::nodes::attrib_tools::{domain, size, set_with_group};
pub(super) const NODE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "attribute_randomize",
    inputs: GEO,
    params: crate::params!(
        "name": "Cd";
        "domain": "points";
        "type": "color";
        "min": [0.0, 0.0, 0.0];
        "max": [1.0, 1.0, 1.0];
        "distribution": "uniform";
        "seed": 0;
        "by": "";
        "group": "";
    ),
    cook: randomize,
};

fn randomize(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = NODE.one(inputs, 0)?;
    let name = p.s("name")?;
    crate::nodes::group::valid_name(&name)?;
    let dom = p.e("domain", &["points", "primitives", "vertices"])?;
    let dom = domain(&dom);
    let kind = p.e("type", &["color", "float", "vector", "int"])?;
    let width = if kind == "float" || kind == "int" { 1 } else { 3 };
    let min_val = p.v3("min")?;
    let max_val = p.v3("max")?;
    let dist = p.e("distribution", &["uniform", "normal"])?;
    let is_normal = dist == "normal";
    let seed = p.i("seed")?;
    let by = p.s("by")?;
    let group = p.s("group")?;
    let n = size(&g, dom);

    let (r, inv) = if !by.is_empty() {
        let src = g.table(dom).get(&by).ok_or_else(|| Error::new(format!("no {dom} attribute {by:?} to randomize by")))?;
        let keys: Vec<i64> = match &src.data {
            AttrData::I32(v) => (0..n).map(|i| v[i * src.width] as i64).collect(),
            AttrData::F32(v) => (0..n).map(|i| v[i * src.width] as i64).collect(),
        };
        let mut uniq = keys.clone();
        uniq.sort_unstable();
        uniq.dedup();
        let inv: Vec<usize> = keys.iter().map(|k| uniq.binary_search(k).unwrap()).collect();
        let mut rng = crate::nodes::np_random::Rng::new(&[(seed as u64) & 0xFFFFFFFF, 7]);
        let mut r = Vec::with_capacity(uniq.len() * width);
        for _ in 0..uniq.len() {
            for _ in 0..width {
                let val = if is_normal {
                    (0.5 + rng.standard_normal() / 6.0).clamp(0.0, 1.0)
                } else {
                    rng.random()
                };
                r.push(val);
            }
        }
        (r, Some(inv))
    } else {
        let mut rng = crate::nodes::np_random::Rng::new(&[(seed as u64) & 0xFFFFFFFF]);
        let mut r = Vec::with_capacity(n * width);
        for _ in 0..n {
            for _ in 0..width {
                let val = if is_normal {
                    (0.5 + rng.standard_normal() / 6.0).clamp(0.0, 1.0)
                } else {
                    rng.random()
                };
                r.push(val);
            }
        }
        (r, None)
    };

    let arr = if kind == "int" {
        let lo = min_val[0];
        let hi = max_val[0];
        let mut data = Vec::with_capacity(n);
        for i in 0..n {
            let r_idx = inv.as_ref().map_or(i, |inv| inv[i]);
            let r_val = r[r_idx];
            let val = (lo + r_val * (hi - lo + 1.0)).floor().clamp(lo, hi) as i32;
            data.push(val);
        }
        Attr::i32(1, data)
    } else {
        let lo = &min_val[..width];
        let hi = &max_val[..width];
        let mut data = Vec::with_capacity(n * width);
        for i in 0..n {
            let r_idx = inv.as_ref().map_or(i, |inv| inv[i]);
            for k in 0..width {
                let r_val = r[r_idx * width + k];
                let val = (lo[k] + r_val * (hi[k] - lo[k])) as f32;
                data.push(val);
            }
        }
        Attr::f32(width, data)
    };

    set_with_group(&g, dom, &name, arr, &group)
}


#[cfg(test)]
#[path = "tests.rs"]
mod tests;
