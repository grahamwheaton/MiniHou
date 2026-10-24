use super::*;

impl App {
    pub(super) fn start_model_modal(&mut self, ctx: &egui::Context, kind: ModelKind) {
        if self.modal.is_some() || self.model_modal.is_some() || self.scene.is_none() { return; }
        let Some(scene)=self.scene.as_ref() else { return; };
        let Some(range)=self.active_object().and_then(|id|active_range(id,&self.display_ranges)).cloned() else { self.file_status="Select a displayed object".into(); return; };
        let source_points=(range.end-range.start) as usize;
        let tri_offset=scene.triangle_indices().chunks_exact(3).take_while(|tri|tri[0]<range.start).count() as u32;
        let poly_offset=element_select::cached(&self.elem_topo,scene).poly_of_tri(tri_offset).unwrap_or(0);
        let Some(mut selection)=self.active_selection() else { self.file_status="Select a displayed object".into(); return; };
        selection.mode=match self.editor_state.selection { ui::SelectionMode::Points => ui::SelectionMode::Points,
            ui::SelectionMode::Edges => ui::SelectionMode::Edges,
            ui::SelectionMode::Faces => ui::SelectionMode::Faces,
            ui::SelectionMode::Object => { self.file_status="Select mesh elements for this tool".into(); return; } };
        let params=if kind==ModelKind::Bevel {
            if selection.mode!=ui::SelectionMode::Edges {
                selection=element_select::cached(&self.elem_topo,scene).convert(&selection,ui::SelectionMode::Edges);
            }
            if selection.edges.is_empty() { self.file_status="Select edges to bevel".into(); return; }
            json!({"edges":selection.edges.iter().map(|(a,b)|format!("{}-{}",a-range.start,b-range.start)).collect::<Vec<_>>().join(" "),"distance":0.,"divisions":1})
        } else if kind==ModelKind::Extrude && selection.mode!=ui::SelectionMode::Faces {
            let mut params=match selection.mode {
                ui::SelectionMode::Edges => json!({"edges":selection.edges.iter().map(|(a,b)|format!("{}-{}",a-range.start,b-range.start)).collect::<Vec<_>>().join(" "),"distance":0.}),
                ui::SelectionMode::Points => json!({"points":group_text(&selection.points.iter().map(|p|*p-range.start).collect::<Vec<_>>()),"distance":0.}),
                _ => unreachable!(),
            };
            if selection.is_empty() { self.file_status="Select mesh elements for this tool".into(); return; }
            {
                let points=selected_point_ids(&selection,selection.mode,scene.triangle_indices());
                selection.tris=scene.triangle_indices().chunks_exact(3).enumerate()
                    .filter(|(_,tri)| tri.iter().any(|p| points.contains(p)))
                    .map(|(tri,_)|tri as u32).collect();
            }
            params["group"]=json!("");
            params
        } else {
            if selection.tris.is_empty() {
                self.file_status="Select mesh elements for this tool".into(); return;
            }
            let faces=element_select::cached(&self.elem_topo,scene).polys_of(&selection.tris);
            let mut params=json!({"group":group_text(&faces.iter().map(|p|*p-poly_offset).collect::<Vec<_>>())});
            params[kind.parameter()]=json!(0.);
            params
        };
        let source=self.active_object().unwrap().to_owned();
        let Some((doc,id))=ui::modal_model::insert_graph(&self.editor.doc,&source,kind,params) else {
            self.file_status=format!("Could not add {}",kind.node()); return;
        };
        self.model_before_selection=Some((self.selection.clone(),self.stashed_selection.clone()));
        self.extrude_keep=keeps_face_selection(kind,self.editor_state.selection).then(||self.selection.clone());
        self.finish_history();
        let before=self.editor.doc.clone();
        self.editor.doc=doc;
        self.send(Edit::Replace(self.editor.doc.value().clone()));
        let previous=self.selected.clone();
        self.selected=id.clone(); self.selected_set=std::collections::BTreeSet::from([id.clone()]);
        self.display=Graph::from_value(self.editor.doc.value()).ok().and_then(|g|g.main_output().ok()).unwrap_or(id.clone());
        let pointer=ctx.input(|i|i.pointer.hover_pos()).unwrap_or(self.last_viewport.center());
        let radius=self.bounds.1.max(0.001);
        let mut state=ModelState::new(kind,pointer,(radius/200.).max(0.00001));
        state.units=Some(self.prefs.units); state.source_points=source_points;
        if matches!(kind,ModelKind::Extrude | ModelKind::PushPull) {
            let scene=self.scene.as_ref().unwrap();
            let tris=scene.triangle_indices(); let positions=scene.positions();
            let mut normal=[0f32;3]; let mut center=[0f32;3]; let mut count=0f32;
            for &tri in &selection.tris {
                if let Some(ids)=tris.get(tri as usize*3..tri as usize*3+3) {
                    let a=picking::at(positions,ids[0]); let b=picking::at(positions,ids[1]); let c=picking::at(positions,ids[2]);
                    let u=[0,1,2].map(|k|b[k]-a[k]); let v=[0,1,2].map(|k|c[k]-a[k]);
                    let n=[u[1]*v[2]-u[2]*v[1],u[2]*v[0]-u[0]*v[2],u[0]*v[1]-u[1]*v[0]];
                    for k in 0..3 { normal[k]+=n[k]; center[k]+=(a[k]+b[k]+c[k])/3.; }
                    count+=1.;
                }
            }
            if count>0. && kind==ModelKind::PushPull {
                // SketchUp-style inference: far face (Through) and parallel faces (On Face).
                if let Some(local_geo)=self.cooking.geometry.as_deref().and_then(|g|ui::mesh_cut::object_geometry(g,&range)) {
                    let local_scene=Scene::new(&local_geo);
                    let local_tris=selection.tris.iter().map(|tri|*tri-tri_offset).collect::<Vec<_>>();
                    state.targets=ui::modal_model::pushpull_targets(local_scene.positions(),local_scene.triangle_indices(),&local_tris,normal);
                }
            }
            if count>0. {
                center=center.map(|x|x/count);
                let length=normal.iter().map(|x|x*x).sum::<f32>().sqrt();
                if length>1e-6 {
                    normal=normal.map(|x|x/length);
                    let tip=[0,1,2].map(|k|center[k]+normal[k]*radius*0.25);
                    if let (Some(a),Some(b))=(ui::transform_gizmo::project(self.camera_info(),self.last_viewport,center),ui::transform_gizmo::project(self.camera_info(),self.last_viewport,tip)) {
                        let delta=b-a; if delta.length()>1. { state.screen_axis=delta.normalized(); }
                    }
                }
            }
        }
        self.model_modal=Some((state,before,id,previous));
    }
    pub(super) fn finish_model_modal(&mut self) {
        self.extrude_keep=None;
        if let Some((state,before,id,_))=self.model_modal.take() {
            if state.kind==ModelKind::Extrude {
                self.last_operator=Some(LastOperator::Extrude {params:repeat::extrude_next_params(&self.editor.doc.value()["nodes"][&id]["params"],state.source_points)});
            }
            let before_selection=self.model_before_selection.take().map(|(selection,stash)|selection_state(&selection,&stash));
            let after_selection=self.selection_state();
            self.editor.history.record_with_selection(before,&self.editor.doc,before_selection,Some(after_selection));
        }
    }
    pub(super) fn cancel_model_modal(&mut self) {
        if let Some((_,before,_,previous))=self.model_modal.take() {
            self.model_before_selection=None; self.extrude_keep=None;
            self.editor.doc=before;
            self.selected=previous.clone(); self.selected_set=std::collections::BTreeSet::from([previous]);
            self.display=Graph::from_value(self.editor.doc.value()).ok().and_then(|g|g.main_output().ok()).unwrap_or_default();
            self.send(Edit::Replace(self.editor.doc.value().clone()));
        }
    }
    pub(super) fn update_model_modal(&mut self, ctx: &egui::Context) {
        let Some((mut state,before,id,previous))=self.model_modal.take() else { return; };
        let (events,pointer,modifiers,scroll)=ctx.input(|i|(i.events.clone(),i.pointer.hover_pos(),i.modifiers,i.raw_scroll_delta.y));
        let mut confirm=false; let mut cancel=false;
        for event in events {
            match event {
                egui::Event::Key { key,pressed:true,.. } => match state.key_event(key,modifiers.ctrl) {
                    ModelOutcome::Cancel=>cancel=true, ModelOutcome::Confirm=>confirm=true, ModelOutcome::Continue=>{}
                },
                egui::Event::PointerButton { button:egui::PointerButton::Primary,pressed:true,.. } => confirm=true,
                egui::Event::PointerButton { button:egui::PointerButton::Secondary,pressed:true,.. } => cancel=true,
                _=>{}
            }
        }
        if cancel { self.model_modal=Some((state,before,id,previous)); self.cancel_model_modal(); return; }
        if let Some(pointer)=pointer { state.move_to(pointer,modifiers.shift,modifiers.ctrl); }
        if state.kind==ModelKind::Bevel && scroll!=0. { state.divisions_step(if scroll>0. {1} else {-1}); }
        let param=state.kind.parameter();
        let value=json!(state.value);
        if self.editor.doc.value()["nodes"][&id]["params"][param]!=value {
            let edit=Edit::Param { node:id.clone(),name:param.into(),value };
            if self.editor.doc.edit(&edit).is_ok() { self.send(edit); }
        }
        if state.kind==ModelKind::Bevel && self.editor.doc.value()["nodes"][&id]["params"]["divisions"]!=json!(state.divisions) {
            let edit=Edit::Param { node:id.clone(),name:"divisions".into(),value:json!(state.divisions) };
            if self.editor.doc.edit(&edit).is_ok() { self.send(edit); }
        }
        if state.kind==ModelKind::Extrude {
            let direction=state.axis.map(|axis| [0,1,2].map(|k|if k==axis {1.} else {0.})).unwrap_or([0.;3]);
            if self.editor.doc.value()["nodes"][&id]["params"]["direction"]!=json!(direction) {
                let edit=Edit::Param { node:id.clone(),name:"direction".into(),value:json!(direction) };
                if self.editor.doc.edit(&edit).is_ok() { self.send(edit); }
            }
        }
        self.model_modal=Some((state,before,id,previous));
        if confirm { self.finish_model_modal(); } else { ctx.request_repaint(); }
    }
}
