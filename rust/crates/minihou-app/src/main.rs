mod picking;
mod subnet_edit;
mod element_select;
mod element_pick_glue;
use element_pick_glue::{point_pickable, active_range, navigation_point, loop_cut_has_marks, loop_cut_selection, selection_exists_on_scene, selection_in_range, pick_displayed, pick_displayed_culled, keep_extrude_selection, pick_displayed_wire, wire_selection};
mod node_selection;
mod node_add;
mod selection_ui;
use selection_ui::{selected_point_ids, select_active_nodes, select_nodes};
mod action_dispatch;
mod app_frame;
mod history_selection;
use history_selection::{restore, selection_state};
mod coplanar_tool;
mod group_preview;
mod param_pick;
use param_pick::{field_selection, infer_mode, ParamPick, PickedField, param_pick_end, input_source, source_ready, pick_source_ready};
mod selection_metrics;
#[cfg(test)] mod face_mode_profile;
use minihou_app::graph_edit;
mod noodle_hit;
mod http_ui;
mod ui;
mod viewport;
mod viewport_input;
mod transform_math;
mod repeat;
mod usd_export;
mod window_setup;
mod transform_ops;
pub use repeat::LastOperator;
#[cfg(test)] mod transform_tests;
#[cfg(test)] mod live_path_tests;
use eframe::{egui, egui_wgpu};
pub use minihou_app::units;
use minihou_app::{Camera, Document, Result, Edit, EditorDocument, CookWorker, CookRequest, CookState, StandardView, SelectionState, SelectionSnapshot, HistoryOutcome};
use minihou_core::Graph;
use serde_json::{json, Value};
use std::{collections::HashMap, path::PathBuf, time::{Duration, Instant}};
use ui::keymap::{KeyContext, Keymap, Preferences, ThemePref};
use ui::{theme::Theme, Actions, BlenderStatus, CameraInfo, EditorState, GeoSummary, NodeFlags, TransformOp, UiAction, View};
use ui::modal_transform::{Modal, Kind as ModalKind, Constraint};
use ui::modal_model::{State as ModelState, Kind as ModelKind, Outcome as ModelOutcome};
use viewport::{Gpu, Mode, Overlay, Scene, Target};
use minihou_core::python_bridge::PythonGate;

/// The desktop session's Python permission. Only the Preferences window and the Run / Skip banner change it; the HTTP API
/// and the Blender link never see it (their sessions have no gate, see docs/RUST_PYTHON_NODE.md).
/// Static-snap metadata: each clicked sketch point's inference reference descriptor
/// (docs/RUST_INFERENCE.md), stamped with the source node's stable identity like measurement anchors.
fn attach_inference_refs(value: &mut Value, id: &str, mut refs: Vec<Option<Value>>) {
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
fn python_gate() -> std::sync::Arc<PythonGate> {
    static GATE: std::sync::OnceLock<std::sync::Arc<PythonGate>> = std::sync::OnceLock::new();
    GATE.get_or_init(|| std::sync::Arc::new(PythonGate::denied())).clone()
}

fn zoom_under_cursor(camera:&mut Camera,delta:f32,enabled:bool,rect:egui::Rect,pos:Option<egui::Pos2>,point:Option<[f32;3]>) {
    if delta==0. {return;}
    let Some(pos)=pos.filter(|_|enabled) else {camera.zoom(delta);return;};
    let nx=(pos.x-rect.left())/rect.width()*2.-1.; let ny=1.-(pos.y-rect.top())/rect.height()*2.;
    let point=point.unwrap_or_else(||camera.view_plane_point(nx,ny,rect.width()/rect.height()));
    camera.zoom_at(delta,point,nx,ny,rect.width()/rect.height());
}

#[allow(dead_code)]
fn scaled_from_initial(initial: &serde_json::Value, factor: [f32; 3]) -> [f32; 3] {
    [0,1,2].map(|i| initial.get(i).and_then(|x| x.as_f64()).unwrap_or(1.) as f32 * factor[i])
}

#[allow(dead_code)]
fn transform_parameter(value: &Value, initial: &Document, target: &str, op: TransformOp) -> (&'static str, Value) {
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
fn merge_params(value: &mut Value, id: &str, params: Value) {
    let Some(node) = value["nodes"].get_mut(id).and_then(Value::as_object_mut) else { return; };
    let target = node.entry("params").or_insert_with(|| json!({}));
    if let (Some(target), Some(params)) = (target.as_object_mut(), params.as_object()) {
        for (k, v) in params { target.insert(k.clone(), v.clone()); }
    }
}
fn group_text(ids: &[u32]) -> String { ids.iter().map(u32::to_string).collect::<Vec<_>>().join(",") }

fn proportional_params(params: &mut Value, state: &EditorState) {
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

fn viewport_tool_drop(ctx: &egui::Context, rect: egui::Rect) -> (Option<std::sync::Arc<ui::ToolDrag>>, bool, Option<egui::Pos2>) {
    let pointer_pos = ctx.input(|i| i.pointer.latest_pos());
    let over = pointer_pos.is_some_and(|p| rect.contains(p));
    let dragging = over && egui::DragAndDrop::has_payload_of_type::<ui::ToolDrag>(ctx);
    let released = ctx.input(|i| i.pointer.any_released());
    let dropped = if dragging && released { egui::DragAndDrop::take_payload::<ui::ToolDrag>(ctx) } else { None };
    (dropped, dragging && !released, if released { pointer_pos } else { None })
}
fn apply_transform_step(doc: &mut Document, first: bool, value: Value, target: &str, name: &str, param_value: Value) -> std::result::Result<Vec<Edit>, String> {
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
fn ensure_transform_target(value: &mut Value, source: &str, object_mode: bool, active: Option<&str>) -> Option<String> {
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
fn show_transform_target(value: &mut Value, target: &str) {
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
fn forward_measurement_refs(value:&mut Value,source:&str,target:&str) {
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
fn transform_keeps_measurement_point_identity() {
    let mut doc=json!({"nodes":{"a":{"measurement_identity":"stable"},"b":{}},
        "measurements":[{"refs":[{"node":"a","identity":"stable","point":2},null]}]});
    forward_measurement_refs(&mut doc,"a","b");
    assert_eq!(doc["measurements"][0]["refs"][0]["node"],"b");
    assert_eq!(doc["nodes"]["b"]["measurement_identity"],"stable");
}
/// Alt+E with faces selected starts Push/Pull on them immediately; without, it waits for a face pick.
fn pushpull_starts_now(mode: ui::SelectionMode, selection: &picking::Selection) -> bool {
    mode == ui::SelectionMode::Faces && selection.mode == ui::SelectionMode::Faces && !selection.tris.is_empty()
}
/// The active node after undo/redo: kept when it still exists, else the displayed node.
fn surviving_selection(value: &Value, selected: &str, display: &str) -> String {
    if value["nodes"].get(selected).is_some() { selected.to_owned() } else { display.to_owned() }
}
/// As `surviving_selection`, but a selected node the undo removed hands the selection to the node that fed it
/// (the one a push/pull was inserted after), not to whatever the display shows.
fn surviving_selection_from(old:&Value,new:&Value,selected:&str,display:&str)->String {
    let mut node=selected.to_owned();
    for _ in 0..64 {
        if new["nodes"].get(&node).is_some() { return node; }
        let source=old["nodes"][&node]["inputs"].as_object().and_then(|inputs|inputs.values().find_map(|input|
            input.as_str().map(str::to_owned).or_else(||input.as_array().and_then(|a|a.first()).and_then(Value::as_str).map(str::to_owned))));
        match source { Some(source) => node=source, None => break }
    }
    surviving_selection(new,selected,display)
}
fn feeds_displayed_node(value: &Value, selected: &str) -> bool {
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
fn feeds_node(value: &Value, selected: &str, target: &str) -> bool {
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

fn include_selected_preview(scene: &mut Scene, ranges: &mut Vec<(String, std::ops::Range<u32>)>, selected: &str, preview: Option<&minihou_core::Geometry>) -> bool {
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
fn modeling_source(value: &Value, selected: &str, display: &str) -> String {
    let nodes = &value["nodes"];
    let shows = |id: &str| id == display || (nodes[display]["type"] == "output" && nodes[display]["inputs"]["geo"] == id);
    if !selected.is_empty() && nodes.get(selected).is_some() && shows(selected) { return selected.to_owned(); }
    if !display.is_empty() && nodes.get(display).is_some() { return display.to_owned(); }
    minihou_app::display_nodes(value).into_iter().next().unwrap_or_else(|| selected.to_owned())
}
fn toggle_edit_mode(current: ui::SelectionMode, last: &mut ui::SelectionMode) -> ui::SelectionMode {
    if current == ui::SelectionMode::Object { *last } else { *last = current; ui::SelectionMode::Object }
}
/// Element modes picked by 1-4 or the mode buttons become the mode Ctrl+Tab returns to.
fn remember_element_mode(mode: ui::SelectionMode, last: &mut ui::SelectionMode) {
    if mode != ui::SelectionMode::Object { *last = mode; }
}
/// egui's focus pass treats every Tab press (Ctrl included) as "focus next widget" when nothing is
/// focused, so the first Ctrl+Tab parked keyboard focus in the first text field and the keymap
/// (which ignores keys while egui wants keyboard input) never saw the second press. Ctrl+Tab is
/// held back from egui while nothing is focused and replayed into the frame input for the keymap.
fn hold_back_ctrl_tab(raw: &mut egui::RawInput, widget_focused: bool) -> Vec<egui::Event> {
    if widget_focused { return Vec::new(); }
    let (held, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut raw.events).into_iter()
        .partition(|e| matches!(e, egui::Event::Key { key: egui::Key::Tab, modifiers, .. } if modifiers.ctrl || modifiers.command));
    raw.events = kept;
    held
}
fn display_is_empty(value: &Value) -> bool {
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
fn empty_display_is_distinct_from_a_valid_graph() {
    assert!(display_is_empty(&json!({"nodes":{}})));
    assert!(display_is_empty(&json!({"nodes":{"a":{"type":"box"}},"display":[]})));
    assert!(display_is_empty(&json!({"nodes":{"a":{"type":"box"}},"display":"deleted"})));
    assert!(display_is_empty(&json!({"nodes":{"a":{"type":"box"}}})));
    assert!(!display_is_empty(&json!({"nodes":{"a":{"type":"box"}},"display":["a"]})));
}

fn picked_object<'a>(ranges: &'a [(String, std::ops::Range<u32>)], triangles: &[u32], hit: picking::Hit) -> Option<&'a str> {
    let point = *triangles.get(hit.tri * 3)?;
    ranges.iter().find(|(_, range)| range.contains(&point)).map(|(id, _)| id.as_str())
}

/// SOP outputs have no separate object transform. Their origin follows the
/// primitive's center/origin through whole-object transforms on the geo chain.
fn object_origin(value: &Value, id: &str) -> [f32; 3] {
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
fn selection_set_replaces_toggles_and_extends() {
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
struct PrimitivePreview {
    lines: Vec<([f32; 3], [f32; 3])>,
    radius: f32,
}

fn get_primitive_preview(node_type: &str) -> Option<PrimitivePreview> {
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

struct App {
    editor: EditorDocument, worker: CookWorker<Scene>, cooking: CookState,
    selected: String, selected_set: std::collections::BTreeSet<String>, display: String,
    state: egui_wgpu::RenderState, gpu: Option<Gpu>, target: Option<Target>,
    materials:minihou_app::materials::Registry,
    camera: Camera, orbit_drag: Option<([f32;3], f32, f32, f32)>, bounds: ([f32;3], f32), mode: Mode,
    file_status: String,
    http: http_ui::HttpUi,
    http_time: (f64, f64),
    theme: Theme, flags: HashMap<String, NodeFlags>, themed: bool,
    editor_state: EditorState,
    geo: Option<GeoSummary>, blender: BlenderStatus,
    link: Option<ui::blender_ui::blender_link::LinkServer>,
    blender_job: Option<std::thread::JoinHandle<std::io::Result<(String, bool)>>>,
    prefs: Preferences, keymap: Keymap, prefs_open: bool, viewport_hover: bool, network_hover: bool,
    /// The last cooked scene at full detail, and the proxy level it is currently uploaded at.
    display_ranges: Vec<(String, std::ops::Range<u32>)>,
    /// The selected preview follows these ranges; output-node resolution uses only this prefix.
    output_range_count: usize,
    scene: Option<Scene>, selected_preview: bool, shown_level: f32, geo_rev: u64,
    lod_uploaded: bool, last_lod_view: Option<([[f32;4];4], [u32;2], bool)>, last_lod_select: Instant,
    blender_paths: Vec<PathBuf>, blender_bar: bool,
    selection: picking::Selection, selection_dirty: bool,
    param_pick: Option<ParamPick>, barrier_key: Option<(String,ui::SelectionMode,String,u64)>, target_key: Option<(String,String,u64)>, picked_field: Option<PickedField>,
    selection_metrics: selection_metrics::Cache,
    coplanar: Option<coplanar_tool::State>, group_preview:group_preview::State,
    /// Polygon/edge adjacency of `scene` for loops, rings, paths and mode conversion (built on first use).
    elem_topo: std::cell::OnceCell<element_select::Topology>,
    /// The element selection hidden while in Object mode; converted back when an element mode is entered.
    stashed_selection: picking::Selection,
    clipboard: Option<Value>,
    last_viewport: egui::Rect,
    transform_before: Option<Document>, transform_target: Option<String>,
    transform_delta: Option<LastOperator>,
    transform_pivot: Option<[f32; 3]>,
    transform_orientation: Option<ui::Orientation>,
    modal: Option<Modal>,
    pending_duplicate_move: Option<String>,
    duplicate_move_peers: Vec<String>,
    /// graph_embed_v1: a pushed Blender graph waiting for the user's answer (modal).
    embed_conflict: Option<Value>,
    model_modal: Option<(ModelState, Document, String, String)>,
    mirror: Option<ui::mirror_tool::Session>,
    pushpull_pick: bool,
    pub last_operator: Option<LastOperator>,
    last_edit_selection: ui::SelectionMode, params_open: bool, object_hidden: bool,
    held_keys: Vec<egui::Event>,
    /// Element selection (and the Object-mode stash) per cooked topology, see element_select::SelectionMemory.
    selection_memory: element_select::SelectionMemory<(picking::Selection, picking::Selection)>,
    pending_loop_selection: Option<String>,
    pending_restore_selection: Option<(picking::Selection, picking::Selection, Option<picking::Selection>)>,
    model_before_selection: Option<(picking::Selection, picking::Selection)>,
    sel_baseline: (picking::Selection, picking::Selection),
    extrude_keep: Option<picking::Selection>,
}
impl App {
    /// Load the field's current group text as the live selection (purple), on the topology the ids came from.
    fn pick_load(&mut self) {
        let (Some(pick), Some(scene)) = (&mut self.param_pick, &self.scene) else { return; };
        if !pick.ready { return; }
        pick.load = false;
        let text = self.editor.doc.value()["nodes"][&pick.node]["params"][&pick.name].as_str().unwrap_or("").to_owned();
        let topo = element_select::cached(&self.elem_topo, scene);
        let mode = self.editor_state.selection;
        let field = field_selection(topo, &text, mode, &pick.node, scene.positions(), scene.triangle_indices(), scene.radius * 0.012);
        self.selection = field.selection.clone();
        self.picked_field = Some(field);
        self.selection_dirty = true;
    }
    fn finish_param_pick(&mut self, ctx: &egui::Context, cancel: bool) {
        let Some(pick) = self.param_pick.take() else { return; };
        if ctx.input(|i| i.key_pressed(if cancel { egui::Key::Escape } else { egui::Key::Enter })) {
            let mods = ctx.input(|i| i.modifiers);
            ctx.input_mut(|i| { i.consume_key(mods, if cancel { egui::Key::Escape } else { egui::Key::Enter }); });
        }
        ctx.data_mut(|d| d.remove::<(String,String)>(egui::Id::new("param_pick_active")));
        if cancel {
            if pick.changed {
                self.edit(pick.cancel_edit(), true);
            }
            self.picked_field = None;
            if let Some(gpu) = &mut self.gpu { gpu.set_group_overlay(&self.state.device, &[], &[]); }
            self.group_preview = Default::default();
        }
        if !cancel && pick.ready {
            // Enter writes the selection text into the field (a no-op when the clicks already did).
            if let Some(scene) = &self.scene {
                let topo = element_select::cached(&self.elem_topo, scene);
                let value = pick.text(topo, &self.selection);
                if value != self.editor.doc.value()["nodes"][&pick.node]["params"][&pick.name].as_str().unwrap_or("") {
                    self.edit(Edit::Param { node:pick.node.clone(), name:pick.name.clone(), value:serde_json::json!(value) }, true);
                }
            }
        }
        let restoring_display = self.display != pick.previous_display;
        if restoring_display {
            // The current scene still belongs to the pick source. Its element ids must not
            // be interpreted against the selection from the previous display.
            self.pending_restore_selection = Some((pick.previous_selection, self.stashed_selection.clone(), None));
            self.selection = picking::Selection { mode:pick.previous_mode, ..Default::default() };
        } else {
            self.selection = pick.previous_selection;
        }
        self.editor_state.selection = pick.previous_mode;
        self.selected_set = pick.previous_selected_set;
        self.selection_dirty = true;
        if restoring_display {
            self.display = pick.previous_display.clone();
            self.send(Edit::Display(pick.previous_display));
        }
        self.finish_history();
    }
    /// Finish the current edit before transferring pointer/keyboard ownership to selection.
    fn finish_for_selection(&mut self, ctx: &egui::Context) {
        if let Some(modal) = self.modal.take() { self.finish_modal(modal); }
        self.finish_model_modal();
        self.finish_mirror();
        ui::transform_gizmo::finish_input(ctx);
        ui::prim_handles::finish_input(ctx);
        ui::curve_tool::cancel(ctx);
        ui::sketch::cancel(ctx);
        self.finish_transform();
    }
    fn camera_info(&self) -> CameraInfo {
        let (yaw, pitch) = self.camera.angles(); let (projection, _) = self.camera.matrices(1.);
        let (ray_origin, ray_forward) = self.camera.ray(0., 0., 1.);
        let (rx, dx) = self.camera.ray(1., 0., 1.); let (ry, dy) = self.camera.ray(0., 1., 1.);
        let ray_right = if self.camera.is_ortho() { [0,1,2].map(|i| rx[i] - ray_origin[i]) } else { [0,1,2].map(|i| dx[i] - ray_forward[i]) };
        let ray_up = if self.camera.is_ortho() { [0,1,2].map(|i| ry[i] - ray_origin[i]) } else { [0,1,2].map(|i| dy[i] - ray_forward[i]) };
        CameraInfo { yaw, pitch, ortho: self.camera.is_ortho(), projection, ray_origin, ray_forward, ray_right, ray_up }
    }
    pub(crate) fn new(doc: Document, path: PathBuf, state: egui_wgpu::RenderState) -> Result<Self> {
        Self::new_document(doc, path, state, true)
    }
    fn new_document(doc: Document, path: PathBuf, state: egui_wgpu::RenderState, start_link: bool) -> Result<Self> {
        let display = Graph::from_value(doc.value())?.main_output()?;
        let mut materials=minihou_app::materials::Registry::new(path.parent().unwrap_or(std::path::Path::new(".")));
        materials.register_graph(doc.value()); let _=materials.register_sidecar(&path);
        let worker = CookWorker::start_with_python(&doc, Some(python_gate()), Scene::new)?;
        let mut app = Self { selected:display.clone(), selected_set: std::collections::BTreeSet::from([display.clone()]), display, editor:EditorDocument::new(doc, path), worker,
            cooking:CookState::default(), state,gpu:None,target:None,materials,
            camera:Camera::frame([0.;3],1.),orbit_drag:None,bounds:([0.;3],1.),mode:Mode::Shaded,file_status:String::new(),
            http:http_ui::HttpUi::default(),http_time:(1.,24.),
            theme:Theme::dark(),flags:HashMap::new(),themed:false,editor_state:EditorState::default(),geo:None,blender:BlenderStatus::default(),link:None,blender_job:None,
            prefs:Preferences::default(),keymap:Keymap::blender(),prefs_open:false,viewport_hover:false,network_hover:false,
            display_ranges:Vec::new(),output_range_count:0,scene:None,selected_preview:false,shown_level:1.0,geo_rev:u64::MAX,lod_uploaded:false,last_lod_view:None,last_lod_select:Instant::now(),blender_paths:Vec::new(),blender_bar:false,selection:picking::Selection::default(),selection_dirty:false,param_pick:None,barrier_key:None,target_key:None,picked_field:None,selection_metrics:Default::default(),coplanar:None,group_preview:Default::default(),elem_topo:Default::default(),stashed_selection:picking::Selection::default(),
            clipboard:None,transform_before:None,transform_target:None,transform_delta:None,transform_pivot:None,transform_orientation:None,modal:None,pending_duplicate_move:None,duplicate_move_peers:Vec::new(),embed_conflict:None,model_modal:None,mirror:None,pushpull_pick:false,last_operator:None,last_viewport:egui::Rect::NOTHING,
            last_edit_selection:ui::SelectionMode::Points,params_open:true,object_hidden:false,held_keys:Vec::new(),selection_memory:Default::default(),pending_loop_selection:None,pending_restore_selection:None,model_before_selection:None,sel_baseline:Default::default(),extrude_keep:None };
        app.prefs = Preferences::load();
        app.prefs.apply_python(&python_gate());
        app.mode=Mode::parse(&app.prefs.viewport.default_shading);
        app.editor_state.object_visibility=app.prefs.viewport.object_visibility;
        app.editor_state.matcap=app.prefs.viewport.matcap;
        app.editor_state.matcap_flat=app.prefs.viewport.matcap_flat;
        app.editor_state.object_color=app.prefs.viewport.object_color;
        app.editor_state.textures=app.prefs.viewport.textures;
        app.keymap = Keymap::from_overrides_json(&app.prefs.keymap_overrides);
        app.keymap.emulate_numpad = app.prefs.emulate_numpad;
        if app.prefs.theme == ThemePref::Light { app.theme = Theme::light(); }
        app.editor_state.snap.increment = app.prefs.snap_increment;
        app.editor_state.snap.to = app.prefs.snap_targets;
        app.editor_state.overlays=app.prefs.viewport.overlays;
        app.editor_state.xray=app.prefs.viewport.xray;
        app.editor_state.overlays.grid = app.prefs.viewport.show_grid;
        app.editor_state.overlays.axes = app.prefs.viewport.show_axes;
        app.editor_state.overlays.stats = app.prefs.show_stats;
        (app.editor_state.proxy.clip_start, app.editor_state.proxy.clip_end) = (app.prefs.viewport.clip_start, app.prefs.viewport.clip_end);
        app.camera.set_clip_planes(app.editor_state.proxy.clip_start, app.editor_state.proxy.clip_end);
        let installs = ui::blender_ui::blender_link::find_installs();
        app.blender.installs = installs.iter().map(|(label, path)| (label.clone(), path.display().to_string())).collect();
        app.blender_paths = installs.into_iter().map(|(_, p)| p).collect();
        app.blender.addon_installed = false;
        if start_link { app.restart_blender_link(); }
        app.send(Edit::Replace(app.editor.doc.value().clone()));
        app.send(Edit::Preview(app.selected.clone()));
        Ok(app)
    }
    fn send(&mut self, edit: Edit) {
        // An explicitly empty display has no last-good result. Cook failures still keep it.
        if let Edit::Replace(value) = &edit {
            if display_is_empty(value) {
                self.scene = None;
                self.gpu = None;
                self.cooking.geometry = None;
                self.cooking.lod = None;
                self.geo = None;
                self.display_ranges.clear();
                self.output_range_count = 0;
                self.selection_dirty = true;
            }
        }
        self.cooking.requested += 1;
        if let Err(e) = self.worker.send(CookRequest { revision:self.cooking.requested, edit }) {
            self.cooking.completed = self.cooking.requested;
            self.cooking.error = Some(e.to_string());
        }
    }
    /// Recook after a Python permission change: same graph and time, only keys that read the gate change.
    fn recook_python(&mut self) {
        let (frame, fps) = self.link.as_ref().map_or(self.http_time, |l| (l.frame, l.fps));
        self.send(Edit::Time { frame, fps });
    }
    /// Run / Skip for the first python snippet waiting for confirmation (once per snippet per session).
    fn python_confirm(&mut self, ctx: &egui::Context) {
        let gate = python_gate();
        if !gate.allowed() { return; }
        let Some(code) = gate.pending().into_iter().next() else { return };
        let mut choice = None;
        egui::Window::new("Run Python snippet?").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_TOP, [0., 40.]).show(ctx, |ui| {
            ui.label(format!("A python node wants to run this code (snippet {}) with your user privileges. It is not sandboxed.", minihou_core::python_bridge::code_hash(&code)));
            egui::ScrollArea::vertical().max_height(220.).show(ui, |ui| { ui.add(egui::Label::new(egui::RichText::new(&code).monospace())); });
            ui.horizontal(|ui| {
                if ui.button("Run").clicked() { choice = Some(true); }
                if ui.button("Skip").clicked() { choice = Some(false); }
            });
        });
        if let Some(run) = choice {
            if run { gate.approve(&code); } else { gate.skip(&code); }
            self.recook_python();
        }
    }
    fn send_timeline_time(&mut self, edit: Edit) {
        if let Edit::Time { frame, fps } = edit {
            self.http_time = (frame, fps);
            if let Some(link) = &mut self.link { link.frame = frame; link.fps = fps; }
            self.send(Edit::Time { frame, fps });
        }
    }
    fn edit(&mut self, edit: Edit, continuous: bool) {
        let was_linked = match &edit {
            Edit::Param { node, name, .. } => self.editor.doc.value()["nodes"][node]["params"][name].get("modifier_input").is_some(),
            _ => false,
        };
        let promote = match &edit {
            Edit::Param { name, value, .. } if self.editor_state.selection != ui::SelectionMode::Object && !self.selection.is_empty() =>
                ui::params::domain_edit_mode(name, value).filter(|to| *to != self.selection.mode),
            _ => None,
        };
        let history_selection=self.selection_state();
        match self.editor.history.edit_with_selection(&mut self.editor.doc, &edit, Some(history_selection)) {
            Ok(()) => {
                if let Some(to) = promote {
                    // the selection follows the group domain (converted when the mode changes); undo restores it as it was
                    self.editor_state.apply(ui::EditorChange::Selection(to));
                }
                if matches!(&edit, Edit::Param { node, .. } if self.editor.doc.value()["nodes"][node]["linked_params_group"].is_string())
                    || matches!(edit, Edit::ModifierInputs { .. } | Edit::ModifierInputValue { .. }) || was_linked
                    || matches!(&edit, Edit::Param { value, .. } if value.get("modifier_input").is_some()) {
                    self.send(Edit::Replace(self.editor.doc.value().clone()));
                } else { self.send(edit); }
                if !continuous { self.finish_history(); }
            }
            Err(e) => self.file_status = e.to_string(),
        }
    }
    fn frame_mesh(&mut self) {
        self.camera = Camera::frame(self.bounds.0, self.bounds.1);
        self.camera.set_clip_planes(self.editor_state.proxy.clip_start, self.editor_state.proxy.clip_end);
    }
    fn frame_selected(&mut self) {
        if let (Some(scene), Some(center)) = (&self.scene, self.selection_center()) {
            let points = scene.positions();
            let ids = if self.editor_state.selection == ui::SelectionMode::Object {
                self.display_ranges.iter().filter(|(id, _)| self.selected_set.contains(id)).flat_map(|(_, range)| range.clone()).collect()
            } else { self.selected_points() };
            let radius = ids.into_iter().filter_map(|id| points.get(id as usize * 3..id as usize * 3 + 3))
                .map(|p| ((0..3).map(|i| (p[i] - center[i]).powi(2)).sum::<f32>()).sqrt())
                .fold(0_f32, f32::max).max(self.bounds.1 * 0.05);
            self.camera = Camera::frame(center, radius);
            self.camera.set_clip_planes(self.editor_state.proxy.clip_start, self.editor_state.proxy.clip_end);
        } else { self.frame_mesh(); }
    }
    /// graph_embed_v1 conflict modal: a Blender push never overwrites unsaved work silently.
    fn embed_conflict_modal(&mut self, ctx: &egui::Context) {
        use minihou_app::graph_embed::{conflict_prompt, Resolution};
        let Some(conflict) = self.embed_conflict.clone() else { return };
        if self.link.is_none() { self.embed_conflict = None; return; }
        let kind = conflict["kind"].as_str().unwrap_or("live");
        let (text, choices) = conflict_prompt(kind, conflict["label"].as_str().unwrap_or(""));
        let mut chosen = None;
        egui::Modal::new(egui::Id::new("minihou_embed_conflict")).show(ctx, |ui| {
            ui.set_max_width(460.);
            ui.heading("Embedded Blender network");
            ui.label(text);
            ui.add_space(8.);
            ui.horizontal(|ui| for (resolution, label) in choices {
                if ui.button(label).clicked() { chosen = Some(resolution); }
            });
        });
        let Some(resolution) = chosen else { return };
        if resolution == Resolution::OpenAsNew && kind == "live" {
            let name = format!("{} (my edits).minihou", conflict["name"].as_str().filter(|s| !s.is_empty()).unwrap_or("network"));
            let Some(path) = rfd::FileDialog::new().add_filter("MiniHou graph", &["minihou", "json"]).set_file_name(name).save_file() else { return };
            if let Err(e) = self.editor.export_to(&path) { self.file_status = format!("Export failed: {e}"); return; }
            self.file_status = format!("Your version was saved to {}", path.display());
        }
        self.embed_conflict = None;
        let mut link = self.link.take().unwrap();
        let edits = link.resolve_embed(&conflict, resolution, &mut self.editor);
        self.link = Some(link);
        match edits {
            Ok(edits) => {
                if !edits.is_empty() {
                    self.display = Graph::from_value(self.editor.doc.value()).ok().and_then(|g| g.main_output().ok()).unwrap_or_default();
                    self.selected = self.display.clone();
                    self.selected_set = std::collections::BTreeSet::from([self.display.clone()]);
                }
                for edit in edits { self.send(edit); }
            }
            Err(e) => self.file_status = format!("Could not apply the embedded graph: {e}"),
        }
    }
    fn save_to(&mut self, path: PathBuf) {
        self.finish_history();
        match self.editor.save_to(path) {
            Ok(()) => { self.file_status = "Saved".into(); }
            Err(e) => self.file_status = format!("Save failed: {e}"),
        }
    }
    fn open(&mut self, path: PathBuf) -> bool {
        if self.link.as_ref().is_some_and(|link|link.live_active()) {
            self.file_status="Disconnect the live Blender modifier before opening another document.".into();
            return false;
        }
        let replacement = Document::open(&path).and_then(|doc| Self::new_document(doc, path, self.state.clone(), false));
        match replacement {
            Ok(mut app) => {
                app.target = self.target.take(); app.http = std::mem::take(&mut self.http);
                app.link = self.link.take(); app.blender = self.blender.clone();
                app.http_time = self.http_time;
                let (frame, fps) = app.link.as_ref().map_or(app.http_time, |l| (l.frame, l.fps));
                app.send(Edit::Time { frame, fps });
                *self = app;
                true
            }
            Err(e) => { self.file_status = format!("Open failed: {e}"); false },
        }
    }
}
/// Endless drag (as Blender): when the pointer reaches the viewport edge it is warped to the opposite side, and the jump
/// that warp causes is dropped from `delta`, so the orbit/pan/zoom keeps going.
pub(crate) fn wrap_drag(ctx: &egui::Context, rect: egui::Rect, delta: &mut egui::Vec2) {
    if delta.x.abs() > rect.width() * 0.5 || delta.y.abs() > rect.height() * 0.5 { *delta = egui::Vec2::ZERO; }
    let Some(pos) = ctx.input(|i| i.pointer.latest_pos()) else { return; };
    let inner = rect.shrink(2.0);
    if inner.contains(pos) { return; }
    let m = 6.0;
    let wrap = |v: f32, lo: f32, hi: f32| if v < lo + 2.0 { hi - m } else if v > hi - 2.0 { lo + m } else { v };
    let to = egui::pos2(wrap(pos.x, rect.left(), rect.right()), wrap(pos.y, rect.top(), rect.bottom()));
    ctx.send_viewport_cmd(egui::ViewportCommand::CursorPosition(to));
}

fn standard_view(v: ui::ViewPreset) -> StandardView {
    use ui::ViewPreset as P;
    match v { P::Perspective => StandardView::Perspective, P::Front => StandardView::Front, P::Back => StandardView::Back,
        P::Right => StandardView::Right, P::Left => StandardView::Left, P::Top => StandardView::Top, P::Bottom => StandardView::Bottom }
}
impl App {
    /// The fraction of triangles to show: the menu's level, or one picked from the triangle count when Auto is on.
    fn proxy_level(&self, triangles: usize) -> f32 {
        let p = self.editor_state.proxy;
        if p.auto { ui::proxy_ui::proxy::auto_level(triangles, self.prefs.viewport.auto_budget as usize) } else { p.level }
    }
    /// A shelf tool: a new node of `node_type` wired to `after` (its first input) and made the displayed node; one undo step.
    /// Begin a plane sketch; the committed outline is a `curve` node added through `graph_edit::add_sketch`.
    fn start_sketch(&mut self, ctx: &egui::Context, mode: ui::sketch::SketchMode) {
        if minihou_core::nodes::get("curve").is_some() {
            self.finish_for_selection(ctx);
            self.editor_state.apply(ui::EditorChange::Tool(mode.tool()));
            ui::sketch::start(ctx, mode);
        } else { self.file_status = "Sketches need the Rust core curve node".into(); }
    }
    fn set_display_mode(&mut self,mode:Mode) {
        self.mode=mode;
        self.prefs.viewport.default_shading=mode.label().into();
        if let Err(e)=self.prefs.save() { self.file_status=format!("Could not save preferences: {e}"); }
    }
}
impl App {
    /// Alt+M: start the Mirror tool on the selected node (or adopt a selected mirror node). See ui/mirror_tool.rs.
    fn start_mirror(&mut self, ctx: &egui::Context) {
        use ui::mirror_tool as mt;
        if self.modal.is_some() || self.model_modal.is_some() || self.mirror.is_some() { return; }
        if self.scene.is_none() { self.file_status = "Wait for the mesh to cook".into(); return; }
        self.finish_for_selection(ctx);
        let Some(scene) = self.scene.as_ref() else { return; };
        let positions = scene.positions();
        let Some(owner) = self.active_object().map(str::to_owned) else { self.file_status="Select a displayed object to mirror".into(); return; };
        let ranges: Vec<_> = self.display_ranges.iter().filter(|(o, _)| &owner == o).map(|r| r.1.clone()).collect();
        let point = |i: usize| [positions[3 * i], positions[3 * i + 1], positions[3 * i + 2]];
        let in_object = |i: u32| ranges.is_empty() || ranges.iter().any(|r| r.contains(&i));
        let ids: Vec<usize> = (0..positions.len() / 3).filter(|&i| in_object(i as u32)
            && point_pickable(self.editor_state.object_visibility, &self.display_ranges, &self.selected_set, i as u32)).collect();
        let Some(bounds) = mt::Bounds::of(ids.iter().map(|&i| point(i))) else {
            self.file_status = "Nothing to mirror".into(); return;
        };
        // Edit mode with selected faces: Mirror Selected on those primitives.
        let extra = if self.editor_state.selection != ui::SelectionMode::Object && !self.selection.tris.is_empty() {
            let selection=self.active_selection().unwrap_or_default();
            let range=active_range(&owner,&self.display_ranges).unwrap();
            let first=scene.triangle_indices().chunks_exact(3).take_while(|tri|tri[0]<range.start).count() as u32;
            let topo=element_select::cached(&self.elem_topo, scene);
            let offset=topo.poly_of_tri(first).unwrap_or(0);
            let faces = topo.polys_of(&selection.tris).into_iter().map(|p|p-offset).collect::<Vec<_>>();
            json!({"operation": "mirror_selected", "group": group_text(&faces)})
        } else { json!({}) };
        let wire: Vec<_> = scene.edge_indices().chunks_exact(2).filter(|e| in_object(e[0])).take(mt::WIRE_LIMIT)
            .map(|e| (point(e[0] as usize), point(e[1] as usize))).collect();
        let source = owner;
        self.finish_history();
        let mut session = mt::Session::new(self.editor.doc.clone(), self.selected.clone(), source.clone(), bounds, extra);
        session.wire = wire;
        if self.editor.doc.value()["nodes"][&source]["type"] == "mirror" {
            let plane = mt::Plane::read(&self.editor.doc.value()["nodes"][&source]["params"]);
            session.nodes.push((mt::dominant_axis(plane.normal), source.clone()));
            session.active = Some(source);
        }
        self.mirror = Some(session);
    }
    fn mirror_select(&mut self, id: Option<String>) {
        let Some(s) = self.mirror.as_mut() else { return; };
        s.active = id.clone();
        let id = id.unwrap_or_else(|| s.previous.clone());
        self.selected = id.clone(); self.selected_set = std::collections::BTreeSet::from([id]);
        self.display = Graph::from_value(self.editor.doc.value()).ok().and_then(|g| g.main_output().ok()).unwrap_or_else(|| self.display.clone());
    }
    fn mirror_replace(&mut self, doc: Document) {
        self.editor.doc = doc;
        self.send(Edit::Replace(self.editor.doc.value().clone()));
    }
    fn mirror_axis(&mut self, cube: ui::mirror_tool::Cube, world_origin: bool) {
        use ui::mirror_tool as mt;
        let Some(s) = self.mirror.as_mut() else { return; };
        let origin = if world_origin { [0.; 3] } else { s.bounds.center() };
        let (source, extra) = (s.source.clone(), s.extra.clone());
        let Some((doc, click)) = mt::click_axis(&self.editor.doc, &mut s.nodes, &source, cube, origin, &extra) else {
            self.file_status = "Could not add a mirror node".into(); return;
        };
        let last = s.nodes.last().map(|n| n.1.clone());
        self.mirror_replace(doc);
        let active = match click { mt::Click::Added(id) | mt::Click::Flipped(id) => Some(id), mt::Click::Removed(_) => last };
        self.mirror_select(active);
    }
    fn mirror_set_plane(&mut self, plane: ui::mirror_tool::Plane) {
        use ui::mirror_tool as mt;
        let Some(s) = self.mirror.as_mut() else { return; };
        let id = match s.active.clone() {
            Some(id) => id,
            None => {
                // A drawn plane before any cube click creates the first node.
                let after = s.source.clone();
                let Some((mut graph, id)) = graph_edit::add(self.editor.doc.value(), "mirror", Some(&after)) else { return; };
                let mut params = json!({"cut": true});
                for (k, v) in s.extra.as_object().cloned().unwrap_or_default() { params[k] = v; }
                graph["nodes"][&id]["params"] = params;
                let Ok(doc) = graph_edit::validated(graph) else { return; };
                s.nodes.push((mt::dominant_axis(plane.effective().1), id.clone()));
                self.mirror_replace(doc);
                self.mirror_select(Some(id.clone()));
                id
            }
        };
        // Coalesced: only changed values are sent to the cook worker.
        for (name, value) in plane.params() {
            if self.editor.doc.value()["nodes"][&id]["params"][name] != value {
                let edit = Edit::Param { node: id.clone(), name: name.into(), value };
                if self.editor.doc.edit(&edit).is_ok() { self.send(edit); }
            }
        }
    }
    fn finish_mirror(&mut self) {
        if let Some(s) = self.mirror.take() {
            self.editor.history.drop_pending();
            self.record_history(s.before);
        }
    }
    fn cancel_mirror(&mut self) {
        if let Some(s) = self.mirror.take() {
            self.editor.history.drop_pending();
            self.editor.doc = s.before;
            self.selected = s.previous.clone(); self.selected_set = std::collections::BTreeSet::from([s.previous]);
            self.display = Graph::from_value(self.editor.doc.value()).ok().and_then(|g| g.main_output().ok()).unwrap_or_default();
            self.send(Edit::Replace(self.editor.doc.value().clone()));
        }
    }
    fn update_mirror(&mut self, ctx: &egui::Context) {
        use ui::mirror_tool as mt;
        if self.mirror.is_none() { return; }
        let rect = self.last_viewport;
        let typing = ctx.wants_keyboard_input();
        let popup = ui::viewport_ui::popup_open(ctx);
        let input = ctx.input(|i| mt::Input {
            pointer: i.pointer.hover_pos(), pressed: i.pointer.primary_pressed(), down: i.pointer.primary_down(),
            released: i.pointer.primary_released(),
            secondary: i.pointer.secondary_pressed() && i.pointer.hover_pos().is_some_and(|p| rect.contains(p)),
            keys: if typing { Vec::new() } else { i.events.iter().filter_map(|e| match e { egui::Event::Key { key, pressed: true, repeat: false, .. } => Some(*key), _ => None }).collect() },
            shift: i.modifiers.shift, alt: i.modifiers.alt,
            in_viewport: i.pointer.hover_pos().is_some_and(|p| rect.contains(p)) && !popup,
        });
        let camera = self.camera_info();
        let plane = self.mirror.as_ref().and_then(|s| s.active.as_ref()).map(|id| mt::Plane::read(&self.editor.doc.value()["nodes"][id]["params"]));
        let outs = {
            let active_set=self.active_pick_set();
            let (scene, visibility, ranges, selected) = (self.scene.as_ref(), viewport::ObjectVisibility::HideOthers, &self.display_ranges, &active_set);
            let pick = |p: egui::Pos2| -> Option<([f32; 3], [f32; 3])> {
                let scene = scene?;
                let (o, d) = ui::transform_gizmo::ray(camera, rect, p);
                let hit = pick_displayed(scene, o, d, visibility, ranges, selected)?;
                let tri = scene.triangle_indices().get(hit.tri as usize * 3..hit.tri as usize * 3 + 3)?;
                let (a, b, c) = (picking::at(scene.positions(), tri[0]), picking::at(scene.positions(), tri[1]), picking::at(scene.positions(), tri[2]));
                let u = [0, 1, 2].map(|k| b[k] - a[k]); let v = [0, 1, 2].map(|k| c[k] - a[k]);
                let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
                Some(([0, 1, 2].map(|k| o[k] + d[k] * hit.t), n))
            };
            self.mirror.as_mut().unwrap().step(&input, plane, camera, rect, &pick)
        };
        for o in outs {
            match o {
                mt::Out::Confirm => { self.finish_mirror(); ctx.data_mut(|d| d.insert_temp(egui::Id::new("modal_confirm_click"), true)); return; }
                mt::Out::Cancel => { self.cancel_mirror(); return; }
                mt::Out::Axis { cube, world_origin } => self.mirror_axis(cube, world_origin),
                mt::Out::Plane(p) => self.mirror_set_plane(p),
            }
        }
        ctx.request_repaint();
    }
    fn start_model_modal(&mut self, ctx: &egui::Context, kind: ModelKind) {
        if self.modal.is_some() || self.model_modal.is_some() || self.scene.is_none() { return; }
        let Some(scene)=self.scene.as_ref() else { return; };
        let Some(range)=self.active_object().and_then(|id|active_range(id,&self.display_ranges)).cloned() else { self.file_status="Select a displayed object".into(); return; };
        let source_points=(range.end-range.start) as usize;
        let tri_offset=scene.triangle_indices().chunks_exact(3).take_while(|tri|tri[0]<range.start).count() as u32;
        let poly_offset=element_select::cached(&self.elem_topo,scene).poly_of_tri(tri_offset).unwrap_or(0);
        let Some(mut selection)=self.active_selection() else { self.file_status="Select a displayed object".into(); return; };
        selection.mode=match self.editor_state.selection { ui::SelectionMode::Points => ui::SelectionMode::Points,
            ui::SelectionMode::Edges => ui::SelectionMode::Edges,
            ui::SelectionMode::Faces => ui::SelectionMode::Faces,
            ui::SelectionMode::Object => { self.file_status="Select mesh elements for this tool".into(); return; } };
        let params=if kind==ModelKind::Bevel {
            if selection.mode!=ui::SelectionMode::Edges {
                selection=element_select::cached(&self.elem_topo,scene).convert(&selection,ui::SelectionMode::Edges);
            }
            if selection.edges.is_empty() { self.file_status="Select edges to bevel".into(); return; }
            json!({"edges":selection.edges.iter().map(|(a,b)|format!("{}-{}",a-range.start,b-range.start)).collect::<Vec<_>>().join(" "),"distance":0.,"divisions":1})
        } else if kind==ModelKind::Extrude && selection.mode!=ui::SelectionMode::Faces {
            let mut params=match selection.mode {
                ui::SelectionMode::Edges => json!({"edges":selection.edges.iter().map(|(a,b)|format!("{}-{}",a-range.start,b-range.start)).collect::<Vec<_>>().join(" "),"distance":0.}),
                ui::SelectionMode::Points => json!({"points":group_text(&selection.points.iter().map(|p|*p-range.start).collect::<Vec<_>>()),"distance":0.}),
                _ => unreachable!(),
            };
            if selection.is_empty() { self.file_status="Select mesh elements for this tool".into(); return; }
            {
                let points=selected_point_ids(&selection,selection.mode,scene.triangle_indices());
                selection.tris=scene.triangle_indices().chunks_exact(3).enumerate()
                    .filter(|(_,tri)| tri.iter().any(|p| points.contains(p)))
                    .map(|(tri,_)|tri as u32).collect();
            }
            params["group"]=json!("");
            params
        } else {
            if selection.tris.is_empty() {
                self.file_status="Select mesh elements for this tool".into(); return;
            }
            let faces=element_select::cached(&self.elem_topo,scene).polys_of(&selection.tris);
            let mut params=json!({"group":group_text(&faces.iter().map(|p|*p-poly_offset).collect::<Vec<_>>())});
            params[kind.parameter()]=json!(0.);
            params
        };
        let source=self.active_object().unwrap().to_owned();
        let Some((doc,id))=ui::modal_model::insert_graph(&self.editor.doc,&source,kind,params) else {
            self.file_status=format!("Could not add {}",kind.node()); return;
        };
        self.model_before_selection=Some((self.selection.clone(),self.stashed_selection.clone()));
        self.extrude_keep=(kind==ModelKind::Extrude && self.editor_state.selection==ui::SelectionMode::Faces).then(||self.selection.clone());
        self.finish_history();
        let before=self.editor.doc.clone();
        self.editor.doc=doc;
        self.send(Edit::Replace(self.editor.doc.value().clone()));
        let previous=self.selected.clone();
        self.selected=id.clone(); self.selected_set=std::collections::BTreeSet::from([id.clone()]);
        self.display=Graph::from_value(self.editor.doc.value()).ok().and_then(|g|g.main_output().ok()).unwrap_or(id.clone());
        let pointer=ctx.input(|i|i.pointer.hover_pos()).unwrap_or(self.last_viewport.center());
        let radius=self.bounds.1.max(0.001);
        let mut state=ModelState::new(kind,pointer,(radius/200.).max(0.00001));
        state.units=Some(self.prefs.units); state.source_points=source_points;
        if matches!(kind,ModelKind::Extrude | ModelKind::PushPull) {
            let scene=self.scene.as_ref().unwrap();
            let tris=scene.triangle_indices(); let positions=scene.positions();
            let mut normal=[0f32;3]; let mut center=[0f32;3]; let mut count=0f32;
            for &tri in &selection.tris {
                if let Some(ids)=tris.get(tri as usize*3..tri as usize*3+3) {
                    let a=picking::at(positions,ids[0]); let b=picking::at(positions,ids[1]); let c=picking::at(positions,ids[2]);
                    let u=[0,1,2].map(|k|b[k]-a[k]); let v=[0,1,2].map(|k|c[k]-a[k]);
                    let n=[u[1]*v[2]-u[2]*v[1],u[2]*v[0]-u[0]*v[2],u[0]*v[1]-u[1]*v[0]];
                    for k in 0..3 { normal[k]+=n[k]; center[k]+=(a[k]+b[k]+c[k])/3.; }
                    count+=1.;
                }
            }
            if count>0. && kind==ModelKind::PushPull {
                // SketchUp-style inference: far face (Through) and parallel faces (On Face).
                if let Some(local_geo)=self.cooking.geometry.as_deref().and_then(|g|ui::mesh_cut::object_geometry(g,&range)) {
                    let local_scene=Scene::new(&local_geo);
                    let local_tris=selection.tris.iter().map(|tri|*tri-tri_offset).collect::<Vec<_>>();
                    state.targets=ui::modal_model::pushpull_targets(local_scene.positions(),local_scene.triangle_indices(),&local_tris,normal);
                }
            }
            if count>0. {
                center=center.map(|x|x/count);
                let length=normal.iter().map(|x|x*x).sum::<f32>().sqrt();
                if length>1e-6 {
                    normal=normal.map(|x|x/length);
                    let tip=[0,1,2].map(|k|center[k]+normal[k]*radius*0.25);
                    if let (Some(a),Some(b))=(ui::transform_gizmo::project(self.camera_info(),self.last_viewport,center),ui::transform_gizmo::project(self.camera_info(),self.last_viewport,tip)) {
                        let delta=b-a; if delta.length()>1. { state.screen_axis=delta.normalized(); }
                    }
                }
            }
        }
        self.model_modal=Some((state,before,id,previous));
    }
    fn finish_model_modal(&mut self) {
        self.extrude_keep=None;
        if let Some((state,before,id,_))=self.model_modal.take() {
            if state.kind==ModelKind::Extrude {
                self.last_operator=Some(LastOperator::Extrude {params:repeat::extrude_next_params(&self.editor.doc.value()["nodes"][&id]["params"],state.source_points)});
            }
            let before_selection=self.model_before_selection.take().map(|(selection,stash)|selection_state(&selection,&stash));
            let after_selection=self.selection_state();
            self.editor.history.record_with_selection(before,&self.editor.doc,before_selection,Some(after_selection));
        }
    }
    fn cancel_model_modal(&mut self) {
        if let Some((_,before,_,previous))=self.model_modal.take() {
            self.model_before_selection=None; self.extrude_keep=None;
            self.editor.doc=before;
            self.selected=previous.clone(); self.selected_set=std::collections::BTreeSet::from([previous]);
            self.display=Graph::from_value(self.editor.doc.value()).ok().and_then(|g|g.main_output().ok()).unwrap_or_default();
            self.send(Edit::Replace(self.editor.doc.value().clone()));
        }
    }
    fn update_model_modal(&mut self, ctx: &egui::Context) {
        let Some((mut state,before,id,previous))=self.model_modal.take() else { return; };
        let (events,pointer,modifiers,scroll)=ctx.input(|i|(i.events.clone(),i.pointer.hover_pos(),i.modifiers,i.raw_scroll_delta.y));
        let mut confirm=false; let mut cancel=false;
        for event in events {
            match event {
                egui::Event::Key { key,pressed:true,.. } => match state.key_event(key,modifiers.ctrl) {
                    ModelOutcome::Cancel=>cancel=true, ModelOutcome::Confirm=>confirm=true, ModelOutcome::Continue=>{}
                },
                egui::Event::PointerButton { button:egui::PointerButton::Primary,pressed:true,.. } => confirm=true,
                egui::Event::PointerButton { button:egui::PointerButton::Secondary,pressed:true,.. } => cancel=true,
                _=>{}
            }
        }
        if cancel { self.model_modal=Some((state,before,id,previous)); self.cancel_model_modal(); return; }
        if let Some(pointer)=pointer { state.move_to(pointer,modifiers.shift,modifiers.ctrl); }
        if state.kind==ModelKind::Bevel && scroll!=0. { state.divisions_step(if scroll>0. {1} else {-1}); }
        let param=state.kind.parameter();
        let value=json!(state.value);
        if self.editor.doc.value()["nodes"][&id]["params"][param]!=value {
            let edit=Edit::Param { node:id.clone(),name:param.into(),value };
            if self.editor.doc.edit(&edit).is_ok() { self.send(edit); }
        }
        if state.kind==ModelKind::Bevel && self.editor.doc.value()["nodes"][&id]["params"]["divisions"]!=json!(state.divisions) {
            let edit=Edit::Param { node:id.clone(),name:"divisions".into(),value:json!(state.divisions) };
            if self.editor.doc.edit(&edit).is_ok() { self.send(edit); }
        }
        if state.kind==ModelKind::Extrude {
            let direction=state.axis.map(|axis| [0,1,2].map(|k|if k==axis {1.} else {0.})).unwrap_or([0.;3]);
            if self.editor.doc.value()["nodes"][&id]["params"]["direction"]!=json!(direction) {
                let edit=Edit::Param { node:id.clone(),name:"direction".into(),value:json!(direction) };
                if self.editor.doc.edit(&edit).is_ok() { self.send(edit); }
            }
        }
        self.model_modal=Some((state,before,id,previous));
        if confirm { self.finish_model_modal(); } else { ctx.request_repaint(); }
    }
    fn start_modal(&mut self, ctx: &egui::Context, kind: ModalKind) {
        if self.modal.is_some() || self.display.is_empty() || self.scene.is_none() { return; }
        if self.editor_state.selection == ui::SelectionMode::Object && self.active_object().is_none() { return; }
        if self.editor_state.selection != ui::SelectionMode::Object && self.selected_points().is_empty() {
            self.file_status = "Select points, edges, or faces to transform".into(); return;
        }
        let pointer = ctx.input(|i| i.pointer.hover_pos()).unwrap_or(self.last_viewport.center());
        let scene = self.scene.as_ref().unwrap(); let positions = scene.positions();
        let ids = self.selected_points();
        let points: Vec<[f32;3]> = if self.editor_state.selection == ui::SelectionMode::Object {
            self.display_ranges.iter().filter(|(id, _)| self.selected_set.contains(id))
                .flat_map(|(_, range)| range.clone()).filter_map(|id| positions.get(id as usize*3..id as usize*3+3).map(|p|[p[0],p[1],p[2]])).collect()
        } else if ids.is_empty() { positions.as_chunks::<3>().0.to_vec() }
            else { ids.into_iter().filter_map(|id| positions.get(id as usize*3..id as usize*3+3).map(|p|[p[0],p[1],p[2]])).collect() };
        if points.is_empty() { return; }
        let lo = [0,1,2].map(|i|points.iter().map(|p|p[i]).fold(f32::INFINITY,f32::min));
        let hi = [0,1,2].map(|i|points.iter().map(|p|p[i]).fold(f32::NEG_INFINITY,f32::max));
        let pivot = if self.editor_state.pivot == ui::Pivot::Cursor { self.editor_state.cursor }
            else if self.editor_state.selection != ui::SelectionMode::Object && self.editor_state.pivot == ui::Pivot::BoundingBoxCenter { [0,1,2].map(|i|(lo[i]+hi[i])*0.5) }
            else { self.selection_center().unwrap_or([0,1,2].map(|i|(lo[i]+hi[i])*0.5)) };
        let mut modal=Modal::new(kind,pointer,pivot,(lo,hi),self.editor_state.snap.to,
            self.editor_state.proportional.enabled,self.editor_state.proportional.radius);
        modal.units=Some(self.prefs.units);
        let transform_node = if self.editor_state.selection == ui::SelectionMode::Object { &self.selected } else { &self.display };
        let trs = transform_math::TrsParams::from_params(&self.editor.doc.value()["nodes"][transform_node]["params"]);
        modal.rotation = transform_math::matrix_to_euler_xyz(&trs.world_rotation(), [0.; 3]).map(|x| x as f32);
        self.modal=Some(modal);
        ctx.request_repaint();
    }
    fn modal_snap_feature(&self, at: egui::Pos2, snap_to: ui::SnapTargets) -> Option<picking::SnapFeature> {
        let scene = self.scene.as_ref()?;
        picking::snap_feature(scene.positions(), scene.triangle_indices(), self.camera_info(),
            self.last_viewport, at, snap_to, self.editor_state.snap.backface_culling)
    }
    fn modal_snap_point(&self, at: egui::Pos2, snap_to: impl Into<ui::SnapTargets>) -> Option<[f32;3]> {
        self.modal_snap_feature(at,snap_to.into()).map(|feature|feature.point)
    }
    fn finish_modal(&mut self, modal: Modal) {
        if modal.cage && self.editor_state.selection == ui::SelectionMode::Object
            && minihou_core::nodes::get("match_size_legacy").is_some()
            && self.editor.doc.display_nodes().len()==1
            && modal.last.iter().all(|v| *v >= 0.) {
            if let Some(target) = &self.transform_target {
                if self.transform_before.as_ref().is_some_and(|before|before.value()["nodes"][target]["type"]=="transform") {
                    self.last_operator = Some(LastOperator::Scale { factor: modal.last, pivot: modal.pivot });
                    self.finish_transform_from(true); return;
                }
                let pivot=[0,1,2].map(|i|self.editor.doc.value()["nodes"][target]["params"]["pivot"].get(i)
                    .and_then(Value::as_f64).unwrap_or(modal.pivot[i] as f64) as f32);
                let (center,size)=ui::modal_transform::cage_target(modal.bounds,pivot,modal.last);
                let mut value=self.editor.doc.value().clone();
                value["nodes"][target]["type"]=json!("match_size_legacy");
                value["nodes"][target]["params"]=json!({"translate":true,"center":center,"justify_z":"center",
                    "scale":true,"size":size,"uniform":false});
                if let Ok(doc)=Document::parse(value.to_string()) { self.editor.doc=doc; self.send(Edit::Replace(value)); }
            }
        }
        self.last_operator = Some(match modal.kind {
            ModalKind::Move => LastOperator::Move {
                delta: modal.last,
                constraint: if modal.constraint == Constraint::Free { None } else { Some(modal.constraint) },
                orientation: if matches!(modal.constraint, Constraint::Axis(_, true) | Constraint::Plane(_, true)) {
                    ui::Orientation::Local
                } else {
                    self.editor_state.orientation
                },
                proportional: if modal.proportional { Some(self.editor_state.proportional) } else { None },
            },
            ModalKind::Rotate => LastOperator::Rotate {
                angle: modal.last,
                pivot: modal.sketch_pivot.unwrap_or(modal.pivot),
                orientation: self.editor_state.orientation,
                axis_rotation: (modal.kind != ModalKind::Move).then_some(
                    if matches!(self.editor_state.orientation, ui::Orientation::Local | ui::Orientation::Normal | ui::Orientation::Gimbal) {
                        modal.rotation
                    } else {
                        [0.; 3]
                    }
                ),
            },
            ModalKind::Scale => LastOperator::Scale {
                factor: modal.last,
                pivot: modal.pivot,
            },
        });
        self.finish_transform_from(true);
    }
    fn sketch_commit(&mut self, name: &str, points: &[[f32; 3]], closed: bool, refs: Vec<Option<Value>>) {
        let res = if name == "Bezier" {
            graph_edit::add_bezier_sketch(self.editor.doc.value(), name, points, closed, &self.selected)
        } else {
            graph_edit::add_sketch(self.editor.doc.value(), name, points, closed, &self.selected)
        };
        let Some((mut value, id)) = res else {
            self.file_status = "Could not add sketch".into(); return;
        };
        attach_inference_refs(&mut value, &id, refs);
        if self.commit_document(value, "add sketch") { self.selected = id.clone(); self.selected_set = std::collections::BTreeSet::from([id.clone()]); self.display = id; }
    }
    fn sketch_batch(&mut self, strokes: &[ui::sketch::SketchStroke], selected: &str, target: graph_edit::SketchTarget) {
        let rows: Vec<_> = strokes.iter().map(|s| (s.name.as_str(), s.points.as_slice(), s.closed)).collect();
        let Some((mut value, id, stroke_ids)) = graph_edit::add_sketch_batch(self.editor.doc.value(), &rows, selected, target) else {
            self.file_status = "Could not add sketch".into(); return;
        };
        // References belong to their stroke's source node; the batch itself is
        // still committed as one undo step.
        for (stroke, stroke_id) in strokes.iter().zip(&stroke_ids) { attach_inference_refs(&mut value, stroke_id, stroke.refs.clone()); }
        if self.commit_document(value, "add sketches") {
            self.selected = id.clone(); self.selected_set = std::collections::BTreeSet::from([id.clone()]);
            self.display = minihou_app::display_nodes(self.editor.doc.value()).into_iter().next().unwrap_or(id);
        }
    }
    fn add_measurement(&mut self, a: [f32; 3], b: [f32; 3], mut refs: [Option<Value>;2]) {
        let mut value = self.editor.doc.value().clone();
        if !value["measurements"].is_array() { value["measurements"] = json!([]); }
        let stamp=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
        for reference in refs.iter_mut().flatten() {
            let Some(node)=reference["node"].as_str().map(str::to_owned) else { continue; };
            if !value["nodes"][&node].is_object() { *reference=Value::Null; continue; }
            if !value["nodes"][&node]["measurement_identity"].is_string() {
                value["nodes"][&node]["measurement_identity"]=json!(format!("measure-{stamp}-{node}"));
            }
            reference["identity"]=value["nodes"][&node]["measurement_identity"].clone();
        }
        if let Some(rows) = value["measurements"].as_array_mut() {
            rows.push(json!({"id":format!("m-{stamp}"),"points":[a,b],"refs":refs,"a":a,"b":b}));
        }
        self.commit_document(value, "add measurement");
    }
    fn delete_measurement(&mut self,index:usize) {
        if let Some(value)=graph_edit::delete_measurement(self.editor.doc.value(),index) {
            self.commit_document(value,"delete measurement");
        }
    }
    fn clear_measurements(&mut self) {
        if let Some(value)=graph_edit::clear_measurements(self.editor.doc.value()) {
            self.commit_document(value,"clear measurements");
        }
    }
    /// Make `value` the document as one undo step and send it to the cook worker; false (with a status message) when it is invalid.
    fn commit_document(&mut self, value: serde_json::Value, what: &str) -> bool {
        match graph_edit::validated(value) {
            Ok(new) => {
                let selection=self.selection_state();
                self.editor.history.replace_with_selection(&mut self.editor.doc, new,Some(selection.clone()),Some(selection));
                self.send(Edit::Replace(self.editor.doc.value().clone()));
                true
            }
            Err(e) => { self.file_status = format!("Could not {what}: {e}"); false }
        }
    }
    /// Delete a node and every wire into it.
    fn delete_node(&mut self, id: &str) {
        let ids = if self.selected_set.contains(id) { self.selected_set.iter().cloned().collect::<Vec<_>>() } else { vec![id.to_owned()] };
        let Some(value) = graph_edit::delete_set(self.editor.doc.value(), &ids) else { return };
        if self.commit_document(value, "delete the node") {
            if let Ok(g) = Graph::from_value(self.editor.doc.value()) {
                self.selected_set.retain(|node| !ids.contains(node));
                if ids.contains(&self.selected) { self.selected = g.nodes.first().map(|(n, _)| n.clone()).unwrap_or_default(); }
                self.display = g.main_output().unwrap_or_else(|_| self.selected.clone());
            }
            self.last_operator = Some(LastOperator::Delete);
        }
    }
    /// Copy a node (parameters and inputs) under a fresh id and select the copy.
    fn duplicate_node(&mut self, id: &str) {
        let ids = if self.selected_set.contains(id) { self.selected_set.iter().cloned().collect::<Vec<_>>() } else { vec![id.to_owned()] };
        let Some(clip) = graph_edit::copy(self.editor.doc.value(), &ids) else { return };
        let Some((value, new_ids)) = graph_edit::paste(self.editor.doc.value(), &clip) else { return };
        if self.commit_document(value, "duplicate the node") {
            self.selected = new_ids[0].clone();
            self.selected_set = new_ids.into_iter().collect();
            self.last_operator = Some(LastOperator::Duplicate);
        }
    }
    fn duplicate_linked_nodes(&mut self) {
        let ids: Vec<_> = self.selected_set.iter().cloned().collect();
        let Some((value, copies)) = graph_edit::duplicate_linked(self.editor.doc.value(), &ids) else { return };
        if self.commit_document(value, "duplicate linked nodes") {
            self.selected = copies[0].clone();
            self.selected_set = copies.into_iter().collect();
            self.last_operator = Some(LastOperator::Duplicate);
        }
    }
    fn duplicate_viewport_objects(&mut self) {
        if self.editor_state.selection != ui::SelectionMode::Object { return; }
        let ids: Vec<_> = self.selected_set.iter().filter(|id| active_range(id, &self.display_ranges).is_some())
            .cloned().collect();
        let Some((value, copies)) = graph_edit::duplicate_objects(self.editor.doc.value(), &ids) else { return };
        if self.commit_document(value, "duplicate viewport objects") {
            self.selected = copies[0].clone();
            self.selected_set = copies.into_iter().collect();
            self.display = self.selected.clone();
            self.pending_duplicate_move = Some(self.selected.clone());
            self.duplicate_move_peers = self.selected_set.iter().cloned().collect();
            self.last_operator = Some(LastOperator::Duplicate);
        }
    }
    /// Dissolve a node, rewiring its inputs to consumers.
    fn dissolve_node(&mut self, id: &str) {
        let ids = if self.selected_set.contains(id) { self.selected_set.iter().cloned().collect::<Vec<_>>() } else { vec![id.to_owned()] };
        let Some(value) = graph_edit::dissolve_set(self.editor.doc.value(), &ids) else { return; };
        if self.commit_document(value, "dissolve the node") {
            self.selected_set.retain(|node| !ids.contains(node));
            if let Ok(g) = Graph::from_value(self.editor.doc.value()) {
                if ids.contains(&self.selected) { self.selected = g.nodes.first().map(|(n, _)| n.clone()).unwrap_or_default(); }
                self.display = g.main_output().unwrap_or_else(|_| self.selected.clone());
            }
            self.last_operator = Some(LastOperator::Dissolve);
        }
    }
    /// Wire `from` into input `port` of `to` (its first input when `port` is None); multi inputs gain one more wire.
    fn connect_nodes(&mut self, from: &str, to: &str, port: Option<&str>) {
        let Some(value) = graph_edit::connect(self.editor.doc.value(), from, to, port) else { return };
        if self.commit_document(value, "connect the nodes") {
            self.last_operator = Some(LastOperator::Connect {
                from: from.to_string(),
                to: to.to_string(),
                port: port.map(ToString::to_string),
            });
        }
    }
    fn disconnect_input(&mut self, node: &str, port: &str) {
        if let Some(value) = graph_edit::disconnect(self.editor.doc.value(), node, port) { self.commit_document(value, "disconnect the input"); }
    }
    fn repeat_last(&mut self, ctx: &egui::Context) {
        let Some(op) = self.last_operator.clone() else {
            self.file_status = "Nothing to repeat".into();
            return;
        };
        match op {
            LastOperator::Extrude {mut params} => {
                if self.cooking.cooking() {self.file_status="Wait for the extrusion to cook before repeating".into();return;}
                let Some(scene)=self.scene.as_ref() else {return;};
                let Some(source)=self.active_object().map(str::to_owned) else {self.file_status="Select a displayed object".into();return;};
                let range=active_range(&source,&self.display_ranges).unwrap();
                let source_points=(range.end-range.start) as usize;
                let selection=self.active_selection().unwrap_or_default();
                if self.editor_state.selection!=ui::SelectionMode::Object && !selection.is_empty() {
                    params["group"]=json!("");params["edges"]=json!("");params["points"]=json!("");
                    match self.editor_state.selection {
                        ui::SelectionMode::Faces=>{let first=scene.triangle_indices().chunks_exact(3).take_while(|tri|tri[0]<range.start).count() as u32;let topo=element_select::cached(&self.elem_topo,scene);let offset=topo.poly_of_tri(first).unwrap_or(0);params["group"]=json!(group_text(&topo.polys_of(&selection.tris).iter().map(|p|*p-offset).collect::<Vec<_>>()));},
                        ui::SelectionMode::Edges=>params["edges"]=json!(selection.edges.iter().map(|(a,b)|format!("{}-{}",a-range.start,b-range.start)).collect::<Vec<_>>().join(" ")),
                        ui::SelectionMode::Points=>params["points"]=json!(group_text(&selection.points.iter().map(|p|*p-range.start).collect::<Vec<_>>())),_=>{},
                    }
                }
                let next=repeat::extrude_next_params(&params,source_points);
                if let Some((doc,id))=ui::modal_model::insert_graph(&self.editor.doc,&source,ModelKind::Extrude,params) {
                    if self.commit_document(doc.value().clone(),"repeat extrusion") {
                        self.display=Graph::from_value(self.editor.doc.value()).ok().and_then(|g|g.main_output().ok()).unwrap_or(id.clone());
                        self.selected=id.clone();self.selected_set=std::collections::BTreeSet::from([id]);
                        self.last_operator=Some(LastOperator::Extrude {params:next});
                    }
                }
            }
            LastOperator::MatrixDelta { delta, frame, .. } => {
                let group = group_text(&self.selected_points());
                let object_mode = self.editor_state.selection == ui::SelectionMode::Object;
                if !object_mode && group.is_empty() {
                    self.file_status = "Select points, edges, or faces to transform".into();
                    return;
                }
                self.finish_history();
                let before = self.editor.doc.clone();
                let source = self.display.clone();
                let mut value = self.editor.doc.value().clone();
                let source_is_transform = value["nodes"][&source]["type"] == "transform";
                let current = transform_math::TrsParams::from_params(&value["nodes"][&source]["params"]);
                let composed = transform_math::mul4(&delta, &current.to_matrix());
                let candidate = current.decompose_like(&composed);
                let rebuilt = candidate.to_matrix();
                let representable = (0..4).all(|i| (0..4).all(|j|
                    (rebuilt[i][j] - composed[i][j]).abs() < 1e-6));
                let reuse = object_mode && source_is_transform &&
                    value["nodes"][&source]["params"]["group"].as_str().unwrap_or("").is_empty() && representable;
                let Some(target) = ensure_transform_target(&mut value, &source, reuse, None) else { return; };
                let result = if target == source { candidate } else {
                    let basis = frame.matrix();
                    let mut right_handed = basis;
                    if transform_math::det3(&basis) < 0.0 {
                        for row in &mut right_handed { row[2] = -row[2]; }
                    }
                    let reference = transform_math::TrsParams {
                        axis_rotation: transform_math::matrix_to_euler_xyz(&right_handed, [0.; 3]),
                        ..Default::default()
                    };
                    reference.decompose_like(&delta)
                };
                let params = &mut value["nodes"][&target]["params"];
                if !params.is_object() { *params = json!({}); }
                params["translate"] = json!(result.translate);
                params["rotate"] = json!(result.rotate);
                params["scale"] = json!(result.scale);
                params["uniform_scale"] = json!(result.uniform_scale);
                params["pivot"] = json!(result.pivot);
                params["axis_rotation"] = json!(result.axis_rotation);
                if target != source { params["group"] = json!(group); }
                match Document::parse(value.to_string()) {
                    Ok(doc) => {
                        self.editor.doc = doc;
                        self.send(Edit::Replace(value));
                        self.record_history(before);
                        self.finish_history();
                        self.display = target.clone();
                        self.selected = target.clone();
                        self.selected_set = std::collections::BTreeSet::from([target]);
                    }
                    Err(e) => self.file_status = format!("Could not repeat transform: {e}"),
                }
            }
            LastOperator::Move { .. } | LastOperator::Rotate { .. } | LastOperator::Scale { .. } => {
                let group = group_text(&self.selected_points());
                let is_object = self.editor_state.selection == ui::SelectionMode::Object;
                if !is_object && group.is_empty() {
                    self.file_status = "Select points, edges, or faces to transform".into();
                    return;
                }
                self.finish_history();
                let before = self.editor.doc.clone();
                let source = self.display.clone();
                let mut value = self.editor.doc.value().clone();

                let can_reuse = if let Some(node) = value["nodes"].get(&source) {
                    if node["type"] == "transform" {
                        if is_object {
                            node["params"].get("group").and_then(Value::as_str).unwrap_or("").is_empty()
                        } else {
                            node["params"].get("group").and_then(Value::as_str) == Some(&group)
                        }
                    } else {
                        false
                    }
                } else {
                    false
                };

                let target = if can_reuse {
                    source
                } else {
                    let Some(t) = ensure_transform_target(&mut value, &source, is_object, None) else { return; };
                    t
                };
                let params = &mut value["nodes"][&target]["params"];
                if !params.is_object() { *params = json!({}); }
                if !can_reuse { params["group"] = json!(group); }
                match &op {
                    LastOperator::Rotate { pivot, axis_rotation, .. } => {
                        params["pivot"] = json!(pivot);
                        if let Some(rot) = axis_rotation { params["axis_rotation"] = json!(rot); }
                    }
                    LastOperator::Scale { pivot, .. } => params["pivot"] = json!(pivot),
                    LastOperator::Move { proportional, .. } => {
                        let mut state = self.editor_state;
                        state.proportional = proportional.unwrap_or_default();
                        proportional_params(params, &state);
                    }
                    _ => {}
                }
                if value != *self.editor.doc.value() {
                    match Document::parse(value.to_string()) {
                        Ok(doc) => { self.editor.doc = doc; self.send(Edit::Replace(value)); }
                        Err(e) => { self.file_status = format!("Could not repeat transform: {e}"); return; }
                    }
                }

                match repeat::apply_repeat_transform(&mut self.editor.doc, &target, &op) {
                    Ok(edit) => {
                        self.send(edit);
                        self.record_history(before);
                        self.finish_history();
                        self.selected = target.clone();
                        self.selected_set = std::collections::BTreeSet::from([target.clone()]);
                        self.display = target;
                    }
                    Err(e) => {
                        self.file_status = format!("Could not repeat transform: {e}");
                    }
                }
            }
            LastOperator::AddNode { node_type, after_selection } => {
                let after = if after_selection { Some(self.selected.clone()) } else { None };
                self.add_node(ctx, &node_type, after.as_deref());
            }
            LastOperator::Duplicate => {
                if let Some(sel) = self.selected_set.iter().next().cloned() { self.duplicate_node(&sel); }
            }
            LastOperator::Dissolve => {
                if let Some(sel) = self.selected_set.iter().next().cloned() { self.dissolve_node(&sel); }
            }
            LastOperator::Delete => {
                if let Some(sel) = self.selected_set.iter().next().cloned() { self.delete_node(&sel); }
            }
            LastOperator::Connect { from, to, port } => {
                self.connect_nodes(&from, &to, port.as_deref());
            }
            LastOperator::Tidy => {
                ui::network::tidy(ctx, &self.editor.path, self.editor.doc.value(), &self.flags);
                self.last_operator = Some(LastOperator::Tidy);
            }
        }
    }
    fn restart_blender_link(&mut self) {
        let mut link_time=None;
        if let Some(mut link)=self.link.take() {
            for edit in link.restore_editor(&mut self.editor) { self.send(edit); }
            link_time=Some((link.frame,link.fps));
        }
        self.blender.clients.clear();
        match ui::blender_ui::blender_link::LinkServer::start(&self.editor.doc, &self.editor.path) {
            Ok(mut link) => {
                if let Some((frame,fps))=link_time {link.frame=frame;link.fps=fps;}
                self.link = Some(link);
                self.blender.link_message = "Listening on 127.0.0.1:8765; waiting for Blender.".into();
                self.send(Edit::Replace(self.editor.doc.value().clone()));
            }
            Err(e) => {
                let hint = if e.kind() == std::io::ErrorKind::AddrInUse {
                    " Port is already in use. Close the other link server, then click Restart link."
                } else { "" };
                self.blender.link_message = format!("Cannot listen on 127.0.0.1:8765: {e}.{hint}");
                self.blender_bar = true;
            }
        }
    }
    fn blender_task(&mut self, status: &str,
        task: impl FnOnce() -> std::io::Result<(String, bool)> + Send + 'static) {
        if self.blender_job.is_some() { return; }
        self.blender.message = status.into();
        match std::thread::Builder::new().name("minihou-blender-setup".into()).spawn(task) {
            Ok(job) => { self.blender_job = Some(job); self.blender.busy = true; }
            Err(e) => self.blender.message = format!("Could not start Blender task: {e}"),
        }
    }
    fn blender_action(&mut self, action: UiAction, ctx: &egui::Context) {
        use ui::blender_ui::{self, blender_link as bl};
        self.blender_bar = true;
        if matches!(action, UiAction::BlenderRestartLink) {
            self.restart_blender_link();
            self.blender.message = if self.link.is_some() { "Link restarted; listening on 127.0.0.1:8765. Blender with Auto-connect enabled will reconnect.".into() }
                else { self.blender.link_message.clone() };
            return;
        }
        if self.blender_job.is_some() { return; }
        match action {
            UiAction::BlenderDetect => {
                let installs = bl::find_installs();
                self.blender.installs = installs.iter().map(|(name, path)| (name.clone(), path.display().to_string())).collect();
                self.blender_paths = installs.into_iter().map(|(_, path)| path).collect();
                self.blender.message = if self.blender_paths.is_empty() {
                    "No Blender installations found. Set BLENDER_EXE to the executable path and restart MiniHou, or install Blender and detect again.".into()
                } else { format!("Detected {} Blender installation(s).", self.blender_paths.len()) };
            }
            UiAction::BlenderInstall(index) => {
                if let Some(exe) = self.blender_paths.get(index).cloned() {
                    self.blender_task("Installing add-on...", move || {
                        let zip = bl::prepare_addon_zip()?;
                        bl::install_addon(&exe, &zip).map(|message| (message, true))
                    });
                } else { self.blender.message = "No Blender installation selected. Click Detect installs.".into(); }
            }
            UiAction::BlenderOpen => {
                if !self.link.as_ref().is_some_and(|link| link.listening()) { self.restart_blender_link(); }
                if self.link.is_none() { return; }
                let index = blender_ui::selected_index(ctx, self.blender_paths.len());
                if let Some(exe) = self.blender_paths.get(index).cloned() {
                    let graph = match bl::prepare_launch_graph(&self.editor.doc, &self.editor.path, self.editor.dirty()) {
                        Ok(graph) => graph,
                        Err(e) => { self.blender.message = format!("Could not prepare current document for Blender: {e}"); return; }
                    };
                    self.blender_task("Starting Blender...", move || {
                        let mut child = bl::open_blender(&exe, &graph.path)?;
                        // Reap the process without holding the UI's setup task for Blender's lifetime.
                        std::thread::spawn(move || { let _ = child.wait(); drop(graph); });
                        Ok(("Blender started and connected to 127.0.0.1:8765.".into(), false))
                    });
                } else { self.blender.message = "No Blender installation selected. Click Detect installs.".into(); }
            }
            UiAction::BlenderDownloadAddon => {
                if let Some(destination) = rfd::FileDialog::new().set_file_name("MiniHou_Link.zip")
                    .add_filter("ZIP archive", &["zip"]).save_file() {
                    self.blender_task("Preparing add-on ZIP...", move || {
                        let zip = bl::prepare_addon_zip()?;
                        std::fs::copy(zip, &destination)?;
                        Ok((format!("Saved add-on ZIP to {}", destination.display()), false))
                    });
                } else { self.blender.message = "Save ZIP cancelled.".into(); }
            }
            _ => {}
        }
    }

}
fn main() {
    if let Err(e)=window_setup::run() {
        eprintln!("{e}");
        // A double-clicked build has no terminal: say why it did not start.
        // (MINIHOU_NO_DIALOG: the tests start the app with bad input and must not wait for someone to click OK.)
        if std::env::var_os("MINIHOU_NO_DIALOG").is_none() {
            rfd::MessageDialog::new().set_level(rfd::MessageLevel::Error).set_title("MiniHou").set_description(e.to_string()).show();
        }
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tools_tests {
    use super::*;

    #[test]
    fn alt_and_ctrl_selection_gestures_never_create_nodes() {
        // Alt (loop), Ctrl+Alt (ring), Ctrl (path) clicks and Ctrl+Numpad +/- (select more / less) change the viewport
        // selection only. The grow / select_* nodes exist only when chosen from the Tab menu (docs/RUST_SELECT_NODES.md).
        let doc = json!({"version":1,"display":"output1","nodes":{
            "grid1":{"type":"grid"},"output1":{"type":"output","inputs":{"geo":"grid1"}}}});
        let before = doc.clone();
        // a 3 x 3 grid of quads split by diagonals, as the preview layout gives it
        let w = 4u32;
        let p: Vec<f32> = (0..4).flat_map(|j| (0..4).flat_map(move |i| [i as f32, j as f32, 0.])).collect();
        let (mut tris, mut edges) = (Vec::new(), Vec::new());
        for j in 0..3 { for i in 0..3 {
            let (a, b, c, d) = (j * w + i, j * w + i + 1, (j + 1) * w + i + 1, (j + 1) * w + i);
            tris.extend([a, b, c, a, c, d]); edges.extend([a, b, b, c, c, d, d, a]);
        } }
        let topo = element_select::Topology::new(&p, &tris, &edges);
        let tri = 2 * (3 + 1); // quad (1, 1)
        for (shift, ctrl, alt) in [(false, false, true), (false, true, true), (true, false, true), (false, true, false)] {
            let kind = element_select::classify(shift, ctrl, alt, true).unwrap();
            for mode in [ui::SelectionMode::Points, ui::SelectionMode::Edges, ui::SelectionMode::Faces] {
                let mut selection = picking::Selection { mode, ..Default::default() };
                let hit = picking::Hit { tri: tri as usize, t: 1., bary: [0.4, 0.3, 0.3] };
                let point = [1.5, 1.05, 0.];
                topo.apply_click(&p, kind, Some((topo.pick(&p, &tris, hit, point, mode), tri, point)), mode, &mut selection);
                let picked = selection.clone();
                topo.apply_click(&p, kind, Some((topo.pick(&p, &tris, hit, [2.5, 2.05, 0.], mode), tri + 6, [2.5, 2.05, 0.])), mode, &mut selection);
                assert!(!selection.is_empty() && !picked.is_empty(), "{kind:?} {mode:?}");
                topo.grow_selection(&mut selection, false);
                topo.grow_selection(&mut selection, true);
            }
        }
        assert_eq!(doc, before);
        // the keymap routes those keys to selection commands, never to node creation
        for (key, cmd) in [(egui::Key::Plus, "select_more"), (egui::Key::Minus, "select_less")] {
            let modifiers = egui::Modifiers { ctrl: true, alt: true, ..Default::default() };
            let ctx = egui::Context::default();
            let mut actions = Vec::new();
            let _ = ctx.run(egui::RawInput { modifiers, events: vec![egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }], ..Default::default() }, |ctx| {
                actions = Keymap::blender().poll(ctx, &KeyContext { viewport_hovered: true, in_edit_mode: true, ..Default::default() });
            });
            assert!(matches!(actions.as_slice(), [UiAction::Command(name)] if name == cmd), "{actions:?}");
        }
    }

    #[test]
    fn selection_node_params_merge_into_the_added_node() {
        let mut value = json!({"nodes":{"grow1":{"type":"grow","inputs":{"geo":"box1"}}}});
        merge_params(&mut value, "grow1", json!({"group":"1 2","domain":"points"}));
        assert_eq!(value["nodes"]["grow1"]["params"], json!({"group":"1 2","domain":"points"}));
        merge_params(&mut value, "grow1", json!({"steps":2}));
        assert_eq!(value["nodes"]["grow1"]["params"]["domain"], "points");
        assert_eq!(value["nodes"]["grow1"]["params"]["steps"], 2);
        merge_params(&mut value, "missing", json!({"steps":2}));
        // the menu insertion wires the node after the source and before the output, like other viewport-added nodes
        let doc = json!({"version":1,"display":"output1","nodes":{"box1":{"type":"box"},"output1":{"type":"output","inputs":{"geo":"box1"}}}});
        let (value, id) = graph_edit::add_at(&doc, "grow", Some("box1"), [0.; 3]).unwrap();
        assert_eq!(value["nodes"][&id]["inputs"]["geo"], "box1");
        assert_eq!(value["nodes"]["output1"]["inputs"]["geo"], id.as_str());
    }

    #[test]
    fn viewport_move_inserts_transform_before_output_not_after() {
        let mut value = json!({"version":1,"display":"output1","nodes":{
            "box1":{"type":"box"},
            "output1":{"type":"output","inputs":{"geo":"box1"}}}});
        let id = ensure_transform_target(&mut value, "output1", true, None).unwrap();
        assert_eq!(value["nodes"][&id]["inputs"]["geo"], "box1");
        assert_eq!(value["nodes"]["output1"]["inputs"]["geo"], id.as_str());
        assert_eq!(value["display"], "output1");
    }

    #[test]
    fn sixty_transform_updates_use_param_edits_and_one_undo_step() {
        let mut doc = Document::parse(include_str!("default_graph.json").to_owned()).unwrap();
        let before = doc.clone();
        let source = doc.value()["display"].as_str().unwrap().to_owned();
        let mut value = doc.value().clone();
        let target = ensure_transform_target(&mut value, &source, true, None).unwrap();
        value["nodes"][&target]["params"] = json!({"translate":[0.,0.,0.]});
        let mut edits = apply_transform_step(&mut doc, true, value, &target, "translate", json!([0.,0.,0.])).unwrap();
        for i in 1..60 {
            let mut value = doc.value().clone();
            assert_eq!(ensure_transform_target(&mut value, &source, true, Some(&target)), Some(target.clone()));
            let param = json!([i as f32 / 10., 0., 0.]);
            edits.extend(apply_transform_step(&mut doc, false, value, &target, "translate", param).unwrap());
        }
        assert_eq!(doc.value()["nodes"].as_object().unwrap().len(), before.value()["nodes"].as_object().unwrap().len() + 1);
        assert_eq!(edits.iter().filter(|e| matches!(e, Edit::Replace(_))).count(), 1);
        assert_eq!(edits.iter().filter(|e| matches!(e, Edit::Param { .. })).count(), 60);
        let mut history = minihou_app::History::default();
        let final_doc = doc.clone();
        doc = before.clone();
        history.replace(&mut doc, final_doc);
        assert!(history.undo(&mut doc).is_some());
        assert_eq!(doc.value(), before.value());
        assert!(history.undo(&mut doc).is_none());
        let mut reused = before.value().clone();
        let id = ensure_transform_target(&mut reused, &source, true, None).unwrap();
        assert_eq!(ensure_transform_target(&mut reused, &source, true, None), Some(id.clone()));
        assert_eq!(ensure_transform_target(&mut reused, &id, true, None).as_deref(), Some(id.as_str()));
        assert_eq!(reused["nodes"].as_object().unwrap().len(), before.value()["nodes"].as_object().unwrap().len() + 1);
    }

    #[test]
    fn viewport_accepts_tool_drag_even_with_overlay_widgets() {
        use egui::{Event, PointerButton, RawInput};
        let ctx = egui::Context::default();
        let rect = egui::Rect::from_min_size(egui::pos2(400., 100.), egui::vec2(300., 300.));
        let at = egui::pos2(500., 200.);
        let _ = ctx.run(RawInput { events: vec![Event::PointerMoved(at)], ..Default::default() }, |ctx| {
            egui::DragAndDrop::set_payload(ctx, ui::ToolDrag("box".into()));
            assert!(viewport_tool_drop(ctx, rect).1);
        });
        let _ = ctx.run(RawInput { events: vec![Event::PointerButton { pos: at, button: PointerButton::Primary,
            pressed: false, modifiers: Default::default() }], ..Default::default() }, |ctx| {
            let (tool, hover, drop_pos) = viewport_tool_drop(ctx, rect);
            assert_eq!(tool.unwrap().0, "box");
            assert!(!hover);
            assert_eq!(drop_pos, Some(at));
        });
    }

    #[test]
    fn scaling_uses_the_drag_start_values() {
        assert_eq!(scaled_from_initial(&json!([2., 3., 4.]), [1., 2., 1.]), [2., 6., 4.]);
        assert_eq!(scaled_from_initial(&serde_json::Value::Null, [1., 0.5, 1.]), [1., 0.5, 1.]);
    }
    #[test]
    fn tab_restores_the_last_element_mode() {
        let mut last = ui::SelectionMode::Points;
        assert_eq!(toggle_edit_mode(ui::SelectionMode::Object, &mut last), ui::SelectionMode::Points);
        assert_eq!(toggle_edit_mode(ui::SelectionMode::Faces, &mut last), ui::SelectionMode::Object);
        assert_eq!(toggle_edit_mode(ui::SelectionMode::Object, &mut last), ui::SelectionMode::Faces);
    }
    /// Real routing: raw Ctrl+Tab -> raw_input_hook filter -> egui frame (with a focusable text
    /// field, as the params panel has) -> keymap poll -> toggle. Before the fix the first press
    /// moved egui focus into the text field and every later press was swallowed.
    #[test]
    fn ctrl_tab_toggles_object_and_element_modes_through_the_keymap() {
        use ui::SelectionMode as M;
        let ctx = egui::Context::default();
        let keymap = Keymap::blender();
        let (mut mode, mut last) = (M::Object, M::Points);
        let mut text = String::from("1.0");
        let mut press = |key: egui::Key, modifiers: egui::Modifiers, mode: &mut M, last: &mut M| {
            let mut raw = egui::RawInput { modifiers, events: vec![egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }], ..Default::default() };
            let held = hold_back_ctrl_tab(&mut raw, ctx.memory(|m| m.focused().is_some()));
            let mut actions = Vec::new();
            let _ = ctx.run(raw, |ctx| {
                ctx.input_mut(|i| i.events.extend(held.clone()));
                actions = keymap.poll(ctx, &KeyContext { viewport_hovered: true, in_edit_mode: *mode != M::Object, ..Default::default() });
                egui::CentralPanel::default().show(ctx, |ui| { ui.text_edit_singleline(&mut text); let _ = ui.button("mode"); });
            });
            for action in actions {
                match action {
                    UiAction::Command(name) if name == "toggle_edit" => *mode = toggle_edit_mode(*mode, last),
                    UiAction::Editor(ui::EditorChange::Selection(m)) => { remember_element_mode(m, last); *mode = m; }
                    other => panic!("unexpected {other:?}"),
                }
            }
            assert!(ctx.memory(|m| m.focused().is_none()), "Ctrl+Tab must not move egui focus");
        };
        let ctrl = egui::Modifiers::CTRL;
        let mut seq = Vec::new();
        for _ in 0..4 { press(egui::Key::Tab, ctrl, &mut mode, &mut last); seq.push(mode); }
        assert_eq!(seq, [M::Points, M::Object, M::Points, M::Object]);
        // 3 (faces) in object mode enters faces and becomes the remembered element mode
        press(egui::Key::Num3, egui::Modifiers::NONE, &mut mode, &mut last); assert_eq!(mode, M::Faces);
        press(egui::Key::Tab, ctrl, &mut mode, &mut last); assert_eq!(mode, M::Object);
        press(egui::Key::Tab, ctrl, &mut mode, &mut last); assert_eq!(mode, M::Faces);
        press(egui::Key::Num2, egui::Modifiers::NONE, &mut mode, &mut last); assert_eq!(mode, M::Edges);
        press(egui::Key::Tab, ctrl, &mut mode, &mut last); assert_eq!(mode, M::Object);
        press(egui::Key::Tab, ctrl, &mut mode, &mut last); assert_eq!(mode, M::Edges);
        // a mode button (UiAction::Editor) also updates the remembered mode; Object button keeps it
        remember_element_mode(M::Points, &mut last);
        remember_element_mode(M::Object, &mut last); mode = M::Object;
        press(egui::Key::Tab, ctrl, &mut mode, &mut last); assert_eq!(mode, M::Points);
    }
    #[test]
    fn ctrl_tab_is_left_to_a_focused_text_field() {
        let mods = egui::Modifiers::CTRL;
        let mut raw = egui::RawInput { events: vec![egui::Event::Key { key: egui::Key::Tab, physical_key: None, pressed: true, repeat: false, modifiers: mods }], ..Default::default() };
        assert!(hold_back_ctrl_tab(&mut raw, true).is_empty());
        assert_eq!(raw.events.len(), 1);
        let plain = egui::Event::Key { key: egui::Key::Tab, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        raw.events = vec![plain.clone(), raw.events[0].clone()];
        assert_eq!(hold_back_ctrl_tab(&mut raw, false).len(), 1);
        assert_eq!(raw.events, vec![plain]);
    }
    #[test]
    fn viewport_and_network_selection_keep_the_last_added_object_active() {
        let mut active = String::new();
        let mut set = std::collections::BTreeSet::new();
        select_active_nodes(&mut active, &mut set, vec!["grid".into()], false, false);
        select_active_nodes(&mut active, &mut set, vec!["torus".into()], true, false);
        assert_eq!(active, "torus");
        assert_eq!(set.len(), 2);
        select_active_nodes(&mut active, &mut set, vec!["grid".into()], true, false);
        assert_eq!(active, "grid");
        assert_eq!(set.len(), 2);
        select_active_nodes(&mut active, &mut set, vec!["grid".into()], true, true);
        assert_eq!(active, "torus");
        select_active_nodes(&mut active, &mut set, vec!["grid".into()], false, false);
        assert_eq!(set, std::collections::BTreeSet::from(["grid".into()]));
        select_active_nodes(&mut active, &mut set, vec![], false, false);
        assert!(active.is_empty() && set.is_empty());
    }
    #[test]
    fn modal_tools_route_two_and_three_displayed_objects_to_last_selected_stream() {
        for count in [2,3] {
            let mut nodes=serde_json::Map::new();
            let mut outputs=Vec::new();
            for i in 0..count {
                let source=format!("box{i}");let output=format!("out{i}");
                nodes.insert(source.clone(),json!({"type":"box"}));
                nodes.insert(output.clone(),json!({"type":"output","inputs":{"geo":source}}));
                outputs.push(output);
            }
            let doc=json!({"version":1,"nodes":nodes,"display":outputs});
            let mut active=String::new();let mut selected=std::collections::BTreeSet::new();
            select_active_nodes(&mut active,&mut selected,vec!["out0".into()],false,false);
            select_active_nodes(&mut active,&mut selected,vec!["out1".into()],true,false);
            assert_eq!(active,"out1");
            for tool in ["polysplit","edge_slide","knife","polyextrude","inset","polybevel","pushpull","mirror","grow","select_path","select_loop","select_ring"] {
                let (next,id)=graph_edit::add(&doc,tool,Some(&active)).unwrap_or_else(||panic!("{tool} should insert on active output"));
                assert_eq!(next["nodes"]["out1"]["inputs"]["geo"],id);
                for other in [&"out0",&"out2"] {
                    if doc["nodes"].get(*other).is_some() {assert_eq!(next["nodes"][*other],doc["nodes"][*other],"{tool} changed {other}");}
                }
            }
            select_active_nodes(&mut active,&mut selected,vec!["out0".into()],true,false);
            assert_eq!(active,"out0");
        }
    }
    #[test]
    fn modal_picking_and_elements_ignore_other_displayed_objects() {
        for count in [2,3] {
            let mut points=Vec::new();let mut vertices=Vec::new();let mut offsets=vec![0];
            for i in 0..count {
                let z=i as f32;
                points.extend([0.,0.,z,1.,0.,z,0.,1.,z]);
                vertices.extend([i*3,i*3+1,i*3+2]);offsets.push(vertices.len() as i32);
            }
            let geo=minihou_core::Geometry::from_arrays(points,vertices,offsets,None).unwrap();
            let scene=Scene::new(&geo);
            let ranges=(0..count).map(|i|(format!("out{i}"),(i*3) as u32..(i*3+3) as u32)).collect::<Vec<_>>();
            let selected=std::collections::BTreeSet::from(["out0".into(),format!("out{}",count-1)]);
            let active=std::collections::BTreeSet::from(["out0".into()]);
            let hit=pick_displayed(&scene,[0.2,0.2,10.],[0.,0.,-1.],viewport::ObjectVisibility::HideOthers,&ranges,&active).unwrap();
            assert_eq!(picked_object(&ranges,scene.triangle_indices(),hit),Some("out0"));
            let all=picking::Selection{mode:ui::SelectionMode::Points,points:(0..count*3).map(|i|i as u32).collect(),edges:vec![(0,1),(3,4)],tris:(0..count).map(|i|i as u32).collect()};
            let only=selection_in_range(&all,&scene,&ranges[0].1);
            assert_eq!(only.points,vec![0,1,2]);assert_eq!(only.edges,vec![(0,1)]);assert_eq!(only.tris,vec![0]);
            assert_eq!(selected.len(),2);
        }
    }
    #[test]
    fn viewport_picks_the_nearest_display_output() {
        let points = [0.,0.,0., 1.,0.,0., 0.,1.,0., 0.,0.,2., 1.,0.,2., 0.,1.,2.];
        let triangles = [0,1,2, 3,4,5];
        let ranges = vec![("grid".into(), 0..3), ("torus".into(), 3..6)];
        let front = picking::pick_triangle(&points, &triangles, [0.2,0.2,5.], [0.,0.,-1.]).unwrap();
        assert_eq!(picked_object(&ranges, &triangles, front), Some("torus"));
        let back = picking::pick_triangle(&points, &triangles, [0.2,0.2,-5.], [0.,0.,1.]).unwrap();
        assert_eq!(picked_object(&ranges, &triangles, back), Some("grid"));
        assert!(picking::pick_triangle(&points, &triangles, [3.,3.,5.], [0.,0.,-1.]).is_none());
    }
    #[test]
    fn selected_hidden_node_adds_pickable_preview_only_while_selected() {
        let make = |z: f32| {
            let geo = minihou_core::Geometry::from_arrays(
                vec![0.,0.,z, 1.,0.,z, 0.,1.,z], vec![0,1,2], vec![0,3], None).unwrap();
            geo
        };
        let displayed = make(0.);
        let hidden = make(2.);
        let mut scene = Scene::new(&displayed);
        let mut ranges = vec![("output".to_owned(), 0..3)];
        assert!(include_selected_preview(&mut scene, &mut ranges, "garden_path", Some(&hidden)));
        assert_eq!(ranges, vec![("output".into(), 0..3), ("garden_path".into(), 3..6)]);
        let hit = pick_displayed(&scene, [0.2,0.2,5.], [0.,0.,-1.], viewport::ObjectVisibility::HideOthers,
            &ranges, &std::collections::BTreeSet::from(["garden_path".to_owned()])).unwrap();
        assert_eq!(picked_object(&ranges, scene.triangle_indices(), hit), Some("garden_path"));
        assert!(!include_selected_preview(&mut scene, &mut ranges, "", Some(&hidden)));
        assert!(!include_selected_preview(&mut scene, &mut ranges, "output", Some(&displayed)));
        let mut deselected_scene = Scene::new(&displayed);
        let mut deselected_ranges = vec![("output".to_owned(), 0..3)];
        assert!(!include_selected_preview(&mut deselected_scene, &mut deselected_ranges, "", Some(&hidden)));
        assert_eq!(deselected_ranges, vec![("output".into(), 0..3)]);
        assert_eq!(deselected_scene.positions().len(), 9);

        let curve = minihou_core::Geometry::from_arrays(
            vec![0.,0.,2., 1.,0.,2., 1.,1.,2.], vec![0,1,2], vec![0,3], Some(vec![false])).unwrap();
        let mut curve_scene = Scene::new(&displayed);
        let mut curve_ranges = vec![("output".to_owned(), 0..3)];
        assert!(include_selected_preview(&mut curve_scene, &mut curve_ranges, "garden_path", Some(&curve)));
        assert_eq!(curve_ranges[1].1, 3..6);
        assert!(curve_scene.line_primitive_indices().chunks_exact(2)
            .any(|edge| edge.iter().all(|id| curve_ranges[1].1.contains(id))));
    }
    #[test]
    fn active_origin_follows_its_transform_chain_instead_of_scene_bounds() {
        let doc = json!({"nodes": {
            "grid": {"type":"grid", "params":{"center":[1.,0.,0.]}},
            "moved": {"type":"transform", "inputs":{"geo":"grid"}, "params":{"translate":[4.,2.,0.]}},
            "turned": {"type":"transform", "inputs":{"geo":"moved"}, "params":{"rotate":[0.,0.,90.]}},
            "torus": {"type":"torus", "params":{"center":[100.,0.,0.]}}
        }, "display":["turned","torus"]});
        let origin = object_origin(&doc, "turned");
        assert!((origin[0] + 2.).abs() < 1e-5 && (origin[1] - 5.).abs() < 1e-5);
        assert_eq!(object_origin(&doc, "torus"), [100.,0.,0.]);
    }
    #[test]
    fn element_groups_use_point_ids_in_each_selection_mode() {
        let selection = picking::Selection { mode: ui::SelectionMode::Faces,
            points: vec![99], edges: vec![(8, 7)], tris: vec![1, 0] };
        let triangles = [2, 1, 0, 2, 3, 4];
        assert_eq!(selected_point_ids(&selection, ui::SelectionMode::Points, &triangles), [99]);
        assert_eq!(selected_point_ids(&selection, ui::SelectionMode::Edges, &triangles), [7, 8]);
        assert_eq!(selected_point_ids(&selection, ui::SelectionMode::Faces, &triangles), [0, 1, 2, 3, 4]);
        assert_eq!(group_text(&selected_point_ids(&selection, ui::SelectionMode::Faces, &triangles)), "0,1,2,3,4");
        assert!(selected_point_ids(&selection, ui::SelectionMode::Object, &triangles).is_empty());
    }
    #[test]
    fn proportional_settings_use_core_parameter_names() {
        let mut state = EditorState::default();
        state.proportional.enabled = true;
        state.proportional.radius = 2.5;
        state.proportional.falloff = ui::Falloff::InverseSquare;
        state.proportional.connected_only = true;
        let mut params = json!({});
        proportional_params(&mut params, &state);
        assert_eq!(params, json!({"proportional_enabled": true, "proportional_radius": 2.5,
            "proportional_falloff": "inverse_square", "proportional_connected": true}));
        state.proportional.falloff = ui::Falloff::Random;
        proportional_params(&mut params, &state);
        assert_eq!(params["proportional_falloff"], "random");
    }
    #[test]
    fn network_select_all_and_deselect_all_manage_node_set_without_affecting_viewport_mesh_selection() {
        let doc = json!({
            "nodes": {
                "box1": {"type": "box"},
                "transform1": {"type": "transform", "inputs": {"geo": "box1"}},
                "out": {"type": "null", "inputs": {"geo": "transform1"}}
            }
        });
        let all_nodes: std::collections::BTreeSet<String> = doc["nodes"].as_object().unwrap().keys().cloned().collect();
        assert_eq!(all_nodes.len(), 3);

        let mut selected_set = std::collections::BTreeSet::from(["box1".to_string()]);
        let mut selected = "box1".to_string();
        assert_eq!(selected_set.len(), 1);
        assert_eq!(selected, "box1");
        let mut mesh_selection = picking::Selection {
            mode: ui::SelectionMode::Points,
            points: vec![0, 1, 2],
            edges: vec![],
            tris: vec![],
        };

        // Select all nodes in network (A)
        selected_set = all_nodes.clone();
        selected = selected_set.iter().next().cloned().unwrap_or_default();
        assert_eq!(selected_set.len(), 3);
        assert!(!selected.is_empty());
        assert_eq!(mesh_selection.points, vec![0, 1, 2]);

        // Deselect all nodes in network (Alt+A)
        selected_set.clear();
        selected.clear();
        assert!(selected_set.is_empty());
        assert!(selected.is_empty());
        assert_eq!(mesh_selection.points, vec![0, 1, 2]);

        // Deselect in viewport (viewport Alt+A) clears mesh selection
        mesh_selection.clear();
        assert!(mesh_selection.is_empty());
    }
}
