//! Viewport operator state, redo controls and procedural Group Create conversion.
use super::*;
use minihou_core::coplanar::{self,Plane};
pub struct State {pub planes:Vec<Plane>,pub angle:f64,pub tolerance:f64,pub name:String,pub source:String}
impl State {
    pub fn params(&self)->Value {
        let plane=self.planes[0];
        json!({"name":self.name,"domain":"primitives","method":"coplanar","direction":plane.normal,"plane_offset":plane.offset,
            "planes":if self.planes.len()>1 {coplanar::encode(&self.planes)}else{String::new()},"angle":self.angle,"plane_tolerance":self.tolerance})
    }
    pub fn selection(&self,g:&minihou_core::Geometry)->Result<picking::Selection> {
        let mask=coplanar::mask(g,&self.planes,self.angle,self.tolerance)?;
        let layout=coplanar::preview_layout(g);
        Ok(picking::Selection {mode:ui::SelectionMode::Faces,tris:layout.tri_prim.iter().enumerate().filter(|(_,prim)|mask[**prim as usize]).map(|(tri,_)|tri as u32).collect(),..Default::default()})
    }
}
fn panel_id()->egui::Id {egui::Id::new("coplanar_redo_bounds")}
pub fn contains(ctx:&egui::Context,viewport:egui::Rect,p:egui::Pos2)->bool {
    ctx.data(|d|d.get_temp::<egui::Rect>(panel_id())).unwrap_or_else(|| {
        let available=ui::viewport_ui::operator_panel_rect(viewport);
        egui::Rect::from_min_size(available.left_bottom()-egui::vec2(0.,174.),egui::vec2(278.,174.))
    }).contains(p)
}
pub fn show(ui:&egui::Ui,rect:egui::Rect,state:&mut Option<State>,units:units::UnitSettings,out:&mut Actions) {
    let Some(operator)=state.as_mut() else {return;}; let mut close=false;
    let available=ui::viewport_ui::operator_panel_rect(rect);
    let area=egui::Area::new(egui::Id::new("coplanar_redo")).order(egui::Order::Foreground)
        .pivot(egui::Align2::LEFT_BOTTOM).fixed_pos(available.left_bottom()).constrain_to(available).show(ui.ctx(),|ui| {
        egui::Frame::popup(ui.style()).show(ui,|ui| {
            ui.set_width(246.);ui.horizontal(|ui|{ui.strong("Select Coplanar");if ui.small_button("×").clicked(){close=true;}});
            let mut changed=false;
            ui.horizontal(|ui|{ui.label("Angle");changed|=ui.add(egui::DragValue::new(&mut operator.angle).range(0.0..=180.0).speed(0.1).suffix("°")).changed();});
            let mut distance=units.display_value(operator.tolerance);
            ui.horizontal(|ui|{ui.label("Plane tolerance");if ui.add(egui::DragValue::new(&mut distance).range(0.0..=f64::MAX).speed(0.01).suffix(format!(" {}",units.display.symbol()))).changed(){operator.tolerance=distance*units.display.metres()/units.unit_scale;changed=true;}});
            ui.horizontal(|ui|{ui.label("Group");ui.add(egui::TextEdit::singleline(&mut operator.name).desired_width(156.));});
            if changed {out.push(UiAction::Command("update_coplanar".into()));}
            if ui.button("Make group").clicked(){out.push(UiAction::Command("make_coplanar_group".into()));}
        });
    });
    ui.ctx().data_mut(|d|d.insert_temp(panel_id(),area.response.rect));
    if close {*state=None;}
}
impl App {
    pub fn select_coplanar(&mut self) {
        if self.editor_state.selection!=ui::SelectionMode::Faces || self.selection.tris.is_empty() {self.file_status="Select faces before Select Coplanar".into();return;}
        if self.cooking.cooking(){self.file_status="Wait for geometry to cook".into();return;}
        let Some(g)=self.cooking.geometry.as_ref() else {return;};
        let planes=coplanar::seeds(g,&self.selection.tris);
        if planes.is_empty(){self.file_status="Selected faces have no valid planes".into();return;}
        self.coplanar=Some(State {planes,angle:0.1,tolerance:self.prefs.units.parse("0.1").unwrap_or(0.0001),name:"coplanar".into(),source:self.active_object().map(str::to_owned).unwrap_or_else(||self.display.clone())});
        self.update_coplanar();
    }
    pub fn update_coplanar(&mut self) {
        let (Some(state),Some(g),Some(scene))=(&self.coplanar,&self.cooking.geometry,&self.scene) else {return;};
        match state.selection(g) {
            Ok(mut selection)=>{
                selection.tris.retain(|tri|scene.triangle_indices().get(*tri as usize*3..*tri as usize*3+3).is_some_and(|ids|ids.iter().all(|id|point_pickable(self.editor_state.object_visibility,&self.display_ranges,&self.selected_set,*id))));
                self.selection=selection;self.selection_dirty=true;
            }
            Err(e)=>self.file_status=e.to_string(),
        }
    }
    pub fn make_coplanar_group(&mut self) {
        let Some(operator)=self.coplanar.as_ref() else {return;};
        if !self.editor.doc.value()["nodes"].get(&operator.source).is_some(){self.file_status="The selection source no longer exists".into();return;}
        let params=operator.params();let source=operator.source.clone();
        // Validate the name before making a document edit.
        if !valid_group_name(params["name"].as_str().unwrap_or("")){self.file_status="Group name must start with a letter or underscore and contain only letters, digits or underscores".into();return;}
        let Some((mut graph,id))=graph_edit::add(self.editor.doc.value(),"group_create",Some(&source)) else {return;};
        graph["nodes"][&id]["params"]=params;
        if self.commit_document(graph,"make coplanar group") {
            self.display=Graph::from_value(self.editor.doc.value()).ok().and_then(|g|g.main_output().ok()).unwrap_or(id.clone());
            self.selected=id.clone();self.selected_set=std::collections::BTreeSet::from([id]);
            self.selection.clear();self.selection_dirty=true;self.coplanar=None;
        }
    }
}
fn valid_group_name(name:&str)->bool { !name.is_empty()&&!name.as_bytes()[0].is_ascii_digit()&&name.chars().all(|c|c.is_ascii_alphanumeric()||c=='_') }

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn selection_and_procedural_group_capture_the_same_faces() {
        let base=minihou_app::Document::parse(json!({"nodes":{"box":{"type":"box"},"out":{"type":"output","inputs":{"geo":"box"}}},"display":"out"}).to_string()).unwrap();
        let geo=base.session().unwrap().cook(Some("out")).unwrap();
        let planes=coplanar::seeds(&geo,&[0]);assert_eq!(planes.len(),1);
        let state=State {planes,angle:0.1,tolerance:0.0001,name:"flat".into(),source:"out".into()};
        let selected=state.selection(&geo).unwrap();assert_eq!(selected.tris.len(),2);
        let (mut value,id)=graph_edit::add(base.value(),"group_create",Some("out")).unwrap();value["nodes"][&id]["params"]=state.params();
        let mut doc=base.clone();let mut history=minihou_app::History::default();
        history.replace(&mut doc,minihou_app::Document::parse(value.to_string()).unwrap());
        let grouped=doc.session().unwrap().cook(Some("out")).unwrap();
        let mut changed=doc.session().unwrap();
        changed.set_param(&id,"plane_offset",json!(state.planes[0].offset+10.)).unwrap();
        let shifted=changed.cook(Some("out")).unwrap();
        assert!(crate::group_preview::geometry(&shifted,"flat").0.is_empty(),"editing the plane offset must update the preview membership");
        let (faces,lines)=crate::group_preview::geometry(&grouped,"flat");
        assert_eq!(faces.len(),selected.tris.len()*9);assert_eq!(lines.len(),4*6);
        history.undo(&mut doc).unwrap();assert_eq!(doc.value(),base.value());
        history.redo(&mut doc).unwrap();assert_eq!(doc.value(),&value);
    }
    #[test] fn make_group_popup_emits_action_and_excludes_viewport_clicks() {
        let ctx=egui::Context::default();let rect=egui::Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(800.,600.));
        let mut state=Some(State {planes:vec![],angle:0.1,tolerance:0.0001,name:"coplanar".into(),source:"box".into()});
        let mut button=egui::Pos2::ZERO;let mut actions=Actions::default();
        for phase in 0..5 {
            let events=match phase {2|3=>vec![egui::Event::PointerMoved(button),egui::Event::PointerButton {pos:button,button:egui::PointerButton::Primary,pressed:phase==2,modifiers:Default::default()}],_=>vec![]};
            let output=ctx.run(egui::RawInput {screen_rect:Some(rect),events,..Default::default()},|ctx| {egui::CentralPanel::default().show(ctx,|ui|show(ui,rect,&mut state,units::UnitSettings::default(),&mut actions));});
            if phase>0 {
                let bounds=ctx.data(|d|d.get_temp::<egui::Rect>(panel_id())).unwrap();
                assert!(ui::viewport_ui::operator_panel_rect(rect).contains_rect(bounds), "panel overlaps viewport chrome: {bounds:?}");
            }
            if phase==1 {for shape in output.shapes {if let egui::Shape::Text(text)=shape.shape {if text.galley.text()=="Make group" {button=text.pos+text.galley.size()*0.5;}}}}
        }
        assert!(contains(&ctx,rect,button));assert!(actions.0.iter().any(|a|matches!(a,UiAction::Command(name) if name=="make_coplanar_group")));
    }
}
