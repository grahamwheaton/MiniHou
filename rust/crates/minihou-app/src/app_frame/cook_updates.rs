use super::*;

impl App {
    pub(super) fn accept_frame_cook(&mut self, ctx: &egui::Context) {
        if let Some(response) = self.worker.poll() {
            let display_ranges = response.display_ranges.clone();
            if response.revision == self.cooking.requested {
                if let Some(link) = &self.link { link.cooked(&response); }
            }
            let accepted=self.cooking.accept(response);
            if display_is_empty(self.editor.doc.value()) {
                self.cooking.geometry=None;self.cooking.lod=None;self.geo=None;
            }
            if let Some(mut scene) = accepted.filter(|_| !display_is_empty(self.editor.doc.value())) {
                self.display_ranges = display_ranges;
                self.output_range_count = self.display_ranges.len();
                let preview = self.cooking.preview.as_ref().filter(|(id, _, _)| id == &self.selected).map(|(_, geo, _)| geo.as_ref());
                self.selected_preview = include_selected_preview(&mut scene, &mut self.display_ranges, &self.selected, preview);
                self.picked_field = None;
                self.selection_dirty = true;
                let first = self.gpu.is_none();
                self.bounds = (scene.center, scene.radius);
                self.materials.register_graph(self.editor.doc.value()); let _=self.materials.register_sidecar(&self.editor.path);
                let textured=scene.has_textured_slots();
                let level = if textured {1.0}else{self.proxy_level(scene.triangles())};
                let use_lod = !self.selected_preview && !textured && self.editor_state.proxy.clustered && self.cooking.lod.is_some();
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
                    if !pick.ready && pick_source_ready(&self.display, &self.display_ranges, &pick.source) {
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
                if self.pending_duplicate_move.as_ref().is_some_and(|id| active_range(id, &self.display_ranges).is_some()) {
                    self.pending_duplicate_move = None;
                    self.start_modal(ctx, ModalKind::Move);
                }
                self.sync_selection_baseline();
                self.elem_topo = Default::default();
                if self.param_pick.as_ref().is_some_and(|p| p.load && p.ready) { self.pick_load(); }
                if first { self.frame_mesh(); }
            }
        }
    }
    pub(super) fn refresh_frame_scene(&mut self, ctx: &egui::Context) {
        if self.cooking.cooking() { ctx.request_repaint_after(Duration::from_millis(16)); }
        // the proxy level changed (menu, auto budget): re-upload the reduced scene
        if let Some(scene) = &self.scene {
            let textured=scene.has_textured_slots();
            let level = if textured {1.0}else{self.proxy_level(scene.triangles())};
            let use_lod = !self.selected_preview && !textured && self.editor_state.proxy.clustered && self.cooking.lod.is_some();
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
    }
}
