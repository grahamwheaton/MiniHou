//! Shared capture frame and viewport guides for deformers.
use eframe::egui::{self, Color32, Id, Pos2, Rect, Stroke};
use serde_json::{json, Value};
use minihou_app::Edit;
use minihou_core::Graph;
use super::{Actions, CameraInfo, Tool, UiAction, View};

type V3 = [f64; 3];
fn add(a: V3, b: V3) -> V3 { [a[0]+b[0], a[1]+b[1], a[2]+b[2]] }
fn sub(a: V3, b: V3) -> V3 { [a[0]-b[0], a[1]-b[1], a[2]-b[2]] }
fn mul(a: V3, s: f64) -> V3 { a.map(|x|x*s) }
fn dot(a: V3, b: V3) -> f64 { a.iter().zip(b).map(|(x,y)|x*y).sum() }
fn cross(a: V3, b: V3) -> V3 { [a[1]*b[2]-a[2]*b[1],a[2]*b[0]-a[0]*b[2],a[0]*b[1]-a[1]*b[0]] }
fn unit(a: V3) -> Option<V3> { let n=dot(a,a).sqrt(); (n>1e-10 && n.is_finite()).then(||mul(a,1./n)) }
fn vec3(params:&Value,name:&str,default:V3)->V3 {
    params[name].as_array().filter(|a|a.len()==3).and_then(|a|Some([
        a[0].as_f64()?,a[1].as_f64()?,a[2].as_f64()?,
    ])).filter(|v|v.iter().all(|x|x.is_finite())).unwrap_or(default)
}
fn number(params:&Value,name:&str,default:f64)->f64 { params[name].as_f64().filter(|x|x.is_finite()).unwrap_or(default) }
fn f32v(v:V3)->[f32;3] { v.map(|x|x as f32) }

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CaptureFrame { pub origin:V3, pub direction:V3, pub up:V3, pub side:V3, pub length:f64, pub radius:f64 }
impl CaptureFrame {
    pub fn from_params(ty:&str, params:&Value, radius:f64)->Self {
        let axis=if ty=="bend" { [0.,0.,1.] } else { [0.,1.,0.] };
        let direction=unit(vec3(params,"direction",axis)).unwrap_or(axis);
        let raw_up=vec3(params,"up",if ty=="bend" {[0.,1.,0.]} else {[0.,0.,1.]});
        let mut up=sub(raw_up,mul(direction,dot(raw_up,direction)));
        if unit(up).is_none() {
            // Pick the least aligned world axis so the frame cannot flip unpredictably.
            let mut idx=0;
            for k in 1..3 { if direction[k].abs()<direction[idx].abs() { idx=k; } }
            let mut fallback=[0.;3]; fallback[idx]=1.;
            up=sub(fallback,mul(direction,direction[idx]));
        }
        up=unit(up).unwrap_or([0.,0.,1.]);
        let angle=number(params,"up_vector_angle",0.).to_radians();
        let (sn,cs)=angle.sin_cos();
        up=add(mul(up,cs),mul(cross(direction,up),sn));
        let side=unit(cross(up,direction)).unwrap_or([1.,0.,0.]);
        Self { origin:vec3(params,"origin",vec3(params,"center",[0.;3])), direction, up,
            side, length:number(params,"length",1.).max(1e-6), radius:radius.max(0.05) }
    }
    pub fn at(&self, axial:f64, sideways:f64, upwards:f64)->V3 {
        add(self.origin,add(mul(self.direction,axial),add(mul(self.side,sideways),mul(self.up,upwards))))
    }
    pub fn segments(&self)->Vec<(V3,V3)> {
        let mut edges=Vec::with_capacity(12);
        for &a in &[0.,self.length] {
            let corners=[self.at(a,-self.radius,-self.radius),self.at(a,self.radius,-self.radius),
                self.at(a,self.radius,self.radius),self.at(a,-self.radius,self.radius)];
            for k in 0..4 { edges.push((corners[k],corners[(k+1)%4])); }
        }
        for &s in &[-1.,1.] { for &u in &[-1.,1.] {
            edges.push((self.at(0.,s*self.radius,u*self.radius),self.at(self.length,s*self.radius,u*self.radius)));
        } }
        edges
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle { Origin, Direction, Length, Up, BendArc, TwistRing, TaperX, TaperY,
    TaperCenter, SquishPivot, LatticeStrength, PathOffset, PathTwist, PointRadius, PointStrength,
    SoftRadius }
pub fn capture_handles(frame:&CaptureFrame)->[(Handle,V3);4] {
    [(Handle::Origin,frame.origin),(Handle::Direction,frame.at(frame.length+frame.radius*0.35,0.,0.)),
     (Handle::Length,frame.at(frame.length,0.,0.)),(Handle::Up,frame.at(0.,0.,frame.radius))]
}
/// Drag input is a world-space displacement measured from the pointer's original position.
pub fn apply_handle_drag(handle:Handle, frame:&CaptureFrame, params:&Value, delta:V3)->Vec<(&'static str,Value)> {
    match handle {
        Handle::Origin => vec![("origin",json!(add(frame.origin,delta)))],
        Handle::Direction => {
            let tip=add(mul(frame.direction,frame.length),delta);
            unit(tip).map(|v|vec![("direction",json!(v))]).unwrap_or_default()
        }
        Handle::Length => vec![("length",json!((frame.length+dot(delta,frame.direction)).max(1e-6)))],
        Handle::Up => {
            let candidate=add(frame.up,delta);
            let projected=sub(candidate,mul(frame.direction,dot(candidate,frame.direction)));
            unit(projected).map(|v|vec![("up",json!(v))]).unwrap_or_default()
        }
        Handle::BendArc => vec![("bend",json!(number(params,"bend",90.)+dot(delta,frame.side)*90./frame.radius))],
        Handle::TwistRing => vec![("strength",json!(number(params,"strength",90.)+dot(delta,frame.side)*90./frame.radius))],
        Handle::TaperX => vec![("taper_x",json!(number(params,"taper_x",0.5)+dot(delta,frame.side)/frame.radius))],
        Handle::TaperY => vec![("taper_y",json!(number(params,"taper_y",0.5)+dot(delta,frame.up)/frame.radius))],
        Handle::TaperCenter => vec![("origin",json!(add(frame.origin,delta)))],
        Handle::SquishPivot => vec![("center",json!(add(frame.origin,delta)))],
        Handle::LatticeStrength | Handle::PointStrength => vec![("strength",json!(number(params,"strength",1.)+dot(delta,frame.side)/frame.radius))],
        Handle::PathOffset => vec![("offset",json!(number(params,"offset",0.)+dot(delta,frame.direction)))],
        Handle::PathTwist => vec![("twist",json!(number(params,"twist",0.)+dot(delta,frame.side)*90./frame.radius))],
        Handle::PointRadius => vec![("radius",json!((number(params,"radius",0.)+dot(delta,frame.side)).max(0.)))],
        Handle::SoftRadius => vec![("radius",json!((number(params,"radius",1.)+dot(delta,frame.side)).max(1e-6)))],
    }
}

pub fn is_deformer(ty:&str)->bool { matches!(ty,"bend"|"twist"|"linear_taper"|"squash_stretch"|"lattice"|"path_deform"|"point_deform"|"soft_transform") }
pub fn handles_for_node(ty:&str,frame:&CaptureFrame)->Vec<(Handle,V3)> {
    let mut handles=Vec::new();
    match ty {
        "bend" => {
            handles.extend(capture_handles(frame));
            handles.push((Handle::BendArc,frame.at(frame.length*0.5,frame.radius,0.)));
        }
        "twist" => {
            handles.push((Handle::Origin,frame.origin));
            handles.push((Handle::Direction,frame.at(frame.length+frame.radius*0.35,0.,0.)));
            handles.push((Handle::TwistRing,frame.at(frame.length,frame.radius,0.)));
        }
        "linear_taper" => {
            handles.extend(capture_handles(frame).into_iter().filter(|(h,_)|*h!=Handle::Origin));
            handles.push((Handle::TaperX,frame.at(frame.length,frame.radius,0.)));
            handles.push((Handle::TaperY,frame.at(frame.length,0.,frame.radius)));
            handles.push((Handle::TaperCenter,frame.origin));
        }
        "squash_stretch" => {
            handles.push((Handle::SquishPivot,frame.origin));
            handles.push((Handle::Direction,frame.at(frame.length+frame.radius*0.35,0.,0.)));
        }
        "lattice" => handles.push((Handle::LatticeStrength,frame.at(frame.length*0.5,frame.radius,0.))),
        "path_deform" => {
            handles.push((Handle::Up,frame.at(0.,0.,frame.radius)));
            handles.push((Handle::PathOffset,frame.at(frame.length,0.,0.)));
            handles.push((Handle::PathTwist,frame.at(frame.length,frame.radius,0.)));
        }
        "point_deform" => {
            handles.push((Handle::PointRadius,frame.at(0.,frame.radius,0.)));
            handles.push((Handle::PointStrength,frame.at(frame.length*0.5,frame.radius,0.)));
        }
        "soft_transform" => {
            handles.push((Handle::SquishPivot,frame.origin));
            handles.push((Handle::SoftRadius,frame.at(0.,frame.radius,0.)));
        }
        _ => {}
    }
    handles
}

pub fn paint_deformer(ui:&egui::Ui,rect:Rect,v:&View,ty:&str,frame:&CaptureFrame) {
    let p=ui.painter_at(rect);
    let purple=Color32::from_rgb(180,90,240);
    let cyan=Color32::from_rgb(50,220,225);
    if matches!(ty,"bend"|"twist"|"linear_taper") { paint_capture(ui,rect,v,frame); }
    if ty=="bend" {
        let mut last=None;
        for k in 0..=24 {
            let theta=(k as f64/24.)*std::f64::consts::FRAC_PI_2;
            let pt=frame.at(frame.length*0.5,frame.radius*theta.cos(),frame.radius*theta.sin());
            let now=projected(v.camera,rect,pt);
            if let (Some(a),Some(b))=(last,now) { p.line_segment([a,b],Stroke::new(1.5_f32,purple)); }
            last=now;
        }
    }
    if ty=="twist" {
        let mut last=None;
        for k in 0..=32 {
            let theta=(k as f64/32.)*std::f64::consts::TAU;
            let now=projected(v.camera,rect,frame.at(frame.length,frame.radius*theta.cos(),frame.radius*theta.sin()));
            if let (Some(a),Some(b))=(last,now) { p.line_segment([a,b],Stroke::new(1.5_f32,purple)); }
            last=now;
        }
    }
    for (handle,world) in handles_for_node(ty,frame) {
        if let Some(pos)=projected(v.camera,rect,world) {
            let color=if matches!(handle,Handle::TaperX|Handle::TaperY|Handle::TaperCenter) { Color32::RED }
                else if matches!(handle,Handle::Up|Handle::PathOffset|Handle::PointRadius) { cyan } else {purple};
            if matches!(handle,Handle::TaperX|Handle::TaperY|Handle::TaperCenter) {
                p.rect_filled(Rect::from_center_size(pos,egui::Vec2::splat(9.)),0.,color);
            } else { p.circle_filled(pos,5.,color); }
        }
    }
}

#[derive(Clone)]
struct Drag { node:String, handle:Handle, start:Pos2, frame:CaptureFrame, params:Value, camera:CameraInfo, rect:Rect }
fn drag_id()->Id { Id::new("deform_handle_drag") }
fn armed_id()->Id { Id::new("deform_handle_armed") }
pub fn active(ctx:&egui::Context)->bool { ctx.data(|d|d.get_temp::<Drag>(drag_id()).is_some()) }
pub fn armed(ctx:&egui::Context)->bool { ctx.data(|d|d.get_temp::<(String,Handle)>(armed_id()).is_some()) }
pub fn finish_input(ctx:&egui::Context) { ctx.data_mut(|d|d.remove::<Drag>(drag_id())); }
fn selected<'a>(v:&'a View)->Option<(&'a str,&'a str,&'a Value)> {
    if v.editor.tool!=Tool::Select || v.selected.is_empty() { return None; }
    if v.selected!=v.display && !minihou_app::display_nodes(v.doc).iter().any(|n|n==v.selected) { return None; }
    let node=v.doc["nodes"].get(v.selected)?;
    let ty=node["type"].as_str()?;
    is_deformer(ty).then_some((v.selected,ty,&node["params"]))
}
fn frame_for(v:&View,ty:&str,params:&Value)->CaptureFrame {
    let radius=v.geo.and_then(|g|g.bounds).map(|(lo,hi)|{
        let span=[0,1,2].map(|k|(hi[k]-lo[k]).abs() as f64);
        span.into_iter().fold(0.,f64::max)*0.25
    }).unwrap_or(0.5);
    CaptureFrame::from_params(ty,params,radius)
}
fn pick_handle(rect:Rect,v:&View,ty:&str,frame:&CaptureFrame,pointer:Pos2)->Option<Handle> {
    handles_for_node(ty,frame).into_iter().filter_map(|(handle,world)|{
        let pos=projected(v.camera,rect,world)?;
        let dist=pos.distance(pointer);
        (dist<=10.).then_some((handle,dist))
    }).min_by(|a,b|a.1.total_cmp(&b.1)).map(|(h,_)|h)
}
pub fn hit(rect:Rect,v:&View,pointer:Pos2)->bool {
    let Some((_,ty,params))=selected(v) else { return false; };
    pick_handle(rect,v,ty,&frame_for(v,ty,params),pointer).is_some()
}
fn pointer_delta(d:&Drag,now:Pos2)->V3 {
    let o=projected(d.camera,d.rect,d.frame.origin).unwrap_or(d.start);
    let right=projected(d.camera,d.rect,add(d.frame.origin,d.camera.ray_right.map(f64::from)))
        .map(|p|p.distance(o)).unwrap_or(50.).max(1.);
    let up=projected(d.camera,d.rect,add(d.frame.origin,d.camera.ray_up.map(f64::from)))
        .map(|p|p.distance(o)).unwrap_or(50.).max(1.);
    let diff=now-d.start;
    add(mul(d.camera.ray_right.map(f64::from),diff.x as f64/right as f64),
        mul(d.camera.ray_up.map(f64::from),-diff.y as f64/up as f64))
}
pub fn show(ui:&mut egui::Ui,rect:Rect,v:&View,out:&mut Actions)->bool {
    let Some((node,ty,params))=selected(v) else {
        ui.ctx().data_mut(|d|d.remove::<(String,Handle)>(armed_id()));
        return false;
    };
    let frame=frame_for(v,ty,params);
    paint_deformer(ui,rect,v,ty,&frame);
    let pointer=ui.input(|i|i.pointer.hover_pos());
    let mut drag=ui.ctx().data(|d|d.get_temp::<Drag>(drag_id()));
    if let Some(d)=&drag {
        if ui.input(|i|i.pointer.primary_down()) {
            if let Some(now)=pointer {
                for (name,value) in apply_handle_drag(d.handle,&d.frame,&d.params,pointer_delta(d,now)) {
                    out.push(UiAction::Edit{edit:Edit::Param{node:d.node.clone(),name:name.into(),value},continuous:true});
                }
            }
        } else { finish_input(ui.ctx()); drag=None; }
    }
    let hovered=pointer.filter(|p|rect.contains(*p)).and_then(|p|pick_handle(rect,v,ty,&frame,p));
    let armed=ui.ctx().data(|d|d.get_temp::<(String,Handle)>(armed_id()))
        .and_then(|(owner,handle)|if owner==node {Some(handle)} else {
            ui.ctx().data_mut(|d|d.remove::<(String,Handle)>(armed_id())); None
        });
    if drag.is_none() && ui.input(|i|i.pointer.primary_pressed()) && !v.flags_of(node).locked {
        if let (Some(start),Some(handle))=(pointer.filter(|p|rect.contains(*p)),hovered.or(armed)) {
            ui.ctx().data_mut(|d|d.insert_temp(drag_id(),Drag{node:node.into(),handle,start,frame,params:params.clone(),camera:v.camera,rect}));
            ui.ctx().data_mut(|d|d.remove::<(String,Handle)>(armed_id()));
        }
    }
    let legend="F Frame  G Grab  H Guide  B Deform  N Fit Capture  M Alternate";
    ui.painter_at(rect).text(egui::pos2(rect.left()+12.,rect.bottom()-18.),egui::Align2::LEFT_BOTTOM,legend,
        egui::FontId::monospace(11.),v.theme.text);
    hovered.is_some() || drag.is_some() || armed.is_some()
}

#[derive(Clone,Copy,Debug,PartialEq,Eq)]
enum Shortcut { Frame, Arm(Handle), Fit }
fn shortcut(key:egui::Key,ty:&str)->Option<Shortcut> {
    use egui::Key;
    Some(match key {
        Key::F=>Shortcut::Frame,
        Key::G=>Shortcut::Arm(match ty {"squash_stretch"|"soft_transform"=>Handle::SquishPivot,"linear_taper"=>Handle::TaperCenter,
            "lattice"=>Handle::LatticeStrength,"point_deform"=>Handle::PointRadius,"path_deform"=>Handle::PathOffset,_=>Handle::Origin}),
        Key::H=>Shortcut::Arm(match ty {"bend"|"linear_taper"|"path_deform"=>Handle::Up,
            "twist"|"squash_stretch"=>Handle::Direction,"lattice"=>Handle::LatticeStrength,
            "point_deform"=>Handle::PointRadius,"soft_transform"=>Handle::SoftRadius,_=>return None}),
        Key::B=>Shortcut::Arm(match ty {"bend"=>Handle::BendArc,"twist"=>Handle::TwistRing,"linear_taper"=>Handle::TaperX,
            "squash_stretch"=>Handle::SquishPivot,"lattice"=>Handle::LatticeStrength,"path_deform"=>Handle::PathTwist,
            "point_deform"=>Handle::PointRadius,"soft_transform"=>Handle::SoftRadius,_=>return None}),
        Key::N=>Shortcut::Fit,
        Key::M=>Shortcut::Arm(match ty {"bend"=>Handle::Length,"twist"=>Handle::Direction,"linear_taper"=>Handle::TaperY,
            "squash_stretch"=>Handle::Direction,"lattice"=>Handle::LatticeStrength,"path_deform"=>Handle::PathOffset,
            "point_deform"=>Handle::PointStrength,"soft_transform"=>Handle::SoftRadius,_=>return None}),
        _=>return None,
    })
}
pub fn fit_capture_edits(ty:&str,params:&Value,lo:V3,hi:V3)->Vec<(&'static str,Value)> {
    let center=mul(add(lo,hi),0.5);
    let frame=CaptureFrame::from_params(ty,params,1.);
    let size=sub(hi,lo);
    let axial=(0..3).map(|k|size[k].abs()*frame.direction[k].abs()).sum::<f64>().max(1e-6);
    let radius=size.iter().map(|s|s.abs()).fold(0.,f64::max)*0.5;
    match ty {
        "bend"|"linear_taper"=>vec![("origin",json!(sub(center,mul(frame.direction,axial*0.5)))),("length",json!(axial))],
        "twist"=>vec![("origin",json!(center))],
        "squash_stretch"=>vec![("center",json!(center))],
        "point_deform"|"soft_transform"=>vec![("radius",json!(radius))],
        "path_deform"=>vec![("offset",json!(0.))],
        "lattice"=>vec![], // The rest cage, rather than a capture parameter, defines its bounds.
        _=>vec![],
    }
}
/// Consume only unmodified viewport keys with a selected deformer and no active modal/gizmo.
pub fn keyboard(ctx:&egui::Context,graph:&Graph,selected:&str,tool:Tool,viewport_hovered:bool,busy:bool,out:&mut Actions) {
    if busy || !viewport_hovered || tool!=Tool::Select || ctx.wants_keyboard_input() || active(ctx) {return;}
    let Some((_,def))=graph.nodes.iter().find(|(id,_)|id==selected) else {return;};
    let ty=def.node_type.as_str();
    if !is_deformer(ty) {return;}
    let events=ctx.input(|i|i.events.clone());
    if events.iter().any(|e|matches!(e,egui::Event::Key{key:egui::Key::Escape,pressed:true,..})) {
        ctx.data_mut(|d|d.remove::<(String,Handle)>(armed_id()));
        return;
    }
    let mut consumed=Vec::new();
    for (i,event) in events.iter().enumerate() {
        let egui::Event::Key{key,pressed:true,repeat:false,modifiers,..}=event else {continue;};
        if modifiers.alt || modifiers.ctrl || modifiers.shift || modifiers.mac_cmd {continue;}
        let Some(action)=shortcut(*key,ty) else {continue;};
        consumed.push(i);
        match action {
            Shortcut::Frame=>out.push(UiAction::FrameSelected),
            Shortcut::Arm(handle)=>ctx.data_mut(|d|d.insert_temp(armed_id(),(selected.to_owned(),handle))),
            Shortcut::Fit=>{
                let input=def.inputs.iter().find(|(port,_)|port=="geo").and_then(|(_,ids)|ids.first());
                let bounds=input.and_then(|id|graph.cook(Some(id)).ok()).and_then(|geo|{
                    let mut lo=[f64::INFINITY;3];let mut hi=[f64::NEG_INFINITY;3];
                    for p in geo.p().chunks_exact(3) {for k in 0..3 {lo[k]=lo[k].min(p[k] as f64);hi[k]=hi[k].max(p[k] as f64);}}
                    (geo.npoints>0).then_some((lo,hi))
                });
                if let Some((lo,hi))=bounds {
                    for (name,value) in fit_capture_edits(ty,&json!(def.params),lo,hi) {
                        out.push(UiAction::Edit{edit:Edit::Param{node:selected.into(),name:name.into(),value},continuous:false});
                    }
                }
            }
        }
    }
    if !consumed.is_empty() {let mut n=0;ctx.input_mut(|i|i.events.retain(|_|{let keep=!consumed.contains(&n);n+=1;keep}));}
}

fn projected(camera:CameraInfo,rect:Rect,p:V3)->Option<Pos2> { super::transform_gizmo::project(camera,rect,f32v(p)) }
pub fn paint_capture(ui:&egui::Ui,rect:Rect,v:&View,frame:&CaptureFrame) {
    let painter=ui.painter_at(rect);
    let box_color=Color32::from_rgb(180,90,240);
    let guide=Stroke::new(1.1,box_color);
    for (index,(a,b)) in frame.segments().into_iter().enumerate() {
        let stroke=if (4..8).contains(&index) {Stroke::new(1.4,Color32::from_rgb(50,220,225))} else {guide};
        if let (Some(a),Some(b))=(projected(v.camera,rect,a),projected(v.camera,rect,b)) { painter.line_segment([a,b],stroke); }
    }
    let origin=projected(v.camera,rect,frame.origin);
    if let Some(origin)=origin {
        painter.circle_filled(origin,4.,Color32::from_rgb(70,210,220));
        for (axis,color) in [(frame.side,Color32::RED),(frame.up,Color32::GREEN),(frame.direction,Color32::LIGHT_BLUE)] {
            if let Some(tip)=projected(v.camera,rect,add(frame.origin,mul(axis,frame.radius*0.45))) {
                painter.line_segment([origin,tip],Stroke::new(2.,color));
            }
        }
    }
    let arrow_tip=frame.at(frame.length+frame.radius*0.35,0.,0.);
    let arrow_left=frame.at(frame.length+frame.radius*0.1,frame.radius*0.12,0.);
    let arrow_right=frame.at(frame.length+frame.radius*0.1,-frame.radius*0.12,0.);
    if let (Some(t),Some(l),Some(r))=(projected(v.camera,rect,arrow_tip),projected(v.camera,rect,arrow_left),projected(v.camera,rect,arrow_right)) {
        painter.add(egui::Shape::convex_polygon(vec![t,l,r],Color32::from_rgb(180,90,240),Stroke::NONE));
    }
    let fin=[frame.at(0.,0.,frame.radius),frame.at(frame.length*0.2,0.,frame.radius*0.5),frame.origin];
    if let (Some(a),Some(b),Some(c))=(projected(v.camera,rect,fin[0]),projected(v.camera,rect,fin[1]),projected(v.camera,rect,fin[2])) {
        painter.add(egui::Shape::convex_polygon(vec![a,b,c],Color32::from_rgba_unmultiplied(50,220,225,70),Stroke::new(1.,Color32::from_rgb(50,220,225))));
    }
    for (handle,world) in capture_handles(frame) {
        if let Some(pos)=projected(v.camera,rect,world) {
            let color=if handle==Handle::Up {Color32::from_rgb(50,220,225)} else {box_color};
            painter.circle_filled(pos,5.,color);
            painter.circle_stroke(pos,5.,Stroke::new(1.,Color32::WHITE));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aligned_up_has_stable_orthogonal_frame_and_angle() {
        let p=json!({"direction":[0.,0.,1.],"up":[0.,0.,1.],"up_vector_angle":90.});
        let f=CaptureFrame::from_params("bend",&p,1.);
        assert!(dot(f.up,f.direction).abs()<1e-12);
        assert!(dot(f.side,f.up).abs()<1e-12);
        assert_eq!(f.segments().len(),12);
        assert_eq!(f,CaptureFrame::from_params("bend",&p,1.));
    }
    #[test]
    fn capture_drag_produces_exact_json() {
        let p=json!({"origin":[0.,0.,0.],"length":2.});
        let f=CaptureFrame::from_params("bend",&p,1.);
        assert_eq!(apply_handle_drag(Handle::Origin,&f,&p,[1.,2.,3.]),vec![("origin",json!([1.,2.,3.]))]);
        assert_eq!(apply_handle_drag(Handle::Length,&f,&p,[0.,0.,1.]),vec![("length",json!(3.))]);
        assert_eq!(apply_handle_drag(Handle::Direction,&f,&p,[2.,0.,-2.]),vec![("direction",json!([1.,0.,0.]))]);
        assert_eq!(apply_handle_drag(Handle::Up,&f,&p,[0.,-2.,0.]),vec![("up",json!([0.,-1.,0.]))]);
    }
    #[test]
    fn each_deformer_has_working_handle_edits() {
        let f=CaptureFrame::from_params("bend",&json!({}),2.);
        let cases=[
            ("bend",Handle::BendArc,json!({"bend":45.}),"bend",json!(90.)),
            ("twist",Handle::TwistRing,json!({"strength":90.}),"strength",json!(135.)),
            ("linear_taper",Handle::TaperX,json!({"taper_x":0.5}),"taper_x",json!(1.)),
            ("squash_stretch",Handle::SquishPivot,json!({"center":[0.,0.,0.]}),"center",json!([1.,0.,0.])),
            ("lattice",Handle::LatticeStrength,json!({"strength":1.}),"strength",json!(1.5)),
            ("path_deform",Handle::PathTwist,json!({"twist":0.}),"twist",json!(45.)),
            ("point_deform",Handle::PointRadius,json!({"radius":0.}),"radius",json!(1.)),
            ("soft_transform",Handle::SoftRadius,json!({"radius":1.}),"radius",json!(2.)),
        ];
        for (ty,handle,params,name,want) in cases {
            assert!(handles_for_node(ty,&f).iter().any(|(h,_)|*h==handle),"{ty}");
            assert_eq!(apply_handle_drag(handle,&f,&params,[1.,0.,0.]),vec![(name,want)],"{ty}");
        }
        assert_eq!(apply_handle_drag(Handle::TaperY,&f,&json!({}),[0.,1.,0.]),vec![("taper_y",json!(1.))]);
        assert_eq!(apply_handle_drag(Handle::PathOffset,&f,&json!({}),[0.,0.,2.]),vec![("offset",json!(2.))]);
    }
    #[test]
    fn shortcuts_cover_eight_deformers_and_fit_uses_input_bounds() {
        for ty in ["bend","twist","linear_taper","squash_stretch","lattice","path_deform","point_deform","soft_transform"] {
            for key in [egui::Key::F,egui::Key::G,egui::Key::H,egui::Key::B,egui::Key::N,egui::Key::M] {
                assert!(shortcut(key,ty).is_some(),"{ty:?} {key:?}");
            }
        }
        let edits=fit_capture_edits("bend",&json!({"direction":[0.,0.,1.]}),[-1.,-2.,-3.],[1.,2.,3.]);
        assert_eq!(edits,vec![("origin",json!([0.,0.,-3.])),("length",json!(6.))]);
        assert_eq!(fit_capture_edits("point_deform",&json!({}),[-1.,-2.,-3.],[1.,2.,3.]),vec![("radius",json!(3.))]);
    }
    #[test]
    fn keyboard_consumes_conflicting_g_and_b_only_in_deformer_context() {
        let graph=Graph::from_value(&json!({"nodes":{"box":{"type":"box"},"bend":{"type":"bend","inputs":{"geo":"box"}}}})).unwrap();
        let ctx=egui::Context::default();
        for (selected,tool,expect_consumed) in [("bend",Tool::Select,true),("box",Tool::Select,false),("bend",Tool::Move,false)] {
            let events=[egui::Key::G,egui::Key::B].map(|key|egui::Event::Key{key,physical_key:None,pressed:true,repeat:false,modifiers:egui::Modifiers::NONE});
            let _=ctx.run(egui::RawInput{events:events.into(),..Default::default()},|ctx|{
                keyboard(ctx,&graph,selected,tool,true,false,&mut Actions::default());
                assert_eq!(ctx.input(|i|i.events.iter().any(|e|matches!(e,egui::Event::Key{key:egui::Key::G,..}))),!expect_consumed);
            });
        }
    }
    #[test]
    fn fit_key_emits_input_bounds_parameter_edits() {
        let graph=Graph::from_value(&json!({"nodes":{
            "box":{"type":"box","params":{"size":[2.,4.,6.]}},
            "bend":{"type":"bend","inputs":{"geo":"box"},"params":{"direction":[0.,0.,1.]}}
        }})).unwrap();
        let ctx=egui::Context::default();
        let _=ctx.run(egui::RawInput{events:vec![egui::Event::Key{key:egui::Key::N,physical_key:None,pressed:true,
            repeat:false,modifiers:egui::Modifiers::NONE}],..Default::default()},|ctx|{
            let mut out=Actions::default();
            keyboard(ctx,&graph,"bend",Tool::Select,true,false,&mut out);
            let edits:Vec<_>=out.0.into_iter().filter_map(|a|match a {UiAction::Edit{edit:Edit::Param{name,value,..},..}=>Some((name,value)),_=>None}).collect();
            assert_eq!(edits,vec![("origin".into(),json!([0.,0.,-3.])),("length".into(),json!(6.))]);
        });
    }
    #[test]
    fn viewport_drag_wins_over_region_select_and_emits_edit() {
        let mut f=crate::ui::testing::Fixture::new();
        f.doc=minihou_app::Document::parse(json!({"nodes":{"box":{"type":"box"},
            "bend":{"type":"bend","inputs":{"geo":"box"}}},"display":"bend"}).to_string()).unwrap();
        f.graph=Graph::from_value(f.doc.value()).unwrap();
        f.selected="bend".into(); f.display="bend".into();
        f.camera=CameraInfo{projection:[[1.,0.,0.,0.],[0.,1.,0.,0.],[0.,0.,1.,0.],[0.,0.,0.,1.]],
            ortho:true,ray_right:[1.,0.,0.],ray_up:[0.,1.,0.],ray_forward:[0.,0.,-1.],..Default::default()};
        let ctx=egui::Context::default();
        let rect=Rect::from_min_max(egui::pos2(0.,0.),egui::pos2(500.,500.));
        let center=rect.center();
        let frame=|events:Vec<egui::Event>|{
            let mut out=Actions::default();
            let _=ctx.run(egui::RawInput{screen_rect:Some(rect),events,..Default::default()},|ctx|{
                egui::CentralPanel::default().show(ctx,|ui|{
                    super::super::selection_region::handle(ctx,rect,&f.view(),&mut out);
                    show(ui,rect,&f.view(),&mut out);
                });
            });
            out.0
        };
        frame(vec![egui::Event::PointerMoved(center),egui::Event::PointerButton{pos:center,
            button:egui::PointerButton::Primary,pressed:true,modifiers:Default::default()}]);
        assert!(active(&ctx));
        let actions=frame(vec![egui::Event::PointerMoved(center+egui::vec2(25.,0.))]);
        assert!(actions.iter().any(|a|matches!(a,UiAction::Edit{edit:Edit::Param{name,value,..},continuous:true}
            if name=="origin" && value[0].as_f64().is_some_and(|x|x>0.05))));
        assert!(!actions.iter().any(|a|matches!(a,UiAction::RegionSelect{..})));
    }
}
