use super::*;

impl App {
    pub(crate) fn add_node(&mut self, ctx: &egui::Context, node_type: &str, after: Option<&str>) {
        self.add_node_at(ctx, node_type, after, self.editor_state.cursor);
    }
    pub(crate) fn add_node_at(&mut self, ctx: &egui::Context, node_type: &str, after: Option<&str>, pos: [f32; 3]) {
        let Some((mut value, id)) = graph_edit::add_at(self.editor.doc.value(), node_type, after, pos) else {
            self.file_status = format!("The Rust core has no {node_type} node yet");
            return;
        };
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
        let before = ui::network::position_add(ctx, &self.editor.path, self.editor.doc.value(), &mut value, after);
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
