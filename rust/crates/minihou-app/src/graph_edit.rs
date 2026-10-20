//! Pure document changes used by the network editor.
use minihou_app::Document;
use minihou_core::nodes;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};

mod document;
pub use document::validated;
use document::{fresh_id, nodes, nodes_mut};
mod layout;
use layout::position_in_stream;

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
            // An explicit noodle branches from its endpoint; terminal outputs insert upstream.
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
    let mut repaired = value.clone();
    repair_output_hangs(&mut repaired);
    add_impl(&repaired, ty, after, true)
}

/// Repair the older viewport shape `source -> output -> operator` in place.
/// Each hanging operator becomes the last operation before its output.
pub fn repair_output_hangs(value: &mut Value) -> usize {
    let Some(all) = nodes(value) else { return 0 };
    let repairs: Vec<_> = all.iter().filter(|(_, n)| n["type"] == "output").flat_map(|(out, _)| {
        all.iter().filter_map(move |(id, child)| {
            (child["inputs"]["geo"].as_str() == Some(out.as_str()) && child["type"] != "output")
                .then(|| (out.clone(), id.clone()))
        })
    }).collect();
    let mut count = 0;
    for (out, id) in repairs {
        let Some(upstream) = value["nodes"][&out]["inputs"]["geo"].as_str().map(str::to_owned) else { continue };
        let old = value.clone();
        value["nodes"][&id]["inputs"]["geo"] = json!(upstream);
        value["nodes"][&out]["inputs"]["geo"] = json!(id);
        position_in_stream(&old, value, &out, &id, true);
        count += 1;
    }
    count
}

fn add_impl(value: &Value, ty: &str, after: Option<&str>, splice: bool) -> Option<(Value, String)> {
    let spec = nodes::get(ty)?;
    let original = value;
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
            position_in_stream(original, &mut value, out_id, &id, true);
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
    if splice && ty != "output" {
        if let Some(source) = source { position_in_stream(original, &mut value, source, &id, false); }
    }
    Some((value, id))
}

/// Adds a node placed at world position `pos`. If `ty` is a primitive with a `center` parameter,
/// sets `center` to `pos`. If `ty` is a primitive without `center`, adds the node and then adds a
/// `transform` node after it with `translate` set to `pos`. For non-primitives, behaves like `add`.
pub fn node_has_center_param(ty: &str) -> bool {
    matches!(ty, "box" | "sphere" | "tube" | "torus" | "grid" | "circle" | "point")
}
pub fn is_primitive_node(ty: &str) -> bool {
    node_has_center_param(ty) || matches!(ty, "line" | "curve" | "bezier" | "helix" | "lsystem" | "null" | "lattice_grid")
}
pub fn add_at(value: &Value, ty: &str, after: Option<&str>, pos: [f32; 3]) -> Option<(Value, String)> {
    if !is_primitive_node(ty) {
        return add(value, ty, after);
    }
    if node_has_center_param(ty) {
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
    node["params"] = json!({"points": text, "closed": closed, "split": !closed});
    node["name"] = json!(name);
    Some((value, id))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SketchTarget { New, Curve, Mesh }
impl Default for SketchTarget { fn default() -> Self { Self::New } }

/// A displayed stream may contain both polygons and curves. Surface primitives win;
/// otherwise the viewport curve attribute or open primitives identify a curve stream.
pub fn sketch_target(g: Option<&minihou_core::Geometry>) -> SketchTarget {
    let Some(g) = g else { return SketchTarget::New };
    let t = &g.topology;
    let mut curve = false;
    for i in 0..t.nprims() {
        let marked = g.prim.get("__viewport_curve").and_then(|a| a.as_i32())
            .is_some_and(|v| v.get(i) == Some(&1));
        if t.is_closed(i) && !marked && t.prim_offsets[i + 1] - t.prim_offsets[i] >= 3 { return SketchTarget::Mesh; }
        curve |= marked || !t.is_closed(i);
    }
    if curve { SketchTarget::Curve } else { SketchTarget::New }
}

fn splice_sketch_consumers(value: &mut Value, source: &str, inserted: &str) -> Option<()> {
    for (id, node) in nodes_mut(value)?.iter_mut() {
        if id == inserted { continue; }
        if let Some(inputs) = node.get_mut("inputs").and_then(Value::as_object_mut) {
            for input in inputs.values_mut() {
                if input.as_str() == Some(source) { *input = json!(inserted); }
                else if let Some(items) = input.as_array_mut() {
                    for item in items { if item.as_str() == Some(source) { *item = json!(inserted); } }
                }
            }
        }
    }
    Some(())
}

/// Insert all strokes into one displayed object stream. The caller commits the
/// resulting document once, so undo removes the entire drawing session.
pub fn add_sketch_batch(value: &Value, strokes: &[(&str, &[[f32; 3]], bool)], selected: &str, target: SketchTarget) -> Option<(Value, String, Vec<String>)> {
    if strokes.is_empty() { return None; }
    let mut next = value.clone();
    let mut source = if target == SketchTarget::New { None } else {
        let node = nodes(&next)?.get(selected)?;
        if node["type"] == "output" { node["inputs"]["geo"].as_str().map(str::to_owned) }
        else { Some(selected.to_owned()) }
    };
    let mut last = String::new();
    let mut stroke_ids = Vec::new();
    for &(name, points, closed) in strokes {
        if target == SketchTarget::Mesh {
            let (mut with_curve, curve) = if name == "Bezier" { add_bezier_sketch(&next, name, points, closed, "")? }
                else { add_sketch(&next, name, points, closed, "")? };
            with_curve["nodes"][&curve]["params"]["as_mesh"] = json!(true);
            let anchor = source.as_deref()?;
            let (mut merged, id) = add(&with_curve, "merge", Some(anchor))?;
            merged["nodes"][&id]["inputs"]["geos"] = json!([anchor, curve]);
            stroke_ids.push(curve);
            next = merged; source = Some(id.clone()); last = id;
        } else {
            let after = source.as_deref().unwrap_or("");
            let (mut updated, id) = if name == "Bezier" { add_bezier_sketch(&next, name, points, closed, after)? }
                else { add_sketch(&next, name, points, closed, after)? };
            if let Some(source_id) = source.as_deref() { splice_sketch_consumers(&mut updated, source_id, &id)?; }
            next = updated; source = Some(id.clone()); last = id.clone(); stroke_ids.push(id);
        }
    }
    let display: Vec<String> = if target == SketchTarget::New { vec![last.clone()] }
        else { minihou_app::display_nodes(value).into_iter().map(|id| if id == selected && value["nodes"][&id]["type"] != "output" { last.clone() } else { id }).collect() };
    next["display"] = if display.len() == 1 { json!(display[0]) } else { json!(display) };
    validated(next.clone()).ok()?;
    Some((next, last, stroke_ids))
}

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

/// Viewport topology edits use one displayed object stream.
pub fn add_viewport_cut(value: &Value, source: &str, ty: &str, params: Value) -> Option<(Value, String)> {
    if !matches!(ty, "knife" | "loop_cut" | "polysplit" | "edge_slide") || !params.is_object() { return None; }
    if !minihou_app::display_nodes(value).iter().any(|id|id==source) { return None; }
    nodes(value)?.get(source)?;
    let (mut next, id) = add(value, ty, Some(source))?;
    next["nodes"][&id]["params"] = params;
    validated(next.clone()).ok()?;
    Some((next, id))
}

/// Insert a point deletion into the displayed curve stream. The output node remains terminal.
pub fn add_curve_blast(value: &Value, display: &str, points: &[u32]) -> Option<(Value, String)> {
    if points.is_empty() || !curve_stream(value, display) { return None; }
    let mut ids = points.to_vec();
    ids.sort_unstable();
    ids.dedup();
    let (mut next, id) = add(value, "blast", Some(display))?;
    next["nodes"][&id]["params"] = json!({"domain": "points", "group": ids.iter().map(u32::to_string).collect::<Vec<_>>().join(",")});
    validated(next.clone()).ok()?;
    Some((next, id))
}

fn curve_stream(value: &Value, display: &str) -> bool {
    let mut node = display;
    let mut seen = HashSet::new();
    while seen.insert(node.to_owned()) {
        let current = &value["nodes"][node];
        if matches!(current["type"].as_str(), Some("curve" | "bezier" | "helix" | "polypath" | "line" | "circle" | "convert_line")) {
            return true;
        }
        let input = match current.get("inputs").and_then(|i| i.get("geo")) {
            Some(Value::String(s)) => Some(s.as_str()),
            Some(Value::Array(a)) => a.first().and_then(Value::as_str),
            _ => None,
        };
        let Some(input) = input else { break; };
        node = input;
    }
    false
}

/// Add a bezier sketch to an existing cooked node, or as a standalone bezier curve when there is no usable source.
pub fn add_bezier_sketch(value: &Value, name: &str, points: &[[f32; 3]], closed: bool, selected: &str) -> Option<(Value, String)> {
    if points.len() < 2 || points.iter().flatten().any(|x| !x.is_finite()) { return None; }
    let after = nodes(value)?.get(selected).and_then(|node| node.get("type")?.as_str())
        .filter(|ty| nodes::get(ty).is_some()).map(|_| selected);
    let (mut value, id) = add(value, "bezier", after)?;
    if let Some(after) = after { value = connect(&value, after, &id, Some("geo"))?; }
    let controls = points.iter().map(|p| json!({"p": p, "left": p, "right": p, "mode": "AUTO"})).collect::<Vec<_>>();
    let node = nodes_mut(&mut value)?.get_mut(&id)?;
    node["params"] = json!({"controls": serde_json::to_string(&controls).unwrap_or_default(), "closed": closed});
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
    connect_output(value, from, 0, to, port)
}

fn endpoint_node(value: &Value) -> Option<&str> {
    value.as_str().or_else(|| value.get("node")?.as_str())
}

fn endpoint_output(value: &Value) -> Option<usize> {
    if value.is_string() { Some(0) }
    else { value.get("output")?.as_u64().and_then(|n| usize::try_from(n).ok()) }
}

pub fn connect_output(value: &Value, from: &str, output: usize, to: &str, port: Option<&str>) -> Option<Value> {
    if from == to { return None; }
    let all = nodes(value)?;
    let source = all.get(from)?;
    let source_type = source.get("type")?.as_str()?;
    let native = output < nodes::get(source_type)?.outputs.len();
    let subnet = source_type == "subnet" && source.get("output_sockets").and_then(Value::as_array)
        .is_some_and(|sockets| sockets.iter().any(|socket| socket.get("index").and_then(Value::as_u64) == Some(output as u64)));
    if !native && !subnet { return None; }
    let ty = all.get(to)?.get("type")?.as_str()?;
    let spec = nodes::get(ty)?;
    let input = match port { Some(name) => spec.inputs.iter().find(|p| p.name == name)?, None => spec.inputs.first()? };
    let mut value = value.clone();
    let target = nodes_mut(&mut value)?.get_mut(to)?;
    if target.get("inputs").is_none() { target["inputs"] = json!({}); }
    let inputs = target.get_mut("inputs")?.as_object_mut()?;
    let endpoint = if output == 0 { json!(from) } else { json!({"node":from,"output":output}) };
    if input.multi {
        let list = inputs.entry(input.name).or_insert_with(|| json!([]));
        if let Some(a) = list.as_array_mut() { if !a.contains(&endpoint) { a.push(endpoint); } }
        else { *list = json!([endpoint]); }
    } else { inputs.insert(input.name.to_owned(), endpoint); }
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
        inputs.values().any(|v| endpoint_node(v).is_some() || v.as_array().is_some_and(|a| !a.is_empty()))) { return None; }
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
                endpoint_node(v) == Some(source) || v.as_array().is_some_and(|a| a.iter().any(|r| endpoint_node(r) == Some(source))))) {
                pending.push(target.as_str());
            }
        }
    }
    let original = all.get(&wire.to)?.get("inputs")?.get(&wire.port)?;
    let original = if let Some(a) = original.as_array() { a.iter().find(|r| endpoint_node(r)==Some(&wire.from))? } else { original };
    let mut next = connect_output(value, &wire.from, endpoint_output(original)?, id, Some(input.name))?;
    let edge = next.get_mut("nodes")?.get_mut(&wire.to)?.get_mut("inputs")?.get_mut(&wire.port)?;
    if let Some(refs) = edge.as_array_mut() {
        if refs.iter().any(|r| endpoint_node(r) == Some(id)) { return None; }
        let index = refs.iter().position(|r| endpoint_node(r) == Some(&wire.from))?;
        refs[index] = json!(id);
        refs.retain(|r| endpoint_node(r) != Some(&wire.from));
    } else if endpoint_node(edge) == Some(&wire.from) { *edge = json!(id); }
    else { return None; }
    Some(next)
}

/// A pickup remains a UI preview until release, so cancellation needs no graph edit.
#[derive(Clone, Debug)]
pub struct WirePickup { pub original: WireRef }
impl WirePickup {
    pub fn begin(value: &Value, node: &str, port: &str) -> Option<Self> {
        let refs = value.get("nodes")?.get(node)?.get("inputs")?.get(port)?;
        let from = endpoint_node(refs).or_else(|| refs.as_array()?.last().and_then(endpoint_node))?;
        Some(Self { original: WireRef { from: from.into(), to: node.into(), port: port.into() } })
    }
    pub fn release(&self, value: &Value, target: Option<(&str, &str)>) -> Option<Value> {
        if target == Some((self.original.to.as_str(), self.original.port.as_str())) { return None; }
        let original = value.get("nodes")?.get(&self.original.to)?.get("inputs")?.get(&self.original.port)?;
        let original = if let Some(a)=original.as_array() { a.iter().find(|r| endpoint_node(r)==Some(&self.original.from))? } else { original };
        let output = endpoint_output(original)?;
        let next = disconnect_wires(value, &std::collections::BTreeSet::from([self.original.clone()]))?;
        if let Some((node, port)) = target {
            if value["nodes"][&self.original.from]["type"] == "output" { return None; }
            let next = connect_output(&next, &self.original.from, output, node, Some(port))?;
            // Reject cycles, including those outside the displayed branch.
            let mut pending = vec![self.original.from.as_str()];
            let mut seen = HashSet::new();
            while let Some(id) = pending.pop() {
                if id == node { return None; }
                if !seen.insert(id) { continue; }
                if let Some(inputs) = next["nodes"][id]["inputs"].as_object() {
                    for refs in inputs.values() {
                        if let Some(source) = endpoint_node(refs) { pending.push(source); }
                        else if let Some(refs) = refs.as_array() { pending.extend(refs.iter().filter_map(endpoint_node)); }
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
            refs.retain(|r| endpoint_node(r) != Some(wire.from.as_str()));
            changed |= before != refs.len();
            if refs.is_empty() { inputs.remove(&wire.port); }
        } else if endpoint_node(input) == Some(wire.from.as_str()) {
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
        node.as_object_mut()?.remove("linked_params_group");
        if let Some(channels) = node.get_mut("channels").and_then(Value::as_object_mut) {
            for expr in channels.values_mut() {
                if let Some(source) = expr.as_str() {
                    *expr = json!(crate::value_link_ui::rewrite_expression(source, old, &ids[old.as_str()], |referenced| ids.get(referenced).cloned().unwrap_or_else(||referenced.into())));
                }
            }
        }
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

/// Copy displayed objects with their upstream graphs so later source edits do not alter the copies.
pub fn duplicate_objects(value: &Value, ids: &[String]) -> Option<(Value, Vec<String>)> {
    let all = nodes(value)?;
    let mut closure = HashSet::new();
    let mut pending = ids.to_vec();
    while let Some(id) = pending.pop() {
        if !closure.insert(id.clone()) { continue; }
        let inputs = all.get(&id)?.get("inputs").and_then(Value::as_object);
        if let Some(inputs) = inputs {
            for source in inputs.values() {
                if let Some(id) = source.as_str() { pending.push(id.to_owned()); }
                if let Some(list) = source.as_array() {
                    pending.extend(list.iter().filter_map(Value::as_str).map(str::to_owned));
                }
            }
        }
    }
    let mut old_ids: Vec<_> = closure.into_iter().collect();
    old_ids.sort();
    let clip = copy(value, &old_ids)?;
    let (mut next, new_ids) = paste(value, &clip)?;
    let mapping: HashMap<_, _> = old_ids.iter().cloned().zip(new_ids).collect();
    let roots: Vec<_> = ids.iter().map(|id| mapping.get(id).cloned()).collect::<Option<_>>()?;
    let all = nodes_mut(&mut next)?;
    let mut copies = Vec::new();
    for root in roots {
        let wrapper = fresh_id(all, "transform");
        all.insert(wrapper.clone(), json!({"type":"transform","inputs":{"geo":root}}));
        copies.push(wrapper);
    }
    let mut shown = minihou_app::display_nodes(value);
    shown.extend(copies.iter().cloned());
    next["display"] = json!(shown);
    Some((next, copies))
}

/// Copies nodes and joins each pair's parameter edits, including non-numeric parameters.
pub fn duplicate_linked(value: &Value, ids: &[String]) -> Option<(Value, Vec<String>)> {
    let clip = copy(value, ids)?;
    let (mut next, copies) = paste(value, &clip)?;
    for (source, copy) in ids.iter().zip(&copies) {
        let group = next["nodes"][source]["linked_params_group"].as_str()
            .map(str::to_owned).unwrap_or_else(|| source.clone());
        next["nodes"][source]["linked_params_group"] = json!(group);
        next["nodes"][copy]["linked_params_group"] = json!(group);
    }
    Some((next, copies))
}

#[cfg(test)]
mod duplicate_tests {
    use super::*;
    #[test]
    fn network_linked_duplicates_share_groups_but_regular_copies_do_not() {
        let doc = json!({"nodes":{"box1":{"type":"box","params":{"size":[1,2,3]}}}});
        let (linked, copies) = duplicate_linked(&doc, &["box1".into()]).unwrap();
        assert_eq!(linked["nodes"]["box1"]["linked_params_group"], "box1");
        assert_eq!(linked["nodes"][&copies[0]]["linked_params_group"], "box1");
        let (regular, regular_ids) = paste(&linked, &copy(&linked, &["box1".into()]).unwrap()).unwrap();
        assert!(regular["nodes"][&regular_ids[0]].get("linked_params_group").is_none());
    }
    #[test]
    fn pick_viewport_duplicate_copies_upstream_nodes_and_displays_both() {
        let doc = json!({"display":"transform1","nodes":{
            "box1":{"type":"box"},
            "transform1":{"type":"transform","inputs":{"geo":"box1"}}
        }});
        let (copy, ids) = duplicate_objects(&doc, &["transform1".into()]).unwrap();
        assert!(crate::Document::parse(copy.to_string()).is_ok());
        let root = copy["nodes"][&ids[0]]["inputs"]["geo"].as_str().unwrap();
        let upstream = copy["nodes"][root]["inputs"]["geo"].as_str().unwrap();
        assert_ne!(upstream, "box1");
        assert!(copy["nodes"].get(upstream).is_some());
        assert_eq!(copy["display"], json!(["transform1", ids[0]]));
    }
    #[test]
    fn pick_viewport_duplicate_of_output_is_valid() {
        let doc = json!({"display":"out","nodes":{
            "box1":{"type":"box"},
            "out":{"type":"output","inputs":{"geo":"box1"}}
        }});
        let (copy, ids) = duplicate_objects(&doc, &["out".into()]).unwrap();
        assert!(crate::Document::parse(copy.to_string()).is_ok());
        assert_eq!(copy["display"], json!(["out", ids[0]]));
    }
}

/// Rename a node and preserve every channel's resolved target in one graph edit.
pub fn rename(value: &Value, old: &str, new: &str) -> Option<Value> {
    if new.is_empty() || new.contains('"') || new.contains('\'') || new.contains(' ') { return None; }
    let mut value=value.clone();
    let all=nodes_mut(&mut value)?;
    if all.contains_key(new) { return None; }
    let mut renamed=all.remove(old)?;
    // A Blender Object node's group picker keeps its socket (value, links) across renames.
    if renamed["type"]=="blender_object" && renamed.get(minihou_app::modifier_inputs::PICKER_ID).is_none() { renamed[minihou_app::modifier_inputs::PICKER_ID]=json!(old); }
    all.insert(new.into(),renamed);
    for (owner,node) in all.iter_mut() {
        if let Some(inputs)=node.get_mut("inputs").and_then(Value::as_object_mut) {
            for source in inputs.values_mut() {
                match source { Value::String(s) if s==old => *s=new.into(), Value::Array(xs) => for x in xs {if x==old {*x=json!(new)}}, _=>{} }
            }
        }
        if let Some(channels)=node.get_mut("channels").and_then(Value::as_object_mut) {
            for expr in channels.values_mut() {
                if let Some(source)=expr.as_str() {
                    let before=if owner==new {old} else {owner};
                    *expr=json!(crate::value_link_ui::rewrite_expression(source,before,owner,|node|if node==old {new.into()} else {node.into()}));
                }
            }
        }
    }
    if value["display"]==old {value["display"]=json!(new);}
    if let Some(xs)=value["display"].as_array_mut() {for x in xs {if x==old {*x=json!(new)}}}
    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn viewport_insertion_splices_branches_pushes_stream_and_undoes_once() {
        let base = json!({"nodes": {
            "source":{"type":"box","pos":[96,32]},
            "middle":{"type":"transform","inputs":{"geo":"source"},"pos":[96,160]},
            "out":{"type":"output","inputs":{"geo":"middle"},"pos":[96,288]},
            "branch":{"type":"normal","inputs":{"geo":"middle"},"pos":[416,288]}
        },"display":["out","branch"]});
        for source in ["out", "middle", "source"] {
            for ty in ["transform", "polyextrude", "inset", "polybevel", "pushpull", "mirror", "grow", "loop_cut"] {
                let (next, id) = add(&base, ty, Some(source)).unwrap();
                let anchor = if source == "out" { "middle" } else { source };
                assert_eq!(next["nodes"][&id]["inputs"]["geo"], anchor, "{source} {ty}");
                assert_eq!(next["nodes"][&id]["pos"][0].as_f64(), base["nodes"][if source == "out" { "out" } else { source }]["pos"][0].as_f64());
                if source == "out" { assert_eq!(next["nodes"]["out"]["inputs"]["geo"], id); }
                else {
                    for consumer in ["middle", "out", "branch"] {
                        if base["nodes"][consumer]["inputs"]["geo"] == source { assert_eq!(next["nodes"][consumer]["inputs"]["geo"], id); }
                    }
                }
                assert_eq!(next["display"], base["display"]);
                let origin_y = base["nodes"][if source == "out" { "out" } else { source }]["pos"][1].as_f64().unwrap();
                assert_eq!(next["nodes"][&id]["pos"][1].as_f64().unwrap(), origin_y + if source == "out" { 0.0 } else { 128.0 });
                if source == "out" { assert_eq!(next["nodes"]["out"]["pos"], json!([96.,416.])); }
                if source == "middle" { assert_eq!(next["nodes"]["out"]["pos"], json!([96.,416.])); }
                let mut doc = validated(base.clone()).unwrap();
                let mut history = minihou_app::History::default();
                history.replace(&mut doc, validated(next).unwrap());
                history.undo(&mut doc).unwrap();
                assert_eq!(doc.value(), &base);
            }
        }
    }
    #[test] fn output_insertion_materializes_unpositioned_stream() {
        let base = json!({"nodes":{"box1":{"type":"box"},"out":{"type":"output","inputs":{"geo":"box1"}}},"display":"out"});
        let (next,id) = add(&base,"transform",Some("out")).unwrap();
        assert_eq!(next["nodes"][&id]["pos"], json!([96.,160.]));
        assert_eq!(next["nodes"]["out"]["pos"], json!([96.,288.]));
        assert_eq!(next["nodes"]["out"]["inputs"]["geo"], id);
    }
    #[test] fn repairs_operator_hanging_after_output() {
        let mut bad = json!({"nodes":{"source":{"type":"box","pos":[96,32]},
            "out":{"type":"output","inputs":{"geo":"source"},"pos":[96,160]},
            "transform1":{"type":"transform","inputs":{"geo":"out"},"pos":[96,288]}},"display":"out"});
        assert_eq!(repair_output_hangs(&mut bad), 1);
        assert_eq!(bad["nodes"]["transform1"]["inputs"]["geo"], "source");
        assert_eq!(bad["nodes"]["out"]["inputs"]["geo"], "transform1");
        assert_eq!(bad["nodes"]["transform1"]["pos"], json!([96.,160.]));
        assert_eq!(bad["nodes"]["out"]["pos"], json!([96.,288.]));
    }
    #[test] fn measurement_deletion_and_clear_are_undoable_document_edits() {
        let mut base=graph(); base["nodes"]["box1"]["measurement_identity"]=json!("stable");
        base["measurements"]=json!([{"id":"one","points":[[0,0,0],[1,0,0]],"refs":[{"node":"box1","identity":"stable","point":0,"point_count":8},null],"a":[0,0,0],"b":[1,0,0]},{"a":[0,0,0],"b":[2,0,0]}]);
        let mut doc=validated(base.clone()).unwrap();
        let mut history=minihou_app::History::default();
        let one=delete_measurement(doc.value(),0).unwrap();
        assert_eq!(one["measurements"].as_array().unwrap().len(),1);
        history.replace(&mut doc,validated(one).unwrap());
        let empty=validated(clear_measurements(doc.value()).unwrap()).unwrap();
        history.replace(&mut doc,empty);
        assert!(doc.value()["measurements"].as_array().unwrap().is_empty());
        history.undo(&mut doc).unwrap();
        assert_eq!(doc.value()["measurements"].as_array().unwrap().len(),1);
        history.undo(&mut doc).unwrap();
        assert_eq!(doc.value()["measurements"].as_array().unwrap().len(),2);
    }
    #[test] fn renaming_a_blender_object_pins_its_picker_id() {
        let g=json!({"display":"a","nodes":{"a":{"type":"blender_object"}}});
        let once=rename(&g,"a","b").unwrap();
        assert_eq!(once["nodes"]["b"]["picker_id"],"a");
        let twice=rename(&once,"b","c").unwrap();
        assert_eq!(twice["nodes"]["c"]["picker_id"],"a");
        assert_eq!(minihou_app::modifier_inputs::pickers(&twice),vec![("c".to_string(),"a".to_string())]);
        assert!(rename(&json!({"nodes":{"x":{"type":"box"}}}),"x","y").unwrap()["nodes"]["y"].get("picker_id").is_none());
    }
    #[test] fn rename_and_paste_keep_reference_targets() {
        let doc=json!({"nodes":{"driver":{"type":"box","params":{"size":[2,3,4]}},"target":{"type":"box","channels":{"size.x":"ch('../driver/size.y')"}},"external":{"type":"box","channels":{"size.x":"ch('../target/size.x')"}}},"display":"target"});
        let renamed=rename(&doc,"driver","source").unwrap();
        assert_eq!(renamed["nodes"]["target"]["channels"]["size.x"],json!("ch('../source/size.y')"));
        let clip=copy(&renamed,&["source".into(),"target".into()]).unwrap();
        let (pasted,ids)=paste(&renamed,&clip).unwrap();
        let new_target=ids.iter().find(|id|pasted["nodes"][*id]["channels"].is_object()).unwrap();
        assert!(crate::value_link_ui::links(&pasted).iter().any(|l|l.target==*new_target && l.source!= "source" && !l.broken));
        assert_eq!(pasted["nodes"]["external"]["channels"]["size.x"],renamed["nodes"]["external"]["channels"]["size.x"]);
        let deleted=delete(&renamed,"source").unwrap();
        assert_eq!(deleted["nodes"]["target"]["channels"]["size.x"],json!("ch('../source/size.y')"));
    }
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
        assert_eq!(next["nodes"]["out"]["inputs"], base["nodes"]["out"]["inputs"]);
        assert_eq!(next["nodes"]["out"]["pos"], json!([96., 544.]));
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
            ("polysplit",json!({"split_edges":"0","mode":"edge_loop","loops":1,"position":0.5})),
            ("edge_slide",json!({"group":"p0-1","factor":0.25})),
        ] {
            let (next,id) = add_viewport_cut(&base,"out",ty,params).unwrap();
            assert_eq!(next["nodes"][&id]["inputs"]["geo"],"box1");
            assert_eq!(next["nodes"]["out"]["inputs"]["geo"],id.as_str());
            assert_eq!(next["display"],"out");
            let cut = minihou_core::Graph::from_value(&next).unwrap().cook(None).unwrap();
            if ty == "edge_slide" {
                // Sliding an existing edge changes positions, unlike the three cutting nodes which add points.
                assert_eq!(cut.npoints, original.npoints);
                assert_ne!(cut.p(), original.p());
            } else {
                assert!(cut.npoints > original.npoints, "{ty}: {} -> {} points", original.npoints, cut.npoints);
            }
            let mut doc = validated(base.clone()).unwrap(); let mut history = minihou_app::History::default();
            history.replace(&mut doc,validated(next.clone()).unwrap());
            history.undo(&mut doc).unwrap(); assert_eq!(doc.value(),&base);
            history.redo(&mut doc).unwrap(); assert_eq!(doc.value(),&next);
        }
        assert!(add_viewport_cut(&base,"box1","loop_cut",json!({})).is_none());
        assert!(add_viewport_cut(&base,"out","transform",json!({})).is_none());
    }
    #[test] fn viewport_cut_splices_only_active_output_with_three_displays() {
        let base=json!({"version":1,"display":["a","b","c"],"nodes":{
            "box_a":{"type":"box"},"box_b":{"type":"box"},"box_c":{"type":"box"},
            "a":{"type":"output","inputs":{"geo":"box_a"}},
            "b":{"type":"output","inputs":{"geo":"box_b"}},
            "c":{"type":"output","inputs":{"geo":"box_c"}}}});
        let (next,id)=add_viewport_cut(&base,"b","edge_slide",json!({"group":"p0-1","factor":0.25})).unwrap();
        assert_eq!(next["nodes"]["b"]["inputs"]["geo"],id);
        assert_eq!(next["nodes"]["a"],base["nodes"]["a"]);
        assert_eq!(next["nodes"]["c"],base["nodes"]["c"]);
        assert_eq!(next["display"],base["display"]);
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
    #[test] fn sketch_batch_follows_stream_primitive_type_and_undoes_once() {
        let strokes: Vec<(&str, &[[f32;3]], bool)> = vec![
            ("Line", &[[0.,0.,0.],[1.,0.,0.]], false),
            ("Polygon", &[[0.,0.,1.],[1.,0.,1.],[0.,1.,1.]], true),
        ];
        for (base, selected, target) in [
            (json!({"version":1,"display":"out","nodes":{"box1":{"type":"box"},"out":{"type":"output","inputs":{"geo":"box1"}}}}), "out", SketchTarget::Mesh),
            (json!({"version":1,"display":"out","nodes":{"curve1":{"type":"curve","params":{"points":"0,0,0 1,0,0"}},"out":{"type":"output","inputs":{"geo":"curve1"}}}}), "out", SketchTarget::Curve),
            (json!({"version":1,"nodes":{}}), "", SketchTarget::New),
        ] {
            let actual = if selected.is_empty() { SketchTarget::New } else {
                let g = minihou_core::Graph::from_value(&base).unwrap().cook(Some(selected)).unwrap();
                sketch_target(Some(&g))
            };
            assert_eq!(actual, target);
            let (next, id, ids) = add_sketch_batch(&base, &strokes, selected, actual).unwrap();
            assert_eq!(ids.len(), 2);
            if target == SketchTarget::Mesh {
                assert_eq!(next["nodes"][&id]["type"], "merge");
                assert_eq!(next["nodes"]["out"]["inputs"]["geo"], id);
                assert_eq!(next["display"], "out");
                assert_eq!(next["nodes"][&ids[0]]["params"]["as_mesh"], true);
            } else if target == SketchTarget::Curve {
                assert_eq!(next["nodes"][&ids[0]]["inputs"]["geo"], "curve1");
                assert_eq!(next["nodes"][&ids[1]]["inputs"]["geo"], ids[0]);
                assert_eq!(next["display"], "out");
            } else { assert_eq!(next["display"], id); }
            let cooked = minihou_core::Graph::from_value(&next).unwrap().cook(None).unwrap();
            assert!(cooked.nprims() >= 2, "{target:?}: {} primitives", cooked.nprims());
            let mut doc = validated(base.clone()).unwrap();
            let mut history = minihou_app::History::default();
            history.replace(&mut doc, validated(next).unwrap());
            history.undo(&mut doc).unwrap();
            assert_eq!(doc.value(), &base);
            assert!(history.undo(&mut doc).is_none());
        }
    }
    #[test] fn curve_blast_inserts_before_output_and_cleans_points() {
        let base = json!({
            "version": 1,
            "display": "out",
            "nodes": {
                "c": {"type": "curve", "params": {"points": "0,0,0 1,0,0 2,0,0 3,0,0", "closed": false}},
                "out": {"type": "output", "inputs": {"geo": "c"}}
            }
        });
        let original = minihou_core::Graph::from_value(&base).unwrap().cook(None).unwrap();
        assert_eq!(original.npoints, 4);

        let (next, blast_id) = add_curve_blast(&base, "out", &[1, 2]).unwrap();
        assert_eq!(next["nodes"]["out"]["inputs"]["geo"], blast_id.as_str());
        assert_eq!(next["nodes"][&blast_id]["inputs"]["geo"], "c");
        assert_eq!(next["nodes"][&blast_id]["params"]["domain"], "points");
        assert_eq!(next["nodes"][&blast_id]["params"]["group"], "1,2");
        assert_eq!(next["display"], "out");

        let cooked = minihou_core::Graph::from_value(&next).unwrap().cook(None).unwrap();
        assert_eq!(cooked.npoints, 2);

        assert!(add_curve_blast(&base, "out", &[]).is_none());
        let mesh_base = json!({"version": 1, "display": "out", "nodes": {"b": {"type": "box"}, "out": {"type": "output", "inputs": {"geo": "b"}}}});
        assert!(add_curve_blast(&mesh_base, "out", &[0]).is_none());
    }
    #[test] fn bezier_sketch_creates_bezier_node_with_controls() {
        let points = [[0., 0., 0.], [1., 1., 0.], [2., 0., 0.]];
        let (v, id) = add_bezier_sketch(&graph(), "Bezier", &points, false, "box1").unwrap();
        assert_eq!(v["nodes"][&id]["type"], "bezier");
        assert_eq!(v["nodes"][&id]["inputs"]["geo"], "box1");
        assert_eq!(v["nodes"][&id]["params"]["closed"], false);
        let controls: Value = serde_json::from_str(v["nodes"][&id]["params"]["controls"].as_str().unwrap()).unwrap();
        assert_eq!(controls.as_array().unwrap().len(), 3);
        validated(v.clone()).unwrap();

        let cooked = minihou_core::Graph::from_value(&v).unwrap().cook(None).unwrap();
        assert!(cooked.npoints >= 3);
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
    #[test]
    fn graph_multi_output_wire_reconnection() {
        let base=json!({"nodes":{
            "box":{"type":"box"},
            "fill":{"type":"flood_fill","inputs":{"geometry":"box"}},
            "pass":{"type":"null"},
            "out":{"type":"output"}
        }});
        assert!(connect_output(&base,"fill",2,"out",Some("geo")).is_none());
        let connected=connect_output(&base,"fill",1,"out",Some("geo")).unwrap();
        assert_eq!(connected["nodes"]["out"]["inputs"]["geo"],json!({"node":"fill","output":1}));
        let spliced=insert_existing(&connected,"pass",&wire("fill","out","geo")).unwrap();
        assert_eq!(spliced["nodes"]["pass"]["inputs"]["geo"],json!({"node":"fill","output":1}));
        assert_eq!(spliced["nodes"]["out"]["inputs"]["geo"],"pass");
        let cut=disconnect_wires(&connected,&BTreeSet::from([wire("fill","out","geo")])).unwrap();
        assert!(cut["nodes"]["out"]["inputs"].get("geo").is_none());
    }
    fn wire(from: &str, to: &str, port: &str) -> WireRef {
        WireRef { from: from.into(), to: to.into(), port: port.into() }
    }
    #[test]
    fn transform_drop_splices_exact_edge_once() {
        let base = json!({"nodes": {
            "grid1":{"type":"grid"},
            "noise1":{"type":"noise","inputs":{"geo":"grid1"}},
            "transform1":{"type":"transform","inputs":{"geo":"noise1"}},
            "normal1":{"type":"normal","inputs":{"geo":"transform1"}},
            "transform2":{"type":"transform","inputs":{"geo":"normal1"}},
            "output1":{"type":"output","inputs":{"geo":"transform2"}}
        },"display":"output1"});
        let (added, id) = add(&base, "transform", None).unwrap();
        assert_eq!(id, "transform3");
        let next = insert_existing(&added, &id, &wire("transform2", "output1", "geo")).unwrap();
        let graph = minihou_core::Graph::from_value(&next).unwrap();
        let edges: Vec<_> = graph.nodes.iter().flat_map(|(to, def)| def.inputs.iter().flat_map(move |(port, sources)|
            sources.iter().map(move |from| (from.as_str(), to.as_str(), port.as_str())))).collect();
        assert_eq!(edges.iter().filter(|(from, to, _)| *from == "transform2" && *to == "output1").count(), 0);
        assert_eq!(edges.iter().filter(|(from, to, _)| *from == "transform2" && *to == "transform3").count(), 1);
        assert_eq!(edges.iter().filter(|(from, to, _)| *from == "transform3" && *to == "output1").count(), 1);
        assert_eq!(next["nodes"]["transform3"]["inputs"], json!({"geo":"transform2"}));
        assert_eq!(next["nodes"]["output1"]["inputs"], json!({"geo":"transform3"}));
        let duplicate = json!({"nodes": {
            "grid1":{"type":"grid"},
            "merge1":{"type":"merge","inputs":{"geos":["grid1","grid1"]}},
            "transform1":{"type":"transform"}
        }});
        let deduped = insert_existing(&duplicate, "transform1", &wire("grid1", "merge1", "geos")).unwrap();
        assert_eq!(deduped["nodes"]["merge1"]["inputs"]["geos"], json!(["transform1"]));
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
