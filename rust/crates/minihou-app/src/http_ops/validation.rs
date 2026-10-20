use super::*;
pub(super) fn string<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v[key]
        .as_str()
        .ok_or_else(|| format!("{key} must be a string").into())
}
pub(super) fn object(v: &Value) -> Result<&Map<String, Value>> {
    v.as_object().ok_or_else(|| "expected an object".into())
}
pub(super) fn node<'a>(g: &'a Value, id: &str) -> Result<&'a Value> {
    g["nodes"]
        .get(id)
        .ok_or_else(|| format!("no such node '{id}'").into())
}
pub(super) fn spec(g: &Value, id: &str) -> Result<&'static nodes::NodeSpec> {
    let ty = string(node(g, id)?, "type")?;
    nodes::get(ty).ok_or_else(|| {
        format!("node '{id}': node type '{ty}' is not in the Rust backend yet").into()
    })
}
pub(super) fn refs(v: &Value) -> Result<Vec<String>> {
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
pub(super) fn ids(v: &Value) -> Result<Vec<String>> {
    if !v.is_string() && !v.is_array() {
        return Err("id must be a string or list of strings".into());
    }
    refs(v)
}
pub(super) fn fresh(g: &Value, ty: &str) -> String {
    (1..)
        .map(|n| format!("{ty}{n}"))
        .find(|id| g["nodes"].get(id).is_none())
        .unwrap()
}
pub(super) fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .enumerate()
            .all(|(i, c)| c == '_' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit()))
}
pub(super) fn pos(v: &Value) -> Result<Value> {
    let a = v
        .as_array()
        .filter(|a| a.len() == 2 && a.iter().all(Value::is_number))
        .ok_or("pos must be [x, y]")?;
    Ok(json!(a
        .iter()
        .map(|x| (x.as_f64().unwrap() * 10.).round() / 10.)
        .collect::<Vec<_>>()))
}
pub(super) fn ensure_object(v: &mut Value) -> &mut Map<String, Value> {
    if !v.is_object() {
        *v = json!({});
    }
    v.as_object_mut().unwrap()
}
pub(super) fn entries(g: &mut Value) -> &mut Vec<Value> {
    if !g["exposed"].is_array() {
        g["exposed"] = json!([]);
    }
    g["exposed"].as_array_mut().unwrap()
}
pub(super) fn clean_box(b: &mut Value) -> Result<()> {
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
pub(super) fn validate_param(ty: &str, name: &str, value: &Value) -> Result<()> {
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
pub(super) fn order(g: &Value) -> Result<Vec<String>> {
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
