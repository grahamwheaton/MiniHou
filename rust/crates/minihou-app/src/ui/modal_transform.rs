//! Input and maths for viewport modal transforms. No document or renderer state lives here.
use eframe::egui::{Key, Pos2, Vec2};
use super::{CameraInfo, Orientation, SnapTo, TransformOp};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind { Move, Rotate, Scale }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Constraint { Free, Axis(usize, bool), Plane(usize, bool) }
impl Constraint {
    pub fn key(self, axis: usize, plane: bool) -> Self {
        if plane { return Self::Plane(axis, false); }
        match self { Self::Axis(a, false) if a == axis => Self::Axis(axis, true),
            Self::Axis(a, true) if a == axis => Self::Free, _ => Self::Axis(axis, false) }
    }
    pub fn label(self) -> &'static str { match self {
        Self::Free => "Free", Self::Axis(0, false) => "X", Self::Axis(1, false) => "Y", Self::Axis(2, false) => "Z",
        Self::Axis(0, true) => "Local X", Self::Axis(1, true) => "Local Y", Self::Axis(2, true) => "Local Z",
        Self::Plane(0, _) => "YZ plane", Self::Plane(1, _) => "XZ plane", Self::Plane(2, _) => "XY plane", _ => "Free" } }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SketchStage { Off, Pivot, Reference, Angle }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CageHandle { Face(usize, bool), Corner([bool; 3]) }

#[derive(Clone, Debug)]
pub struct Modal {
    pub kind: Kind, pub start: Pos2, pub pointer: Pos2, pub pivot: [f32; 3],
    pub rotation: [f32; 3],
    pub constraint: Constraint, pub numeric: [String; 3], pub field: usize,
    pub snap_to: SnapTo, pub proportional: bool, pub radius: f32,
    pub initial_proportional: bool, pub initial_radius: f32,
    pub trackball: bool, pub sketch: SketchStage, pub sketch_pivot: Option<[f32; 3]>,
    pub sketch_reference: Option<Pos2>, pub cage: bool, pub cage_handle: Option<CageHandle>,
    pub cage_start: Pos2, pub bounds: ([f32; 3], [f32; 3]),
    pub reference: Option<[f32; 3]>, pub target: Option<[f32; 3]>,
    pub target_shape: Option<[[f32;3];3]>,
    pub last: [f32; 3], pub mmb_start: Option<Pos2>,
}
impl Modal {
    pub fn new(kind: Kind, start: Pos2, pivot: [f32; 3], bounds: ([f32; 3], [f32; 3]), snap_to: SnapTo, proportional: bool, radius: f32) -> Self {
        Self { kind, start, pointer: start, pivot, rotation:[0.;3], constraint: Constraint::Free,
            numeric: Default::default(), field: 0, snap_to, proportional, radius,
            initial_proportional:proportional, initial_radius:radius, trackball: false,
            sketch: SketchStage::Off, sketch_pivot: None, sketch_reference: None,
            cage: false, cage_handle: None, cage_start: start, bounds, reference: None, target: None, target_shape:None,
            last: if kind == Kind::Scale { [1.; 3] } else { [0.; 3] }, mmb_start: None }
    }
    pub fn type_key(&mut self, key: Key, shift: bool) -> bool {
        let field = if self.kind == Kind::Move && self.constraint == Constraint::Free { self.field } else { 0 };
        let s = &mut self.numeric[field];
        match key {
            Key::Num0 | Key::Num1 | Key::Num2 | Key::Num3 | Key::Num4 | Key::Num5 | Key::Num6 | Key::Num7 | Key::Num8 | Key::Num9 => {
                s.push(char::from(b'0' + (key as u8 - Key::Num0 as u8))); true
            }
            Key::Period if !s.contains('.') => { s.push('.'); true }
            Key::Minus if s.is_empty() => { s.push('-'); true }
            Key::Backspace => { s.pop(); true }
            Key::Tab if self.kind == Kind::Move => { self.field = (self.field + if shift { 2 } else { 1 }) % 3; true }
            _ => false,
        }
    }
    pub fn typed(&self, axis: usize) -> Option<f32> {
        let index = if self.constraint == Constraint::Free && self.kind == Kind::Move { axis } else { 0 };
        self.numeric[index].parse().ok()
    }
    pub fn toggle_axis(&mut self, axis: usize, plane: bool) { self.constraint = self.constraint.key(axis, plane); }
    pub fn cycle_snap(&mut self) { self.snap_to = match self.snap_to { SnapTo::Increment => SnapTo::Vertex,
        SnapTo::Vertex => SnapTo::Edge, SnapTo::Edge => SnapTo::Face, _ => SnapTo::Increment }; }
    pub fn wheel(&mut self, delta: f32) { self.radius = (self.radius * (delta * 0.12).exp()).clamp(0.001, 1e6); }
    pub fn status(&self, snap: bool) -> String {
        let title = match self.kind { Kind::Move => "Move", Kind::Rotate => "Rotate", Kind::Scale => "Scale" };
        let numbers = if self.kind == Kind::Move { format!("X: {:.3}  Y: {:.3}  Z: {:.3}", self.last[0], self.last[1], self.last[2]) }
            else { format!("{:.3}{}", self.last.iter().copied().find(|v| *v != 0. && *v != 1.).unwrap_or(self.last[0]), if self.kind == Kind::Rotate { "°" } else { "×" }) };
        format!("{title}  {}  {numbers}  (Snap: {}){}{}", self.constraint.label(),
            if snap { self.snap_to.label() } else { "Off" }, if self.proportional { "  Proportional" } else { "" },
            if self.cage { "  Cage" } else if self.sketch != SketchStage::Off { "  Two-point" }
            else if self.trackball { "  Trackball" } else { "" })
    }
    #[allow(clippy::too_many_arguments)]
    pub fn desired(&self, center: Pos2, pixels_per_unit: f32, frame: [[f32; 3]; 3], camera: CameraInfo,
        snap: bool, increment: f32, precision: bool) -> ([f32; 3], [f32; 3]) {
        let speed = if precision { 0.1 } else { 1. };
        let delta = (self.pointer - self.start) * speed;
        let step = match self.kind { Kind::Move => increment, Kind::Rotate => 5., Kind::Scale => 0.1 };
        let q = |v: f32| if snap { snap_value(v, step) } else { v };
        let mut pivot = self.pivot;
        let mut values = match self.kind {
            Kind::Move => {
                let basis = [frame[0], frame[1], frame[2]];
                let mut v = [delta.x / pixels_per_unit, -delta.y / pixels_per_unit, 0.];
                match self.constraint {
                    Constraint::Axis(axis, _) => {
                        let direction = projected_axis(basis[axis], camera);
                        let amount = delta.dot(direction) / pixels_per_unit;
                        v = basis[axis].map(|c| c * amount);
                    }
                    Constraint::Plane(axis, _) => { v[axis] = 0.; }
                    Constraint::Free => {}
                }
                if let (Some(reference), Some(target)) = (self.reference, self.target) {
                    v = [0, 1, 2].map(|i| target[i] - reference[i]);
                }
                if let Constraint::Axis(axis, _) = self.constraint {
                    if let Some(amount)=self.typed(0) { v=frame[axis].map(|c|c*amount); }
                    else { v=v.map(q); }
                } else { for (i,item) in v.iter_mut().enumerate() { *item = self.typed(i).unwrap_or_else(|| q(*item)); } }
                v
            }
            Kind::Rotate => {
                if matches!(self.sketch, SketchStage::Pivot | SketchStage::Reference) { return (self.last,pivot); }
                if self.trackball && self.constraint==Constraint::Free && self.sketch==SketchStage::Off && self.typed(0).is_none() {
                    let mut v=[0.;3]; v[0]=-delta.y*0.4; v[1]=delta.x*0.4;
                    return (v.map(q),pivot);
                }
                let angle = if self.sketch == SketchStage::Angle {
                    self.sketch_reference.map(|reference| angle_between(reference, self.pointer, center)).unwrap_or(0.)
                } else { angle_between(self.start, self.pointer, center) };
                let angle = self.typed(0).unwrap_or_else(|| q(angle * speed));
                let axis = match self.constraint { Constraint::Axis(a, _) => a, _ => dominant_axis(camera.ray_forward) };
                let mut v = [0.; 3]; v[axis] = angle; v
            }
            Kind::Scale => {
                if self.cage && self.cage_handle.is_none() { return ([1.;3],pivot); }
                let start_distance = (self.start - center).length();
                let denominator = start_distance.max(12.);
                let factor = self.typed(0).unwrap_or_else(|| q(1. +
                    ((self.pointer - center).length() - start_distance) / denominator * speed));
                let mut v = match self.constraint { Constraint::Axis(a, _) => { let mut x = [1.; 3]; x[a] = factor; x },
                    Constraint::Plane(a, _) => { let mut x = [factor; 3]; x[a] = 1.; x }, _ => [factor; 3] };
                if let Some(handle) = self.cage_handle {
                    let (p, scales) = cage_transform(self.bounds, handle, (self.pointer - self.cage_start) * speed,
                        pixels_per_unit, precision, camera);
                    pivot = p; v = scales;
                }
                v
            }
        };
        if self.kind == Kind::Move && self.constraint == Constraint::Free && self.numeric.iter().any(|s| !s.is_empty()) {
            for (i, value) in values.iter_mut().enumerate() { if self.numeric[i].is_empty() { *value = 0.; } }
        }
        (values, pivot)
    }
    pub fn step(&mut self, values: [f32; 3]) -> TransformOp {
        self.last = values;
        match self.kind {
            Kind::Move => TransformOp::Move(values),
            Kind::Rotate => TransformOp::Rotate(values),
            Kind::Scale => TransformOp::Scale(values),
        }
    }
}
pub fn snap_value(value: f32, step: f32) -> f32 { if step <= 0. { value } else { (value / step).round() * step } }
pub fn projected_axis(axis: [f32; 3], camera: CameraInfo) -> Vec2 {
    let x = dot(axis, camera.ray_right); let y = -dot(axis, camera.ray_up);
    let v=Vec2::new(x,y); if v.length_sq()>1e-8 { v.normalized() } else { Vec2::X }
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 { (0..3).map(|i| a[i] * b[i]).sum() }
pub fn dominant_axis(v: [f32; 3]) -> usize { (0..3).max_by(|&a, &b| v[a].abs().total_cmp(&v[b].abs())).unwrap_or(2) }
pub fn closest_axis(motion: Vec2, frame: [[f32; 3]; 3], camera: CameraInfo) -> usize {
    let score = |i: usize| {
        let x=dot(frame[i],camera.ray_right); let y=-dot(frame[i],camera.ray_up);
        let direction=Vec2::new(x,y);
        if direction.length_sq()<1e-8 { -1. } else { motion.dot(direction.normalized()).abs() }
    };
    (0..3).max_by(|&a,&b|score(a).total_cmp(&score(b))).unwrap_or(0)
}
pub fn angle_between(start: Pos2, end: Pos2, pivot: Pos2) -> f32 {
    let a = start - pivot; let b = end - pivot;
    (a.x * b.y - a.y * b.x).atan2(a.dot(b)).to_degrees()
}
pub fn cage_transform(bounds: ([f32; 3], [f32; 3]), handle: CageHandle, motion: Vec2, pixels_per_unit: f32,
    center: bool, camera: CameraInfo) -> ([f32; 3], [f32; 3]) {
    let (lo, hi) = bounds;
    let mut pivot = [0,1,2].map(|i| (lo[i] + hi[i]) * 0.5);
    let mut scale = [1.; 3];
    match handle {
        CageHandle::Face(axis, high) => {
            if !center { pivot[axis] = if high { lo[axis] } else { hi[axis] }; }
            let mut axis_vector=[0.;3]; axis_vector[axis]=1.;
            let distance = motion.dot(projected_axis(axis_vector,camera));
            let sign = if high { 1. } else { -1. };
            scale[axis] += sign * distance / (pixels_per_unit * (hi[axis] - lo[axis]).abs().max(1e-4)) * if center { 2. } else { 1. };
        }
        CageHandle::Corner(sides) => {
            if !center { pivot = [0,1,2].map(|i| if sides[i] { lo[i] } else { hi[i] }); }
            let extent = (0..3).map(|i| (hi[i] - lo[i]).powi(2)).sum::<f32>().sqrt().max(1e-4);
            let outward=[0,1,2].map(|i|if sides[i] {hi[i]-lo[i]} else {lo[i]-hi[i]});
            let factor = 1. + motion.dot(projected_axis(outward,camera)) / (pixels_per_unit * extent)
                * if center { 2. } else { 1. };
            scale = [factor; 3];
        }
    }
    (pivot, scale)
}
pub fn cage_target(bounds: ([f32;3],[f32;3]), pivot: [f32;3], scale: [f32;3]) -> ([f32;3],[f32;3]) {
    let (lo,hi)=bounds;
    let a=[0,1,2].map(|i|pivot[i]+(lo[i]-pivot[i])*scale[i]);
    let b=[0,1,2].map(|i|pivot[i]+(hi[i]-pivot[i])*scale[i]);
    ([0,1,2].map(|i|(a[i]+b[i])*0.5),[0,1,2].map(|i|(b[i]-a[i]).abs()))
}
pub fn orientation_frame(orientation: Orientation, camera: CameraInfo, rotation: [f32; 3]) -> [[f32; 3]; 3] {
    match orientation {
        Orientation::View => [camera.ray_right, camera.ray_up, camera.ray_forward],
        Orientation::Local | Orientation::Gimbal | Orientation::Normal => {
            let [x,y,z] = rotation.map(f32::to_radians); let (sx,cx)=x.sin_cos(); let (sy,cy)=y.sin_cos(); let (sz,cz)=z.sin_cos();
            [[cz*cy, sz*cy, -sy], [cz*sy*sx-sz*cx, sz*sy*sx+cz*cx, cy*sx], [cz*sy*cx+sz*sx, sz*sy*cx-cz*sx, cy*cx]]
        }
        _ => [[1.,0.,0.],[0.,1.,0.],[0.,0.,1.]],
    }
}

#[cfg(test)] mod tests {
    use super::*;
    use eframe::egui::pos2;
    #[test] fn constraint_cycles_and_plane() {
        let a = Constraint::Free.key(0, false); assert_eq!(a, Constraint::Axis(0,false));
        assert_eq!(a.key(0,false), Constraint::Axis(0,true)); assert_eq!(a.key(0,false).key(0,false), Constraint::Free);
        assert_eq!(a.key(2,true), Constraint::Plane(2,false));
    }
    #[test] fn numeric_and_snap() {
        let mut m = Modal::new(Kind::Move,pos2(0.,0.),[0.;3],([0.;3],[1.;3]),SnapTo::Increment,false,1.);
        assert!(m.type_key(Key::Minus,false)); assert!(m.type_key(Key::Num2,false)); assert!(m.type_key(Key::Period,false));
        assert!(m.type_key(Key::Num5,false)); assert_eq!(m.typed(0),Some(-2.5));
        m.type_key(Key::Backspace,false); assert_eq!(m.typed(0),Some(-2.)); m.type_key(Key::Tab,false); assert_eq!(m.field,1);
        assert_eq!(snap_value(1.24,0.25),1.25);
    }
    #[test] fn motion_angle_and_cage() {
        let camera = CameraInfo { ray_right:[1.,0.,0.], ray_up:[0.,1.,0.], ..Default::default() };
        assert_eq!(closest_axis(Vec2::new(20.,1.),[[1.,0.,0.],[0.,1.,0.],[0.,0.,1.]],camera),0);
        assert!((angle_between(pos2(1.,0.),pos2(0.,1.),pos2(0.,0.))-90.).abs()<1e-4);
        let (p,s) = cage_transform(([0.;3],[2.;3]),CageHandle::Face(0,true),Vec2::new(100.,0.),100.,false,camera);
        assert_eq!(p[0],0.); assert!((s[0]-1.5).abs()<1e-4);
        assert_eq!(cage_target(([0.;3],[2.;3]),p,s),([1.5,1.,1.],[3.,2.,2.]));
    }
    #[test] fn scale_at_pivot_starts_at_one_and_trackball_changes_two_axes() {
        let camera=CameraInfo {ray_right:[1.,0.,0.],ray_up:[0.,1.,0.],ray_forward:[0.,0.,1.],..Default::default()};
        let frame=[[1.,0.,0.],[0.,1.,0.],[0.,0.,1.]];
        let mut scale=Modal::new(Kind::Scale,pos2(10.,10.),[0.;3],([0.;3],[1.;3]),SnapTo::Increment,false,1.);
        assert_eq!(scale.desired(pos2(10.,10.),100.,frame,camera,false,0.25,false).0,[1.;3]);
        scale.pointer=pos2(22.,10.);
        assert!(scale.desired(pos2(10.,10.),100.,frame,camera,false,0.25,false).0[0]>1.);
        let mut rotate=Modal::new(Kind::Rotate,pos2(10.,10.),[0.;3],([0.;3],[1.;3]),SnapTo::Increment,false,1.);
        rotate.trackball=true; rotate.pointer=pos2(30.,20.);
        let values=rotate.desired(pos2(10.,10.),100.,frame,camera,false,0.25,false).0;
        assert!(values[0]!=0. && values[1]!=0.);
    }
}
