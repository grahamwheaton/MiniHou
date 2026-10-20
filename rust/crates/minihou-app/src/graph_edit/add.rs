use super::*;

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

