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

pub fn add(value: &Value, ty: &str, after: Option<&str>) -> Option<(Value, String)> {
    let spec = nodes::get(ty)?;
    let mut value = value.clone();
    let displayed = minihou_app::display_nodes(&value);
    let all = nodes_mut(&mut value)?;
    let id = fresh_id(all, ty);
    let mut node = json!({"type": ty});
    let source = after.filter(|id| all.contains_key(*id));
    if let (Some(after), Some(port)) = (source, spec.inputs.first().filter(|p| !p.optional)) {
        node["inputs"] = if port.multi { json!({port.name: [after]}) } else { json!({port.name: after}) };
    }
    all.insert(id.clone(), node);
    let mut next = displayed;
    if spec.inputs.iter().all(|port| port.optional) {
        next.push(id.clone());
    } else if let Some(source) = source {
        if let Some(index) = next.iter().position(|item| item == source) { next[index] = id.clone(); }
        else { next = vec![id.clone()]; }
    } else { next = vec![id.clone()]; }
    value["display"] = if next.len() == 1 { json!(next[0]) } else { json!(next) };
    Some((value, id))
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
    fn graph() -> Value { json!({"version": 1, "display": "merge1", "nodes": {
        "box1": {"type": "box"}, "box2": {"type": "box"},
        "merge1": {"type": "merge", "inputs": {"geos": ["box1", "box2"]}}
    }}) }
    #[test] fn add_and_duplicate_find_unused_ids() {
        let (v, id) = add(&graph(), "box", Some("box1")).unwrap();
        assert_eq!(id, "box3"); assert_eq!(minihou_app::display_nodes(&v), vec!["merge1", "box3"]);
        assert!(v["nodes"][&id].get("inputs").is_none()); validated(v.clone()).unwrap();
        let (v, next) = duplicate(&v, "box1").unwrap();
        assert_eq!(next, "box4"); assert_eq!(v["nodes"]["box4"], v["nodes"]["box1"]); validated(v).unwrap();
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
}
