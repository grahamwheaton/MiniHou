//! Input and maths for viewport modal transforms. No document or renderer state lives here.
use eframe::egui::{Key, Pos2, Vec2};
use super::{CameraInfo, Orientation, SnapTo, SnapTargets, TransformOp};
use crate::{transform_math, ui, App};
use eframe::egui;
use serde_json::json;
use Kind as ModalKind;

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
pub enum BaseStage { Off, Base, Target, SourceDirection, TargetDirection, Done }
impl BaseStage {
    pub fn enter(active: bool) -> Self { if active { Self::Base } else { Self::Off } }
}
pub fn base_snap_shortcut(key: Key, transform_active: bool) -> bool {
    key==Key::B && transform_active
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CageHandle { Face(usize, bool), Corner([bool; 3]) }

#[derive(Clone, Debug)]
pub struct Modal {
    pub kind: Kind, pub start: Pos2, pub pointer: Pos2, pub pivot: [f32; 3],
    pub rotation: [f32; 3],
    pub units: Option<crate::units::UnitSettings>,
    pub constraint: Constraint, pub numeric: [String; 3], pub field: usize,
    pub snap_to: SnapTargets, pub target_kind: Option<SnapTo>, pub proportional: bool, pub radius: f32,
    pub initial_proportional: bool, pub initial_radius: f32,
    pub trackball: bool, pub sketch: SketchStage, pub sketch_pivot: Option<[f32; 3]>,
    pub sketch_reference: Option<Pos2>, pub cage: bool, pub cage_handle: Option<CageHandle>,
    pub base_stage: BaseStage, pub base_point: Option<[f32; 3]>, pub base_target: Option<[f32; 3]>,
    pub source_direction: Option<[f32; 3]>, pub target_direction: Option<[f32; 3]>,
    pub base_axis: Option<[f32; 3]>, pub pointer_direction: Option<[f32; 3]>, pub base_angle: Option<f32>,
    pub cage_start: Pos2, pub bounds: ([f32; 3], [f32; 3]),
    pub reference: Option<[f32; 3]>, pub target: Option<[f32; 3]>,
    pub target_shape: Option<[[f32;3];3]>,
    pub last: [f32; 3], pub mmb_start: Option<Pos2>,
}
impl Modal {
    pub fn new(kind: Kind, start: Pos2, pivot: [f32; 3], bounds: ([f32; 3], [f32; 3]), snap_to: impl Into<SnapTargets>, proportional: bool, radius: f32) -> Self {
        Self { kind, start, pointer: start, pivot, units:None, rotation:[0.;3], constraint: Constraint::Free,
            numeric: Default::default(), field: 0, snap_to: snap_to.into(), target_kind: None, proportional, radius,
            initial_proportional:proportional, initial_radius:radius, trackball: false,
            sketch: SketchStage::Off, sketch_pivot: None, sketch_reference: None,
            base_stage: BaseStage::Off, base_point: None, base_target: None,
            source_direction: None, target_direction: None,
            base_axis: None, pointer_direction: None, base_angle: None,
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
        if self.kind==Kind::Move {
            if let Some(units)=self.units {return units.parse(&self.numeric[index]).map(|value|value as f32);}
        }
        self.numeric[index].parse().ok()
    }
    pub fn toggle_axis(&mut self, axis: usize, plane: bool) { self.constraint = self.constraint.key(axis, plane); }
    pub fn cycle_snap(&mut self) { self.snap_to = match self.snap_to.iter().next().unwrap_or_default() { SnapTo::Increment => SnapTo::Vertex,
        SnapTo::Vertex => SnapTo::Edge, SnapTo::Edge => SnapTo::Face, _ => SnapTo::Increment }.into(); }
    pub fn wheel(&mut self, delta: f32) { self.radius = (self.radius * (delta * 0.12).exp()).clamp(0.001, 1e6); }
    pub fn middle_press(&mut self, pos: Pos2) {
        if self.cage { return; }
        if self.constraint != Constraint::Free { self.constraint = Constraint::Free; }
        else { self.mmb_start = Some(pos); }
    }
    pub fn after_cage_apply(&self, pivot: [f32; 3]) -> Self {
        let (center, size) = cage_target(self.bounds, pivot, self.last);
        let bounds = ([0,1,2].map(|i| center[i] - size[i] * 0.5),
            [0,1,2].map(|i| center[i] + size[i] * 0.5));
        let mut next = Self::new(Kind::Scale, self.pointer, center, bounds, self.snap_to,
            self.proportional, self.radius);
        next.cage = true;
        next.rotation = self.rotation;
        next.units = self.units;
        next
    }
    pub fn status(&self, snap: bool) -> String {
        let title = match self.kind { Kind::Move => "Move", Kind::Rotate => "Rotate", Kind::Scale => "Scale" };
        let numbers = if self.kind == Kind::Move {
            if let Some(units)=self.units {format!("X: {}  Y: {}  Z: {}",units.format(self.last[0] as f64),units.format(self.last[1] as f64),units.format(self.last[2] as f64))}
            else {format!("X: {:.3}  Y: {:.3}  Z: {:.3}", self.last[0], self.last[1], self.last[2])}
        }
            else { format!("{:.3}{}", if self.kind==Kind::Rotate && self.base_stage!=BaseStage::Off {
                self.base_angle.unwrap_or(0.)
            } else { self.last.iter().copied().find(|v| *v != 0. && *v != 1.).unwrap_or(self.last[0]) },
                if self.kind == Kind::Rotate { "°" } else { "×" }) };
        format!("{title}  {}  {numbers}  (Snap: {}){}{}", self.constraint.label(),
            if snap && self.kind==Kind::Rotate && self.base_stage!=BaseStage::Off {
                "15° (Shift: 5°)".into()
            } else if snap { self.snap_to.label() } else { "Off".into() },
            if self.proportional { "  Proportional" } else { "" },
            if self.base_stage != BaseStage::Off { match self.base_stage {
                BaseStage::Base => "  Base snap: pick base", BaseStage::Target => "  Base snap: pick target",
                BaseStage::SourceDirection => "  Base snap: pick source edge direction",
                BaseStage::TargetDirection => "  Base snap: pick target direction", BaseStage::Done => "  Base snap", BaseStage::Off => "" } }
            else if self.cage { "  Cage" } else if self.sketch != SketchStage::Off { "  Two-point" }
            else if self.trackball { "  Trackball" } else { "" })
    }
    #[allow(clippy::too_many_arguments)]
    pub fn desired(&self, center: Pos2, pixels_per_unit: f32, frame: [[f32; 3]; 3], camera: CameraInfo,
        snap: bool, increment: f32, precision: bool) -> ([f32; 3], [f32; 3]) {
        let speed = if precision { 0.1 } else { 1. };
        let step = match self.kind { Kind::Move => increment, Kind::Rotate => 5., Kind::Scale => 0.1 };
        let q = |v: f32| if snap && (self.kind != Kind::Move || (self.snap_to.contains(SnapTo::Increment) && self.target.is_none())) { snap_value(v, step) } else { v };
        if self.base_stage != BaseStage::Off {
            let pivot = self.base_point.unwrap_or(self.pivot);
            let values = match self.kind {
                Kind::Move => self.base_point.zip(self.base_target).map(|(a,b)| {
                    let mut offset = base_snap_offset(a,b);
                    match self.constraint {
                        Constraint::Axis(axis, _) => {
                            let amount = dot(offset, frame[axis]);
                            offset = frame[axis].map(|component| component * amount);
                        }
                        Constraint::Plane(axis, _) => {
                            let amount = dot(offset, frame[axis]);
                            offset = [0,1,2].map(|i| offset[i] - frame[axis][i] * amount);
                        }
                        Constraint::Free => {}
                    }
                    offset
                }).unwrap_or(self.last),
                Kind::Rotate => {
                    let axis=self.rotation_axis(frame,camera);
                    let target=if self.base_stage==BaseStage::Done { self.target_direction }
                        else { self.pointer_direction.or(self.target_direction) };
                    let angle=self.base_point.zip(self.source_direction).zip(target)
                        .and_then(|((base,source),target)|rotate_alignment_angle(base,source,target,axis))
                        .unwrap_or(0.);
                    let angle=self.typed(0).unwrap_or_else(|| if snap {snap_value(angle,if precision {5.} else {15.})} else {angle});
                    rotation_about_axis(axis,angle)
                }
                Kind::Scale => {
                    if self.base_point.is_none() { self.last } else {
                        let start_distance = (self.start - center).length();
                        let denominator = start_distance.max(12.);
                        let factor = self.typed(0).unwrap_or_else(|| q(1. +
                            ((self.pointer - center).length() - start_distance) / denominator * speed));
                        match self.constraint {
                            Constraint::Axis(axis, _) => { let mut values = [1.; 3]; values[axis] = factor; values }
                            Constraint::Plane(axis, _) => { let mut values = [factor; 3]; values[axis] = 1.; values }
                            Constraint::Free => [factor; 3],
                        }
                    }
                }
            };
            return (values,pivot);
        }
        let delta = (self.pointer - self.start) * speed;
        let mut pivot = self.pivot;
        let mut values = match self.kind {
            Kind::Move => {
                let basis = [frame[0], frame[1], frame[2]];
                let mut v = view_plane_motion(delta / pixels_per_unit, camera);
                match self.constraint {
                    Constraint::Axis(axis, _) => {
                        let direction = rightward_axis(basis[axis], camera);
                        let amount = delta.dot(direction) / pixels_per_unit;
                        v = basis[axis].map(|c| c * amount);
                    }
                    Constraint::Plane(axis, _) => { v[axis] = 0.; }
                    Constraint::Free => {}
                }
                if let Some(target) = self.target.filter(|_| snap) {
                    let reference = self.reference.unwrap_or(self.pivot);
                    v = [0, 1, 2].map(|i| target[i] - reference[i]);
                    match self.constraint {
                        Constraint::Axis(axis, _) => { let amount = dot(v, basis[axis]); v = basis[axis].map(|c| c * amount); }
                        Constraint::Plane(axis, _) => { let amount = dot(v, basis[axis]); v = [0,1,2].map(|i| v[i] - basis[axis][i] * amount); }
                        _ => {}
                    }
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
    pub fn rotation_axis(&self, frame: [[f32;3];3], camera: CameraInfo) -> [f32;3] {
        if let Constraint::Axis(axis,_) = self.constraint { frame[axis] }
        else { self.base_axis.unwrap_or(frame[dominant_axis(camera.ray_forward)]) }
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
/// Free move (Blender G): screen motion in units mapped onto the camera's right and up vectors, so the object slides in the view
/// plane in perspective and orthographic views alike. A camera without a basis (tests' default) falls back to world XY.
pub fn view_plane_motion(motion: Vec2, camera: CameraInfo) -> [f32; 3] {
    if dot(camera.ray_right, camera.ray_right) < 1e-8 || dot(camera.ray_up, camera.ray_up) < 1e-8 { return [motion.x, -motion.y, 0.]; }
    [0, 1, 2].map(|i| camera.ray_right[i] * motion.x - camera.ray_up[i] * motion.y)
}
/// The frame axis most aligned with the view direction (Blender: middle-click during G locks to it).
pub fn view_axis(frame: [[f32; 3]; 3], camera: CameraInfo) -> usize {
    (0..3).max_by(|&a, &b| dot(frame[a], camera.ray_forward).abs().total_cmp(&dot(frame[b], camera.ray_forward).abs())).unwrap_or(2)
}
impl Modal {
    /// Middle button released: a drag picks the axis closest to the drag on screen; a click locks the axis closest to the view.
    pub fn middle_release(&mut self, pos: Pos2, frame: [[f32; 3]; 3], camera: CameraInfo) {
        let Some(start) = self.mmb_start.take() else { return };
        let axis = if (pos - start).length() > 4. { closest_axis(pos - start, frame, camera) } else { view_axis(frame, camera) };
        self.constraint = Constraint::Axis(axis, false);
    }
}
/// Blender GG: pressing G again during an edge-mode move switches to Edge Slide on the selected edges.
pub fn double_g_slides(kind: Kind, mode: super::SelectionMode) -> bool { kind == Kind::Move && mode == super::SelectionMode::Edges }
pub fn snap_value(value: f32, step: f32) -> f32 { if step <= 0. { value } else { (value / step).round() * step } }
/// The axis' screen direction, flipped so a mouse move to the right (or up, for an axis pointing along the view) is positive
/// whichever way the camera faces.
pub fn rightward_axis(axis: [f32; 3], camera: CameraInfo) -> Vec2 {
    let d = projected_axis(axis, camera);
    if d.x < -0.05 || (d.x.abs() <= 0.05 && d.y > 0.) { -d } else { d }
}
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
pub fn base_snap_offset(base: [f32;3], target: [f32;3]) -> [f32;3] {
    [0,1,2].map(|i|target[i]-base[i])
}
/// Signed rotation of two directions about an axis. Returns None for a direction parallel to it.
pub fn rotate_alignment_angle(base: [f32;3], source: [f32;3], target: [f32;3], axis: [f32;3]) -> Option<f32> {
    let length=dot(axis,axis).sqrt(); if length<1e-6 { return None; }
    let n=axis.map(|x|x/length);
    let projected=|point: [f32;3]| {
        let v=base_snap_offset(base,point); let along=dot(v,n);
        [0,1,2].map(|i|v[i]-n[i]*along)
    };
    let a=projected(source); let b=projected(target);
    if dot(a,a)<1e-12 || dot(b,b)<1e-12 { return None; }
    let cross=[a[1]*b[2]-a[2]*b[1],a[2]*b[0]-a[0]*b[2],a[0]*b[1]-a[1]*b[0]];
    Some(dot(n,cross).atan2(dot(a,b)).to_degrees())
}
pub fn rotation_about_axis(axis: [f32;3], angle: f32) -> [f32;3] {
    if angle.abs()<1e-6 { return [0.;3]; }
    let matrix=transform_math::axis_angle_matrix(axis.map(f64::from),angle as f64);
    transform_math::matrix_to_euler_xyz(&matrix,[0.;3]).map(|value|value as f32)
}
pub fn feature_rotation_axis(feature: &crate::picking::SnapFeature) -> Option<[f32;3]> {
    let triangle=feature.triangle;
    let edges=[(0,1),(1,2),(2,0)];
    let vector=match feature.kind {
        SnapTo::Face => {
            let a=base_snap_offset(triangle[0],triangle[1]);
            let b=base_snap_offset(triangle[0],triangle[2]);
            [a[1]*b[2]-a[2]*b[1],a[2]*b[0]-a[0]*b[2],a[0]*b[1]-a[1]*b[0]]
        }
        SnapTo::Edge | SnapTo::EdgeCenter => {
            let (a,b)=edges.into_iter().min_by(|&(a,b),&(c,d)| {
                let distance=|i,j| { let edge=base_snap_offset(triangle[i],triangle[j]);
                    let from=base_snap_offset(triangle[i],feature.point);
                    let t=(dot(from,edge)/dot(edge,edge).max(1e-12)).clamp(0.,1.);
                    let residual=[0,1,2].map(|k|from[k]-edge[k]*t); dot(residual,residual) };
                distance(a,b).total_cmp(&distance(c,d))
            }).unwrap();
            base_snap_offset(triangle[a],triangle[b])
        }
        _ => return None,
    };
    let length=dot(vector,vector).sqrt();
    (length>1e-6).then(||vector.map(|component|component/length))
}
pub fn ray_plane_point(origin: [f32;3], direction: [f32;3], point: [f32;3], normal: [f32;3]) -> Option<[f32;3]> {
    let denominator=dot(direction,normal);
    if denominator.abs()<1e-6 {return None;}
    let t=dot(base_snap_offset(origin,point),normal)/denominator;
    (t>0.).then(||[0,1,2].map(|i|origin[i]+direction[i]*t))
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
    #[test] fn base_snap_geometry_and_rotation_direction() {
        assert_eq!(base_snap_offset([2.,-1.,4.],[-3.,5.,1.]),[-5.,6.,-3.]);
        let base=[1.,2.,3.];
        let source=[3.,2.,3.];
        let target=[1.,4.,3.];
        assert!((rotate_alignment_angle(base,source,target,[0.,0.,1.]).unwrap()-90.).abs()<1e-5);
        assert!((rotate_alignment_angle(base,target,source,[0.,0.,1.]).unwrap()+90.).abs()<1e-5);
        assert_eq!(rotate_alignment_angle(base,[1.,2.,4.],target,[0.,0.,1.]),None);
    }
    #[test] fn b_enters_base_snap_only_during_modal_transform() {
        assert!(!base_snap_shortcut(Key::B,false));
        assert!(!base_snap_shortcut(Key::G,true));
        assert_eq!(BaseStage::enter(base_snap_shortcut(Key::B,false)),BaseStage::Off);
        for kind in [Kind::Move,Kind::Rotate,Kind::Scale] {
            let mut modal=Modal::new(kind,pos2(0.,0.),[0.;3],([0.;3],[1.;3]),SnapTo::Vertex,false,1.);
            modal.base_stage=BaseStage::enter(base_snap_shortcut(Key::B,true));
            assert_eq!(modal.base_stage,BaseStage::Base);
        }
    }
    #[test] fn base_snap_scale_follows_pointer_from_base() {
        let frame=[[1.,0.,0.],[0.,1.,0.],[0.,0.,1.]];
        let base=[2.,3.,4.];
        let mut modal=Modal::new(Kind::Scale,pos2(40.,50.),[0.;3],([0.;3],[1.;3]),SnapTo::Increment,false,1.);
        modal.base_stage=BaseStage::Target;
        modal.base_point=Some(base);
        assert_eq!(modal.desired(pos2(40.,50.),100.,frame,CameraInfo::default(),false,1.,false),([1.;3],base));
        modal.pointer=pos2(52.,50.);
        assert_eq!(modal.desired(pos2(40.,50.),100.,frame,CameraInfo::default(),false,1.,false),([2.;3],base));
        modal.pointer=pos2(53.,50.);
        let snapped=modal.desired(pos2(40.,50.),100.,frame,CameraInfo::default(),true,1.,false).0;
        assert!(snapped.iter().all(|factor| (*factor-2.1).abs()<1e-6),"{snapped:?}");
        modal.base_target=Some([5.,7.,4.]);
        assert_eq!(base_snap_offset(base,modal.base_target.unwrap()),[3.,4.,0.]);
    }
    #[test] fn base_snap_rotate_follows_pointer_after_base_is_picked() {
        let frame=[[1.,0.,0.],[0.,1.,0.],[0.,0.,1.]];
        let camera=CameraInfo {ray_forward:[0.,0.,1.],..Default::default()};
        let mut modal=Modal::new(Kind::Rotate,pos2(0.,0.),[0.;3],([0.;3],[1.;3]),SnapTo::Increment,false,1.);
        modal.base_stage=BaseStage::Target; modal.base_point=Some([2.,3.,4.]);
        modal.source_direction=Some([3.,3.,4.]);
        modal.pointer_direction=Some([2.,4.,4.]);
        let (values,pivot)=modal.desired(pos2(0.,0.),100.,frame,camera,false,1.,false);
        assert_eq!(pivot,[2.,3.,4.]); assert!((values[2]-90.).abs()<1e-4,"{values:?}");
        modal.pointer_direction=Some([3.,3.,4.]);
        assert_eq!(modal.desired(pos2(0.,0.),100.,frame,camera,false,1.,false).0,[0.;3]);
    }
    #[test] fn base_snap_rotate_typed_angle_wins() {
        let frame=[[1.,0.,0.],[0.,1.,0.],[0.,0.,1.]];
        let mut modal=Modal::new(Kind::Rotate,pos2(0.,0.),[0.;3],([0.;3],[1.;3]),SnapTo::Increment,false,1.);
        modal.base_stage=BaseStage::TargetDirection; modal.base_point=Some([0.;3]);
        modal.source_direction=Some([1.,0.,0.]); modal.pointer_direction=Some([0.,1.,0.]);
        modal.numeric[0]="37.5".into();
        let values=modal.desired(pos2(0.,0.),100.,frame,CameraInfo::default(),true,1.,false).0;
        assert!((values[2]-37.5).abs()<1e-4,"{values:?}");
    }
    #[test] fn base_snap_rotate_uses_picked_face_normal_and_edge_axis() {
        let frame=[[1.,0.,0.],[0.,1.,0.],[0.,0.,1.]];
        let face=crate::picking::SnapFeature {point:[0.;3],triangle:[[0.,0.,0.],[0.,1.,0.],[0.,0.,1.]],kind:SnapTo::Face};
        let mut modal=Modal::new(Kind::Rotate,pos2(0.,0.),[0.;3],([0.;3],[1.;3]),SnapTo::Increment,false,1.);
        modal.base_stage=BaseStage::TargetDirection; modal.base_point=Some([0.;3]);
        modal.base_axis=feature_rotation_axis(&face);
        assert_eq!(modal.rotation_axis(frame,CameraInfo::default()),[1.,0.,0.]);
        modal.source_direction=Some([0.,1.,0.]); modal.pointer_direction=Some([0.,0.,1.]);
        let values=modal.desired(pos2(0.,0.),100.,frame,CameraInfo::default(),false,1.,false).0;
        assert!((values[0]-90.).abs()<1e-4,"{values:?}");
        let edge=crate::picking::SnapFeature {kind:SnapTo::Edge,point:[0.,0.5,0.],..face};
        assert_eq!(feature_rotation_axis(&edge),Some([0.,1.,0.]));
    }
    #[test] fn base_snap_rotate_protractor_snaps_to_fifteen_and_five_degrees() {
        let frame=[[1.,0.,0.],[0.,1.,0.],[0.,0.,1.]];
        let mut modal=Modal::new(Kind::Rotate,pos2(0.,0.),[0.;3],([0.;3],[1.;3]),SnapTo::Increment,false,1.);
        modal.base_stage=BaseStage::TargetDirection; modal.base_point=Some([0.;3]);
        modal.source_direction=Some([1.,0.,0.]);
        let (sin,cos)=12_f32.to_radians().sin_cos(); modal.pointer_direction=Some([cos,sin,0.]);
        let camera=CameraInfo {ray_forward:[0.,0.,1.],..Default::default()};
        assert!((modal.desired(pos2(0.,0.),100.,frame,camera,true,1.,false).0[2]-15.).abs()<1e-4);
        assert!((modal.desired(pos2(0.,0.),100.,frame,camera,true,1.,true).0[2]-10.).abs()<1e-4);
        assert!((modal.desired(pos2(0.,0.),100.,frame,camera,false,1.,false).0[2]-12.).abs()<1e-4);
    }
    #[test] fn base_snap_scale_typed_value_wins() {
        let frame=[[1.,0.,0.],[0.,1.,0.],[0.,0.,1.]];
        let mut modal=Modal::new(Kind::Scale,pos2(40.,50.),[0.;3],([0.;3],[1.;3]),SnapTo::Increment,false,1.);
        modal.base_stage=BaseStage::Target;
        modal.base_point=Some([2.,3.,4.]);
        modal.pointer=pos2(52.,50.);
        modal.numeric[0]="2.75".into();
        modal.constraint=Constraint::Axis(1,false);
        assert_eq!(modal.desired(pos2(40.,50.),100.,frame,CameraInfo::default(),true,1.,false).0,[1.,2.75,1.]);
    }
    #[test] fn base_snap_move_respects_axis_constraint() {
        let frame=[[1.,0.,0.],[0.,1.,0.],[0.,0.,1.]];
        let mut modal=Modal::new(Kind::Move,pos2(0.,0.),[0.;3],([0.;3],[1.;3]),SnapTo::Increment,false,1.);
        modal.base_stage=BaseStage::Done;
        modal.base_point=Some([2.,3.,4.]);
        modal.base_target=Some([5.,7.,9.]);
        modal.constraint=Constraint::Axis(1,false);
        assert_eq!(modal.desired(pos2(0.,0.),100.,frame,CameraInfo::default(),false,1.,false).0,[0.,4.,0.]);
    }
    #[test] fn typed_move_uses_display_units_but_angles_and_scale_do_not() {
        use crate::units::{UnitSettings,LengthUnit};
        let mut m=Modal::new(Kind::Move,pos2(0.,0.),[0.;3],([0.;3],[1.;3]),SnapTo::Increment,false,1.);
        m.units=Some(UnitSettings::new(LengthUnit::Mm,1.));
        m.toggle_axis(2,false);m.numeric[0]="50".into();
        assert!((m.typed(0).unwrap()-0.05).abs()<1e-6);
        let f=crate::ui::testing::Fixture::new();
        let camera=f.view().camera;
        let (values,_)=m.desired(pos2(0.,0.),100.,[[1.,0.,0.],[0.,1.,0.],[0.,0.,1.]],camera,false,1.,false);
        assert_eq!(values[0],0.);assert_eq!(values[1],0.);assert!((values[2]-0.05).abs()<1e-6);
        m.last=values;assert!(m.status(false).contains("50 mm"));
        m.units=Some(UnitSettings::new(LengthUnit::Mm,0.01));assert_eq!(m.typed(0),Some(5.));
        m.kind=Kind::Rotate;assert_eq!(m.typed(0),Some(50.));
        m.kind=Kind::Scale;assert_eq!(m.typed(0),Some(50.));
        m.kind=Kind::Move;m.constraint=Constraint::Free;m.numeric=["10".into(),"20".into(),"30".into()];
        assert_eq!([m.typed(0),m.typed(1),m.typed(2)],[Some(1.),Some(2.),Some(3.)]);
    }
    fn perspective() -> CameraInfo {
        // Looking down -Y and tilted: right is +X, up mixes +Z and +Y, forward mostly -Y.
        let (c, s) = (0.8f32, 0.6f32);
        CameraInfo { ray_right:[1.,0.,0.], ray_up:[0.,s,c], ray_forward:[0.,-c,s], ..Default::default() }
    }
    #[test] fn g_in_perspective_moves_in_the_view_plane() {
        let camera=perspective();
        let mut m=Modal::new(Kind::Move,pos2(0.,0.),[0.;3],([0.;3],[1.;3]),SnapTo::Increment,false,1.);
        m.pointer=pos2(0.,-100.);
        let frame=[[1.,0.,0.],[0.,1.,0.],[0.,0.,1.]];
        let v=m.desired(pos2(0.,0.),100.,frame,camera,false,1.,false).0;
        assert!((v[0]).abs()<1e-6 && (v[1]-0.6).abs()<1e-6 && (v[2]-0.8).abs()<1e-6, "{v:?}");
        assert!(dot(v,camera.ray_forward).abs()<1e-6);
        m.pointer=pos2(50.,0.);
        assert_eq!(m.desired(pos2(0.,0.),100.,frame,camera,false,1.,false).0,[0.5,0.,0.]);
        m.toggle_axis(0,false);m.pointer=pos2(50.,-100.);
        assert_eq!(m.desired(pos2(0.,0.),100.,frame,camera,false,1.,false).0,[0.5,0.,0.], "X still locks");
    }
    #[test] fn axis_move_is_positive_when_the_mouse_moves_right_in_any_view() {
        let mut camera = perspective();
        for sign in [1.0_f32, -1.0] {
            camera.ray_right = [sign, 0., 0.];
            let d = rightward_axis([1., 0., 0.], camera);
            assert!(d.x > 0., "mouse right must be +X for right={sign}");
        }
    }
    #[test] fn middle_click_locks_the_axis_closest_to_the_view() {
        let camera=perspective();let frame=[[1.,0.,0.],[0.,1.,0.],[0.,0.,1.]];
        let mut m=Modal::new(Kind::Move,pos2(0.,0.),[0.;3],([0.;3],[1.;3]),SnapTo::Increment,false,1.);
        m.mmb_start=Some(pos2(10.,10.));m.middle_release(pos2(11.,10.),frame,camera);
        assert_eq!(m.constraint,Constraint::Axis(1,false));
        m.constraint=Constraint::Free;m.mmb_start=Some(pos2(10.,10.));m.middle_release(pos2(60.,10.),frame,camera);
        assert_eq!(m.constraint,Constraint::Axis(0,false), "a drag still picks the dragged axis");
    }
    #[test] fn middle_button_does_not_start_a_cage_drag() {
        let mut modal=Modal::new(Kind::Scale,pos2(0.,0.),[0.;3],([0.;3],[1.;3]),SnapTo::Increment,false,1.);
        modal.cage=true;
        modal.middle_press(pos2(10.,10.));
        assert_eq!(modal.mmb_start,None);
        assert_eq!(modal.cage_handle,None);
        assert_eq!(modal.constraint,Constraint::Free);
    }
    #[test] fn applied_cage_is_ready_for_another_adjustment() {
        let mut modal=Modal::new(Kind::Scale,pos2(10.,10.),[1.;3],([0.;3],[2.;3]),SnapTo::Increment,false,1.);
        modal.cage=true;
        modal.cage_handle=Some(CageHandle::Face(0,true));
        modal.pointer=pos2(30.,10.);
        modal.last=[1.5,1.,1.];
        let next=modal.after_cage_apply([0.,1.,1.]);
        assert!(next.cage);
        assert_eq!(next.cage_handle,None);
        assert_eq!(next.last,[1.;3]);
        assert_eq!(next.bounds,([0.,0.,0.],[3.,2.,2.]));
        assert_eq!(next.start,modal.pointer);
    }
    #[test] fn double_g_in_edge_mode_starts_edge_slide() {
        use crate::ui::SelectionMode as M;
        assert!(double_g_slides(Kind::Move,M::Edges));
        assert!(!double_g_slides(Kind::Move,M::Faces) && !double_g_slides(Kind::Move,M::Object) && !double_g_slides(Kind::Rotate,M::Edges));
    }
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
    #[test] fn geometry_snap_is_exact_and_respects_toggle_and_constraint() {
        let frame=[[1.,0.,0.],[0.,1.,0.],[0.,0.,1.]];
        let camera=CameraInfo::default();
        let mut m=Modal::new(Kind::Move,pos2(0.,0.),[0.;3],([0.;3],[1.;3]),SnapTo::Vertex,false,1.);
        m.snap_to.toggle(SnapTo::Increment);
        m.target=Some([0.13,0.27,0.41]);
        assert_eq!(m.desired(pos2(0.,0.),100.,frame,camera,true,0.25,false).0,[0.13,0.27,0.41]);
        assert_eq!(m.desired(pos2(0.,0.),100.,frame,camera,false,0.25,false).0,[0.;3]);
        m.constraint=Constraint::Axis(1,false);
        assert_eq!(m.desired(pos2(0.,0.),100.,frame,camera,true,0.25,false).0,[0.,0.27,0.]);
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

impl App {
    pub(crate) fn update_modal(&mut self, ctx: &egui::Context) {
        let Some(mut modal)=self.modal.take() else { return; };
        let input=ctx.input(|i|(i.events.clone(),i.pointer.hover_pos(),i.modifiers,i.raw_scroll_delta.y));
        let (events,pointer,modifiers,scroll)=input;
        if let Some(pointer)=pointer { modal.pointer=pointer; }
        let mut confirm=false; let mut cancel=false; let mut gg=false; let mut base_changed=false;
        let camera=self.camera_info();
        let orientation=if matches!(modal.constraint,Constraint::Axis(_,true)|Constraint::Plane(_,true))
            { ui::Orientation::Local } else { self.editor_state.orientation };
        let frame=if orientation == ui::Orientation::Cursor {
            transform_math::Frame::from_rotation(&transform_math::euler_xyz(self.editor_state.cursor_rotation.map(f64::from)))
                .axes.map(|axis| axis.map(|x| x as f32))
        } else if orientation == ui::Orientation::Normal {
            let trs = transform_math::TrsParams::from_params(&self.editor.doc.value()["nodes"][&self.display]["params"]);
            self.selection_normal_frame(&trs).axes.map(|axis| axis.map(|x| x as f32))
        } else { ui::modal_transform::orientation_frame(orientation,camera,modal.rotation) };
        for event in events {
            match event {
                egui::Event::Key { key, pressed:true, modifiers:m, .. } => {
                    if modal.type_key(key,m.shift) { continue; }
                    match key {
                        egui::Key::Escape => cancel=true,
                        egui::Key::Enter | egui::Key::Space if matches!(modal.base_stage,BaseStage::Off|BaseStage::Done)
                            || (modal.kind==ModalKind::Rotate && modal.base_point.is_some() && modal.typed(0).is_some()) => confirm=true,
                        egui::Key::G if ui::modal_transform::double_g_slides(modal.kind,self.editor_state.selection) => gg=true,
                        egui::Key::X | egui::Key::Y | egui::Key::Z => modal.toggle_axis(match key { egui::Key::X=>0,egui::Key::Y=>1,_=>2 },m.shift),
                        egui::Key::O => modal.proportional = !modal.proportional,
                        egui::Key::PageUp => modal.wheel(1.), egui::Key::PageDown => modal.wheel(-1.),
                        egui::Key::B => {
                            self.cancel_transform();
                            modal.base_stage=BaseStage::enter(true);
                            modal.base_point=None; modal.base_target=None;
                            modal.source_direction=None; modal.target_direction=None;
                            modal.base_axis=None; modal.pointer_direction=None; modal.base_angle=None;
                            modal.sketch=SketchStage::Off; modal.cage=false;
                            modal.last=if modal.kind==ModalKind::Scale {modal.last} else {[0.;3]};
                        },
                        egui::Key::R if modal.kind==ModalKind::Rotate => modal.trackball = !modal.trackball,
                        egui::Key::V if modal.kind==ModalKind::Scale => { modal.cage = !modal.cage; modal.cage_handle=None; },
                        _=>{}
                    }
                }
                egui::Event::PointerButton { pos, button:egui::PointerButton::Primary, pressed:true, .. } => {
                    if modal.base_stage!=BaseStage::Off {
                        // Point and surface features work even when incremental snapping is the current preference.
                        let geometry=ui::SnapTargets::from_bits(0b101110);
                        let feature=self.modal_snap_feature(pos,geometry);
                        let picked=feature.map(|feature|feature.point);
                        let plane_point=modal.base_point.unwrap_or(modal.pivot);
                        let point=picked.or_else(|| {
                            let (origin,direction)=ui::transform_gizmo::ray(camera,self.last_viewport,pos);
                            let normal=camera.ray_forward;
                            let denominator=(0..3).map(|i|direction[i]*normal[i]).sum::<f32>();
                            if denominator.abs()<1e-6 { return None; }
                            let t=(0..3).map(|i|(plane_point[i]-origin[i])*normal[i]).sum::<f32>()/denominator;
                            (t>0.).then(||[0,1,2].map(|i|origin[i]+direction[i]*t))
                        });
                        match modal.base_stage {
                            BaseStage::Base => if let Some(point)=picked {
                                modal.base_point=Some(point);modal.pivot=point;
                                if modal.kind==ModalKind::Rotate {
                                    modal.base_axis=feature.as_ref().and_then(feature_rotation_axis);
                                    let axis=modal.rotation_axis(frame,camera);
                                    let right=camera.ray_right;
                                    let along=dot(right,axis);
                                    let projected=[0,1,2].map(|i|right[i]-axis[i]*along);
                                    let reference=if dot(projected,projected)>1e-8 {projected} else {frame[(dominant_axis(axis)+1)%3]};
                                    modal.source_direction=Some([0,1,2].map(|i|point[i]+reference[i]));
                                }
                                if modal.kind==ModalKind::Scale {modal.start=pos;}
                                modal.base_stage=BaseStage::Target;base_changed=true;
                            },
                            BaseStage::Target => if let Some(point)=point {modal.base_target=Some(point);modal.base_stage=if modal.kind==ModalKind::Rotate {BaseStage::SourceDirection} else {BaseStage::Done};base_changed=true;confirm=modal.base_stage==BaseStage::Done;},
                            BaseStage::SourceDirection => if let Some(point)=point {
                                let offset=base_snap_offset(modal.base_point.unwrap(),modal.base_target.unwrap());
                                modal.source_direction=Some([0,1,2].map(|i|point[i]-offset[i]));
                                modal.base_stage=BaseStage::TargetDirection;
                            },
                            BaseStage::TargetDirection => if let Some(point)=point {
                                let offset=base_snap_offset(modal.base_point.unwrap(),modal.base_target.unwrap());
                                modal.target_direction=Some([0,1,2].map(|i|point[i]-offset[i]));
                                modal.base_stage=BaseStage::Done;base_changed=true;confirm=true;
                            },
                            BaseStage::Off|BaseStage::Done => confirm=true,
                        }
                    } else if modal.sketch==SketchStage::Pivot {
                        let to=if !modal.snap_to.geometry() {ui::SnapTo::Face.into()} else {modal.snap_to};
                        modal.sketch_pivot=self.modal_snap_point(pos,to).or(Some(modal.pivot));
                        modal.pivot=modal.sketch_pivot.unwrap(); modal.sketch=SketchStage::Reference;
                    } else if modal.sketch==SketchStage::Reference { modal.sketch_reference=Some(pos); modal.sketch=SketchStage::Angle; }
                    else if modal.cage && modal.cage_handle.is_none() {
                        modal.cage_handle=ui::transform_overlay::cage_hit(self.last_viewport,camera,&modal,pos);
                        modal.cage_start=pos;
                        if modal.cage_handle.is_none() { confirm=true; }
                    } else if modifiers.ctrl && modifiers.shift && modal.kind==ModalKind::Move {
                        modal.reference=self.modal_snap_point(pos,ui::SnapTo::Vertex);
                    } else { confirm=true; }
                }
                egui::Event::PointerButton { button:egui::PointerButton::Secondary, pressed:true, .. } => cancel=true,
                egui::Event::PointerButton { pos, button:egui::PointerButton::Middle, pressed:true, .. } => {
                    modal.middle_press(pos);
                }
                egui::Event::PointerButton { pos, button:egui::PointerButton::Middle, pressed:false, .. } => {
                    if !modal.cage { modal.middle_release(pos,frame,camera); }
                }
                _=>{}
            }
        }
        if let Some(start)=modal.mmb_start {
            if (modal.pointer-start).length()>4. {
                modal.constraint=Constraint::Axis(ui::modal_transform::closest_axis(modal.pointer-start,frame,camera),false);
            }
        }
        if modal.proportional && scroll!=0. { modal.wheel(scroll.signum()); }
        if gg { self.cancel_transform(); self.command("edge_slide",ctx); return; }
        if cancel {
            self.editor_state.proportional.enabled=modal.initial_proportional;
            self.editor_state.proportional.radius=modal.initial_radius;
            self.cancel_transform(); return;
        }
        self.editor_state.proportional.enabled=modal.proportional;
        self.editor_state.proportional.radius=modal.radius;
        let snap=self.editor_state.snap.enabled ^ modifiers.ctrl;
        if modal.base_stage==BaseStage::Off && modal.kind==ModalKind::Move && snap && modal.snap_to.geometry() {
            let feature=self.modal_snap_feature(modal.pointer,modal.snap_to);
            modal.target=feature.map(|f|f.point);
            modal.target_kind=feature.map(|f|f.kind);
            modal.target_shape=feature.map(|f|f.triangle);
        } else { modal.target=None; modal.target_shape=None; modal.target_kind=None; }
        if modal.kind==ModalKind::Rotate && modal.base_stage!=BaseStage::Off {
            modal.pointer_direction=modal.base_point.and_then(|base| {
                let axis=modal.rotation_axis(frame,camera);
                let offset=modal.base_target.map(|target|base_snap_offset(base,target)).unwrap_or([0.;3]);
                let plane=[0,1,2].map(|i|base[i]+offset[i]);
                let feature=self.modal_snap_feature(modal.pointer,ui::SnapTargets::from_bits(0b101110));
                let point=feature.map(|feature|feature.point).or_else(|| {
                    let (origin,direction)=ui::transform_gizmo::ray(camera,self.last_viewport,modal.pointer);
                    ray_plane_point(origin,direction,plane,axis)
                }).or_else(|| {
                    let center=ui::transform_gizmo::project(camera,self.last_viewport,plane)?;
                    let delta=modal.pointer-center;
                    let vector=[0,1,2].map(|i|camera.ray_right[i]*delta.x-camera.ray_up[i]*delta.y);
                    Some([0,1,2].map(|i|plane[i]+vector[i]*0.01))
                })?;
                Some([0,1,2].map(|i|point[i]-offset[i]))
            });
        }
        let center=ui::transform_gizmo::project(camera,self.last_viewport,modal.pivot).unwrap_or(self.last_viewport.center());
        let unit=ui::transform_gizmo::project(camera,self.last_viewport,[modal.pivot[0]+1.,modal.pivot[1],modal.pivot[2]])
            .map(|p|(p-center).length()).unwrap_or(80.).max(1.);
        let orientation=if matches!(modal.constraint,Constraint::Axis(_,true)|Constraint::Plane(_,true))
            { ui::Orientation::Local } else { self.editor_state.orientation };
        let frame=if orientation == ui::Orientation::Cursor {
            transform_math::Frame::from_rotation(&transform_math::euler_xyz(self.editor_state.cursor_rotation.map(f64::from)))
                .axes.map(|axis| axis.map(|x| x as f32))
        } else if orientation == ui::Orientation::Normal {
            let trs = transform_math::TrsParams::from_params(&self.editor.doc.value()["nodes"][&self.display]["params"]);
            self.selection_normal_frame(&trs).axes.map(|axis| axis.map(|x| x as f32))
        } else { ui::modal_transform::orientation_frame(orientation,camera,modal.rotation) };
        if modal.base_stage==BaseStage::Off && snap && modal.kind==ModalKind::Move && modal.snap_to.contains(ui::SnapTo::Increment) {
            if let Some(target)=modal.target.take() {
                let (grid,_) = modal.desired(center,unit,frame,camera,true,self.editor_state.snap.increment,modifiers.shift);
                let reference=modal.reference.unwrap_or(modal.pivot);
                let grid_point=[0,1,2].map(|k|reference[k]+grid[k]);
                let distance=|p|ui::transform_gizmo::project(camera,self.last_viewport,p).map(|p|p.distance(modal.pointer)).unwrap_or(f32::INFINITY);
                if distance(target) <= distance(grid_point) { modal.target=Some(target); }
                else { modal.target_shape=None; modal.target_kind=None; }
            }
        }
        let (values,pivot)=modal.desired(center,unit,frame,camera,snap,self.editor_state.snap.increment,modifiers.shift);
        if modal.kind==ModalKind::Rotate && modal.base_stage!=BaseStage::Off {
            let axis=modal.rotation_axis(frame,camera);
            let target=if modal.base_stage==BaseStage::Done {modal.target_direction} else {modal.pointer_direction};
            modal.base_angle=Some(modal.typed(0).unwrap_or_else(|| {
                let raw=modal.base_point.zip(modal.source_direction).zip(target)
                    .and_then(|((base,source),target)|rotate_alignment_angle(base,source,target,axis)).unwrap_or(0.);
                if snap {snap_value(raw,if modifiers.shift {5.} else {15.})} else {raw}
            }));
        }
        let pivot_changed=self.transform_target.as_ref().is_some_and(|target|
            self.editor.doc.value()["nodes"][target]["params"]["pivot"]!=json!(pivot));
        if values!=modal.last || pivot_changed || base_changed {
            let op=modal.step(values);
            let offset=if modal.base_stage!=BaseStage::Off && modal.kind!=ModalKind::Move {
                modal.base_point.zip(modal.base_target).map(|(base,target)|base_snap_offset(base,target))
            } else {None};
            let orientation=if modal.kind==ModalKind::Rotate && modal.base_stage!=BaseStage::Off {
                ui::Orientation::Global
            } else {orientation};
            self.transform_node_at_with_offset(op,Some(pivot),Some(orientation),offset);
        }
        if confirm {
            let next_cage = modal.cage.then(|| modal.after_cage_apply(pivot));
            self.finish_modal(modal);
            if let Some(next_cage) = next_cage { self.modal=Some(next_cage); ctx.request_repaint(); }
        } else { self.modal=Some(modal); ctx.request_repaint(); }
    }
}
