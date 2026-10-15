mod picking;
mod graph_edit;
mod ui;
mod viewport;
#[cfg(test)] mod transform_tests;
use eframe::{egui, egui_wgpu};
use minihou_app::{Camera, Document, Result, Edit, EditorDocument, CookWorker, CookRequest, CookState, StandardView};
use minihou_core::Graph;
use serde_json::{json, Value};
use std::{collections::HashMap, path::PathBuf, time::Duration};
use ui::keymap::{KeyContext, Keymap, Preferences, ThemePref};
use ui::{theme::Theme, Actions, BlenderStatus, CameraInfo, EditorState, GeoSummary, NodeFlags, TransformOp, UiAction, View};
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

struct App {
    editor: EditorDocument, worker: CookWorker<Scene>, cooking: CookState,
    selected: String, display: String,
    state: egui_wgpu::RenderState, gpu: Option<Gpu>, target: Option<Target>,
    camera: Camera, bounds: ([f32;3], f32), mode: Mode,
    file_status: String,
    theme: Theme, flags: HashMap<String, NodeFlags>, themed: bool,
    editor_state: EditorState,
    geo: Option<GeoSummary>, blender: BlenderStatus,
    prefs: Preferences, keymap: Keymap, prefs_open: bool, viewport_hover: bool, network_hover: bool,
    /// The last cooked scene at full detail, and the proxy level it is currently uploaded at.
    scene: Option<Scene>, shown_level: f32, geo_rev: u64,
    blender_paths: Vec<PathBuf>, blender_bar: bool,
    selection: picking::Selection, selection_dirty: bool,
    clipboard: Option<Value>,
    last_viewport: egui::Rect,
    transform_before: Option<Document>, transform_target: Option<String>,
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
            ui::SelectionMode::Object => { if inside(self.bounds.0) { self.selected = self.display.clone(); } }
        }
        if subtract { self.selection.points.retain(|x| !found.points.contains(x)); self.selection.edges.retain(|x| !found.edges.contains(x)); self.selection.tris.retain(|x| !found.tris.contains(x)); }
        else if extend { for x in found.points { if !self.selection.points.contains(&x) { self.selection.points.push(x); } } for x in found.edges { if !self.selection.edges.contains(&x) { self.selection.edges.push(x); } } for x in found.tris { if !self.selection.tris.contains(&x) { self.selection.tris.push(x); } } }
        else { self.selection = found; }
        self.selection_dirty = true;
    }
    fn new(doc: Document, path: PathBuf, state: egui_wgpu::RenderState) -> Result<Self> {
        let display = Graph::from_value(doc.value())?.main_output()?;
        let worker = CookWorker::start(&doc, Scene::new)?;
        let mut app = Self { selected:display.clone(), display, editor:EditorDocument::new(doc, path), worker,
            cooking:CookState::default(), state,gpu:None,target:None,
            camera:Camera::frame([0.;3],1.),bounds:([0.;3],1.),mode:Mode::Shaded,file_status:String::new(),
            theme:Theme::dark(),flags:HashMap::new(),themed:false,editor_state:EditorState::default(),geo:None,blender:BlenderStatus::default(),
            prefs:Preferences::default(),keymap:Keymap::blender(),prefs_open:false,viewport_hover:false,network_hover:false,
            scene:None,shown_level:1.0,geo_rev:u64::MAX,blender_paths:Vec::new(),blender_bar:false,selection:picking::Selection::default(),selection_dirty:false,
            clipboard:None,transform_before:None,transform_target:None,last_viewport:egui::Rect::NOTHING,
            last_edit_selection:ui::SelectionMode::Points,params_open:true,object_hidden:false };
        app.prefs = Preferences::load();
        app.keymap = Keymap::from_overrides_json(&app.prefs.keymap_overrides);
        app.keymap.emulate_numpad = app.prefs.emulate_numpad;
        if app.prefs.theme == ThemePref::Light { app.theme = Theme::light(); }
        app.editor_state.snap.increment = app.prefs.snap_increment;
        app.editor_state.overlays.grid = app.prefs.viewport.show_grid;
        app.editor_state.overlays.axes = app.prefs.viewport.show_axes;
        app.editor_state.overlays.stats = app.prefs.show_stats;
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
    fn frame_mesh(&mut self) { self.camera = Camera::frame(self.bounds.0,self.bounds.1); }
    fn frame_selected(&mut self) {
        if let (Some(scene), Some(center)) = (&self.scene, self.selection_center()) {
            let points = scene.positions();
            let radius = self.selected_points().into_iter().filter_map(|id| points.get(id as usize * 3..id as usize * 3 + 3))
                .map(|p| ((0..3).map(|i| (p[i] - center[i]).powi(2)).sum::<f32>()).sqrt())
                .fold(0_f32, f32::max).max(self.bounds.1 * 0.05);
            self.camera = Camera::frame(center, radius);
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
    fn transform_node(&mut self, op: TransformOp) {
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
        if first {
            params["group"] = json!(group);
            proportional_params(params, &self.editor_state);
            params["pivot"] = json!(if self.editor_state.pivot == ui::Pivot::Cursor { self.editor_state.cursor } else { self.selection_center().unwrap_or(self.bounds.0) });
        }
        match apply_transform_step(&mut self.editor.doc, first, value, &target, name, param_value) {
            Ok(edits) => { self.transform_target = Some(target.clone()); self.display = target; for edit in edits { self.send(edit); } }
            Err(e) => self.file_status = format!("Could not transform node: {e}"),
        }
    }
    fn finish_transform(&mut self) {
        if let Some(before) = self.transform_before.take() {
            self.editor.history.record(before, &self.editor.doc);
        }
        if let Some(target) = self.transform_target.take() { self.selected = target; }
    }
    fn cancel_transform(&mut self) {
        if let Some(before) = self.transform_before.take() {
            self.editor.doc = before;
            self.send(Edit::Replace(self.editor.doc.value().clone()));
            self.display = Graph::from_value(self.editor.doc.value()).ok().and_then(|g| g.main_output().ok()).unwrap_or_else(|| self.display.clone());
        }
        self.transform_target = None;
    }
    fn sketch_commit(&mut self, name: &str, points: &[[f32; 3]], closed: bool) {
        let Some((value, id)) = graph_edit::add_sketch(self.editor.doc.value(), name, points, closed, &self.selected) else {
            self.file_status = "Could not add sketch".into(); return;
        };
        if self.commit_document(value, "add sketch") { self.selected = id.clone(); self.display = id; }
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
        let Some(value) = graph_edit::delete(self.editor.doc.value(), id) else { return };
        if self.commit_document(value, "delete the node") {
            if let Ok(g) = Graph::from_value(self.editor.doc.value()) {
                if self.selected == id { self.selected = g.nodes.first().map(|(n, _)| n.clone()).unwrap_or_default(); }
                self.display = g.main_output().unwrap_or_else(|_| self.selected.clone());
            }
        }
    }
    /// Copy a node (parameters and inputs) under a fresh id and select the copy.
    fn duplicate_node(&mut self, id: &str) {
        let Some((value, new_id)) = graph_edit::duplicate(self.editor.doc.value(), id) else { return };
        if self.commit_document(value, "duplicate the node") { self.selected = new_id; }
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
                self.clipboard = graph_edit::copy(self.editor.doc.value(), std::slice::from_ref(&self.selected));
                if self.clipboard.is_some() { self.file_status = "Node copied".into(); }
            }
            "paste" => {
                if let Some((value, ids)) = self.clipboard.as_ref().and_then(|clip| graph_edit::paste(self.editor.doc.value(), clip)) {
                    if self.commit_document(value, "paste nodes") {
                        self.selected = ids[0].clone();
                        self.file_status = format!("Pasted {} node(s)", ids.len());
                    }
                }
            }
            "toggle_edit" => {
                let mode = toggle_edit_mode(e.selection, &mut self.last_edit_selection);
                self.editor_state.apply(C::Selection(mode));
            }
            "select_none" => { self.selection.clear(); self.selection_dirty = true; }
            "select_all" => {
                if let Some(scene) = &self.scene {
                    match e.selection {
                        ui::SelectionMode::Points => self.selection.points = (0..scene.positions().len() / 3).map(|i| i as u32).collect(),
                        ui::SelectionMode::Faces => self.selection.tris = (0..scene.triangle_indices().len() / 3).map(|i| i as u32).collect(),
                        ui::SelectionMode::Edges => {
                            let mut edges = std::collections::BTreeSet::new();
                            for t in scene.triangle_indices().as_chunks::<3>().0 { for (a,b) in [(t[0],t[1]),(t[1],t[2]),(t[2],t[0])] { edges.insert((a.min(b),a.max(b))); } }
                            self.selection.edges = edges.into_iter().collect();
                        }
                        ui::SelectionMode::Object => self.object_hidden = false,
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
                if let Some(value) = graph_edit::set_display(self.editor.doc.value(), &self.selected, ctx.input(|i| i.modifiers.shift)) {
                    self.commit_document(value, "set display flag");
                }
            }
            "mute_flag" => {
                let bypass = self.editor.doc.value()["nodes"][&self.selected]["flags"]["bypass"].as_bool().unwrap_or(false);
                self.edit(Edit::Bypass { node: self.selected.clone(), flag: !bypass }, false);
            }
            "template_flag" => { let f = self.flags.entry(self.selected.clone()).or_default(); f.template = !f.template; }
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
                        self.display = Graph::from_value(self.editor.doc.value()).unwrap().main_output().unwrap();
                        self.send(command);
                    }
                }
                UiAction::Frame => self.frame_mesh(),
                UiAction::FrameSelected => self.frame_selected(),
                UiAction::FrameNetwork => ui::network::frame(ctx, &self.editor.path),
                UiAction::Select(id) => self.selected = id,
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
                }
                UiAction::ViewPreset(v) => { self.editor_state.apply(ui::EditorChange::View(v)); self.camera.set_view(standard_view(v)); }
                UiAction::OpenPreferences => self.prefs_open = true,
                UiAction::Command(name) => self.command(&name, ctx),
                UiAction::Orbit { yaw, pitch } => self.camera.orbit_degrees(yaw, pitch),
                UiAction::Pan { dx, dy } => self.camera.pan(dx, dy),
                UiAction::Zoom(factor) => self.camera.zoom_factor(factor),
                UiAction::DeleteNode(id) => self.delete_node(&id),
                UiAction::DuplicateNode(id) => self.duplicate_node(&id),
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
        if let Some(response) = self.worker.poll() {
            if let Some(scene) = self.cooking.accept(response) {
                let first = self.gpu.is_none();
                self.bounds = (scene.center, scene.radius);
                let level = self.proxy_level(scene.triangles());
                let shown = if level < 1.0 { scene.proxied(level) } else { scene.proxied(1.0) };
                if let Some(gpu) = &mut self.gpu { gpu.upload(&self.state.device, &shown); }
                else { self.gpu = Some(Gpu::new(&self.state.device, &shown)); }
                self.shown_level = level;
                self.scene = Some(scene);
                if first { self.frame_mesh(); }
            }
        }
        if self.cooking.cooking() { ctx.request_repaint_after(Duration::from_millis(16)); }
        // the proxy level changed (menu, auto budget): re-upload the reduced scene
        if let Some(scene) = &self.scene {
            let level = self.proxy_level(scene.triangles());
            if (level - self.shown_level).abs() > 1e-6 {
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
                self.file_status = if self.selection.is_empty() { String::new() } else { format!("{} selected", self.selection.count()) };
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
        if !ui::sketch::active(ctx) && !ui::transform_gizmo::active(ctx) && !ui::selection_region::active(ctx) { for action in self.keymap.poll(ctx, &kctx) { out.push(action); } }
        {
            let v = View { graph: &graph, doc: self.editor.doc.value(), path: &self.editor.path, dirty: self.editor.dirty(),
                can_undo: self.editor.history.can_undo(), can_redo: self.editor.history.can_redo(),
                selected: &self.selected, display: &self.display, cooking: &self.cooking, file_status: &self.file_status,
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
                if response.dragged() && !ui::sketch::active(ctx) && !ui::transform_gizmo::active(ctx) && !ui::selection_region::active(ctx)
                    && !ctx.input(|i| i.pointer.hover_pos()).is_some_and(|p| ui::transform_gizmo::hit(rect, &v, p)) {
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
                if response.hovered() { self.camera.zoom(ctx.input(|i| i.smooth_scroll_delta.y)); }
                let limit = self.state.device.limits().max_texture_dimension_2d;
                let pixels = [(size.x * ctx.pixels_per_point()).round() as u32, (size.y * ctx.pixels_per_point()).round() as u32].map(|x| x.clamp(1, limit));
                if self.target.as_ref().is_none_or(|t| t.size != pixels) {
                    if let Some(old) = self.target.take() { self.state.renderer.write().free_texture(&old.id); }
                    self.target = Some(Target::new(&self.state, pixels));
                }
                if let (Some(gpu), Some(target)) = (&self.gpu, &self.target) {
                    gpu.draw(&self.state, target, &self.camera, self.mode, Overlay { normals: self.editor_state.overlays.normals, bounds: self.editor_state.overlays.bounds, xray: self.editor_state.xray });
                    if !self.object_hidden { ui.painter().image(target.id, rect, egui::Rect::from_min_max(egui::pos2(0., 0.), egui::pos2(1., 1.)), egui::Color32::WHITE); }
                }
                self.viewport_hover = response.hovered();
                if response.hovered() && !ctx.wants_keyboard_input() && ctx.input(|i| i.key_pressed(egui::Key::Tab)) {
                    out.push(UiAction::Command("add_menu".into()));
                }
                if response.clicked_by(egui::PointerButton::Primary) && !ui::sketch::active(ctx) && !ui::selection_region::active(ctx) && self.editor_state.tool == ui::Tool::Select {
                    if let (Some(at), Some(scene)) = (response.interact_pointer_pos(), &self.scene) {
                        let (nx, ny) = (((at.x - rect.left()) / rect.width()) * 2. - 1., 1. - ((at.y - rect.top()) / rect.height()) * 2.);
                        let (o, d) = self.camera.ray(nx, ny, rect.width() / rect.height());
                        let mode = self.editor_state.selection;
                        let extend = ctx.input(|i| i.modifiers.shift);
                        let new = picking::pick_triangle(scene.positions(), scene.triangle_indices(), o, d)
                            .map(|hit| picking::element_of(hit, scene.triangle_indices(), mode))
                            .unwrap_or(picking::Selection { mode, ..Default::default() });
                        if extend && !new.is_empty() { picking::extend(&mut self.selection, &new); } else { self.selection = new; }
                        self.selection_dirty = true;
                    }
                }
                if response.clicked_by(egui::PointerButton::Primary) && self.editor_state.tool == ui::Tool::Cursor {
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
                    }
                }
                ui::transform_gizmo::cursor(ui, rect, &v);
                ui::transform_gizmo::show(ui, rect, &v, &mut out);
                ui::sketch::show(ui, rect, &v, &mut out);
                ui::selection_region::show(ui, rect, &v, &mut out);
            });
        }
        if self.prefs_open && ui::keymap::show_preferences(ctx, &mut self.prefs_open, &mut self.prefs, &mut self.keymap, &self.theme) {
            self.prefs.keymap_overrides = self.keymap.overrides_json();
            self.prefs.emulate_numpad = self.keymap.emulate_numpad;
            if let Err(e) = self.prefs.save() { self.file_status = format!("Could not save preferences: {e}"); }
        }
        if !ctx.input(|i| i.pointer.primary_down()) { self.editor.history.finish(&self.editor.doc); }
        self.apply(ctx, out.0);
        if !ctx.input(|i| i.pointer.primary_down()) { self.finish_transform(); }
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



