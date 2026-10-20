use super::*;
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
        "paint_assign" => {
            let id = string(op, "id")?;
            if string(node(g, id)?, "type")? != "attrib_paint" { return Err("paint_assign needs an attrib_paint node".into()); }
            let s = nodes::get("attrib_paint").unwrap();
            let current = node(g, id)?["params"].as_object();
            let p = Params::resolve(s.params, current)?;
            let kind = p.e("kind", &["float", "int", "vec3"])?;
            let width = if kind == "vec3" { 3 } else { 1 };
            let mut data = if op["clear"].as_bool() == Some(true) { String::new() } else { p.s("data")? };
            if let Some(indices) = op.get("indices") {
                let indices = indices.as_array().ok_or("indices must be a list of whole numbers")?
                    .iter().map(|v| v.as_i64().filter(|&n| n >= 0 && n <= i32::MAX as i64)
                        .map(|n| n as i32).ok_or_else(|| "indices must be nonnegative whole numbers".into()))
                    .collect::<Result<Vec<i32>>>()?;
                if op["erase"].as_bool() == Some(true) {
                    data = nodes::attrib_paint::merge(&data, width, &indices, None).map_err(|e| e.to_string())?;
                } else if !indices.is_empty() {
                    let value_rows = if let Some(v) = op.get("values") {
                        let a = v.as_array().ok_or("values must be a list")?;
                        if width == 1 { a.iter().map(|v| vec![v.as_f64().or_else(|| v.as_array().filter(|a| a.len() == 1).and_then(|a| a[0].as_f64())).unwrap_or(f64::NAN) as f32]).collect::<Vec<_>>() }
                        else if a.iter().all(Value::is_number) {
                            a.chunks(width).map(|row| row.iter().map(|x| x.as_f64().unwrap_or(f64::NAN) as f32).collect()).collect()
                        } else { a.iter().map(|v| v.as_array().map(|v| v.iter().map(|x| x.as_f64().unwrap_or(f64::NAN) as f32).collect()).unwrap_or_default()).collect() }
                    } else {
                        let v = op.get("value").ok_or("give value or values (or erase)")?;
                        let row = if width == 1 { vec![v.as_f64().unwrap_or(f64::NAN) as f32] }
                                  else { v.as_array().ok_or("value must hold 3 numbers")?.iter().map(|x| x.as_f64().unwrap_or(f64::NAN) as f32).collect() };
                        vec![row; indices.len()]
                    };
                    let value_rows: Vec<Vec<f32>> = if kind == "int" { value_rows.into_iter().map(|r| r.into_iter().map(f32::round_ties_even).collect()).collect() } else { value_rows };
                    data = nodes::attrib_paint::merge(&data, width, &indices, Some(&value_rows)).map_err(|e| e.to_string())?;
                }
            }
            let default = if let Some(v) = op.get("default") {
                let name = if width == 3 { "default_vec" } else { "default" };
                validate_param("attrib_paint", name, v)?;
                Some((name, v.clone()))
            } else { None };
            let params = ensure_object(&mut g["nodes"][id]["params"]);
            params.insert("data".into(), json!(data));
            if let Some((name, v)) = default { params.insert(name.into(), v); }
            Ok(json!({"edited":true,"bytes":data.len()}))
        },
        _ => Err(format!("unknown op '{kind}'").into()),
    }
}
