//! Screen-space sketch inferences. No UI state or document mutation lives here.
use eframe::egui::{Pos2, Vec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind { Endpoint, Midpoint, Edge, Face, Axis(usize), Parallel, Perpendicular, FromPoint }
impl Kind {
    pub fn label(self) -> &'static str { match self { Self::Endpoint => "Endpoint", Self::Midpoint => "Midpoint", Self::Edge => "On Edge", Self::Face => "On Face", Self::Axis(0) => "On Red Axis", Self::Axis(1) => "On Green Axis", Self::Axis(_) => "On Blue Axis", Self::Parallel => "Parallel", Self::Perpendicular => "Perpendicular", Self::FromPoint => "From Point" } }
    fn priority(self) -> u8 { match self { Self::Endpoint => 0, Self::Midpoint => 1, Self::Edge => 2, Self::Parallel | Self::Perpendicular => 3, Self::FromPoint => 4, Self::Axis(_) => 5, Self::Face => 6 } }
}
#[derive(Clone, Debug)]
pub struct Hit { pub point: [f32; 3], pub kind: Kind, pub indices: Vec<usize>, pub weights: Vec<f32>, pub distance: f32, pub depth: f32 }
pub fn choose(best: &mut Option<Hit>, hit: Hit, radius: f32) {
    if hit.distance > radius || hit.depth <= 0. || !hit.distance.is_finite() { return; }
    let replace = best.as_ref().is_none_or(|old| hit.kind.priority() < old.kind.priority()
        || (hit.kind.priority() == old.kind.priority() && (hit.distance < old.distance - 0.01
        || ((hit.distance-old.distance).abs() <= 0.01 && hit.depth < old.depth))));
    if replace { *best = Some(hit); }
}
pub fn infer_mesh(positions: &[f32], triangles: &[u32], pointer: Pos2,
    project: impl Fn([f32; 3]) -> Option<(Pos2, f32)>) -> Option<Hit> {
    let mut best = None;
    let at = |id: usize| [positions[id*3], positions[id*3+1], positions[id*3+2]];
    let mut consider = |point, kind, indices: Vec<usize>, weights: Vec<f32>| {
        if let Some((screen,depth)) = project(point) {
            choose(&mut best, Hit { point, kind, indices, weights, distance: screen.distance(pointer), depth }, 12.);
        }
    };
    for ids in triangles.chunks_exact(3) {
        if ids.iter().any(|&i| i as usize >= positions.len()/3) { continue; }
        let ids = [ids[0] as usize, ids[1] as usize, ids[2] as usize];
        let points = ids.map(at);
        for i in 0..3 { consider(points[i], Kind::Endpoint, vec![ids[i]], vec![1.]); }
        for (a,b) in [(0,1),(1,2),(2,0)] {
            let mid = [0,1,2].map(|k| (points[a][k]+points[b][k])*0.5);
            consider(mid, Kind::Midpoint, vec![ids[a],ids[b]], vec![0.5,0.5]);
            if let (Some((sa,_)),Some((sb,_))) = (project(points[a]),project(points[b])) {
                let d=sb-sa; let t=((pointer-sa).dot(d)/d.length_sq().max(1e-8)).clamp(0.,1.);
                let edge=[0,1,2].map(|k|points[a][k]*(1.-t)+points[b][k]*t);
                consider(edge,Kind::Edge,vec![ids[a],ids[b]],vec![1.-t,t]);
            }
        }
        if let (Some((a,_)),Some((b,_)),Some((c,_)))=(project(points[0]),project(points[1]),project(points[2])) {
            let cross=|u:Vec2,v:Vec2|u.x*v.y-u.y*v.x;
            let area=cross(b-a,c-a);
            if area.abs()>1e-7 {
                let w1=cross(pointer-a,c-a)/area;
                let w2=cross(b-a,pointer-a)/area;
                let w0=1.-w1-w2;
                if [w0,w1,w2].iter().all(|w|*w>=-1e-5) {
                    let ws=[w0,w1,w2];
                    let point=[0,1,2].map(|k|(0..3).map(|i|points[i][k]*ws[i]).sum());
                    consider(point,Kind::Face,ids.to_vec(),ws.to_vec());
                }
            }
        }
    }
    best
}
pub fn infer_direction(pointer: Pos2, anchor: [f32;3], raw: [f32;3], edge: ([f32;3],[f32;3]),
    project: impl Fn([f32;3])->Option<(Pos2,f32)>) -> Option<Hit> {
    let mut best=None;
    let d=[0,1,2].map(|k|edge.1[k]-edge.0[k]);
    let perpendicular=[-d[1],d[0],d[2]];
    for (kind,dir) in [(Kind::Parallel,d),(Kind::Perpendicular,perpendicular)] {
        let denom=dir.iter().map(|x|x*x).sum::<f32>();
        if denom<1e-9 { continue; }
        let t=(0..3).map(|k|(raw[k]-anchor[k])*dir[k]).sum::<f32>()/denom;
        let point=[0,1,2].map(|k|anchor[k]+dir[k]*t);
        if let Some((screen,depth))=project(point) {
            choose(&mut best,Hit {point,kind,indices:vec![],weights:vec![],distance:screen.distance(pointer),depth},8.);
        }
    }
    best
}
pub fn infer_axis(pointer: Pos2, anchor: [f32;3], raw: [f32;3],
    project: impl Fn([f32;3]) -> Option<(Pos2,f32)>) -> Option<Hit> {
    let mut best=None;
    for axis in 0..3 {
        let point=[0,1,2].map(|i|if i==axis {raw[i]} else {anchor[i]});
        if let Some((screen,depth))=project(point) {
            choose(&mut best,Hit {point,kind:Kind::Axis(axis),indices:vec![],weights:vec![],distance:screen.distance(pointer),depth},8.);
        }
    }
    best
}
#[cfg(test)] mod tests {
    use super::*;
    #[test] fn endpoint_priority_and_radius() {
        let positions=[0.,0.,0.,10.,0.,0.,0.,10.,0.];
        let project=|p:[f32;3]|Some((Pos2::new(p[0],p[1]),1.));
        let hit=infer_mesh(&positions,&[0,1,2],Pos2::new(0.2,0.2),project).unwrap();
        assert_eq!(hit.kind,Kind::Endpoint); assert_eq!(hit.indices,vec![0]);
        assert!(infer_mesh(&positions,&[0,1,2],Pos2::new(100.,100.),project).is_none());
    }
    #[test] fn axis_candidate() {
        let hit=infer_axis(Pos2::new(3.,0.),[0.;3],[3.,2.,0.],|p|Some((Pos2::new(p[0],p[1]),1.))).unwrap();
        assert_eq!(hit.kind,Kind::Axis(0)); assert_eq!(hit.point,[3.,0.,0.]);
    }
}
