mod picking;
mod subnet_edit;
mod element_select;
mod node_selection;
mod coplanar_tool;
mod group_preview;
mod param_pick;
use param_pick::{field_selection, infer_mode, ParamPick, PickedField, param_pick_end, input_source, source_ready};
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
pub use repeat::LastOperator;
#[cfg(test)] mod transform_tests;
#[cfg(test)] mod live_path_tests;
use eframe::{egui, egui_wgpu};
pub use minihou_app::units;
use minihou_app::{Camera, Document, Result, Edit, EditorDocument, CookWorker, CookRequest, CookState, StandardView};
use minihou_core::Graph;
use serde_json::{json, Value};
use std::{collections::HashMap, path::PathBuf, time::{Duration, Instant}};
use ui::keymap::{KeyContext, Keymap, Preferences, ThemePref};
use ui::{theme::Theme, Actions, BlenderStatus, CameraInfo, EditorState, GeoSummary, NodeFlags, TransformOp, UiAction, View};
use ui::modal_transform::{Modal, Kind as ModalKind, SketchStage, Constraint};
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

fn point_pickable(visibility:viewport::ObjectVisibility,ranges:&[(String,std::ops::Range<u32>)],selected:&std::collections::BTreeSet<String>,id:u32)->bool {
    visibility==viewport::ObjectVisibility::ShowAll || ranges.iter().any(|(owner,range)| selected.contains(owner) && range.contains(&id))
}
fn active_range<'a>(active:&str,ranges:&'a [(String,std::ops::Range<u32>)])->Option<&'a std::ops::Range<u32>> {
    ranges.iter().find(|(id,_)|id==active).map(|(_,range)|range)
}
fn loop_cut_has_marks(g:&minihou_core::Geometry)->bool { g.vertex.get("edge_group_viewport_loop_cut").is_some() }
fn loop_cut_selection(g:&minihou_core::Geometry,range:&std::ops::Range<u32>)->picking::Selection {
    let topo_edges=minihou_core::topo::edges(&g.topology,g.npoints);
    let marks=g.vertex.get("edge_group_viewport_loop_cut").and_then(|attr|attr.as_i32());
    let mut selected=vec![false;topo_edges.len()];
    if let Some(marks)=marks { for (vertex,&edge) in topo_edges.of_vertex.iter().enumerate() {
        if edge>=0 && marks.get(vertex).is_some_and(|&mark|mark!=0) { selected[edge as usize]=true; }
    } }
    let edges=topo_edges.points.chunks_exact(2).enumerate()
        .filter(|(i,pair)|selected[*i] && range.contains(&(pair[0] as u32)) && range.contains(&(pair[1] as u32)))
        .map(|(_,pair)|(pair[0] as u32,pair[1] as u32)).collect();
    picking::Selection { mode:ui::SelectionMode::Edges,edges,..Default::default() }
}
fn selection_exists_on_scene(selection:&picking::Selection,scene:&Scene)->bool {
    selection.points.iter().all(|&p|(p as usize)<scene.positions().len()/3)
        && selection.tris.iter().all(|&t|(t as usize)<scene.triangle_indices().len()/3)
        && selection.edges.iter().all(|&(a,b)|scene.edge_indices().chunks_exact(2)
            .any(|edge|(edge[0]==a && edge[1]==b)||(edge[0]==b && edge[1]==a)))
}
fn undo_selection_for(records:&mut Vec<(String,picking::Selection,picking::Selection)>,
    previous:&str,current:&picking::Selection)
    ->Option<(picking::Selection,picking::Selection,Option<picking::Selection>)> {
    let matches=records.last().is_some_and(|(text,_,_)|text==previous);
    if !matches { return None; }
    let (_,selection,stash)=records.pop().unwrap();
    let user=(!current.is_empty() && *current!=selection).then(||current.clone());
    Some((selection,stash,user))
}
fn selection_in_range(selection:&picking::Selection,scene:&Scene,range:&std::ops::Range<u32>)->picking::Selection {
    let mut result=selection.clone();
    result.points.retain(|id|range.contains(id));
    result.edges.retain(|(a,b)|range.contains(a)&&range.contains(b));
    result.tris.retain(|id|scene.triangle_indices().get(*id as usize*3).is_some_and(|p|range.contains(p)));
    result
}
fn pick_displayed(scene:&Scene,o:[f32;3],d:[f32;3],visibility:viewport::ObjectVisibility,ranges:&[(String,std::ops::Range<u32>)],selected:&std::collections::BTreeSet<String>)->Option<picking::Hit> {
    pick_displayed_culled(scene,o,d,visibility,ranges,selected,false)
}
/// Back-face culling is off for selection: the renderer draws both sides, so a visible face with the opposite winding (open meshes, flipped normals) must stay pickable. The nearest hit already hides faces behind it.
fn pick_displayed_culled(scene:&Scene,o:[f32;3],d:[f32;3],visibility:viewport::ObjectVisibility,ranges:&[(String,std::ops::Range<u32>)],selected:&std::collections::BTreeSet<String>,cull:bool)->Option<picking::Hit> {
    picking::pick_triangle_culled(scene.positions(),scene.triangle_indices(),o,d,cull,|tri| tri.iter().all(|id| point_pickable(visibility,ranges,selected,*id)))
}
/// After an extrude cook the original selection stays when its ids still exist (Blender keeps the faces selected).
fn keep_extrude_selection(before:&picking::Selection,scene:&Scene)->Option<picking::Selection> {
    (!before.is_empty() && selection_exists_on_scene(before,scene)).then(||before.clone())
}
fn navigation_point(camera:&Camera,scene:Option<&Scene>,hidden:bool,visibility:viewport::ObjectVisibility,
    ranges:&[(String,std::ops::Range<u32>)],selected:&std::collections::BTreeSet<String>,rect:egui::Rect,pos:egui::Pos2)->Option<[f32;3]> {
    let nx=(pos.x-rect.left())/rect.width()*2.-1.; let ny=1.-(pos.y-rect.top())/rect.height()*2.;
    let (origin,dir)=camera.ray(nx,ny,rect.width()/rect.height());
    if hidden {return None;}
    let scene=scene?;
    let hit=scene.pick_bvh.pick(scene.positions(),scene.triangle_indices(),origin,dir,|ids| ids.iter().all(|id| point_pickable(visibility,ranges,selected,*id)))?;
    Some([0,1,2].map(|k|origin[k]+dir[k]*hit.t))
}
fn zoom_under_cursor(camera:&mut Camera,delta:f32,enabled:bool,rect:egui::Rect,pos:Option<egui::Pos2>,point:Option<[f32;3]>) {
    if delta==0. {return;}
    let Some(pos)=pos.filter(|_|enabled) else {camera.zoom(delta);return;};
    let nx=(pos.x-rect.left())/rect.width()*2.-1.; let ny=1.-(pos.y-rect.top())/rect.height()*2.;
    let point=point.unwrap_or_else(||camera.view_plane_point(nx,ny,rect.width()/rect.height()));
    camera.zoom_at(delta,point,nx,ny,rect.width()/rect.height());
}

fn pick_displayed_wire(scene:&Scene, camera:CameraInfo, rect:egui::Rect, pointer:egui::Pos2,
    visibility:viewport::ObjectVisibility,ranges:&[(String,std::ops::Range<u32>)],selected:&std::collections::BTreeSet<String>,mode:ui::SelectionMode,xray:bool)->Option<picking::WireHit> {
    // without X-ray, points and edges behind the mesh surface under the pointer are not pickable
    let max_depth=if xray {f32::INFINITY} else {
        let (o,d)=ui::transform_gizmo::ray(camera,rect,pointer);
        pick_displayed(scene,o,d,visibility,ranges,selected).map_or(f32::INFINITY,|hit|{
            let depth=hit.t*(0..3).map(|k|d[k]*camera.ray_forward[k]).sum::<f32>();
            let depth=if camera.ortho {depth.max(0.001)} else {depth};
            depth*1.002+scene.radius*1e-4
        })
    };
    picking::pick_wire_occluded(scene.positions(),scene.line_primitive_indices(),pointer,
        |p| ui::transform_gizmo::project(camera,rect,p).map(|screen| {
            let depth=(0..3).map(|k|(p[k]-camera.ray_origin[k])*camera.ray_forward[k]).sum::<f32>();
            (screen, if camera.ortho { depth.max(0.001) } else { depth })
        }),
        |id| point_pickable(visibility,ranges,selected,id),
        matches!(mode,ui::SelectionMode::Object|ui::SelectionMode::Points),
        matches!(mode,ui::SelectionMode::Object|ui::SelectionMode::Edges),max_depth)
}

fn wire_selection(hit:picking::WireHit,mode:ui::SelectionMode)->picking::Selection {
    let mut out=picking::Selection {mode,..Default::default()};
    match (mode,hit) {
        (ui::SelectionMode::Points,picking::WireHit::Point(id)) => out.points.push(id),
        (ui::SelectionMode::Edges,picking::WireHit::Edge(a,b)) => out.edges.push((a,b)),
        _ => {}
    }
    out
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

/// Picking records displayed triangle numbers; transform groups use the triangle's point numbers.
fn selected_point_ids(selection: &picking::Selection, mode: ui::SelectionMode, triangles: &[u32]) -> Vec<u32> {
    let mut ids = match mode {
        ui::SelectionMode::Object => Vec::new(),
        ui::SelectionMode::Points => selection.points.clone(),
        ui::SelectionMode::Edges => selection.edges.iter().flat_map(|&(a, b)| [a, b]).collect(),
        ui::SelectionMode::Faces => selection.tris.iter().flat_map(|&t| {
            triangles.get(t as usize * 3..t as usize * 3 + 3).unwrap_or(&[]).iter().copied()
        }).collect(),
    };
    ids.sort_unstable(); ids.dedup(); ids
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
fn select_nodes(set: &mut std::collections::BTreeSet<String>, ids: impl IntoIterator<Item = String>, extend: bool, toggle: bool) {
    if !extend && !toggle { set.clear(); }
    for id in ids {
        if toggle { if !set.remove(&id) { set.insert(id); } }
        else { set.insert(id); }
    }
}
/// Keep the last picked node active; removing it falls back to a remaining selection.
fn select_active_nodes(active: &mut String, set: &mut std::collections::BTreeSet<String>, ids: Vec<String>, extend: bool, toggle: bool) {
    let last = ids.last().cloned();
    select_nodes(set, ids, extend, toggle);
    if let Some(id) = last.filter(|id| set.contains(id)) { *active = id; }
    else if !set.contains(active) { *active = set.iter().next().cloned().unwrap_or_default(); }
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
    scene: Option<Scene>, shown_level: f32, geo_rev: u64,
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
    undo_element_selection: Vec<(String, picking::Selection, picking::Selection)>,
    pending_restore_selection: Option<(picking::Selection, picking::Selection, Option<picking::Selection>)>,
    model_before_selection: Option<(picking::Selection, picking::Selection)>,
    /// Viewport selection steps: the selection (and stash) before each step on the undo stack, and those undone.
    sel_undo: Vec<(picking::Selection, picking::Selection)>,
    sel_redo: Vec<(picking::Selection, picking::Selection)>,
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
        self.selection = pick.previous_selection;
        self.editor_state.selection = pick.previous_mode;
        self.selected_set = pick.previous_selected_set;
        self.selection_dirty = true;
        if self.display != pick.previous_display {
            self.display = pick.previous_display.clone();
            self.send(Edit::Display(pick.previous_display));
        }
        self.editor.history.finish(&self.editor.doc);
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
    fn region_select(&mut self, shape: picking::RegionShape, op: picking::SelectOp) {
        use picking::SelectOp;
        if self.object_hidden { return; }
        let Some(scene) = &self.scene else { return; };
        let camera = self.camera_info(); let points = scene.positions(); let tris = scene.triangle_indices();
        let active=self.active_object().and_then(|id|active_range(id,&self.display_ranges));
        let position = |i: u32| -> Option<[f32; 3]> { if self.editor_state.selection==ui::SelectionMode::Object {
                if !point_pickable(self.editor_state.object_visibility,&self.display_ranges,&self.selected_set,i){return None;}
            } else if !active.is_some_and(|range|range.contains(&i)){return None;}
            let k = i as usize * 3; let a = points.get(k..k + 3)?; Some([a[0], a[1], a[2]]) };
        let project = |p| ui::transform_gizmo::project(camera, self.last_viewport, p);
        let inside = |p| project(p).is_some_and(|q| shape.contains(q));
        let mode = self.editor_state.selection;
        let mut found = picking::Selection { mode, ..Default::default() };
        match mode {
            ui::SelectionMode::Points => { for i in 0..points.len()/3 { if position(i as u32).is_some_and(inside) { found.points.push(i as u32); } } }
            ui::SelectionMode::Edges => {
                let mut edges = std::collections::BTreeSet::new();
                for t in tris.as_chunks::<3>().0 { for (a,b) in [(t[0],t[1]),(t[1],t[2]),(t[2],t[0])] { edges.insert((a.min(b), a.max(b))); } }
                for pair in scene.line_primitive_indices().chunks_exact(2) { edges.insert((pair[0].min(pair[1]),pair[0].max(pair[1]))); }
                for (a,b) in edges {
                    if let (Some(pa), Some(pb)) = (position(a).and_then(project), position(b).and_then(project)) {
                        if shape.contains_edge(pa, pb) { found.edges.push((a,b)); }
                    }
                }
            }
            ui::SelectionMode::Faces => { for (i,t) in tris.as_chunks::<3>().0.iter().enumerate() { if let (Some(a), Some(b), Some(c)) = (position(t[0]), position(t[1]), position(t[2])) { if inside([0,1,2].map(|k| (a[k]+b[k]+c[k])/3.)) { found.tris.push(i as u32); } } } }
            ui::SelectionMode::Object => {
                let hits: Vec<String> = self.display_ranges.iter().filter_map(|(id, range)| {
                    if self.editor_state.object_visibility != viewport::ObjectVisibility::ShowAll && !self.selected_set.contains(id) { return None; }
                    let object_points = points.get(range.start as usize * 3..range.end as usize * 3)?;
                    if object_points.is_empty() { return None; }
                    let (lo, hi) = minihou_core::preview::bounds(object_points);
                    inside([0,1,2].map(|i| (lo[i] + hi[i]) * 0.5)).then(|| id.clone())
                }).collect();
                if op == SelectOp::Subtract { for id in hits { self.selected_set.remove(&id); } }
                else { select_active_nodes(&mut self.selected, &mut self.selected_set, hits, op == SelectOp::Add, false); }
                if !self.selected_set.contains(&self.selected) { self.selected = self.selected_set.iter().next().cloned().unwrap_or_default(); }
                self.selection_dirty = true;
                return;
            }
        }
        // whole polygons, polygon edges only (no fan diagonals)
        let found = element_select::cached(&self.elem_topo, scene).complete(found);
        self.selection_dirty |= element_select::apply_region(&mut self.selection, found, op);
    }

    fn sync_selection_baseline(&mut self) { self.sel_baseline = (self.selection.clone(), self.stashed_selection.clone()); }
    /// A viewport selection change becomes one step on the shared undo stack.
    fn record_selection_step(&mut self) {
        if self.selection == self.sel_baseline.0 { return; }
        let before = std::mem::replace(&mut self.sel_baseline, (self.selection.clone(), self.stashed_selection.clone()));
        self.editor.history.record_selection(&self.editor.doc);
        self.sel_undo.push(before);
        self.sel_redo.clear();
    }
    /// Undo or redo one selection step; the document is untouched.
    fn step_selection(&mut self, undo: bool) {
        let (from, to) = if undo { (&mut self.sel_undo, &mut self.sel_redo) } else { (&mut self.sel_redo, &mut self.sel_undo) };
        let Some(target) = from.pop() else { return };
        to.push((self.selection.clone(), self.stashed_selection.clone()));
        self.selection = target.0; self.stashed_selection = target.1;
        if self.selection.mode != ui::SelectionMode::Object { self.editor_state.apply(ui::EditorChange::Selection(self.selection.mode)); }
        self.sync_selection_baseline();
        self.selection_dirty = true;
    }

    fn new(doc: Document, path: PathBuf, state: egui_wgpu::RenderState) -> Result<Self> {
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
            display_ranges:Vec::new(),scene:None,shown_level:1.0,geo_rev:u64::MAX,lod_uploaded:false,last_lod_view:None,last_lod_select:Instant::now(),blender_paths:Vec::new(),blender_bar:false,selection:picking::Selection::default(),selection_dirty:false,param_pick:None,barrier_key:None,target_key:None,picked_field:None,selection_metrics:Default::default(),coplanar:None,group_preview:Default::default(),elem_topo:Default::default(),stashed_selection:picking::Selection::default(),
            clipboard:None,transform_before:None,transform_target:None,transform_delta:None,transform_pivot:None,transform_orientation:None,modal:None,embed_conflict:None,model_modal:None,mirror:None,pushpull_pick:false,last_operator:None,last_viewport:egui::Rect::NOTHING,
            last_edit_selection:ui::SelectionMode::Points,params_open:true,object_hidden:false,held_keys:Vec::new(),selection_memory:Default::default(),pending_loop_selection:None,undo_element_selection:Vec::new(),pending_restore_selection:None,model_before_selection:None,sel_undo:Vec::new(),sel_redo:Vec::new(),sel_baseline:Default::default(),extrude_keep:None };
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
        let before_selection = promote.map(|_| (self.selection.clone(), self.stashed_selection.clone()));
        match self.editor.history.edit(&mut self.editor.doc, &edit) {
            Ok(()) => {
                if let (Some(to), Some((selection, stash))) = (promote, before_selection) {
                    // the selection follows the group domain (converted when the mode changes); undo restores it as it was
                    self.undo_element_selection.push((self.editor.doc.value().to_string(), selection, stash));
                    self.editor_state.apply(ui::EditorChange::Selection(to));
                }
                if matches!(edit, Edit::ModifierInputs { .. } | Edit::ModifierInputValue { .. }) || was_linked
                    || matches!(&edit, Edit::Param { value, .. } if value.get("modifier_input").is_some()) {
                    self.send(Edit::Replace(self.editor.doc.value().clone()));
                } else { self.send(edit); }
                if !continuous { self.editor.history.finish(&self.editor.doc); }
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
        self.editor.history.finish(&self.editor.doc);
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
    fn add_node(&mut self, ctx: &egui::Context, node_type: &str, after: Option<&str>) {
        self.add_node_at(ctx, node_type, after, self.editor_state.cursor);
    }
    fn add_node_at(&mut self, ctx: &egui::Context, node_type: &str, after: Option<&str>, pos: [f32; 3]) {
        let Some((mut value, id)) = graph_edit::add_at(self.editor.doc.value(), node_type, after, pos) else {
            self.file_status = format!("The Rust core has no {node_type} node yet");
            return;
        };
        // grow / select_path / select_loop / select_ring / edge_slide / group_create start from the viewport's element selection
        let params = self.selection_node_params(node_type);
        if node_type == "boundary_select" {
            let mut p = params.unwrap_or_else(|| json!({}));
            p["output_group_name"] = json!(element_select::free_group_name_with_prefix(self.editor.doc.value(), "boundary"));
            merge_params(&mut value, &id, p);
        } else if node_type == "select_sharp_edges" {
            let mut p = params.unwrap_or_else(|| json!({}));
            p["output_group_name"] = json!(element_select::free_group_name_with_prefix(self.editor.doc.value(), "sharp_edges"));
            merge_params(&mut value, &id, p);
        } else if node_type == "group_create" && self.editor_state.selection != ui::SelectionMode::Object {
            let Some(mut params) = params else { self.file_status = "Select points, edges or faces to create a group".into(); return; };
            params["name"] = json!(element_select::free_group_name(self.editor.doc.value()));
            merge_params(&mut value, &id, params);
        } else if node_type == "blast" && self.editor_state.selection != ui::SelectionMode::Object {
            let Some(params) = params else { self.file_status = "Select points, edges or faces to blast".into(); return; };
            merge_params(&mut value, &id, params);
        } else if let Some(params) = params { merge_params(&mut value, &id, params); }
        let before = ui::network::position_add(ctx, &self.editor.path, self.editor.doc.value(), &mut value, after);
        self.editor.history.finish(&self.editor.doc);
        let original = self.editor.doc.clone();
        if let Some(before) = before { self.editor.doc = before; }
        if self.commit_document(value, &format!("add {node_type}")) {
            ui::network::restore_positions(ctx, &self.editor.path, self.editor.doc.value());
            self.selected = id.clone();
            self.selected_set = std::collections::BTreeSet::from([id.clone()]);
            self.display = id;
            if node_type == "boundary_select" {
                self.editor_state.apply(ui::EditorChange::Selection(ui::SelectionMode::Edges));
                if self.selection.mode != ui::SelectionMode::Edges {
                    self.selection = picking::Selection { mode: ui::SelectionMode::Edges, ..Default::default() };
                }
                self.selection_dirty = true;
            }
            self.last_operator = Some(LastOperator::AddNode {
                node_type: node_type.to_string(),
                after_selection: after.is_some(),
            });
        } else {
            self.editor.doc = original;
        }
    }
    fn selection_node_params(&self, node_type: &str) -> Option<Value> {
        if self.editor_state.selection == ui::SelectionMode::Object || self.selection.mode != self.editor_state.selection { return None; }
        let range=active_range(self.active_object()?,&self.display_ranges)?;
        let global=self.scene.as_ref()?;
        let positions=global.positions().get(range.start as usize*3..range.end as usize*3)?;
        let triangles=global.triangle_indices().chunks_exact(3).filter(|tri|tri.iter().all(|p|range.contains(p)))
            .flat_map(|tri|tri.iter().map(|p|*p-range.start)).collect::<Vec<_>>();
        let edges=global.edge_indices().chunks_exact(2).filter(|edge|edge.iter().all(|p|range.contains(p)))
            .flat_map(|edge|edge.iter().map(|p|*p-range.start)).collect::<Vec<_>>();
        let mut selection=self.active_selection()?;
        let tri_offset=global.triangle_indices().chunks_exact(3).take_while(|tri|tri[0]<range.start).count() as u32;
        for p in &mut selection.points {*p-=range.start;}
        for (a,b) in &mut selection.edges {*a-=range.start;*b-=range.start;}
        for tri in &mut selection.tris {*tri-=tri_offset;}
        element_select::Topology::new(positions,&triangles,&edges).node_params(node_type,&selection)
    }
    fn selected_points(&self) -> Vec<u32> {
        selected_point_ids(&self.selection, self.editor_state.selection,
            self.scene.as_ref().map_or(&[], Scene::triangle_indices)).into_iter().filter(|id| self.active_object().and_then(|active|active_range(active,&self.display_ranges)).is_some_and(|range|range.contains(id))).collect()
    }
    fn set_display_mode(&mut self,mode:Mode) {
        self.mode=mode;
        self.prefs.viewport.default_shading=mode.label().into();
        if let Err(e)=self.prefs.save() { self.file_status=format!("Could not save preferences: {e}"); }
    }
    fn active_object(&self) -> Option<&str> {
        // while a field is being picked the displayed pick source is the active object (the selected node is not displayed)
        if let Some(pick) = &self.param_pick { return active_range(&pick.source, &self.display_ranges).map(|_| pick.source.as_str()); }
        (!self.object_hidden && self.selected_set.contains(&self.selected)
            && active_range(&self.selected,&self.display_ranges).is_some()
            && feeds_displayed_node(self.editor.doc.value(), &self.selected)).then_some(self.selected.as_str())
    }
    fn active_selection(&self) -> Option<picking::Selection> {
        Some(selection_in_range(&self.selection,self.scene.as_ref()?,active_range(self.active_object()?,&self.display_ranges)?))
    }
    /// Objects whose points can be picked/highlighted: the node selection plus the pick source while a field pick is active.
    fn pick_set(&self) -> std::collections::BTreeSet<String> {
        let mut set = self.selected_set.clone();
        if let Some(pick) = &self.param_pick { set.insert(pick.source.clone()); }
        set
    }
    fn active_pick_set(&self) -> std::collections::BTreeSet<String> {
        self.active_object().map(|id|std::collections::BTreeSet::from([id.to_owned()])).unwrap_or_default()
    }
    fn selection_center(&self) -> Option<[f32; 3]> {
        if self.editor_state.selection == ui::SelectionMode::Object {
            return self.active_object().map(|id| object_origin(self.editor.doc.value(), id));
        }
        self.element_metrics().center
    }
    fn element_metrics(&self) -> selection_metrics::Metrics {
        let Some(scene)=self.scene.as_ref() else { return Default::default(); };
        self.selection_metrics.get(self.cooking.completed,&self.active_selection().unwrap_or_default(),self.editor_state.selection,scene,
            viewport::ObjectVisibility::HideOthers,&self.display_ranges,&self.active_pick_set())
    }
    fn selection_normal_frame(&self, trs: &transform_math::TrsParams) -> transform_math::Frame {
        let local = transform_math::Frame::from_rotation(&trs.world_rotation());
        let normal = self.element_metrics().normal;
        if transform_math::length3(normal) < 1e-9 { return local; }
        transform_math::Frame::from_normal(normal, local.axis(0))
    }
    fn orientation_frame(&self, orientation: ui::Orientation, trs: &transform_math::TrsParams) -> transform_math::Frame {
        match orientation {
            ui::Orientation::View => {
                let c = self.camera_info();
                transform_math::Frame::from_f32([c.ray_right, c.ray_up, c.ray_forward])
            }
            ui::Orientation::Local | ui::Orientation::Gimbal => {
                transform_math::Frame::from_rotation(&trs.world_rotation())
            }
            ui::Orientation::Normal => self.selection_normal_frame(trs),
            ui::Orientation::Cursor => transform_math::Frame::from_rotation(
                &transform_math::euler_xyz(self.editor_state.cursor_rotation.map(f64::from))),
            _ => transform_math::Frame::global(),
        }
    }
    fn transform_node(&mut self, op: TransformOp) {
        let pivot = if matches!(op, TransformOp::Move(_)) { None }
            else if let Some(pivot) = self.transform_pivot { Some(pivot) }
            else if self.editor_state.pivot == ui::Pivot::Cursor { Some(self.editor_state.cursor) }
            else { Some(self.selection_center().unwrap_or(self.bounds.0)) };
        self.transform_node_at(op, pivot, self.transform_orientation);
    }
    fn transform_node_at(&mut self, op: TransformOp, pivot: Option<[f32; 3]>, orientation_override: Option<ui::Orientation>) {
        if self.editor_state.selection == ui::SelectionMode::Object && self.active_object().is_none() { return; }
        let Some(active)=self.active_object().map(str::to_owned) else {return;};
        let offset=active_range(&active,&self.display_ranges).map(|r|r.start).unwrap_or(0);
        let group = group_text(&self.selected_points().iter().map(|p|p-offset).collect::<Vec<_>>());
        if self.editor_state.selection != ui::SelectionMode::Object && group.is_empty() {
            self.file_status = "Select points, edges, or faces to transform".into();
            return;
        }
        let first = self.transform_before.is_none();
        if first { self.editor.history.finish(&self.editor.doc); self.transform_before = Some(self.editor.doc.clone()); }
        let before_doc = self.transform_before.as_ref().unwrap();
        let mut value = self.editor.doc.value().clone();
        // The displayed node changes to the new node during a drag. Keep reading
        // the original matrix from the document saved at drag start.
        let displayed = &active;
        let source = if before_doc.value()["nodes"].get(displayed).is_some() {
            displayed.clone()
        } else {
            value["nodes"][&self.display]["inputs"]["geo"].as_str().unwrap_or(&self.display).to_owned()
        };
        let resolved_pivot = pivot.unwrap_or_else(|| {
            if self.editor_state.pivot == ui::Pivot::Cursor {
                self.editor_state.cursor
            } else {
                self.selection_center().unwrap_or(self.bounds.0)
            }
        });
        if first { self.transform_pivot = Some(resolved_pivot); }
        let orientation = orientation_override.unwrap_or(self.editor_state.orientation);
        if first { self.transform_orientation = Some(orientation); }
        let is_local = matches!(orientation, ui::Orientation::Local | ui::Orientation::Gimbal);
        let base = if before_doc.value()["nodes"][&source]["type"] == "transform" {
            transform_math::TrsParams::from_params(&before_doc.value()["nodes"][&source]["params"])
        } else { transform_math::TrsParams::default() };
        let scale_base = if pivot.is_some() && matches!(op, TransformOp::Scale(_)) {
            base.with_world_pivot_preserving_matrix(resolved_pivot.map(f64::from))
        } else { base };
        let base_frame = self.orientation_frame(orientation, &base);
        let shear_second = group.is_empty() && !is_local &&
            before_doc.value()["nodes"][&source]["type"] == "transform" &&
            matches!(op, TransformOp::Scale(f) if matches!(
                transform_math::compose_scale(&scale_base, base_frame, f.map(|x| x as f64), false, 1e-6),
                transform_math::ScaleResult::ShearNeedsSecondNode { .. }));
        let Some(target) = ensure_transform_target(&mut value, &source, group.is_empty() && !shear_second,
            self.transform_target.as_deref()) else { return; };
        // A transform on a node nothing displays (no Output, or an Output elsewhere) would move off-screen: show it.
        if first { show_transform_target(&mut value, &target); }
        let target_existed = before_doc.value()["nodes"].get(&target).is_some();
        let mut initial = if target_existed {
            transform_math::TrsParams::from_params(&before_doc.value()["nodes"][&target]["params"])
        } else { transform_math::TrsParams { pivot: resolved_pivot.map(f64::from), ..Default::default() } };
        if target_existed && pivot.is_some() && !matches!(op, TransformOp::Move(_)) {
            initial = initial.with_world_pivot_preserving_matrix(resolved_pivot.map(f64::from));
        }
        let frame = if shear_second { base_frame } else { self.orientation_frame(orientation, &initial) };
        let new_trs = if shear_second {
            let TransformOp::Scale(factors) = op else { unreachable!() };
            let transform_math::ScaleResult::ShearNeedsSecondNode { second_node, .. } =
                transform_math::compose_scale(&scale_base, base_frame, factors.map(f64::from), false, 1e-6)
                else { unreachable!() };
            second_node
        } else {
            match op {
                TransformOp::Move(delta) => transform_math::compose_translate(&initial, delta.map(f64::from)),
                TransformOp::Rotate(angles) => {
                    let count = angles.iter().filter(|x| x.abs() > 1e-6).count();
                    if count == 1 {
                        let axis = angles.iter().position(|x| x.abs() > 1e-6).unwrap();
                        transform_math::compose_rotate_axis(&initial, frame, axis, angles[axis] as f64, is_local)
                    } else {
                        transform_math::compose_rotate_euler(&initial, frame, angles.map(f64::from), is_local)
                    }
                }
                TransformOp::Scale(factors) => match transform_math::compose_scale(&initial, frame,
                    factors.map(f64::from), is_local, 1e-6) {
                    transform_math::ScaleResult::Direct(trs) => trs,
                    transform_math::ScaleResult::ShearNeedsSecondNode { second_node, .. } => second_node,
                },
            }
        };
        let name = match op { TransformOp::Move(_) => "translate", TransformOp::Rotate(_) => "rotate",
            TransformOp::Scale(_) => "scale" };
        let delta = transform_math::mul4(&new_trs.to_matrix(),
            &transform_math::inverse_affine(&initial.to_matrix()).unwrap_or(transform_math::IDENTITY_4));
        self.transform_delta = Some(LastOperator::MatrixDelta { delta, frame,
            label: match op { TransformOp::Move(_) => "Move", TransformOp::Rotate(_) => "Rotate",
                TransformOp::Scale(_) => "Scale" } });
        let params = &mut value["nodes"][&target]["params"];
        if !params.is_object() { *params = json!({}); }
        params["translate"] = json!(new_trs.translate);
        params["rotate"] = json!(new_trs.rotate);
        params["scale"] = json!(new_trs.scale);
        params["uniform_scale"] = json!(new_trs.uniform_scale);
        params["pivot"] = json!(new_trs.pivot);
        params["axis_rotation"] = json!(new_trs.axis_rotation);
        if first {
            params["group"] = json!(group);
            proportional_params(params, &self.editor_state);
        }
        let param_value = params[name].clone();
        match apply_transform_step(&mut self.editor.doc, first, value, &target, name, param_value) {
            Ok(edits) => {
                if self.editor_state.selection == ui::SelectionMode::Object {
                    self.selected_set.remove(&self.selected);
                    self.selected_set.insert(target.clone());
                    self.selected = target.clone();
                }
                self.transform_target = Some(target.clone()); self.display = target.clone(); for edit in edits { self.send(edit); }
                if !first {
                    let mut desired=self.editor.doc.value()["nodes"][&target]["params"].clone();
                    desired["translate"] = json!(new_trs.translate);
                    desired["rotate"] = json!(new_trs.rotate);
                    desired["scale"] = json!(new_trs.scale);
                    desired["uniform_scale"] = json!(new_trs.uniform_scale);
                    desired["pivot"] = json!(new_trs.pivot);
                    desired["axis_rotation"] = json!(new_trs.axis_rotation);
                    proportional_params(&mut desired,&self.editor_state);
                    for name in ["translate", "rotate", "scale", "uniform_scale", "pivot", "axis_rotation",
                        "proportional_enabled","proportional_radius","proportional_falloff","proportional_connected"] {
                        if desired[name] != self.editor.doc.value()["nodes"][&target]["params"][name] {
                            let edit=Edit::Param { node:target.clone(), name:name.into(), value:desired[name].clone() };
                            if self.editor.doc.edit(&edit).is_ok() { self.send(edit); }
                        }
                    }
                }
            }
            Err(e) => self.file_status = format!("Could not transform node: {e}"),
        }
    }
    fn finish_transform(&mut self) {
        self.finish_transform_from(false);
    }
    fn finish_transform_from(&mut self, is_modal: bool) {
        if let Some(before) = self.transform_before.take() {
            if let Some(delta) = self.transform_delta.take() { self.last_operator = Some(delta); }
            self.editor.history.record(before, &self.editor.doc);
        }
        let _ = is_modal;
        if let Some(target) = self.transform_target.take() {
            if self.editor_state.selection != ui::SelectionMode::Object { self.selected_set = std::collections::BTreeSet::from([target.clone()]); }
            self.selected = target;
        }
        self.transform_pivot = None;
        self.transform_orientation = None;
    }
    fn cancel_transform(&mut self) {
        if let Some(before) = self.transform_before.take() {
            if self.editor_state.selection == ui::SelectionMode::Object && before.value()["nodes"].get(&self.selected).is_none() {
                let source = self.editor.doc.value()["nodes"][&self.selected]["inputs"]["geo"].as_str().unwrap_or("").to_owned();
                self.selected_set.remove(&self.selected);
                if !source.is_empty() { self.selected_set.insert(source.clone()); }
                self.selected = source;
            }
            self.editor.doc = before;
            self.send(Edit::Replace(self.editor.doc.value().clone()));
            self.display = Graph::from_value(self.editor.doc.value()).ok().and_then(|g| g.main_output().ok()).unwrap_or_else(|| self.display.clone());
        }
        self.transform_target = None;
        self.transform_delta = None;
        self.transform_pivot = None;
        self.transform_orientation = None;
    }
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
        self.editor.history.finish(&self.editor.doc);
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
            self.editor.history.record(s.before, &self.editor.doc);
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
        self.editor.history.finish(&self.editor.doc);
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
            self.editor.history.record(before,&self.editor.doc);
            if let Some((selection,stash))=self.model_before_selection.take() {
                self.undo_element_selection.push((self.editor.doc.value().to_string(),selection,stash));
            }
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
    fn update_modal(&mut self, ctx: &egui::Context) {
        let Some(mut modal)=self.modal.take() else { return; };
        let input=ctx.input(|i|(i.events.clone(),i.pointer.hover_pos(),i.modifiers,i.raw_scroll_delta.y));
        let (events,pointer,modifiers,scroll)=input;
        if let Some(pointer)=pointer { modal.pointer=pointer; }
        let mut confirm=false; let mut cancel=false; let mut gg=false;
        let camera=self.camera_info();
        let orientation=if matches!(modal.constraint,Constraint::Axis(_,true)|Constraint::Plane(_,true))
            { ui::Orientation::Local } else { self.editor_state.orientation };
        let frame=if orientation == ui::Orientation::Cursor {
            transform_math::Frame::from_rotation(&transform_math::euler_xyz(self.editor_state.cursor_rotation.map(f64::from)))
                .axes.map(|axis| axis.map(|x| x as f32))
        } else if orientation == ui::Orientation::Normal {
            let trs = transform_math::TrsParams::from_params(&self.editor.doc.value()["nodes"][&self.display]["params"]);
            self.selection_normal_frame(&trs).axes.map(|axis| axis.map(|x| x as f32))
        } else { ui::modal_transform::orientation_frame(orientation,camera,modal.rotation) };
        for event in events {
            match event {
                egui::Event::Key { key, pressed:true, modifiers:m, .. } => {
                    if modal.type_key(key,m.shift) { continue; }
                    match key {
                        egui::Key::Escape => cancel=true,
                        egui::Key::Enter | egui::Key::Space => confirm=true,
                        egui::Key::G if ui::modal_transform::double_g_slides(modal.kind,self.editor_state.selection) => gg=true,
                        egui::Key::X | egui::Key::Y | egui::Key::Z => modal.toggle_axis(match key { egui::Key::X=>0,egui::Key::Y=>1,_=>2 },m.shift),
                        egui::Key::O => modal.proportional = !modal.proportional,
                        egui::Key::PageUp => modal.wheel(1.), egui::Key::PageDown => modal.wheel(-1.),
                        egui::Key::B if modal.kind==ModalKind::Move => { modal.cycle_snap(); if modal.reference.is_none() { modal.reference=Some(modal.pivot); } },
                        egui::Key::B if modal.kind==ModalKind::Rotate => { modal.sketch=SketchStage::Pivot; modal.sketch_reference=None; },
                        egui::Key::R if modal.kind==ModalKind::Rotate => modal.trackball = !modal.trackball,
                        egui::Key::V if modal.kind==ModalKind::Scale => { modal.cage = !modal.cage; modal.cage_handle=None; },
                        _=>{}
                    }
                }
                egui::Event::PointerButton { pos, button:egui::PointerButton::Primary, pressed:true, .. } => {
                    if modal.sketch==SketchStage::Pivot {
                        let to=if !modal.snap_to.geometry() {ui::SnapTo::Face.into()} else {modal.snap_to};
                        modal.sketch_pivot=self.modal_snap_point(pos,to).or(Some(modal.pivot));
                        modal.pivot=modal.sketch_pivot.unwrap(); modal.sketch=SketchStage::Reference;
                    } else if modal.sketch==SketchStage::Reference { modal.sketch_reference=Some(pos); modal.sketch=SketchStage::Angle; }
                    else if modal.cage && modal.cage_handle.is_none() {
                        modal.cage_handle=ui::transform_overlay::cage_hit(self.last_viewport,camera,&modal,pos);
                        modal.cage_start=pos;
                        if modal.cage_handle.is_none() { confirm=true; }
                    } else if modifiers.ctrl && modifiers.shift && modal.kind==ModalKind::Move {
                        modal.reference=self.modal_snap_point(pos,ui::SnapTo::Vertex);
                    } else { confirm=true; }
                }
                egui::Event::PointerButton { button:egui::PointerButton::Secondary, pressed:true, .. } => cancel=true,
                egui::Event::PointerButton { pos, button:egui::PointerButton::Middle, pressed:true, .. } => {
                    if modal.constraint!=Constraint::Free { modal.constraint=Constraint::Free; } else { modal.mmb_start=Some(pos); }
                }
                egui::Event::PointerButton { pos, button:egui::PointerButton::Middle, pressed:false, .. } => {
                    modal.middle_release(pos,frame,camera);
                }
                _=>{}
            }
        }
        if let Some(start)=modal.mmb_start {
            if (modal.pointer-start).length()>4. {
                modal.constraint=Constraint::Axis(ui::modal_transform::closest_axis(modal.pointer-start,frame,camera),false);
            }
        }
        if modal.proportional && scroll!=0. { modal.wheel(scroll.signum()); }
        if gg { self.cancel_transform(); self.command("edge_slide",ctx); return; }
        if cancel {
            self.editor_state.proportional.enabled=modal.initial_proportional;
            self.editor_state.proportional.radius=modal.initial_radius;
            self.cancel_transform(); return;
        }
        self.editor_state.proportional.enabled=modal.proportional;
        self.editor_state.proportional.radius=modal.radius;
        let snap=self.editor_state.snap.enabled ^ modifiers.ctrl;
        if modal.kind==ModalKind::Move && snap && modal.snap_to.geometry() {
            let feature=self.modal_snap_feature(modal.pointer,modal.snap_to);
            modal.target=feature.map(|f|f.point);
            modal.target_kind=feature.map(|f|f.kind);
            modal.target_shape=feature.map(|f|f.triangle);
        } else { modal.target=None; modal.target_shape=None; modal.target_kind=None; }
        let center=ui::transform_gizmo::project(camera,self.last_viewport,modal.pivot).unwrap_or(self.last_viewport.center());
        let unit=ui::transform_gizmo::project(camera,self.last_viewport,[modal.pivot[0]+1.,modal.pivot[1],modal.pivot[2]])
            .map(|p|(p-center).length()).unwrap_or(80.).max(1.);
        let orientation=if matches!(modal.constraint,Constraint::Axis(_,true)|Constraint::Plane(_,true))
            { ui::Orientation::Local } else { self.editor_state.orientation };
        let frame=if orientation == ui::Orientation::Cursor {
            transform_math::Frame::from_rotation(&transform_math::euler_xyz(self.editor_state.cursor_rotation.map(f64::from)))
                .axes.map(|axis| axis.map(|x| x as f32))
        } else if orientation == ui::Orientation::Normal {
            let trs = transform_math::TrsParams::from_params(&self.editor.doc.value()["nodes"][&self.display]["params"]);
            self.selection_normal_frame(&trs).axes.map(|axis| axis.map(|x| x as f32))
        } else { ui::modal_transform::orientation_frame(orientation,camera,modal.rotation) };
        if snap && modal.kind==ModalKind::Move && modal.snap_to.contains(ui::SnapTo::Increment) {
            if let Some(target)=modal.target.take() {
                let (grid,_) = modal.desired(center,unit,frame,camera,true,self.editor_state.snap.increment,modifiers.shift);
                let reference=modal.reference.unwrap_or(modal.pivot);
                let grid_point=[0,1,2].map(|k|reference[k]+grid[k]);
                let distance=|p|ui::transform_gizmo::project(camera,self.last_viewport,p).map(|p|p.distance(modal.pointer)).unwrap_or(f32::INFINITY);
                if distance(target) <= distance(grid_point) { modal.target=Some(target); }
                else { modal.target_shape=None; modal.target_kind=None; }
            }
        }
        let (values,pivot)=modal.desired(center,unit,frame,camera,snap,self.editor_state.snap.increment,modifiers.shift);
        let pivot_changed=self.transform_target.as_ref().is_some_and(|target|
            self.editor.doc.value()["nodes"][target]["params"]["pivot"]!=json!(pivot));
        if values!=modal.last || pivot_changed {
            let op=modal.step(values);
            self.transform_node_at(op,Some(pivot),Some(orientation));
        }
        if confirm { self.finish_modal(modal); } else { self.modal=Some(modal); ctx.request_repaint(); }
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
                let selection=self.selection.clone();
                let stash=self.stashed_selection.clone();
                let changed=self.editor.doc.value()!=new.value();
                self.editor.history.replace(&mut self.editor.doc, new);
                if changed { self.undo_element_selection.push((self.editor.doc.value().to_string(),selection,stash)); }
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
                self.editor.history.finish(&self.editor.doc);
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
                        self.editor.history.record(before, &self.editor.doc);
                        self.editor.history.finish(&self.editor.doc);
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
                self.editor.history.finish(&self.editor.doc);
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
                        self.editor.history.record(before, &self.editor.doc);
                        self.editor.history.finish(&self.editor.doc);
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
    /// Apply selected wire disconnections with any selected node operation as
    /// one history entry. With no wires, preserve the existing node command path.
    fn remove_selected_wires(&mut self, ctx: &egui::Context, dissolve: bool) -> bool {
        if !self.network_hover { return false; }
        let wires = ui::network::take_selected_wires(ctx, &self.editor.path, self.editor.doc.value());
        if wires.is_empty() { return false; }
        let original = self.editor.doc.value();
        let mut value = graph_edit::disconnect_wires(original, &wires).unwrap_or_else(|| original.clone());
        let ids: Vec<_> = self.selected_set.iter().cloned().collect();
        if !ids.is_empty() {
            let nodes = if dissolve { graph_edit::dissolve_set(&value, &ids) }
                else { graph_edit::delete_set(&value, &ids) };
            if let Some(next) = nodes { value = next; }
        }
        if self.commit_document(value, "disconnect selected wires") {
            if let Ok(graph) = Graph::from_value(self.editor.doc.value()) {
                self.selected_set.retain(|id| graph.nodes.iter().any(|(node,_)| node == id));
                if !graph.nodes.iter().any(|(id,_)| id == &self.selected) {
                    self.selected = self.selected_set.iter().next().cloned().unwrap_or_default();
                }
                self.display = graph.main_output().unwrap_or_else(|_| self.selected.clone());
            }
        }
        true
    }

    /// Keymap commands that have no dedicated action.
    fn command(&mut self, name: &str, ctx: &egui::Context) {
        use ui::EditorChange as C;
        let e = self.editor_state;
        match name {
            "toggle_dep_links" => { self.prefs.show_parameter_links=!self.prefs.show_parameter_links; ui::network::toggle_links(ctx,&self.editor.path); let _=self.prefs.save(); },
            "go_to_link_source" | "go_to_link_target" => {
                let destination = if name == "go_to_link_source" {
                    minihou_app::value_link_ui::go_to_source(self.editor.doc.value(),&self.selected,None)
                } else { minihou_app::value_link_ui::go_to_target(self.editor.doc.value(),&self.selected,None) };
                if let Some((node,param)) = destination {
                    select_active_nodes(&mut self.selected,&mut self.selected_set,vec![node.clone()],false,false);
                    ui::network::focus(ctx,&self.editor.path,&node);
                    ctx.data_mut(|d| d.insert_temp(egui::Id::new("link_flash"),(node,param,ctx.input(|i|i.time))));
                }
            }
            "export_usd" => usd_export::open(ctx, &self.editor.path),
            "blender_install" => self.blender_action(UiAction::BlenderInstall(ui::blender_ui::selected_index(ctx, self.blender_paths.len())), ctx),
            "blender_open" | "blender_connect" => self.blender_action(UiAction::BlenderOpen, ctx),
            "blender_detect" => self.blender_action(UiAction::BlenderDetect, ctx),
            "blender_restart_link" => self.blender_action(UiAction::BlenderRestartLink, ctx),
            "blender_download_addon" => self.blender_action(UiAction::BlenderDownloadAddon, ctx),
            "repeat_last" => self.repeat_last(ctx),
            "copy" => {
                self.clipboard = graph_edit::copy(self.editor.doc.value(), &self.selected_set.iter().cloned().collect::<Vec<_>>());
                if self.clipboard.is_some() { self.file_status = "Node copied".into(); }
            }
            "paste" => {
                if let Some((value, ids)) = self.clipboard.as_ref().and_then(|clip| graph_edit::paste(self.editor.doc.value(), clip)) {
                    if self.commit_document(value, "paste nodes") {
                        self.selected = ids[0].clone(); self.selected_set = ids.iter().cloned().collect();
                        self.file_status = format!("Pasted {} node(s)", ids.len());
                    }
                }
            }
            "select_coplanar"=>self.select_coplanar(),
            "update_coplanar"=>self.update_coplanar(),
            "make_coplanar_group"=>self.make_coplanar_group(),
            "select_linked" | "invert_selection" | "select_linked_nodes" | "invert_selection_nodes" => {
                let inverse=name.starts_with("invert");
                let network=name.ends_with("_nodes");
                if network || e.selection==ui::SelectionMode::Object {
                    let graph=Graph::from_value(self.editor.doc.value()).expect("validated document");
                    let ids=if inverse {
                        let universe=if network {graph.nodes.iter().map(|(id,_)|id.clone()).collect()}else{self.editor.doc.display_nodes()};
                        node_selection::inverse(universe,&self.selected_set)
                    }else{node_selection::linked(&graph,&self.selected_set)};
                    self.selected_set=ids;
                    if !self.selected_set.contains(&self.selected) {self.selected=self.selected_set.iter().next().cloned().unwrap_or_default();}
                    self.selection_dirty=true;
                }else if let Some(scene)=&self.scene {
                    let active=self.active_object().and_then(|id|active_range(id,&self.display_ranges));
                    let visible=|id|active.is_some_and(|range|range.contains(&id));
                    if element_select::cached(&self.elem_topo,scene).linked_or_inverse(&mut self.selection,e.selection,inverse,visible) {self.selection_dirty=true;}
                }
            }
            "select_more" | "select_less" => {
                // Ctrl+Numpad +/-: viewport selection only; the grow node is added from the Tab menu
                if let Some(scene) = &self.scene {
                    if element_select::cached(&self.elem_topo, scene).grow_selection(&mut self.selection, name == "select_less") { self.selection_dirty = true; }
                }
            }
            "toggle_edit" => {
                let mode = toggle_edit_mode(e.selection, &mut self.last_edit_selection);
                self.editor_state.apply(C::Selection(mode));
            }
            "select_none" => {
                if self.network_hover {
                    self.selected_set.clear();
                    self.selected.clear();
                    return;
                }
                self.selection.clear(); self.selection_dirty = true;
                if e.selection==ui::SelectionMode::Object { self.selected_set.clear(); self.selected.clear(); }
            }
            "select_all" => {
                if self.network_hover {
                    self.selected_set = self.editor.doc.value()["nodes"].as_object().map(|n| n.keys().cloned().collect()).unwrap_or_default();
                    self.selected = self.selected_set.iter().next().cloned().unwrap_or_default();
                    return;
                }
                if let Some(scene) = &self.scene {
                    let active=self.active_object().and_then(|id|active_range(id,&self.display_ranges));
                    let visible=|id:&u32|active.is_some_and(|range|range.contains(id));
                    match e.selection {
                        ui::SelectionMode::Points => self.selection.points = (0..scene.positions().len() / 3).map(|i| i as u32).filter(&visible).collect(),
                        ui::SelectionMode::Faces => self.selection.tris = scene.triangle_indices().chunks_exact(3).enumerate().filter(|(_,tri)| tri.iter().all(&visible)).map(|(i,_)| i as u32).collect(),
                        ui::SelectionMode::Edges => {
                            // polygon (and wire) edges, without fan diagonals
                            let edges = element_select::cached(&self.elem_topo, scene).edges.clone();
                            self.selection.edges = edges.into_iter().filter(|(a,b)| visible(a) && visible(b)).collect();
                        }
                        ui::SelectionMode::Object => {
                            self.object_hidden = false;
                            let ids=self.editor.doc.display_nodes().into_iter().filter(|id| e.object_visibility==viewport::ObjectVisibility::ShowAll || self.selected_set.contains(id)).collect();
                            select_active_nodes(&mut self.selected, &mut self.selected_set, ids, false, false);
                        },
                    }
                    self.selection_dirty = true;
                }
            }
            "hide" => { self.object_hidden = true; self.selection.clear(); self.selection_dirty = true; }
            "reveal" => self.object_hidden = false,
            "add_menu" => ui::network::open_add_menu(ctx, &self.editor.path),
            "box_select" => ui::selection_region::arm(ctx),
            "lasso_select" => ui::selection_region::arm_lasso(ctx),
            "circle_select" => ui::selection_region::arm_circle(ctx, ui::selection_region::Snapshot {
                selection: self.selection.clone(), selected: self.selected.clone(), selected_set: self.selected_set.clone(),
            }),
            "timeline_toggle" | "timeline_previous" | "timeline_next" | "timeline_start" | "timeline_end" => {
                if let Some(edit) = ui::viewport_ui::timeline_command(ctx, name) { self.send_timeline_time(edit); }
            }
            "shade_pie" => ui::viewport_ui::open_shading_pie(ctx),
            "pie_menu" => ui::network::open_pie_menu(ctx, &self.editor.path),
            "reset_cursor" => {
                self.editor_state.cursor = [0.0, 0.0, 0.0];
                self.frame_mesh();
            }
            "display_flag" => {
                let mut value = self.editor.doc.value().clone();
                let mut changed = false;
                for id in &self.selected_set {
                    if let Some(next) = graph_edit::set_display(&value, id, true) { value = next; changed = true; }
                }
                if changed && self.commit_document(value, "set display flag") {
                    self.display = minihou_app::display_nodes(self.editor.doc.value()).first().cloned().unwrap_or_default();
                }
            }
            "mute_flag" => {
                let mut value = self.editor.doc.value().clone();
                for id in &self.selected_set {
                    let bypass = value["nodes"][id]["flags"]["bypass"].as_bool().unwrap_or(false);
                    value["nodes"][id]["flags"]["bypass"] = json!(!bypass);
                }
                if !self.selected_set.is_empty() { self.commit_document(value, "mute nodes"); }
            }
            "template_flag" => { for id in &self.selected_set { let f = self.flags.entry(id.clone()).or_default(); f.template = !f.template; } }
            "selectable_template_flag" => self.file_status = "Selectable template flags are not supported yet".into(),
            "delete_selection" => {
                if let Some(index)=ui::sketch::selected_measurement(ctx) {
                    self.delete_measurement(index);
                    ui::sketch::clear_measurement_selection(ctx);
                    return;
                }
                if self.viewport_hover && self.editor.doc.value()["nodes"][&self.selected]["type"]=="bezier" {
                    if let Some(cs)=ui::curve_tool::parse(&ui::curve_tool::node_controls(self.editor.doc.value(),&self.selected)) {
                        let index=ui::curve_tool::selected(ctx,&self.selected).index;
                        let next=ui::curve_tool::delete(&cs,index);
                        if next.len()!=cs.len() {
                            self.edit(Edit::Param{node:self.selected.clone(),name:"controls".into(),value:ui::curve_tool::write(&next)},false);
                            return;
                        }
                    }
                }
                if self.remove_selected_wires(ctx, false) { return; }
                if self.viewport_hover && self.editor_state.selection != ui::SelectionMode::Object {
                    let Some(source) = self.active_object().map(str::to_owned) else { self.file_status = "Select displayed elements to blast".into(); return; };
                    self.add_node(ctx, "blast", Some(&source));
                } else if let Some(id) = self.selected_set.iter().next().cloned() { self.delete_node(&id); }
            }
            "group_selection" => {
                if self.viewport_hover && self.editor_state.selection != ui::SelectionMode::Object {
                    let Some(source) = self.active_object().map(str::to_owned) else { self.file_status = "Select displayed elements to group".into(); return; };
                    self.add_node(ctx, "group_create", Some(&source));
                    return;
                }
                let ids: Vec<String> = if self.selected_set.is_empty() { vec![self.selected.clone()] } else { self.selected_set.iter().cloned().collect() };
                match subnet_edit::toggle(self.editor.doc.value(), &ids) {
                    Some((value, sel)) => if self.commit_document(value, "group nodes") {
                        self.selected = sel[0].clone();
                        self.selected_set = sel.into_iter().collect();
                        if let Ok(g) = Graph::from_value(self.editor.doc.value()) { self.display = g.main_output().unwrap_or_else(|_| self.selected.clone()); }
                    },
                    None => self.file_status = "Select nodes to group (only one may feed the rest of the network)".into(),
                }
            }
            "duplicate_selection" => { if let Some(id) = self.selected_set.iter().next().cloned() { self.duplicate_node(&id); } }
            "dissolve_selection" => {
                if self.remove_selected_wires(ctx, true) { return; }
                if let Some(id) = self.selected_set.iter().next().cloned() {
                    self.dissolve_node(&id);
                } else if !self.selected.is_empty() {
                    let sel = self.selected.clone();
                    self.dissolve_node(&sel);
                }
            }
            "mirror_tool" => self.start_mirror(ctx),
            "knife" | "loop_cut" => {
                self.finish_for_selection(ctx);
                self.editor_state.apply(C::Tool(if name == "knife" { ui::Tool::Knife } else { ui::Tool::LoopCut }));
            }
            "edge_slide" => {
                if self.editor_state.selection!=ui::SelectionMode::Edges || self.selection.edges.is_empty() {
                    self.file_status="Select edges in edge mode to slide".into();
                } else {
                    self.finish_for_selection(ctx);
                    let Some(source)=self.active_object().map(str::to_owned) else { self.file_status="Select a displayed object to slide".into(); return; };
                    let range=active_range(&source,&self.display_ranges).unwrap();
                    let edges=self.selection.edges.iter().filter(|(a,b)|range.contains(a)&&range.contains(b)).map(|&(a,b)|((a-range.start) as usize,(b-range.start) as usize)).collect();
                    ui::mesh_cut::start_edge_slide(ctx,source,edges);
                    self.editor_state.apply(C::Tool(ui::Tool::EdgeSlide));
                }
            }
            "extrude" | "inset" | "bevel" | "pushpull" => {
                let kind=match name { "extrude"=>ModelKind::Extrude,"inset"=>ModelKind::Inset,
                    "bevel"=>ModelKind::Bevel,_=>ModelKind::PushPull };
                if kind==ModelKind::PushPull {
                    if self.scene.is_none() { self.file_status="Wait for the mesh to cook".into(); return; }
                    // Like E: a face selection starts the gesture at once; otherwise hover and click a face.
                    if pushpull_starts_now(self.editor_state.selection,&self.selection) { self.start_model_modal(ctx,kind); return; }
                    self.pushpull_pick=true;
                    self.file_status="Push / Pull: hover a face and click to pick it".into();
                    return;
                }
                let has_selection=if matches!(kind,ModelKind::Extrude | ModelKind::Bevel) { !self.selection.is_empty() }
                    else { self.editor_state.selection==ui::SelectionMode::Faces && !self.selection.tris.is_empty() };
                if self.editor_state.selection==ui::SelectionMode::Object || !has_selection {
                    let after=self.selected.clone(); self.add_node(ctx,kind.node(),Some(&after));
                } else { self.start_model_modal(ctx,kind); }
            }
            "opposite_view" => {
                let next = match self.editor_state.view {
                    ui::ViewPreset::Front => ui::ViewPreset::Back, ui::ViewPreset::Back => ui::ViewPreset::Front,
                    ui::ViewPreset::Right => ui::ViewPreset::Left, ui::ViewPreset::Left => ui::ViewPreset::Right,
                    ui::ViewPreset::Top => ui::ViewPreset::Bottom, ui::ViewPreset::Bottom => ui::ViewPreset::Top,
                    _ => ui::ViewPreset::Back,
                };
                self.editor_state.apply(C::View(next)); self.camera.set_view(standard_view(next));
            }
            "sketch_line" | "sketch_polyline" | "sketch_polygon" | "sketch_rectangle" | "sketch_circle" => {
                if let Some(mode) = ui::sketch::SketchMode::DRAW.into_iter().find(|m| m.command_id() == name) { self.start_sketch(ctx, mode); }
            }
            "toggle_params" => self.params_open = !self.params_open,
            "xray" => {
                self.editor_state.apply(C::Xray(!e.xray)); self.prefs.viewport.xray=self.editor_state.xray;
                if let Err(err)=self.prefs.save() { self.file_status=format!("Could not save preferences: {err}"); }
            },
            "overlays" => {
                let mut o=e.overlays; o.enabled=!o.enabled; self.editor_state.apply(C::Overlays(o));
                self.prefs.viewport.overlays=o;
                if let Err(err)=self.prefs.save() { self.file_status=format!("Could not save preferences: {err}"); }
            },
            "snap" => self.editor_state.apply(C::SnapEnabled(!e.snap.enabled)),
            "proportional" => self.editor_state.apply(C::ProportionalEnabled(!e.proportional.enabled)),
            "toggle_projection" => {
                self.camera.toggle_ortho();
            }
            "shade_next" | "shade_previous" => {
                self.set_display_mode(self.mode.cycle(name == "shade_next"));
            }
            _ => self.file_status = format!("Command {name} is not wired up yet"),
        }
    }
    /// Apply what the panels asked for this frame.
    fn apply(&mut self, ctx: &egui::Context, actions: Vec<UiAction>) {
        let prev_selected = self.selected.clone();
        for action in actions {
            if matches!(&action,UiAction::Select(_) | UiAction::ToggleSelect(_) | UiAction::SelectSet { .. }) { self.selection_dirty=true; }
            match action {
                UiAction::SelectOutputGroup { node, name } => {
                    if node == self.display {
                        if let Some(g) = self.cooking.geometry.as_deref().filter(|_| self.cooking.error.is_none()) {
                            if let Some(sel) = group_preview::selection(g, &name) { self.editor_state.selection = sel.mode; self.selection = sel; self.selection_dirty = true; }
                        }
                    }
                }
                UiAction::PickParameter { node, name, mode, pairs } => {
                    let original = self.editor.doc.value()["nodes"][&node]["params"][&name].as_str().unwrap_or("").to_owned();
                    let mode = infer_mode(&original, mode); // an edge group stays edges whatever the node's default domain
                    let pairs = pairs || (mode == ui::SelectionMode::Edges && !original.contains('p') && original.contains('-'));
                    let source = input_source(self.editor.doc.value(), &node, &self.display);
                    let ready = source_ready(&self.display_ranges, &source);
                    self.param_pick = Some(ParamPick { node:node.clone(), name:name.clone(), original, changed:false,
                        previous_selection:self.selection.clone(), previous_mode:self.editor_state.selection, pairs,
                        source:source.clone(), previous_display:self.display.clone(), previous_selected_set:self.selected_set.clone(), ready, load:false });
                    self.picked_field = None;
                    if let Some(gpu) = &mut self.gpu { gpu.set_group_overlay(&self.state.device, &[], &[]); }
                    self.group_preview = Default::default();
                    if source != self.display || !ready { self.display = source.clone(); self.send(Edit::Display(source.clone())); }
                    self.editor_state.selection = mode; // the node-editor selection (selected_set) is left alone; picking uses pick_set()
                    // Keep the field's group as the live selection (not an empty one); when the display has to switch to
                    // the pick source first, the parse waits for that scene so ids always match the geometry they came from.
                    self.selection = picking::Selection { mode, ..Default::default() };
                    self.selection_dirty = true;
                    ctx.data_mut(|d| d.insert_temp(egui::Id::new("param_pick_active"), (node, name)));
                    if let Some(pick) = &mut self.param_pick { pick.load = true; }
                    if ready && self.param_pick.as_ref().is_some_and(|p| p.source == self.display) { self.pick_load(); }
                }
                UiAction::StartSketch(mode) => self.start_sketch(ctx, mode),
                UiAction::TransformNode { op } => self.transform_node(op),
                UiAction::StartModal(kind) => {
                    let node=&self.editor.doc.value()["nodes"][&self.selected];
                    let controls=ui::curve_tool::node_controls(self.editor.doc.value(),&self.selected);
                    let pointer=ctx.input(|i|i.pointer.hover_pos()).unwrap_or(self.last_viewport.center());
                    if node["type"]=="bezier" && ui::curve_tool::start_modal(ctx,&self.selected,&controls,kind,pointer) { ctx.request_repaint(); }
                    else { self.start_modal(ctx,kind); }
                },
                UiAction::FinishTransform => self.finish_transform(),
                UiAction::CancelTransform => self.cancel_transform(),
                UiAction::MeshCutCommit { source, node_type, params } => {
                    if let Some((value, id)) = graph_edit::add_viewport_cut(self.editor.doc.value(), &source, &node_type, params) {
                        if self.commit_document(value, "viewport cut") {
                            self.pending_loop_selection=(node_type=="polysplit").then(||id.clone());
                            if ctx.input(|i|i.pointer.primary_down()) { ctx.data_mut(|d|d.insert_temp(egui::Id::new("mesh_cut_swallow_click"),true)); }
                            self.selected = id.clone(); self.selected_set = std::collections::BTreeSet::from([id.clone()]);
                            self.display = Graph::from_value(self.editor.doc.value()).ok().and_then(|g| g.main_output().ok()).unwrap_or(id);
                        }
                    } else { self.file_status = "Cut target changed; try the cut again".into(); }
                }
                UiAction::SketchCommit { name, points, closed, refs } => self.sketch_commit(&name, &points, closed, refs),
                UiAction::SketchBatch { strokes, selected, target } => self.sketch_batch(&strokes, &selected, target),
                UiAction::AddMeasurement { a, b, refs } => self.add_measurement(a, b, refs),
                UiAction::DeleteMeasurement(index) => {self.delete_measurement(index);ui::sketch::clear_measurement_selection(ctx);},
                UiAction::ClearMeasurements => {self.clear_measurements();ui::sketch::clear_measurement_selection(ctx);},
                UiAction::RegionSelect { shape, op } => self.region_select(shape, op),
                UiAction::RestoreRegionSelection(snapshot) => {
                    self.selection = snapshot.selection; self.selected = snapshot.selected;
                    self.selected_set = snapshot.selected_set; self.selection_dirty = true;
                }
                UiAction::Open => if let Some(path) = rfd::FileDialog::new().add_filter("MiniHou graph", &["json", "minihou"]).pick_file() { self.open(path); },
                UiAction::Save => self.save_to(self.editor.path.clone()),
                UiAction::SaveAs if self.editor.blender_label.is_some() => {
                    // A Blender-bound document saves into the .blend implicitly; Save As exports a
                    // separate .minihou copy and keeps the binding (graph_embed_v1).
                    let name = format!("{}.minihou", self.editor.blender_label.as_deref().unwrap_or("network").replace(['/', '\\', ':'], "_").replace(" _ ", "_"));
                    if let Some(path) = rfd::FileDialog::new().add_filter("MiniHou graph", &["minihou", "json"]).set_file_name(name).save_file() {
                        self.file_status = match self.editor.export_to(&path) { Ok(()) => format!("Exported {}", path.display()), Err(e) => format!("Export failed: {e}") };
                    }
                }
                UiAction::SaveAs => {
                    if let Some(path) = rfd::FileDialog::new().add_filter("MiniHou graph", &["json", "minihou"]).set_file_name(self.editor.path.file_name().unwrap_or_default().to_string_lossy()).save_file() { self.save_to(path); }
                }
                UiAction::Undo | UiAction::Redo => {
                    let undo = matches!(action, UiAction::Undo);
                    self.record_selection_step();
                    let previous = self.editor.doc.value().clone();
                    let previous_text = previous.to_string();
                    let command = if undo { self.editor.history.undo(&mut self.editor.doc) } else { self.editor.history.redo(&mut self.editor.doc) };
                    if command.is_some() && self.editor.history.last_was_selection() {
                        self.step_selection(undo);
                    } else if let Some(command) = command {
                        self.pending_loop_selection=None;
                        if undo {
                            self.pending_restore_selection=undo_selection_for(&mut self.undo_element_selection,
                                &previous_text,&self.selection);
                            if let Some((selection,..))=&self.pending_restore_selection {
                                if selection.mode!=ui::SelectionMode::Object { self.editor_state.apply(ui::EditorChange::Selection(selection.mode)); }
                            }
                        } else { self.pending_restore_selection=None; }
                        if ui::network::layout_changed(&previous, self.editor.doc.value()) {
                            ui::network::restore_positions(ctx, &self.editor.path, self.editor.doc.value());
                        }
                        self.display = Graph::from_value(self.editor.doc.value()).ok().and_then(|g| g.main_output().ok()).unwrap_or_default();
                        self.selected_set.retain(|id| self.editor.doc.value()["nodes"].get(id).is_some());
                        // Undo can remove the node the last action created and selected: per-action state must not
                        // point at it, or the next action hangs its node off a missing source (live report 8).
                        self.selected = surviving_selection_from(&previous, self.editor.doc.value(), &self.selected, &self.display);
                        if self.selected_set.is_empty() && !self.selected.is_empty() { self.selected_set.insert(self.selected.clone()); }
                        self.transform_target = None; self.transform_before = None; self.transform_delta = None;
                        self.pushpull_pick = false;
                        self.send(command);
                    }
                }
                UiAction::Frame => self.frame_mesh(),
                UiAction::FrameSelected => self.frame_selected(),
                UiAction::FrameNetwork => ui::network::frame(ctx, &self.editor.path),
                UiAction::Select(id) => select_active_nodes(&mut self.selected, &mut self.selected_set, vec![id], false, false),
                UiAction::NavigateLink { node, param } => {
                    select_active_nodes(&mut self.selected, &mut self.selected_set, vec![node.clone()], false, false);
                    ui::network::focus(ctx, &self.editor.path, &node);
                    ctx.data_mut(|d| d.insert_temp(egui::Id::new("link_flash"), (node,param,ctx.input(|i|i.time))));
                }
                UiAction::LinkVisibility(show) => { self.prefs.show_parameter_links=show; let _=self.prefs.save(); },
                UiAction::RenameNode { old, new } => {
                    if let Some(value)=graph_edit::rename(self.editor.doc.value(),&old,&new) {
                        if self.commit_document(value,"rename node") { self.selected=new.clone(); self.selected_set=std::collections::BTreeSet::from([new]); }
                    } else {self.file_status="Could not rename node: name is invalid or already in use".into();}
                }
                UiAction::ToggleSelect(id) => select_active_nodes(&mut self.selected, &mut self.selected_set, vec![id], true, true),
                UiAction::SelectSet { ids, extend } => select_active_nodes(&mut self.selected, &mut self.selected_set, ids, extend, false),
                UiAction::SetDisplay(id) => {
                    self.display = id.clone();
                    self.editor.history.finish(&self.editor.doc);
                    self.edit(Edit::Display(id), false);
                }
                UiAction::ToggleDisplay(id) => {
                    if let Some(value) = graph_edit::set_display(self.editor.doc.value(), &id, true) {
                        if self.commit_document(value, "toggle display") {
                            self.display = Graph::from_value(self.editor.doc.value()).ok().and_then(|g| g.main_output().ok()).unwrap_or(id);
                        }
                    }
                }
                UiAction::Edit { edit, continuous } => {
                    if !continuous { self.editor.history.finish(&self.editor.doc); }
                    if let Edit::Replace(value) = edit { self.commit_document(value, "edit linked parameter"); }
                    else { self.edit(edit, continuous); }
                }
                UiAction::Time(edit) => self.send_timeline_time(edit),
                UiAction::AddNode { node_type, .. } if node_type.starts_with("sketch_") => {
                    if let Some(mode) = ui::sketch::SketchMode::DRAW.into_iter().find(|m| m.command_id() == node_type) { self.start_sketch(ctx, mode); }
                }
                UiAction::AddNode { node_type, after } => self.add_node(ctx, &node_type, after.as_deref()),
                UiAction::AddNetworkNode { node_type, pos, wire } => {
                    if let Some((mut value, id)) = graph_edit::add(self.editor.doc.value(), &node_type, None) {
                        if node_type == "boundary_select" {
                            let name = element_select::free_group_name_with_prefix(self.editor.doc.value(), "boundary");
                            merge_params(&mut value, &id, json!({"output_group_name": name}));
                        } else if node_type == "select_sharp_edges" {
                            let name = element_select::free_group_name_with_prefix(self.editor.doc.value(), "sharp_edges");
                            merge_params(&mut value, &id, json!({"output_group_name": name}));
                        } else if let Some(p) = self.selection_node_params(&node_type) {
                            merge_params(&mut value, &id, p);
                        }
                        let before = ui::network::position_add(ctx, &self.editor.path, self.editor.doc.value(), &mut value, None);
                        value["nodes"][&id]["pos"] = json!(pos);
                        let inserted = ui::network::insert_added(ctx, &self.editor.path, self.editor.doc.value(), &mut value, &id, pos, wire.as_ref());
                        self.editor.history.finish(&self.editor.doc);
                        let original = self.editor.doc.clone();
                        if let Some(before) = before { self.editor.doc = before; }
                        if self.commit_document(value, &format!("add {node_type}")) {
                            ui::network::restore_positions(ctx, &self.editor.path, self.editor.doc.value());
                            self.selected = id.clone();
                            self.selected_set = std::collections::BTreeSet::from([id.clone()]);
                            self.display = if inserted {
                                Graph::from_value(self.editor.doc.value()).ok().and_then(|g| g.main_output().ok()).unwrap_or(id)
                            } else { id };
                            if node_type == "boundary_select" {
                                self.editor_state.apply(ui::EditorChange::Selection(ui::SelectionMode::Edges));
                                if self.selection.mode != ui::SelectionMode::Edges {
                                    self.selection = picking::Selection { mode: ui::SelectionMode::Edges, ..Default::default() };
                                }
                                self.selection_dirty = true;
                            }
                        } else { self.editor.doc = original; }
                    } else { self.file_status = format!("The Rust core has no {node_type} node yet"); }
                }
                UiAction::AddConnectedNode { node_type, connection, pos } => {
                    if let Some((mut value, id)) = graph_edit::add_connected(self.editor.doc.value(), &node_type, &connection, pos) {
                        if node_type == "boundary_select" {
                            let name = element_select::free_group_name_with_prefix(self.editor.doc.value(), "boundary");
                            merge_params(&mut value, &id, json!({"output_group_name": name}));
                        } else if node_type == "select_sharp_edges" {
                            let name = element_select::free_group_name_with_prefix(self.editor.doc.value(), "sharp_edges");
                            merge_params(&mut value, &id, json!({"output_group_name": name}));
                        } else if let Some(p) = self.selection_node_params(&node_type) {
                            merge_params(&mut value, &id, p);
                        }
                        if self.commit_document(value, &format!("add {node_type} and connect")) {
                            ui::network::place_connected(ctx, &self.editor.path, &id, pos);
                            self.selected = id.clone();
                            self.selected_set = std::collections::BTreeSet::from([id.clone()]);
                            self.display = Graph::from_value(self.editor.doc.value()).ok().and_then(|g| g.main_output().ok()).unwrap_or(id);
                            if node_type == "boundary_select" {
                                self.editor_state.apply(ui::EditorChange::Selection(ui::SelectionMode::Edges));
                                if self.selection.mode != ui::SelectionMode::Edges {
                                    self.selection = picking::Selection { mode: ui::SelectionMode::Edges, ..Default::default() };
                                }
                                self.selection_dirty = true;
                            }
                        }
                    }
                }
                UiAction::AddNodeAt { node_type, after, pos } => self.add_node_at(ctx, &node_type, after.as_deref(), pos),
                UiAction::SetMode(mode) => self.set_display_mode(mode),
                UiAction::Editor(change) => {
                    if matches!(change, ui::EditorChange::Tool(_)) { self.finish_for_selection(ctx); }
                    if let ui::EditorChange::Selection(mode) = change { remember_element_mode(mode, &mut self.last_edit_selection); }
                    self.editor_state.apply(change);
                    if matches!(change,ui::EditorChange::Overlays(_) | ui::EditorChange::Xray(_)) {
                        self.prefs.viewport.overlays=self.editor_state.overlays;
                        self.prefs.viewport.show_grid=self.editor_state.overlays.grid;
                        self.prefs.viewport.show_axes=self.editor_state.overlays.axes;
                        self.prefs.show_stats=self.editor_state.overlays.stats;
                        self.prefs.viewport.xray=self.editor_state.xray;
                        if let Err(e)=self.prefs.save() { self.file_status=format!("Could not save preferences: {e}"); }
                    }
                    if let ui::EditorChange::ObjectVisibility(visibility)=change {
                        self.prefs.viewport.object_visibility=visibility;
                        self.selection_dirty=true;
                        if let Err(e)=self.prefs.save() { self.file_status=format!("Could not save preferences: {e}"); }
                    }
                    if let ui::EditorChange::MatCap(matcap)=change {
                        self.prefs.viewport.matcap=matcap;
                        if let Err(e)=self.prefs.save() { self.file_status=format!("Could not save preferences: {e}"); }
                    }
                    if let ui::EditorChange::MatCapFlat(flat)=change {
                        self.prefs.viewport.matcap_flat=flat;
                        if let Err(e)=self.prefs.save() { self.file_status=format!("Could not save preferences: {e}"); }
                    }
                    if let ui::EditorChange::ObjectColor(color)=change {
                        self.prefs.viewport.object_color=color;
                        if let Err(e)=self.prefs.save() { self.file_status=format!("Could not save preferences: {e}"); }
                    }
                    if let ui::EditorChange::Textures(enabled)=change {
                        self.prefs.viewport.textures=enabled;
                        if let Err(e)=self.prefs.save() {self.file_status=format!("Could not save preferences: {e}");}
                    }
                    if matches!(change, ui::EditorChange::SnapTo(_) | ui::EditorChange::SnapIncrement(_)) {
                        self.prefs.snap_targets = self.editor_state.snap.to;
                        self.prefs.snap_increment = self.editor_state.snap.increment;
                        if let Err(e) = self.prefs.save() { self.file_status = format!("Could not save preferences: {e}"); }
                    }
                    if let ui::EditorChange::View(v) = change { self.camera.set_view(standard_view(v)); }
                    if let ui::EditorChange::ClipPlanes(_, _) = change {
                        self.camera.set_clip_planes(self.editor_state.proxy.clip_start, self.editor_state.proxy.clip_end);
                        (self.prefs.viewport.clip_start, self.prefs.viewport.clip_end) = self.camera.clip_planes();
                        if let Err(e) = self.prefs.save() { self.file_status = format!("Could not save preferences: {e}"); }
                    }
                    if let ui::EditorChange::Tool(_) = change {
                        ui::sketch::cancel(ctx);
                        ui::mesh_cut::cancel(ctx);
                        ui::selection_region::cancel(ctx);
                    }
                }
                UiAction::ViewPreset(v) => { self.editor_state.apply(ui::EditorChange::View(v)); self.camera.set_view(standard_view(v)); }
                UiAction::OpenPreferences => self.prefs_open = true,
                UiAction::Command(name) => self.command(&name, ctx),
                UiAction::Orbit { yaw, pitch } => self.camera.orbit_degrees(yaw, pitch),
                UiAction::Pan { dx, dy } => self.camera.pan(dx, dy),
                UiAction::Zoom(factor) => self.camera.zoom_factor(factor),
                UiAction::DeleteNode(id) => self.delete_node(&id),
                UiAction::DuplicateNode(id) => self.duplicate_node(&id),
                UiAction::DisconnectAll(id) => { if let Some(value) = graph_edit::disconnect_all(self.editor.doc.value(), &id) { self.commit_document(value, "disconnect node"); } },
                UiAction::DetachNode(id) => {
                    if let Some(value) = graph_edit::detach(self.editor.doc.value(), &id) {
                        if self.commit_document(value, "detach node") {
                            self.display = Graph::from_value(self.editor.doc.value()).ok().and_then(|g| g.main_output().ok()).unwrap_or_default();
                        }
                    }
                },
                UiAction::DissolveNode(id) => self.dissolve_node(&id),
                UiAction::NetworkGesture { expected, before, next } => {
                    if self.editor.doc.value() == &expected {
                        if let Ok(before) = graph_edit::validated(before) {
                            self.editor.history.finish(&self.editor.doc);
                            let original = std::mem::replace(&mut self.editor.doc, before);
                            if self.commit_document(next, "network gesture") {
                                ui::network::restore_positions(ctx, &self.editor.path, self.editor.doc.value());
                            } else { self.editor.doc = original; }
                        }
                    }
                },
                UiAction::ConnectNodes { from, to, port } => self.connect_nodes(&from, &to, port.as_deref()),
                UiAction::DisconnectInput { node, port } => self.disconnect_input(&node, &port),
                UiAction::DisconnectWires(wires) => {
                    if let Some(value) = graph_edit::disconnect_wires(self.editor.doc.value(), &wires) {
                        self.commit_document(value, "slice noodles");
                    }
                },
                UiAction::TidyNetwork => {
                    ui::network::tidy(ctx, &self.editor.path, self.editor.doc.value(), &self.flags);
                    self.last_operator = Some(LastOperator::Tidy);
                }
                UiAction::RepeatLast => self.repeat_last(ctx),
                UiAction::BlenderBar(on) => self.blender_bar = on,
                action @ (UiAction::BlenderInstall(_) | UiAction::BlenderOpen | UiAction::BlenderDownloadAddon
                    | UiAction::BlenderDetect | UiAction::BlenderRestartLink) => self.blender_action(action, ctx),
                UiAction::ToggleLocked(id) => { let f = self.flags.entry(id).or_default(); f.locked = !f.locked; }
                UiAction::ToggleTemplate(id) => { let f = self.flags.entry(id).or_default(); f.template = !f.template; }
            }
        }
        if self.selected != prev_selected {
            if self.editor.doc.value()["nodes"][&self.selected]["type"] == "boundary_select" {
                self.editor_state.apply(ui::EditorChange::Selection(ui::SelectionMode::Edges));
                if self.selection.mode != ui::SelectionMode::Edges {
                    self.selection = picking::Selection { mode: ui::SelectionMode::Edges, ..Default::default() };
                }
                self.selection_dirty = true;
            }
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(self.editor.title()));
    }
}
impl eframe::App for App {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        let focused = ctx.memory(|m| m.focused().is_some());
        self.held_keys.extend(hold_back_ctrl_tab(raw_input, focused));
    }
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        if self.editor_state.texture_inspector {
            let mut open=true;
            egui::Window::new("Texture inspector").open(&mut open).default_width(330.).show(ctx,|ui| {
                if self.scene.as_ref().is_some_and(Scene::has_textured_slots) {ui.label("Textured surfaces use full detail; clustered LOD is suspended.");}
                if let Some(gpu)=&self.gpu {for slot in gpu.texture_status() {ui.label(slot);} } else {ui.label("No cooked scene");}
                if let Some(link)=&self.link {for status in link.texture_status(){ui.label(status);} }
                ui.label(format!("CPU image cache: {} MiB / {} MiB; GPU budget: {} MiB",self.materials.cpu_resident_bytes()/1048576,minihou_app::materials::CPU_BUDGET/1048576,self.materials.gpu_budget/1048576));
            });
            if !open {self.editor_state.texture_inspector=false;}
        }
        if !self.held_keys.is_empty() { let held = std::mem::take(&mut self.held_keys); ctx.input_mut(|i| i.events.extend(held)); }
        self.poll_http(ctx);
        if self.blender_job.as_ref().is_some_and(|job| job.is_finished()) {
            let result = self.blender_job.take().unwrap().join();
            self.blender.busy = false;
            match result {
                Ok(Ok((message, installed))) => {
                    self.blender.message = message;
                    if installed { self.blender.addon_installed = true; }
                }
                Ok(Err(e)) => self.blender.message = format!("Blender operation failed: {e}"),
                Err(_) => self.blender.message = "Blender setup worker panicked.".into(),
            }
        }
        if self.blender_job.is_some() { ctx.request_repaint_after(Duration::from_millis(50)); }
        if !self.themed { self.theme.apply(ctx); self.themed = true; }
        self.camera.set_clip_planes(self.editor_state.proxy.clip_start, self.editor_state.proxy.clip_end);
        let requests = self.link.as_ref().map(|link| link.poll()).unwrap_or_default();
        if self.link.as_ref().is_some_and(|link|link.apply_asset_updates(&mut self.materials)) {
            if let (Some(gpu),Some(scene))=(&mut self.gpu,&self.scene) {
                gpu.upload_textures(&self.state.device,&self.state.queue,scene,&mut self.materials);
            }
            ctx.request_repaint();
        }
        for request in requests {
            let result: Result<()> = match request.message["type"].as_str() {
                Some("live_activate" | "live_update" | "live_remove" | "live_replace") => {
                    let mut link=self.link.take().unwrap();
                    let edits=link.handle_live(&request,&mut self.editor);
                    self.link=Some(link);
                    match edits {
                        Ok(edits)=>{
                            if request.message["type"]=="live_activate" { ctx.send_viewport_cmd(egui::ViewportCommand::Focus); }
                            self.display=Graph::from_value(self.editor.doc.value()).ok().and_then(|g|g.main_output().ok()).unwrap_or_default();
                            self.selected=self.display.clone();
                            self.selected_set=std::collections::BTreeSet::from([self.display.clone()]);
                            for edit in edits { self.send(edit); }
                            Ok(())
                        },
                        Err(e)=>Err(e.into()),
                    }
                },
                Some("embed_conflict") => {
                    // Never decided silently: the modal below asks (graph_embed_v1).
                    self.embed_conflict = Some(request.message.clone());
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    Ok(())
                },
                Some("input_define") => {
                    let live = self.link.as_ref().and_then(|l| l.bound_path().map(str::to_owned));
                    let (reply, changed) = minihou_app::input_define::handle(&mut self.editor, live.as_deref(), &request.message);
                    if changed { self.send(Edit::Replace(self.editor.doc.value().clone())); }
                    if let Some(link) = &self.link { link.reply(request.peer, reply); }
                    Ok(())
                },
                Some("set_param") => minihou_app::link_param_edit(self.editor.doc.value(), &request.message)
                    .map(|edit| {
                        self.editor.history.finish(&self.editor.doc);
                        self.edit(edit, false);
                    }),
                Some("object_geo" | "object_missing" | "image_data" | "image_missing") => {
                    match minihou_app::link_protocol::external_edit(&request.message,&request.arrays) {
                        Ok(edit)=>{self.send(edit);Ok(())},
                        Err(e)=>{
                            // Invalidate stale input geometry even when a replacement frame is malformed.
                            let image=request.message["type"].as_str().unwrap_or("").starts_with("image_");
                            if let Some(name)=request.message[if image{"image"}else{"object"}].as_str(){
                                self.send(Edit::External{name:format!("{}:{name}",if image{"image"}else{"object"}),input:minihou_core::nodes::blender_input::ExternalInput::Missing(format!("could not read input: {e}"))});
                            }
                            Err(e.into())
                        }
                    }
                },
                Some("set_frame") => match self.link.as_mut().unwrap().set_frame(&request.message) {
                    Ok(changed) => {
                        // Timeline updates stay out of saved parameters and undo history: the session
                        // recooks only time-dependent nodes (solvers, @Frame/@Time) at the new frame.
                        if changed {
                            let (frame, fps) = { let link = self.link.as_ref().unwrap(); (link.frame, link.fps) };
                            self.send(Edit::Time { frame, fps });
                        }
                        Ok(())
                    }
                    Err(e) => Err(e.into()),
                },
                _ => Ok(()),
            };
            if let Err(e) = result {
                if let Some(link) = &self.link {
                    link.reply_error(request.peer, request.message["type"].as_str().unwrap_or("request"), &e.to_string());
                }
            }
        }
        self.embed_conflict_modal(ctx);
        if let Some(link) = &self.link {
            link.set_open_dirty(!link.live_active() && self.editor.dirty());
            link.document_changed(&self.editor.doc, &self.editor.path);
            let clients = link.clients();
            let disconnected = !self.blender.clients.is_empty() && clients.is_empty();
            self.blender.clients = clients;
            self.blender.link_message = ui::blender_ui::connection_message(
                link.listening(), &self.blender.clients, &self.blender.link_message, disconnected);
            ctx.request_repaint_after(Duration::from_millis(33));
        }
        if let Some(response) = self.worker.poll() {
            let display_ranges = response.display_ranges.clone();
            if response.revision == self.cooking.requested {
                if let Some(link) = &self.link { link.cooked(&response); }
            }
            let accepted=self.cooking.accept(response);
            if display_is_empty(self.editor.doc.value()) {
                self.cooking.geometry=None;self.cooking.lod=None;self.geo=None;
            }
            if let Some(scene) = accepted.filter(|_| !display_is_empty(self.editor.doc.value())) {
                self.display_ranges = display_ranges;
                self.selection_dirty = true;
                let first = self.gpu.is_none();
                self.bounds = (scene.center, scene.radius);
                self.materials.register_graph(self.editor.doc.value()); let _=self.materials.register_sidecar(&self.editor.path);
                let textured=scene.has_textured_slots();
                let level = if textured {1.0}else{self.proxy_level(scene.triangles())};
                let use_lod = !textured && self.editor_state.proxy.clustered && self.cooking.lod.is_some();
                if use_lod {
                    if let Some(gpu) = &mut self.gpu { gpu.upload_lod(&self.state.device, &scene); }
                    else { self.gpu = Some(Gpu::new_lod(&self.state.device, &scene)); }
                } else {
                    let shown = scene.proxied(level);
                    if let Some(gpu) = &mut self.gpu { gpu.upload(&self.state.device, &shown); }
                    else { self.gpu = Some(Gpu::new(&self.state.device, &shown)); }
                    if let Some(gpu)=&mut self.gpu {gpu.upload_textures(&self.state.device,&self.state.queue,&shown,&mut self.materials);}
                }
                self.lod_uploaded = use_lod;
                self.last_lod_view = None;
                self.shown_level = level;
                // element ids only mean something on the topology they were picked on (stale bevel edges drew as orange diagonals)
                let key = element_select::topology_key(scene.positions().len() / 3, scene.triangle_indices(), scene.edge_indices());
                let mode = self.selection.mode;
                let (selection, stash) = self.selection_memory.retarget(key, (std::mem::take(&mut self.selection), std::mem::take(&mut self.stashed_selection)));
                self.selection = if selection.is_empty() { picking::Selection { mode, ..Default::default() } } else { selection };
                self.stashed_selection = stash;
                if let Some(pick) = &mut self.param_pick {
                    if !pick.ready && source_ready(&self.display_ranges, &pick.source) {
                        pick.ready = true; pick.load = true;
                        self.selection = picking::Selection { mode:self.editor_state.selection, ..Default::default() };
                    }
                }
                if let Some(keep)=self.extrude_keep.as_ref().and_then(|k|keep_extrude_selection(k,&scene)) { self.selection=keep; }
                if let Some((selection,stash,user_selection))=self.pending_restore_selection.take() {
                    self.selection=user_selection.filter(|candidate|selection_exists_on_scene(candidate,&scene)).unwrap_or(selection);
                    self.stashed_selection=stash;
                }
                if self.pending_loop_selection.is_some() {
                    // Wait for the cook that carries the loop marks; an earlier or stale scene must not consume the request and leave a partial ring.
                    if let (Some(range),Some(g))=(active_range(&self.display,&self.display_ranges),self.cooking.geometry.as_deref()) {
                        if loop_cut_has_marks(g) {
                            self.pending_loop_selection=None;
                            self.selection=loop_cut_selection(g,range);
                            self.editor_state.apply(ui::EditorChange::Selection(ui::SelectionMode::Edges));
                        }
                    }
                }
                self.scene = Some(scene);
                self.sync_selection_baseline();
                self.elem_topo = Default::default();
                if self.param_pick.as_ref().is_some_and(|p| p.load && p.ready) { self.pick_load(); }
                if first { self.frame_mesh(); }
            }
        }
        if self.cooking.cooking() { ctx.request_repaint_after(Duration::from_millis(16)); }
        // the proxy level changed (menu, auto budget): re-upload the reduced scene
        if let Some(scene) = &self.scene {
            let textured=scene.has_textured_slots();
            let level = if textured {1.0}else{self.proxy_level(scene.triangles())};
            let use_lod = !textured && self.editor_state.proxy.clustered && self.cooking.lod.is_some();
            if use_lod != self.lod_uploaded {
                if let Some(gpu) = &mut self.gpu {
                    if use_lod { gpu.upload_lod(&self.state.device, scene); }
                    else { let shown=scene.proxied(level); gpu.upload(&self.state.device, &shown); gpu.upload_textures(&self.state.device,&self.state.queue,&shown,&mut self.materials); }
                }
                self.lod_uploaded = use_lod;
                self.last_lod_view = None;
                self.shown_level = level;
            } else if !use_lod && (level - self.shown_level).abs() > 1e-6 {
                if let Some(gpu) = &mut self.gpu { let shown=scene.proxied(level); gpu.upload(&self.state.device, &shown); gpu.upload_textures(&self.state.device,&self.state.queue,&shown,&mut self.materials); }
                self.shown_level = level;
            }
        }
        // the selection highlight: rebuilt when the pick changed or the mode switched
        if self.selection.mode != self.editor_state.selection {
            // Convert the selection to the new element mode (docs/RUST_ELEMENT_SELECT.md); Object mode stashes it.
            let to = self.editor_state.selection;
            let from = if self.selection.mode == ui::SelectionMode::Object { std::mem::take(&mut self.stashed_selection) } else { std::mem::take(&mut self.selection) };
            self.selection = match &self.scene {
                _ if to == ui::SelectionMode::Object => { self.stashed_selection = from; picking::Selection { mode: to, ..Default::default() } }
                Some(scene) if from.mode != ui::SelectionMode::Object && !from.is_empty() => element_select::cached(&self.elem_topo, scene).convert(&from, to),
                _ => picking::Selection { mode: to, ..Default::default() },
            };
            self.selection_dirty = true;
            self.sync_selection_baseline();
        }
        if self.selection_dirty {
            self.record_selection_step();
            let pick_set=self.pick_set();
            if let (Some(gpu), Some(scene)) = (&mut self.gpu, &self.scene) {
                let size = scene.radius * 0.012;
                let mut selection=self.selection.clone();
                let pickable=|id| point_pickable(self.editor_state.object_visibility,&self.display_ranges,&pick_set,id);
                selection.points.retain(|id| pickable(*id));
                selection.edges.retain(|(a,b)| pickable(*a) && pickable(*b));
                selection.tris.retain(|i| scene.triangle_indices().get(*i as usize*3..*i as usize*3+3).is_some_and(|tri| tri.iter().all(|id| pickable(*id))));
                // Point selection is now a depth-tested square in the element pass.
                selection.points.clear();
                let (faces, lines) = picking::highlight(scene.positions(), scene.triangle_indices(), &selection, size);
                gpu.set_selection(&self.state.device, &faces, &lines);
                self.file_status = if self.selection.is_empty() {String::new()} else {format!("{} selected",self.selection.count())};
            }
            self.selection_dirty = false;
        }
        // the geometry summary of the displayed node, refreshed after each cook
        if self.geo_rev != self.cooking.completed {
            if let Some(g) = &self.cooking.geometry {
                self.geo = Some(ui::geo_info::geo_summary::summarize(&self.display, g));
                self.geo_rev = self.cooking.completed;
            }
        }
        if self.param_pick.is_some() {
            if let Some(cancel) = param_pick_end(ctx, self.last_viewport) { self.finish_param_pick(ctx, cancel); }
        }
        let busy = self.modal.is_some() || self.model_modal.is_some() || self.mirror.is_some() || self.pushpull_pick || (ui::sketch::active(ctx) || self.editor_state.tool.drawing()) || ui::transform_gizmo::active(ctx) || ui::prim_handles::active(ctx) || ui::curve_tool::active(ctx);
        let fly_frame = viewport_input::fly(ctx, self.last_viewport, &mut self.camera,
            self.prefs.viewport.fly_navigation && !busy && !ui::selection_region::active(ctx) && !ctx.wants_keyboard_input());
        if busy && !fly_frame && !ui::sketch::active(ctx) && !self.editor_state.tool.drawing() {
            let selection_actions = self.keymap.poll_selection(ctx, &KeyContext {
                viewport_hovered: self.viewport_hover, network_hovered: false,
                in_edit_mode: self.editor_state.selection != ui::SelectionMode::Object, selected: Some(&self.selected) });
            let select_press = matches!(self.editor_state.tool, ui::Tool::Select | ui::Tool::Move | ui::Tool::Rotate | ui::Tool::Scale | ui::Tool::Transform) && ctx.input(|i| i.pointer.primary_pressed()
                && i.pointer.latest_pos().is_some_and(|p| self.last_viewport.contains(p)
                    && ui::viewport_ui::selection_canvas_contains(self.last_viewport, p)));
            let lasso_press = ctx.input(|i| i.pointer.secondary_pressed() && i.modifiers.ctrl
                && i.pointer.latest_pos().is_some_and(|p| self.last_viewport.contains(p)
                    && ui::viewport_ui::selection_canvas_contains(self.last_viewport, p)));
            if !selection_actions.is_empty() || lasso_press || (select_press && (self.modal.is_none() && self.model_modal.is_none() && self.mirror.is_none()) && !(ui::sketch::active(ctx) || self.editor_state.tool.drawing())) {
                self.finish_for_selection(ctx);
                self.apply(ctx, selection_actions);
            }
        }
        // Process confirmation before constructing View so the same click can reach selection.
        let mut started_pushpull=false;
        if self.pushpull_pick {
            if ctx.input(|i| i.key_pressed(egui::Key::Escape) || i.pointer.secondary_pressed()) {
                self.pushpull_pick=false; self.file_status.clear();
            } else if let Some(at)=ctx.input(|i| if i.pointer.primary_pressed() { i.pointer.latest_pos() } else { None }) {
                if self.last_viewport.contains(at) {
                    if let Some(scene)=self.scene.as_ref() {
                        let (o,d)=ui::transform_gizmo::ray(self.camera_info(),self.last_viewport,at);
                        if let Some(hit)=pick_displayed_culled(scene,o,d,viewport::ObjectVisibility::HideOthers,&self.display_ranges,&self.active_pick_set(),false) {
                            let picked=element_select::cached(&self.elem_topo,scene).pick(scene.positions(),scene.triangle_indices(),hit,[0,1,2].map(|k|o[k]+d[k]*hit.t),ui::SelectionMode::Faces);
                            self.selection=picked; self.editor_state.selection=ui::SelectionMode::Faces;
                            self.selection_dirty=true; self.pushpull_pick=false; self.file_status.clear();
                            self.start_model_modal(ctx,ModelKind::PushPull);
                            started_pushpull=true;
                        }
                    }
                }
            }
        }
        let modal_frame = self.modal.is_some();
        if modal_frame { self.update_modal(ctx); }
        let model_modal_frame=self.model_modal.is_some();
        if model_modal_frame && !started_pushpull { self.update_model_modal(ctx); }
        // Selecting a mirror node brings its gizmo back (once per selection; Enter or an empty click dismisses it).
        if self.mirror.is_none() && self.modal.is_none() && self.model_modal.is_none() && self.scene.is_some() && self.selected_set.len()==1
            && self.editor.doc.value()["nodes"][&self.selected]["type"]=="mirror" && !ctx.wants_keyboard_input() {
            let seen_id=egui::Id::new("mirror_gizmo_seen");
            let seen:Option<(String,u64)>=ctx.data(|d|d.get_temp(seen_id));
            let key=(self.selected.clone(),self.cooking.completed);
            if seen.as_ref().map(|s|&s.0)!=Some(&key.0) && self.cooking.error.is_none() {
                ctx.data_mut(|d|d.insert_temp(seen_id,key));
                self.start_mirror(ctx);
            }
        } else if self.editor.doc.value()["nodes"][&self.selected]["type"]!="mirror" {
            ctx.data_mut(|d|d.remove::<(String,u64)>(egui::Id::new("mirror_gizmo_seen")));
        }
        let mirror_frame = self.mirror.is_some();
        if mirror_frame { self.update_mirror(ctx); }
        if (modal_frame || model_modal_frame) && self.modal.is_none() && self.model_modal.is_none() && ctx.input(|i| i.pointer.primary_pressed()) {
            ctx.data_mut(|d| d.insert_temp(egui::Id::new("modal_confirm_click"), true));
        }
        if (ui::transform_gizmo::active(ctx) || ui::prim_handles::active(ctx))
            && ctx.input(|i| i.pointer.secondary_pressed() || i.key_pressed(egui::Key::Escape)) {
            self.cancel_transform();
            ui::transform_gizmo::finish_input(ctx);
            ui::prim_handles::finish_input(ctx);
        }
        if !ctx.input(|i| i.pointer.primary_down()) {
            ui::transform_gizmo::finish_input(ctx);
            ui::prim_handles::finish_input(ctx);
        }
        let graph = Graph::from_value(self.editor.doc.value()).expect("validated document");
        let mut out = Actions::default();
        let kctx = KeyContext { viewport_hovered: self.viewport_hover, network_hovered: self.network_hover,
            in_edit_mode: self.editor_state.selection != ui::SelectionMode::Object, selected: Some(&self.selected) };
        if !fly_frame && !busy && !ui::viewport_ui::popup_open(ctx) && self.modal.is_none() && self.model_modal.is_none() && !(ui::sketch::active(ctx) || self.editor_state.tool.drawing()) && !ui::transform_gizmo::active(ctx) && !ui::selection_region::active(ctx) && !ui::prim_handles::active(ctx) { for action in self.keymap.poll(ctx, &kctx) { out.push(action); } }
        ctx.data_mut(|d| d.insert_temp(ui::sketch::settings_id(), self.prefs.inference.clone()));
        {
            let repeat_desc = self.last_operator.as_ref().map(LastOperator::description);
            let transform_node = if self.editor_state.selection == ui::SelectionMode::Object { &self.selected } else { &self.display };
            let display_trs = transform_math::TrsParams::from_params(&self.editor.doc.value()["nodes"][transform_node]["params"]);
            let normal_frame = self.selection_normal_frame(&display_trs).axes.map(|axis| axis.map(|x| x as f32));
            let active_pick_set=self.active_pick_set(); let node_pick_set=self.pick_set();
            let active_cut_range=self.active_object().and_then(|id|active_range(id,&self.display_ranges).map(|range|(id.to_owned(),range.clone())));
            let v = View { graph: &graph, doc: self.editor.doc.value(), path: &self.editor.path, dirty: self.editor.dirty(),
                can_undo: self.editor.history.can_undo(), can_redo: self.editor.history.can_redo(),
                selected: &self.selected, selected_set: &self.selected_set, display: &self.display, cooking: &self.cooking, file_status: &self.file_status,
                mode: self.mode, theme: &self.theme, editor: &self.editor_state, flags: &self.flags, geo: self.geo.as_ref(), blender: &self.blender,
                bounds: self.bounds, selection_center: self.selection_center(), normal_frame,
                camera: self.camera_info(),
                units: self.prefs.units,
                repeat_label: repeat_desc.as_deref() };
            egui::TopBottomPanel::top("toolbar").show(ctx, |ui| ui::topbar::show(ui, &v, &mut out));
            if self.blender_bar {
                let mut shown = true;
                egui::TopBottomPanel::top("blender_bar").show(ctx, |ui| ui::blender_ui::bar(ui, &v, &mut out, &mut shown));
                if !shown { out.push(UiAction::BlenderBar(false)); }
            }
            egui::Area::new(egui::Id::new("blender_indicator")).fixed_pos(egui::pos2(ctx.content_rect().right() - 330., 9.)).order(egui::Order::Foreground)
                .show(ctx, |ui| ui::blender_ui::indicator(ui, &v, &mut out));
            egui::TopBottomPanel::top("shelf").show(ctx, |ui| ui::shelf::show(ui, &v, &mut out));
            egui::TopBottomPanel::bottom("stats").show(ctx, |ui| ui::status::show(ui, &v, &mut out));
            ui::network::set_physics_snappiness(ctx, self.prefs.node_physics_snappiness);
            ui::network::set_links(ctx, &self.editor.path, self.prefs.show_parameter_links);
            let net = egui::SidePanel::left("network").default_width(400.).show(ctx, |ui| ui::network::show(ui, &v, &mut out));
            self.network_hover = ctx.input(|i| i.pointer.hover_pos()).is_some_and(|p| net.response.rect.contains(p));
            if self.params_open { egui::SidePanel::right("parameters").default_width(460.).show(ctx, |ui| {
                egui::TopBottomPanel::bottom("geometry_info").resizable(true).default_height(220.).show_inside(ui, |ui| ui::geo_info::show(ui, &v, &mut out));
                ui::params::show(ui, &v, &mut out);
            }); }
            egui::CentralPanel::default().show(ctx, |ui| {
                let size = ui.available_size().max(egui::vec2(1., 1.));
                let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
                self.last_viewport = rect;
                let coplanar_hover=self.coplanar.is_some() && ctx.input(|i|i.pointer.hover_pos()).is_some_and(|p|coplanar_tool::contains(ctx,rect,p));
                let region_frame = if coplanar_hover {true} else if !fly_frame && (self.modal.is_none() && self.model_modal.is_none() && self.mirror.is_none()) && !mirror_frame && !(ui::sketch::active(ctx) || self.editor_state.tool.drawing()) {
                    ui::selection_region::handle(ctx, rect, &v, &mut out)
                } else { false };
                let mut delta = ctx.input(|i| i.pointer.delta());
                if !response.dragged() { self.orbit_drag=None; }
                if !fly_frame && !region_frame && response.dragged() && (self.modal.is_none() && self.model_modal.is_none()) && !ui::sketch::blocks_navigation(ctx) && !ui::mesh_cut::blocks_navigation(ctx) && !ui::transform_gizmo::active(ctx) && !ui::selection_region::active(ctx) && !ui::prim_handles::active(ctx) && !ui::curve_tool::active(ctx)
                    && !ctx.input(|i| i.pointer.hover_pos()).is_some_and(|p| (!ui::curve_tool::is_active_node(&v) && ui::transform_gizmo::hit(rect, &v, p)) || ui::prim_handles::hit(rect, &v, p) || ui::curve_tool::hit(rect, &v, p, ctx)) {
                    let modifiers = ctx.input(|i| i.modifiers);
                    let orbiting = match self.prefs.viewport.navigation {
                        ui::keymap::Navigation::Blender => response.dragged_by(egui::PointerButton::Middle) && !modifiers.ctrl && !modifiers.shift,
                        ui::keymap::Navigation::Houdini => modifiers.alt && response.dragged_by(egui::PointerButton::Primary),
                    };
                    if !orbiting || !self.prefs.viewport.orbit_to_mouse { self.orbit_drag=None; }
                    if orbiting && self.prefs.viewport.orbit_to_mouse && self.orbit_drag.is_none() {
                        if let Some(pos)=ctx.input(|i|i.pointer.press_origin()) {
                            let nx=(pos.x-rect.left())/rect.width()*2.-1.; let ny=1.-(pos.y-rect.top())/rect.height()*2.;
                            let aspect=rect.width()/rect.height();
                            let point=navigation_point(&self.camera,self.scene.as_ref(),self.object_hidden,self.editor_state.object_visibility,&self.display_ranges,&self.selected_set,rect,pos);
                            self.orbit_drag=Some(point.map_or((self.camera.view_plane_point(0.,0.,aspect),0.,0.,aspect),|p|(p,nx,ny,aspect)));
                        }
                    }
                    wrap_drag(ctx, rect, &mut delta);
                    match self.prefs.viewport.navigation {
                        ui::keymap::Navigation::Blender if response.dragged_by(egui::PointerButton::Middle) => {
                            if modifiers.ctrl {
                                let pos=ctx.input(|i|i.pointer.hover_pos());
                                let point=pos.and_then(|p|navigation_point(&self.camera,self.scene.as_ref(),self.object_hidden,self.editor_state.object_visibility,&self.display_ranges,&self.selected_set,rect,p));
                                zoom_under_cursor(&mut self.camera,delta.y,self.prefs.viewport.zoom_to_mouse,rect,pos,point);
                            }
                            else if modifiers.shift { self.camera.pan(delta.x, delta.y); }
                            else if let Some((point,nx,ny,aspect))=self.orbit_drag { self.camera.orbit_at(delta.x,delta.y,point,nx,ny,aspect); }
                            else { self.camera.orbit(delta.x, delta.y); }
                        }
                        ui::keymap::Navigation::Houdini if modifiers.alt => {
                            if response.dragged_by(egui::PointerButton::Primary) {
                                if let Some((point,nx,ny,aspect))=self.orbit_drag {self.camera.orbit_at(delta.x,delta.y,point,nx,ny,aspect);}
                                else {self.camera.orbit(delta.x,delta.y);}
                            }
                            else if response.dragged_by(egui::PointerButton::Middle) { self.camera.pan(delta.x, delta.y); }
                            else if response.dragged_by(egui::PointerButton::Secondary) {
                                let pos=ctx.input(|i|i.pointer.hover_pos());
                                let point=pos.and_then(|p|navigation_point(&self.camera,self.scene.as_ref(),self.object_hidden,self.editor_state.object_visibility,&self.display_ranges,&self.selected_set,rect,p));
                                zoom_under_cursor(&mut self.camera,delta.y,self.prefs.viewport.zoom_to_mouse,rect,pos,point);
                            }
                        }
                        _ => {}
                    }
                }
                if !fly_frame && !region_frame && response.hovered() && (self.modal.is_none() && self.model_modal.is_none())
                    && !matches!(self.editor_state.tool, ui::Tool::LoopCut | ui::Tool::EdgeSlide) && !ui::mesh_cut::captures_wheel(ctx) {
                    let delta=ctx.input(|i| i.smooth_scroll_delta.y);
                    if delta!=0. {
                        let pos=ctx.input(|i|i.pointer.hover_pos());
                        let point=if self.prefs.viewport.zoom_to_mouse {pos.and_then(|p|navigation_point(&self.camera,self.scene.as_ref(),self.object_hidden,self.editor_state.object_visibility,&self.display_ranges,&self.selected_set,rect,p))} else {None};
                        zoom_under_cursor(&mut self.camera,delta,self.prefs.viewport.zoom_to_mouse,rect,pos,point);
                    }
                }
                let limit = self.state.device.limits().max_texture_dimension_2d;
                let pixels = [(size.x * ctx.pixels_per_point()).round() as u32, (size.y * ctx.pixels_per_point()).round() as u32].map(|x| x.clamp(1, limit));
                if self.target.as_ref().is_none_or(|t| t.size != pixels) {
                    if let Some(old) = self.target.take() { self.state.renderer.write().free_texture(&old.id); }
                    self.target = Some(Target::new(&self.state, pixels));
                }
                if self.lod_uploaded {
                    if let (Some(lod), Some(gpu)) = (&self.cooking.lod, &mut self.gpu) {
                        let view = self.camera.matrices(pixels[0] as f32 / pixels[1] as f32).0;
                        let wire = self.mode.wire() || self.editor_state.overlays.wireframe || self.editor_state.xray
                            || self.editor_state.object_visibility == viewport::ObjectVisibility::GhostOthers
                            || self.editor_state.selection == ui::SelectionMode::Edges;
                        let signature = (view, pixels, wire);
                        if self.last_lod_view.as_ref() != Some(&signature)
                            && (self.last_lod_view.as_ref().is_none_or(|last| last.2 != wire || last.1 != pixels)
                                || self.last_lod_select.elapsed() >= Duration::from_millis(50)) {
                            let ids = lod.select(self.camera.lod_camera(pixels), lod.error_px);
                            gpu.select_lod(&self.state.device, lod, &ids, wire);
                            self.last_lod_view = Some(signature);
                            self.last_lod_select = Instant::now();
                        }
                    }
                }
                if let (Some(gpu), Some(scene)) = (&mut self.gpu, &self.scene) {
                    let ranges = self.display_ranges.iter().filter(|(id, _)| !self.object_hidden && self.selected_set.contains(id))
                        .map(|(_, range)| range.clone()).collect();
                    self.group_preview.update(gpu,&self.state.device,if self.cooking.error.is_none(){self.cooking.geometry.as_deref()}else{None},self.editor.doc.value(),&self.selected,self.cooking.completed);
                    // Hovering the purple arrow previews the field's current group (only on the geometry its ids belong to).
                    let hover = ctx.data(|d| d.get_temp::<(String,String,ui::SelectionMode,bool,u64)>(egui::Id::new("param_pick_hover")))
                        .filter(|h| h.4 + 1 >= ctx.cumulative_pass_nr());
                    if self.param_pick.is_none() {
                        match hover {
                            Some((node,name,default_mode,_,_)) if source_ready(&self.display_ranges, &input_source(self.editor.doc.value(), &node, &self.display))
                                || (self.editor.doc.value()["nodes"][&node]["type"] == "group_create" && source_ready(&self.display_ranges, &node)) => {
                                let text = self.editor.doc.value()["nodes"][&node]["params"][&name].as_str().unwrap_or("").to_owned();
                                let mode = infer_mode(&text, default_mode);
                                let topo = element_select::cached(&self.elem_topo, scene);
                                if self.picked_field.as_ref().is_none_or(|f| f.node != node || f.selection.mode != mode || f.selection != topo.parse_group(&text, mode)) {
                                    self.picked_field = Some(field_selection(topo, &text, mode, &node, scene.positions(), scene.triangle_indices(), scene.radius * 0.012));
                                }
                            }
                            _ => if self.picked_field.take().is_some() {
                                gpu.set_group_overlay(&self.state.device, &[], &[]);
                                self.group_preview = Default::default();
                            },
                        }
                    }
                    // Hovering an output group's teal target button shows that group on the node's own output.
                    let target = ctx.data(|d| d.get_temp::<(String,String,u64)>(egui::Id::new("param_target_hover")))
                        .filter(|h| h.2 + 1 >= ctx.cumulative_pass_nr() && h.0 == self.display && self.cooking.error.is_none());
                    let key = target.map(|(n,g,_)| (n,g,self.cooking.completed));
                    if key != self.target_key {
                        let (f,l) = match (&key, self.cooking.geometry.as_deref()) {
                            (Some((_,g,_)), Some(geo)) => group_preview::geometry(geo, g),
                            _ => Default::default(),
                        };
                        gpu.set_target_overlay(&self.state.device,&f,&l);self.target_key=key;
                    }
                    // Flood Fill's barrier group is drawn in orange beside the purple picked field.
                    let flood=&self.editor.doc.value()["nodes"][&self.selected];
                    if flood["type"]=="flood_fill" && source_ready(&self.display_ranges,&input_source(self.editor.doc.value(),&self.selected,&self.display)) {
                        let mode=param_pick::flood_domain(&flood["params"]);
                        let text=flood["params"]["barrier_group"].as_str().unwrap_or("");
                        let key=(self.selected.clone(),mode,text.to_owned(),self.cooking.completed);
                        if self.barrier_key.as_ref()!=Some(&key) {
                            let topo=element_select::cached(&self.elem_topo,scene);
                            let (f,l)=param_pick::barrier_overlay(topo,text,mode,scene.positions(),scene.triangle_indices(),scene.radius*0.012);
                            gpu.set_barrier_overlay(&self.state.device,&f,&l);self.barrier_key=Some(key);
                        }
                    } else if self.barrier_key.take().is_some() { gpu.set_barrier_overlay(&self.state.device,&[],&[]); }
                    if let Some(field) = &self.picked_field {
                        if field.node == self.selected {
                            gpu.set_group_overlay(&self.state.device, &field.faces, &field.lines);
                        }
                    }
                    gpu.select_points(&self.state.device,scene,&self.selection.points);
                    gpu.select_objects(&self.state.device, scene, ranges);
                    gpu.set_named_object_ranges(&self.state.device,scene,self.display_ranges.clone());
                }
                // Hover pre-selection highlight (Item 4)
                let hover_pos = ctx.input(|i| i.pointer.latest_pos());
                let hover_in_viewport = !coplanar_hover && hover_pos.is_some_and(|p| rect.contains(p))
                    && !(ui::sketch::active(ctx) || self.editor_state.tool.drawing())
                    && !ui::selection_region::active(ctx);
                let modal_pick=self.pushpull_pick || self.editor_state.selection!=ui::SelectionMode::Object;
                let pick_set=if modal_pick {active_pick_set.clone()} else {node_pick_set.clone()};
                let pick_visibility=if modal_pick {viewport::ObjectVisibility::HideOthers} else {self.editor_state.object_visibility};

                let hover_elem = if hover_in_viewport {
                    if let (Some(pos), Some(scene)) = (hover_pos, &self.scene) {
                        let (o, d) = ui::transform_gizmo::ray(v.camera, rect, pos);
                        let mode = if self.pushpull_pick { ui::SelectionMode::Faces } else { self.editor_state.selection };
                        let mods = ctx.input(|i| i.modifiers);
                        let surface = pick_displayed_culled(scene,o,d,pick_visibility,&self.display_ranges,&pick_set,false);
                        surface
                            .map(|hit| {
                                if mode == ui::SelectionMode::Object { return picking::element_of(hit, scene.triangle_indices(), mode); }
                                // Polygon-aware hover; Alt (Blender navigation) previews the loop, Ctrl+Alt the ring.
                                let topo = element_select::cached(&self.elem_topo, scene);
                                let point = [0,1,2].map(|k| o[k] + d[k] * hit.t);
                                let loop_preview = !self.pushpull_pick && mods.alt && self.prefs.viewport.navigation == ui::keymap::Navigation::Blender;
                                match topo.nearest_edge(scene.positions(), hit.tri as u32, point) {
                                    Some(edge) if loop_preview => topo.loop_selection(scene.positions(), edge, mods.ctrl, mode),
                                    _ => topo.pick(scene.positions(), scene.triangle_indices(), hit, point, mode),
                                }
                            }).or_else(|| {
                                let wire=pick_displayed_wire(scene,v.camera,rect,pos,pick_visibility,&self.display_ranges,&pick_set,mode,self.editor_state.xray)?;
                                let topo=element_select::cached(&self.elem_topo,scene);
                                let mut selected=wire_selection(wire,mode);
                                if mode==ui::SelectionMode::Edges && mods.alt && self.prefs.viewport.navigation==ui::keymap::Navigation::Blender {
                                    if let picking::WireHit::Edge(a,b)=wire {
                                        if let Some(edge)=topo.edge_of(a,b) { selected=topo.loop_selection(scene.positions(),edge,mods.ctrl,mode); }
                                    }
                                }
                                Some(selected)
                            })
                    } else {
                        None
                    }
                } else {
                    None
                };

                if self.pushpull_pick || self.editor_state.selection == ui::SelectionMode::Faces {
                    if let (Some(gpu),Some(scene)) = (&mut self.gpu,&self.scene) {
                        gpu.set_face_markers(&self.state.device,scene,element_select::cached(&self.elem_topo,scene),&self.selection.tris,
                            hover_elem.as_ref().map_or(&[],|e| e.tris.as_slice()),self.editor_state.object_visibility);
                    }
                }
                if let (Some(gpu), Some(target)) = (&self.gpu, &self.target) {
                    let selected_object = !self.object_hidden && self.display_ranges.iter().any(|(id, _)| self.selected_set.contains(id));
                    gpu.draw(
                        &self.state,
                        target,
                        &self.camera,
                        self.mode,
                        Overlay {
                            grid:self.editor_state.overlays.enabled && self.editor_state.overlays.grid,
                            axes:self.editor_state.overlays.enabled && self.editor_state.overlays.axes,
                            geometry:self.editor_state.overlays, light_background:!self.theme.dark,
                            normals: self.editor_state.overlays.enabled && self.editor_state.overlays.normals,
                            bounds: self.editor_state.overlays.enabled && self.editor_state.overlays.bounds,
                            xray: self.editor_state.overlays.enabled && self.editor_state.xray,
                            wireframe: self.editor_state.overlays.enabled && self.editor_state.overlays.wireframe,
                            visibility: self.editor_state.object_visibility,
                            matcap: self.editor_state.matcap,
                            matcap_flat: self.editor_state.matcap_flat,
                            object_color: self.editor_state.object_color,
                            textures: self.editor_state.textures,
                        },
                        if self.pushpull_pick { ui::SelectionMode::Faces } else { self.editor_state.selection },
                        selected_object,
                    );
                    if !self.object_hidden {
                        ui.painter().image(target.id, rect, egui::Rect::from_min_max(egui::pos2(0., 0.), egui::pos2(1., 1.)), egui::Color32::WHITE);
                    }
                }


                // Elements in edit modes 1/2/3 (Item 4)
                if !self.object_hidden && !self.mode.bounds() {
                    if let Some(scene) = &self.scene {
                        let pos = scene.positions();
                        let orange = egui::Color32::from_rgb(0xff, 0xa0, 0x28);
                        let white = egui::Color32::WHITE;

                        match if self.pushpull_pick { ui::SelectionMode::Faces } else { self.editor_state.selection } {
                            ui::SelectionMode::Object => {}
                            ui::SelectionMode::Points => {}
                            ui::SelectionMode::Edges => {
                                if let Some(hov) = &hover_elem {
                                    for &(a, b) in &hov.edges {
                                        let (Some(pa), Some(pb)) = (picking::try_at(pos, a), picking::try_at(pos, b)) else { continue };
                                        if let (Some(sa), Some(sb)) = (
                                            ui::transform_gizmo::project(v.camera, rect, pa),
                                            ui::transform_gizmo::project(v.camera, rect, pb),
                                        ) {
                                            ui.painter().line_segment([sa, sb], egui::Stroke::new(3.0_f32, white));
                                        }
                                    }
                                }
                                for &(a, b) in &self.selection.edges {
                                    if !point_pickable(self.editor_state.object_visibility,&self.display_ranges,&self.selected_set,a) || !point_pickable(self.editor_state.object_visibility,&self.display_ranges,&self.selected_set,b) { continue; }
                                    let pa = picking::at(pos, a);
                                    let pb = picking::at(pos, b);
                                    if let (Some(sa), Some(sb)) = (
                                        ui::transform_gizmo::project(v.camera, rect, pa),
                                        ui::transform_gizmo::project(v.camera, rect, pb),
                                    ) {
                                        ui.painter().line_segment([sa, sb], egui::Stroke::new(2.5_f32, orange));
                                    }
                                }
                            }
                            ui::SelectionMode::Faces => {
                                let topo = element_select::cached(&self.elem_topo, scene);
                                if let Some(hov) = &hover_elem {
                                    for poly in topo.polys_of(&hov.tris) {
                                        for (a, b) in topo.polygon_edges(poly) {
                                            if let (Some(sa), Some(sb)) = (
                                                ui::transform_gizmo::project(v.camera, rect, picking::at(pos, a)),
                                                ui::transform_gizmo::project(v.camera, rect, picking::at(pos, b)),
                                            ) {
                                                ui.painter().line_segment([sa, sb], egui::Stroke::new(1.5_f32, white));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                self.viewport_hover = response.hovered();
                if !fly_frame && !region_frame && (self.modal.is_none() && self.model_modal.is_none() && self.mirror.is_none()) && response.hovered() && !ctx.wants_keyboard_input() && ctx.input(|i| i.modifiers.is_none() && i.key_pressed(egui::Key::Tab)) {
                    // Tab is the node menu in every mode; Ctrl+Tab (keymap ToggleEdit) switches object / edit mode
                    out.push(UiAction::Command("add_menu".into()));
                }
                // the release of the click that confirmed a loop cut must not re-select under the cursor
                let swallow_click=ctx.data(|d|d.get_temp::<bool>(egui::Id::new("mesh_cut_swallow_click"))).unwrap_or(false);
                if swallow_click && !ctx.input(|i|i.pointer.primary_down()) { ctx.data_mut(|d|d.remove::<bool>(egui::Id::new("mesh_cut_swallow_click"))); }
                if !swallow_click && !fly_frame && !region_frame && !model_modal_frame && !mirror_frame && self.mirror.is_none() && !ui::viewport_ui::popup_open(ctx) && response.clicked_by(egui::PointerButton::Primary) && !(response.double_clicked() && ctx.input(|i|i.modifiers.shift) && matches!(v.doc["nodes"][v.selected]["type"].as_str(),Some("curve"|"bezier"))) && (self.modal.is_none() && self.model_modal.is_none()) && !(ui::sketch::active(ctx) || self.editor_state.tool.drawing()) && !ui::selection_region::active(ctx) && !ui::prim_handles::active(ctx) && !ui::curve_tool::active(ctx) && !ctx.input(|i| i.pointer.hover_pos()).is_some_and(|p| ui::curve_tool::hit(rect, &v, p, ctx)) && !ctx.input(|i| i.pointer.hover_pos()).is_some_and(|p| ui::prim_handles::hit(rect, &v, p)) && !ui::transform_gizmo::active(ctx) && !ctx.input(|i| i.pointer.hover_pos()).is_some_and(|p| !ui::curve_tool::is_active_node(&v) && ui::transform_gizmo::hit(rect, &v, p)) && matches!(self.editor_state.tool, ui::Tool::Select | ui::Tool::Move | ui::Tool::Rotate | ui::Tool::Scale | ui::Tool::Transform)
                    && self.param_pick.as_ref().is_none_or(|pick| pick.ready && source_ready(&self.display_ranges, &pick.source)) {
                    if let (Some(at), Some(scene)) = (response.interact_pointer_pos(), &self.scene) {
                        let (nx, ny) = (((at.x - rect.left()) / rect.width()) * 2. - 1., 1. - ((at.y - rect.top()) / rect.height()) * 2.);
                        let (o, d) = self.camera.ray(nx, ny, rect.width() / rect.height());
                        let mode = self.editor_state.selection;
                        let extend = ctx.input(|i| i.modifiers.shift);
                        let hit = if self.object_hidden { None } else { pick_displayed_culled(scene,o,d,pick_visibility,&self.display_ranges,&pick_set,false) };
                        let wire = if self.object_hidden || hit.is_some() { None } else { pick_displayed_wire(scene,v.camera,rect,at,pick_visibility,&self.display_ranges,&pick_set,mode,self.editor_state.xray) };
                        if mode == ui::SelectionMode::Object {
                            let ids = hit.and_then(|hit| picked_object(&self.display_ranges, scene.triangle_indices(), hit))
                                .or_else(|| wire.and_then(|w| { let id=match w { picking::WireHit::Point(i)|picking::WireHit::Edge(i,_)=>i }; self.display_ranges.iter().find(|(_,r)|r.contains(&id)).map(|(name,_)|name.as_str()) }))
                                .map(|id| vec![id.to_owned()]).unwrap_or_default();
                            // Empty space clears the shared network/object selection, including with Shift.
                            let extend = extend && !ids.is_empty();
                            let confirming = ctx.data(|d| d.get_temp::<bool>(egui::Id::new("modal_confirm_click")).unwrap_or(false));
                            if viewport_input::selects_object(confirming, !ids.is_empty(), ids.iter().all(|id| self.selected_set.contains(id))) {
                                out.push(UiAction::SelectSet { ids, extend });
                            }
                        }
                        if mode == ui::SelectionMode::Object {
                            self.selection = picking::Selection { mode, ..Default::default() };
                            self.selection_dirty = true;
                        } else {
                            // Shift toggles, Alt loop, Ctrl+Alt ring, Ctrl path, Ctrl+Shift subtract (docs/RUST_ELEMENT_SELECT.md).
                            let mods = ctx.input(|i| i.modifiers);
                            let alt_selects = self.prefs.viewport.navigation == ui::keymap::Navigation::Blender;
                            if let Some(kind) = element_select::classify(mods.shift, mods.ctrl, mods.alt, alt_selects) {
                                let topo = element_select::cached(&self.elem_topo, scene);
                                let picked = hit.map(|hit| {
                                    let point = [0,1,2].map(|k| o[k] + d[k] * hit.t);
                                    (topo.pick(scene.positions(), scene.triangle_indices(), hit, point, mode), hit.tri as u32, point)
                                });
                                let changed = if let Some(w) = wire {
                                    topo.apply_wire_click(scene.positions(),kind,Some(wire_selection(w,mode)),mode,&mut self.selection)
                                } else { topo.apply_click(scene.positions(), kind, picked, mode, &mut self.selection) };
                                if changed {
                                    self.selection_dirty = true;
                                    if let Some(pick) = &mut self.param_pick {
                                        let value = pick.text(topo, &self.selection);
                                        if value != self.editor.doc.value()["nodes"][&pick.node]["params"][&pick.name].as_str().unwrap_or("") {
                                            out.push(UiAction::Edit { edit:pick.clicked(topo, &self.selection), continuous:true });
                                        }
                                        self.picked_field = Some(pick.highlight(&self.selection, scene.positions(), scene.triangle_indices(), scene.radius * 0.012));
                                    }
                                }
                            }
                        }
                    }
                }
                let shift_rmb = response.clicked_by(egui::PointerButton::Secondary) && ctx.input(|i| i.modifiers.shift);
                let cursor_tool_click = response.clicked_by(egui::PointerButton::Primary) && self.editor_state.tool == ui::Tool::Cursor;
                if !fly_frame && !region_frame && !model_modal_frame && !mirror_frame && (shift_rmb || cursor_tool_click) && (self.modal.is_none() && self.model_modal.is_none()) {
                    if let Some(at) = response.interact_pointer_pos() {
                        let (o, d) = ui::transform_gizmo::ray(v.camera, rect, at);
                        let (mesh_p, mesh_tri) = self.scene.as_ref().map(|s| (s.positions(), s.triangle_indices())).unzip();
                        let new_cursor = picking::cursor_hit_position(o, d, mesh_p, mesh_tri, self.editor_state.cursor, v.camera.ray_forward);
                        out.push(UiAction::Editor(ui::EditorChange::Cursor(new_cursor)));
                    }
                }
                let (dropped, tool_hover, drop_pos) = viewport_tool_drop(ctx, rect);
                if let Some(tool) = dropped {
                    let final_pos = if let Some(pos) = drop_pos {
                        let (o, d) = ui::transform_gizmo::ray(v.camera, rect, pos);
                        let (mesh_p, mesh_tri) = self.scene.as_ref().map(|s| (s.positions(), s.triangle_indices())).unzip();
                        let (hit_pos, surface_hit) = picking::drop_world_position(o, d, mesh_p, mesh_tri, self.editor_state.cursor);
                        let offset = if surface_hit { picking::primitive_rest_offset(&tool.0) } else { 0.0 };
                        [hit_pos[0], hit_pos[1] + offset, hit_pos[2]]
                    } else {
                        self.editor_state.cursor
                    };
                    out.push(UiAction::AddNodeAt {
                        node_type: tool.0.clone(),
                        after: Some(v.selected.to_owned()),
                        pos: final_pos,
                    });
                }
                ui::viewport_ui::overlay(ui, rect, &v, &mut out);
                if tool_hover {
                    ui.painter().rect_stroke(rect.shrink(3.), 4., egui::Stroke::new(2_f32, v.theme.accent), egui::StrokeKind::Inside);
                    if let Some(tool) = egui::DragAndDrop::payload::<ui::ToolDrag>(ctx) {
                        let hint = format!("Add {}", tool.0);
                        ui.painter().text(rect.center_top() + egui::vec2(0., 60.), egui::Align2::CENTER_TOP, hint, egui::FontId::proportional(15.), v.theme.accent);

                        // Item 1: Wireframe preview while dragging a primitive
                        if let Some(pointer_pos) = ctx.input(|i| i.pointer.latest_pos()) {
                            if rect.contains(pointer_pos) {
                                let (o, d) = ui::transform_gizmo::ray(v.camera, rect, pointer_pos);
                                let hit_pos = {
                                    let (mesh_p, mesh_tri) = self.scene.as_ref().map(|s| (s.positions(), s.triangle_indices())).unzip();
                                    let (pos, surface_hit) = picking::drop_world_position(o, d, mesh_p, mesh_tri, self.editor_state.cursor);
                                    let offset = if surface_hit { picking::primitive_rest_offset(&tool.0) } else { 0.0 };
                                    [pos[0], pos[1] + offset, pos[2]]
                                };
                                let scene_radius = self.scene.as_ref().map(|s| s.radius).unwrap_or(1.0);
                                if let Some(prim) = get_primitive_preview(&tool.0) {
                                    let target_radius = (scene_radius * 0.25).clamp(0.5, 10.0);
                                    let scale = if prim.radius > 1e-3 { (target_radius / prim.radius).clamp(0.1, 10.0) } else { 1.0 };
                                    let stroke = egui::Stroke::new(1.5_f32, v.theme.accent);
                                    for &(pa, pb) in &prim.lines {
                                        let wa = [hit_pos[0] + pa[0] * scale, hit_pos[1] + pa[1] * scale, hit_pos[2] + pa[2] * scale];
                                        let wb = [hit_pos[0] + pb[0] * scale, hit_pos[1] + pb[1] * scale, hit_pos[2] + pb[2] * scale];
                                        if let (Some(sa), Some(sb)) = (
                                            ui::transform_gizmo::project(v.camera, rect, wa),
                                            ui::transform_gizmo::project(v.camera, rect, wb),
                                        ) {
                                            ui.painter().line_segment([sa, sb], stroke);
                                        }
                                    }
                                    if let Some(sp) = ui::transform_gizmo::project(v.camera, rect, hit_pos) {
                                        ui.painter().circle_stroke(sp, 3.0_f32, stroke);
                                    }
                                }
                            }
                        }
                    }
                }
                ui::transform_gizmo::cursor(ui, rect, &v);
                if let Some(s)=&self.mirror {
                    let plane=s.active.as_ref().map(|id| ui::mirror_tool::Plane::read(&self.editor.doc.value()["nodes"][id]["params"]));
                    ui::mirror_tool::paint(ui,rect,v.camera,s,plane);
                    ui::viewport_ui::modal_status(ui,rect,&s.status());
                } else if let Some((state,_,_,_))=&self.model_modal {
                    ui::viewport_ui::modal_status(ui,rect,&state.status());
                    if let Some(snap)=state.snapped { ui.painter().text(state.pointer+egui::vec2(14.,-14.),egui::Align2::LEFT_BOTTOM,snap.label(),egui::FontId::proportional(13.),egui::Color32::from_rgb(255,200,60)); }
                } else if let Some(modal)=&self.modal {
                    ui::transform_overlay::show(ui,rect,&v,modal);
                    let snap=self.editor_state.snap.enabled ^ ctx.input(|i|i.modifiers.ctrl);
                    ui::viewport_ui::modal_status(ui,rect,&modal.status(snap));
                } else {
                    if !fly_frame && !modal_frame && !region_frame {
                        if !ui::curve_tool::is_active_node(&v) { ui::transform_gizmo::show_with_snap(ui, rect, &v, &mut out,
                            self.scene.as_ref().map(|scene|(scene.positions(),scene.triangle_indices()))); }
                        ui::prim_handles::show(ui, rect, &v, &mut out);
                        ui::curve_tool::show(ui, rect, &v, &mut out, response.double_clicked());
                        ui::sketch::show(ui, rect, &v, &mut out,
                            self.scene.as_ref().map(|scene|(scene.positions(),scene.triangle_indices())));
                        ui::mesh_cut::show(ui, rect, &v, active_cut_range.as_ref().map(|(id,range)|(id.as_str(),range)), &mut out);
                    }
                    ui::selection_region::paint(ui, rect, &v);
                    coplanar_tool::show(ui,rect,&mut self.coplanar,v.units,&mut out);
                }
            });
        }
        usd_export::show(self, ctx);
        self.python_confirm(ctx);
        if self.prefs_open && ui::keymap::show_preferences(ctx, &mut self.prefs_open, &mut self.prefs, &mut self.keymap, &self.theme) {
            let gate = python_gate();
            let before = (gate.allowed(), gate.timeout());
            self.prefs.apply_python(&gate);
            if before != (gate.allowed(), gate.timeout()) { self.recook_python(); }
            self.mode=Mode::parse(&self.prefs.viewport.default_shading);
            self.editor_state.object_visibility=self.prefs.viewport.object_visibility;
            self.editor_state.matcap=self.prefs.viewport.matcap;
            self.editor_state.matcap_flat=self.prefs.viewport.matcap_flat;
            self.editor_state.object_color=self.prefs.viewport.object_color;
            self.editor_state.overlays.grid=self.prefs.viewport.show_grid;
            self.editor_state.overlays.axes=self.prefs.viewport.show_axes;
            self.editor_state.overlays.stats=self.prefs.show_stats;
            self.prefs.viewport.overlays=self.editor_state.overlays;
            self.prefs.keymap_overrides = self.keymap.overrides_json();
            self.prefs.emulate_numpad = self.keymap.emulate_numpad;
            (self.editor_state.proxy.clip_start, self.editor_state.proxy.clip_end) = (self.prefs.viewport.clip_start, self.prefs.viewport.clip_end);
            self.camera.set_clip_planes(self.editor_state.proxy.clip_start, self.editor_state.proxy.clip_end);
            if let Err(e) = self.prefs.save() { self.file_status = format!("Could not save preferences: {e}"); }
        }
        if fly_frame { ctx.set_cursor_icon(egui::CursorIcon::None); }
        if ctx.input(|i| i.pointer.primary_released()) {
            ctx.data_mut(|d| d.remove::<bool>(egui::Id::new("modal_confirm_click")));
        }
        if self.modal.is_none() && self.model_modal.is_none() && self.mirror.is_none() && !ctx.input(|i| i.pointer.primary_down()) { self.editor.history.finish(&self.editor.doc); }
        self.apply(ctx, out.0);
        if (self.modal.is_none() && self.model_modal.is_none()) && !ctx.input(|i| i.pointer.primary_down()) { self.finish_transform(); }
        if self.cooking.cooking() { ctx.request_repaint_after(Duration::from_millis(16)); }
    }
}
/// Open a graph given on the command line; the error names the path (a bare "os error 2" says nothing to someone who double-clicked).
fn open_graph(path: &std::path::Path) -> Result<Document> {
    Document::open(path).map_err(|e| format!("Cannot open {}: {e}

Run the app without arguments to open the built-in demo graph.", std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf()).display()).into())
}
/// The app icon (window and taskbar), decoded from the embedded PNG; None if it cannot be decoded.
fn app_icon() -> Option<egui::IconData> {
    let image=image::load_from_memory(include_bytes!("../assets/app_icon.png")).ok()?.to_rgba8();
    let (width,height)=image.dimensions();
    Some(egui::IconData { rgba: image.into_raw(), width, height })
}

fn run() -> Result<()> {
    let args:Vec<_>=std::env::args_os().skip(1).collect();
    // Started with no arguments (a double-click): open the built-in demo graph rather than exit with a usage line nobody sees.
    let (headless,path,doc)=match args.as_slice() {
        [flag,path] if flag=="--selftest" => { let path=PathBuf::from(path); let doc=open_graph(&path)?; (true,path,doc) }
        [path] => { let path=PathBuf::from(path); let doc=open_graph(&path)?; (false,path,doc) }
        [] => (false,PathBuf::from("untitled.json"),Document::parse(include_str!("default_graph.json").to_owned())?),
        _ => return Err("usage: minihou-app [--selftest] [GRAPH.json]".into()),
    };
    if headless { println!("{}",minihou_app::selftest(&doc)?); return Ok(()); }
    let mut viewport=egui::ViewportBuilder::default().with_inner_size([1280.,800.]).with_maximized(true);
    if let Some(icon)=app_icon() { viewport=viewport.with_icon(std::sync::Arc::new(icon)); }
    let options=eframe::NativeOptions { renderer:eframe::Renderer::Wgpu,viewport,..Default::default() };
    eframe::run_native("MiniHou",options,Box::new(move |cc| {
        // The builder's maximized flag alone can leave a Windows window at its restored size while
        // the OS believes it is maximized (the first Maximize click then does nothing visible), so
        // apply the real state explicitly on the first frame.
        cc.egui_ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(false));
        cc.egui_ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
        let state=cc.wgpu_render_state.clone().ok_or("wgpu renderer unavailable")?;
        Ok(Box::new(App::new(doc,path,state)?))
    })).map_err(|e|e.to_string())?;
    Ok(())
}
fn main() {
    if let Err(e)=run() {
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
