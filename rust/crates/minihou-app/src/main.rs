mod picking;
mod element_select;
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

fn point_pickable(visibility:viewport::ObjectVisibility,ranges:&[(String,std::ops::Range<u32>)],selected:&std::collections::BTreeSet<String>,id:u32)->bool {
    visibility==viewport::ObjectVisibility::ShowAll || ranges.iter().any(|(owner,range)| selected.contains(owner) && range.contains(&id))
}
fn pick_displayed(scene:&Scene,o:[f32;3],d:[f32;3],visibility:viewport::ObjectVisibility,ranges:&[(String,std::ops::Range<u32>)],selected:&std::collections::BTreeSet<String>)->Option<picking::Hit> {
    picking::pick_triangle_filtered(scene.positions(),scene.triangle_indices(),o,d,|tri| tri.iter().all(|id| point_pickable(visibility,ranges,selected,*id)))
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
    if let Some(id) = active { return Some(id.to_owned()); }
    let mut displayed = minihou_app::display_nodes(value);
    let nodes = value["nodes"].as_object_mut()?;
    if object_mode && nodes.get(source).and_then(|n| n["type"].as_str()) == Some("transform") { return Some(source.to_owned()); }
    if object_mode {
        if let Some(id) = nodes.iter().find(|(id, n)| displayed.iter().any(|shown| shown == *id) && n["type"] == "transform" && n["inputs"]["geo"] == source).map(|(id, _)| id.clone()) { return Some(id); }
    }
    if nodes.get(source).and_then(|n| n["type"].as_str()) == Some("output") {
        let up = nodes[source]["inputs"]["geo"].as_str().map(str::to_owned);
        if let Some(up) = up.filter(|up| object_mode && nodes.get(up).is_some_and(|n| n["type"] == "transform")) { return Some(up); }
        // Never hang a node off the end of the stream: insert it just before the output.
        let (next, id) = graph_edit::add(value, "transform", Some(source))?;
        *value = next;
        return Some(id);
    }
    let id = (1..).map(|n| format!("transform{n}")).find(|id| !nodes.contains_key(id))?;
    nodes.insert(id.clone(), json!({"type":"transform", "inputs":{"geo":source}}));
    if let Some(index) = displayed.iter().position(|shown| shown == source) { displayed[index] = id.clone(); }
    else { displayed = vec![id.clone()]; }
    value["display"] = if displayed.len() == 1 { json!(displayed[0]) } else { json!(displayed) };
    Some(id)
}
fn toggle_edit_mode(current: ui::SelectionMode, last: &mut ui::SelectionMode) -> ui::SelectionMode {
    if current == ui::SelectionMode::Object { *last } else { *last = current; ui::SelectionMode::Object }
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
    camera: Camera, bounds: ([f32;3], f32), mode: Mode,
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
    model_modal: Option<(ModelState, Document, String, String)>,
    mirror: Option<ui::mirror_tool::Session>,
    pushpull_pick: bool,
    pub last_operator: Option<LastOperator>,
    last_edit_selection: ui::SelectionMode, params_open: bool, object_hidden: bool,
}
impl App {
    /// Finish the current edit before transferring pointer/keyboard ownership to selection.
    fn finish_for_selection(&mut self, ctx: &egui::Context) {
        if let Some(modal) = self.modal.take() { self.finish_modal(modal); }
        self.finish_model_modal();
        self.finish_mirror();
        ui::transform_gizmo::finish_input(ctx);
        ui::prim_handles::finish_input(ctx);
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
        let position = |i: u32| -> Option<[f32; 3]> { if !point_pickable(self.editor_state.object_visibility,&self.display_ranges,&self.selected_set,i) { return None; } let k = i as usize * 3; let a = points.get(k..k + 3)?; Some([a[0], a[1], a[2]]) };
        let project = |p| ui::transform_gizmo::project(camera, self.last_viewport, p);
        let inside = |p| project(p).is_some_and(|q| shape.contains(q));
        let mode = self.editor_state.selection;
        let mut found = picking::Selection { mode, ..Default::default() };
        match mode {
            ui::SelectionMode::Points => { for i in 0..points.len()/3 { if position(i as u32).is_some_and(inside) { found.points.push(i as u32); } } }
            ui::SelectionMode::Edges => {
                let mut edges = std::collections::BTreeSet::new();
                for t in tris.as_chunks::<3>().0 { for (a,b) in [(t[0],t[1]),(t[1],t[2]),(t[2],t[0])] { edges.insert((a.min(b), a.max(b))); } }
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
        if self.selection.mode != mode { self.selection = picking::Selection { mode, ..Default::default() }; }
        match op {
            SelectOp::Subtract => { self.selection.points.retain(|x| !found.points.contains(x)); self.selection.edges.retain(|x| !found.edges.contains(x)); self.selection.tris.retain(|x| !found.tris.contains(x)); }
            SelectOp::Add => { for x in found.points { if !self.selection.points.contains(&x) { self.selection.points.push(x); } } for x in found.edges { if !self.selection.edges.contains(&x) { self.selection.edges.push(x); } } for x in found.tris { if !self.selection.tris.contains(&x) { self.selection.tris.push(x); } } }
            SelectOp::Replace => self.selection = found,
        }
        self.selection_dirty = true;
    }
    fn new(doc: Document, path: PathBuf, state: egui_wgpu::RenderState) -> Result<Self> {
        Self::new_document(doc, path, state, true)
    }
    fn new_document(doc: Document, path: PathBuf, state: egui_wgpu::RenderState, start_link: bool) -> Result<Self> {
        let display = Graph::from_value(doc.value())?.main_output()?;
        let worker = CookWorker::start(&doc, Scene::new)?;
        let mut app = Self { selected:display.clone(), selected_set: std::collections::BTreeSet::from([display.clone()]), display, editor:EditorDocument::new(doc, path), worker,
            cooking:CookState::default(), state,gpu:None,target:None,
            camera:Camera::frame([0.;3],1.),bounds:([0.;3],1.),mode:Mode::Shaded,file_status:String::new(),
            http:http_ui::HttpUi::default(),http_time:(1.,24.),
            theme:Theme::dark(),flags:HashMap::new(),themed:false,editor_state:EditorState::default(),geo:None,blender:BlenderStatus::default(),link:None,blender_job:None,
            prefs:Preferences::default(),keymap:Keymap::blender(),prefs_open:false,viewport_hover:false,network_hover:false,
            display_ranges:Vec::new(),scene:None,shown_level:1.0,geo_rev:u64::MAX,lod_uploaded:false,last_lod_view:None,last_lod_select:Instant::now(),blender_paths:Vec::new(),blender_bar:false,selection:picking::Selection::default(),selection_dirty:false,elem_topo:Default::default(),stashed_selection:picking::Selection::default(),
            clipboard:None,transform_before:None,transform_target:None,transform_delta:None,transform_pivot:None,transform_orientation:None,modal:None,model_modal:None,mirror:None,pushpull_pick:false,last_operator:None,last_viewport:egui::Rect::NOTHING,
            last_edit_selection:ui::SelectionMode::Points,params_open:true,object_hidden:false };
        app.prefs = Preferences::load();
        app.mode=Mode::parse(&app.prefs.viewport.default_shading);
        app.editor_state.object_visibility=app.prefs.viewport.object_visibility;
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
        self.cooking.requested += 1;
        if let Err(e) = self.worker.send(CookRequest { revision:self.cooking.requested, edit }) {
            self.cooking.completed = self.cooking.requested;
            self.cooking.error = Some(e.to_string());
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
        match self.editor.history.edit(&mut self.editor.doc, &edit) {
            Ok(()) => {
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
        // grow / select_path / select_loop / select_ring start from the viewport's element selection
        if let Some(params) = self.selection_node_params(node_type) { merge_params(&mut value, &id, params); }
        let before = ui::network::position_add(ctx, &self.editor.path, self.editor.doc.value(), &mut value, after);
        self.editor.history.finish(&self.editor.doc);
        let original = self.editor.doc.clone();
        if let Some(before) = before { self.editor.doc = before; }
        if self.commit_document(value, &format!("add {node_type}")) {
            ui::network::restore_positions(ctx, &self.editor.path, self.editor.doc.value());
            self.selected = id.clone();
            self.selected_set = std::collections::BTreeSet::from([id.clone()]);
            self.display = id;
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
        element_select::cached(&self.elem_topo, self.scene.as_ref()?).node_params(node_type, &self.selection)
    }
    fn selected_points(&self) -> Vec<u32> {
        selected_point_ids(&self.selection, self.editor_state.selection,
            self.scene.as_ref().map_or(&[], Scene::triangle_indices)).into_iter().filter(|id| point_pickable(self.editor_state.object_visibility,&self.display_ranges,&self.selected_set,*id)).collect()
    }
    fn set_display_mode(&mut self,mode:Mode) {
        self.mode=mode;
        self.prefs.viewport.default_shading=mode.label().into();
        if let Err(e)=self.prefs.save() { self.file_status=format!("Could not save preferences: {e}"); }
    }
    fn active_object(&self) -> Option<&str> {
        (!self.object_hidden && self.selected_set.contains(&self.selected)
            && self.editor.doc.display_nodes().contains(&self.selected)).then_some(self.selected.as_str())
    }
    fn selection_center(&self) -> Option<[f32; 3]> {
        if self.editor_state.selection == ui::SelectionMode::Object {
            return self.active_object().map(|id| object_origin(self.editor.doc.value(), id));
        }
        let scene = self.scene.as_ref()?; let points = scene.positions(); let ids = self.selected_points();
        if ids.is_empty() { return None; }
        let mut sum = [0.; 3]; let mut count = 0.;
        for id in ids { let k = id as usize * 3; if let Some(p) = points.get(k..k + 3) { for a in 0..3 { sum[a] += p[a]; } count += 1.; } }
        (count > 0.).then(|| sum.map(|x| x / count))
    }
    fn selection_normal_frame(&self, trs: &transform_math::TrsParams) -> transform_math::Frame {
        let local = transform_math::Frame::from_rotation(&trs.world_rotation());
        let Some(scene) = &self.scene else { return local; };
        let ids = self.selected_points();
        if ids.is_empty() { return local; }
        let mut normal = [0.0_f64; 3];
        for id in ids {
            if let Some(n) = scene.normals().get(id as usize * 3..id as usize * 3 + 3) {
                for i in 0..3 { normal[i] += n[i] as f64; }
            }
        }
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
        let group = group_text(&self.selected_points());
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
        let displayed = if self.editor_state.selection == ui::SelectionMode::Object { &self.selected } else { &self.display };
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
        // The active object's points when one is selected among several displayed objects, else every pickable point.
        let owner = self.active_object().map(str::to_owned);
        let ranges: Vec<_> = self.display_ranges.iter().filter(|(o, _)| owner.as_deref().is_none_or(|w| w == o)).map(|r| r.1.clone()).collect();
        let point = |i: usize| [positions[3 * i], positions[3 * i + 1], positions[3 * i + 2]];
        let in_object = |i: u32| ranges.is_empty() || ranges.iter().any(|r| r.contains(&i));
        let ids: Vec<usize> = (0..positions.len() / 3).filter(|&i| in_object(i as u32)
            && point_pickable(self.editor_state.object_visibility, &self.display_ranges, &self.selected_set, i as u32)).collect();
        let Some(bounds) = mt::Bounds::of(ids.iter().map(|&i| point(i))).or_else(|| mt::Bounds::of((0..positions.len() / 3).map(point))) else {
            self.file_status = "Nothing to mirror".into(); return;
        };
        // Edit mode with selected faces: Mirror Selected on those primitives.
        let extra = if self.editor_state.selection != ui::SelectionMode::Object && !self.selection.tris.is_empty() {
            let faces = element_select::cached(&self.elem_topo, scene).polys_of(&self.selection.tris);
            json!({"operation": "mirror_selected", "group": group_text(&faces)})
        } else { json!({}) };
        let wire: Vec<_> = scene.edge_indices().chunks_exact(2).filter(|e| in_object(e[0])).take(mt::WIRE_LIMIT)
            .map(|e| (point(e[0] as usize), point(e[1] as usize))).collect();
        let source = if self.selected.is_empty() { self.display.clone() } else { self.selected.clone() };
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
            let (scene, visibility, ranges, selected) = (self.scene.as_ref(), self.editor_state.object_visibility, &self.display_ranges, &self.selected_set);
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
        let mut selection=self.selection.clone();
        selection.mode=match self.editor_state.selection { ui::SelectionMode::Points => ui::SelectionMode::Points,
            ui::SelectionMode::Edges => ui::SelectionMode::Edges,
            ui::SelectionMode::Faces => ui::SelectionMode::Faces,
            ui::SelectionMode::Object => { self.file_status="Select mesh elements for this tool".into(); return; } };
        let params=if kind==ModelKind::Bevel {
            if selection.mode!=ui::SelectionMode::Edges {
                selection=element_select::cached(&self.elem_topo,scene).convert(&selection,ui::SelectionMode::Edges);
            }
            if selection.edges.is_empty() { self.file_status="Select edges to bevel".into(); return; }
            json!({"edges":selection.edges.iter().map(|(a,b)|format!("{a}-{b}")).collect::<Vec<_>>().join(" "),"distance":0.,"divisions":1})
        } else if kind==ModelKind::Extrude && selection.mode!=ui::SelectionMode::Faces {
            let mut params=match selection.mode {
                ui::SelectionMode::Edges => json!({"edges":selection.edges.iter().map(|(a,b)|format!("{a}-{b}")).collect::<Vec<_>>().join(" "),"distance":0.}),
                ui::SelectionMode::Points => json!({"points":group_text(&selection.points),"distance":0.}),
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
            let mut params=json!({"group":group_text(&faces)});
            params[kind.parameter()]=json!(0.);
            params
        };
        let source=if self.selected.is_empty() { self.display.clone() } else { self.selected.clone() };
        let Some((doc,id))=ui::modal_model::insert_graph(&self.editor.doc,&source,kind,params) else {
            self.file_status=format!("Could not add {}",kind.node()); return;
        };
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
        if let Some((_,before,_,_))=self.model_modal.take() { self.editor.history.record(before,&self.editor.doc); }
    }
    fn cancel_model_modal(&mut self) {
        if let Some((_,before,_,previous))=self.model_modal.take() {
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
            && minihou_core::nodes::get("match_size").is_some()
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
                value["nodes"][target]["type"]=json!("match_size");
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
        let mut confirm=false; let mut cancel=false;
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
                        egui::Key::X | egui::Key::Y | egui::Key::Z => modal.toggle_axis(match key { egui::Key::X=>0,egui::Key::Y=>1,_=>2 },m.shift),
                        egui::Key::O => modal.proportional = !modal.proportional,
                        egui::Key::PageUp => modal.wheel(1.), egui::Key::PageDown => modal.wheel(-1.),
                        egui::Key::B if modal.kind==ModalKind::Move => { modal.cycle_snap(); if modal.reference.is_none() { modal.reference=Some(modal.pivot); } },
                        egui::Key::B if modal.kind==ModalKind::Rotate => { modal.sketch=SketchStage::Pivot; modal.sketch_reference=None; },
                        egui::Key::R if modal.kind==ModalKind::Rotate => modal.trackball = !modal.trackball,
                        egui::Key::C if modal.kind==ModalKind::Scale => { modal.cage = !modal.cage; modal.cage_handle=None; },
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
                    if let Some(start)=modal.mmb_start.take() { modal.constraint=Constraint::Axis(ui::modal_transform::closest_axis(pos-start,frame,camera),false); }
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
    fn sketch_commit(&mut self, name: &str, points: &[[f32; 3]], closed: bool) {
        let Some((value, id)) = graph_edit::add_sketch(self.editor.doc.value(), name, points, closed, &self.selected) else {
            self.file_status = "Could not add sketch".into(); return;
        };
        if self.commit_document(value, "add sketch") { self.selected = id.clone(); self.selected_set = std::collections::BTreeSet::from([id.clone()]); self.display = id; }
    }
    fn add_measurement(&mut self, a: [f32; 3], b: [f32; 3]) {
        let mut value = self.editor.doc.value().clone();
        if !value["measurements"].is_array() { value["measurements"] = json!([]); }
        if let Some(rows) = value["measurements"].as_array_mut() { rows.push(json!({"a":a,"b":b})); }
        self.commit_document(value, "add measurement");
    }
    /// Make `value` the document as one undo step and send it to the cook worker; false (with a status message) when it is invalid.
    fn commit_document(&mut self, value: serde_json::Value, what: &str) -> bool {
        match graph_edit::validated(value) {
            Ok(new) => {
                self.editor.history.replace(&mut self.editor.doc, new);
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
                ui::network::tidy(ctx, &self.editor.path);
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
            "select_none" => { self.selection.clear(); self.selection_dirty = true; self.selected_set.clear(); self.selected.clear(); }
            "select_all" => {
                if self.network_hover {
                    self.selected_set = self.editor.doc.value()["nodes"].as_object().map(|n| n.keys().cloned().collect()).unwrap_or_default();
                    self.selected = self.selected_set.iter().next().cloned().unwrap_or_default();
                    return;
                }
                if let Some(scene) = &self.scene {
                    match e.selection {
                        ui::SelectionMode::Points => self.selection.points = (0..scene.positions().len() / 3).map(|i| i as u32).filter(|id| point_pickable(e.object_visibility,&self.display_ranges,&self.selected_set,*id)).collect(),
                        ui::SelectionMode::Faces => self.selection.tris = scene.triangle_indices().chunks_exact(3).enumerate().filter(|(_,tri)| tri.iter().all(|id| point_pickable(e.object_visibility,&self.display_ranges,&self.selected_set,*id))).map(|(i,_)| i as u32).collect(),
                        ui::SelectionMode::Edges => {
                            // polygon (and wire) edges, without fan diagonals
                            let edges = element_select::cached(&self.elem_topo, scene).edges.clone();
                            self.selection.edges = edges.into_iter().filter(|(a,b)| point_pickable(e.object_visibility,&self.display_ranges,&self.selected_set,*a) && point_pickable(e.object_visibility,&self.display_ranges,&self.selected_set,*b)).collect();
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
                if self.remove_selected_wires(ctx, false) { return; }
                if self.viewport_hover && self.editor_state.selection != ui::SelectionMode::Object {
                    self.file_status = "Component deletion is not supported by the Rust graph yet".into();
                } else if let Some(id) = self.selected_set.iter().next().cloned() { self.delete_node(&id); }
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
            "extrude" | "inset" | "bevel" | "pushpull" => {
                let kind=match name { "extrude"=>ModelKind::Extrude,"inset"=>ModelKind::Inset,
                    "bevel"=>ModelKind::Bevel,_=>ModelKind::PushPull };
                if kind==ModelKind::PushPull {
                    if self.scene.is_none() { self.file_status="Wait for the mesh to cook".into(); return; }
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
        for action in actions {
            if matches!(&action,UiAction::Select(_) | UiAction::ToggleSelect(_) | UiAction::SelectSet { .. }) { self.selection_dirty=true; }
            match action {
                UiAction::StartSketch(mode) => self.start_sketch(ctx, mode),
                UiAction::TransformNode { op } => self.transform_node(op),
                UiAction::StartModal(kind) => self.start_modal(ctx, kind),
                UiAction::FinishTransform => self.finish_transform(),
                UiAction::CancelTransform => self.cancel_transform(),
                UiAction::MeshCutCommit { source, node_type, params } => {
                    if let Some((value, id)) = graph_edit::add_viewport_cut(self.editor.doc.value(), &source, &node_type, params) {
                        if self.commit_document(value, "viewport cut") {
                            self.selected = id.clone(); self.selected_set = std::collections::BTreeSet::from([id.clone()]);
                            self.display = Graph::from_value(self.editor.doc.value()).ok().and_then(|g| g.main_output().ok()).unwrap_or(id);
                        }
                    } else { self.file_status = "Cut target changed; try the cut again".into(); }
                }
                UiAction::SketchCommit { name, points, closed } => self.sketch_commit(&name, &points, closed),
                UiAction::AddMeasurement { a, b } => self.add_measurement(a, b),
                UiAction::RegionSelect { shape, op } => self.region_select(shape, op),
                UiAction::RestoreRegionSelection(snapshot) => {
                    self.selection = snapshot.selection; self.selected = snapshot.selected;
                    self.selected_set = snapshot.selected_set; self.selection_dirty = true;
                }
                UiAction::Open => if let Some(path) = rfd::FileDialog::new().add_filter("MiniHou graph", &["json"]).pick_file() { self.open(path); },
                UiAction::Save => self.save_to(self.editor.path.clone()),
                UiAction::SaveAs => {
                    if let Some(path) = rfd::FileDialog::new().add_filter("MiniHou graph", &["json"]).set_file_name(self.editor.path.file_name().unwrap_or_default().to_string_lossy()).save_file() { self.save_to(path); }
                }
                UiAction::Undo | UiAction::Redo => {
                    let undo = matches!(action, UiAction::Undo);
                    let previous = self.editor.doc.value().clone();
                    let command = if undo { self.editor.history.undo(&mut self.editor.doc) } else { self.editor.history.redo(&mut self.editor.doc) };
                    if let Some(command) = command {
                        if ui::network::layout_changed(&previous, self.editor.doc.value()) {
                            ui::network::restore_positions(ctx, &self.editor.path, self.editor.doc.value());
                        }
                        self.display = Graph::from_value(self.editor.doc.value()).ok().and_then(|g| g.main_output().ok()).unwrap_or_default();
                        self.selected_set.retain(|id| self.editor.doc.value()["nodes"].get(id).is_some());
                        self.send(command);
                    }
                }
                UiAction::Frame => self.frame_mesh(),
                UiAction::FrameSelected => self.frame_selected(),
                UiAction::FrameNetwork => ui::network::frame(ctx, &self.editor.path),
                UiAction::Select(id) => select_active_nodes(&mut self.selected, &mut self.selected_set, vec![id], false, false),
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
                    self.edit(edit, continuous);
                }
                UiAction::Time(edit) => self.send_timeline_time(edit),
                UiAction::AddNode { node_type, .. } if node_type.starts_with("sketch_") => {
                    if let Some(mode) = ui::sketch::SketchMode::DRAW.into_iter().find(|m| m.command_id() == node_type) { self.start_sketch(ctx, mode); }
                }
                UiAction::AddNode { node_type, after } => self.add_node(ctx, &node_type, after.as_deref()),
                UiAction::AddNetworkNode { node_type, pos, wire } => {
                    if let Some((mut value, id)) = graph_edit::add(self.editor.doc.value(), &node_type, None) {
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
                        } else { self.editor.doc = original; }
                    } else { self.file_status = format!("The Rust core has no {node_type} node yet"); }
                }
                UiAction::AddConnectedNode { node_type, connection, pos } => {
                    if let Some((value, id)) = graph_edit::add_connected(self.editor.doc.value(), &node_type, &connection, pos) {
                        if self.commit_document(value, &format!("add {node_type} and connect")) {
                            ui::network::place_connected(ctx, &self.editor.path, &id, pos);
                            self.selected = id.clone();
                            self.selected_set = std::collections::BTreeSet::from([id.clone()]);
                            self.display = Graph::from_value(self.editor.doc.value()).ok().and_then(|g| g.main_output().ok()).unwrap_or(id);
                        }
                    }
                }
                UiAction::AddNodeAt { node_type, after, pos } => self.add_node_at(ctx, &node_type, after.as_deref(), pos),
                UiAction::SetMode(mode) => self.set_display_mode(mode),
                UiAction::Editor(change) => {
                    if matches!(change, ui::EditorChange::Tool(_)) { self.finish_for_selection(ctx); }
                    if let ui::EditorChange::Selection(mode) = change { if mode != ui::SelectionMode::Object { self.last_edit_selection = mode; } }
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
                    ui::network::tidy(ctx, &self.editor.path);
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
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(self.editor.title()));
    }
}
impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
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
        for request in requests {
            let result: Result<()> = match request.message["type"].as_str() {
                Some("live_activate" | "live_update" | "live_remove") => {
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
        if let Some(link) = &self.link {
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
            if let Some(scene) = self.cooking.accept(response) {
                self.display_ranges = display_ranges;
                self.selection_dirty = true;
                let first = self.gpu.is_none();
                self.bounds = (scene.center, scene.radius);
                let level = self.proxy_level(scene.triangles());
                let use_lod = self.editor_state.proxy.clustered && self.cooking.lod.is_some();
                if use_lod {
                    if let Some(gpu) = &mut self.gpu { gpu.upload_lod(&self.state.device, &scene); }
                    else { self.gpu = Some(Gpu::new_lod(&self.state.device, &scene)); }
                } else {
                    let shown = scene.proxied(level);
                    if let Some(gpu) = &mut self.gpu { gpu.upload(&self.state.device, &shown); }
                    else { self.gpu = Some(Gpu::new(&self.state.device, &shown)); }
                }
                self.lod_uploaded = use_lod;
                self.last_lod_view = None;
                self.shown_level = level;
                self.scene = Some(scene);
                self.elem_topo = Default::default();
                if first { self.frame_mesh(); }
            }
        }
        if self.cooking.cooking() { ctx.request_repaint_after(Duration::from_millis(16)); }
        // the proxy level changed (menu, auto budget): re-upload the reduced scene
        if let Some(scene) = &self.scene {
            let level = self.proxy_level(scene.triangles());
            let use_lod = self.editor_state.proxy.clustered && self.cooking.lod.is_some();
            if use_lod != self.lod_uploaded {
                if let Some(gpu) = &mut self.gpu {
                    if use_lod { gpu.upload_lod(&self.state.device, scene); }
                    else { gpu.upload(&self.state.device, &scene.proxied(level)); }
                }
                self.lod_uploaded = use_lod;
                self.last_lod_view = None;
                self.shown_level = level;
            } else if !use_lod && (level - self.shown_level).abs() > 1e-6 {
                if let Some(gpu) = &mut self.gpu { gpu.upload(&self.state.device, &scene.proxied(level)); }
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
        }
        if self.selection_dirty {
            if let (Some(gpu), Some(scene)) = (&mut self.gpu, &self.scene) {
                let size = scene.radius * 0.012;
                let mut selection=self.selection.clone();
                let pickable=|id| point_pickable(self.editor_state.object_visibility,&self.display_ranges,&self.selected_set,id);
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
        let busy = self.modal.is_some() || self.model_modal.is_some() || self.mirror.is_some() || self.pushpull_pick || (ui::sketch::active(ctx) || self.editor_state.tool.drawing()) || ui::transform_gizmo::active(ctx) || ui::prim_handles::active(ctx);
        let fly_frame = viewport_input::fly(ctx, self.last_viewport, &mut self.camera,
            self.prefs.viewport.fly_navigation && !busy && !ui::selection_region::active(ctx) && !ctx.wants_keyboard_input());
        if busy && !fly_frame && !ui::sketch::active(ctx) && !self.editor_state.tool.drawing() {
            let selection_actions = self.keymap.poll_selection(ctx, &KeyContext {
                viewport_hovered: self.viewport_hover, network_hovered: false,
                in_edit_mode: self.editor_state.selection != ui::SelectionMode::Object, selected: Some(&self.selected) });
            let select_press = self.editor_state.tool == ui::Tool::Select && ctx.input(|i| i.pointer.primary_pressed()
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
                        if let Some(hit)=pick_displayed(scene,o,d,self.editor_state.object_visibility,&self.display_ranges,&self.selected_set) {
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
        {
            let repeat_desc = self.last_operator.as_ref().map(LastOperator::description);
            let transform_node = if self.editor_state.selection == ui::SelectionMode::Object { &self.selected } else { &self.display };
            let display_trs = transform_math::TrsParams::from_params(&self.editor.doc.value()["nodes"][transform_node]["params"]);
            let normal_frame = self.selection_normal_frame(&display_trs).axes.map(|axis| axis.map(|x| x as f32));
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
                let region_frame = if !fly_frame && (self.modal.is_none() && self.model_modal.is_none() && self.mirror.is_none()) && !mirror_frame && !(ui::sketch::active(ctx) || self.editor_state.tool.drawing()) {
                    ui::selection_region::handle(ctx, rect, &v, &mut out)
                } else { false };
                let mut delta = ctx.input(|i| i.pointer.delta());
                if !fly_frame && !region_frame && response.dragged() && (self.modal.is_none() && self.model_modal.is_none()) && !ui::sketch::blocks_navigation(ctx) && !ui::mesh_cut::blocks_navigation(ctx) && !ui::transform_gizmo::active(ctx) && !ui::selection_region::active(ctx) && !ui::prim_handles::active(ctx)
                    && !ctx.input(|i| i.pointer.hover_pos()).is_some_and(|p| ui::transform_gizmo::hit(rect, &v, p) || ui::prim_handles::hit(rect, &v, p)) {
                    let modifiers = ctx.input(|i| i.modifiers);
                    wrap_drag(ctx, rect, &mut delta);
                    match self.prefs.viewport.navigation {
                        ui::keymap::Navigation::Blender if response.dragged_by(egui::PointerButton::Middle) => {
                            if modifiers.ctrl { self.camera.zoom(delta.y); }
                            else if modifiers.shift { self.camera.pan(delta.x, delta.y); }
                            else { self.camera.orbit(delta.x, delta.y); }
                        }
                        ui::keymap::Navigation::Houdini if modifiers.alt => {
                            if response.dragged_by(egui::PointerButton::Primary) { self.camera.orbit(delta.x, delta.y); }
                            else if response.dragged_by(egui::PointerButton::Middle) { self.camera.pan(delta.x, delta.y); }
                            else if response.dragged_by(egui::PointerButton::Secondary) { self.camera.zoom(delta.y); }
                        }
                        _ => {}
                    }
                }
                if !fly_frame && !region_frame && response.hovered() && (self.modal.is_none() && self.model_modal.is_none())
                    && self.editor_state.tool != ui::Tool::LoopCut && !ui::mesh_cut::captures_wheel(ctx) {
                    self.camera.zoom(ctx.input(|i| i.smooth_scroll_delta.y));
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
                    gpu.select_points(&self.state.device,scene,&self.selection.points);
                    gpu.select_objects(&self.state.device, scene, ranges);
                    gpu.set_object_ranges(&self.state.device,scene,self.display_ranges.iter().map(|(_,r)| r.clone()).collect());
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
                        },
                        self.editor_state.selection,
                        selected_object,
                    );
                    if !self.object_hidden {
                        ui.painter().image(target.id, rect, egui::Rect::from_min_max(egui::pos2(0., 0.), egui::pos2(1., 1.)), egui::Color32::WHITE);
                    }
                }


                // Hover pre-selection highlight (Item 4)
                let hover_pos = ctx.input(|i| i.pointer.latest_pos());
                let hover_in_viewport = hover_pos.is_some_and(|p| rect.contains(p))
                    && !(ui::sketch::active(ctx) || self.editor_state.tool.drawing())
                    && !ui::selection_region::active(ctx);

                let hover_elem = if hover_in_viewport {
                    if let (Some(pos), Some(scene)) = (hover_pos, &self.scene) {
                        let (o, d) = ui::transform_gizmo::ray(v.camera, rect, pos);
                        let mode = if self.pushpull_pick { ui::SelectionMode::Faces } else { self.editor_state.selection };
                        let mods = ctx.input(|i| i.modifiers);
                        pick_displayed(scene,o,d,self.editor_state.object_visibility,&self.display_ranges,&self.selected_set)
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
                            })
                    } else {
                        None
                    }
                } else {
                    None
                };

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
                                        let pa = picking::at(pos, a);
                                        let pb = picking::at(pos, b);
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
                                let center_fill = egui::Color32::from_rgb(0x30, 0x38, 0x48);
                                let center_stroke = egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(0x80, 0x90, 0xa8));
                                let npoly = topo.polygons().len();
                                let stride = if npoly > 50_000 { (npoly / 20_000).max(2) } else { 1 };
                                for poly in topo.polygons().step_by(stride) {
                                    let vertices = topo.polygon_vertices(poly);
                                    if vertices.is_empty() || !vertices.iter().all(|id| point_pickable(self.editor_state.object_visibility,&self.display_ranges,&self.selected_set,*id)) { continue; }
                                    let center = [0, 1, 2].map(|k| vertices.iter().map(|&id| picking::at(pos, id)[k]).sum::<f32>() / vertices.len() as f32);
                                    if let Some(sp) = ui::transform_gizmo::project(v.camera, rect, center) {
                                        let is_sel = topo.poly_tris(poly).iter().any(|id| self.selection.tris.contains(id));
                                        let is_hov = hover_elem.as_ref().is_some_and(|e| topo.poly_tris(poly).iter().any(|id| e.tris.contains(id)));
                                        if is_hov {
                                            ui.painter().circle_filled(sp, 4.0_f32, white);
                                            ui.painter().circle_stroke(sp, 4.0_f32, egui::Stroke::new(1.0_f32, orange));
                                        } else if is_sel {
                                            ui.painter().circle_filled(sp, 3.5_f32, orange);
                                        } else {
                                            ui.painter().circle_filled(sp, 2.0_f32, center_fill);
                                            ui.painter().circle_stroke(sp, 2.0_f32, center_stroke);
                                        }
                                    }
                                }
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
                if !fly_frame && !region_frame && !model_modal_frame && !mirror_frame && self.mirror.is_none() && !ui::viewport_ui::popup_open(ctx) && response.clicked_by(egui::PointerButton::Primary) && (self.modal.is_none() && self.model_modal.is_none()) && !(ui::sketch::active(ctx) || self.editor_state.tool.drawing()) && !ui::selection_region::active(ctx) && !ui::prim_handles::active(ctx) && !ctx.input(|i| i.pointer.hover_pos()).is_some_and(|p| ui::prim_handles::hit(rect, &v, p)) && !ui::transform_gizmo::active(ctx) && !ctx.input(|i| i.pointer.hover_pos()).is_some_and(|p| ui::transform_gizmo::hit(rect, &v, p)) && (self.editor_state.tool == ui::Tool::Select || (self.editor_state.selection == ui::SelectionMode::Object && !matches!(self.editor_state.tool, ui::Tool::Cursor | ui::Tool::Measure | ui::Tool::Line | ui::Tool::Polyline | ui::Tool::Polygon | ui::Tool::Rectangle | ui::Tool::Circle | ui::Tool::Knife | ui::Tool::LoopCut))) {
                    if let (Some(at), Some(scene)) = (response.interact_pointer_pos(), &self.scene) {
                        let (nx, ny) = (((at.x - rect.left()) / rect.width()) * 2. - 1., 1. - ((at.y - rect.top()) / rect.height()) * 2.);
                        let (o, d) = self.camera.ray(nx, ny, rect.width() / rect.height());
                        let mode = self.editor_state.selection;
                        let extend = ctx.input(|i| i.modifiers.shift);
                        let hit = if self.object_hidden { None } else { pick_displayed(scene,o,d,self.editor_state.object_visibility,&self.display_ranges,&self.selected_set) };
                        if mode == ui::SelectionMode::Object {
                            let ids = hit.and_then(|hit| picked_object(&self.display_ranges, scene.triangle_indices(), hit)).map(|id| vec![id.to_owned()]).unwrap_or_default();
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
                                if topo.apply_click(scene.positions(), kind, picked, mode, &mut self.selection) { self.selection_dirty = true; }
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
                } else if let Some(modal)=&self.modal {
                    ui::transform_overlay::show(ui,rect,&v,modal);
                    let snap=self.editor_state.snap.enabled ^ ctx.input(|i|i.modifiers.ctrl);
                    ui::viewport_ui::modal_status(ui,rect,&modal.status(snap));
                } else {
                    if !fly_frame && !modal_frame && !region_frame {
                        ui::transform_gizmo::show_with_snap(ui, rect, &v, &mut out,
                            self.scene.as_ref().map(|scene|(scene.positions(),scene.triangle_indices())));
                        ui::prim_handles::show(ui, rect, &v, &mut out);
                        ui::sketch::show(ui, rect, &v, &mut out);
                        ui::mesh_cut::show(ui, rect, &v, &mut out);
                    }
                    ui::selection_region::paint(ui, rect, &v);
                }
            });
        }
        usd_export::show(self, ctx);
        if self.prefs_open && ui::keymap::show_preferences(ctx, &mut self.prefs_open, &mut self.prefs, &mut self.keymap, &self.theme) {
            self.mode=Mode::parse(&self.prefs.viewport.default_shading);
            self.editor_state.object_visibility=self.prefs.viewport.object_visibility;
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
    let options=eframe::NativeOptions { renderer:eframe::Renderer::Wgpu,viewport:egui::ViewportBuilder::default().with_inner_size([1280.,800.]).with_maximized(true),..Default::default() };
    eframe::run_native("MiniHou",options,Box::new(move |cc| {
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
}



