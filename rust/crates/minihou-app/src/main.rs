mod picking;
mod graph_edit;
mod ui;
mod viewport;
#[cfg(test)] mod transform_tests;
use eframe::{egui, egui_wgpu};
use minihou_app::{Camera, Document, Result, Edit, EditorDocument, CookWorker, CookRequest, CookState, StandardView};
use minihou_core::Graph;
use serde_json::{json, Value};
use std::{collections::HashMap, path::PathBuf, time::{Duration, Instant}};
use ui::keymap::{KeyContext, Keymap, Preferences, ThemePref};
use ui::{theme::Theme, Actions, BlenderStatus, CameraInfo, EditorState, GeoSummary, NodeFlags, TransformOp, UiAction, View};
use ui::modal_transform::{Modal, Kind as ModalKind, SketchStage, Constraint};
use viewport::{Gpu, Mode, Overlay, Scene, Target};

fn scaled_from_initial(initial: &serde_json::Value, factor: [f32; 3]) -> [f32; 3] {
    [0,1,2].map(|i| initial.get(i).and_then(|x| x.as_f64()).unwrap_or(1.) as f32 * factor[i])
}

/// Move and Rotate actions carry per-frame deltas. Scale actions carry a factor
/// relative to the start of the drag, including when the pointer moves back.
fn transform_parameter(value: &Value, initial: &Document, target: &str, op: TransformOp) -> (&'static str, Value) {
    let (name, delta, default) = match op {
        TransformOp::Move(delta) => ("translate", delta, 0.),
        TransformOp::Rotate(delta) => ("rotate", delta, 0.),
        TransformOp::Scale(factor) => ("scale", factor, 1.),
    };
    let current = &value["nodes"][target]["params"][name];
    let next = if name == "scale" {
        scaled_from_initial(&initial.value()["nodes"][target]["params"][name], delta)
    } else {
        [0, 1, 2].map(|i| current.get(i).and_then(Value::as_f64).unwrap_or(default) as f32 + delta[i])
    };
    (name, json!(next))
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

fn viewport_tool_drop(ctx: &egui::Context, rect: egui::Rect) -> (Option<std::sync::Arc<ui::ToolDrag>>, bool) {
    let over = ctx.input(|i| i.pointer.latest_pos().is_some_and(|p| rect.contains(p)));
    let dragging = over && egui::DragAndDrop::has_payload_of_type::<ui::ToolDrag>(ctx);
    let released = ctx.input(|i| i.pointer.any_released());
    (if dragging && released { egui::DragAndDrop::take_payload::<ui::ToolDrag>(ctx) } else { None }, dragging && !released)
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
    theme: Theme, flags: HashMap<String, NodeFlags>, themed: bool,
    editor_state: EditorState,
    geo: Option<GeoSummary>, blender: BlenderStatus,
    prefs: Preferences, keymap: Keymap, prefs_open: bool, viewport_hover: bool, network_hover: bool,
    /// The last cooked scene at full detail, and the proxy level it is currently uploaded at.
    scene: Option<Scene>, shown_level: f32, geo_rev: u64,
    lod_uploaded: bool, last_lod_view: Option<([[f32;4];4], [u32;2], bool)>, last_lod_select: Instant,
    blender_paths: Vec<PathBuf>, blender_bar: bool,
    selection: picking::Selection, selection_dirty: bool,
    clipboard: Option<Value>,
    last_viewport: egui::Rect,
    transform_before: Option<Document>, transform_target: Option<String>,
    modal: Option<Modal>,
    last_edit_selection: ui::SelectionMode, params_open: bool, object_hidden: bool,
}
impl App {
    fn camera_info(&self) -> CameraInfo {
        let (yaw, pitch) = self.camera.angles(); let (projection, _) = self.camera.matrices(1.);
        let (ray_origin, ray_forward) = self.camera.ray(0., 0., 1.);
        let (rx, dx) = self.camera.ray(1., 0., 1.); let (ry, dy) = self.camera.ray(0., 1., 1.);
        let ray_right = if self.camera.is_ortho() { [0,1,2].map(|i| rx[i] - ray_origin[i]) } else { [0,1,2].map(|i| dx[i] - ray_forward[i]) };
        let ray_up = if self.camera.is_ortho() { [0,1,2].map(|i| ry[i] - ray_origin[i]) } else { [0,1,2].map(|i| dy[i] - ray_forward[i]) };
        CameraInfo { yaw, pitch, ortho: self.camera.is_ortho(), projection, ray_origin, ray_forward, ray_right, ray_up }
    }
    fn region_select(&mut self, rect: egui::Rect, extend: bool, subtract: bool) {
        let Some(scene) = &self.scene else { return; };
        let camera = self.camera_info(); let points = scene.positions(); let tris = scene.triangle_indices();
        let position = |i: u32| -> Option<[f32; 3]> { let k = i as usize * 3; let a = points.get(k..k + 3)?; Some([a[0], a[1], a[2]]) };
        let inside = |p: [f32; 3]| ui::transform_gizmo::project(camera, self.last_viewport, p).is_some_and(|q| rect.contains(q));
        let mode = self.editor_state.selection;
        let mut found = picking::Selection { mode, ..Default::default() };
        match mode {
            ui::SelectionMode::Points => { for i in 0..points.len()/3 { if position(i as u32).is_some_and(inside) { found.points.push(i as u32); } } }
            ui::SelectionMode::Edges => {
                let mut edges = std::collections::BTreeSet::new();
                for t in tris.as_chunks::<3>().0 { for (a,b) in [(t[0],t[1]),(t[1],t[2]),(t[2],t[0])] { edges.insert((a.min(b), a.max(b))); } }
                for (a,b) in edges { if let (Some(pa), Some(pb)) = (position(a), position(b)) { if inside([0,1,2].map(|k| (pa[k] + pb[k]) * 0.5)) { found.edges.push((a,b)); } } }
            }
            ui::SelectionMode::Faces => { for (i,t) in tris.as_chunks::<3>().0.iter().enumerate() { if let (Some(a), Some(b), Some(c)) = (position(t[0]), position(t[1]), position(t[2])) { if inside([0,1,2].map(|k| (a[k]+b[k]+c[k])/3.)) { found.tris.push(i as u32); } } } }
            ui::SelectionMode::Object => { if inside(self.bounds.0) { self.selected = self.display.clone(); select_nodes(&mut self.selected_set, [self.display.clone()], extend, false); } }
        }
        if subtract { self.selection.points.retain(|x| !found.points.contains(x)); self.selection.edges.retain(|x| !found.edges.contains(x)); self.selection.tris.retain(|x| !found.tris.contains(x)); }
        else if extend { for x in found.points { if !self.selection.points.contains(&x) { self.selection.points.push(x); } } for x in found.edges { if !self.selection.edges.contains(&x) { self.selection.edges.push(x); } } for x in found.tris { if !self.selection.tris.contains(&x) { self.selection.tris.push(x); } } }
        else { self.selection = found; }
        self.selection_dirty = true;
    }
    fn new(doc: Document, path: PathBuf, state: egui_wgpu::RenderState) -> Result<Self> {
        let display = Graph::from_value(doc.value())?.main_output()?;
        let worker = CookWorker::start(&doc, Scene::new)?;
        let mut app = Self { selected:display.clone(), selected_set: std::collections::BTreeSet::from([display.clone()]), display, editor:EditorDocument::new(doc, path), worker,
            cooking:CookState::default(), state,gpu:None,target:None,
            camera:Camera::frame([0.;3],1.),bounds:([0.;3],1.),mode:Mode::Shaded,file_status:String::new(),
            theme:Theme::dark(),flags:HashMap::new(),themed:false,editor_state:EditorState::default(),geo:None,blender:BlenderStatus::default(),
            prefs:Preferences::default(),keymap:Keymap::blender(),prefs_open:false,viewport_hover:false,network_hover:false,
            scene:None,shown_level:1.0,geo_rev:u64::MAX,lod_uploaded:false,last_lod_view:None,last_lod_select:Instant::now(),blender_paths:Vec::new(),blender_bar:false,selection:picking::Selection::default(),selection_dirty:false,
            clipboard:None,transform_before:None,transform_target:None,modal:None,last_viewport:egui::Rect::NOTHING,
            last_edit_selection:ui::SelectionMode::Points,params_open:true,object_hidden:false };
        app.prefs = Preferences::load();
        app.keymap = Keymap::from_overrides_json(&app.prefs.keymap_overrides);
        app.keymap.emulate_numpad = app.prefs.emulate_numpad;
        if app.prefs.theme == ThemePref::Light { app.theme = Theme::light(); }
        app.editor_state.snap.increment = app.prefs.snap_increment;
        app.editor_state.overlays.grid = app.prefs.viewport.show_grid;
        app.editor_state.overlays.axes = app.prefs.viewport.show_axes;
        app.editor_state.overlays.stats = app.prefs.show_stats;
        app.editor_state.proxy.view_distance = app.prefs.viewport.view_distance;
        app.camera.set_view_distance(app.editor_state.proxy.view_distance);
        let installs = ui::blender_ui::blender_link::find_installs();
        app.blender.installs = installs.iter().map(|(label, path)| (label.clone(), path.display().to_string())).collect();
        app.blender_paths = installs.into_iter().map(|(_, p)| p).collect();
        app.blender.addon_installed = false;
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
    fn edit(&mut self, edit: Edit, continuous: bool) {
        match self.editor.history.edit(&mut self.editor.doc, &edit) {
            Ok(()) => { self.send(edit); if !continuous { self.editor.history.finish(&self.editor.doc); } }
            Err(e) => self.file_status = e.to_string(),
        }
    }
    fn frame_mesh(&mut self) {
        self.camera = Camera::frame(self.bounds.0, self.bounds.1);
        self.camera.set_view_distance(self.editor_state.proxy.view_distance);
    }
    fn frame_selected(&mut self) {
        if let (Some(scene), Some(center)) = (&self.scene, self.selection_center()) {
            let points = scene.positions();
            let radius = self.selected_points().into_iter().filter_map(|id| points.get(id as usize * 3..id as usize * 3 + 3))
                .map(|p| ((0..3).map(|i| (p[i] - center[i]).powi(2)).sum::<f32>()).sqrt())
                .fold(0_f32, f32::max).max(self.bounds.1 * 0.05);
            self.camera = Camera::frame(center, radius);
            self.camera.set_view_distance(self.editor_state.proxy.view_distance);
        } else { self.frame_mesh(); }
    }
    fn save_to(&mut self, path: PathBuf) {
        self.editor.history.finish(&self.editor.doc);
        match self.editor.save_to(path) {
            Ok(()) => { self.file_status = "Saved".into(); }
            Err(e) => self.file_status = format!("Save failed: {e}"),
        }
    }
    fn open(&mut self, path: PathBuf) {
        let replacement = Document::open(&path).and_then(|doc| Self::new(doc, path, self.state.clone()));
        match replacement {
            Ok(mut app) => { app.target = self.target.take(); *self = app; }
            Err(e) => self.file_status = format!("Open failed: {e}"),
        }
    }
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
    fn add_node(&mut self, node_type: &str, after: Option<&str>) {
        let Some((value, id)) = graph_edit::add(self.editor.doc.value(), node_type, after) else {
            self.file_status = format!("The Rust core has no {node_type} node yet");
            return;
        };
        if self.commit_document(value, &format!("add {node_type}")) {
            self.selected = id.clone();
            self.selected_set = std::collections::BTreeSet::from([id.clone()]);
            self.display = id;
        }
    }
    fn selected_points(&self) -> Vec<u32> {
        selected_point_ids(&self.selection, self.editor_state.selection,
            self.scene.as_ref().map_or(&[], Scene::triangle_indices))
    }
    fn selection_center(&self) -> Option<[f32; 3]> {
        let scene = self.scene.as_ref()?; let points = scene.positions(); let ids = self.selected_points();
        if ids.is_empty() { return None; }
        let mut sum = [0.; 3]; let mut count = 0.;
        for id in ids { let k = id as usize * 3; if let Some(p) = points.get(k..k + 3) { for a in 0..3 { sum[a] += p[a]; } count += 1.; } }
        (count > 0.).then(|| sum.map(|x| x / count))
    }
    fn transform_node(&mut self, op: TransformOp) { self.transform_node_at(op, None, None); }
    fn transform_node_at(&mut self, op: TransformOp, pivot: Option<[f32; 3]>, axis_rotation: Option<[f32;3]>) {
        let group = group_text(&self.selected_points());
        if self.editor_state.selection != ui::SelectionMode::Object && group.is_empty() {
            self.file_status = "Select points, edges, or faces to transform".into();
            return;
        }
        let first = self.transform_before.is_none();
        if first { self.editor.history.finish(&self.editor.doc); self.transform_before = Some(self.editor.doc.clone()); }
        let mut value = self.editor.doc.value().clone();
        // The viewport handle is positioned on the displayed geometry. A graph
        // selection may name an unrelated node, so always transform what is shown.
        let source = &self.display;
        let Some(target) = ensure_transform_target(&mut value, source, group.is_empty(), self.transform_target.as_deref()) else { return; };
        let (name, param_value) = transform_parameter(&value, self.transform_before.as_ref().unwrap(), &target, op);
        let params = &mut value["nodes"][&target]["params"];
        if !params.is_object() { *params = json!({}); }
        params[name] = param_value.clone();
        let old_pivot = self.editor.doc.value()["nodes"][&target]["params"]["pivot"].clone();
        let old_axis=self.editor.doc.value()["nodes"][&target]["params"]["axis_rotation"].clone();
        if first {
            params["group"] = json!(group);
            proportional_params(params, &self.editor_state);
            params["pivot"] = json!(pivot.unwrap_or_else(|| if self.editor_state.pivot == ui::Pivot::Cursor { self.editor_state.cursor } else { self.selection_center().unwrap_or(self.bounds.0) }));
            if let Some(rotation)=axis_rotation { params["axis_rotation"]=json!(rotation); }
        }
        match apply_transform_step(&mut self.editor.doc, first, value, &target, name, param_value) {
            Ok(edits) => {
                self.transform_target = Some(target.clone()); self.display = target.clone(); for edit in edits { self.send(edit); }
                if !first {
                    if let Some(pivot) = pivot { let value = json!(pivot); if old_pivot != value {
                        let edit = Edit::Param { node: target.clone(), name: "pivot".into(), value };
                        if self.editor.doc.edit(&edit).is_ok() { self.send(edit); }
                    } }
                    if let Some(rotation)=axis_rotation { let value=json!(rotation); if old_axis!=value {
                        let edit=Edit::Param {node:target, name:"axis_rotation".into(),value};
                        if self.editor.doc.edit(&edit).is_ok() { self.send(edit); }
                    } }
                    let mut desired=self.editor.doc.value()["nodes"][&self.display]["params"].clone();
                    proportional_params(&mut desired,&self.editor_state);
                    for name in ["proportional_enabled","proportional_radius","proportional_falloff","proportional_connected"] {
                        if desired[name] != self.editor.doc.value()["nodes"][&self.display]["params"][name] {
                            let edit=Edit::Param { node:self.display.clone(), name:name.into(), value:desired[name].clone() };
                            if self.editor.doc.edit(&edit).is_ok() { self.send(edit); }
                        }
                    }
                }
            }
            Err(e) => self.file_status = format!("Could not transform node: {e}"),
        }
    }
    fn finish_transform(&mut self) {
        if let Some(before) = self.transform_before.take() {
            self.editor.history.record(before, &self.editor.doc);
        }
        if let Some(target) = self.transform_target.take() { self.selected_set = std::collections::BTreeSet::from([target.clone()]); self.selected = target; }
    }
    fn cancel_transform(&mut self) {
        if let Some(before) = self.transform_before.take() {
            self.editor.doc = before;
            self.send(Edit::Replace(self.editor.doc.value().clone()));
            self.display = Graph::from_value(self.editor.doc.value()).ok().and_then(|g| g.main_output().ok()).unwrap_or_else(|| self.display.clone());
        }
        self.transform_target = None;
    }
    fn start_modal(&mut self, ctx: &egui::Context, kind: ModalKind) {
        if self.modal.is_some() || self.display.is_empty() || self.scene.is_none() { return; }
        if self.editor_state.selection != ui::SelectionMode::Object && self.selected_points().is_empty() {
            self.file_status = "Select points, edges, or faces to transform".into(); return;
        }
        let pointer = ctx.input(|i| i.pointer.hover_pos()).unwrap_or(self.last_viewport.center());
        let scene = self.scene.as_ref().unwrap(); let positions = scene.positions();
        let ids = self.selected_points();
        let points: Vec<[f32;3]> = if ids.is_empty() { positions.as_chunks::<3>().0.to_vec() }
            else { ids.into_iter().filter_map(|id| positions.get(id as usize*3..id as usize*3+3).map(|p|[p[0],p[1],p[2]])).collect() };
        if points.is_empty() { return; }
        let lo = [0,1,2].map(|i|points.iter().map(|p|p[i]).fold(f32::INFINITY,f32::min));
        let hi = [0,1,2].map(|i|points.iter().map(|p|p[i]).fold(f32::NEG_INFINITY,f32::max));
        let pivot = if self.editor_state.pivot == ui::Pivot::Cursor { self.editor_state.cursor }
            else if self.editor_state.pivot == ui::Pivot::BoundingBoxCenter { [0,1,2].map(|i|(lo[i]+hi[i])*0.5) }
            else { self.selection_center().unwrap_or([0,1,2].map(|i|(lo[i]+hi[i])*0.5)) };
        let mut modal=Modal::new(kind,pointer,pivot,(lo,hi),self.editor_state.snap.to,
            self.editor_state.proportional.enabled,self.editor_state.proportional.radius);
        modal.rotation=[0,1,2].map(|i|self.editor.doc.value()["nodes"][&self.display]["params"]["rotate"].get(i)
            .and_then(Value::as_f64).unwrap_or(0.) as f32);
        self.modal=Some(modal);
        ctx.request_repaint();
    }
    fn modal_snap_feature(&self, at: egui::Pos2, snap_to: ui::SnapTo) -> Option<([f32;3],[[f32;3];3])> {
        let scene = self.scene.as_ref()?; let positions = scene.positions(); let triangles = scene.triangle_indices();
        let (origin,dir) = ui::transform_gizmo::ray(self.camera_info(),self.last_viewport,at);
        let hit = picking::pick_triangle(positions,triangles,origin,dir)?;
        let ids = triangles.get(hit.tri*3..hit.tri*3+3)?;
        let point = |id: u32| -> Option<[f32;3]> { let p=positions.get(id as usize*3..id as usize*3+3)?; Some([p[0],p[1],p[2]]) };
        let p = [point(ids[0])?,point(ids[1])?,point(ids[2])?];
        let target=match snap_to {
            ui::SnapTo::Vertex => p[(0..3).max_by(|&a,&b|hit.bary[a].total_cmp(&hit.bary[b])).unwrap_or(0)],
            ui::SnapTo::Edge | ui::SnapTo::EdgeCenter => { let far=(0..3).min_by(|&a,&b|hit.bary[a].total_cmp(&hit.bary[b])).unwrap_or(0); [0,1,2].map(|i|(p[(far+1)%3][i]+p[(far+2)%3][i])*0.5) },
            _ => [0,1,2].map(|i|(p[0][i]+p[1][i]+p[2][i])/3.),
        };
        Some((target,p))
    }
    fn modal_snap_point(&self, at: egui::Pos2, snap_to: ui::SnapTo) -> Option<[f32;3]> {
        self.modal_snap_feature(at,snap_to).map(|(p,_)|p)
    }
    fn finish_modal(&mut self, modal: Modal) {
        if modal.cage && self.editor_state.selection == ui::SelectionMode::Object
            && minihou_core::nodes::get("match_size").is_some()
            && self.editor.doc.display_nodes().len()==1
            && modal.last.iter().all(|v| *v >= 0.) {
            if let Some(target) = &self.transform_target {
                if self.transform_before.as_ref().is_some_and(|before|before.value()["nodes"][target]["type"]=="transform") {
                    self.finish_transform(); return;
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
        self.finish_transform();
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
        let frame=ui::modal_transform::orientation_frame(orientation,camera,modal.rotation);
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
                        let to=if modal.snap_to==ui::SnapTo::Increment {ui::SnapTo::Face} else {modal.snap_to};
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
        let snap=(self.editor_state.snap.enabled ^ modifiers.ctrl)
            || (modal.kind==ModalKind::Move && modal.snap_to!=ui::SnapTo::Increment);
        if modal.kind==ModalKind::Move && snap && modal.snap_to!=ui::SnapTo::Increment {
            let feature=self.modal_snap_feature(modal.pointer,modal.snap_to);
            modal.target=feature.as_ref().map(|(target,_)|*target);
            modal.target_shape=feature.map(|(_,shape)|shape);
        } else { modal.target=None; modal.target_shape=None; }
        let center=ui::transform_gizmo::project(camera,self.last_viewport,modal.pivot).unwrap_or(self.last_viewport.center());
        let unit=ui::transform_gizmo::project(camera,self.last_viewport,[modal.pivot[0]+1.,modal.pivot[1],modal.pivot[2]])
            .map(|p|(p-center).length()).unwrap_or(80.).max(1.);
        let (values,pivot)=modal.desired(center,unit,frame,camera,snap,self.editor_state.snap.increment,modifiers.shift);
        let pivot_changed=self.transform_target.as_ref().is_some_and(|target|
            self.editor.doc.value()["nodes"][target]["params"]["pivot"]!=json!(pivot));
        if values!=modal.last || pivot_changed {
            let op=modal.step(values);
            let axis_rotation=(modal.kind!=ModalKind::Move).then_some(
                if matches!(orientation,ui::Orientation::Local|ui::Orientation::Normal|ui::Orientation::Gimbal) {modal.rotation} else {[0.;3]});
            self.transform_node_at(op,Some(pivot),axis_rotation);
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
        }
    }
    /// Copy a node (parameters and inputs) under a fresh id and select the copy.
    fn duplicate_node(&mut self, id: &str) {
        let ids = if self.selected_set.contains(id) { self.selected_set.iter().cloned().collect::<Vec<_>>() } else { vec![id.to_owned()] };
        let Some(clip) = graph_edit::copy(self.editor.doc.value(), &ids) else { return };
        let Some((value, new_ids)) = graph_edit::paste(self.editor.doc.value(), &clip) else { return };
        if self.commit_document(value, "duplicate the node") { self.selected = new_ids[0].clone(); self.selected_set = new_ids.into_iter().collect(); }
    }
    /// Wire `from` into input `port` of `to` (its first input when `port` is None); multi inputs gain one more wire.
    fn connect_nodes(&mut self, from: &str, to: &str, port: Option<&str>) {
        let Some(value) = graph_edit::connect(self.editor.doc.value(), from, to, port) else { return };
        self.commit_document(value, "connect the nodes");
    }
    fn disconnect_input(&mut self, node: &str, port: &str) {
        if let Some(value) = graph_edit::disconnect(self.editor.doc.value(), node, port) { self.commit_document(value, "disconnect the input"); }
    }
    /// Keymap commands that have no dedicated action.
    fn command(&mut self, name: &str, ctx: &egui::Context) {
        use ui::EditorChange as C;
        let e = self.editor_state;
        match name {
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
            "toggle_edit" => {
                let mode = toggle_edit_mode(e.selection, &mut self.last_edit_selection);
                self.editor_state.apply(C::Selection(mode));
            }
            "select_none" => { self.selection.clear(); self.selection_dirty = true; self.selected_set.clear(); }
            "select_all" => {
                if self.network_hover {
                    self.selected_set = self.editor.doc.value()["nodes"].as_object().map(|n| n.keys().cloned().collect()).unwrap_or_default();
                    self.selected = self.selected_set.iter().next().cloned().unwrap_or_default();
                    return;
                }
                if let Some(scene) = &self.scene {
                    match e.selection {
                        ui::SelectionMode::Points => self.selection.points = (0..scene.positions().len() / 3).map(|i| i as u32).collect(),
                        ui::SelectionMode::Faces => self.selection.tris = (0..scene.triangle_indices().len() / 3).map(|i| i as u32).collect(),
                        ui::SelectionMode::Edges => {
                            let mut edges = std::collections::BTreeSet::new();
                            for t in scene.triangle_indices().as_chunks::<3>().0 { for (a,b) in [(t[0],t[1]),(t[1],t[2]),(t[2],t[0])] { edges.insert((a.min(b),a.max(b))); } }
                            self.selection.edges = edges.into_iter().collect();
                        }
                        ui::SelectionMode::Object => { self.object_hidden = false; self.selected_set = self.editor.doc.value()["nodes"].as_object().map(|n| n.keys().cloned().collect()).unwrap_or_default(); },
                    }
                    self.selection_dirty = true;
                }
            }
            "hide" => { self.object_hidden = true; self.selection.clear(); self.selection_dirty = true; }
            "reveal" => self.object_hidden = false,
            "add_menu" => ui::network::open_add_menu(ctx, &self.editor.path),
            "box_select" => ui::selection_region::arm(ctx),
            "timeline_toggle" | "timeline_previous" | "timeline_next" | "timeline_start" | "timeline_end" => ui::viewport_ui::timeline_command(ctx, name),
            "pie_menu" => ui::network::open_pie_menu(ctx, &self.editor.path),
            "display_flag" => {
                let mut value = self.editor.doc.value().clone();
                let mut changed = false;
                for (index, id) in self.selected_set.iter().enumerate() {
                    if let Some(next) = graph_edit::set_display(&value, id, index > 0 || ctx.input(|i| i.modifiers.shift)) { value = next; changed = true; }
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
                if self.viewport_hover && self.editor_state.selection != ui::SelectionMode::Object {
                    self.file_status = "Component deletion is not supported by the Rust graph yet".into();
                } else if let Some(id) = self.selected_set.iter().next().cloned() { self.delete_node(&id); }
            }
            "duplicate_selection" => { if let Some(id) = self.selected_set.iter().next().cloned() { self.duplicate_node(&id); } }
            "dissolve_selection" => {
                let ids = self.selected_set.iter().cloned().collect::<Vec<_>>();
                if let Some(value) = graph_edit::dissolve_set(self.editor.doc.value(), &ids) {
                    if self.commit_document(value, "dissolve nodes") { self.selected_set.clear(); self.display = minihou_app::display_nodes(self.editor.doc.value()).first().cloned().unwrap_or_default(); self.selected = self.display.clone(); }
                }
            }
            "extrude" | "inset" | "bevel" | "pushpull" | "loop_cut" => {
                let ty = match name { "extrude" => "polyextrude", "inset" => "inset", "bevel" => "polybevel", "pushpull" => "pushpull", _ => "loop_cut" };
                let after = self.selected.clone(); self.add_node(ty, Some(&after));
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
            "toggle_params" => self.params_open = !self.params_open,
            "xray" => self.editor_state.apply(C::Xray(!e.xray)),
            "snap" => self.editor_state.apply(C::SnapEnabled(!e.snap.enabled)),
            "proportional" => self.editor_state.apply(C::ProportionalEnabled(!e.proportional.enabled)),
            "toggle_projection" => {
                let next = if self.camera.is_ortho() { StandardView::Perspective } else { StandardView::Front };
                self.camera.set_view(next);
            }
            "shade_next" | "shade_previous" => {
                const ORDER: [Mode; 4] = [Mode::Solid, Mode::Shaded, Mode::Wire, Mode::ShadedWire];
                let i = ORDER.iter().position(|m| *m == self.mode).unwrap_or(1);
                self.mode = ORDER[if name == "shade_next" { (i + 1) % 4 } else { (i + 3) % 4 }];
            }
            _ => self.file_status = format!("Command {name} is not wired up yet"),
        }
    }
    /// Apply what the panels asked for this frame.
    fn apply(&mut self, ctx: &egui::Context, actions: Vec<UiAction>) {
        for action in actions {
            match action {
                UiAction::StartSketch(mode) => {
                    if minihou_core::nodes::get("curve").is_some() {
                        self.editor_state.apply(ui::EditorChange::Tool(ui::Tool::Select));
                        ui::sketch::start(ctx, mode);
                    } else { self.file_status = "Sketches need the Rust core curve node".into(); }
                },
                UiAction::TransformNode { op } => self.transform_node(op),
                UiAction::StartModal(kind) => self.start_modal(ctx, kind),
                UiAction::FinishTransform => self.finish_transform(),
                UiAction::CancelTransform => self.cancel_transform(),
                UiAction::SketchCommit { name, points, closed } => self.sketch_commit(&name, &points, closed),
                UiAction::AddMeasurement { a, b } => self.add_measurement(a, b),
                UiAction::RegionSelect { rect, extend, subtract } => self.region_select(rect, extend, subtract),
                UiAction::Open => if let Some(path) = rfd::FileDialog::new().add_filter("MiniHou graph", &["json"]).pick_file() { self.open(path); },
                UiAction::Save => self.save_to(self.editor.path.clone()),
                UiAction::SaveAs => {
                    if let Some(path) = rfd::FileDialog::new().add_filter("MiniHou graph", &["json"]).set_file_name(self.editor.path.file_name().unwrap_or_default().to_string_lossy()).save_file() { self.save_to(path); }
                }
                UiAction::Undo | UiAction::Redo => {
                    let undo = matches!(action, UiAction::Undo);
                    let command = if undo { self.editor.history.undo(&mut self.editor.doc) } else { self.editor.history.redo(&mut self.editor.doc) };
                    if let Some(command) = command {
                        self.display = Graph::from_value(self.editor.doc.value()).ok().and_then(|g| g.main_output().ok()).unwrap_or_default();
                        self.selected_set.retain(|id| self.editor.doc.value()["nodes"].get(id).is_some());
                        self.send(command);
                    }
                }
                UiAction::Frame => self.frame_mesh(),
                UiAction::FrameSelected => self.frame_selected(),
                UiAction::FrameNetwork => ui::network::frame(ctx, &self.editor.path),
                UiAction::Select(id) => { select_nodes(&mut self.selected_set, [id.clone()], false, false); self.selected = id; },
                UiAction::ToggleSelect(id) => { select_nodes(&mut self.selected_set, [id], false, true); self.selected = self.selected_set.iter().next().cloned().unwrap_or_default(); },
                UiAction::SelectSet { ids, extend } => { select_nodes(&mut self.selected_set, ids, extend, false); self.selected = self.selected_set.iter().next().cloned().unwrap_or_default(); },
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
                UiAction::AddNode { node_type, after } => self.add_node(&node_type, after.as_deref()),
                UiAction::SetMode(mode) => self.mode = mode,
                UiAction::Editor(change) => {
                    if let ui::EditorChange::Selection(mode) = change { if mode != ui::SelectionMode::Object { self.last_edit_selection = mode; } }
                    self.editor_state.apply(change);
                    if let ui::EditorChange::View(v) = change { self.camera.set_view(standard_view(v)); }
                    if let ui::EditorChange::ViewDistance(d) = change { self.camera.set_view_distance(d); }
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
                UiAction::ConnectNodes { from, to, port } => self.connect_nodes(&from, &to, port.as_deref()),
                UiAction::DisconnectInput { node, port } => self.disconnect_input(&node, &port),
                UiAction::TidyNetwork => ui::network::tidy(ctx, &self.editor.path),
                UiAction::BlenderBar(on) => self.blender_bar = on,
                UiAction::BlenderInstall(i) => {
                    use ui::blender_ui::blender_link as bl;
                    self.blender.message = match (self.blender_paths.get(i), bl::addon_zip()) {
                        (Some(exe), Some(zip)) => bl::install_addon(exe, &zip).unwrap_or_else(|e| format!("Install failed: {e}")),
                        (None, _) => "No Blender installation selected.".into(),
                        (_, None) => "The add-on ZIP (MiniHou_Link*.zip) was not found next to the app or under dist.".into(),
                    };
                    self.blender_bar = true;
                }
                UiAction::BlenderOpen => {
                    use ui::blender_ui::blender_link as bl;
                    self.blender.message = match self.blender_paths.first() {
                        Some(exe) => match bl::open_blender(exe, &self.editor.path) { Ok(_) => "Starting Blender...".into(), Err(e) => format!("Could not start Blender: {e}") },
                        None => "No Blender installation found.".into(),
                    };
                    self.blender_bar = true;
                }
                UiAction::BlenderDownloadAddon => {
                    self.blender.message = match ui::blender_ui::blender_link::addon_zip() {
                        Some(zip) => { let _ = std::process::Command::new("explorer").arg("/select,").arg(&zip).spawn(); format!("Add-on: {}", zip.display()) }
                        None => "The add-on ZIP was not found next to the app or under dist.".into(),
                    };
                    self.blender_bar = true;
                }
                UiAction::ToggleLocked(id) => { let f = self.flags.entry(id).or_default(); f.locked = !f.locked; }
                UiAction::ToggleTemplate(id) => { let f = self.flags.entry(id).or_default(); f.template = !f.template; }
            }
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(self.editor.title()));
    }
}
impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        if !self.themed { self.theme.apply(ctx); self.themed = true; }
        self.camera.set_view_distance(self.editor_state.proxy.view_distance);
        if let Some(response) = self.worker.poll() {
            if let Some(scene) = self.cooking.accept(response) {
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
        if self.selection.mode != self.editor_state.selection { self.selection.clear(); self.selection.mode = self.editor_state.selection; self.selection_dirty = true; }
        if self.selection_dirty {
            if let (Some(gpu), Some(scene)) = (&mut self.gpu, &self.scene) {
                let size = scene.radius * 0.012;
                let (faces, lines) = picking::highlight(scene.positions(), scene.triangle_indices(), &self.selection, size);
                gpu.set_selection(&self.state.device, &faces, &lines);
                let dec_notice = if self.editor_state.selection == ui::SelectionMode::Points {
                    picking::decimate_dots(scene.positions().len() / 3).status
                } else {
                    None
                };
                self.file_status = match (self.selection.is_empty(), dec_notice) {
                    (true, None) => String::new(),
                    (false, None) => format!("{} selected", self.selection.count()),
                    (true, Some(d)) => d.to_string(),
                    (false, Some(d)) => format!("{d} · {} selected", self.selection.count()),
                };
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
        let graph = Graph::from_value(self.editor.doc.value()).expect("validated document");
        let mut out = Actions::default();
        let kctx = KeyContext { viewport_hovered: self.viewport_hover, network_hovered: self.network_hover,
            in_edit_mode: self.editor_state.selection != ui::SelectionMode::Object, selected: Some(&self.selected) };
        if self.modal.is_none() && !ui::sketch::active(ctx) && !ui::transform_gizmo::active(ctx) && !ui::selection_region::active(ctx) && !ui::prim_handles::active(ctx) { for action in self.keymap.poll(ctx, &kctx) { out.push(action); } }
        {
            let v = View { graph: &graph, doc: self.editor.doc.value(), path: &self.editor.path, dirty: self.editor.dirty(),
                can_undo: self.editor.history.can_undo(), can_redo: self.editor.history.can_redo(),
                selected: &self.selected, selected_set: &self.selected_set, display: &self.display, cooking: &self.cooking, file_status: &self.file_status,
                mode: self.mode, theme: &self.theme, editor: &self.editor_state, flags: &self.flags, geo: self.geo.as_ref(), blender: &self.blender,
                bounds: self.bounds, selection_center: self.selection_center(),
                camera: self.camera_info() };
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
            let net = egui::SidePanel::left("network").default_width(400.).show(ctx, |ui| ui::network::show(ui, &v, &mut out));
            self.network_hover = ctx.input(|i| i.pointer.hover_pos()).is_some_and(|p| net.response.rect.contains(p));
            if let Some(tool) = net.response.dnd_release_payload::<ui::ToolDrag>() {
                out.push(ui::shelf::dropped_tool(&tool, &self.selected));
            }
            if self.params_open { egui::SidePanel::right("parameters").default_width(320.).show(ctx, |ui| {
                egui::TopBottomPanel::bottom("geometry_info").resizable(true).default_height(220.).show_inside(ui, |ui| ui::geo_info::show(ui, &v, &mut out));
                ui::params::show(ui, &v, &mut out);
            }); }
            egui::CentralPanel::default().show(ctx, |ui| {
                let size = ui.available_size().max(egui::vec2(1., 1.));
                let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
                self.last_viewport = rect;
                let delta = ctx.input(|i| i.pointer.delta());
                if response.dragged() && self.modal.is_none() && !ui::sketch::active(ctx) && !ui::transform_gizmo::active(ctx) && !ui::selection_region::active(ctx) && !ui::prim_handles::active(ctx)
                    && !ctx.input(|i| i.pointer.hover_pos()).is_some_and(|p| ui::transform_gizmo::hit(rect, &v, p) || ui::prim_handles::hit(rect, &v, p)) {
                    let modifiers = ctx.input(|i| i.modifiers);
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
                if response.hovered() && self.modal.is_none() { self.camera.zoom(ctx.input(|i| i.smooth_scroll_delta.y)); }
                let limit = self.state.device.limits().max_texture_dimension_2d;
                let pixels = [(size.x * ctx.pixels_per_point()).round() as u32, (size.y * ctx.pixels_per_point()).round() as u32].map(|x| x.clamp(1, limit));
                if self.target.as_ref().is_none_or(|t| t.size != pixels) {
                    if let Some(old) = self.target.take() { self.state.renderer.write().free_texture(&old.id); }
                    self.target = Some(Target::new(&self.state, pixels));
                }
                if self.lod_uploaded {
                    if let (Some(lod), Some(gpu)) = (&self.cooking.lod, &mut self.gpu) {
                        let view = self.camera.matrices(pixels[0] as f32 / pixels[1] as f32).0;
                        let wire = self.mode == Mode::Wire || self.mode == Mode::ShadedWire || self.editor_state.xray;
                        let signature = (view, pixels, wire);
                        if self.last_lod_view.as_ref() != Some(&signature)
                            && (self.last_lod_view.is_none() || self.last_lod_select.elapsed() >= Duration::from_millis(50)) {
                            let ids = lod.select(self.camera.lod_camera(pixels), lod.error_px);
                            gpu.select_lod(&self.state.device, lod, &ids, wire);
                            self.last_lod_view = Some(signature);
                            self.last_lod_select = Instant::now();
                        }
                    }
                }
                if let (Some(gpu), Some(target)) = (&self.gpu, &self.target) {
                    gpu.draw(
                        &self.state,
                        target,
                        &self.camera,
                        self.mode,
                        Overlay {
                            normals: self.editor_state.overlays.normals,
                            bounds: self.editor_state.overlays.bounds,
                            xray: self.editor_state.xray,
                            wireframe: self.editor_state.overlays.wireframe,
                        },
                        self.editor_state.selection,
                    );
                    if !self.object_hidden {
                        ui.painter().image(target.id, rect, egui::Rect::from_min_max(egui::pos2(0., 0.), egui::pos2(1., 1.)), egui::Color32::WHITE);
                    }
                }
                // Silhouette stroke for selected object (Item 2)
                if !self.object_hidden {
                    let displayed_nodes = self.editor.doc.display_nodes();
                    if displayed_nodes.contains(&self.selected) {
                        if let Some(scene) = &self.scene {
                            let silhouettes = picking::silhouette_edges(
                                scene.positions(),
                                scene.triangle_indices(),
                                v.camera.ray_origin,
                                v.camera.ortho,
                                v.camera.ray_forward,
                            );
                            let stroke = egui::Stroke::new(2.0_f32, egui::Color32::from_rgb(0xff, 0xa0, 0x28));
                            for [u, w] in silhouettes {
                                let pa = picking::at(scene.positions(), u);
                                let pb = picking::at(scene.positions(), w);
                                if let (Some(sa), Some(sb)) = (
                                    ui::transform_gizmo::project(v.camera, rect, pa),
                                    ui::transform_gizmo::project(v.camera, rect, pb),
                                ) {
                                    ui.painter().line_segment([sa, sb], stroke);
                                }
                            }
                        }
                    }
                }

                // Hover pre-selection highlight (Item 4)
                let hover_pos = ctx.input(|i| i.pointer.latest_pos());
                let hover_in_viewport = hover_pos.is_some_and(|p| rect.contains(p))
                    && !ui::sketch::active(ctx)
                    && !ui::selection_region::active(ctx);

                let hover_elem = if hover_in_viewport {
                    if let (Some(pos), Some(scene)) = (hover_pos, &self.scene) {
                        let (o, d) = ui::transform_gizmo::ray(v.camera, rect, pos);
                        picking::pick_triangle(scene.positions(), scene.triangle_indices(), o, d)
                            .map(|hit| picking::element_of(hit, scene.triangle_indices(), self.editor_state.selection))
                    } else {
                        None
                    }
                } else {
                    None
                };

                // Elements in edit modes 1/2/3 (Item 4)
                if !self.object_hidden {
                    if let Some(scene) = &self.scene {
                        let pos = scene.positions();
                        let tris = scene.triangle_indices();
                        let orange = egui::Color32::from_rgb(0xff, 0xa0, 0x28);
                        let white = egui::Color32::WHITE;

                        match self.editor_state.selection {
                            ui::SelectionMode::Object => {}
                            ui::SelectionMode::Points => {
                                let npoints = pos.len() / 3;
                                let dec = picking::decimate_dots(npoints);
                                let base_fill = egui::Color32::from_rgb(0x40, 0x50, 0x70);
                                let base_stroke = egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(0x90, 0xa5, 0xc0));

                                if !dec.hidden {
                                    for i in (0..npoints).step_by(dec.stride) {
                                        let pt = picking::at(pos, i as u32);
                                        if let Some(sp) = ui::transform_gizmo::project(v.camera, rect, pt) {
                                            let id = i as u32;
                                            let is_sel = self.selection.points.contains(&id);
                                            let is_hov = hover_elem.as_ref().is_some_and(|e| e.points.contains(&id));
                                            if is_hov {
                                                ui.painter().circle_filled(sp, 4.5_f32, white);
                                                ui.painter().circle_stroke(sp, 4.5_f32, egui::Stroke::new(1.0_f32, orange));
                                            } else if is_sel {
                                                ui.painter().circle_filled(sp, 3.5_f32, orange);
                                            } else {
                                                ui.painter().circle_filled(sp, 2.0_f32, base_fill);
                                                ui.painter().circle_stroke(sp, 2.0_f32, base_stroke);
                                            }
                                        }
                                    }
                                }
                                for &id in &self.selection.points {
                                    if (id as usize) < npoints {
                                        let pt = picking::at(pos, id);
                                        if let Some(sp) = ui::transform_gizmo::project(v.camera, rect, pt) {
                                            ui.painter().circle_filled(sp, 3.5_f32, orange);
                                        }
                                    }
                                }
                                if let Some(hov) = &hover_elem {
                                    for &id in &hov.points {
                                        if (id as usize) < npoints {
                                            let pt = picking::at(pos, id);
                                            if let Some(sp) = ui::transform_gizmo::project(v.camera, rect, pt) {
                                                ui.painter().circle_filled(sp, 4.5_f32, white);
                                                ui.painter().circle_stroke(sp, 4.5_f32, egui::Stroke::new(1.0_f32, orange));
                                            }
                                        }
                                    }
                                }
                            }
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
                                let ntri = tris.len() / 3;
                                let center_fill = egui::Color32::from_rgb(0x30, 0x38, 0x48);
                                let center_stroke = egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(0x80, 0x90, 0xa8));
                                let stride = if ntri > 50_000 { (ntri / 20_000).max(2) } else { 1 };
                                for i in (0..ntri).step_by(stride) {
                                    let t = &tris[i * 3..i * 3 + 3];
                                    let (a, b, c) = (picking::at(pos, t[0]), picking::at(pos, t[1]), picking::at(pos, t[2]));
                                    let center = [0, 1, 2].map(|k| (a[k] + b[k] + c[k]) / 3.0_f32);
                                    if let Some(sp) = ui::transform_gizmo::project(v.camera, rect, center) {
                                        let id = i as u32;
                                        let is_sel = self.selection.tris.contains(&id);
                                        let is_hov = hover_elem.as_ref().is_some_and(|e| e.tris.contains(&id));
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
                                    for &t_idx in &hov.tris {
                                        if let Some(chunk) = tris.as_chunks::<3>().0.get(t_idx as usize) {
                                            let (a, b, c) = (picking::at(pos, chunk[0]), picking::at(pos, chunk[1]), picking::at(pos, chunk[2]));
                                            if let (Some(sa), Some(sb), Some(sc)) = (
                                                ui::transform_gizmo::project(v.camera, rect, a),
                                                ui::transform_gizmo::project(v.camera, rect, b),
                                                ui::transform_gizmo::project(v.camera, rect, c),
                                            ) {
                                                ui.painter().add(egui::Shape::convex_polygon(
                                                    vec![sa, sb, sc],
                                                    egui::Color32::from_rgba_unmultiplied(255, 255, 255, 60),
                                                    egui::Stroke::new(1.5_f32, white),
                                                ));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                self.viewport_hover = response.hovered();
                if self.modal.is_none() && response.hovered() && !ctx.wants_keyboard_input() && ctx.input(|i| i.key_pressed(egui::Key::Tab)) {
                    out.push(UiAction::Command("add_menu".into()));
                }
                if response.clicked_by(egui::PointerButton::Primary) && self.modal.is_none() && !ui::sketch::active(ctx) && !ui::selection_region::active(ctx) && !ui::prim_handles::active(ctx) && !ctx.input(|i| i.pointer.hover_pos()).is_some_and(|p| ui::prim_handles::hit(rect, &v, p)) && self.editor_state.tool == ui::Tool::Select {
                    if let (Some(at), Some(scene)) = (response.interact_pointer_pos(), &self.scene) {
                        let (nx, ny) = (((at.x - rect.left()) / rect.width()) * 2. - 1., 1. - ((at.y - rect.top()) / rect.height()) * 2.);
                        let (o, d) = self.camera.ray(nx, ny, rect.width() / rect.height());
                        let mode = self.editor_state.selection;
                        let extend = ctx.input(|i| i.modifiers.shift);
                        let hit = picking::pick_triangle(scene.positions(), scene.triangle_indices(), o, d);
                        let hit_object = hit.is_some();
                        let new = hit.map(|hit| picking::element_of(hit, scene.triangle_indices(), mode))
                            .unwrap_or(picking::Selection { mode, ..Default::default() });
                        if extend && !new.is_empty() { picking::extend(&mut self.selection, &new); } else { self.selection = new; }
                        if mode == ui::SelectionMode::Object && hit_object {
                            out.push(UiAction::SelectSet { ids: vec![self.display.clone()], extend });
                        }
                        self.selection_dirty = true;
                    }
                }
                if response.clicked_by(egui::PointerButton::Primary) && self.modal.is_none() && self.editor_state.tool == ui::Tool::Cursor {
                    if let Some(at) = response.interact_pointer_pos() {
                        let (o,d) = ui::transform_gizmo::ray(v.camera, rect, at);
                        if d[2].abs() > 1e-5 { let t = -o[2] / d[2]; if t >= 0. { out.push(UiAction::Editor(ui::EditorChange::Cursor([0,1,2].map(|k| o[k] + t*d[k])))); } }
                    }
                }
                let (dropped, tool_hover) = viewport_tool_drop(ctx, rect);
                if let Some(tool) = dropped {
                    out.push(ui::shelf::dropped_tool(&tool, &self.selected));
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
                                    let scene_radius = self.scene.as_ref().map(|s| s.radius).unwrap_or(1.0);
                                    picking::intersect_ground_or_mesh(o, d, mesh_p, mesh_tri, (scene_radius * 2.0).max(5.0))
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
                if let Some(modal)=&self.modal {
                    ui::transform_overlay::show(ui,rect,&v,modal);
                    let snap=(self.editor_state.snap.enabled ^ ctx.input(|i|i.modifiers.ctrl))
                        || (modal.kind==ModalKind::Move && modal.snap_to!=ui::SnapTo::Increment);
                    ui::viewport_ui::modal_status(ui,rect,&modal.status(snap));
                } else {
                    ui::transform_gizmo::show(ui, rect, &v, &mut out);
                    ui::prim_handles::show(ui, rect, &v, &mut out);
                    ui::sketch::show(ui, rect, &v, &mut out);
                    ui::selection_region::show(ui, rect, &v, &mut out);
                }
            });
        }
        if self.prefs_open && ui::keymap::show_preferences(ctx, &mut self.prefs_open, &mut self.prefs, &mut self.keymap, &self.theme) {
            self.prefs.keymap_overrides = self.keymap.overrides_json();
            self.prefs.emulate_numpad = self.keymap.emulate_numpad;
            self.editor_state.proxy.view_distance = self.prefs.viewport.view_distance;
            self.camera.set_view_distance(self.editor_state.proxy.view_distance);
            if let Err(e) = self.prefs.save() { self.file_status = format!("Could not save preferences: {e}"); }
        }
        if self.modal.is_some() { self.update_modal(ctx); }
        if self.modal.is_none() && !ctx.input(|i| i.pointer.primary_down()) { self.editor.history.finish(&self.editor.doc); }
        self.apply(ctx, out.0);
        if self.modal.is_none() && !ctx.input(|i| i.pointer.primary_down()) { self.finish_transform(); }
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
    let options=eframe::NativeOptions { renderer:eframe::Renderer::Wgpu,viewport:egui::ViewportBuilder::default().with_inner_size([1280.,800.]),..Default::default() };
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
            let (tool, hover) = viewport_tool_drop(ctx, rect);
            assert_eq!(tool.unwrap().0, "box");
            assert!(!hover);
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



