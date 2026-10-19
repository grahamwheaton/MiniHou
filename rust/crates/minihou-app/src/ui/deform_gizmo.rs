//! Shared capture frame and viewport guides for deformers.
use eframe::egui::{self, Color32, Pos2, Rect, Stroke};
use serde_json::{json, Value};
use super::{CameraInfo, View};

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

fn projected(camera:CameraInfo,rect:Rect,p:V3)->Option<Pos2> { super::transform_gizmo::project(camera,rect,f32v(p)) }
pub fn paint_capture(ui:&egui::Ui,rect:Rect,v:&View,frame:&CaptureFrame) {
    let painter=ui.painter_at(rect);
    let box_color=Color32::from_rgb(180,90,240);
    let guide=Stroke::new(1.1,box_color);
    for (a,b) in frame.segments() {
        if let (Some(a),Some(b))=(projected(v.camera,rect,a),projected(v.camera,rect,b)) { painter.line_segment([a,b],guide); }
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
}
