mod picking;
mod graph_edit;
mod ui;
mod viewport;
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
            clipboard:None,transform_before:None,transform_target:None,last_viewport:egui::Rect::NOTHING };
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
        app.send(Edit::Display(app.display.clone()));
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
    fn transform_node(&mut self, node: &str, op: TransformOp) {
        let group = group_text(&self.selected_points());
        if self.editor_state.selection != ui::SelectionMode::Object && group.is_empty() {
            self.file_status = "Select points, edges, or faces to transform".into();
            return;
        }
        let first = self.transform_before.is_none();
        if first { self.editor.history.finish(&self.editor.doc); self.transform_before = Some(self.editor.doc.clone()); }
        let mut value = self.editor.doc.value().clone();
        let source = if group.is_empty() { node } else { &self.display };
        let target = if let Some(id) = &self.transform_target { id.clone() } else {
            let Some(nodes) = value["nodes"].as_object_mut() else { return; };
            if group.is_empty() && nodes.get(source).and_then(|n| n["type"].as_str()) == Some("transform") { source.to_owned() }
            else {
                let id = (1..).map(|n| format!("transform{n}")).find(|id| !nodes.contains_key(id)).unwrap();
                nodes.insert(id.clone(), json!({"type":"transform", "inputs":{"geo":source}}));
                value["display"] = json!(id); id
            }
        };
        let name = match op { TransformOp::Move(_) => "translate", TransformOp::Rotate(_) => "rotate", TransformOp::Scale(_) => "scale" };
        let delta = match op { TransformOp::Move(d) | TransformOp::Rotate(d) | TransformOp::Scale(d) => d };
        let defaults = if name == "scale" { [1.; 3] } else { [0.; 3] };
        let params = &mut value["nodes"][&target]["params"];
        if !params.is_object() { *params = json!({}); }
        let before = std::array::from_fn::<f32, 3, _>(|i| params[name].get(i).and_then(|n| n.as_f64()).map_or(defaults[i], |n| n as f32));
        let updated = match op {
            TransformOp::Scale(factor) => {
                let initial = self.transform_before.as_ref().map(|doc| &doc.value()["nodes"][&target]["params"]["scale"])
                    .unwrap_or(&serde_json::Value::Null);
                scaled_from_initial(initial, factor)
            }
            _ => [0,1,2].map(|i| before[i] + delta[i]),
        };
        params[name] = json!(updated);
        if first {
            params["group"] = json!(group);
            proportional_params(params, &self.editor_state);
            params["pivot"] = json!(if self.editor_state.pivot == ui::Pivot::Cursor { self.editor_state.cursor } else { self.selection_center().unwrap_or(self.bounds.0) });
        }
        match serde_json::to_string(&value).ok().and_then(|s| Document::parse(s).ok()) {
            Some(doc) => { self.editor.doc = doc; self.transform_target = Some(target.clone()); self.display = target; self.send(Edit::Replace(value)); }
            None => self.file_status = "Could not transform node".into(),
        }
    }
    fn finish_transform(&mut self) {
        if let Some(before) = self.transform_before.take() {
            let final_doc = self.editor.doc.clone(); self.editor.doc = before;
            self.editor.history.replace(&mut self.editor.doc, final_doc);
        }
        if let Some(target) = self.transform_target.take() { self.selected = target; }
    }
    fn cancel_transform(&mut self) {
        if let Some(before) = self.transform_before.take() {
            self.editor.doc = before;
            self.send(Edit::Replace(self.editor.doc.value().clone()));
            self.display = self.editor.doc.value()["display"].as_str().unwrap_or(&self.display).to_owned();
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
    fn command(&mut self, name: &str) {
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
                UiAction::TransformNode { node, op, continuous } => { self.transform_node(&node, op); if !continuous { self.finish_transform(); } },
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
                UiAction::Select(id) => self.selected = id,
                UiAction::SetDisplay(id) => {
                    self.display = id.clone();
                    self.editor.history.finish(&self.editor.doc);
                    self.edit(Edit::Display(id), false);
                }
                UiAction::Edit { edit, continuous } => {
                    if !continuous { self.editor.history.finish(&self.editor.doc); }
                    self.edit(edit, continuous);
                }
                UiAction::AddNode { node_type, after } => self.add_node(&node_type, after.as_deref()),
                UiAction::SetMode(mode) => self.mode = mode,
                UiAction::Editor(change) => {
                    self.editor_state.apply(change);
                    if let ui::EditorChange::View(v) = change { self.camera.set_view(standard_view(v)); }
                }
                UiAction::ViewPreset(v) => { self.editor_state.apply(ui::EditorChange::View(v)); self.camera.set_view(standard_view(v)); }
                UiAction::OpenPreferences => self.prefs_open = true,
                UiAction::Command(name) => self.command(&name),
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
        let kctx = KeyContext { viewport_hovered: self.viewport_hover, network_hovered: self.network_hover, selected: Some(&self.selected) };
        if !ui::sketch::active(ctx) && !ui::transform_gizmo::active(ctx) { for action in self.keymap.poll(ctx, &kctx) { out.push(action); } }
        if !ctx.wants_keyboard_input() && ctx.input(|i| i.key_pressed(egui::Key::F)) { out.push(UiAction::Frame); }
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
            egui::SidePanel::right("parameters").default_width(320.).show(ctx, |ui| {
                egui::TopBottomPanel::bottom("geometry_info").resizable(true).default_height(220.).show_inside(ui, |ui| ui::geo_info::show(ui, &v, &mut out));
                ui::params::show(ui, &v, &mut out);
            });
            egui::CentralPanel::default().show(ctx, |ui| {
                let size = ui.available_size().max(egui::vec2(1., 1.));
                let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
                self.last_viewport = rect;
                let delta = ctx.input(|i| i.pointer.delta());
                if response.dragged() && !ui::sketch::active(ctx) && !ui::transform_gizmo::active(ctx) && !ui::selection_region::active(ctx)
                    && !ctx.input(|i| i.pointer.hover_pos()).is_some_and(|p| ui::transform_gizmo::hit(rect, &v, p)) {
                    if response.dragged_by(egui::PointerButton::Middle) || response.dragged_by(egui::PointerButton::Secondary) || ctx.input(|i| i.modifiers.shift) {
                        self.camera.pan(delta.x, delta.y);
                    } else { self.camera.orbit(delta.x, delta.y); }
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
                    ui.painter().image(target.id, rect, egui::Rect::from_min_max(egui::pos2(0., 0.), egui::pos2(1., 1.)), egui::Color32::WHITE);
                }
                self.viewport_hover = response.hovered();
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
                if let Some(tool) = response.dnd_release_payload::<ui::ToolDrag>() {
                    out.push(ui::shelf::dropped_tool(&tool, &self.selected));
                }
                ui::viewport_ui::overlay(ui, rect, &v, &mut out);
                ui::transform_gizmo::cursor(ui, rect, &v);
                ui::transform_gizmo::show(ui, rect, &v, &mut out);
                ui::sketch::show(ui, rect, &v, &mut out);
                ui::selection_region::show(ui, rect, &v, &mut out);
                // proxy level, under the view gizmo
                egui::Area::new(egui::Id::new("proxy_menu")).fixed_pos(egui::pos2(rect.right() - 150., rect.top() + 130.)).order(egui::Order::Foreground)
                    .show(ctx, |ui| ui::proxy_ui::menu(ui, &v, &mut out));
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
    fn scaling_uses_the_drag_start_values() {
        assert_eq!(scaled_from_initial(&json!([2., 3., 4.]), [1., 2., 1.]), [2., 6., 4.]);
        assert_eq!(scaled_from_initial(&serde_json::Value::Null, [1., 0.5, 1.]), [1., 0.5, 1.]);
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



