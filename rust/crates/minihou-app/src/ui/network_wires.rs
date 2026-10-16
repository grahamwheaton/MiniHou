//! Screen-space noodle geometry shared by painting, picking, marquee and slicing.
use eframe::egui::{vec2, Pos2, Rect};
use std::collections::BTreeSet;
use crate::graph_edit::WireRef;

pub fn distance(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let d = b - a;
    let t = if d.length_sq() > 0.0 { ((p-a).dot(d) / d.length_sq()).clamp(0.0, 1.0) } else { 0.0 };
    p.distance(a + d*t)
}

pub fn segments_cross(a: Pos2, b: Pos2, c: Pos2, d: Pos2) -> bool {
    let cross = |u: eframe::egui::Vec2, v: eframe::egui::Vec2| u.x*v.y-u.y*v.x;
    let ab = b-a;
    let cd = d-c;
    let den = cross(ab, cd);
    if den.abs() < 1e-6 {
        return distance(a,c,d) < 1e-4 || distance(b,c,d) < 1e-4
            || distance(c,a,b) < 1e-4 || distance(d,a,b) < 1e-4;
    }
    let t = cross(c-a, cd)/den;
    let u = cross(c-a, ab)/den;
    (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u)
}

/// Flatten adaptively to subpixel precision, including backward/looping noodles.
pub fn curve(a: Pos2, b: Pos2) -> Vec<Pos2> {
    fn flatten(p: [Pos2;4], depth: u8, out: &mut Vec<Pos2>) {
        if depth == 12 || (distance(p[1],p[0],p[3]) <= 0.25 && distance(p[2],p[0],p[3]) <= 0.25) {
            out.push(p[3]); return;
        }
        let ab=p[0].lerp(p[1],0.5); let bc=p[1].lerp(p[2],0.5); let cd=p[2].lerp(p[3],0.5);
        let abc=ab.lerp(bc,0.5); let bcd=bc.lerp(cd,0.5); let mid=abc.lerp(bcd,0.5);
        flatten([p[0],ab,abc,mid],depth+1,out);
        flatten([mid,bcd,cd,p[3]],depth+1,out);
    }
    let dy=((b.y-a.y).abs()*0.5).max(24.0);
    let mut out=vec![a];
    flatten([a,a+vec2(0.0,dy),b-vec2(0.0,dy),b],0,&mut out);
    out
}
pub fn crosses(points: &[Pos2], a: Pos2, b: Pos2) -> bool {
    points.windows(2).any(|w| segments_cross(a,b,w[0],w[1]))
}
pub fn in_box(points: &[Pos2], r: Rect) -> bool {
    points.iter().any(|p| r.contains(*p)) || points.windows(2).any(|w| {
        let corners=[r.left_top(),r.right_top(),r.right_bottom(),r.left_bottom(),r.left_top()];
        corners.windows(2).any(|e| segments_cross(w[0],w[1],e[0],e[1]))
    })
}
/// Distance to the node centre, only when the noodle intersects its rectangle.
pub fn node_hit(points: &[Pos2], rect: Rect) -> Option<f32> {
    in_box(points, rect).then(|| points.windows(2)
        .map(|w| distance(rect.center(), w[0], w[1])).fold(f32::INFINITY, f32::min))
}

#[derive(Clone, Copy, Default)]
pub enum SelectionOp { #[default] Replace, Add, Subtract }
impl SelectionOp {
    pub fn modifiers(shift: bool, ctrl: bool) -> Self {
        if ctrl { Self::Subtract } else if shift { Self::Add } else { Self::Replace }
    }
}
pub fn select<T: Ord>(set: &mut BTreeSet<T>, hits: impl IntoIterator<Item=T>, op: SelectionOp) {
    match op {
        SelectionOp::Replace => *set=hits.into_iter().collect(),
        SelectionOp::Add => set.extend(hits),
        SelectionOp::Subtract => for hit in hits { set.remove(&hit); },
    }
}
#[derive(Clone, Default)]
pub struct Cut { pub points: Vec<Pos2>, pub hits: BTreeSet<WireRef>, pub active: bool }
impl Cut {
    pub fn new(at: Pos2) -> Self { Self { points: vec![at], ..Default::default() } }
    pub fn update(&mut self, at: Pos2, wires: &[(WireRef, Vec<Pos2>)]) {
        let prev=*self.points.last().unwrap();
        if !self.active && self.points[0].distance(at) < 5.0 { return; }
        self.active=true;
        for (wire, points) in wires {
            if crosses(points,prev,at) { self.hits.insert(wire.clone()); }
        }
        if prev != at { self.points.push(at); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::pos2;
    #[test] fn noodle_segment_vs_bezier() {
        let points=curve(pos2(0.,0.),pos2(100.,100.));
        assert!(crosses(&points,pos2(40.,50.),pos2(60.,50.)));
        assert!(!crosses(&points,pos2(110.,50.),pos2(120.,50.)));
        assert!(segments_cross(pos2(0.,0.),pos2(10.,0.),pos2(5.,0.),pos2(15.,0.)));
        assert!(!segments_cross(pos2(0.,0.),pos2(10.,0.),pos2(11.,0.),pos2(15.,0.)));
        assert!(segments_cross(pos2(0.,0.),pos2(10.,0.),pos2(10.,0.),pos2(10.,10.)));
    }
    #[test] fn noodle_box_intersects_curve_without_endpoints() {
        let points=curve(pos2(0.,0.),pos2(100.,100.));
        assert!(in_box(&points,Rect::from_min_max(pos2(45.,45.),pos2(55.,55.))));
        assert!(!in_box(&points,Rect::from_min_max(pos2(0.,90.),pos2(10.,100.))));
        // A backwards noodle bows outside the endpoint rectangle.
        let points=curve(pos2(0.,100.),pos2(100.,0.));
        let tip=points.iter().max_by(|a,b| a.y.total_cmp(&b.y)).unwrap();
        assert!(tip.y>100.);
        assert!(in_box(&points,Rect::from_center_size(*tip,vec2(2.,2.))));
        assert!(in_box(&[pos2(-10.,0.),pos2(10.,0.)],Rect::from_center_size(Pos2::ZERO,vec2(2.,2.))));
    }
    #[test] fn noodle_node_rect_distance_and_backward_curve() {
        let rect = Rect::from_center_size(pos2(0.,0.),vec2(20.,10.));
        assert_eq!(node_hit(&[pos2(-30.,0.),pos2(30.,0.)],rect),Some(0.));
        assert_eq!(node_hit(&[pos2(-30.,5.),pos2(30.,5.)],rect),Some(5.));
        assert!(node_hit(&[pos2(-30.,6.),pos2(30.,6.)],rect).is_none());
        let points = curve(pos2(0.,100.),pos2(100.,0.));
        let tip = *points.iter().max_by(|a,b| a.y.total_cmp(&b.y)).unwrap();
        assert!(node_hit(&points,Rect::from_center_size(tip,vec2(4.,4.))).unwrap() < 0.01);
    }
    #[test] fn noodle_selection_sets() {
        let mut set=BTreeSet::from([1,2]);
        select(&mut set,[2,3],SelectionOp::Add); assert_eq!(set,BTreeSet::from([1,2,3]));
        select(&mut set,[2,4],SelectionOp::Subtract); assert_eq!(set,BTreeSet::from([1,3]));
        select(&mut set,[4],SelectionOp::Replace); assert_eq!(set,BTreeSet::from([4]));
        select(&mut set,[],SelectionOp::Replace); assert!(set.is_empty());
    }
    #[test] fn noodle_cut_threshold_and_freehand_union() {
        let wire=WireRef { from:"a".into(),to:"b".into(),port:"geo".into() };
        let wires=vec![(wire.clone(),vec![pos2(2.,-5.),pos2(2.,5.)])];
        let mut cut=Cut::new(Pos2::ZERO);
        cut.update(pos2(3.,0.),&wires); assert!(!cut.active); assert!(cut.hits.is_empty());
        cut.update(pos2(10.,0.),&wires); assert!(cut.hits.contains(&wire));
        cut.update(pos2(0.,10.),&wires); assert_eq!(cut.hits.len(),1);
    }
}
