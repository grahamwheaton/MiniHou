use crate::nodes::{Inputs, NodeSpec, GEO};
use crate::error::{Error, Result};
use crate::geometry::{Attr, Geometry};
use crate::params::Params;
use crate::nodes::attrib_tools::{domain, size, set};
use serde_json::{json, Value};

pub(super) const NODE: NodeSpec = NodeSpec {
    name: "attribute_create",
    inputs: GEO,
    params: crate::params!(
        "name": "attrib1";
        "domain": "points";
        "type": "float";
        "value": [0., 0., 0.];
        "group": "";
        "attributes": Value::Null;
        "group_type": "guess";
        "encode_invalid": false;
        "enable": true;
        "existing": "overwrite";
        "varname": "";
        "save_to_info_block": false;
        "guess_from_name": false;
        "precision": "auto";
        "write_values": true;
        "allow_local": true
    ),
    cook: create,
};

fn sanitize_name(name: &str, encode: bool) -> Result<String> {
    if !encode {
        crate::nodes::group::valid_name(name)?;
        return Ok(name.to_string());
    }
    if name.is_empty() {
        return Ok("_".to_string());
    }
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if i == 0 {
            if c.is_ascii_alphabetic() || c == '_' {
                out.push(c);
            } else {
                out.push('_');
                if c.is_ascii_digit() {
                    out.push(c);
                }
            }
        } else if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    Ok(out)
}

fn infer_type_from_name(name: &str) -> &'static str {
    match name {
        "P" | "N" | "v" | "force" | "accel" | "up" => "vector",
        "Cd" => "color",
        "uv" => "vector2",
        "id" | "ptnum" | "primnum" | "vtxnum" => "int",
        "name" | "path" | "material" | "shop_materialpath" => "string",
        _ => "float",
    }
}

fn create(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = NODE.one(inputs, 0)?;
    let group = p.s("group")?;
    let group_type = p.s("group_type").unwrap_or_else(|_| "guess".to_string());
    let encode_invalid = p.b("encode_invalid").unwrap_or(false);

    let Some(attrs) = p.value("attributes").as_array() else {
        if !p.value("attributes").is_null() {
            return Err(Error::new("attributes must be an array"));
        }
        let enable = p.b("enable").unwrap_or(true);
        if !enable {
            return Ok((*g).clone());
        }
        let existing = p.s("existing").unwrap_or_else(|_| "overwrite".to_string());
        let guess_from_name = p.b("guess_from_name").unwrap_or(false);
        let write_values = p.b("write_values").unwrap_or(true);
        let raw_name = p.s("name")?;
        let name = sanitize_name(&raw_name, encode_invalid)?;
        let class = p.s("domain")?;
        let raw_type = p.s("type")?;
        let kind = if guess_from_name { infer_type_from_name(&name) } else { raw_type.as_str() };
        return create_one(
            &g,
            &name,
            &class,
            kind,
            None,
            None,
            p.value("value"),
            &group,
            &group_type,
            &existing,
            write_values,
        );
    };

    let mut out = (*g).clone();
    for (i, a) in attrs.iter().enumerate() {
        let enable = a.get("enable").and_then(|v| v.as_bool()).unwrap_or(true);
        if !enable {
            continue;
        }
        let field = |key: &str| a.get(key).ok_or_else(|| Error::new(format!("attribute {} missing {key}", i + 1)));
        let string = |key: &str| -> Result<&str> {
            field(key)?.as_str().ok_or_else(|| Error::new(format!("attribute {} {key} must be a string", i + 1)))
        };
        let raw_name = string("name")?;
        let name = sanitize_name(raw_name, encode_invalid)?;
        let class = string("class")?;
        let raw_type = string("type")?;
        let guess_from_name = a.get("guess_from_name").and_then(|v| v.as_bool()).unwrap_or(false);
        let kind = if guess_from_name { infer_type_from_name(&name) } else { raw_type };
        let existing = a.get("existing").and_then(|v| v.as_str()).unwrap_or("overwrite");
        let write_values = a.get("write_values").and_then(|v| v.as_bool()).unwrap_or(true);
        let item_group_type = a.get("group_type").and_then(|v| v.as_str()).unwrap_or(&group_type);
        out = create_one(
            &out,
            &name,
            class,
            kind,
            Some(field("size")?),
            Some(field("default")?),
            field("value")?,
            &group,
            item_group_type,
            existing,
            write_values,
        )?;
    }
    Ok(out)
}

fn components(value: &Value, w: usize, label: &str) -> Result<Vec<f64>> {
    let a = value.as_array().ok_or_else(|| Error::new(format!("{label} must be an array")))?;
    if a.len() < w {
        return Err(Error::new(format!("{label} needs {w} components")));
    }
    a.iter().take(w).map(|v| v.as_f64().ok_or_else(|| Error::new(format!("{label} components must be numbers")))).collect()
}

fn create_one(
    g: &Geometry,
    name: &str,
    class: &str,
    kind: &str,
    width: Option<&Value>,
    default: Option<&Value>,
    value: &Value,
    group: &str,
    group_type: &str,
    existing: &str,
    write_values: bool,
) -> Result<Geometry> {
    if !["points", "primitives", "vertices", "detail"].contains(&class) {
        return Err(Error::new(format!("invalid attribute class {class:?}")));
    }
    let dom = domain(class);
    if existing == "keep" || existing == "never" {
        let exists = match dom {
            "detail" => g.detail.contains_key(name),
            "point" => g.point.contains_key(name),
            "prim" => g.prim.contains_key(name),
            "vertex" => g.vertex.contains_key(name),
            _ => false,
        };
        if exists {
            return Ok(g.clone());
        }
    }
    let natural = match kind {
        "float" | "int" | "string" | "index" => 1,
        "vector2" => 2,
        "vector4" => 4,
        "vector" | "color" => 3,
        "array" => 3,
        "dict" => 1,
        _ => return Err(Error::new(format!("invalid attribute type {kind:?}"))),
    };
    let w = if let Some(width) = width {
        let w = width.as_u64().ok_or_else(|| Error::new("attribute size must be a positive integer"))?;
        if !(1..=4).contains(&w) {
            return Err(Error::new("attribute size must be between 1 and 4"));
        }
        w as usize
    } else {
        natural
    };
    if kind == "string" || kind == "dict" {
        if dom != "detail" {
            return Err(Error::new("string attributes are supported on Detail only"));
        }
        let s = if kind == "string" {
            let text = if write_values {
                value.as_str().ok_or_else(|| Error::new("string attribute value must be text"))?
            } else if let Some(d) = default {
                d.as_str().unwrap_or("")
            } else {
                ""
            };
            json!(text)
        } else {
            value.clone()
        };
        let mut out = g.clone();
        out.detail.insert(name.into(), s);
        return Ok(out);
    }
    let d = if let Some(default) = default {
        components(default, w, "default")?
    } else {
        vec![0.; w]
    };
    let v = if !write_values {
        d.clone()
    } else if width.is_none() {
        let a = components(value, 3, "value")?;
        vec![a[0], a[1], a[2], 1.][..w].to_vec()
    } else {
        components(value, w, "value")?
    };
    if dom == "detail" {
        let mut out = g.clone();
        out.detail.insert(name.into(), if w == 1 { json!(v[0]) } else { json!(v) });
        return Ok(out);
    }
    if name == "P" && (dom != "point" || w != 3) {
        return Err(Error::new("P must be a point vector attribute"));
    }
    let n = size(g, dom);
    let eval_dom = if group_type == "guess" || group_type.is_empty() {
        dom
    } else {
        domain(group_type)
    };
    let sel = if !group.trim().is_empty() && (eval_dom == "point" || eval_dom == "prim") {
        if eval_dom == dom {
            crate::nodes::group::mask(g, dom, group)?
        } else {
            let base_mask = crate::nodes::group::mask(g, eval_dom, group)?;
            if eval_dom == "prim" && dom == "point" {
                let mut p_sel = vec![false; n];
                for (p, &selected) in base_mask.iter().enumerate() {
                    if selected {
                        let r = g.topology.prim_offsets[p] as usize..g.topology.prim_offsets[p + 1] as usize;
                        for v in r {
                            p_sel[g.topology.vertex_point[v] as usize] = true;
                        }
                    }
                }
                p_sel
            } else if eval_dom == "point" && dom == "prim" {
                let mut pr_sel = vec![false; n];
                for pr in 0..n {
                    let r = g.topology.prim_offsets[pr] as usize..g.topology.prim_offsets[pr + 1] as usize;
                    if r.clone().any(|v| base_mask[g.topology.vertex_point[v] as usize]) {
                        pr_sel[pr] = true;
                    }
                }
                pr_sel
            } else {
                vec![true; n]
            }
        }
    } else if !group.trim().is_empty() && (dom == "point" || dom == "prim") {
        crate::nodes::group::mask(g, dom, group)?
    } else {
        vec![true; n]
    };

    let arr = if kind == "int" || kind == "index" {
        let mut data = Vec::with_capacity(n * w);
        for selected in &sel {
            for k in 0..w {
                data.push(if *selected { v[k] as i32 } else { d[k] as i32 });
            }
        }
        Attr::i32(w, data)
    } else {
        let mut data = Vec::with_capacity(n * w);
        for selected in &sel {
            for k in 0..w {
                data.push(if *selected { v[k] as f32 } else { d[k] as f32 });
            }
        }
        Attr::f32(w, data)
    };
    Ok(set(g, dom, name.into(), arr))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
