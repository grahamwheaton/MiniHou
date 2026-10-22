use super::*;

impl App {
    /// Make `value` the document as one undo step and send it to the cook worker; false (with a status message) when it is invalid.
    pub(super) fn commit_document(&mut self, value: serde_json::Value, what: &str) -> bool {
        match graph_edit::validated(value) {
            Ok(new) => {
                let selection=self.selection_state();
                self.editor.history.replace_with_selection(&mut self.editor.doc, new,Some(selection.clone()),Some(selection));
                self.send(Edit::Replace(self.editor.doc.value().clone()));
                true
            }
            Err(e) => { self.file_status = format!("Could not {what}: {e}"); false }
        }
    }
    /// Delete a node and every wire into it.
    pub(super) fn delete_node(&mut self, id: &str) {
        let ids = if self.selected_set.contains(id) { self.selected_set.iter().cloned().collect::<Vec<_>>() } else { vec![id.to_owned()] };
        let Some(value) = graph_edit::delete_set(self.editor.doc.value(), &ids) else { return };
        if self.commit_document(value, "delete the node") {
            if let Ok(g) = Graph::from_value(self.editor.doc.value()) {
                self.selected_set.retain(|node| !ids.contains(node));
                if ids.contains(&self.selected) { self.selected = g.nodes.first().map(|(n, _)| n.clone()).unwrap_or_default(); }
                self.display = g.main_output().unwrap_or_else(|_| self.selected.clone());
            }
            self.last_operator = Some(LastOperator::Delete);
        }
    }
    /// Copy a node (parameters and inputs) under a fresh id and select the copy.
    pub(super) fn duplicate_node(&mut self, id: &str) {
        let ids = if self.selected_set.contains(id) { self.selected_set.iter().cloned().collect::<Vec<_>>() } else { vec![id.to_owned()] };
        let Some(clip) = graph_edit::copy(self.editor.doc.value(), &ids) else { return };
        let Some((value, new_ids)) = graph_edit::paste(self.editor.doc.value(), &clip) else { return };
        if self.commit_document(value, "duplicate the node") {
            self.selected = new_ids[0].clone();
            self.selected_set = new_ids.into_iter().collect();
            self.last_operator = Some(LastOperator::Duplicate);
        }
    }
    pub(super) fn duplicate_linked_nodes(&mut self) {
        let ids: Vec<_> = self.selected_set.iter().cloned().collect();
        let Some((value, copies)) = graph_edit::duplicate_linked(self.editor.doc.value(), &ids) else { return };
        if self.commit_document(value, "duplicate linked nodes") {
            self.selected = copies[0].clone();
            self.selected_set = copies.into_iter().collect();
            self.last_operator = Some(LastOperator::Duplicate);
        }
    }
    pub(super) fn duplicate_viewport_objects(&mut self) {
        if self.editor_state.selection != ui::SelectionMode::Object { return; }
        let ids: Vec<_> = self.selected_set.iter().filter(|id| active_range(id, &self.display_ranges).is_some())
            .cloned().collect();
        let Some((value, copies)) = graph_edit::duplicate_objects(self.editor.doc.value(), &ids) else { return };
        if self.commit_document(value, "duplicate viewport objects") {
            self.selected = copies[0].clone();
            self.selected_set = copies.into_iter().collect();
            self.display = self.selected.clone();
            self.pending_duplicate_move = Some(self.selected.clone());
            self.duplicate_move_peers = self.selected_set.iter().cloned().collect();
            self.last_operator = Some(LastOperator::Duplicate);
        }
    }
    pub(super) fn duplicate_viewport_selection(&mut self) {
        if self.editor_state.selection == ui::SelectionMode::Object {
            self.duplicate_viewport_objects();
            return;
        }
        let Some(source) = self.active_object().map(str::to_owned) else { return };
        let Some(params) = self.selection_node_params("split") else { return };
        let Some((value, [_split, transform, merge])) =
            graph_edit::duplicate_selection_nodes(self.editor.doc.value(), &source, params) else { return };
        if self.commit_document(value, "duplicate viewport selection") {
            self.selected = merge.clone();
            self.selected_set = [merge.clone()].into();
            self.display = merge;
            self.transform_target = Some(transform);
            self.pending_duplicate_move = Some(self.display.clone());
            self.last_operator = Some(LastOperator::Duplicate);
        }
    }
    /// Dissolve a node, rewiring its inputs to consumers.
    pub(super) fn dissolve_node(&mut self, id: &str) {
        let ids = if self.selected_set.contains(id) { self.selected_set.iter().cloned().collect::<Vec<_>>() } else { vec![id.to_owned()] };
        let Some(value) = graph_edit::dissolve_set(self.editor.doc.value(), &ids) else { return; };
        if self.commit_document(value, "dissolve the node") {
            self.selected_set.retain(|node| !ids.contains(node));
            if let Ok(g) = Graph::from_value(self.editor.doc.value()) {
                if ids.contains(&self.selected) { self.selected = g.nodes.first().map(|(n, _)| n.clone()).unwrap_or_default(); }
                self.display = g.main_output().unwrap_or_else(|_| self.selected.clone());
            }
            self.last_operator = Some(LastOperator::Dissolve);
        }
    }
    /// Wire `from` into input `port` of `to` (its first input when `port` is None); multi inputs gain one more wire.
    pub(super) fn connect_nodes(&mut self, from: &str, to: &str, port: Option<&str>) {
        let Some(value) = graph_edit::connect(self.editor.doc.value(), from, to, port) else { return };
        if self.commit_document(value, "connect the nodes") {
            self.last_operator = Some(LastOperator::Connect {
                from: from.to_string(),
                to: to.to_string(),
                port: port.map(ToString::to_string),
            });
        }
    }
    pub(super) fn disconnect_input(&mut self, node: &str, port: &str) {
        if let Some(value) = graph_edit::disconnect(self.editor.doc.value(), node, port) { self.commit_document(value, "disconnect the input"); }
    }
    pub(super) fn repeat_last(&mut self, ctx: &egui::Context) {
        let Some(op) = self.last_operator.clone() else {
            self.file_status = "Nothing to repeat".into();
            return;
        };
        match op {
            LastOperator::Extrude {mut params} => {
                if self.cooking.cooking() {self.file_status="Wait for the extrusion to cook before repeating".into();return;}
                let Some(scene)=self.scene.as_ref() else {return;};
                let Some(source)=self.active_object().map(str::to_owned) else {self.file_status="Select a displayed object".into();return;};
                let range=active_range(&source,&self.display_ranges).unwrap();
                let source_points=(range.end-range.start) as usize;
                let selection=self.active_selection().unwrap_or_default();
                if self.editor_state.selection!=ui::SelectionMode::Object && !selection.is_empty() {
                    params["group"]=json!("");params["edges"]=json!("");params["points"]=json!("");
                    match self.editor_state.selection {
                        ui::SelectionMode::Faces=>{let first=scene.triangle_indices().chunks_exact(3).take_while(|tri|tri[0]<range.start).count() as u32;let topo=element_select::cached(&self.elem_topo,scene);let offset=topo.poly_of_tri(first).unwrap_or(0);params["group"]=json!(group_text(&topo.polys_of(&selection.tris).iter().map(|p|*p-offset).collect::<Vec<_>>()));},
                        ui::SelectionMode::Edges=>params["edges"]=json!(selection.edges.iter().map(|(a,b)|format!("{}-{}",a-range.start,b-range.start)).collect::<Vec<_>>().join(" ")),
                        ui::SelectionMode::Points=>params["points"]=json!(group_text(&selection.points.iter().map(|p|*p-range.start).collect::<Vec<_>>())),_=>{},
                    }
                }
                let next=repeat::extrude_next_params(&params,source_points);
                if let Some((doc,id))=ui::modal_model::insert_graph(&self.editor.doc,&source,ModelKind::Extrude,params) {
                    if self.commit_document(doc.value().clone(),"repeat extrusion") {
                        self.display=Graph::from_value(self.editor.doc.value()).ok().and_then(|g|g.main_output().ok()).unwrap_or(id.clone());
                        self.selected=id.clone();self.selected_set=std::collections::BTreeSet::from([id]);
                        self.last_operator=Some(LastOperator::Extrude {params:next});
                    }
                }
            }
            LastOperator::MatrixDelta { delta, frame, .. } => {
                let group = group_text(&self.selected_points());
                let object_mode = self.editor_state.selection == ui::SelectionMode::Object;
                if !object_mode && group.is_empty() {
                    self.file_status = "Select points, edges, or faces to transform".into();
                    return;
                }
                self.finish_history();
                let before = self.editor.doc.clone();
                let source = self.display.clone();
                let mut value = self.editor.doc.value().clone();
                let source_is_transform = value["nodes"][&source]["type"] == "transform";
                let current = transform_math::TrsParams::from_params(&value["nodes"][&source]["params"]);
                let composed = transform_math::mul4(&delta, &current.to_matrix());
                let candidate = current.decompose_like(&composed);
                let rebuilt = candidate.to_matrix();
                let representable = (0..4).all(|i| (0..4).all(|j|
                    (rebuilt[i][j] - composed[i][j]).abs() < 1e-6));
                let reuse = object_mode && source_is_transform &&
                    value["nodes"][&source]["params"]["group"].as_str().unwrap_or("").is_empty() && representable;
                let Some(target) = ensure_transform_target(&mut value, &source, reuse, None) else { return; };
                let result = if target == source { candidate } else {
                    let basis = frame.matrix();
                    let mut right_handed = basis;
                    if transform_math::det3(&basis) < 0.0 {
                        for row in &mut right_handed { row[2] = -row[2]; }
                    }
                    let reference = transform_math::TrsParams {
                        axis_rotation: transform_math::matrix_to_euler_xyz(&right_handed, [0.; 3]),
                        ..Default::default()
                    };
                    reference.decompose_like(&delta)
                };
                let params = &mut value["nodes"][&target]["params"];
                if !params.is_object() { *params = json!({}); }
                params["translate"] = json!(result.translate);
                params["rotate"] = json!(result.rotate);
                params["scale"] = json!(result.scale);
                params["uniform_scale"] = json!(result.uniform_scale);
                params["pivot"] = json!(result.pivot);
                params["axis_rotation"] = json!(result.axis_rotation);
                if target != source { params["group"] = json!(group); }
                match Document::parse(value.to_string()) {
                    Ok(doc) => {
                        self.editor.doc = doc;
                        self.send(Edit::Replace(value));
                        self.record_history(before);
                        self.finish_history();
                        self.display = target.clone();
                        self.selected = target.clone();
                        self.selected_set = std::collections::BTreeSet::from([target]);
                    }
                    Err(e) => self.file_status = format!("Could not repeat transform: {e}"),
                }
            }
            LastOperator::Move { .. } | LastOperator::Rotate { .. } | LastOperator::Scale { .. } => {
                let group = group_text(&self.selected_points());
                let is_object = self.editor_state.selection == ui::SelectionMode::Object;
                if !is_object && group.is_empty() {
                    self.file_status = "Select points, edges, or faces to transform".into();
                    return;
                }
                self.finish_history();
                let before = self.editor.doc.clone();
                let source = self.display.clone();
                let mut value = self.editor.doc.value().clone();

                let can_reuse = if let Some(node) = value["nodes"].get(&source) {
                    if node["type"] == "transform" {
                        if is_object {
                            node["params"].get("group").and_then(Value::as_str).unwrap_or("").is_empty()
                        } else {
                            node["params"].get("group").and_then(Value::as_str) == Some(&group)
                        }
                    } else {
                        false
                    }
                } else {
                    false
                };

                let target = if can_reuse {
                    source
                } else {
                    let Some(t) = ensure_transform_target(&mut value, &source, is_object, None) else { return; };
                    t
                };
                let params = &mut value["nodes"][&target]["params"];
                if !params.is_object() { *params = json!({}); }
                if !can_reuse { params["group"] = json!(group); }
                match &op {
                    LastOperator::Rotate { pivot, axis_rotation, .. } => {
                        params["pivot"] = json!(pivot);
                        if let Some(rot) = axis_rotation { params["axis_rotation"] = json!(rot); }
                    }
                    LastOperator::Scale { pivot, .. } => params["pivot"] = json!(pivot),
                    LastOperator::Move { proportional, .. } => {
                        let mut state = self.editor_state;
                        state.proportional = proportional.unwrap_or_default();
                        proportional_params(params, &state);
                    }
                    _ => {}
                }
                if value != *self.editor.doc.value() {
                    match Document::parse(value.to_string()) {
                        Ok(doc) => { self.editor.doc = doc; self.send(Edit::Replace(value)); }
                        Err(e) => { self.file_status = format!("Could not repeat transform: {e}"); return; }
                    }
                }

                match repeat::apply_repeat_transform(&mut self.editor.doc, &target, &op) {
                    Ok(edit) => {
                        self.send(edit);
                        self.record_history(before);
                        self.finish_history();
                        self.selected = target.clone();
                        self.selected_set = std::collections::BTreeSet::from([target.clone()]);
                        self.display = target;
                    }
                    Err(e) => {
                        self.file_status = format!("Could not repeat transform: {e}");
                    }
                }
            }
            LastOperator::AddNode { node_type, after_selection } => {
                let after = if after_selection { Some(self.selected.clone()) } else { None };
                self.add_node(ctx, &node_type, after.as_deref(), false);
            }
            LastOperator::Duplicate => {
                if let Some(sel) = self.selected_set.iter().next().cloned() { self.duplicate_node(&sel); }
            }
            LastOperator::Dissolve => {
                if let Some(sel) = self.selected_set.iter().next().cloned() { self.dissolve_node(&sel); }
            }
            LastOperator::Delete => {
                if let Some(sel) = self.selected_set.iter().next().cloned() { self.delete_node(&sel); }
            }
            LastOperator::Connect { from, to, port } => {
                self.connect_nodes(&from, &to, port.as_deref());
            }
            LastOperator::Tidy => {
                ui::network::tidy(ctx, &self.editor.path, self.editor.doc.value(), &self.flags);
                self.last_operator = Some(LastOperator::Tidy);
            }
        }
    }
}
