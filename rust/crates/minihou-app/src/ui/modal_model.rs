//! Pointer state for node-backed scalar modeling gestures.
use eframe::egui::{Key, Pos2, Vec2};
use minihou_app::Document;
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind { Extrude, Inset, Bevel, PushPull }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome { Continue, Confirm, Cancel }
impl Kind {
    pub fn node(self) -> &'static str { match self { Self::Extrude => "polyextrude", Self::Inset => "inset", Self::Bevel => "polybevel", Self::PushPull => "pushpull" } }
    pub fn parameter(self) -> &'static str { if self == Self::Inset { "amount" } else { "distance" } }
    pub fn label(self) -> &'static str { match self { Self::Extrude => "Extrude", Self::Inset => "Inset", Self::Bevel => "Bevel", Self::PushPull => "Push / Pull" } }
}

#[derive(Clone, Debug)]
pub struct State {
    pub kind: Kind, pub start: Pos2, pub pointer: Pos2, pub raw: f32,
    pub value: f32, pub divisions: i32, pub numeric: String, pub scale: f32,
    pub screen_axis: Vec2, pub axis: Option<usize>,
}
impl State {
    pub fn new(kind: Kind, pointer: Pos2, scale: f32) -> Self {
        Self { kind, start:pointer, pointer, raw:0., value:0., divisions:1, numeric:String::new(), scale,
            screen_axis:Vec2::new(1.,-1.).normalized(), axis:None }
    }
    pub fn move_to(&mut self, pointer: Pos2, shift: bool, ctrl: bool) {
        let previous = if matches!(self.kind, Kind::Bevel | Kind::Inset) { self.pointer.distance(self.start) } else { 0. };
        let next = if matches!(self.kind, Kind::Bevel | Kind::Inset) { pointer.distance(self.start) } else { 0. };
        let delta = if matches!(self.kind, Kind::Bevel | Kind::Inset) { next-previous }
            else { (pointer-self.pointer).dot(self.screen_axis) };
        self.pointer=pointer;
        if self.numeric.is_empty() { self.raw += delta*self.scale*if shift { 0.1 } else { 1. }; }
        self.recalculate(ctrl);
    }
    pub fn recalculate(&mut self, ctrl: bool) {
        let value=self.numeric.parse::<f32>().ok().unwrap_or_else(|| if ctrl { (self.raw*10.).round()/10. } else { self.raw });
        self.value=if matches!(self.kind, Kind::Bevel | Kind::Inset) { value.max(0.) } else { value };
    }
    pub fn type_key(&mut self, key: Key, ctrl: bool) -> bool {
        // Share the transform modal's exact numeric-key mapping.
        let mut input=super::modal_transform::Modal::new(super::modal_transform::Kind::Rotate,self.start,[0.;3],([0.;3],[0.;3]),super::SnapTo::Increment,false,1.);
        input.numeric[0]=self.numeric.clone();
        if !input.type_key(key,false) { return false; }
        self.numeric=input.numeric[0].clone(); self.recalculate(ctrl); true
    }
    pub fn divisions_step(&mut self, step: i32) { self.divisions=(self.divisions+step).clamp(1,64); }
    pub fn key_event(&mut self, key: Key, ctrl: bool) -> Outcome {
        match key {
            Key::Escape => Outcome::Cancel,
            Key::Enter => Outcome::Confirm,
            Key::PageUp | Key::Plus if self.kind==Kind::Bevel => { self.divisions_step(1); Outcome::Continue }
            Key::PageDown if self.kind==Kind::Bevel => { self.divisions_step(-1); Outcome::Continue }
            Key::X | Key::Y | Key::Z if self.kind==Kind::Extrude => {
                let axis=match key { Key::X=>0, Key::Y=>1, _=>2 };
                self.axis=if self.axis==Some(axis) { None } else { Some(axis) }; Outcome::Continue
            }
            _ => { self.type_key(key,ctrl); Outcome::Continue }
        }
    }
    pub fn status(&self) -> String {
        let measure=if matches!(self.kind,Kind::Extrude | Kind::PushPull) { "Distance" } else { "Offset" };
        let mut status=format!("{}  {}: {:.3}",self.kind.label(),measure,self.value);
        if self.kind==Kind::Bevel { status.push_str(&format!("  Divisions: {}",self.divisions)); }
        if let Some(axis)=self.axis { status.push_str(&format!("  {}",["X","Y","Z"][axis])); }
        status
    }
}

pub fn insert_graph(before: &Document, source: &str, kind: Kind, params: Value) -> Option<(Document,String)> {
    let (mut graph,id)=minihou_app::graph_edit::add(before.value(),kind.node(),Some(source))?;
    graph["nodes"][&id]["params"]=params;
    Some((minihou_app::graph_edit::validated(graph).ok()?,id))
}

#[cfg(test)] mod tests {
    use super::*;
    use eframe::egui::pos2;
    #[test] fn precision_snap_numeric_and_division_bounds() {
        let mut s=State::new(Kind::Bevel,pos2(0.,0.),0.01);
        s.move_to(pos2(100.,0.),true,false); assert!((s.value-0.1).abs()<1e-5);
        s.recalculate(true); assert_eq!(s.value,0.1);
        s.type_key(Key::Num2,false); assert_eq!(s.value,2.);
        s.divisions_step(100); assert_eq!(s.divisions,64);
        s.divisions_step(-100); assert_eq!(s.divisions,1);
        assert_eq!(s.key_event(Key::PageUp,false),Outcome::Continue); assert_eq!(s.divisions,2);
        assert_eq!(s.key_event(Key::Escape,false),Outcome::Cancel);
        assert_eq!(s.key_event(Key::Enter,false),Outcome::Confirm);
    }
    #[test] fn bevel_cancel_restores_graph_and_confirm_is_one_undo_step() {
        use minihou_app::{Edit,History};
        use serde_json::json;
        let base=Document::parse(json!({"version":1,"nodes":{"box1":{"type":"box"},"out":{"type":"output","inputs":{"geo":"box1"}}},"display":"out"}).to_string()).unwrap();
        let (inserted,id)=insert_graph(&base,"out",Kind::Bevel,json!({"edges":"0-1","distance":0.,"divisions":1})).unwrap();
        assert_eq!(inserted.value()["nodes"]["out"]["inputs"]["geo"],id);
        assert_eq!(inserted.value()["nodes"][&id]["inputs"]["geo"],"box1");
        let mut history=History::default(); let mut doc=inserted.clone();
        doc.edit(&Edit::Param{node:id.clone(),name:"distance".into(),value:json!(0.2)}).unwrap();
        doc=base.clone(); // Escape restores the snapshot without recording.
        assert_eq!(doc.value(),base.value()); assert!(!history.can_undo());
        doc=inserted;
        doc.edit(&Edit::Param{node:id,name:"divisions".into(),value:json!(4)}).unwrap();
        history.record(base.clone(),&doc);
        assert!(history.undo(&mut doc).is_some()); assert_eq!(doc.value(),base.value());
        assert!(history.undo(&mut doc).is_none());
    }
    #[test] fn extrude_projected_drag_axis_numeric_and_graph_history() {
        use minihou_app::{Edit,History};
        use serde_json::json;
        let mut s=State::new(Kind::Extrude,pos2(10.,10.),0.01);
        s.screen_axis=eframe::egui::vec2(0.,-1.);
        s.move_to(pos2(10.,0.),false,false); assert!((s.value-0.1).abs()<1e-6);
        s.move_to(pos2(10.,-10.),true,false); assert!((s.value-0.11).abs()<1e-6);
        s.recalculate(true); assert_eq!(s.value,0.1);
        s.key_event(Key::X,false); assert_eq!(s.axis,Some(0));
        s.key_event(Key::X,false); assert_eq!(s.axis,None);
        s.key_event(Key::Num2,false); assert_eq!(s.value,2.);
        let base=Document::parse(json!({"version":1,"nodes":{"box1":{"type":"box"},"out":{"type":"output","inputs":{"geo":"box1"}}},"display":"out"}).to_string()).unwrap();
        let (mut doc,id)=insert_graph(&base,"out",Kind::Extrude,json!({"group":"0","distance":0.})).unwrap();
        assert_eq!(doc.value()["nodes"]["out"]["inputs"]["geo"],id);
        doc.edit(&Edit::Param{node:id.clone(),name:"distance".into(),value:json!(2.)}).unwrap();
        let mut history=History::default(); history.record(base.clone(),&doc);
        assert!(history.undo(&mut doc).is_some()); assert_eq!(doc.value(),base.value());
        assert!(history.undo(&mut doc).is_none());
        let (mut cancelled,id)=insert_graph(&base,"out",Kind::Extrude,json!({"group":"0","distance":0.})).unwrap();
        cancelled.edit(&Edit::Param{node:id,name:"distance".into(),value:json!(1.)}).unwrap();
        cancelled=base.clone(); assert_eq!(cancelled.value(),base.value());
    }

}
