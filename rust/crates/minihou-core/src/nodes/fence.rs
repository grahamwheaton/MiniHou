//! Garden fence along the first open curve. Z is up; each bay follows the path tangent.
use super::{InputSpec, Inputs, NodeSpec};
use crate::{error::{Error, Result}, geometry::Geometry, params::Params};

pub const FENCE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None,
    name: "fence", inputs: &[InputSpec { name: "path", multi: false, optional: false }],
    params: crate::params!("bay_width": 2.0; "height": 1.8; "rail_height": 0.85;
        "slat_width": 0.10; "slat_gap": 0.05; "batten_thickness": 0.07; "post_size": 0.12),
    cook: fence,
};

struct Mesh { p: Vec<f32>, verts: Vec<i32>, offsets: Vec<i32> }
impl Mesh {
    fn new() -> Self { Self { p: vec![], verts: vec![], offsets: vec![0] } }
    fn box_between(&mut self, a: [f64;3], b: [f64;3], width: f64, depth: f64, sideways: [f64;3]) {
        let axis = [b[0]-a[0],b[1]-a[1],b[2]-a[2]];
        let len = axis.iter().map(|v|v*v).sum::<f64>().sqrt();
        if len < 1e-8 { return; }
        let u = axis.map(|v|v/len);
        // Width axis lies in the fence plane; depth points out of it.
        let w = [sideways[1]*u[2]-sideways[2]*u[1], sideways[2]*u[0]-sideways[0]*u[2], sideways[0]*u[1]-sideways[1]*u[0]];
        let base = (self.p.len()/3) as i32;
        for end in [a,b] { for (s,t) in [(-1.,-1.),(1.,-1.),(1.,1.),(-1.,1.)] {
            for k in 0..3 { self.p.push((end[k]+s*width*0.5*w[k]+t*depth*0.5*sideways[k]) as f32); }
        }}
        for q in [[0,3,2,1],[4,5,6,7],[0,1,5,4],[1,2,6,5],[2,3,7,6],[3,0,4,7]] {
            self.verts.extend(q.map(|i|base+i)); self.offsets.push(self.verts.len() as i32);
        }
    }
}

fn fence(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let path = FENCE.one(inputs,0)?;
    let [bay,height,rail,slat,gap,batten,post] = ["bay_width","height","rail_height","slat_width","slat_gap","batten_thickness","post_size"].map(|n|p.f(n)).into_iter().collect::<Result<Vec<_>>>()?.try_into().unwrap();
    if [bay,height,rail,slat,batten,post].iter().any(|v|!v.is_finite() || *v<=0.) || !gap.is_finite() || gap<0. || rail>=height || post>=bay {
        return Err(Error::new("fence dimensions must be positive and finite; rail_height < height and post_size < bay_width"));
    }
    let topo=&path.topology;
    let curve=(0..path.nprims()).find(|&i|!topo.is_closed(i) && topo.prim_offsets[i+1]-topo.prim_offsets[i]>=2)
        .ok_or_else(||Error::new("fence needs an open curve with at least two points"))?;
    let ids=&topo.vertex_point[topo.prim_offsets[curve] as usize..topo.prim_offsets[curve+1] as usize];
    let points:Vec<[f64;3]>=ids.iter().map(|&i| {let q=&path.p()[i as usize*3..i as usize*3+3];[q[0] as f64,q[1] as f64,q[2] as f64]}).collect();
    if points.iter().flatten().any(|v|!v.is_finite()) {return Err(Error::new("fence path has non-finite positions"));}
    let mut lengths=vec![0.]; for pair in points.windows(2) {let d=(0..3).map(|k|(pair[1][k]-pair[0][k]).powi(2)).sum::<f64>().sqrt(); lengths.push(lengths.last().unwrap()+d);}
    let total=*lengths.last().unwrap(); if total<=1e-8 {return Err(Error::new("fence path has zero length"));}
    let bays=(total/bay).ceil() as usize; if bays==0 || bays>10_000 {return Err(Error::new("fence exceeds 10000 bays"));}
    let sample=|dist:f64| -> [f64;3] {let d=dist.clamp(0.,total); let j=lengths.partition_point(|x|*x<d).clamp(1,points.len()-1); let span=lengths[j]-lengths[j-1]; let t=if span>1e-12 {(d-lengths[j-1])/span} else {0.}; std::array::from_fn(|k|points[j-1][k]*(1.-t)+points[j][k]*t) };
    let mut mesh=Mesh::new(); let step=total/bays as f64; let mut slats=0usize;
    for i in 0..=bays {let q=sample(i as f64*step); mesh.box_between(q,[q[0],q[1],q[2]+height+post*0.5],post,post,[0.,1.,0.]);}
    for i in 0..bays {
        let a=sample(i as f64*step);let b=sample((i+1) as f64*step);
        let dx=b[0]-a[0];let dy=b[1]-a[1];let horizontal=(dx*dx+dy*dy).sqrt(); if horizontal<1e-8 {continue;}
        let tangent=[dx/horizontal,dy/horizontal,0.];let side=[-tangent[1],tangent[0],0.];
        let inside=horizontal-post; if inside<=0. {continue;}
        let at=|fraction:f64,z:f64| -> [f64;3] {[a[0]+dx*fraction,a[1]+dy*fraction,a[2]+(b[2]-a[2])*fraction+z]};
        let left=post/(2.*horizontal);let right=1.-left;
        for z in [post*0.5,rail,height-post*0.5] {mesh.box_between(at(left,z),at(right,z),post*0.65,post*0.7,side);}
        let n=((inside+gap)/(slat+gap)).floor().max(0.) as usize;
        if n>10_000 {return Err(Error::new("fence exceeds 10000 slats per bay"));}
        slats+=n;
        for j in 0..n {let offset=(inside-(n as f64*slat+(n.saturating_sub(1)) as f64*gap))*0.5+slat*(j as f64+0.5)+gap*j as f64;
            let f=left+offset/horizontal; mesh.box_between(at(f,post),at(f,rail-post*0.5),slat,post*0.4,side);}
        let low=rail+post*0.5;let high=height-post;
        if high>low {for (x,y) in [(low,high),(high,low)] {mesh.box_between(at(left,x),at(right,y),batten,batten,side);}}
    }
    let mut out=Geometry::from_arrays(mesh.p,mesh.verts,mesh.offsets,None)?;
    out.detail.insert("fence_bays".into(),serde_json::json!(bays));
    out.detail.insert("fence_posts".into(),serde_json::json!(bays+1));
    out.detail.insert("fence_slats".into(),serde_json::json!(slats));
    Ok(out)
}

#[cfg(test)] mod tests {
    use super::*;
    use std::sync::Arc;
    use serde_json::json;
    #[test] fn fence_counts_and_bounds() {
        let curve=Geometry::from_arrays(vec![0.,0.,0.,6.,0.,0.],vec![0,1],vec![0,2],Some(vec![false])).unwrap();
        let given=json!({"bay_width":2.0});
        let params=Params::resolve(FENCE.params,given.as_object()).unwrap();
        let g=fence(&[vec![Arc::new(curve)]],&params).unwrap();
        // 4 posts, 3 bays, 12 slats per bay, 9 rails and 6 diagonal battens.
        assert_eq!(g.npoints/8,4+36+9+6);
        assert!(g.p().iter().all(|v|v.is_finite()));
        let max_x=g.p().iter().skip(0).step_by(3).copied().fold(f32::NEG_INFINITY,f32::max);
        assert!(max_x>5.9 && max_x<6.2);
    }
}
