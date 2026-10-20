use super::*;

impl App {
    pub(super) fn start_modal(&mut self, ctx: &egui::Context, kind: ModalKind) {
        if self.modal.is_some() || self.display.is_empty() || self.scene.is_none() { return; }
        if self.editor_state.selection == ui::SelectionMode::Object && self.active_object().is_none() { return; }
        if self.editor_state.selection != ui::SelectionMode::Object && self.selected_points().is_empty() {
            self.file_status = "Select points, edges, or faces to transform".into(); return;
        }
        let pointer = ctx.input(|i| i.pointer.hover_pos()).unwrap_or(self.last_viewport.center());
        let scene = self.scene.as_ref().unwrap(); let positions = scene.positions();
        let ids = self.selected_points();
        let points: Vec<[f32;3]> = if self.editor_state.selection == ui::SelectionMode::Object {
            self.display_ranges.iter().filter(|(id, _)| self.selected_set.contains(id))
                .flat_map(|(_, range)| range.clone()).filter_map(|id| positions.get(id as usize*3..id as usize*3+3).map(|p|[p[0],p[1],p[2]])).collect()
        } else if ids.is_empty() { positions.as_chunks::<3>().0.to_vec() }
            else { ids.into_iter().filter_map(|id| positions.get(id as usize*3..id as usize*3+3).map(|p|[p[0],p[1],p[2]])).collect() };
        if points.is_empty() { return; }
        let lo = [0,1,2].map(|i|points.iter().map(|p|p[i]).fold(f32::INFINITY,f32::min));
        let hi = [0,1,2].map(|i|points.iter().map(|p|p[i]).fold(f32::NEG_INFINITY,f32::max));
        let pivot = if self.editor_state.pivot == ui::Pivot::Cursor { self.editor_state.cursor }
            else if self.editor_state.selection != ui::SelectionMode::Object && self.editor_state.pivot == ui::Pivot::BoundingBoxCenter { [0,1,2].map(|i|(lo[i]+hi[i])*0.5) }
            else { self.selection_center().unwrap_or([0,1,2].map(|i|(lo[i]+hi[i])*0.5)) };
        let mut modal=Modal::new(kind,pointer,pivot,(lo,hi),self.editor_state.snap.to,
            self.editor_state.proportional.enabled,self.editor_state.proportional.radius);
        modal.units=Some(self.prefs.units);
        let transform_node = if self.editor_state.selection == ui::SelectionMode::Object { &self.selected } else { &self.display };
        let trs = transform_math::TrsParams::from_params(&self.editor.doc.value()["nodes"][transform_node]["params"]);
        modal.rotation = transform_math::matrix_to_euler_xyz(&trs.world_rotation(), [0.; 3]).map(|x| x as f32);
        self.modal=Some(modal);
        ctx.request_repaint();
    }
    pub(super) fn modal_snap_feature(&self, at: egui::Pos2, snap_to: ui::SnapTargets) -> Option<picking::SnapFeature> {
        let scene = self.scene.as_ref()?;
        picking::snap_feature(scene.positions(), scene.triangle_indices(), self.camera_info(),
            self.last_viewport, at, snap_to, self.editor_state.snap.backface_culling)
    }
    pub(super) fn modal_snap_point(&self, at: egui::Pos2, snap_to: impl Into<ui::SnapTargets>) -> Option<[f32;3]> {
        self.modal_snap_feature(at,snap_to.into()).map(|feature|feature.point)
    }
    pub(super) fn finish_modal(&mut self, modal: Modal) {
        if modal.cage && self.editor_state.selection == ui::SelectionMode::Object
            && minihou_core::nodes::get("match_size_legacy").is_some()
            && self.editor.doc.display_nodes().len()==1
            && modal.last.iter().all(|v| *v >= 0.) {
            if let Some(target) = &self.transform_target {
                if self.transform_before.as_ref().is_some_and(|before|before.value()["nodes"][target]["type"]=="transform") {
                    self.last_operator = Some(LastOperator::Scale { factor: modal.last, pivot: modal.pivot });
                    self.finish_transform_from(true); return;
                }
                let pivot=[0,1,2].map(|i|self.editor.doc.value()["nodes"][target]["params"]["pivot"].get(i)
                    .and_then(Value::as_f64).unwrap_or(modal.pivot[i] as f64) as f32);
                let (center,size)=ui::modal_transform::cage_target(modal.bounds,pivot,modal.last);
                let mut value=self.editor.doc.value().clone();
                value["nodes"][target]["type"]=json!("match_size_legacy");
                value["nodes"][target]["params"]=json!({"translate":true,"center":center,"justify_z":"center",
                    "scale":true,"size":size,"uniform":false});
                if let Ok(doc)=Document::parse(value.to_string()) { self.editor.doc=doc; self.send(Edit::Replace(value)); }
            }
        }
        self.last_operator = Some(match modal.kind {
            ModalKind::Move => LastOperator::Move {
                delta: modal.last,
                constraint: if modal.constraint == Constraint::Free { None } else { Some(modal.constraint) },
                orientation: if matches!(modal.constraint, Constraint::Axis(_, true) | Constraint::Plane(_, true)) {
                    ui::Orientation::Local
                } else {
                    self.editor_state.orientation
                },
                proportional: if modal.proportional { Some(self.editor_state.proportional) } else { None },
            },
            ModalKind::Rotate => LastOperator::Rotate {
                angle: modal.last,
                pivot: modal.sketch_pivot.unwrap_or(modal.pivot),
                orientation: self.editor_state.orientation,
                axis_rotation: (modal.kind != ModalKind::Move).then_some(
                    if matches!(self.editor_state.orientation, ui::Orientation::Local | ui::Orientation::Normal | ui::Orientation::Gimbal) {
                        modal.rotation
                    } else {
                        [0.; 3]
                    }
                ),
            },
            ModalKind::Scale => LastOperator::Scale {
                factor: modal.last,
                pivot: modal.pivot,
            },
        });
        self.finish_transform_from(true);
    }
    pub(super) fn sketch_commit(&mut self, name: &str, points: &[[f32; 3]], closed: bool, refs: Vec<Option<Value>>) {
        let res = if name == "Bezier" {
            graph_edit::add_bezier_sketch(self.editor.doc.value(), name, points, closed, &self.selected)
        } else {
            graph_edit::add_sketch(self.editor.doc.value(), name, points, closed, &self.selected)
        };
        let Some((mut value, id)) = res else {
            self.file_status = "Could not add sketch".into(); return;
        };
        attach_inference_refs(&mut value, &id, refs);
        if self.commit_document(value, "add sketch") { self.selected = id.clone(); self.selected_set = std::collections::BTreeSet::from([id.clone()]); self.display = id; }
    }
    pub(super) fn sketch_batch(&mut self, strokes: &[ui::sketch::SketchStroke], selected: &str, target: graph_edit::SketchTarget) {
        let rows: Vec<_> = strokes.iter().map(|s| (s.name.as_str(), s.points.as_slice(), s.closed)).collect();
        let Some((mut value, id, stroke_ids)) = graph_edit::add_sketch_batch(self.editor.doc.value(), &rows, selected, target) else {
            self.file_status = "Could not add sketch".into(); return;
        };
        // References belong to their stroke's source node; the batch itself is
        // still committed as one undo step.
        for (stroke, stroke_id) in strokes.iter().zip(&stroke_ids) { attach_inference_refs(&mut value, stroke_id, stroke.refs.clone()); }
        if self.commit_document(value, "add sketches") {
            self.selected = id.clone(); self.selected_set = std::collections::BTreeSet::from([id.clone()]);
            self.display = minihou_app::display_nodes(self.editor.doc.value()).into_iter().next().unwrap_or(id);
        }
    }
    pub(super) fn add_measurement(&mut self, a: [f32; 3], b: [f32; 3], mut refs: [Option<Value>;2]) {
        let mut value = self.editor.doc.value().clone();
        if !value["measurements"].is_array() { value["measurements"] = json!([]); }
        let stamp=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
        for reference in refs.iter_mut().flatten() {
            let Some(node)=reference["node"].as_str().map(str::to_owned) else { continue; };
            if !value["nodes"][&node].is_object() { *reference=Value::Null; continue; }
            if !value["nodes"][&node]["measurement_identity"].is_string() {
                value["nodes"][&node]["measurement_identity"]=json!(format!("measure-{stamp}-{node}"));
            }
            reference["identity"]=value["nodes"][&node]["measurement_identity"].clone();
        }
        if let Some(rows) = value["measurements"].as_array_mut() {
            rows.push(json!({"id":format!("m-{stamp}"),"points":[a,b],"refs":refs,"a":a,"b":b}));
        }
        self.commit_document(value, "add measurement");
    }
    pub(super) fn delete_measurement(&mut self,index:usize) {
        if let Some(value)=graph_edit::delete_measurement(self.editor.doc.value(),index) {
            self.commit_document(value,"delete measurement");
        }
    }
    pub(super) fn clear_measurements(&mut self) {
        if let Some(value)=graph_edit::clear_measurements(self.editor.doc.value()) {
            self.commit_document(value,"clear measurements");
        }
    }
}
