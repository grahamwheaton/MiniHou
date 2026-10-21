use super::*;

/// The desktop session's Python permission. Only the Preferences window and the Run / Skip banner change it; the HTTP API
/// and the Blender link never see it (their sessions have no gate, see docs/RUST_PYTHON_NODE.md).
/// Static-snap metadata: each clicked sketch point's inference reference descriptor
/// (docs/RUST_INFERENCE.md), stamped with the source node's stable identity like measurement anchors.
pub(super) fn attach_inference_refs(value: &mut Value, id: &str, mut refs: Vec<Option<Value>>) {
    if refs.iter().all(Option::is_none) { return; }
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
    for reference in refs.iter_mut().flatten() {
        let Some(node) = reference["node"].as_str().map(str::to_owned) else { continue; };
        if !value["nodes"][&node].is_object() { *reference = Value::Null; continue; }
        if !value["nodes"][&node]["measurement_identity"].is_string() { value["nodes"][&node]["measurement_identity"] = json!(format!("measure-{stamp}-{node}")); }
        reference["identity"] = value["nodes"][&node]["measurement_identity"].clone();
    }
    if value["nodes"][id].is_object() { value["nodes"][id]["inference_refs"] = json!(refs); }
}
pub(super) fn python_gate() -> std::sync::Arc<PythonGate> {
    static GATE: std::sync::OnceLock<std::sync::Arc<PythonGate>> = std::sync::OnceLock::new();
    GATE.get_or_init(|| std::sync::Arc::new(PythonGate::denied())).clone()
}

pub(super) fn zoom_under_cursor(camera:&mut Camera,delta:f32,enabled:bool,rect:egui::Rect,pos:Option<egui::Pos2>,point:Option<[f32;3]>) {
    if delta==0. {return;}
    let Some(pos)=pos.filter(|_|enabled) else {camera.zoom(delta);return;};
    let nx=(pos.x-rect.left())/rect.width()*2.-1.; let ny=1.-(pos.y-rect.top())/rect.height()*2.;
    let point=point.unwrap_or_else(||camera.view_plane_point(nx,ny,rect.width()/rect.height()));
    camera.zoom_at(delta,point,nx,ny,rect.width()/rect.height());
}

#[allow(dead_code)]
pub(super) fn scaled_from_initial(initial: &serde_json::Value, factor: [f32; 3]) -> [f32; 3] {
    [0,1,2].map(|i| initial.get(i).and_then(|x| x.as_f64()).unwrap_or(1.) as f32 * factor[i])
}

#[allow(dead_code)]
pub(super) fn transform_parameter(value: &Value, initial: &Document, target: &str, op: TransformOp) -> (&'static str, Value) {
    let mut initial_trs = if initial.value()["nodes"].get(target).is_some() {
        transform_math::TrsParams::from_params(&initial.value()["nodes"][target]["params"])
    } else {
        transform_math::TrsParams::default()
    };
    if let Some(pivot) = value["nodes"][target]["params"].get("pivot") {
        if let Some(arr) = pivot.as_array() {
            initial_trs.pivot = [
                arr.first().and_then(Value::as_f64).unwrap_or(0.),
                arr.get(1).and_then(Value::as_f64).unwrap_or(0.),
                arr.get(2).and_then(Value::as_f64).unwrap_or(0.),
            ];
        }
    }
    match op {
        TransformOp::Move(delta) => {
            let new_trs = transform_math::compose_translate(&initial_trs, [delta[0] as f64, delta[1] as f64, delta[2] as f64]);
            ("translate", json!(new_trs.translate.map(|x| x as f32)))
        }
        TransformOp::Rotate(delta) => {
            let axis = (0..3).max_by(|&a, &b| delta[a].abs().total_cmp(&delta[b].abs())).unwrap_or(0);
            let new_trs = transform_math::compose_rotate_axis(&initial_trs, transform_math::Frame::global(), axis, delta[axis] as f64, false);
            ("rotate", json!(new_trs.rotate.map(|x| x as f32)))
        }
        TransformOp::Scale(factor) => {
            let new_trs = match transform_math::compose_scale(
                &initial_trs,
                transform_math::Frame::global(),
                [factor[0] as f64, factor[1] as f64, factor[2] as f64],
                true,
                1e-4,
            ) {
                transform_math::ScaleResult::Direct(trs) => trs,
                transform_math::ScaleResult::ShearNeedsSecondNode { second_node, .. } => second_node,
            };
            ("scale", json!(new_trs.scale.map(|x| x as f32)))
        }
    }
}

/// Sets `params` on node `id` of a graph value, keeping its other parameters.
pub(super) fn merge_params(value: &mut Value, id: &str, params: Value) {
    let Some(node) = value["nodes"].get_mut(id).and_then(Value::as_object_mut) else { return; };
    let target = node.entry("params").or_insert_with(|| json!({}));
    if let (Some(target), Some(params)) = (target.as_object_mut(), params.as_object()) {
        for (k, v) in params { target.insert(k.clone(), v.clone()); }
    }
}
pub(super) fn group_text(ids: &[u32]) -> String { ids.iter().map(u32::to_string).collect::<Vec<_>>().join(",") }

pub(super) fn proportional_params(params: &mut Value, state: &EditorState) {
    let p = state.proportional;
    params["proportional_enabled"] = json!(p.enabled);
    if p.enabled {
        params["proportional_radius"] = json!(p.radius);
        params["proportional_falloff"] = json!(match p.falloff {
            ui::Falloff::Smooth => "smooth", ui::Falloff::Sphere => "sphere", ui::Falloff::Root => "root",
            ui::Falloff::InverseSquare => "inverse_square", ui::Falloff::Sharp => "sharp", ui::Falloff::Linear => "linear",
            ui::Falloff::Constant => "constant", ui::Falloff::Random => "random",
        });
        params["proportional_connected"] = json!(p.connected_only);
    }
}

pub(super) fn viewport_tool_drop(ctx: &egui::Context, rect: egui::Rect) -> (Option<std::sync::Arc<ui::ToolDrag>>, bool, Option<egui::Pos2>) {
    let pointer_pos = ctx.input(|i| i.pointer.latest_pos());
    let over = pointer_pos.is_some_and(|p| rect.contains(p));
    let dragging = over && egui::DragAndDrop::has_payload_of_type::<ui::ToolDrag>(ctx);
    let released = ctx.input(|i| i.pointer.any_released());
    let dropped = if dragging && released { egui::DragAndDrop::take_payload::<ui::ToolDrag>(ctx) } else { None };
    (dropped, dragging && !released, if released { pointer_pos } else { None })
}
pub(super) fn apply_transform_step(doc: &mut Document, first: bool, value: Value, target: &str, name: &str, param_value: Value) -> std::result::Result<Vec<Edit>, String> {
    let param = Edit::Param { node: target.into(), name: name.into(), value: param_value };
    if first {
        let text = serde_json::to_string(&value).map_err(|e| e.to_string())?;
        *doc = Document::parse(text).map_err(|e| e.to_string())?;
        Ok(vec![Edit::Replace(value), param])
    } else {
        doc.edit(&param).map_err(|e| e.to_string())?;
        Ok(vec![param])
    }
}
pub(super) fn ensure_transform_target(value: &mut Value, source: &str, object_mode: bool, active: Option<&str>) -> Option<String> {
    graph_edit::repair_output_hangs(value);
    if let Some(id) = active {
        if value["nodes"].get(id).is_some()
            && !(value["nodes"][source]["type"] == "output"
                && value["nodes"][id]["inputs"]["geo"] == source) {
            return Some(id.to_owned());
        }
    }
    let displayed = minihou_app::display_nodes(value);
    let nodes = value["nodes"].as_object_mut()?;
    if object_mode && nodes.get(source).and_then(|n| n["type"].as_str()) == Some("transform") { return Some(source.to_owned()); }
    if object_mode {
        if let Some(id) = nodes.iter().find(|(id, n)| displayed.iter().any(|shown| shown == *id) && n["type"] == "transform" && n["inputs"]["geo"] == source).map(|(id, _)| id.clone()) { return Some(id); }
    }
    if nodes.get(source).and_then(|n| n["type"].as_str()) == Some("output") {
        let up = nodes[source]["inputs"]["geo"].as_str().map(str::to_owned);
        if object_mode {
            if let Some(id) = nodes.iter().find(|(_, n)| n["type"] == "transform" && n["inputs"]["geo"] == source).map(|(id, _)| id.clone()) {
                let upstream = up?;
                nodes[&id]["inputs"]["geo"] = json!(upstream);
                nodes[source]["inputs"]["geo"] = json!(id);
                return Some(id);
            }
        }
        if let Some(up) = up.filter(|up| object_mode && nodes.get(up).is_some_and(|n| n["type"] == "transform")) { return Some(up); }
        // Never hang a node off the end of the stream: insert it just before the output.
        let (next, id) = graph_edit::add(value, "transform", Some(source))?;
        *value = next;
        forward_measurement_refs(value,source,&id);
        return Some(id);
    }
    // Splice every consumer of the selected node, including a displayed output.
    let (next, id) = graph_edit::add(value, "transform", Some(source))?;
    *value = next;
    forward_measurement_refs(value, source, &id);
    Some(id)
}
/// Adds a transform target to the display list unless something displayed already leads to it.
pub(super) fn show_transform_target(value: &mut Value, target: &str) {
    if feeds_displayed_node(value, target) { return; }
    let mut shown = minihou_app::display_nodes(value);
    shown.push(target.to_owned());
    value["display"] = if shown.len() == 1 { json!(shown[0]) } else { json!(shown) };
}
#[cfg(test)]
mod show_transform_target_tests {
    use super::*;
    #[test] fn adds_unshown_target_and_keeps_existing_display() {
        let mut doc=json!({"display":"box","nodes":{"box":{"type":"box"},"t":{"type":"transform","inputs":{"geo":"box"}},"o":{"type":"output","inputs":{"geo":"box"}}}});
        show_transform_target(&mut doc,"t");
        assert_eq!(doc["display"],json!(["box","t"]));
    }
    #[test] fn leaves_display_alone_when_an_output_already_shows_the_target() {
        let mut doc=json!({"display":"o","nodes":{"box":{"type":"box"},"t":{"type":"transform","inputs":{"geo":"box"}},"o":{"type":"output","inputs":{"geo":"t"}}}});
        show_transform_target(&mut doc,"t");
        assert_eq!(doc["display"],"o");
    }
}
pub(super) fn forward_measurement_refs(value:&mut Value,source:&str,target:&str) {
    let identity=value["nodes"][source]["measurement_identity"].clone();
    if !identity.is_string() {return;}
    value["nodes"][target]["measurement_identity"]=identity.clone();
    if let Some(rows)=value["measurements"].as_array_mut() {
        for row in rows {if let Some(refs)=row["refs"].as_array_mut() {
            for reference in refs {if reference["node"]==source && reference["identity"]==identity {
                reference["node"]=json!(target);
            }}
        }}
    }
}
#[cfg(test)]
#[test]
pub(super) fn transform_keeps_measurement_point_identity() {
    let mut doc=json!({"nodes":{"a":{"measurement_identity":"stable"},"b":{}},
        "measurements":[{"refs":[{"node":"a","identity":"stable","point":2},null]}]});
    forward_measurement_refs(&mut doc,"a","b");
    assert_eq!(doc["measurements"][0]["refs"][0]["node"],"b");
    assert_eq!(doc["nodes"]["b"]["measurement_identity"],"stable");
}
/// Alt+E with faces selected starts Push/Pull on them immediately; without, it waits for a face pick.
pub(super) fn pushpull_starts_now(mode: ui::SelectionMode, selection: &picking::Selection) -> bool {
    mode == ui::SelectionMode::Faces && selection.mode == ui::SelectionMode::Faces && !selection.tris.is_empty()
}
/// The active node after undo/redo: kept when it still exists, else the displayed node.
pub(super) fn surviving_selection(value: &Value, selected: &str, display: &str) -> String {
    if value["nodes"].get(selected).is_some() { selected.to_owned() } else { display.to_owned() }
}
/// As `surviving_selection`, but a selected node the undo removed hands the selection to the node that fed it
/// (the one a push/pull was inserted after), not to whatever the display shows.
pub(super) fn surviving_selection_from(old:&Value,new:&Value,selected:&str,display:&str)->String {
    let mut node=selected.to_owned();
    for _ in 0..64 {
        if new["nodes"].get(&node).is_some() { return node; }
        let source=old["nodes"][&node]["inputs"].as_object().and_then(|inputs|inputs.values().find_map(|input|
            input.as_str().map(str::to_owned).or_else(||input.as_array().and_then(|a|a.first()).and_then(Value::as_str).map(str::to_owned))));
        match source { Some(source) => node=source, None => break }
    }
    surviving_selection(new,selected,display)
}
pub(super) fn feeds_displayed_node(value: &Value, selected: &str) -> bool {
    let mut pending = minihou_app::display_nodes(value);
    let mut seen = std::collections::BTreeSet::new();
    while let Some(id) = pending.pop() {
        if !seen.insert(id.clone()) { continue; }
        if id == selected { return true; }
        if let Some(inputs) = value["nodes"][&id]["inputs"].as_object() {
            for input in inputs.values() {
                if let Some(source) = input.as_str() { pending.push(source.to_owned()); }
                else if let Some(sources) = input.as_array() {
                    pending.extend(sources.iter().filter_map(Value::as_str).map(str::to_owned));
                }
            }
        }
    }
    false
}
pub(super) fn feeds_node(value: &Value, selected: &str, target: &str) -> bool {
    if selected.is_empty() || target.is_empty() { return false; }
    let mut pending = vec![target.to_owned()];
    let mut seen = std::collections::BTreeSet::new();
    while let Some(id) = pending.pop() {
        if !seen.insert(id.clone()) { continue; }
        if id == selected { return true; }
        if let Some(inputs) = value["nodes"][&id]["inputs"].as_object() {
            for input in inputs.values() {
                if let Some(source) = input.as_str() { pending.push(source.to_owned()); }
                else if let Some(sources) = input.as_array() {
                    pending.extend(sources.iter().filter_map(Value::as_str).map(str::to_owned));
                }
            }
        }
    }
    false
}
pub(crate) fn resolve_displayed_node<'a>(value: &Value, selected: &str, display_ranges: &'a [(String, std::ops::Range<u32>)]) -> Option<&'a str> {
    if selected.is_empty() { return None; }
    if let Some((id, _)) = display_ranges.iter().find(|(id, _)| id == selected) {
        if feeds_displayed_node(value, selected) {
            return Some(id.as_str());
        }
    }
    display_ranges.iter().find(|(id, _)| feeds_node(value, selected, id)).map(|(id, _)| id.as_str())
}
pub(crate) fn resolve_active_displayed_node<'a>(value: &Value, selected: &str,
    display_ranges: &'a [(String, std::ops::Range<u32>)], output_range_count: usize, selected_preview: bool) -> Option<&'a str> {
    let (outputs, previews) = display_ranges.split_at(output_range_count);
    resolve_displayed_node(value, selected, outputs).or_else(|| {
        selected_preview.then(|| previews.iter().find(|(id, _)| id == selected).map(|(id, _)| id.as_str())).flatten()
    })
}

pub(super) fn include_selected_preview(scene: &mut Scene, ranges: &mut Vec<(String, std::ops::Range<u32>)>, selected: &str, preview: Option<&minihou_core::Geometry>) -> bool {
    if selected.is_empty() || ranges.iter().any(|(id, _)| id == selected) { return false; }
    let Some(preview) = preview else { return false; };
    let range = scene.append_selected(&Scene::new(preview));
    ranges.push((selected.to_owned(), range));
    true
}

/// Where a viewport modelling node (extrude, push/pull, bevel...) is inserted. Element ids come from the displayed
/// scene, so the node must take the displayed geometry: the selected node only when it exists and is what the
/// display shows (itself, or the node feeding a displayed output); otherwise the display (e.g. after an undo removed
/// the selected node, or an upstream node is selected whose ids differ from the scene).
pub(super) fn modeling_source(value: &Value, selected: &str, display: &str) -> String {
    let nodes = &value["nodes"];
    let shows = |id: &str| id == display || (nodes[display]["type"] == "output" && nodes[display]["inputs"]["geo"] == id);
    if !selected.is_empty() && nodes.get(selected).is_some() && shows(selected) { return selected.to_owned(); }
    if !display.is_empty() && nodes.get(display).is_some() { return display.to_owned(); }
    minihou_app::display_nodes(value).into_iter().next().unwrap_or_else(|| selected.to_owned())
}
pub(super) fn toggle_edit_mode(current: ui::SelectionMode, last: &mut ui::SelectionMode) -> ui::SelectionMode {
    if current == ui::SelectionMode::Object { *last } else { *last = current; ui::SelectionMode::Object }
}
/// Element modes picked by 1-4 or the mode buttons become the mode Ctrl+Tab returns to.
pub(super) fn remember_element_mode(mode: ui::SelectionMode, last: &mut ui::SelectionMode) {
    if mode != ui::SelectionMode::Object { *last = mode; }
}
/// egui's focus pass treats every Tab press (Ctrl included) as "focus next widget" when nothing is
/// focused, so the first Ctrl+Tab parked keyboard focus in the first text field and the keymap
/// (which ignores keys while egui wants keyboard input) never saw the second press. Ctrl+Tab is
/// held back from egui while nothing is focused and replayed into the frame input for the keymap.
pub(super) fn hold_back_ctrl_tab(raw: &mut egui::RawInput, widget_focused: bool) -> Vec<egui::Event> {
    if widget_focused { return Vec::new(); }
    let (held, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut raw.events).into_iter()
        .partition(|e| matches!(e, egui::Event::Key { key: egui::Key::Tab, modifiers, .. } if modifiers.ctrl || modifiers.command));
    raw.events = kept;
    held
}
pub(super) fn display_is_empty(value: &Value) -> bool {
    let Some(nodes)=value["nodes"].as_object() else {return false};
    if nodes.is_empty() {return true;}
    if value["display"].is_array() || value["display"].is_string() {
        minihou_app::display_nodes(value).iter().all(|id|!nodes.contains_key(id))
    } else {
        !nodes.values().any(|node|node["type"]=="output")
    }
}

#[cfg(test)]
#[test]
pub(super) fn empty_display_is_distinct_from_a_valid_graph() {
    assert!(display_is_empty(&json!({"nodes":{}})));
    assert!(display_is_empty(&json!({"nodes":{"a":{"type":"box"}},"display":[]})));
    assert!(display_is_empty(&json!({"nodes":{"a":{"type":"box"}},"display":"deleted"})));
    assert!(display_is_empty(&json!({"nodes":{"a":{"type":"box"}}})));
    assert!(!display_is_empty(&json!({"nodes":{"a":{"type":"box"}},"display":["a"]})));
}

pub(super) fn picked_object<'a>(ranges: &'a [(String, std::ops::Range<u32>)], triangles: &[u32], hit: picking::Hit) -> Option<&'a str> {
    let point = *triangles.get(hit.tri * 3)?;
    ranges.iter().find(|(_, range)| range.contains(&point)).map(|(id, _)| id.as_str())
}

/// SOP outputs have no separate object transform. Their origin follows the
/// primitive's center/origin through whole-object transforms on the geo chain.
pub(super) fn object_origin(value: &Value, id: &str) -> [f32; 3] {
    let mut chain = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    let mut current = id;
    let mut point = [0.; 3];
    while seen.insert(current.to_owned()) {
        let node = &value["nodes"][current];
        if node["type"] == "transform" && !node["flags"]["bypass"].as_bool().unwrap_or(false)
            && node["params"]["group"].as_str().unwrap_or("").is_empty() {
            chain.push(transform_math::TrsParams::from_params(&node["params"]).to_matrix());
        }
        if let Some(input) = node["inputs"]["geo"].as_str() { current = input; }
        else {
            let params = &node["params"];
            let origin = params.get("center").or_else(|| params.get("origin"));
            point = [0,1,2].map(|i| origin.and_then(|p| p.get(i)).and_then(Value::as_f64).unwrap_or(0.));
            break;
        }
    }
    for matrix in chain.into_iter().rev() {
        point = [0,1,2].map(|i| matrix[i][3] + (0..3).map(|j| matrix[i][j] * point[j]).sum::<f64>());
    }
    point.map(|x| x as f32)
}

#[cfg(test)]
#[test]
pub(super) fn selection_set_replaces_toggles_and_extends() {
    let mut set = std::collections::BTreeSet::new();
    select_nodes(&mut set, ["a".into()], false, false);
    select_nodes(&mut set, ["b".into()], false, true);
    assert_eq!(set, std::collections::BTreeSet::from(["a".into(), "b".into()]));
    select_nodes(&mut set, ["a".into()], false, true);
    assert_eq!(set, std::collections::BTreeSet::from(["b".into()]));
    select_nodes(&mut set, ["c".into()], true, false);
    assert_eq!(set.len(), 2);
    select_nodes(&mut set, ["d".into()], false, false);
    assert_eq!(set, std::collections::BTreeSet::from(["d".into()]));
}

#[derive(Clone)]
pub(super) struct PrimitivePreview {
    pub(super) lines: Vec<([f32; 3], [f32; 3])>,
    pub(super) radius: f32,
}

pub(super) fn get_primitive_preview(node_type: &str) -> Option<PrimitivePreview> {
    use std::sync::Mutex;
    static CACHE: Mutex<Option<HashMap<String, PrimitivePreview>>> = Mutex::new(None);
    let mut guard = CACHE.lock().ok()?;
    let map = guard.get_or_insert_with(HashMap::new);
    if !map.contains_key(node_type) {
        let val = serde_json::json!({
            "nodes": {
                "n": { "type": node_type }
            }
        });
        if let Ok(graph) = minihou_core::Graph::from_value(&val) {
            if let Ok(geo) = graph.cook(Some("n")) {
                let (p, t) = (geo.p(), &geo.topology);
                let l = minihou_core::preview::layout(
                    p,
                    &t.vertex_point,
                    &t.prim_offsets,
                    t.closed.as_deref(),
                    geo.npoints,
                    usize::MAX,
                    usize::MAX,
                    10_000,
                    1024,
                );
                let mut lines = Vec::new();
                for pair in l.edge.as_chunks::<2>().0 {
                    let a = pair[0] as usize * 3;
                    let b = pair[1] as usize * 3;
                    if a + 2 < p.len() && b + 2 < p.len() {
                        lines.push(([p[a], p[a + 1], p[a + 2]], [p[b], p[b + 1], p[b + 2]]));
                    }
                }
                if lines.is_empty() && geo.npoints > 0 {
                    for i in 0..geo.npoints {
                        let k = i * 3;
                        let pt = [p[k], p[k + 1], p[k + 2]];
                        for axis in 0..3 {
                            let mut lo = pt;
                            let mut hi = pt;
                            lo[axis] -= 0.1;
                            hi[axis] += 0.1;
                            lines.push((lo, hi));
                        }
                    }
                }
                let (_, radius) = if geo.npoints == 0 {
                    ([0.; 3], 1.0)
                } else {
                    let (lo, hi) = minihou_core::preview::bounds(p);
                    let r = (0..3).map(|k| (hi[k] - lo[k]).powi(2)).sum::<f32>().sqrt() * 0.5;
                    ([0.; 3], r.max(1e-3))
                };
                map.insert(node_type.to_string(), PrimitivePreview { lines, radius });
            }
        }
    }
    map.get(node_type).cloned()
}

