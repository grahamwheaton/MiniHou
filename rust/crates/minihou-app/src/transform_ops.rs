use super::*;

impl App {
    pub(super) fn selection_normal_frame(&self, trs: &transform_math::TrsParams) -> transform_math::Frame {
        let local = transform_math::Frame::from_rotation(&trs.world_rotation());
        let normal = self.element_metrics().normal;
        if transform_math::length3(normal) < 1e-9 { return local; }
        transform_math::Frame::from_normal(normal, local.axis(0))
    }
    pub(super) fn orientation_frame(&self, orientation: ui::Orientation, trs: &transform_math::TrsParams) -> transform_math::Frame {
        match orientation {
            ui::Orientation::View => {
                let c = self.camera_info();
                transform_math::Frame::from_f32([c.ray_right, c.ray_up, c.ray_forward])
            }
            ui::Orientation::Local | ui::Orientation::Gimbal => {
                transform_math::Frame::from_rotation(&trs.world_rotation())
            }
            ui::Orientation::Normal => self.selection_normal_frame(trs),
            ui::Orientation::Cursor => transform_math::Frame::from_rotation(
                &transform_math::euler_xyz(self.editor_state.cursor_rotation.map(f64::from))),
            _ => transform_math::Frame::global(),
        }
    }
    pub(super) fn transform_node(&mut self, op: TransformOp) {
        let pivot = if matches!(op, TransformOp::Move(_)) { None }
            else if let Some(pivot) = self.transform_pivot { Some(pivot) }
            else if self.editor_state.pivot == ui::Pivot::Cursor { Some(self.editor_state.cursor) }
            else { Some(self.selection_center().unwrap_or(self.bounds.0)) };
        self.transform_node_at(op, pivot, self.transform_orientation);
    }
    pub(super) fn transform_node_at(&mut self, op: TransformOp, pivot: Option<[f32; 3]>, orientation_override: Option<ui::Orientation>) {
        if self.editor_state.selection == ui::SelectionMode::Object && self.active_object().is_none() { return; }
        let Some(active)=self.active_object().map(str::to_owned) else {return;};
        let offset=active_range(&active,&self.display_ranges).map(|r|r.start).unwrap_or(0);
        let group = group_text(&self.selected_points().iter().map(|p|p-offset).collect::<Vec<_>>());
        if self.editor_state.selection != ui::SelectionMode::Object && group.is_empty() {
            self.file_status = "Select points, edges, or faces to transform".into();
            return;
        }
        let first = self.transform_before.is_none();
        if first { self.finish_history(); self.transform_before = Some(self.editor.doc.clone()); }
        let before_doc = self.transform_before.as_ref().unwrap();
        let mut value = self.editor.doc.value().clone();
        // The displayed node changes to the new node during a drag. Keep reading
        // the original matrix from the document saved at drag start.
        let displayed = &active;
        let source = if before_doc.value()["nodes"].get(displayed).is_some() {
            displayed.clone()
        } else {
            value["nodes"][&self.display]["inputs"]["geo"].as_str().unwrap_or(&self.display).to_owned()
        };
        let resolved_pivot = pivot.unwrap_or_else(|| {
            if self.editor_state.pivot == ui::Pivot::Cursor {
                self.editor_state.cursor
            } else {
                self.selection_center().unwrap_or(self.bounds.0)
            }
        });
        if first { self.transform_pivot = Some(resolved_pivot); }
        let orientation = orientation_override.unwrap_or(self.editor_state.orientation);
        if first { self.transform_orientation = Some(orientation); }
        let is_local = matches!(orientation, ui::Orientation::Local | ui::Orientation::Gimbal);
        let base = if before_doc.value()["nodes"][&source]["type"] == "transform" {
            transform_math::TrsParams::from_params(&before_doc.value()["nodes"][&source]["params"])
        } else { transform_math::TrsParams::default() };
        let scale_base = if pivot.is_some() && matches!(op, TransformOp::Scale(_)) {
            base.with_world_pivot_preserving_matrix(resolved_pivot.map(f64::from))
        } else { base };
        let base_frame = self.orientation_frame(orientation, &base);
        let shear_second = group.is_empty() && !is_local &&
            before_doc.value()["nodes"][&source]["type"] == "transform" &&
            matches!(op, TransformOp::Scale(f) if matches!(
                transform_math::compose_scale(&scale_base, base_frame, f.map(|x| x as f64), false, 1e-6),
                transform_math::ScaleResult::ShearNeedsSecondNode { .. }));
        let Some(target) = ensure_transform_target(&mut value, &source, group.is_empty() && !shear_second,
            self.transform_target.as_deref()) else { return; };
        // A transform on a node nothing displays (no Output, or an Output elsewhere) would move off-screen: show it.
        if first { show_transform_target(&mut value, &target); }
        let target_existed = before_doc.value()["nodes"].get(&target).is_some();
        let mut initial = if target_existed {
            transform_math::TrsParams::from_params(&before_doc.value()["nodes"][&target]["params"])
        } else { transform_math::TrsParams { pivot: resolved_pivot.map(f64::from), ..Default::default() } };
        if target_existed && pivot.is_some() && !matches!(op, TransformOp::Move(_)) {
            initial = initial.with_world_pivot_preserving_matrix(resolved_pivot.map(f64::from));
        }
        let frame = if shear_second { base_frame } else { self.orientation_frame(orientation, &initial) };
        let new_trs = if shear_second {
            let TransformOp::Scale(factors) = op else { unreachable!() };
            let transform_math::ScaleResult::ShearNeedsSecondNode { second_node, .. } =
                transform_math::compose_scale(&scale_base, base_frame, factors.map(f64::from), false, 1e-6)
                else { unreachable!() };
            second_node
        } else {
            match op {
                TransformOp::Move(delta) => transform_math::compose_translate(&initial, delta.map(f64::from)),
                TransformOp::Rotate(angles) => {
                    let count = angles.iter().filter(|x| x.abs() > 1e-6).count();
                    if count == 1 {
                        let axis = angles.iter().position(|x| x.abs() > 1e-6).unwrap();
                        transform_math::compose_rotate_axis(&initial, frame, axis, angles[axis] as f64, is_local)
                    } else {
                        transform_math::compose_rotate_euler(&initial, frame, angles.map(f64::from), is_local)
                    }
                }
                TransformOp::Scale(factors) => match transform_math::compose_scale(&initial, frame,
                    factors.map(f64::from), is_local, 1e-6) {
                    transform_math::ScaleResult::Direct(trs) => trs,
                    transform_math::ScaleResult::ShearNeedsSecondNode { second_node, .. } => second_node,
                },
            }
        };
        let name = match op { TransformOp::Move(_) => "translate", TransformOp::Rotate(_) => "rotate",
            TransformOp::Scale(_) => "scale" };
        let delta = transform_math::mul4(&new_trs.to_matrix(),
            &transform_math::inverse_affine(&initial.to_matrix()).unwrap_or(transform_math::IDENTITY_4));
        self.transform_delta = Some(LastOperator::MatrixDelta { delta, frame,
            label: match op { TransformOp::Move(_) => "Move", TransformOp::Rotate(_) => "Rotate",
                TransformOp::Scale(_) => "Scale" } });
        let params = &mut value["nodes"][&target]["params"];
        if !params.is_object() { *params = json!({}); }
        params["translate"] = json!(new_trs.translate);
        params["rotate"] = json!(new_trs.rotate);
        params["scale"] = json!(new_trs.scale);
        params["uniform_scale"] = json!(new_trs.uniform_scale);
        params["pivot"] = json!(new_trs.pivot);
        params["axis_rotation"] = json!(new_trs.axis_rotation);
        if first {
            params["group"] = json!(group);
            proportional_params(params, &self.editor_state);
        }
        let param_value = params[name].clone();
        if let TransformOp::Move(delta) = op {
            for peer in &self.duplicate_move_peers {
                if peer == &target { continue; }
                let base = transform_math::TrsParams::from_params(&before_doc.value()["nodes"][peer]["params"]);
                value["nodes"][peer]["params"]["translate"] = json!(transform_math::compose_translate(&base, delta.map(f64::from)).translate);
            }
        }
        let peer_translates: Vec<_> = self.duplicate_move_peers.iter().filter(|peer| *peer != &target)
            .map(|peer| (peer.clone(), value["nodes"][peer]["params"]["translate"].clone())).collect();
        match apply_transform_step(&mut self.editor.doc, first, value, &target, name, param_value) {
            Ok(edits) => {
                let active_is_output = self.editor.doc.value()["nodes"].get(&active).is_some_and(|n| n["type"] == "output");
                if self.editor_state.selection == ui::SelectionMode::Object {
                    if active_is_output {
                        self.selected_set.remove(&self.selected);
                        self.selected_set.insert(active.clone());
                        self.selected = active.clone();
                    } else {
                        self.selected_set.remove(&self.selected);
                        self.selected_set.insert(target.clone());
                        self.selected = target.clone();
                    }
                }
                self.transform_target = Some(target.clone());
                self.display = if active_is_output { active.clone() } else { target.clone() };
                for edit in edits { self.send(edit); }
                if !first {
                    for (peer, translate) in peer_translates {
                        let edit = Edit::Param {node:peer, name:"translate".into(), value:translate};
                        if self.editor.doc.edit(&edit).is_ok() { self.send(edit); }
                    }
                }
                if !first {
                    let mut desired=self.editor.doc.value()["nodes"][&target]["params"].clone();
                    desired["translate"] = json!(new_trs.translate);
                    desired["rotate"] = json!(new_trs.rotate);
                    desired["scale"] = json!(new_trs.scale);
                    desired["uniform_scale"] = json!(new_trs.uniform_scale);
                    desired["pivot"] = json!(new_trs.pivot);
                    desired["axis_rotation"] = json!(new_trs.axis_rotation);
                    proportional_params(&mut desired,&self.editor_state);
                    for name in ["translate", "rotate", "scale", "uniform_scale", "pivot", "axis_rotation",
                        "proportional_enabled","proportional_radius","proportional_falloff","proportional_connected"] {
                        if desired[name] != self.editor.doc.value()["nodes"][&target]["params"][name] {
                            let edit=Edit::Param { node:target.clone(), name:name.into(), value:desired[name].clone() };
                            if self.editor.doc.edit(&edit).is_ok() { self.send(edit); }
                        }
                    }
                }
            }
            Err(e) => self.file_status = format!("Could not transform node: {e}"),
        }
    }
    pub(super) fn finish_transform(&mut self) {
        self.finish_transform_from(false);
    }
    pub(super) fn finish_transform_from(&mut self, is_modal: bool) {
        if let Some(before) = self.transform_before.take() {
            if let Some(delta) = self.transform_delta.take() { self.last_operator = Some(delta); }
            self.record_history(before);
        }
        let _ = is_modal;
        if let Some(target) = self.transform_target.take() {
            if self.editor_state.selection != ui::SelectionMode::Object {
                self.selected_set = std::collections::BTreeSet::from([target.clone()]);
                self.selected = target;
            } else if !self.editor.doc.value()["nodes"].get(&self.selected).is_some_and(|n| n["type"] == "output") {
                self.selected = target;
            }
        }
        self.transform_pivot = None;
        self.transform_orientation = None;
        self.duplicate_move_peers.clear();
    }
    pub(super) fn cancel_transform(&mut self) {
        if let Some(before) = self.transform_before.take() {
            if self.editor_state.selection == ui::SelectionMode::Object && before.value()["nodes"].get(&self.selected).is_none() {
                let source = self.editor.doc.value()["nodes"][&self.selected]["inputs"]["geo"].as_str().unwrap_or("").to_owned();
                self.selected_set.remove(&self.selected);
                if !source.is_empty() { self.selected_set.insert(source.clone()); }
                self.selected = source;
            }
            self.editor.doc = before;
            self.send(Edit::Replace(self.editor.doc.value().clone()));
            self.display = Graph::from_value(self.editor.doc.value()).ok().and_then(|g| g.main_output().ok()).unwrap_or_else(|| self.display.clone());
        }
        self.transform_target = None;
        self.transform_delta = None;
        self.transform_pivot = None;
        self.transform_orientation = None;
        self.duplicate_move_peers.clear();
    }
}
