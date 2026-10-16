//! Pure document changes used by the network editor.
use minihou_app::Document;
use minihou_core::nodes;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};

pub fn validated(value: Value) -> Result<Document, String> {
    let text = serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?;
    Document::parse(text).map_err(|e| e.to_string())
}

fn nodes(value: &Value) -> Option<&Map<String, Value>> { value.get("nodes")?.as_object() }
fn nodes_mut(value: &mut Value) -> Option<&mut Map<String, Value>> { value.get_mut("nodes")?.as_object_mut() }

fn fresh_id(nodes: &Map<String, Value>, ty: &str) -> String {
    (1..).map(|n| format!("{ty}{n}")).find(|id| !nodes.contains_key(id)).unwrap()
}
pub fn set_display(value: &Value, id: &str, additive: bool) -> Option<Value> {
    nodes(value)?.get(id)?;
    let mut next = value.clone();
    let mut displayed = minihou_app::display_nodes(value);
    if additive {
        if displayed.iter().any(|node| node == id) {
            if displayed.len() > 1 { displayed.retain(|node| node != id); }
        } else { displayed.push(id.to_owned()); }
    } else { displayed = vec![id.to_owned()]; }
    if displayed.is_empty() { next.as_object_mut()?.remove("display"); }
    else { next["display"] = if displayed.len() == 1 { json!(displayed[0]) } else { json!(displayed) }; }
    Some(next)
}

/// The fixed end of a noodle used to add a connected node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AddConnection {
    Output(String),
    Input { node: String, port: String },
}

/// One atomic add/connect/position edit. Failed connections never leave a loose node.
pub fn add_connected(value: &Value, ty: &str, connection: &AddConnection, pos: [f32; 2]) -> Option<(Value, String)> {
    let spec = nodes::get(ty)?;
    if !pos.iter().all(|v| v.is_finite()) { return None; }
    let (mut next, id) = match connection {
        AddConnection::Output(source) => {
            nodes(value)?.get(source)?;
            spec.inputs.first()?;
            // Delegate terminal output insertion to add; never wire an output downstream.
            let (next, id) = add_impl(value, ty, Some(source), false)?;
            let terminal = value["nodes"][source]["type"] == "output";
            let next = if terminal { next } else { connect(&next, source, &id, None)? };
            (next, id)
        }
        AddConnection::Input { node, port } => {
            if ty == "output" { return None; }
            let (next, id) = add(value, ty, None)?;
            let mut next = connect(&next, &id, node, Some(port))?;
            // Inserting upstream should keep the existing stream displayed.
            if let Some(display) = value.get("display") { next["display"] = display.clone(); }
            else { next.as_object_mut()?.remove("display"); }
            (next, id)
        }
    };
    next["nodes"][&id]["pos"] = json!(pos);
    Some((next, id))
}

pub fn add(value: &Value, ty: &str, after: Option<&str>) -> Option<(Value, String)> {
    add_impl(value, ty, after, true)
}

fn add_impl(value: &Value, ty: &str, after: Option<&str>, splice: bool) -> Option<(Value, String)> {
    let spec = nodes::get(ty)?;
    let mut value = value.clone();
    let displayed = minihou_app::display_nodes(&value);
    let all = nodes_mut(&mut value)?;
    let id = fresh_id(all, ty);
    let mut node = json!({"type": ty});
    let source = after.filter(|id| all.contains_key(*id));
    let mut spliced = false;
    // 'Add after' an output node inserts just before it: output stays the end of the stream
    if let (Some(out_id), Some(port)) = (source, spec.inputs.first()) {
        let is_output = all.get(out_id).and_then(|n| n.get("type")).and_then(Value::as_str) == Some("output");
        let out_port = nodes::get("output").and_then(|o| o.inputs.first());
        if let (true, Some(out_port)) = (is_output, out_port) {
            let upstream = match all[out_id].get("inputs").and_then(|i| i.get(out_port.name)) {
                Some(Value::String(s)) => Some(s.clone()),
                Some(Value::Array(a)) => a.first().and_then(Value::as_str).map(str::to_owned),
                _ => None,
            };
            if let Some(up) = upstream {
                node["inputs"] = if port.multi { json!({port.name: [up]}) } else { json!({port.name: up}) };
            }
            all.insert(id.clone(), node);
            all[out_id]["inputs"] = if out_port.multi { json!({out_port.name: [id.clone()]}) } else { json!({out_port.name: id.clone()}) };
            return Some((value, id));
        }
    }
    if let (Some(after), Some(port)) = (source, spec.inputs.first().filter(|p| !p.optional)) {
        node["inputs"] = if port.multi { json!({port.name: [after]}) } else { json!({port.name: after}) };
        if splice && ty != "output" {
            // Replace every occurrence, including references in multi-input ports.
            // Explicit noodle adds use add_impl without splicing existing wires.
            for consumer in all.values_mut() {
                if let Some(inputs) = consumer.get_mut("inputs").and_then(Value::as_object_mut) {
                    for input in inputs.values_mut() {
                        if input.as_str() == Some(after) { *input = json!(id); spliced = true; }
                        else if let Some(refs) = input.as_array_mut() {
                            for source in refs {
                                if source.as_str() == Some(after) { *source = json!(id); spliced = true; }
                            }
                        }
                    }
                }
            }
        }
    }
    all.insert(id.clone(), node);
    let mut next = displayed;
    if spec.inputs.iter().all(|port| port.optional) {
        next.push(id.clone());
    } else if let Some(source) = source {
        if let Some(index) = next.iter().position(|item| item == source) { next[index] = id.clone(); }
        else if !spliced { next = vec![id.clone()]; }
    } else { next = vec![id.clone()]; }
    value["display"] = if next.len() == 1 { json!(next[0]) } else { json!(next) };
    Some((value, id))
}

/// Adds a node placed at world position `pos`. If `ty` is a primitive with a `center` parameter,
/// sets `center` to `pos`. If `ty` is a primitive without `center`, adds the node and then adds a
/// `transform` node after it with `translate` set to `pos`. For non-primitives, behaves like `add`.
pub fn add_at(value: &Value, ty: &str, after: Option<&str>, pos: [f32; 3]) -> Option<(Value, String)> {
    if !crate::picking::is_primitive_node(ty) {
        return add(value, ty, after);
    }
    if crate::picking::node_has_center_param(ty) {
        let (mut val, id) = add(value, ty, after)?;
        let all = nodes_mut(&mut val)?;
        let node = all.get_mut(&id)?;
        if let Some(params) = node.get_mut("params").and_then(|p| p.as_object_mut()) {
            params.insert("center".into(), json!([pos[0], pos[1], pos[2]]));
        } else {
            node["params"] = json!({"center": [pos[0], pos[1], pos[2]]});
        }
        Some((val, id))
    } else {
        let (val, id) = add(value, ty, after)?;
        let (mut val, xform_id) = add(&val, "transform", Some(&id))?;
        let all = nodes_mut(&mut val)?;
        let node = all.get_mut(&xform_id)?;
        if let Some(params) = node.get_mut("params").and_then(|p| p.as_object_mut()) {
            params.insert("translate".into(), json!([pos[0], pos[1], pos[2]]));
        } else {
            node["params"] = json!({"translate": [pos[0], pos[1], pos[2]]});
        }
        Some((val, xform_id))
    }
}

/// Add a sketch to an existing cooked node, or as a standalone curve when there is no usable source.
pub fn add_sketch(value: &Value, name: &str, points: &[[f32; 3]], closed: bool, selected: &str) -> Option<(Value, String)> {
    if points.len() < if closed { 3 } else { 2 } || points.iter().flatten().any(|x| !x.is_finite()) { return None; }
    let after = nodes(value)?.get(selected).and_then(|node| node.get("type")?.as_str())
        .filter(|ty| nodes::get(ty).is_some()).map(|_| selected);
    let (mut value, id) = add(value, "curve", after)?;
    if let Some(after) = after { value = connect(&value, after, &id, Some("geo"))?; }
    let text = points.iter().map(|p| format!("{},{},{}", p[0], p[1], p[2])).collect::<Vec<_>>().join(" ");
    let node = nodes_mut(&mut value)?.get_mut(&id)?;
    node["params"] = json!({"points": text, "closed": closed});
    node["name"] = json!(name);
    Some((value, id))
}

/// Viewport topology edits use the displayed stream, never an unrelated selected node.
pub fn add_viewport_cut(value: &Value, source: &str, ty: &str, params: Value) -> Option<(Value, String)> {
    if !matches!(ty, "knife" | "loop_cut") || !params.is_object() { return None; }
    if minihou_app::display_nodes(value) != vec![source.to_owned()] { return None; }
    nodes(value)?.get(source)?;
    let (mut next, id) = add(value, ty, Some(source))?;
    next["nodes"][&id]["params"] = params;
    validated(next.clone()).ok()?;
    Some((next, id))
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

pub fn connect(value: &Value, from: &str, to: &str, port: Option<&str>) -> Option<Value> {
    if from == to { return None; }
    let all = nodes(value)?;
    if !all.contains_key(from) { return None; }
    let ty = all.get(to)?.get("type")?.as_str()?;
    let spec = nodes::get(ty)?;
    let input = match port { Some(name) => spec.inputs.iter().find(|p| p.name == name)?, None => spec.inputs.first()? };
    let mut value = value.clone();
    let target = nodes_mut(&mut value)?.get_mut(to)?;
    if target.get("inputs").is_none() { target["inputs"] = json!({}); }
    let inputs = target.get_mut("inputs")?.as_object_mut()?;
    if input.multi {
        let list = inputs.entry(input.name).or_insert_with(|| json!([]));
        if let Some(a) = list.as_array_mut() { if !a.iter().any(|x| x.as_str() == Some(from)) { a.push(json!(from)); } }
        else { *list = json!([from]); }
    } else { inputs.insert(input.name.to_owned(), json!(from)); }
    Some(value)
}

/// Identity of one noodle, including its source within a multi-input port.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct WireRef { pub from: String, pub to: String, pub port: String }

/// Splice one exact edge, preserving multi-input order and terminal Output nodes.
/// Native ports all carry Geometry; the registry declares required/optional ports.
pub fn insert_existing(value: &Value, id: &str, wire: &WireRef) -> Option<Value> {
    let all = nodes(value)?;
    let node = all.get(id)?;
    if id == wire.from || id == wire.to || node["type"] == "output" || all.get(&wire.from)?["type"] == "output" { return None; }
    if node.get("inputs").and_then(Value::as_object).is_some_and(|inputs|
        inputs.values().any(|v| v.as_str().is_some() || v.as_array().is_some_and(|a| !a.is_empty()))) { return None; }
    nodes::get(all.get(&wire.from)?["type"].as_str()?)?;
    let spec = nodes::get(node["type"].as_str()?)?;
    let input = spec.inputs.iter().find(|p| !p.optional).or_else(|| spec.inputs.first())?;
    if spec.inputs.iter().any(|p| p.name != input.name && !p.optional) { return None; }
    let dest = nodes::get(all.get(&wire.to)?["type"].as_str()?)?;
    dest.inputs.iter().find(|p| p.name == wire.port)?;
    // Downstream wiring must not introduce a cycle through an existing consumer.
    let mut pending = vec![id];
    let mut seen = HashSet::new();
    while let Some(source) = pending.pop() {
        if source == wire.from { return None; }
        if !seen.insert(source) { continue; }
        for (target, def) in all {
            if def.get("inputs").and_then(Value::as_object).is_some_and(|i| i.values().any(|v|
                v.as_str() == Some(source) || v.as_array().is_some_and(|a| a.iter().any(|r| r.as_str() == Some(source))))) {
                pending.push(target.as_str());
            }
        }
    }
    let mut next = connect(value, &wire.from, id, Some(input.name))?;
    let edge = next.get_mut("nodes")?.get_mut(&wire.to)?.get_mut("inputs")?.get_mut(&wire.port)?;
    if let Some(refs) = edge.as_array_mut() {
        if refs.iter().any(|r| r.as_str() == Some(id)) { return None; }
        let index = refs.iter().position(|r| r.as_str() == Some(&wire.from))?;
        refs[index] = json!(id);
    } else if edge.as_str() == Some(&wire.from) { *edge = json!(id); }
    else { return None; }
    Some(next)
}

/// A pickup remains a UI preview until release, so cancellation needs no graph edit.
#[derive(Clone, Debug)]
pub struct WirePickup { pub original: WireRef }
impl WirePickup {
    pub fn begin(value: &Value, node: &str, port: &str) -> Option<Self> {
        let refs = value.get("nodes")?.get(node)?.get("inputs")?.get(port)?;
        let from = refs.as_str().or_else(|| refs.as_array()?.last()?.as_str())?;
        Some(Self { original: WireRef { from: from.into(), to: node.into(), port: port.into() } })
    }
    pub fn release(&self, value: &Value, target: Option<(&str, &str)>) -> Option<Value> {
        if target == Some((self.original.to.as_str(), self.original.port.as_str())) { return None; }
        let next = disconnect_wires(value, &std::collections::BTreeSet::from([self.original.clone()]))?;
        if let Some((node, port)) = target {
            if value["nodes"][&self.original.from]["type"] == "output" { return None; }
            let next = connect(&next, &self.original.from, node, Some(port))?;
            // Reject cycles, including those outside the displayed branch.
            let mut pending = vec![self.original.from.as_str()];
            let mut seen = HashSet::new();
            while let Some(id) = pending.pop() {
                if id == node { return None; }
                if !seen.insert(id) { continue; }
                if let Some(inputs) = next["nodes"][id]["inputs"].as_object() {
                    for refs in inputs.values() {
                        if let Some(source) = refs.as_str() { pending.push(source); }
                        else if let Some(refs) = refs.as_array() { pending.extend(refs.iter().filter_map(Value::as_str)); }
                    }
                }
            }
            Some(next)
        } else { Some(next) }
    }
}

/// Disconnect exact edges in one snapshot. Never dissolve or remove nodes.
pub fn disconnect_wires(value: &Value, wires: &std::collections::BTreeSet<WireRef>) -> Option<Value> {
    let mut next = value.clone();
    let mut changed = false;
    for wire in wires {
        let Some(inputs) = next.get_mut("nodes").and_then(|n| n.get_mut(&wire.to))
            .and_then(|n| n.get_mut("inputs")).and_then(Value::as_object_mut) else { continue };
        let Some(input) = inputs.get_mut(&wire.port) else { continue };
        if let Some(refs) = input.as_array_mut() {
            let before = refs.len();
            refs.retain(|r| r.as_str() != Some(wire.from.as_str()));
            changed |= before != refs.len();
            if refs.is_empty() { inputs.remove(&wire.port); }
        } else if input.as_str() == Some(wire.from.as_str()) {
            inputs.remove(&wire.port);
            changed = true;
        }
    }
    changed.then_some(next)
}

pub fn disconnect(value: &Value, node: &str, port: &str) -> Option<Value> {
    let mut value = value.clone();
    nodes_mut(&mut value)?.get_mut(node)?.get_mut("inputs")?.as_object_mut()?.remove(port)?;
    Some(value)
}

/// The clipboard contains node definitions. References outside the copied set are dropped.
pub fn copy(value: &Value, ids: &[String]) -> Option<Value> {
    let all = nodes(value)?;
    let selected: HashSet<_> = ids.iter().map(String::as_str).collect();
    let mut copied = Map::new();
    for id in ids {
        let mut node = all.get(id)?.clone();
        retain_wires(&mut node, &selected);
        copied.insert(id.clone(), node);
    }
    (!copied.is_empty()).then(|| json!({"nodes": copied}))
}

fn retain_wires(node: &mut Value, ids: &HashSet<&str>) {
    if let Some(inputs) = node.get_mut("inputs").and_then(Value::as_object_mut) {
        inputs.retain(|_, source| match source {
            Value::String(s) => ids.contains(s.as_str()),
            Value::Array(a) => { a.retain(|x| x.as_str().is_some_and(|s| ids.contains(s))); !a.is_empty() }
            _ => false,
        });
    }
}

pub fn paste(value: &Value, clipboard: &Value) -> Option<(Value, Vec<String>)> {
    let copied = nodes(clipboard)?;
    if copied.is_empty() { return None; }
    let mut value = value.clone();
    let all = nodes_mut(&mut value)?;
    let mut ids = HashMap::new();
    for (old, node) in copied {
        let ty = node.get("type")?.as_str()?;
        let id = fresh_id(all, ty);
        all.insert(id.clone(), json!({"type": ty}));
        ids.insert(old.as_str(), id);
    }
    for (old, src) in copied {
        let mut node = src.clone();
        if let Some(inputs) = node.get_mut("inputs").and_then(Value::as_object_mut) {
            for source in inputs.values_mut() {
                match source {
                    Value::String(s) => if let Some(id) = ids.get(s.as_str()) { *s = id.clone(); },
                    Value::Array(a) => for item in a { if let Some(id) = item.as_str().and_then(|s| ids.get(s)) { *item = json!(id); } },
                    _ => {},
                }
            }
        }
        all.insert(ids[old.as_str()].clone(), node);
    }
    let new_ids = copied.keys().map(|old| ids[old.as_str()].clone()).collect();
    Some((value, new_ids))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn noodle_add_connects_in_both_directions_and_records_position() {
        let base = graph();
        let (next, id) = add_connected(&base, "transform", &AddConnection::Output("box1".into()), [320., 192.]).unwrap();
        assert_eq!(next["nodes"][&id]["inputs"]["geo"], "box1");
        assert_eq!(next["nodes"][&id]["pos"], json!([320., 192.]));
        let mut doc = validated(base.clone()).unwrap();
        let mut history = minihou_app::History::default();
        history.replace(&mut doc, validated(next.clone()).unwrap());
        history.undo(&mut doc).unwrap();
        assert_eq!(doc.value(), &base);
        history.redo(&mut doc).unwrap();
        assert_eq!(doc.value(), &next);
        let (next, id) = add_connected(&base, "box", &AddConnection::Input { node: "merge1".into(), port: "geos".into() }, [-32., 64.]).unwrap();
        assert_eq!(next["nodes"]["merge1"]["inputs"]["geos"], json!(["box1", "box2", id]));
        assert_eq!(next["display"], base["display"]);
        validated(next).unwrap();
    }
    #[test] fn noodle_add_handles_optional_inputs_and_terminal_output() {
        let base = json!({"version":1,"display":"output1","nodes":{"box1":{"type":"box"},"output1":{"type":"output","inputs":{"geo":"box1"}}}});
        for ty in ["transform", "curve"] {
            let (next, id) = add_connected(&base, ty, &AddConnection::Output("output1".into()), [32., 64.]).unwrap();
            assert_eq!(next["nodes"][&id]["inputs"]["geo"], "box1");
            assert_eq!(next["nodes"]["output1"]["inputs"]["geo"], id.as_str());
            assert_eq!(next["display"], "output1");
            validated(next).unwrap();
        }
        let (next, id) = add_connected(&base, "curve", &AddConnection::Output("box1".into()), [0., 0.]).unwrap();
        assert_eq!(next["nodes"][&id]["inputs"]["geo"], "box1");
    }
    #[test] fn noodle_add_replaces_exact_input_and_rejects_invalid_contexts() {
        let base = json!({"version":1,"nodes":{"a":{"type":"box"},"b":{"type":"transform","inputs":{"geo":"a"}}}});
        let input = AddConnection::Input { node: "b".into(), port: "geo".into() };
        let (next, id) = add_connected(&base, "box", &input, [0., 0.]).unwrap();
        assert_eq!(next["nodes"]["b"]["inputs"]["geo"], id.as_str());
        assert!(next.get("display").is_none());
        assert!(add_connected(&base, "output", &input, [0., 0.]).is_none());
        assert!(add_connected(&base, "point", &AddConnection::Output("a".into()), [0., 0.]).is_none());
        assert!(add_connected(&base, "transform", &AddConnection::Output("missing".into()), [0., 0.]).is_none());
        assert!(add_connected(&base, "box", &AddConnection::Input { node: "b".into(), port: "missing".into() }, [0., 0.]).is_none());
        assert!(add_connected(&base, "box", &input, [f32::NAN, 0.]).is_none());
        assert_eq!(base["nodes"].as_object().unwrap().len(), 2);
    }
    #[test] fn reverse_noodle_preserves_other_named_inputs() {
        let base = json!({"nodes":{"a":{"type":"box"},"b":{"type":"ray","inputs":{"geo":"a","target":"a"}}}});
        let (next, id) = add_connected(&base, "sphere", &AddConnection::Input { node: "b".into(), port: "target".into() }, [32., 64.]).unwrap();
        assert_eq!(next["nodes"]["b"]["inputs"]["geo"], "a");
        assert_eq!(next["nodes"]["b"]["inputs"]["target"], id.as_str());
        validated(next).unwrap();
    }
    #[test] fn detach_keeps_card_and_bypasses_all_consumers() {
        let v = json!({"version":1,"display":"b","nodes":{
            "a":{"type":"box"}, "other":{"type":"box"},
            "b":{"type":"transform","inputs":{"geo":"a"},"params":{"translate":[1,2,3]},"pos":[123,456]},
            "c":{"type":"normal","inputs":{"geo":"b"}},
            "d":{"type":"merge","inputs":{"geos":["other","b","a"]}}
        }});
        let next = detach(&v, "b").unwrap();
        let mut card = v["nodes"]["b"].clone();
        card.as_object_mut().unwrap().remove("inputs");
        assert_eq!(next["nodes"]["b"], card);
        assert_eq!(next["nodes"]["c"]["inputs"]["geo"], "a");
        assert_eq!(next["nodes"]["d"]["inputs"]["geos"], json!(["other","a"]));
        assert_eq!(next["display"], "a");
        validated(next).unwrap();
    }

    fn graph() -> Value { json!({"version": 1, "display": "merge1", "nodes": {
        "box1": {"type": "box"}, "box2": {"type": "box"},
        "merge1": {"type": "merge", "inputs": {"geos": ["box1", "box2"]}}
    }}) }
    #[test] fn adding_after_middle_splices_scalar_and_multi_consumers() {
        let base = json!({"version":1,"display":"out","nodes":{
            "a":{"type":"box"}, "b":{"type":"normal","inputs":{"geo":"a"}},
            "c":{"type":"normal","inputs":{"geo":"b"}},
            "merge":{"type":"merge","inputs":{"geos":["a","b"]}},
            "out":{"type":"output","inputs":{"geo":"c"}}
        }});
        let (next, id) = add(&base, "normal", Some("b")).unwrap();
        assert_eq!(next["nodes"][&id]["inputs"]["geo"], "b");
        assert_eq!(next["nodes"]["c"]["inputs"]["geo"], id.as_str());
        assert_eq!(next["nodes"]["merge"]["inputs"]["geos"], json!(["a",id]));
        assert_eq!(next["nodes"]["out"], base["nodes"]["out"]);
        assert_eq!(next["display"], base["display"]);
        validated(next).unwrap();
        let (next, _) = add_connected(&base, "normal", &AddConnection::Output("b".into()), [0.,0.]).unwrap();
        assert_eq!(next["nodes"]["c"], base["nodes"]["c"]);
        assert_eq!(next["nodes"]["merge"], base["nodes"]["merge"]);
    }
    #[test] fn add_and_duplicate_find_unused_ids() {
        let (v, id) = add(&graph(), "box", Some("box1")).unwrap();
        assert_eq!(id, "box3"); assert_eq!(minihou_app::display_nodes(&v), vec!["merge1", "box3"]);
        assert!(v["nodes"][&id].get("inputs").is_none()); validated(v.clone()).unwrap();
        let (v, next) = duplicate(&v, "box1").unwrap();
        assert_eq!(next, "box4"); assert_eq!(v["nodes"]["box4"], v["nodes"]["box1"]); validated(v).unwrap();
    }
    #[test] fn adding_after_an_output_inserts_before_it() {
        let base = json!({"version":1,"nodes":{"box1":{"type":"box"},"output1":{"type":"output","inputs":{"geo":"box1"}}},"display":"output1"});
        let (v, id) = add(&base, "transform", Some("output1")).unwrap();
        assert_eq!(v["nodes"][&id]["inputs"]["geo"], "box1");
        assert_eq!(v["nodes"]["output1"]["inputs"]["geo"], id.as_str());
        assert_eq!(minihou_app::display_nodes(&v), vec!["output1"]);
        validated(v).unwrap();
    }
    #[test] fn displayed_set_adds_generators_and_replaces_modifier_input() {
        let base = json!({"version":1,"nodes":{"box1":{"type":"box"}},"display":"box1"});
        let (two, sphere) = add(&base, "sphere", Some("box1")).unwrap();
        assert_eq!(minihou_app::display_nodes(&two), vec!["box1", sphere.as_str()]);
        assert!(two["nodes"][&sphere].get("inputs").is_none());
        let (modified, transform) = add(&two, "transform", Some(&sphere)).unwrap();
        assert_eq!(minihou_app::display_nodes(&modified), vec!["box1", transform.as_str()]);
        assert_eq!(modified["nodes"][&transform]["inputs"]["geo"], sphere);
        let removed = set_display(&modified, "box1", true).unwrap();
        assert_eq!(minihou_app::display_nodes(&removed), vec![transform.as_str()]);
        let only = set_display(&modified, "box1", false).unwrap();
        assert_eq!(minihou_app::display_nodes(&only), vec!["box1"]);
        assert_eq!(set_display(&only, "box1", true).unwrap()["display"], "box1");
    }
    #[test] fn only_required_first_inputs_are_wired_on_add() {
        let (v, id) = add(&graph(), "transform", Some("box1")).unwrap();
        assert_eq!(v["nodes"][&id]["inputs"]["geo"], "box1");
        let (v, id) = add(&v, "box", Some("transform1")).unwrap();
        assert!(v["nodes"][&id].get("inputs").is_none());
        validated(v).unwrap();
    }
    #[test] fn viewport_cuts_cook_and_undo_as_one_edit() {
        let base = json!({"version":1,"display":"out","nodes":{"box1":{"type":"box"},"out":{"type":"output","inputs":{"geo":"box1"}}}});
        let original = minihou_core::Graph::from_value(&base).unwrap().cook(None).unwrap();
        for (ty,params) in [
            ("knife",json!({"cuts":json!([{"a":[0.,-2.],"b":[0.,2.],"matrix":[1.,0.,0.,0.,0.,1.,0.,0.,0.,0.,1.,0.,0.,0.,0.,1.],"through":false}]).to_string()})),
            ("loop_cut",json!({"split_edges":"0","mode":"edge_loop","loops":1,"position":0.5})),
        ] {
            let (next,id) = add_viewport_cut(&base,"out",ty,params).unwrap();
            assert_eq!(next["nodes"][&id]["inputs"]["geo"],"box1");
            assert_eq!(next["nodes"]["out"]["inputs"]["geo"],id.as_str());
            assert_eq!(next["display"],"out");
            let cut = minihou_core::Graph::from_value(&next).unwrap().cook(None).unwrap();
            assert!(cut.npoints > original.npoints);
            let mut doc = validated(base.clone()).unwrap(); let mut history = minihou_app::History::default();
            history.replace(&mut doc,validated(next.clone()).unwrap());
            history.undo(&mut doc).unwrap(); assert_eq!(doc.value(),&base);
            history.redo(&mut doc).unwrap(); assert_eq!(doc.value(),&next);
        }
        assert!(add_viewport_cut(&base,"box1","loop_cut",json!({})).is_none());
        assert!(add_viewport_cut(&base,"out","transform",json!({})).is_none());
    }
    #[test] fn sketch_wires_after_a_cooked_node_and_can_stand_alone() {
        let points = [[0.,0.,0.], [1.,0.,0.], [1.,1.,0.]];
        let (v, id) = add_sketch(&graph(), "Polygon", &points, true, "box1").unwrap();
        assert_eq!(v["nodes"][&id]["inputs"]["geo"], "box1");
        assert_eq!(v["nodes"][&id]["params"]["points"], "0,0,0 1,0,0 1,1,0");
        assert_eq!(v["nodes"][&id]["params"]["closed"], true);
        assert_eq!(minihou_app::display_nodes(&v), vec!["merge1", id.as_str()]);
        validated(v).unwrap();
        let (v, id) = add_sketch(&graph(), "Line", &points[..2], false, "missing").unwrap();
        assert!(v["nodes"][&id].get("inputs").is_none());
        validated(v).unwrap();
        assert!(add_sketch(&graph(), "Bad", &[[f32::NAN,0.,0.]; 2], false, "box1").is_none());
    }
    #[test] fn delete_cleans_wires_and_display() {
        let v = delete(&graph(), "box1").unwrap();
        assert_eq!(v["nodes"]["merge1"]["inputs"]["geos"], json!(["box2"])); validated(v).unwrap();
        let v = delete(&graph(), "merge1").unwrap();
        assert_ne!(v["display"], "merge1"); validated(v).unwrap();
    }
    #[test] fn connect_multi_and_disconnect() {
        let v = connect(&graph(), "box1", "merge1", Some("geos")).unwrap();
        assert_eq!(v["nodes"]["merge1"]["inputs"]["geos"], json!(["box1", "box2"]));
        let v = disconnect(&v, "merge1", "geos").unwrap();
        assert!(v["nodes"]["merge1"]["inputs"].get("geos").is_none()); validated(v).unwrap();
    }
    #[test] fn single_input_replaces_its_wire_and_invalid_commit_is_rejected() {
        let mut v = graph();
        v["nodes"]["transform1"] = json!({"type": "transform", "inputs": {"geo": "box1"}});
        let v = connect(&v, "box2", "transform1", Some("geo")).unwrap();
        assert_eq!(v["nodes"]["transform1"]["inputs"]["geo"], "box2");
        assert!(connect(&v, "missing", "transform1", None).is_none());
        assert!(connect(&v, "box2", "transform1", Some("missing")).is_none());
        validated(v).unwrap();
        let mut invalid = graph();
        invalid["nodes"] = json!([]);
        assert!(validated(invalid).is_err());
    }
    #[test] fn paste_remaps_only_internal_wires_and_ids_remain_unique() {
        let source = graph();
        let clip = copy(&source, &["box1".into(), "merge1".into()]).unwrap();
        assert_eq!(clip["nodes"]["merge1"]["inputs"]["geos"], json!(["box1"]));
        let (v, ids) = paste(&source, &clip).unwrap();
        assert_eq!(ids, ["box3", "merge2"]);
        assert_eq!(v["nodes"]["merge2"]["inputs"]["geos"], json!(["box3"]));
        let (v, ids) = paste(&v, &clip).unwrap();
        assert_eq!(ids, ["box4", "merge3"]); validated(v).unwrap();
    }
    #[test] fn dissolve_chain_branch_and_display() {
        let v = json!({"version":1,"display":"transform1","nodes":{
            "box1":{"type":"box"},
            "transform1":{"type":"transform","inputs":{"geo":"box1"}},
            "transform2":{"type":"transform","inputs":{"geo":"transform1"}},
            "transform3":{"type":"transform","inputs":{"geo":"transform1"}}
        }});
        let next = dissolve_set(&v, &["transform1".into()]).unwrap();
        assert!(next["nodes"].get("transform1").is_none());
        assert_eq!(next["nodes"]["transform2"]["inputs"]["geo"], "box1");
        assert_eq!(next["nodes"]["transform3"]["inputs"]["geo"], "box1");
        assert_eq!(next["display"], "box1");
        validated(next).unwrap();
    }
    #[test] fn dissolve_multi_input_and_generator() {
        let next = dissolve_set(&graph(), &["merge1".into()]).unwrap();
        assert_eq!(next["display"], "box1");
        let next = dissolve_set(&graph(), &["box1".into()]).unwrap();
        assert_eq!(next["nodes"]["merge1"]["inputs"]["geos"], json!(["box2"]));
        validated(next).unwrap();
        let mut v = graph();
        v["nodes"]["transform1"] = json!({"type":"transform","inputs":{"geo":"box1"}});
        v["nodes"]["merge1"]["inputs"]["geos"] = json!(["transform1","box2","box1"]);
        let next = dissolve_set(&v, &["transform1".into()]).unwrap();
        assert_eq!(next["nodes"]["merge1"]["inputs"]["geos"], json!(["box1","box2"]));
        validated(next).unwrap();
    }
    #[test] fn dissolve_selected_chain_and_delete_set_are_atomic_values() {
        let v = json!({"version":1,"display":"transform2","nodes":{
            "box1":{"type":"box"},
            "transform1":{"type":"transform","inputs":{"geo":"box1"}},
            "transform2":{"type":"transform","inputs":{"geo":"transform1"}},
            "transform3":{"type":"transform","inputs":{"geo":"transform2"}}
        }});
        let ids = ["transform1".into(), "transform2".into()];
        let next = dissolve_set(&v, &ids).unwrap();
        assert_eq!(next["nodes"]["transform3"]["inputs"]["geo"], "box1");
        assert_eq!(next["display"], "box1");
        validated(next).unwrap();
        let reversed = dissolve_set(&v, &["transform2".into(), "transform1".into()]).unwrap();
        assert_eq!(reversed["nodes"]["transform3"]["inputs"]["geo"], "box1");
        assert_eq!(reversed["display"], "box1");
        validated(reversed).unwrap();
        let deleted = delete_set(&v, &ids).unwrap();
        assert!(deleted["nodes"]["transform3"]["inputs"].as_object().unwrap().is_empty());
        validated(deleted).unwrap();
    }
    #[test] fn disconnect_all_keeps_node_and_clears_both_sides() {
        let v = json!({"version":1,"display":"transform2","nodes":{
            "box1":{"type":"box"},
            "transform1":{"type":"transform","inputs":{"geo":"box1"}},
            "transform2":{"type":"transform","inputs":{"geo":"transform1"}}
        }});
        let next = disconnect_all(&v, "transform1").unwrap();
        assert!(next["nodes"]["transform1"].get("inputs").is_none());
        assert!(next["nodes"]["transform2"]["inputs"].as_object().unwrap().is_empty());
        validated(next).unwrap();
    }
    #[test] fn delete_last_node_leaves_valid_empty_graph() {
        let v = json!({"version":1,"display":"box1","nodes":{"box1":{"type":"box"}}});
        let next = delete_set(&v, &["box1".into()]).unwrap();
        assert!(next.get("display").is_none());
        assert!(next["nodes"].as_object().unwrap().is_empty());
        validated(next).unwrap();
    }
    #[test] fn dissolve_reconnects_input_to_consumers() {
        let mut v = graph();
        v["nodes"]["transform1"] = json!({"type": "transform", "inputs": {"geo": "box1"}});
        v["nodes"]["merge1"]["inputs"]["geos"] = json!(["transform1", "box2"]);
        v["display"] = json!("transform1");
        let v = dissolve(&v, "transform1").unwrap();
        assert!(v["nodes"].get("transform1").is_none());
        assert_eq!(v["nodes"]["merge1"]["inputs"]["geos"], json!(["box1", "box2"]));
        assert_eq!(v["display"], "box1");
        validated(v).unwrap();
    }
}

#[cfg(test)]
mod noodle_tests {
    use super::*;
    use std::collections::BTreeSet;
    fn wire(from: &str, to: &str, port: &str) -> WireRef {
        WireRef { from: from.into(), to: to.into(), port: port.into() }
    }
    #[test]
    fn noodle_insert_exact_edge_and_position_are_one_history_edit() {
        let base = json!({"nodes": {
            "a":{"type":"box"}, "b":{"type":"box"},
            "loose":{"type":"transform","pos":[0,0]},
            "m":{"type":"merge","inputs":{"geos":["a","b"]}},
            "out":{"type":"output","inputs":{"geo":"m"}}
        },"display":"out"});
        let mut moved = base.clone(); moved["nodes"]["loose"]["pos"] = json!([50,80]);
        let next = insert_existing(&moved,"loose",&wire("a","m","geos")).unwrap();
        assert_eq!(next["nodes"]["m"]["inputs"]["geos"],json!(["loose","b"]));
        assert_eq!(next["nodes"]["loose"]["inputs"]["geo"],"a");
        assert_eq!(next["display"],"out");
        let mut doc = validated(base.clone()).unwrap();
        let mut history = minihou_app::History::default();
        history.replace(&mut doc,validated(next.clone()).unwrap());
        history.undo(&mut doc).unwrap(); assert_eq!(doc.value(),&base);
        assert!(history.undo(&mut doc).is_none());
        history.redo(&mut doc).unwrap(); assert_eq!(doc.value(),&next);
        let terminal = insert_existing(&moved,"loose",&wire("m","out","geo")).unwrap();
        assert_eq!(terminal["nodes"]["out"]["inputs"]["geo"],"loose");
        for edge in [wire("loose","m","geos"),wire("a","loose","geo"),wire("b","out","geo"),wire("gone","m","geos")] {
            assert!(insert_existing(&moved,"loose",&edge).is_none());
        }
        assert!(insert_existing(&next,"loose",&wire("b","m","geos")).is_none());
        assert!(insert_existing(&moved,"out",&wire("a","m","geos")).is_none());
        let mut generator = moved.clone(); generator["nodes"]["point"] = json!({"type":"point"});
        assert!(insert_existing(&generator,"point",&wire("a","m","geos")).is_none());
        let mut cyclic = moved.clone(); cyclic["nodes"]["a"]["inputs"]["geo"] = json!("loose");
        assert!(insert_existing(&cyclic,"loose",&wire("a","m","geos")).is_none());
    }
    #[test]
    fn noodle_pickup_preview_cancel_release_and_undo() {
        let base = json!({"nodes": {"a":{"type":"box"},"b":{"type":"box"},
            "m":{"type":"merge","inputs":{"geos":["a","b"]}},
            "t":{"type":"transform"},"out":{"type":"output","inputs":{"geo":"m"}}}});
        assert!(WirePickup::begin(&base,"t","geo").is_none());
        let pickup = WirePickup::begin(&base,"m","geos").unwrap();
        assert_eq!(pickup.original,wire("b","m","geos"));
        // Cancelling simply drops the preview: the original document never changed.
        drop(pickup.clone()); assert_eq!(base["nodes"]["m"]["inputs"]["geos"],json!(["a","b"]));
        assert!(pickup.release(&base,Some(("m","geos"))).is_none());
        assert!(pickup.release(&base,Some(("b","geo"))).is_none());
        assert!(pickup.release(&base,Some(("t","missing"))).is_none());
        let next = pickup.release(&base,Some(("t","geo"))).unwrap();
        assert_eq!(next["nodes"]["t"]["inputs"]["geo"],"b");
        assert_eq!(next["nodes"]["m"]["inputs"]["geos"],json!(["a"]));
        let disconnected = pickup.release(&base,None).unwrap();
        assert_eq!(disconnected["nodes"]["m"]["inputs"]["geos"],json!(["a"]));
        let mut doc = validated(base.clone()).unwrap(); let mut history = minihou_app::History::default();
        history.replace(&mut doc,validated(next.clone()).unwrap());
        history.undo(&mut doc).unwrap(); assert_eq!(doc.value(),&base);
        assert!(history.undo(&mut doc).is_none());
        history.redo(&mut doc).unwrap(); assert_eq!(doc.value(),&next);
        let cycle = WirePickup::begin(&base,"out","geo").unwrap();
        assert!(cycle.release(&base,Some(("m","geos"))).is_none());
    }
    #[test]
    fn noodle_cut_is_one_undoable_edit_and_never_dissolves() {
        let before = json!({"nodes": {
            "a":{"type":"box"}, "b":{"type":"box"},
            "merge":{"type":"merge", "inputs":{"geos":["a","b"]}},
            "out":{"type":"output", "inputs":{"geo":"merge"}}
        }, "output":"out"});
        let cuts=BTreeSet::from([wire("a","merge","geos"),wire("merge","out","geo")]);
        let next=disconnect_wires(&before,&cuts).unwrap();
        assert_eq!(next["nodes"]["merge"]["inputs"]["geos"],json!(["b"]));
        assert!(next["nodes"]["out"]["inputs"].get("geo").is_none());
        assert_eq!(next["nodes"].as_object().unwrap().len(),4);
        assert_eq!(next["output"],before["output"]);
        let mut doc=validated(before.clone()).unwrap();
        let mut history=minihou_app::History::default();
        history.replace(&mut doc,validated(next.clone()).unwrap());
        history.undo(&mut doc).unwrap();
        assert_eq!(doc.value(),&before);
        assert!(history.undo(&mut doc).is_none());
        history.redo(&mut doc).unwrap();
        assert_eq!(doc.value(),&next);
    }
    #[test]
    fn noodle_cut_preserves_order_other_ports_and_ignores_stale_edges() {
        let doc=json!({"nodes":{"m":{"inputs":{"geos":["a","b","c"],"other":"a"}}}});
        let cuts=BTreeSet::from([wire("b","m","geos"),wire("missing","m","geos"),wire("a","gone","geo")]);
        let next=disconnect_wires(&doc,&cuts).unwrap();
        assert_eq!(next["nodes"]["m"]["inputs"]["geos"],json!(["a","c"]));
        assert_eq!(next["nodes"]["m"]["inputs"]["other"],json!("a"));
        assert!(disconnect_wires(&next,&cuts).is_none());
        let next=disconnect_wires(&next,&BTreeSet::from([wire("a","m","geos"),wire("c","m","geos")])).unwrap();
        assert!(next["nodes"]["m"]["inputs"].get("geos").is_none());
    }
}
