use super::*;
mod link_updates;
mod cook_updates;

#[cfg(test)]
mod graph_snapshot_tests {
    use super::*;

    #[test]
    fn moving_camera_does_not_rebuild_graph() {
        let doc = Document::parse(include_str!("default_graph.json").into()).unwrap();
        let mut camera = Camera::frame([0.; 3], 1.);
        let graph = doc.graph() as *const Graph;
        camera.orbit(24., -12.);
        camera.pan(3., 5.);
        assert_eq!(doc.revision(), 0);
        assert_eq!(graph, doc.graph() as *const Graph);
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
        self.poll_frame_jobs(ctx);
        if !self.themed { self.theme.apply(ctx); self.themed = true; }
        self.camera.set_clip_planes(self.editor_state.proxy.clip_start, self.editor_state.proxy.clip_end);
        self.poll_frame_links(ctx);
        self.accept_frame_cook(ctx);
        self.refresh_frame_scene(ctx);
        if self.param_pick.is_some() {
            if let Some(cancel) = param_pick_end(ctx, self.last_viewport) { self.finish_param_pick(ctx, cancel); }
        }
        if let Some(kind)=ui::transform_gizmo::active_kind(ctx).filter(|_|ctx.input(|i|
            ui::modal_transform::base_snap_shortcut(egui::Key::B,i.key_pressed(egui::Key::B)))) {
            self.cancel_transform();
            ui::transform_gizmo::finish_input(ctx);
            self.start_modal(ctx,match kind { ui::Tool::Rotate=>ModalKind::Rotate,
                ui::Tool::Scale=>ModalKind::Scale, _=>ModalKind::Move });
        }
        let busy = self.modal.is_some() || self.model_modal.is_some() || self.mirror.is_some() || self.pushpull_pick || (ui::sketch::active(ctx) || self.editor_state.tool.drawing()) || ui::transform_gizmo::active(ctx) || ui::prim_handles::active(ctx) || ui::curve_tool::active(ctx);
        let fly_frame = viewport_input::fly(ctx, self.last_viewport, &mut self.camera,
            self.prefs.viewport.fly_navigation && !busy && !ui::selection_region::active(ctx) && !ctx.wants_keyboard_input());
        // B during a G/R/S modal is base snap (handled in update_modal), not the box select interrupt.
        let selection_interrupt = ui::modal_transform::selection_may_interrupt(self.modal.is_some(),
            ctx.input(|i| i.key_pressed(egui::Key::B)));
        if busy && selection_interrupt && !fly_frame && !ui::sketch::active(ctx) && !self.editor_state.tool.drawing() {
            let selection_actions = self.keymap.poll_selection(ctx, &KeyContext {
                viewport_hovered: self.viewport_hover, network_hovered: false,
                in_edit_mode: self.editor_state.selection != ui::SelectionMode::Object, selected: Some(&self.selected) });
            let select_press = !ui::curve_tool::modal_active(ctx) && matches!(self.editor_state.tool, ui::Tool::Select | ui::Tool::Move | ui::Tool::Rotate | ui::Tool::Scale | ui::Tool::Transform) && ctx.input(|i| i.pointer.primary_pressed()
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
        // Process confirmation before constructing View; its click stays owned by the modal through release.
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
        viewport_input::claim_modal_click(ctx, modal_frame || model_modal_frame);
        if (ui::transform_gizmo::active(ctx) || ui::prim_handles::active(ctx))
            && ctx.input(|i| i.pointer.secondary_pressed() || i.key_pressed(egui::Key::Escape)) {
            self.cancel_transform();
            ui::transform_gizmo::finish_input(ctx);
            ui::prim_handles::finish_input(ctx);
        }
        if !ctx.input(|i| i.pointer.primary_down()) {
            ui::transform_gizmo::finish_input(ctx);
            ui::prim_handles::finish_input(ctx);
            ui::deform_gizmo::finish_input(ctx);
        }
        let graph = self.editor.doc.graph();
        let mut out = Actions::default();
        let kctx = KeyContext { viewport_hovered: self.viewport_hover, network_hovered: self.network_hover,
            in_edit_mode: self.editor_state.selection != ui::SelectionMode::Object, selected: Some(&self.selected) };
        ui::deform_gizmo::keyboard(ctx,graph,&self.selected,self.editor_state.tool,self.viewport_hover,
            fly_frame || busy || ui::selection_region::active(ctx) || ui::transform_gizmo::active(ctx) || ui::prim_handles::active(ctx) || ui::curve_tool::active(ctx) || ui::viewport_ui::popup_open(ctx),&mut out);
        if !fly_frame && !busy && !ui::viewport_ui::popup_open(ctx) && self.modal.is_none() && self.model_modal.is_none() && !(ui::sketch::active(ctx) || ui::keymap::draw_tool_blocks_keymap(self.editor_state.tool.drawing(), self.network_hover)) && !ui::transform_gizmo::active(ctx) && !ui::selection_region::active(ctx) && !ui::prim_handles::active(ctx) && !ui::deform_gizmo::active(ctx) { for action in self.keymap.poll(ctx, &kctx) { out.push(action); } }
        ctx.data_mut(|d| d.insert_temp(ui::sketch::settings_id(), self.prefs.inference.clone()));
        {
            let repeat_desc = self.last_operator.as_ref().map(LastOperator::description);
            let transform_node = if self.editor_state.selection == ui::SelectionMode::Object { &self.selected } else { &self.display };
            let display_trs = transform_math::TrsParams::from_params(&self.editor.doc.value()["nodes"][transform_node]["params"]);
            let normal_frame = self.selection_normal_frame(&display_trs).axes.map(|axis| axis.map(|x| x as f32));
            let active_pick_set=self.active_pick_set(); let node_pick_set=self.pick_set();
            let active_cut_range=self.active_object().and_then(|id|active_range(id,&self.display_ranges).map(|range|(id.to_owned(),range.clone())));
            let v = View { graph, doc: self.editor.doc.value(), path: &self.editor.path, dirty: self.editor.dirty(),
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
            let layout = self.prefs.ui_layout.clone();
            if layout.shelf_locked { egui::TopBottomPanel::top("shelf").show(ctx, |ui| ui::shelf::show(ui, &v, &mut out)); }
            egui::TopBottomPanel::bottom("stats").show(ctx, |ui| ui::status::show(ui, &v, &mut out));
            ui::network::set_physics_snappiness(ctx, self.prefs.node_physics_snappiness);
            if let Some(targets) = self.subnet_pending_selection.take() {
                ui::network::select_inner(ctx, &self.editor.path, targets);
            }
            ui::network::set_links(ctx, &self.editor.path, self.prefs.show_parameter_links);
            let mut hidden = layout.hidden.clone();
            if !self.params_open { hidden.extend([ui::layout::PanelId::Parameters, ui::layout::PanelId::GeoInfo, ui::layout::PanelId::Spreadsheet]); }
            let mut dock_edit = None;
            egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |dock_ui| {
                use ui::layout::PanelId;
                let (leaves, dividers) = ui::dock::place(&layout.root, dock_ui.max_rect(), &hidden);
                let mut viewport_rect = dock_ui.max_rect();
                self.network_hover = false;
                for leaf in &leaves {
                    if leaf.active == PanelId::Viewport { viewport_rect = leaf.rect; continue; }
                    let content = ui::dock::leaf_frame(dock_ui, leaf);
                    let ui = &mut dock_ui.new_child(egui::UiBuilder::new().max_rect(content).id_salt(("dock_leaf", leaf.active)));
                    ui.set_clip_rect(content.intersect(dock_ui.clip_rect()));
                    match leaf.active {
                        PanelId::Network => {
                            ui::network::show(ui, &v, &mut out);
                            self.network_hover = ctx.input(|i| i.pointer.hover_pos()).is_some_and(|p| leaf.rect.contains(p));
                        }
                        PanelId::Parameters => {
                            if let Some((path,inner,selected,selected_set))=ui::network::current_scope(ctx,&self.editor.path,self.editor.doc.value()) {
                                if let Ok(inner_graph)=Graph::from_value(&inner) {
                                    let mut scoped=v.clone(); scoped.graph=&inner_graph; scoped.doc=&inner;
                                    scoped.selected=&selected; scoped.selected_set=&selected_set;
                                    let mut actions=Actions::default();
                                    ui::params::show(ui,&scoped,&mut actions);
                                    for action in actions.0 {if let Some(edit)=ui::network::scoped_edit(self.editor.doc.value(),&path,action) {out.push(edit);}}
                                }
                            } else {ui::params::show(ui, &v, &mut out);}
                        }
                        PanelId::GeoInfo => ui::geo_info::show_tab(ui, &v, ui::geo_info::GeoTab::Info),
                        PanelId::Spreadsheet => ui::geo_info::show_tab(ui, &v, ui::geo_info::GeoTab::Spreadsheet),
                        PanelId::Shelf => ui::shelf::show(ui, &v, &mut out),
                        PanelId::Viewport => {}
                    }
                }
                ui::dock::paint_dividers(dock_ui, &dividers);
                if let Some(root) = ui::dock::interact(dock_ui, &layout.root, &leaves, &dividers) { dock_edit = Some(root); }
                self.subnet_scope = ui::network::current_scope(ctx, &self.editor.path, self.editor.doc.value())
                    .map(|(path, inner, selected, _)| (path, if selected.is_empty() || inner["nodes"].get(&selected).is_none() {
                        let output = inner["output"].as_str().unwrap_or_default();
                        if inner["nodes"][output]["type"] == "subnet_output" {
                            inner["nodes"][output]["inputs"]["geo"].as_str().unwrap_or(output).to_owned()
                        } else { output.to_owned() }
                    } else { selected }));
                let margin = egui::Frame::central_panel(dock_ui.style()).inner_margin;
                dock_ui.painter().rect_filled(viewport_rect, 0., dock_ui.visuals().panel_fill);
                let ui = &mut dock_ui.new_child(egui::UiBuilder::new().max_rect(viewport_rect - margin).id_salt("dock_viewport"));
                let size = ui.available_size().max(egui::vec2(1., 1.));
                let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
                self.last_viewport = rect;
                let coplanar_hover=self.coplanar.is_some() && ctx.input(|i|i.pointer.hover_pos()).is_some_and(|p|coplanar_tool::contains(ctx,rect,p));
                let region_frame = if coplanar_hover {true} else if !fly_frame && (self.modal.is_none() && self.model_modal.is_none() && self.mirror.is_none()) && !mirror_frame && !(ui::sketch::active(ctx) || self.editor_state.tool.drawing()) {
                    ui::selection_region::handle(ctx, rect, &v, &mut out)
                } else { false };
                let mut delta = ctx.input(|i| i.pointer.delta());
                if !response.dragged() { self.orbit_drag=None; }
                if !fly_frame && !region_frame && response.dragged() && self.modal.as_ref().is_none_or(|modal| modal.cage && response.dragged_by(egui::PointerButton::Middle)) && self.model_modal.is_none() && !ui::sketch::blocks_navigation(ctx) && !ui::mesh_cut::blocks_navigation(ctx) && !ui::transform_gizmo::active(ctx) && !ui::selection_region::active(ctx) && !ui::prim_handles::active(ctx) && !ui::deform_gizmo::active(ctx) && !ui::deform_gizmo::armed(ctx) && !ui::curve_tool::active(ctx)
                    && !ctx.input(|i| i.pointer.hover_pos()).is_some_and(|p| (!ui::curve_tool::is_active_node(&v) && ui::transform_gizmo::hit(rect, &v, p)) || ui::prim_handles::hit(rect, &v, p) || ui::deform_gizmo::hit(rect, &v, p) || ui::curve_tool::hit(rect, &v, p, ctx)) {
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
                if !fly_frame && !region_frame && response.hovered() && (self.modal.is_none() || !self.modal.as_ref().is_some_and(|m| m.proportional)) && self.model_modal.is_none()
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
                    let preview_geo = if self.cooking.error.is_none() {
                        self.cooking.preview.as_ref().filter(|(id,_,_)| id == &self.selected).map(|(_,g,_)| g.as_ref())
                            .or_else(|| (self.selected == self.display).then(|| self.cooking.geometry.as_deref()).flatten())
                    } else { None };
                    let preview_input = self.cooking.preview.as_ref().filter(|(id,_,_)| id == &self.selected).and_then(|(_,_,g)| g.as_deref());
                    self.group_preview.update(gpu,&self.state.device,preview_geo,preview_input,self.editor.doc.value(),&self.selected,self.cooking.completed);
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
                let ignore_pick_occlusion=picking::ignores_surface_occlusion(self.mode,self.editor_state.overlays.enabled,self.editor_state.xray);

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
                                let wire=pick_displayed_wire(scene,v.camera,rect,pos,pick_visibility,&self.display_ranges,&pick_set,mode,ignore_pick_occlusion)?;
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
                    let selection_mode = if self.pushpull_pick { ui::SelectionMode::Faces } else { self.editor_state.selection };
                    let selected_object = selection_mode == ui::SelectionMode::Object && !self.object_hidden && self.display_ranges.iter().any(|(id, _)| self.selected_set.contains(id));
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
                        selection_mode,
                        selected_object,
                    );
                    if !self.object_hidden {
                        ui.painter().image(target.id, rect, egui::Rect::from_min_max(egui::pos2(0., 0.), egui::pos2(1., 1.)), egui::Color32::WHITE);
                    }
                }

                let displayed_node = &v.doc["nodes"][&self.display];
                if !self.object_hidden && displayed_node["type"] == "null" {
                    let size = displayed_node["params"]["size"].as_f64().unwrap_or(1.0).clamp(0.0, 10_000.0) as f32;
                    let color = if self.selected_set.contains(&self.display) { egui::Color32::from_rgb(255, 166, 56) }
                        else { egui::Color32::from_rgb(220, 225, 235) };
                    for [a, b] in viewport::null_crosshair(size) {
                        if let (Some(start), Some(end)) = (ui::transform_gizmo::project(v.camera, rect, a), ui::transform_gizmo::project(v.camera, rect, b)) {
                            ui.painter().line_segment([start, end], egui::Stroke::new(2.0, color));
                        }
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
                if !swallow_click && viewport_input::selection_click_available(ctx) && !fly_frame && !region_frame && !model_modal_frame && !mirror_frame && self.mirror.is_none() && !ui::viewport_ui::popup_open(ctx) && response.clicked_by(egui::PointerButton::Primary) && !(response.double_clicked() && ctx.input(|i|i.modifiers.shift) && matches!(v.doc["nodes"][v.selected]["type"].as_str(),Some("curve"|"bezier"))) && (self.modal.is_none() && self.model_modal.is_none()) && !(ui::sketch::active(ctx) || self.editor_state.tool.drawing()) && !ui::selection_region::active(ctx) && !ui::prim_handles::active(ctx) && !ui::deform_gizmo::active(ctx) && !ui::deform_gizmo::armed(ctx) && !ui::curve_tool::active(ctx) && !ctx.input(|i| i.pointer.hover_pos()).is_some_and(|p| ui::curve_tool::hit(rect, &v, p, ctx)) && !ctx.input(|i| i.pointer.hover_pos()).is_some_and(|p| ui::prim_handles::hit(rect, &v, p) || ui::deform_gizmo::hit(rect,&v,p)) && !ui::transform_gizmo::active(ctx) && !ctx.input(|i| i.pointer.hover_pos()).is_some_and(|p| !ui::curve_tool::is_active_node(&v) && ui::transform_gizmo::hit(rect, &v, p)) && matches!(self.editor_state.tool, ui::Tool::Select | ui::Tool::Move | ui::Tool::Rotate | ui::Tool::Scale | ui::Tool::Transform)
                    && self.param_pick.as_ref().is_none_or(|pick| pick.ready && pick_source_ready(&self.display, &self.display_ranges, &pick.source)) {
                    if let (Some(at), Some(scene)) = (response.interact_pointer_pos(), &self.scene) {
                        let (nx, ny) = (((at.x - rect.left()) / rect.width()) * 2. - 1., 1. - ((at.y - rect.top()) / rect.height()) * 2.);
                        let (o, d) = self.camera.ray(nx, ny, rect.width() / rect.height());
                        let mode = self.editor_state.selection;
                        let extend = ctx.input(|i| i.modifiers.shift);
                        let mut hit = if self.object_hidden { None } else { pick_displayed_culled(scene,o,d,pick_visibility,&self.display_ranges,&pick_set,false) };
                        if mode==ui::SelectionMode::Faces && ignore_pick_occlusion && !extend && self.selection.tris.len()>0 {
                            if let Some(front)=hit.filter(|front|self.selection.tris.contains(&(front.tri as u32))) {
                                hit=pick_displayed_behind(scene,o,d,front,pick_visibility,&self.display_ranges,&pick_set).or(Some(front));
                            }
                        }
                        let wire = if self.object_hidden || (hit.is_some() && !(ignore_pick_occlusion && matches!(mode,ui::SelectionMode::Points|ui::SelectionMode::Edges))) { None }
                            else { pick_displayed_wire(scene,v.camera,rect,at,pick_visibility,&self.display_ranges,&pick_set,mode,ignore_pick_occlusion) };
                        if mode == ui::SelectionMode::Object {
                            let ids = hit.and_then(|hit| picked_object(&self.display_ranges, scene.triangle_indices(), hit))
                                .or_else(|| wire.and_then(|w| { let id=match w { picking::WireHit::Point(i)|picking::WireHit::Edge(i,_)=>i }; self.display_ranges.iter().find(|(_,r)|r.contains(&id)).map(|(name,_)|name.as_str()) }))
                                .map(|id| vec![id.to_owned()]).unwrap_or_default();
                            let selecting_inner = if let Some((path, selected)) = &self.subnet_scope {
                                let point = hit.and_then(|hit| scene.triangle_indices().get(hit.tri * 3).copied())
                                    .or_else(|| wire.map(|hit| match hit { picking::WireHit::Point(point) |
                                        picking::WireHit::Edge(point, _) => point }));
                                let inner = point.and_then(|point| picking::picked_scoped_node(path, selected, &self.display_ranges, point))
                                    .or_else(|| {
                                        let owner = picking::scoped_display_owner(self.editor.doc.value(), path, selected, &self.display_ranges);
                                        ids.first().filter(|id| owner == Some(id.as_str())).map(|_| selected.clone())
                                    });
                                if let Some(inner) = inner {
                                    ui::network::select_inner(ctx, &self.editor.path, vec![inner.clone()]);
                                    self.subnet_scope = Some((path.clone(), inner));
                                    self.selection_dirty = true;
                                    true
                                } else { false }
                            } else { false };
                            if !selecting_inner {
                                // Empty space clears the shared network/object selection, including with Shift.
                                let extend = extend && !ids.is_empty();
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
                ui.scope_builder(egui::UiBuilder::new().layer_id(ui::viewport_ui::scene_layer_id()).max_rect(rect), |ui| {
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
                        ui::deform_gizmo::show(ui, rect, &v, &mut out);
                        ui::curve_tool::show(ui, rect, &v, &mut out, response.double_clicked());
                        ui::sketch::show(ui, rect, &v, &mut out,
                            self.scene.as_ref().map(|scene|(scene.positions(),scene.triangle_indices())));
                        ui::mesh_cut::show(ui, rect, &v, active_cut_range.as_ref().map(|(id,range)|(id.as_str(),range)), &mut out);
                    }
                    ui::selection_region::paint(ui, rect, &v);
                    coplanar_tool::show(ui,rect,&mut self.coplanar,v.units,&mut out);
                }
                });
            });
            if let Some(root) = dock_edit { self.prefs.ui_layout.root = root; }
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
        viewport_input::release_modal_click(ctx);
        if self.modal.is_none() && self.model_modal.is_none() && self.mirror.is_none() && !ctx.input(|i| i.pointer.primary_down()) { self.finish_history(); }
        self.apply(ctx, out.0);
        if (self.modal.is_none() && self.model_modal.is_none()) && !ctx.input(|i| i.pointer.primary_down()) { self.finish_transform(); }
        if self.cooking.cooking() { ctx.request_repaint_after(Duration::from_millis(16)); }
    }
}
