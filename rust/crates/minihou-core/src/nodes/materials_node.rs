//! `set_material` from minihou/nodes/materials_node.py.
use std::sync::Arc;

use super::{GEO, InputSpec, Inputs, NodeSpec};
use crate::error::{Error, Result};
use crate::geometry::{Attr, AttrData, Geometry};
use crate::params::Params;

const KEY: &str = "materials";
const ATTR: &str = "material_index";
const POINT_ATTR: &str = "material_id";
const MAX_NAME: usize = 63;

pub const SET_MATERIAL: NodeSpec = NodeSpec {
    name: "set_material",
    inputs: GEO,
    params: crate::params!(
        "domain": "primitives";
        "group": "";
        "material": "";
        "names": "";
        "index_attrib": "";
    ),
    cook: set_material,
};

fn clean(s: &str) -> String {
    let trimmed = s.trim();
    if trimmed.len() > MAX_NAME {
        trimmed[..MAX_NAME].to_string()
    } else {
        trimmed.to_string()
    }
}

fn get_names(geo: &Geometry) -> Vec<String> {
    if let Some(serde_json::Value::Array(arr)) = geo.detail.get(KEY) {
        arr.iter()
            .map(|v| clean(v.as_str().unwrap_or("")))
            .collect()
    } else {
        Vec::new()
    }
}

fn get_indices(geo: &Geometry, dom: &str) -> Vec<i64> {
    let n = if dom == "prim" { geo.nprims() } else { geo.npoints };
    let table = if dom == "prim" { &geo.prim } else { &geo.point };
    let attr_name = if dom == "prim" { ATTR } else { POINT_ATTR };
    if let Some(a) = table.get(attr_name) {
        if let AttrData::I32(v) = &a.data {
            return (0..n).map(|i| v[i * a.width] as i64).collect();
        }
    }
    vec![0i64; n]
}

fn top(geo: &Geometry) -> i64 {
    let mut top_val = 0i64;
    for (table, name, n) in [(&geo.prim, ATTR, geo.nprims()), (&geo.point, POINT_ATTR, geo.npoints)] {
        if let Some(a) = table.get(name) {
            if let AttrData::I32(v) = &a.data {
                for i in 0..n {
                    top_val = top_val.max(v[i * a.width] as i64 + 1);
                }
            }
        }
    }
    top_val
}

fn slot_for(lst: &mut Vec<String>, name: &str) -> usize {
    let cleaned = clean(name);
    if let Some(pos) = lst.iter().position(|x| x == &cleaned) {
        pos
    } else {
        lst.push(cleaned);
        lst.len() - 1
    }
}

fn compact(mut lst: Vec<String>, top_val: usize) -> Vec<String> {
    let min_len = top_val.max(1);
    while lst.len() > min_len && lst.last().map_or(false, |s| s.is_empty()) {
        lst.pop();
    }
    lst
}

fn assign_picks(
    geo: &Geometry,
    mask: &[bool],
    picks: &[usize],
    choices: &[String],
    dom: &str,
) -> Geometry {
    let mut out = (*geo).clone();
    let mut lst = get_names(geo);
    let mut idx = get_indices(geo, dom);

    let top_limit = (top(geo) as usize).max(1);
    while lst.len() < top_limit {
        lst.push(String::new());
    }

    let slots: Vec<i64> = choices.iter().map(|c| slot_for(&mut lst, c) as i64).collect();
    for i in 0..idx.len() {
        if mask[i] {
            idx[i] = slots[picks[i]];
        }
    }

    let attr_name = if dom == "prim" { ATTR } else { POINT_ATTR };
    let table = if dom == "prim" { &mut out.prim } else { &mut out.point };
    table.insert(
        attr_name.to_string(),
        Arc::new(Attr::i32(1, idx.iter().map(|&x| x as i32).collect())),
    );

    let compacted = compact(lst, top(&out) as usize);
    let detail_val = serde_json::Value::Array(
        compacted.into_iter().map(serde_json::Value::String).collect(),
    );
    out.detail.insert(KEY.to_string(), detail_val);
    out
}

fn set_material(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = SET_MATERIAL.one(inputs, 0)?;
    let domain_param = p.e("domain", &["primitives", "points"])?;
    let dom = if domain_param == "primitives" { "prim" } else { "point" };
    let group = p.s("group")?;
    let mask = super::group::mask(&geo, dom, &group)?;

    if !mask.iter().any(|&b| b) {
        return Ok((*geo).clone());
    }

    let index_attrib = p.s("index_attrib")?;
    let index_attrib_trim = index_attrib.trim();

    if index_attrib_trim.is_empty() {
        let mat = clean(&p.s("material")?);
        let n = if dom == "prim" { geo.nprims() } else { geo.npoints };
        let picks = vec![0usize; n];
        return Ok(assign_picks(&geo, &mask, &picks, &[mat], dom));
    }

    let names_str = p.s("names")?;
    let choices: Vec<String> = names_str
        .split(',')
        .filter_map(|x| {
            let c = clean(x);
            if c.is_empty() {
                None
            } else {
                Some(c)
            }
        })
        .collect();

    if choices.is_empty() {
        return Err(Error::new(
            "list the material names in 'names' (separated by commas) for 'index_attrib' to pick from",
        ));
    }

    let table = if dom == "prim" { &geo.prim } else { &geo.point };
    let a = table
        .get(index_attrib_trim)
        .ok_or_else(|| Error::new(format!("no {} attribute {:?}", if dom == "prim" { "primitive" } else { "point" }, index_attrib_trim)))?;

    if a.width != 1 {
        return Err(Error::new(format!(
            "{index_attrib_trim:?} must be an integer attribute (one value per element) to pick a material"
        )));
    }

    let vals: Vec<i64> = match &a.data {
        AttrData::I32(v) => v.iter().map(|&x| x as i64).collect(),
        _ => {
            return Err(Error::new(format!(
                "{index_attrib_trim:?} must be an integer attribute (one value per element) to pick a material"
            )))
        }
    };

    let n_choices = choices.len() as i64;
    let picks: Vec<usize> = vals.iter().map(|&x| x.rem_euclid(n_choices) as usize).collect();

    Ok(assign_picks(&geo, &mask, &picks, &choices, dom))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_set_material_basic() {
        let pts = vec![0.0; 12];
        let vp = vec![0, 1, 2, 0, 2, 3];
        let offs = vec![0, 3, 6];
        let geo = Geometry::from_arrays(pts, vp, offs, None).unwrap();

        let p = Params::resolve(SET_MATERIAL.params, serde_json::json!({"material": "Wood"}).as_object()).unwrap();
        let out = set_material(&[vec![Arc::new(geo)]], &p).unwrap();

        let names = get_names(&out);
        assert_eq!(names, vec!["", "Wood"]);
        let idx = get_indices(&out, "prim");
        assert_eq!(idx, vec![1, 1]);
    }

    #[test]
    fn test_set_material_index_attrib() {
        let pts = vec![0.0; 12];
        let vp = vec![0, 1, 2, 0, 2, 3];
        let offs = vec![0, 3, 6];
        let mut geo = Geometry::from_arrays(pts, vp, offs, None).unwrap();
        geo.prim.insert("piece".to_string(), Arc::new(Attr::i32(1, vec![0, 1])));

        let p = Params::resolve(SET_MATERIAL.params, serde_json::json!({
            "index_attrib": "piece",
            "names": "Wood, Metal, Glass"
        }).as_object()).unwrap();
        let out = set_material(&[vec![Arc::new(geo)]], &p).unwrap();

        let names = get_names(&out);
        assert_eq!(names, vec!["", "Wood", "Metal", "Glass"]);
        let idx = get_indices(&out, "prim");
        assert_eq!(idx, vec![1, 2]);
    }
}
