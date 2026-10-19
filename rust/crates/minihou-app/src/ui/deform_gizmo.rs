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
pub enum Handle { Origin, Direction, Length, Up }
pub fn capture_handles(frame:&CaptureFrame)->[(Handle,V3);4] {
    [(Handle::Origin,frame.origin),(Handle::Direction,frame.at(frame.length+frame.radius*0.35,0.,0.)),
     (Handle::Length,frame.at(frame.length,0.,0.)),(Handle::Up,frame.at(0.,0.,frame.radius))]
}
/// Drag input is a world-space displacement measured from the pointer's original position.
pub fn apply_handle_drag(handle:Handle, frame:&CaptureFrame, params:&Value, delta:V3)->Vec<(&'static str,Value)> {
    match handle {
        Handle::Origin => vec![(if params.get("origin").is_some() {"origin"} else {"center"},json!(add(frame.origin,delta)))],
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
}
