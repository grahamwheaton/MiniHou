use super::*;

impl App {
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
    pub(crate) fn command(&mut self, name: &str, ctx: &egui::Context) {
        use ui::EditorChange as C;
        let e = self.editor_state;
        match name {
            "move_nodes" | "scale_nodes" => if self.network_hover {
                let kind = if name == "move_nodes" { ui::network::LayoutKind::Move } else { ui::network::LayoutKind::Scale };
                ui::network::begin_layout(ctx, &self.editor.path, self.editor.doc.value(), &self.selected_set, kind);
            },
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
                if self.network_hover {
                    if let Some((path,inner,selected,set))=ui::network::current_scope(ctx,&self.editor.path,self.editor.doc.value()) {
                        let wires=ui::network::take_selected_wires(ctx,&self.editor.path,&inner);
                        let changed=if !wires.is_empty(){graph_edit::disconnect_wires(&inner,&wires)}
                            else {let ids:Vec<String>=if set.is_empty(){vec![selected]}else{set.into_iter().collect()};graph_edit::delete_set(&inner,&ids)};
                        if let Some(changed)=changed.and_then(|doc|ui::network::embed_scope(self.editor.doc.value(),&path,doc)) {
                            if self.commit_document(changed,"delete inner nodes") {ui::network::select_inner(ctx,&self.editor.path,Vec::new());}
                        }
                        return;
                    }
                }
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
                    self.add_node(ctx, "blast", Some(&source), false);
                } else if let Some(id) = self.selected_set.iter().next().cloned() { self.delete_node(&id); }
            }
            "group_selection" => {
                if self.viewport_hover && self.editor_state.selection != ui::SelectionMode::Object {
                    let Some(source) = self.active_object().map(str::to_owned) else { self.file_status = "Select displayed elements to group".into(); return; };
                    self.add_node(ctx, "group_create", Some(&source), false);
                    return;
                }
                if self.network_hover {
                    if let Some((path,inner,selected,set))=ui::network::current_scope(ctx,&self.editor.path,self.editor.doc.value()) {
                        let ids:Vec<String>=if set.is_empty(){vec![selected]}else{set.into_iter().collect()};
                        if let Some((changed,id))=subnet_edit::group(&inner,&ids) {
                            if let Some(root)=ui::network::embed_scope(self.editor.doc.value(),&path,changed) {
                                if self.commit_document(root,"group inner nodes") {ui::network::select_inner(ctx,&self.editor.path,vec![id]);}
                            }
                        } else {self.file_status="Select inner nodes to group".into();}
                        return;
                    }
                }
                let ids: Vec<String> = if self.selected_set.is_empty() { vec![self.selected.clone()] } else { self.selected_set.iter().cloned().collect() };
                match subnet_edit::group(self.editor.doc.value(), &ids) {
                    Some((value, id)) => if self.commit_document(value, "group nodes") {
                        self.selected = id.clone();
                        self.selected_set = std::collections::BTreeSet::from([id]);
                        if let Ok(g) = Graph::from_value(self.editor.doc.value()) { self.display = g.main_output().unwrap_or_else(|_| self.selected.clone()); }
                    },
                    None => self.file_status = "Select nodes to group (only one may feed the rest of the network)".into(),
                }
            }
            "ungroup_selection" => {
                if !self.network_hover { return; }
                if let Some((path, inner, selected, set)) = ui::network::current_scope(ctx, &self.editor.path, self.editor.doc.value()) {
                    let id = if set.len() == 1 { set.iter().next().cloned() } else if set.is_empty() { Some(selected) } else { None };
                    if let Some((changed, ids)) = id.and_then(|id| subnet_edit::ungroup(&inner, &id)) {
                        if let Some(root) = ui::network::embed_scope(self.editor.doc.value(), &path, changed) {
                            if self.commit_document(root, "ungroup inner subnet") { ui::network::select_inner(ctx, &self.editor.path, ids); }
                        }
                    } else { self.file_status = "Select one subnet to ungroup".into(); }
                    return;
                }
                let id = if self.selected_set.len() == 1 { self.selected_set.iter().next().cloned() }
                    else if self.selected_set.is_empty() { Some(self.selected.clone()) } else { None };
                if let Some((value, ids)) = id.and_then(|id| subnet_edit::ungroup(self.editor.doc.value(), &id)) {
                    if self.commit_document(value, "ungroup subnet") {
                        self.selected = ids.first().cloned().unwrap_or_default();
                        self.selected_set = ids.into_iter().collect();
                        if let Ok(g) = Graph::from_value(self.editor.doc.value()) { self.display = g.main_output().unwrap_or_else(|_| self.selected.clone()); }
                    }
                } else { self.file_status = "Select one subnet to ungroup".into(); }
            }
            "duplicate_selection" => { if let Some(id) = self.selected_set.iter().next().cloned() { self.duplicate_node(&id); } }
            "duplicate_linked_selection" => self.duplicate_linked_nodes(),
            "duplicate_objects" => self.duplicate_viewport_objects(),
            "duplicate_viewport_selection" => self.duplicate_viewport_selection(),
            "join_objects" => self.join_viewport_objects(),
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
                    let after=self.selected.clone(); self.add_node(ctx,kind.node(),Some(&after),false);
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
    pub(crate) fn apply(&mut self, ctx: &egui::Context, actions: Vec<UiAction>) {
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
                UiAction::SetAttributeDomain { node, domain } => self.set_attribute_domain(&node, &domain),
                UiAction::PickParameter { node, name, mode, pairs } => {
                    if let Some(active) = &self.param_pick {
                        let same = active.node == node && active.name == name;
                        self.finish_param_pick(ctx, false);
                        if same { continue; }
                    }
                    let original = self.editor.doc.value()["nodes"][&node]["params"][&name].as_str().unwrap_or("").to_owned();
                    let mode = infer_mode(&original, mode); // an edge group stays edges whatever the node's default domain
                    let pairs = pairs || (mode == ui::SelectionMode::Edges && !original.contains('p') && original.contains('-'));
                    let source = input_source(self.editor.doc.value(), &node, &self.display);
                    let ready = pick_source_ready(&self.display, &self.display_ranges, &source);
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
                UiAction::StartScaleCage => {
                    if self.modal.is_none() {
                        self.start_modal(ctx, ui::modal_transform::Kind::Scale);
                        if let Some(modal) = &mut self.modal { modal.cage = true; }
                    }
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
                    self.finish_history();
                    let previous = self.editor.doc.value().clone();
                    let outcome = if undo { self.editor.history.undo_outcome(&mut self.editor.doc) } else { self.editor.history.redo_outcome(&mut self.editor.doc) };
                    if let Some(HistoryOutcome::Selection(selection))=outcome {
                        self.apply_selection_state(selection);
                    } else if let Some(HistoryOutcome::DocEdit { edit:command, selection }) = outcome {
                        self.pending_loop_selection=None;
                        self.pending_restore_selection=selection.map(|state| {
                            let selection=restore(state.selection);
                            let stash=restore(state.stashed);
                            let user=(undo && !self.selection.is_empty() && self.selection!=selection).then(||self.selection.clone());
                            (selection,stash,user)
                        });
                        if undo {
                            if let Some((selection,..))=&self.pending_restore_selection {
                                if selection.mode!=ui::SelectionMode::Object { self.editor_state.apply(ui::EditorChange::Selection(selection.mode)); }
                            }
                        }
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
                UiAction::Select(id) => {
                    self.autofill_selected_node(&id);
                    select_active_nodes(&mut self.selected, &mut self.selected_set, vec![id], false, false);
                }
                UiAction::NavigateLink { node, param } => {
                    select_active_nodes(&mut self.selected, &mut self.selected_set, vec![node.clone()], false, false);
                    ui::network::focus(ctx, &self.editor.path, &node);
                    ctx.data_mut(|d| d.insert_temp(egui::Id::new("link_flash"), (node,param,ctx.input(|i|i.time))));
                }
                UiAction::LinkVisibility(show) => { self.prefs.show_parameter_links=show; let _=self.prefs.save(); },
                UiAction::OpenRenameNode(id) => ui::network::open_rename_dialog(ctx, &id),
                UiAction::RenameNode { old, new } => {
                    if let Some(value)=graph_edit::rename(self.editor.doc.value(),&old,&new) {
                        if self.commit_document(value,"rename node") { self.selected=new.clone(); self.selected_set=std::collections::BTreeSet::from([new]); }
                    } else {self.file_status="Could not rename node: name is invalid or already in use".into();}
                }
                UiAction::ToggleSelect(id) => select_active_nodes(&mut self.selected, &mut self.selected_set, vec![id], true, true),
                UiAction::SelectSet { ids, extend } => select_active_nodes(&mut self.selected, &mut self.selected_set, ids, extend, false),
                UiAction::SetDisplay(id) => {
                    self.display = id.clone();
                    self.finish_history();
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
                    if !continuous { self.finish_history(); }
                    if let Edit::Replace(value) = edit { self.commit_document(value, "edit linked parameter"); }
                    else { self.edit(edit, continuous); }
                }
                UiAction::Time(edit) => self.send_timeline_time(edit),
                UiAction::AddNode { node_type, .. } if node_type.starts_with("sketch_") => {
                    if let Some(mode) = ui::sketch::SketchMode::DRAW.into_iter().find(|m| m.command_id() == node_type) { self.start_sketch(ctx, mode); }
                }
                UiAction::AddNode { node_type, after, tab_menu } => self.add_node(ctx, &node_type, after.as_deref(), tab_menu),
                UiAction::AddNetworkNode { node_type, pos, wire } => {
                    if let Some((mut value, id)) = graph_edit::add(self.editor.doc.value(), &node_type, None) {
                        if node_type == "subnet" { subnet_edit::initialize(&mut value, &id); }
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
                        self.finish_history();
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
                        if node_type == "subnet" { subnet_edit::initialize(&mut value, &id); }
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
                            self.finish_history();
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
                    if let Some((_,inner,_,_))=ui::network::current_scope(ctx,&self.editor.path,self.editor.doc.value()) {
                        ui::network::tidy(ctx, &self.editor.path, &inner, &self.flags);
                    } else {ui::network::tidy(ctx, &self.editor.path, self.editor.doc.value(), &self.flags);}
                    self.last_operator = Some(LastOperator::Tidy);
                }
                UiAction::RepeatLast => self.repeat_last(ctx),
                UiAction::BlenderBar(on) => self.blender_bar = on,
                action @ (UiAction::BlenderInstall(_) | UiAction::BlenderOpen | UiAction::BlenderDownloadAddon
                    | UiAction::BlenderDetect | UiAction::BlenderRestartLink) => self.blender_action(action, ctx),
                UiAction::ToggleLocked(id) => { let f = self.flags.entry(id).or_default(); f.locked = !f.locked; }
                UiAction::ToggleTemplate(id) => { let f = self.flags.entry(id).or_default(); f.template = !f.template; }
                // The scoped parameter panel converts this command to Edit::Replace.
                UiAction::AddSubnetOutput(_) => {}
            }
        }
        if self.selected != prev_selected {
            self.send(Edit::Preview(self.selected.clone()));
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
