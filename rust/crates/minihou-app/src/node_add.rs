use super::*;

impl App {
    /// Fill an existing selection node from the current viewport elements when it becomes active.
    pub(crate) fn autofill_selected_node(&mut self, id: &str) {
        let Some(node_type) = self.editor.doc.value()["nodes"][id]["type"].as_str() else { return };
        if !matches!(node_type, "select_path" | "select_loop" | "select_ring") { return; }
        let Some(params) = self.selection_node_params(node_type) else { return; };
        let Some(value) = selection_autofill_doc(self.editor.doc.value(), id, params) else { return; };
        self.finish_history();
        self.commit_document(value, "fill selection node");
    }
    pub(crate) fn add_node(&mut self, ctx: &egui::Context, node_type: &str, after: Option<&str>) {
        self.add_node_at(ctx, node_type, after, self.editor_state.cursor);
    }
    pub(crate) fn add_node_at(&mut self, ctx: &egui::Context, node_type: &str, after: Option<&str>, pos: [f32; 3]) {
        let merge_sources = (node_type == "merge" && self.selected_set.len() > 1)
            .then(|| self.selected_set.iter().cloned().collect::<Vec<_>>());
        let selected_merge = merge_sources.as_ref().and_then(|sources| after.map(|anchor| (sources, anchor)));
        let Some((mut value, id)) = (match selected_merge {
            Some((sources, anchor)) => graph_edit::add_selected_merge(self.editor.doc.value(), sources, anchor),
            None => graph_edit::add_at(self.editor.doc.value(), node_type, after, pos),
        }) else {
            self.file_status = format!("The Rust core has no {node_type} node yet");
            return;
        };
        if node_type == "subnet" { crate::subnet_edit::initialize(&mut value, &id); }
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
        if node_type == "lattice_grid" {
            if let Some(params) = self.active_object()
                .and_then(|object| active_range(object, &self.display_ranges))
                .and_then(|range| self.scene.as_ref()?.positions().get(range.start as usize * 3..range.end as usize * 3))
                .and_then(lattice_grid_fit_params)
            {
                merge_params(&mut value, &id, params);
            }
        }
        let before = if let Some((sources, _)) = selected_merge {
            ui::network::position_add_merge(ctx, &self.editor.path, self.editor.doc.value(), &mut value, sources)
        } else {
            ui::network::position_add(ctx, &self.editor.path, self.editor.doc.value(), &mut value, after)
        };
        self.finish_history();
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
    pub(crate) fn selection_node_params(&self, node_type: &str) -> Option<Value> {
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
}

fn lattice_grid_fit_params(positions: &[f32]) -> Option<Value> {
    if positions.is_empty() || positions.len() % 3 != 0 { return None; }
    let (lo, hi) = minihou_core::preview::bounds(positions);
    if (0..3).any(|axis| !lo[axis].is_finite() || !hi[axis].is_finite() || hi[axis] <= lo[axis]) { return None; }
    Some(json!({
        "center": ([0, 1, 2].map(|axis| (lo[axis] + hi[axis]) * 0.5)),
        "size": ([0, 1, 2].map(|axis| hi[axis] - lo[axis])),
    }))
}

fn selection_autofill_doc(doc: &Value, id: &str, params: Value) -> Option<Value> {
    let mut value = doc.clone();
    merge_params(&mut value, id, params);
    (value != *doc).then_some(value)
}

#[cfg(test)]
mod autofill_tests {
    use super::*;

    #[test]
    fn viewport_selection_fills_existing_node_group_and_domain() {
        let t = element_select::Topology::new(
            &[0., 0., 0., 1., 0., 0., 1., 1., 0., 0., 1., 0.],
            &[0, 1, 2, 0, 2, 3], &[0, 1, 1, 2, 2, 3, 3, 0]);
        let selection = picking::Selection { mode: ui::SelectionMode::Edges, edges: vec![(0, 1)], ..Default::default() };
        let doc = json!({"nodes":{"path1":{"type":"select_path","params":{"waypoints":"old","domain":"points","method":"fewest_steps"}}}});
        let params = t.node_params("select_path", &selection).unwrap();
        let filled = selection_autofill_doc(&doc, "path1", params.clone()).unwrap();
        assert_eq!(filled["nodes"]["path1"]["params"], json!({"waypoints":"p0-1","domain":"edges","method":"fewest_steps"}));
        assert!(selection_autofill_doc(&filled, "path1", params).is_none());
        assert!(t.node_params("select_path", &picking::Selection { mode: ui::SelectionMode::Edges, ..Default::default() }).is_none());
    }

    #[test]
    fn tab_add_attribute_create_merges_selection_into_new_node() {
        let doc = json!({"display":"box1","nodes":{"box1":{"type":"box"}}});
        let (mut added, id) = graph_edit::add_at(&doc, "attribute_create", Some("box1"), [0.; 3]).unwrap();
        let t = element_select::Topology::new(
            &[0.,0.,0., 1.,0.,0., 1.,1.,0., 0.,1.,0.],
            &[0,1,2, 0,2,3], &[0,1, 1,2, 2,3, 3,0]);
        let selection = picking::Selection { mode:ui::SelectionMode::Faces, tris:vec![0,1], ..Default::default() };
        merge_params(&mut added, &id, t.node_params("attribute_create", &selection).unwrap());
        assert_eq!(added["nodes"][&id]["params"], json!({
            "group":"0", "domain":"primitives", "group_type":"primitives"}));
    }

    #[test]
    fn tab_added_lattice_grid_fits_selected_object_bounds() {
        let doc = json!({"display":"box1","nodes":{"box1":{"type":"box"}}});
        let (mut added, id) = graph_edit::add_at(&doc, "lattice_grid", Some("box1"), [0.; 3]).unwrap();
        let object_positions = [-3., 2., 1., 5., 8., 9., 1., -4., 3.];
        merge_params(&mut added, &id, lattice_grid_fit_params(&object_positions).unwrap());

        let params = &added["nodes"][&id]["params"];
        let center = params["center"].as_array().unwrap();
        let size = params["size"].as_array().unwrap();
        let lo = [-3., -4., 1.];
        let hi = [5., 8., 9.];
        for axis in 0..3 {
            let center = center[axis].as_f64().unwrap() as f32;
            let size = size[axis].as_f64().unwrap() as f32;
            assert_eq!(center - size * 0.5, lo[axis]);
            assert_eq!(center + size * 0.5, hi[axis]);
        }
    }
}
