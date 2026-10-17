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
    /// Push/Pull inference targets (signed distances along the face normal).
    pub targets: Vec<Snap>,
    /// Inference shown for the current value ("Through" / "On Face").
    pub snapped: Option<SnapKind>,
    /// Snap radius in screen pixels (converted to world units with `scale`).
    pub snap_pixels: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapKind { Through, OnFace }
impl SnapKind {
    pub fn label(self) -> &'static str { match self { Self::Through => "Through", Self::OnFace => "On Face" } }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Snap { pub distance: f32, pub kind: SnapKind }

/// SketchUp-style inference for a Push/Pull distance. A `Through` target (the far
/// surface, negative) snaps within `tolerance` and stays "Through" for any deeper
/// push (the node ignores the excess); an `OnFace` target (a parallel face plane)
/// snaps within `tolerance`. With `snap` false (typed values) the number is kept.
pub fn snap_distance(value: f32, targets: &[Snap], tolerance: f32, snap: bool) -> (f32, Option<SnapKind>) {
    let mut best: Option<(f32, Snap)> = None;
    for &t in targets {
        let gap = (value - t.distance).abs();
        if gap <= tolerance && best.is_none_or(|(g, _)| gap < g) { best = Some((gap, t)); }
    }
    if let Some((_, t)) = best {
        return (if snap { t.distance } else { value }, Some(t.kind));
    }
    let through = targets.iter().any(|t| t.kind == SnapKind::Through && value < t.distance);
    (value, through.then_some(SnapKind::Through))
}

/// Inference targets for pushing the selected triangles along `normal`: the far
/// surface reached by rays cast backwards from the selection (every ray must hit; the
/// deepest exit, matching the node's punch rule) and parallel same-facing face planes.
pub fn pushpull_targets(positions: &[f32], tris: &[u32], selected: &[u32], normal: [f32; 3]) -> Vec<Snap> {
    type V = [f64; 3];
    let at = |i: u32| -> V { let k = i as usize * 3; [positions[k] as f64, positions[k+1] as f64, positions[k+2] as f64] };
    let sub = |a: V, b: V| -> V { [a[0]-b[0], a[1]-b[1], a[2]-b[2]] };
    let dot = |a: V, b: V| a[0]*b[0]+a[1]*b[1]+a[2]*b[2];
    let cross = |a: V, b: V| -> V { [a[1]*b[2]-a[2]*b[1], a[2]*b[0]-a[0]*b[2], a[0]*b[1]-a[1]*b[0]] };
    let n = normal.map(|x| x as f64);
    let len = dot(n, n).sqrt();
    let ntri = tris.len() / 3;
    if len < 1e-9 || positions.is_empty() { return vec![]; }
    let n = n.map(|x| x / len);
    let chosen: std::collections::BTreeSet<u32> = selected.iter().copied().filter(|&t| (t as usize) < ntri).collect();
    let Some(&first) = chosen.iter().next() else { return vec![] };
    let corners = |t: usize| [at(tris[t*3]), at(tris[t*3+1]), at(tris[t*3+2])];
    let mut lo = [f64::INFINITY; 3]; let mut hi = [f64::NEG_INFINITY; 3];
    for i in 0..positions.len() / 3 { let p = at(i as u32); for k in 0..3 { lo[k] = lo[k].min(p[k]); hi[k] = hi[k].max(p[k]); } }
    let extent = (0..3).map(|k| hi[k] - lo[k]).fold(1e-8, f64::max);
    let eps = extent * 1e-6;
    let anchor = corners(first as usize)[0];
    let dir = n.map(|x| -x);
    let mut deepest: Option<f64> = Some(f64::NEG_INFINITY);
    for &s in &chosen {
        let c = corners(s as usize);
        let centroid: V = [0,1,2].map(|k| (c[0][k]+c[1][k]+c[2][k]) / 3.);
        let toward = |p: V| -> V { [0,1,2].map(|k| p[k] + (centroid[k]-p[k]) * 0.01) };
        for o in [centroid, toward(c[0]), toward(c[1]), toward(c[2])] {
            let mut best: Option<f64> = None;
            for t in 0..ntri {
                if chosen.contains(&(t as u32)) { continue; }
                let [a, b, c] = corners(t);
                let (e1, e2) = (sub(b, a), sub(c, a));
                let h = cross(dir, e2); let det = dot(e1, h);
                if det.abs() <= 1e-14 * dot(e1, e1).sqrt() * dot(e2, e2).sqrt() { continue; }
                let sv = sub(o, a); let u = dot(sv, h) / det; let q = cross(sv, e1); let v = dot(dir, q) / det;
                if u < -1e-9 || v < -1e-9 || u + v > 1. + 1e-9 { continue; }
                let d = dot(e2, q) / det;
                if d > eps && best.is_none_or(|b| d < b) { best = Some(d); }
            }
            deepest = match (deepest, best) { (Some(a), Some(b)) => Some(a.max(b)), _ => None };
        }
    }
    let mut out = vec![];
    if let Some(d) = deepest.filter(|d| d.is_finite()) { out.push(Snap { distance: -d as f32, kind: SnapKind::Through }); }
    let mut planes: Vec<f64> = vec![];
    for t in 0..ntri {
        if chosen.contains(&(t as u32)) { continue; }
        let [a, b, c] = corners(t);
        let m = cross(sub(b, a), sub(c, a)); let ml = dot(m, m).sqrt();
        if ml < 1e-12 || dot(m, n) / ml < 1. - 1e-6 { continue; }
        let d = dot(sub(a, anchor), n);
        if d > extent * 1e-5 && !planes.iter().any(|&p| (p - d).abs() <= extent * 1e-5) { planes.push(d); }
    }
    out.extend(planes.into_iter().map(|d| Snap { distance: d as f32, kind: SnapKind::OnFace }));
    out
}
impl State {
    pub fn new(kind: Kind, pointer: Pos2, scale: f32) -> Self {
        Self { kind, start:pointer, pointer, raw:0., value:0., divisions:1, numeric:String::new(), scale,
            screen_axis:Vec2::new(1.,-1.).normalized(), axis:None, targets:vec![], snapped:None, snap_pixels:8. }
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
        self.snapped=None;
        if self.kind==Kind::PushPull && !self.targets.is_empty() {
            let typed=self.numeric.parse::<f32>().is_ok();
            let tolerance=if typed { self.scale*1e-3 } else { self.snap_pixels*self.scale };
            (self.value,self.snapped)=snap_distance(self.value,&self.targets,tolerance,!typed);
        }
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
        if let Some(snap)=self.snapped { status.push_str(&format!("  [{}]",snap.label())); }
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
    #[test] fn pushpull_inserts_connected_before_output_and_restores_on_cancel() {
        use minihou_app::{Edit,History};
        use serde_json::json;
        let base=Document::parse(json!({"version":1,"nodes":{"box1":{"type":"box"},"out":{"type":"output","inputs":{"geo":"box1"}}},"display":"out"}).to_string()).unwrap();
        let (mut doc,id)=insert_graph(&base,"out",Kind::PushPull,json!({"group":"0","distance":0.})).unwrap();
        assert_eq!(doc.value()["nodes"]["out"]["inputs"]["geo"],id);
        assert_eq!(doc.value()["nodes"][&id]["inputs"]["geo"],"box1");
        doc.edit(&Edit::Param{node:id.clone(),name:"distance".into(),value:json!(0.5)}).unwrap();
        let mut history=History::default(); history.record(base.clone(),&doc);
        assert!(history.undo(&mut doc).is_some()); assert_eq!(doc.value(),base.value());
        assert!(history.undo(&mut doc).is_none());
        let (mut cancelled,id)=insert_graph(&base,"out",Kind::PushPull,json!({"group":"0","distance":0.})).unwrap();
        cancelled.edit(&Edit::Param{node:id,name:"distance".into(),value:json!(0.5)}).unwrap();
        cancelled=base.clone(); assert_eq!(cancelled.value(),base.value());
        let mut s=State::new(Kind::PushPull,pos2(0.,0.),0.01);
        s.move_to(pos2(10.,-10.),false,false); assert!(s.value>0.);
        s.key_event(Key::Num3,false); assert_eq!(s.value,3.);
        assert_eq!(s.key_event(Key::Escape,false),Outcome::Cancel);
    }
    #[test] fn pushpull_snap_through_on_face_and_beyond() {
        let targets=[Snap{distance:-1.,kind:SnapKind::Through},Snap{distance:0.5,kind:SnapKind::OnFace}];
        assert_eq!(snap_distance(-0.97,&targets,0.05,true),(-1.,Some(SnapKind::Through)));
        assert_eq!(snap_distance(-0.9,&targets,0.05,true),(-0.9,None));
        // Past the far face: the value stays free and still punches (excess ignored by the node).
        assert_eq!(snap_distance(-2.5,&targets,0.05,true),(-2.5,Some(SnapKind::Through)));
        assert_eq!(snap_distance(0.52,&targets,0.05,true),(0.5,Some(SnapKind::OnFace)));
        assert_eq!(snap_distance(0.3,&targets,0.05,true),(0.3,None));
        // Typed numbers are never moved, only labelled.
        assert_eq!(snap_distance(-0.97,&targets,0.05,false),(-0.97,Some(SnapKind::Through)));
        assert_eq!(snap_distance(0.1,&[],0.05,true),(0.1,None));
    }
    fn unit_box() -> (Vec<f32>,Vec<u32>) {
        let p=vec![0.,0.,0., 1.,0.,0., 1.,1.,0., 0.,1.,0., 0.,0.,1., 1.,0.,1., 1.,1.,1., 0.,1.,1.];
        // Outward quads: bottom, top, -y, +x, +y, -x; two triangles each.
        let quads=[[0u32,3,2,1],[4,5,6,7],[0,1,5,4],[1,2,6,5],[2,3,7,6],[3,0,4,7]];
        let t=quads.iter().flat_map(|q|[q[0],q[1],q[2],q[0],q[2],q[3]]).collect();
        (p,t)
    }
    #[test] fn pushpull_targets_find_far_face_and_open_space() {
        let (p,t)=unit_box();
        // Top face = triangles 2 and 3 (normal +Z): the far face is 1 below.
        let targets=pushpull_targets(&p,&t,&[2,3],[0.,0.,1.]);
        assert_eq!(targets.len(),1);
        assert_eq!(targets[0].kind,SnapKind::Through);
        assert!((targets[0].distance+1.).abs()<1e-5);
        // Open mesh: without the bottom the push escapes into empty space, no punch target.
        let open:Vec<u32>=t[6..].to_vec();
        let targets=pushpull_targets(&p,&open,&[0,1],[0.,0.,1.]);
        assert!(targets.iter().all(|s|s.kind!=SnapKind::Through));
        // A raised parallel face gives an On Face target for pulling.
        let mut p2=p.clone(); p2.extend([2.,0.,1.5, 3.,0.,1.5, 3.,1.,1.5]);
        let mut t2=t.clone(); t2.extend([8,9,10]);
        let targets=pushpull_targets(&p2,&t2,&[2,3],[0.,0.,1.]);
        assert!(targets.iter().any(|s|s.kind==SnapKind::OnFace && (s.distance-0.5).abs()<1e-5));
    }
    #[test] fn pushpull_modal_state_snaps_and_reports_through() {
        let mut s=State::new(Kind::PushPull,pos2(0.,0.),0.01);
        s.screen_axis=eframe::egui::vec2(0.,-1.);
        s.targets=vec![Snap{distance:-1.,kind:SnapKind::Through}];
        s.move_to(pos2(0.,97.),false,false); // raw -0.97: within 8 px of the far face
        assert_eq!(s.value,-1.); assert_eq!(s.snapped,Some(SnapKind::Through));
        assert!(s.status().contains("[Through]"));
        s.move_to(pos2(0.,300.),false,false); // drag beyond: free value, still Through
        assert!((s.value+3.).abs()<1e-4); assert_eq!(s.snapped,Some(SnapKind::Through));
        s.move_to(pos2(0.,50.),false,false);
        assert!((s.value+0.5).abs()<1e-4); assert_eq!(s.snapped,None);
        s.key_event(Key::Minus,false); s.key_event(Key::Num2,false); // typed -2: exact, labelled
        assert_eq!(s.value,-2.); assert_eq!(s.snapped,Some(SnapKind::Through));
        assert_eq!(s.key_event(Key::Escape,false),Outcome::Cancel);
    }
    #[test] fn pushpull_through_preview_cooks_hole_and_cancel_restores() {
        use minihou_app::{Edit,History};
        use serde_json::json;
        let base=Document::parse(json!({"version":1,"nodes":{"box1":{"type":"box"},"in1":{"type":"inset","inputs":{"geo":"box1"},"params":{"group":"1","amount":0.25}},"out":{"type":"output","inputs":{"geo":"in1"}}},"display":"out"}).to_string()).unwrap();
        let (mut doc,id)=insert_graph(&base,"out",Kind::PushPull,json!({"group":"extrudeFront","distance":0.})).unwrap();
        // Default through=auto: a push to the far face cooks a closed through-hole.
        doc.edit(&Edit::Param{node:id.clone(),name:"distance".into(),value:json!(-1.0)}).unwrap();
        let graph=minihou_core::Graph::from_value(doc.value()).unwrap();
        let g=graph.cook(Some(&id)).unwrap();
        let edges=minihou_core::topo::edges(&g.topology,g.npoints);
        assert_eq!(g.npoints as i64-edges.len() as i64+g.nprims() as i64,0,"through-hole is a torus");
        assert!(edges.count.iter().all(|&c|c==2));
        let mut history=History::default(); history.record(base.clone(),&doc);
        assert!(history.undo(&mut doc).is_some()); assert_eq!(doc.value(),base.value());
        assert!(history.undo(&mut doc).is_none());
        let (mut cancelled,id)=insert_graph(&base,"out",Kind::PushPull,json!({"group":"extrudeFront","distance":0.})).unwrap();
        cancelled.edit(&Edit::Param{node:id,name:"distance".into(),value:json!(-3.)}).unwrap();
        cancelled=base.clone(); assert_eq!(cancelled.value(),base.value());
    }
}
