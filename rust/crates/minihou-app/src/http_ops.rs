//! Python MCP edit vocabulary. Work on a private snapshot; commit only after validation.
use crate::{param_kind, param_meta, Document, ParamKind, Result};
use minihou_core::{nodes, params::Params};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};

pub const OPS: &[&str] = &[
    "add",
    "insert",
    "remove",
    "rename",
    "connect",
    "disconnect",
    "set",
    "paint_assign",
    "display",
    "expose",
    "unexpose",
    "move",
    "meta",
    "replace",
    "layout",
    "loop",
    "box_add",
    "box_set",
    "box_remove",
];
fn string<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v[key]
        .as_str()
        .ok_or_else(|| format!("{key} must be a string").into())
}
fn object(v: &Value) -> Result<&Map<String, Value>> {
    v.as_object().ok_or_else(|| "expected an object".into())
}
fn node<'a>(g: &'a Value, id: &str) -> Result<&'a Value> {
    g["nodes"]
        .get(id)
        .ok_or_else(|| format!("no such node '{id}'").into())
}
fn spec(g: &Value, id: &str) -> Result<&'static nodes::NodeSpec> {
    let ty = string(node(g, id)?, "type")?;
    nodes::get(ty).ok_or_else(|| {
        format!("node '{id}': node type '{ty}' is not in the Rust backend yet").into()
    })
}
fn refs(v: &Value) -> Result<Vec<String>> {
    match v {
        Value::String(s) => Ok(vec![s.clone()]),
        Value::Array(a) => a
            .iter()
            .map(|x| {
                x.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| "input must name a node".into())
            })
            .collect(),
        Value::Null => Ok(vec![]),
        _ => Err("input must name a node".into()),
    }
}
fn ids(v: &Value) -> Result<Vec<String>> {
    if !v.is_string() && !v.is_array() {
        return Err("id must be a string or list of strings".into());
    }
    refs(v)
}
fn fresh(g: &Value, ty: &str) -> String {
    (1..)
        .map(|n| format!("{ty}{n}"))
        .find(|id| g["nodes"].get(id).is_none())
        .unwrap()
}
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .enumerate()
            .all(|(i, c)| c == '_' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit()))
}
fn pos(v: &Value) -> Result<Value> {
    let a = v
        .as_array()
        .filter(|a| a.len() == 2 && a.iter().all(Value::is_number))
        .ok_or("pos must be [x, y]")?;
    Ok(json!(a
        .iter()
        .map(|x| (x.as_f64().unwrap() * 10.).round() / 10.)
        .collect::<Vec<_>>()))
}
fn ensure_object(v: &mut Value) -> &mut Map<String, Value> {
    if !v.is_object() {
        *v = json!({});
    }
    v.as_object_mut().unwrap()
}
fn entries(g: &mut Value) -> &mut Vec<Value> {
    if !g["exposed"].is_array() {
        g["exposed"] = json!([]);
    }
    g["exposed"].as_array_mut().unwrap()
}
fn clean_box(b: &mut Value) -> Result<()> {
    if !valid_id(string(b, "id")?) {
        return Err("invalid box id".into());
    }
    for key in ["pos", "size"] {
        let p = pos(&b[key])?;
        if p.as_array()
            .unwrap()
            .iter()
            .any(|n| n.as_f64().unwrap().abs() >= 1e7)
        {
            return Err(format!("{key} must be [x, y] in range").into());
        }
        b[key] = if key == "size" {
            json!([
                p[0].as_f64().unwrap().max(40.),
                p[1].as_f64().unwrap().max(40.)
            ])
        } else {
            p
        };
    }
    let color = b["color"].as_str().unwrap_or("");
    if !color.is_empty()
        && (color.len() != 7
            || !color.starts_with('#')
            || !color[1..].chars().all(|c| c.is_ascii_hexdigit()))
    {
        return Err("color must be '#rrggbb'".into());
    }
    b["color"] = json!(color.to_lowercase());
    b["title"] = json!(b["title"]
        .as_str()
        .unwrap_or("")
        .chars()
        .take(80)
        .collect::<String>());
    for key in ["collapsed", "locked", "note"] {
        if b.get(key).is_some_and(|v| !v.is_boolean()) {
            return Err(format!("{key} must be a boolean").into());
        }
    }
    if b.get("text")
        .is_some_and(|v| v.as_str().is_none_or(|s| s.chars().count() > 10000))
    {
        return Err("note text must be a string of at most 10000 characters".into());
    }
    Ok(())
}
fn validate_param(ty: &str, name: &str, value: &Value) -> Result<()> {
    let s =
        nodes::get(ty).ok_or_else(|| format!("node type '{ty}' is not in the Rust backend yet"))?;
    let p = s
        .params
        .iter()
        .find(|p| p.name == name)
        .ok_or_else(|| format!("unknown parameter '{name}'"))?;
    let default = (p.default)();
    let meta = param_meta(ty, name);
    let kind = meta.map_or_else(
        || param_kind(&default),
        |m| crate::metadata_kind(m, &default),
    );
    let good = match kind {
        ParamKind::Bool => value.is_boolean(),
        ParamKind::Integer => value.as_f64().is_some_and(|x| x.fract() == 0.),
        ParamKind::Float => value.is_number(),
        ParamKind::String => value.is_string(),
        ParamKind::Vec3 => value
            .as_array()
            .is_some_and(|a| a.len() == 3 && a.iter().all(Value::is_number)),
        ParamKind::Unsupported => value.is_array() || value.is_object() || value == &default,
    };
    if !good {
        return Err(format!("invalid value for {ty}.{name}").into());
    }
    if let Some(m) = meta {
        if m["kind"] == "enum" {
            if let Some(choices) = m["choices"].as_array().or_else(|| m["options"].as_array()) {
                if !choices.contains(value) {
                    return Err(format!("invalid option for {ty}.{name}").into());
                }
            }
        }
        for x in value
            .as_array()
            .map_or_else(|| vec![value], |a| a.iter().collect())
        {
            if let Some(x) = x.as_f64() {
                if x < m["min"].as_f64().unwrap_or(f64::NEG_INFINITY)
                    || x > m["max"].as_f64().unwrap_or(f64::INFINITY)
                {
                    return Err(format!("value outside limits for {ty}.{name}").into());
                }
            }
        }
    }
    let mut given = Map::new();
    given.insert(name.into(), value.clone());
    Params::resolve(s.params, Some(&given))?;
    Ok(())
}
fn order(g: &Value) -> Result<Vec<String>> {
    let (mut active, mut done, mut out) = (HashSet::new(), HashSet::new(), vec![]);
    // Iterative DFS: validation must not overflow the UI thread's stack on a
    // long graph supplied by an HTTP client.
    for root in object(&g["nodes"])?.keys() {
        let mut stack = vec![(root.clone(), false)];
        while let Some((id, exit)) = stack.pop() {
            if exit {
                active.remove(&id);
                done.insert(id.clone());
                out.push(id);
                continue;
            }
            if done.contains(&id) {
                continue;
            }
            node(g, &id)?;
            if !active.insert(id.clone()) {
                return Err(format!("the graph has a cycle through node '{id}'").into());
            }
            stack.push((id.clone(), true));
            let mut sources = vec![];
            if let Some(ins) = g["nodes"][&id]["inputs"].as_object() {
                for r in ins.values() {
                    sources.extend(refs(r)?);
                }
            }
            for source in sources.into_iter().rev() {
                stack.push((source, false));
            }
        }
    }
    Ok(out)
}
pub fn validate(g: &Value) -> Result<()> {
    for (id, n) in object(&g["nodes"])? {
        if !valid_id(id) {
            return Err(format!("invalid node id '{id}'").into());
        }
        let s = spec(g, id)?;
        if let Some(p) = n.get("params") {
            for (name, v) in object(p)? {
                validate_param(s.name, name, v)?;
            }
        }
        if let Some(ins) = n.get("inputs") {
            for (port, v) in object(ins)? {
                let i =
                    s.inputs.iter().find(|i| i.name == port).ok_or_else(|| {
                        format!("node '{id}': '{}' has no input '{port}'", s.name)
                    })?;
                let r = refs(v)?;
                if !i.multi && r.len() > 1 {
                    return Err(format!("input '{port}' takes one connection").into());
                }
                for src in r {
                    node(g, &src)?;
                }
            }
        }
    }
    for d in crate::display_nodes(g) {
        node(g, &d)?;
    }
    if let Some(exposed) = g.get("exposed") {
        let mut names = HashSet::new();
        for e in exposed.as_array().ok_or("exposed must be a list")? {
            let id = string(e, "node")?;
            let name = string(e, "name")?;
            let param = string(e, "param")?;
            if !names.insert(name) {
                return Err(format!("duplicate exposed name '{name}'").into());
            }
            let s = spec(g, id)?;
            let p = s
                .params
                .iter()
                .find(|p| p.name == param)
                .ok_or("unknown exposed parameter")?;
            if !p.supported
                || !matches!(
                    param_kind(&(p.default)()),
                    ParamKind::Bool | ParamKind::Integer | ParamKind::Float | ParamKind::Vec3
                )
            {
                return Err(
                    "exposed parameter is not available as a Blender socket in the Rust core"
                        .into(),
                );
            }
        }
    }
    order(g)?;
    Document::parse(g.to_string())?;
    Ok(())
}
fn layout(g: &mut Value, all: bool) -> Result<()> {
    let mut depth = HashMap::<String, usize>::new();
    let mut rows = HashMap::<usize, usize>::new();
    for id in order(g)? {
        let srcs = g["nodes"][&id]["inputs"]
            .as_object()
            .into_iter()
            .flat_map(|m| m.values())
            .map(refs)
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        let d = srcs
            .iter()
            .filter_map(|s| depth.get(s))
            .max()
            .map_or(0, |d| d + 1);
        depth.insert(id.clone(), d);
        let col = rows.entry(d).or_default();
        if all || g["nodes"][&id].get("pos").is_none() {
            g["nodes"][&id]["pos"] = json!([*col as f64 * 190., d as f64 * 110.]);
        }
        *col += 1;
    }
    Ok(())
}
fn connect(
    g: &mut Value,
    src: &str,
    dst: &str,
    port: Option<&str>,
    index: Option<i64>,
) -> Result<String> {
    node(g, src)?;
    let s = spec(g, dst)?;
    let p = port
        .or_else(|| {
            s.inputs
                .iter()
                .find(|i| {
                    g["nodes"][dst]["inputs"]
                        .get(i.name)
                        .is_none_or(|v| v.is_null() || v.as_array().is_some_and(Vec::is_empty))
                })
                .map(|i| i.name)
        })
        .or_else(|| s.inputs.first().map(|i| i.name))
        .ok_or("node has no inputs")?;
    let i = s
        .inputs
        .iter()
        .find(|i| i.name == p)
        .ok_or_else(|| format!("node '{dst}' has no input '{p}'"))?;
    let v = if i.multi {
        let mut r = refs(&g["nodes"][dst]["inputs"][p])?;
        r.retain(|r| r != src);
        r.insert(
            index.unwrap_or(r.len() as i64).clamp(0, r.len() as i64) as usize,
            src.into(),
        );
        json!(r)
    } else {
        json!(src)
    };
    ensure_object(&mut g["nodes"][dst]["inputs"]).insert(p.into(), v);
    order(g)?;
    Ok(p.into())
}
fn rewire(g: &mut Value, old: &str, new: Option<&str>) -> usize {
    let mut count = 0;
    for n in g["nodes"].as_object_mut().unwrap().values_mut() {
        if let Some(ins) = n.get_mut("inputs").and_then(Value::as_object_mut) {
            ins.retain(|_, v| {
                if v.as_str() == Some(old) {
                    count += 1;
                    if let Some(new) = new {
                        *v = json!(new);
                        true
                    } else {
                        false
                    }
                } else if let Some(a) = v.as_array_mut() {
                    a.retain_mut(|r| {
                        if r.as_str() == Some(old) {
                            count += 1;
                            if let Some(new) = new {
                                *r = json!(new);
                                true
                            } else {
                                false
                            }
                        } else {
                            true
                        }
                    });
                    !a.is_empty()
                } else {
                    true
                }
            });
        }
    }
    count
}
pub fn apply_ops(original: &Value, ops: &Value) -> Result<(Value, Vec<Value>)> {
    let ops = ops.as_array().ok_or("ops must be a list")?;
    let mut g = original.clone();
    let mut results = vec![];
    for (i, op) in ops.iter().enumerate() {
        let result: Result<Value> = (|| {
            let kind = string(op, "op")?;
            let out = apply_one(&mut g, op, kind)?;
            validate(&g)?;
            Ok(out)
        })();
        match result {
            Ok(r) => results.push(r),
            Err(e) => {
                return Err(format!("op {i} ({}): {e}", op["op"].as_str().unwrap_or("?")).into())
            }
        }
    }
    layout(&mut g, false)?;
    Ok((g, results))
}
fn apply_one(g: &mut Value, op: &Value, kind: &str) -> Result<Value> {
    match kind {
        "add" | "insert" => {
            let ty = string(op, "type")?;
            if nodes::get(ty).is_none() {
                return Err(format!("node type '{ty}' is not in the Rust backend yet").into());
            }
            let id = op["id"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| fresh(g, ty));
            if !valid_id(&id) {
                return Err("invalid node id".into());
            }
            if g["nodes"].get(&id).is_some() {
                return Err(format!("duplicate node id '{id}'").into());
            }
            let mut n = json!({"type":ty});
            for k in ["params", "pos", "comment"] {
                if let Some(v) = op.get(k).filter(|v| !v.is_null()) {
                    n[k] = if k == "pos" { pos(v)? } else { v.clone() };
                }
            }
            let after = if kind == "insert" {
                let a = string(op, "after")?;
                node(g, a)?;
                Some(a.to_owned())
            } else {
                None
            };
            let rewired = after.as_ref().map_or(0, |a| rewire(g, a, Some(&id)));
            g["nodes"]
                .as_object_mut()
                .ok_or("nodes must be an object")?
                .insert(id.clone(), n);
            if let Some(ins) = op.get("inputs").filter(|v| !v.is_null()) {
                for (port, v) in object(ins)? {
                    for src in refs(v)? {
                        connect(g, &src, &id, Some(port), None)?;
                    }
                }
            }
            if let Some(a) = &after {
                connect(g, a, &id, op["port"].as_str(), None)?;
            }
            if op["display"] == true || after.as_ref().is_some_and(|a| g["display"] == a.as_str()) {
                g["display"] = json!(id);
            }
            Ok(if kind == "insert" {
                json!({"id":id,"rewired":rewired})
            } else {
                json!({"id":id})
            })
        }
        "remove" => {
            let mut removed = ids(&op["id"])?;
            let mut seen = HashSet::new();
            removed.retain(|id| seen.insert(id.clone()));
            removed.extend(
                object(&g["nodes"])?
                    .iter()
                    .filter(|(_, n)| {
                        n["type"] == "object_input"
                            && n["input_owner"]
                                .as_str()
                                .is_some_and(|id| removed.contains(&id.to_owned()))
                    })
                    .map(|(id, _)| id.clone())
                    .collect::<Vec<_>>(),
            );
            for id in &removed {
                node(g, id)?;
                rewire(g, id, None);
                g["nodes"].as_object_mut().unwrap().remove(id);
                entries(g).retain(|e| e["node"] != id.as_str());
            }
            let d = crate::display_nodes(g)
                .into_iter()
                .filter(|d| !removed.contains(d))
                .collect::<Vec<_>>();
            g["display"] = match d.len() {
                0 => Value::Null,
                1 => json!(d[0]),
                _ => json!(d),
            };
            Ok(json!({"removed":removed}))
        }
        "rename" => {
            let old = string(op, "id")?;
            let new = string(op, "to")?;
            node(g, old)?;
            if !valid_id(new) {
                return Err("invalid node id".into());
            }
            if old != new && g["nodes"].get(new).is_some() {
                return Err("duplicate node id".into());
            }
            // Preserve file order (the first output is significant).
            let before = g["nodes"].as_object().unwrap().clone();
            g["nodes"] = Value::Object(
                before
                    .into_iter()
                    .map(|(id, n)| (if id == old { new.into() } else { id }, n))
                    .collect(),
            );
            rewire(g, old, Some(new));
            if g["display"] == old {
                g["display"] = json!(new);
            } else if let Some(a) = g["display"].as_array_mut() {
                for v in a {
                    if v == old {
                        *v = json!(new);
                    }
                }
            }
            for e in entries(g) {
                if e["node"] == old {
                    e["node"] = json!(new);
                }
            }
            for n in g["nodes"].as_object_mut().unwrap().values_mut() {
                if n["input_owner"] == old {
                    n["input_owner"] = json!(new);
                }
            }
            Ok(json!({"id":new}))
        }
        "connect" => {
            let p = connect(
                g,
                string(op, "from")?,
                string(op, "to")?,
                op["port"].as_str(),
                op["index"].as_i64(),
            )?;
            Ok(json!({"port":p}))
        }
        "disconnect" => {
            let dst = string(op, "to")?;
            let s = spec(g, dst)?;
            if let Some(p) = op["port"].as_str() {
                if !s.inputs.iter().any(|i| i.name == p) {
                    return Err("unknown input port".into());
                }
            }
            if let Some(ins) = g["nodes"][dst]
                .get_mut("inputs")
                .and_then(Value::as_object_mut)
            {
                ins.retain(|p, v| {
                    if op["port"].as_str().is_some_and(|port| port != p) {
                        return true;
                    }
                    if let Some(src) = op["from"].as_str() {
                        if let Some(a) = v.as_array_mut() {
                            a.retain(|r| r != src);
                            !a.is_empty()
                        } else {
                            v != src
                        }
                    } else {
                        false
                    }
                });
            }
            Ok(json!({}))
        }
        "set" => {
            let id = string(op, "id")?;
            let s = spec(g, id)?;
            let mut out = Map::new();
            for (name, v) in object(&op["params"])? {
                let p = s
                    .params
                    .iter()
                    .find(|p| p.name == name)
                    .ok_or_else(|| format!("unknown parameter '{name}'"))?;
                let value = if v.is_null() {
                    (p.default)()
                } else {
                    v.clone()
                };
                validate_param(s.name, name, &value)?;
                let params = ensure_object(&mut g["nodes"][id]["params"]);
                if v.is_null() {
                    params.remove(name);
                } else {
                    params.insert(name.clone(), value.clone());
                }
                out.insert(name.clone(), value);
            }
            Ok(json!({"params":out}))
        }
        "display" => {
            let id = string(op, "id")?;
            node(g, id)?;
            g["display"] = json!(id);
            for n in g["nodes"].as_object_mut().unwrap().values_mut() {
                n.as_object_mut().unwrap().remove("display_extra");
            }
            Ok(json!({}))
        }
        "expose" => {
            let id = string(op, "id")?;
            let param = string(op, "param")?;
            let s = spec(g, id)?;
            let p = s
                .params
                .iter()
                .find(|p| p.name == param)
                .ok_or("unknown parameter")?;
            if !p.supported {
                return Err("parameter is not supported by the Rust backend".into());
            }
            if !matches!(
                param_kind(&(p.default)()),
                ParamKind::Bool | ParamKind::Integer | ParamKind::Float | ParamKind::Vec3
            ) {
                return Err("Blender can take float/int/bool/vec3 parameters".into());
            }
            let base = param_meta(s.name, param)
                .and_then(|m| m["label"].as_str())
                .map(str::to_owned)
                .unwrap_or_else(|| {
                    param
                        .replace('_', " ")
                        .split_whitespace()
                        .map(|s| {
                            let mut c = s.chars();
                            c.next().unwrap().to_uppercase().to_string() + c.as_str()
                        })
                        .collect::<Vec<_>>()
                        .join(" ")
                });
            let list = entries(g);
            let existing = list
                .iter()
                .position(|e| e["node"] == id && e["param"] == param);
            let mut name = op["name"]
                .as_str()
                .map(str::to_owned)
                .or_else(|| existing.and_then(|i| list[i]["name"].as_str().map(str::to_owned)))
                .unwrap_or(base.clone());
            let taken = |name: &str| {
                list.iter()
                    .enumerate()
                    .any(|(i, e)| Some(i) != existing && e["name"] == name)
            };
            if taken(&name) {
                if op.get("name").is_some() {
                    return Err("exposed name is already used".into());
                }
                name = format!("{id} {base}");
                let mut i = 2;
                while taken(&name) {
                    name = format!("{id} {base} {i}");
                    i += 1;
                }
            }
            let e = json!({"node":id,"param":param,"name":name,"label":op["label"].as_str().unwrap_or("")});
            if let Some(i) = existing {
                let label = list[i]["label"].clone();
                list[i] = e;
                if op.get("label").is_none() {
                    list[i]["label"] = label;
                }
            } else {
                list.push(e);
            }
            Ok(json!({"name":name}))
        }
        "unexpose" => {
            let list = entries(g);
            let before = list.len();
            list.retain(|e| {
                if op.get("name").is_some() {
                    e["name"] != op["name"]
                } else {
                    e["node"] != op["id"] || e["param"] != op["param"]
                }
            });
            if before == list.len() {
                return Err("no such exposed parameter".into());
            }
            Ok(json!({}))
        }
        "move" => {
            let positions = if let Some(p) = op.get("positions") {
                p.clone()
            } else {
                let mut m = Map::new();
                m.insert(string(op, "id")?.into(), op["pos"].clone());
                Value::Object(m)
            };
            for (id, p) in object(&positions)? {
                node(g, id)?;
                g["nodes"][id]["pos"] = pos(p)?;
            }
            Ok(json!({}))
        }
        "meta" => {
            let id = string(op, "id")?;
            node(g, id)?;
            for (k, v) in object(&op["set"])? {
                if ["type", "params", "inputs"].contains(&k.as_str()) {
                    return Err(format!("meta key '{k}' is reserved").into());
                }
                let n = g["nodes"][id].as_object_mut().unwrap();
                if v.is_null() {
                    n.remove(k);
                } else {
                    n.insert(k.clone(), v.clone());
                }
            }
            Ok(json!({}))
        }
        "replace" => {
            *g = op["graph"].clone();
            Ok(json!({}))
        }
        "layout" => {
            layout(g, true)?;
            Ok(json!({}))
        }
        "loop" => {
            let begin = fresh(g, "loop_begin");
            apply_one(g, &json!({"type":"loop_begin","id":begin}), "add")?;
            if let Some(src) = op["from"].as_str() {
                connect(g, src, &begin, None, None)?;
            }
            let body = fresh(g, "transform");
            apply_one(
                g,
                &json!({"type":"transform","id":body,"inputs":{"geo":begin}}),
                "add",
            )?;
            let iterations = op.get("iterations").cloned().unwrap_or(json!(3));
            let end = fresh(g, "loop_end");
            apply_one(
                g,
                &json!({"type":"loop_end","id":end,"inputs":{"geo":body,"begin":begin},"params":{"iterations":iterations}}),
                "add",
            )?;
            let p = pos(op.get("pos").unwrap_or(&json!([0, 0])))?;
            let x = p[0].as_f64().unwrap();
            let y = p[1].as_f64().unwrap();
            for (id, dy) in [(&begin, 0.), (&body, 110.), (&end, 220.)] {
                g["nodes"][id]["pos"] = json!([x, y + dy]);
            }
            if let Some(dst) = op["to"].as_str() {
                connect(g, &end, dst, op["port"].as_str(), None)?;
            }
            Ok(json!({"begin":begin,"body":body,"end":end,"id":end}))
        }
        "box_add" | "box_set" | "box_remove" => {
            if !g["boxes"].is_array() {
                g["boxes"] = json!([]);
            }
            if kind == "box_remove" {
                for id in ids(&op["id"])? {
                    let list = g["boxes"].as_array_mut().unwrap();
                    let i = list
                        .iter()
                        .position(|b| b["id"] == id)
                        .ok_or("no such box")?;
                    list.remove(i);
                }
                return Ok(json!({}));
            }
            let id = op["id"].as_str().map(str::to_owned).unwrap_or_else(|| {
                (1..)
                    .map(|i| format!("box{i}"))
                    .find(|id| {
                        !g["boxes"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|b| b["id"] == id.as_str())
                    })
                    .unwrap()
            });
            let index = g["boxes"]
                .as_array()
                .unwrap()
                .iter()
                .position(|b| b["id"] == id);
            let mut b = if kind == "box_set" {
                g["boxes"][index.ok_or("no such box")?].clone()
            } else {
                if index.is_some() {
                    return Err("duplicate box id".into());
                }
                json!({"id":id,"title":"","color":"","pos":[0,0],"size":[200,160]})
            };
            for k in [
                "title",
                "color",
                "pos",
                "size",
                "collapsed",
                "locked",
                "note",
                "text",
            ] {
                if let Some(v) = op.get(k) {
                    b[k] = if k == "pos" || k == "size" {
                        pos(v)?
                    } else {
                        v.clone()
                    };
                }
            }
            if let Some(around) = op.get("around") {
                layout(g, false)?;
                let pts = ids(around)?
                    .iter()
                    .map(|id| node(g, id).map(|n| n["pos"].clone()))
                    .collect::<Result<Vec<_>>>()?;
                if !pts.is_empty() {
                    let x = pts
                        .iter()
                        .map(|p| p[0].as_f64().unwrap_or(0.))
                        .fold(f64::INFINITY, f64::min)
                        - 30.;
                    let y = pts
                        .iter()
                        .map(|p| p[1].as_f64().unwrap_or(0.))
                        .fold(f64::INFINITY, f64::min)
                        - 58.;
                    let mx = pts
                        .iter()
                        .map(|p| p[0].as_f64().unwrap_or(0.))
                        .fold(f64::NEG_INFINITY, f64::max)
                        + 190.;
                    let my = pts
                        .iter()
                        .map(|p| p[1].as_f64().unwrap_or(0.))
                        .fold(f64::NEG_INFINITY, f64::max)
                        + 76.;
                    b["pos"] = json!([x, y]);
                    b["size"] = json!([mx - x, my - y]);
                }
            }
            clean_box(&mut b)?;
            if let Some(i) = index {
                g["boxes"][i] = b;
            } else {
                g["boxes"].as_array_mut().unwrap().push(b);
            }
            Ok(if kind == "box_add" {
                json!({"id":id})
            } else {
                json!({})
            })
        }
        "paint_assign" => Err("attrib_paint is not in the Rust backend yet".into()),
        _ => Err(format!("unknown op '{kind}'").into()),
    }
}
