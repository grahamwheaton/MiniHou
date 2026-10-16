//! Pointer state for node-backed scalar modeling gestures.
use eframe::egui::{Key, Pos2};
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
}
impl State {
    pub fn new(kind: Kind, pointer: Pos2, scale: f32) -> Self {
        Self { kind, start:pointer, pointer, raw:0., value:0., divisions:1, numeric:String::new(), scale }
    }
    pub fn move_to(&mut self, pointer: Pos2, shift: bool, ctrl: bool) {
        let previous = if matches!(self.kind, Kind::Bevel | Kind::Inset) { self.pointer.distance(self.start) } else { 0. };
        let next = if matches!(self.kind, Kind::Bevel | Kind::Inset) { pointer.distance(self.start) } else { 0. };
        let delta = if matches!(self.kind, Kind::Bevel | Kind::Inset) { next-previous } else { pointer.x-self.pointer.x-(pointer.y-self.pointer.y) };
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
            Key::PageDown | Key::Minus if self.kind==Kind::Bevel => { self.divisions_step(-1); Outcome::Continue }
            _ => { self.type_key(key,ctrl); Outcome::Continue }
        }
    }
    pub fn status(&self) -> String {
        let mut status=format!("{}  Offset: {:.3}",self.kind.label(),self.value);
        if self.kind==Kind::Bevel { status.push_str(&format!("  Divisions: {}",self.divisions)); }
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
}
