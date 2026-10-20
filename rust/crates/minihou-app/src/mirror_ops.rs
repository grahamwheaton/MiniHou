use super::*;

impl App {
    /// Alt+M: start the Mirror tool on the selected node (or adopt a selected mirror node). See ui/mirror_tool.rs.
    pub(super) fn start_mirror(&mut self, ctx: &egui::Context) {
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
    pub(super) fn mirror_select(&mut self, id: Option<String>) {
        let Some(s) = self.mirror.as_mut() else { return; };
        s.active = id.clone();
        let id = id.unwrap_or_else(|| s.previous.clone());
        self.selected = id.clone(); self.selected_set = std::collections::BTreeSet::from([id]);
        self.display = Graph::from_value(self.editor.doc.value()).ok().and_then(|g| g.main_output().ok()).unwrap_or_else(|| self.display.clone());
    }
    pub(super) fn mirror_replace(&mut self, doc: Document) {
        self.editor.doc = doc;
        self.send(Edit::Replace(self.editor.doc.value().clone()));
    }
    pub(super) fn mirror_axis(&mut self, cube: ui::mirror_tool::Cube, world_origin: bool) {
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
    pub(super) fn mirror_set_plane(&mut self, plane: ui::mirror_tool::Plane) {
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
    pub(super) fn finish_mirror(&mut self) {
        if let Some(s) = self.mirror.take() {
            self.editor.history.drop_pending();
            self.record_history(s.before);
        }
    }
    pub(super) fn cancel_mirror(&mut self) {
        if let Some(s) = self.mirror.take() {
            self.editor.history.drop_pending();
            self.editor.doc = s.before;
            self.selected = s.previous.clone(); self.selected_set = std::collections::BTreeSet::from([s.previous]);
            self.display = Graph::from_value(self.editor.doc.value()).ok().and_then(|g| g.main_output().ok()).unwrap_or_default();
            self.send(Edit::Replace(self.editor.doc.value().clone()));
        }
    }
    pub(super) fn update_mirror(&mut self, ctx: &egui::Context) {
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
}
