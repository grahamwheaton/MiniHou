use super::*;

fn should_cook_on_start(value: &Value) -> bool { !display_is_empty(value) }

#[cfg(test)]
#[test]
fn fresh_start_has_no_nodes_and_queues_no_cook() {
    let doc = Document::parse(include_str!("default_graph.json").to_owned()).unwrap();
    assert!(doc.value()["nodes"].as_object().unwrap().is_empty());
    assert!(!should_cook_on_start(doc.value()));
    let worker = CookWorker::start(&doc, Scene::new).unwrap();
    assert!(worker.poll().is_none());
}

impl App {
    /// Load the field's current group text as the live selection (purple), on the topology the ids came from.
    pub(super) fn pick_load(&mut self) {
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
    pub(super) fn finish_param_pick(&mut self, ctx: &egui::Context, cancel: bool) {
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
    pub(super) fn finish_for_selection(&mut self, ctx: &egui::Context) {
        if let Some(modal) = self.modal.take() { self.finish_modal(modal); }
        self.finish_model_modal();
        self.finish_mirror();
        ui::transform_gizmo::finish_input(ctx);
        ui::prim_handles::finish_input(ctx);
        ui::curve_tool::cancel(ctx);
        ui::sketch::cancel(ctx);
        self.finish_transform();
    }
    pub(super) fn camera_info(&self) -> CameraInfo {
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
    pub(super) fn new_document(doc: Document, path: PathBuf, state: egui_wgpu::RenderState, start_link: bool) -> Result<Self> {
        let graph = Graph::from_value(doc.value())?;
        let display = if graph.nodes.is_empty() { String::new() } else { graph.main_output()? };
        let mut materials=minihou_app::materials::Registry::new(path.parent().unwrap_or(std::path::Path::new(".")));
        materials.register_graph(doc.value()); let _=materials.register_sidecar(&path);
        let worker = CookWorker::start_with_python(&doc, Some(python_gate()), Scene::new)?;
        let mut app = Self { selected:display.clone(), selected_set: std::collections::BTreeSet::from([display.clone()]), display,
            subnet_scope:None, subnet_pending_selection:None, editor:EditorDocument::new(doc, path), worker,
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
        if should_cook_on_start(app.editor.doc.value()) {
            app.send(Edit::Replace(app.editor.doc.value().clone()));
            app.send(Edit::Preview(app.selected.clone()));
        }
        Ok(app)
    }
    pub(super) fn send(&mut self, edit: Edit) {
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
    pub(super) fn recook_python(&mut self) {
        let (frame, fps) = self.link.as_ref().map_or(self.http_time, |l| (l.frame, l.fps));
        self.send(Edit::Time { frame, fps });
    }
    /// Run / Skip for the first python snippet waiting for confirmation (once per snippet per session).
    pub(super) fn python_confirm(&mut self, ctx: &egui::Context) {
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
    pub(super) fn send_timeline_time(&mut self, edit: Edit) {
        if let Edit::Time { frame, fps } = edit {
            self.http_time = (frame, fps);
            if let Some(link) = &mut self.link { link.frame = frame; link.fps = fps; }
            self.send(Edit::Time { frame, fps });
        }
    }
    pub(super) fn edit(&mut self, edit: Edit, continuous: bool) {
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
                    || matches!(edit, Edit::ModifierInputs { .. } | Edit::ModifierInputValue { .. }) {
                    self.send(Edit::Replace(self.editor.doc.value().clone()));
                } else { self.send(edit); }
                if !continuous { self.finish_history(); }
            }
            Err(e) => self.file_status = e.to_string(),
        }
    }
    pub(super) fn frame_mesh(&mut self) {
        self.camera = Camera::frame(self.bounds.0, self.bounds.1);
        self.camera.set_clip_planes(self.editor_state.proxy.clip_start, self.editor_state.proxy.clip_end);
    }
    pub(super) fn frame_selected(&mut self) {
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
    pub(super) fn embed_conflict_modal(&mut self, ctx: &egui::Context) {
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
    pub(super) fn save_to(&mut self, path: PathBuf) {
        self.finish_history();
        match self.editor.save_to(path) {
            Ok(()) => { self.file_status = "Saved".into(); }
            Err(e) => self.file_status = format!("Save failed: {e}"),
        }
    }
    pub(super) fn open(&mut self, path: PathBuf) -> bool {
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
    pub(super) fn new_graph(&mut self) -> bool {
        if self.link.as_ref().is_some_and(|link| link.live_active()) {
            self.file_status = "Disconnect the live Blender modifier before opening another document.".into();
            return false;
        }
        let replacement = empty_start_document().and_then(|doc| Self::new_document(doc, PathBuf::from("untitled.json"), self.state.clone(), false));
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
            Err(e) => { self.file_status = format!("New graph failed: {e}"); false }
        }
    }
}

fn empty_start_document() -> Result<Document> {
    Document::parse(include_str!("default_graph.json").to_owned())
}

#[cfg(test)]
#[test]
fn new_graph_uses_empty_start_document() {
    let doc = empty_start_document().unwrap();
    assert!(doc.graph().nodes.is_empty());
}
