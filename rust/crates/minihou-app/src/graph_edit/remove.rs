use super::*;

pub fn delete_measurement(value:&Value,index:usize)->Option<Value> {
    let mut next=value.clone();
    let rows=next.get_mut("measurements")?.as_array_mut()?;
    if index>=rows.len() {return None;}
    rows.remove(index);Some(next)
}
pub fn clear_measurements(value:&Value)->Option<Value> {
    let mut next=value.clone();
    let rows=next.get_mut("measurements")?.as_array_mut()?;
    if rows.is_empty() {return None;}
    rows.clear();Some(next)
}

pub fn delete(value: &Value, id: &str) -> Option<Value> {
    let mut value = value.clone();
    let all = nodes_mut(&mut value)?;
    all.remove(id)?;
    for node in all.values_mut() {
        let Some(inputs) = node.get_mut("inputs").and_then(Value::as_object_mut) else { continue };
        inputs.retain(|_, source| match source {
            Value::String(s) => s != id,
            Value::Array(a) => { a.retain(|x| x.as_str() != Some(id)); !a.is_empty() }
            _ => true,
        });
    }
    let first = all.keys().next().cloned();
    let mut displayed = minihou_app::display_nodes(&value);
    displayed.retain(|item| item != id);
    if displayed.is_empty() { if let Some(first) = first { displayed.push(first); } }
    if displayed.is_empty() { value.as_object_mut()?.remove("display"); }
    else { value["display"] = if displayed.len() == 1 { json!(displayed[0]) } else { json!(displayed) }; }
    Some(value)
}

pub fn delete_set(value: &Value, ids: &[String]) -> Option<Value> {
    let mut next = value.clone();
    let mut changed = false;
    for id in ids {
        if let Some(deleted) = delete(&next, id) { next = deleted; changed = true; }
    }
    changed.then_some(next)
}

/// Remove every incoming and outgoing wire of a node without removing the card.
pub fn disconnect_all(value: &Value, id: &str) -> Option<Value> {
    nodes(value)?.get(id)?;
    let mut next = value.clone();
    nodes_mut(&mut next)?.get_mut(id)?.as_object_mut()?.remove("inputs");
    for node in nodes_mut(&mut next)?.values_mut() {
        if let Some(inputs) = node.get_mut("inputs").and_then(Value::as_object_mut) {
            inputs.retain(|_, source| match source {
                Value::String(s) => s != id,
                Value::Array(a) => { a.retain(|item| item.as_str() != Some(id)); !a.is_empty() },
                _ => true,
            });
        }
    }
    Some(next)
}

/// Bypass the node using dissolve's rewiring, then restore its detached card and parameters.
pub fn detach(value: &Value, id: &str) -> Option<Value> {
    let mut node = nodes(value)?.get(id)?.clone();
    node.as_object_mut()?.remove("inputs");
    let mut next = dissolve_set(value, &[id.to_owned()])?;
    nodes_mut(&mut next)?.insert(id.to_owned(), node);
    Some(next)
}

/// Dissolve nodes in order, replacing each use with its first input source.
/// `connect` supplies the same target-port validity check as an ordinary wire.
pub fn dissolve_set(value: &Value, ids: &[String]) -> Option<Value> {
    let mut next = value.clone();
    let mut changed = false;
    for id in ids {
        let Some(node) = nodes(&next)?.get(id) else { continue };
        let source = node.get("inputs").and_then(Value::as_object)
            .and_then(|inputs| {
                let ty = node.get("type")?.as_str()?;
                let spec = nodes::get(ty)?;
                spec.inputs.iter().find_map(|port| inputs.get(port.name)).or_else(|| inputs.values().next())
            })
            .and_then(|input| input.as_str().or_else(|| input.as_array()?.iter().find_map(Value::as_str)))
            .map(str::to_owned);
        let consumers: Vec<(String, String)> = nodes(&next)?.iter().filter(|(target, _)| *target != id)
            .flat_map(|(target, node)| node.get("inputs").and_then(Value::as_object).into_iter()
                .flat_map(move |inputs| inputs.iter().filter(move |(_, v)| v.as_str() == Some(id)
                    || v.as_array().is_some_and(|a| a.iter().any(|x| x.as_str() == Some(id))))
                    .map(move |(port, _)| (target.clone(), port.clone()))))
            .collect();
        if let Some(source) = source.as_deref() {
            for (target, port) in consumers {
                if connect(&next, source, &target, Some(&port)).is_some() {
                    let input = nodes_mut(&mut next)?.get_mut(&target)?.get_mut("inputs")?.get_mut(&port)?;
                    match input {
                        Value::String(s) if s == id => *s = source.to_owned(),
                        Value::Array(items) => {
                            for item in items.iter_mut() { if item.as_str() == Some(id) { *item = json!(source); } }
                            let mut seen = HashSet::new();
                            items.retain(|item| item.as_str().is_some_and(|s| seen.insert(s.to_owned())));
                        },
                        _ => {},
                    }
                }
            }
        }
        let displayed = minihou_app::display_nodes(&next);
        next = delete(&next, id)?;
        if displayed.iter().any(|shown| shown == id) && source.as_deref().is_some_and(|s| nodes(&next).is_some_and(|n| n.contains_key(s))) {
            let mut replacement = displayed;
            for shown in &mut replacement { if shown == id { *shown = source.clone().unwrap(); } }
            replacement.retain(|shown| nodes(&next).is_some_and(|n| n.contains_key(shown)));
            let mut seen = HashSet::new();
            replacement.retain(|shown| seen.insert(shown.clone()));
            next["display"] = if replacement.len() == 1 { json!(replacement[0]) } else { json!(replacement) };
        }
        changed = true;
    }
    changed.then_some(next)
}

#[cfg(test)]
pub fn dissolve(value: &Value, id: &str) -> Option<Value> {
    dissolve_set(value, &[id.to_owned()])
}
#[cfg(test)]
pub fn duplicate(value: &Value, id: &str) -> Option<(Value, String)> {
    let mut value = value.clone();
    let all = nodes_mut(&mut value)?;
    let src = all.get(id)?.clone();
    let ty = src.get("type").and_then(Value::as_str).unwrap_or("node");
    let new_id = fresh_id(all, ty);
    all.insert(new_id.clone(), src);
    Some((value, new_id))
}

