//! Houdini-style surface Scatter and Align. Local +Z follows the sampled surface normal.
use std::sync::Arc;
use super::{np_random::Rng, Inputs, NodeSpec, GEO};
use crate::{error::{Error, Result}, geometry::{Attr, AttrData, Geometry}, params::Params};

pub const SCATTERALIGN: NodeSpec = NodeSpec { name: "scatteralign", inputs: GEO, params: crate::params!(
    "mode":"surface"; "group":""; "tag_name":"$OS"; "global_seed":0; "coverage":1.0;
    "point_count_method":"by_size"; "point_count":100; "scaling_mode":"uniform";
    "min_radius":0.1; "max_radius":0.1; "uniform_scale":1.0;
    "alignment":"normal"; "rotation":true; "max_random_cone_angle":0.0;
    "rotation_around_normal":true; "min_angle":0.0; "max_angle":360.0;
    "round_to_multiple":0.0; "offset_angle":0.0; "remap_using_distribution_ramp":false;
    "distribution_ramp_start":0.0; "distribution_ramp_end":1.0;
    "copy_point_attributes":true; "point_attributes":"*";
), cook };

fn dot(a:[f64;3],b:[f64;3])->f64{a.iter().zip(b).map(|(x,y)|x*y).sum()}
fn cross(a:[f64;3],b:[f64;3])->[f64;3]{[a[1]*b[2]-a[2]*b[1],a[2]*b[0]-a[0]*b[2],a[0]*b[1]-a[1]*b[0]]}
fn unit(v:[f64;3])->[f64;3]{let l=dot(v,v).sqrt();if l<1e-20{[0.,0.,1.]}else{v.map(|x|x/l)}}
fn sub(a:[f64;3],b:[f64;3])->[f64;3]{std::array::from_fn(|i|a[i]-b[i])}
fn quat_from_axes(x:[f64;3],y:[f64;3],z:[f64;3])->[f32;4]{
    let m=[[x[0],y[0],z[0]],[x[1],y[1],z[1]],[x[2],y[2],z[2]]];
    let (qx,qy,qz,qw)=if m[0][0]+m[1][1]+m[2][2]>0. {
        let s=(1.+m[0][0]+m[1][1]+m[2][2]).sqrt()*2.;((m[2][1]-m[1][2])/s,(m[0][2]-m[2][0])/s,(m[1][0]-m[0][1])/s,s/4.)
    }else if m[0][0]>m[1][1] && m[0][0]>m[2][2]{let s=(1.+m[0][0]-m[1][1]-m[2][2]).sqrt()*2.;(s/4.,(m[0][1]+m[1][0])/s,(m[0][2]+m[2][0])/s,(m[2][1]-m[1][2])/s)}
    else if m[1][1]>m[2][2]{let s=(1.+m[1][1]-m[0][0]-m[2][2]).sqrt()*2.;((m[0][1]+m[1][0])/s,s/4.,(m[1][2]+m[2][1])/s,(m[0][2]-m[2][0])/s)}
    else{let s=(1.+m[2][2]-m[0][0]-m[1][1]).sqrt()*2.;((m[0][2]+m[2][0])/s,(m[1][2]+m[2][1])/s,s/4.,(m[1][0]-m[0][1])/s)};
    [qx as f32,qy as f32,qz as f32,qw as f32]
}
struct Tri { ids:[usize;3], a:[f64;3], e1:[f64;3], e2:[f64;3], n:[f64;3], prim:usize, area:f64 }
fn cook(inputs:&Inputs,p:&Params)->Result<Geometry>{
    let geo=SCATTERALIGN.one(inputs,0)?;
    let mode=p.e("mode", &["surface","volume"])?;
    let coverage=p.f("coverage")?;
    let min_radius=p.f("min_radius")?;let max_radius=p.f("max_radius")?;let scale=p.f("uniform_scale")?;
    if !coverage.is_finite()||coverage<0.||![min_radius,max_radius,scale].iter().all(|x|x.is_finite()&&*x>=0.)||max_radius<min_radius{return Err(Error::new("scatteralign: coverage, radii and scale must be finite and non-negative; max radius >= min radius"));}
    let method=p.e("point_count_method", &["by_size","fixed"])?;
    let scaling=p.e("scaling_mode", &["uniform","random"])?;
    let count=p.i("point_count")?;if count<0{return Err(Error::new("scatteralign: point count must be non-negative"));}
    let seed=p.i("global_seed")?;
    let tag=p.s("tag_name")?;let tag=if tag=="$OS"{"scatteralign".to_string()}else{tag};
    if !tag.is_empty(){super::group::valid_name(&tag)?;if ["P","N","orient","pscale","primnum"].contains(&tag.as_str()){return Err(Error::new("scatteralign: tag name conflicts with a generated attribute"));}}
    let empty=||Geometry::points_only(Vec::new());
    if geo.nprims()==0||coverage==0.{return empty();}
    if mode=="volume" {let n=((count as f64)*coverage).round() as usize;return super::scatter::scatter_volume(&geo,n,seed);}
    let mask=super::group::mask(&geo,"prim",&p.s("group")?)?;
    let mut tris=Vec::new();let mut total=0.;let pos=geo.p();let vp=&geo.topology.vertex_point;let off=&geo.topology.prim_offsets;
    let point=|id:usize|->[f64;3]{std::array::from_fn(|k|pos[id*3+k] as f64)};
    for prim in 0..geo.nprims(){if !mask[prim]||!geo.topology.is_closed(prim){continue;}
        let start=off[prim] as usize;let end=off[prim+1] as usize;
        for j in start+1..end.saturating_sub(1){let ids=[vp[start] as usize,vp[j] as usize,vp[j+1] as usize];
            let a=point(ids[0]);let e1=sub(point(ids[1]),a);let e2=sub(point(ids[2]),a);let c=cross(e1,e2);let area=dot(c,c).sqrt()*0.5;
            if area>1e-20 && area.is_finite(){total+=area;tris.push(Tri{ids,a,e1,e2,n:unit(c),prim,area:total});}
        }
    }
    if tris.is_empty(){return empty();}
    let radius=(min_radius+max_radius)*0.5;
    let base=if method=="by_size"{if radius==0.{return Err(Error::new("scatteralign: By Size needs a positive radius"));}total/(std::f64::consts::PI*radius*radius)}else{count as f64};
    let n=(base*coverage).round().clamp(0.,1_000_000.) as usize;
    if n==0{return empty();}
    let alignment=p.e("alignment", &["normal","world_up"])?;
    let cone=p.f("max_random_cone_angle")?;let lo=p.f("min_angle")?;let hi=p.f("max_angle")?;let round=p.f("round_to_multiple")?;let offset=p.f("offset_angle")?;
    if ![cone,lo,hi,round,offset].iter().all(|x|x.is_finite())||cone<0.||cone>180.||hi<lo||round<0.{return Err(Error::new("scatteralign: invalid rotation angles"));}
    let ramp_start=p.f("distribution_ramp_start")?;let ramp_end=p.f("distribution_ramp_end")?;
    if ![ramp_start,ramp_end].iter().all(|x|x.is_finite()&&*x>=0.&&*x<=1.){return Err(Error::new("scatteralign: ramp endpoints must be in [0, 1]"));}
    let remap=p.b("remap_using_distribution_ramp")?;let use_rotation=p.b("rotation")?;let spin=p.b("rotation_around_normal")?;
    let mut rng=Rng::new(&[(seed as u64)&0xffffffff,0x5ca77e]);
    let mut variation=Rng::new(&[(seed as u64)&0xffffffff,0x0a11a1]);
    let mut positions=Vec::with_capacity(n*3);let mut normals=Vec::with_capacity(n*3);let mut orientations=Vec::with_capacity(n*4);let mut scales=Vec::with_capacity(n);let mut primnums=Vec::with_capacity(n);
    let mut samples=Vec::with_capacity(n);
    for _ in 0..n {let target=rng.random()*total;let ix=tris.partition_point(|t|t.area<=target).min(tris.len()-1);let t=&tris[ix];
        let mut u=rng.random();let mut v=rng.random();if u+v>1.{u=1.-u;v=1.-v;}let w=[1.-u-v,u,v];
        let xyz:[f64;3]=std::array::from_fn(|k|t.a[k]+u*t.e1[k]+v*t.e2[k]);positions.extend(xyz.map(|x|x as f32));normals.extend(t.n.map(|x|x as f32));primnums.push(t.prim as i32);samples.push((t.ids,w));
        let mut z=if alignment=="world_up"{[0.,1.,0.]}else{t.n};
        if use_rotation&&cone>0.{let theta=(1.-variation.random()*(1.-cone.to_radians().cos())).acos();let phi=variation.random()*std::f64::consts::TAU;
            let helper=if z[1].abs()<0.9{[0.,1.,0.]}else{[1.,0.,0.]};let x=unit(cross(helper,z));let y=cross(z,x);
            z=unit(std::array::from_fn(|k|z[k]*theta.cos()+theta.sin()*(x[k]*phi.cos()+y[k]*phi.sin())));
        }
        let helper=if z[1].abs()<0.9{[0.,1.,0.]}else{[1.,0.,0.]};let mut x=unit(cross(helper,z));let mut y=cross(z,x);
        if spin {let mut v=variation.random();if remap{v=ramp_start+(ramp_end-ramp_start)*v;}
            let mut degrees=lo+(hi-lo)*v;if round>0.{degrees=(degrees/round).round()*round;}let a=(degrees+offset).to_radians();let (s,c)=a.sin_cos();
            let xx=std::array::from_fn(|k|x[k]*c+y[k]*s);y=std::array::from_fn(|k|y[k]*c-x[k]*s);x=xx;
        }
        orientations.extend(quat_from_axes(x,y,z));
        let r=if scaling=="random"{min_radius+(max_radius-min_radius)*variation.random()}else{radius};scales.push((r*scale) as f32);
    }
    let mut out=Geometry::points_only(positions)?;
    out.point.insert("N".into(),Arc::new(Attr::f32(3,normals)));out.point.insert("orient".into(),Arc::new(Attr::f32(4,orientations)));
    out.point.insert("pscale".into(),Arc::new(Attr::f32(1,scales)));out.point.insert("primnum".into(),Arc::new(Attr::i32(1,primnums)));
    if !tag.is_empty(){out.point.insert(tag,Arc::new(Attr::i32(1,vec![1;n])));}
    if p.b("copy_point_attributes")? {let names=p.s("point_attributes")?;for (name,a) in &geo.point{
        if name=="P"||out.point.contains_key(name)||(!names.split_whitespace().any(|v|v=="*"||v==name)){continue;}
        let attr=match &a.data{AttrData::F32(src)=>{let mut dst=Vec::with_capacity(n*a.width);for (ids,w) in &samples{for k in 0..a.width{dst.push(ids.iter().enumerate().map(|(j,&id)|src[id*a.width+k] as f64*w[j]).sum::<f64>() as f32);}}Attr::f32(a.width,dst)},
            AttrData::I32(src)=>{let mut dst=Vec::with_capacity(n*a.width);for (ids,w) in &samples{let id=ids[w.iter().position(|v|*v==w.iter().copied().fold(0.,f64::max)).unwrap_or(0)];dst.extend_from_slice(&src[id*a.width..(id+1)*a.width]);}Attr::i32(a.width,dst)}};
        out.point.insert(name.clone(),Arc::new(attr));
    }}
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn plane() -> Geometry {
        Geometry::from_arrays(vec![0.,0.,0., 2.,0.,0., 2.,2.,0., 0.,2.,0.],vec![0,1,2,3],vec![0,4],None).unwrap()
    }
    fn run(g:Geometry, settings:serde_json::Value)->Geometry {
        let p=Params::resolve(SCATTERALIGN.params,settings.as_object()).unwrap();
        cook(&[vec![Arc::new(g)]],&p).unwrap()
    }
    #[test] fn deterministic_coverage_surface_and_normal() {
        let settings=serde_json::json!({"point_count_method":"fixed","point_count":80,"global_seed":42,"rotation_around_normal":false});
        let a=run(plane(),settings.clone());let b=run(plane(),settings);
        assert_eq!(a.p(),b.p());assert_eq!(a.point["orient"].data,b.point["orient"].data);
        assert_eq!(a.npoints,80);
        let half=run(plane(),serde_json::json!({"point_count_method":"fixed","point_count":80,"coverage":0.5}));assert_eq!(half.npoints,40);
        for xyz in a.p().chunks_exact(3){assert!(xyz[0]>=0.&&xyz[0]<=2.&&xyz[1]>=0.&&xyz[1]<=2.);assert_eq!(xyz[2],0.);}
        for normal in a.point["N"].as_f32().unwrap().chunks_exact(3){assert_eq!(normal,&[0.,0.,1.]);}
        for q in a.point["orient"].as_f32().unwrap().chunks_exact(4){
            let z=[2.*(q[0]*q[2]+q[3]*q[1]),2.*(q[1]*q[2]-q[3]*q[0]),1.-2.*(q[0]*q[0]+q[1]*q[1])];
            assert!((z[2]-1.).abs()<1e-6);assert!(z[0].abs()<1e-6&&z[1].abs()<1e-6);
        }
    }
    #[test] fn spin_changes_only_orientation_and_zero_cone_keeps_normal() {
        let base=serde_json::json!({"point_count_method":"fixed","point_count":20,"global_seed":3,"rotation_around_normal":false,"max_random_cone_angle":0.0});
        let a=run(plane(),base.clone());let mut spun=base;spun["rotation_around_normal"]=serde_json::json!(true);spun["min_angle"]=serde_json::json!(90.0);spun["max_angle"]=serde_json::json!(90.0);
        let b=run(plane(),spun);assert_eq!(a.p(),b.p());assert_eq!(a.point["N"].data,b.point["N"].data);assert_ne!(a.point["orient"].data,b.point["orient"].data);
    }
    #[test] fn empty_and_zero_coverage() {
        let empty=Geometry::points_only(Vec::new()).unwrap();assert_eq!(run(empty,serde_json::json!({})).npoints,0);
        assert_eq!(run(plane(),serde_json::json!({"coverage":0.0})).npoints,0);
    }
    #[test] fn copies_point_attributes() {
        let mut g=plane();g.point.insert("Cd".into(),Arc::new(Attr::f32(3,vec![1.;12])));
        let out=run(g,serde_json::json!({"point_count_method":"fixed","point_count":4}));
        assert_eq!(out.point["Cd"].as_f32().unwrap(),&[1.;12]);
    }

    #[test] fn tilted_surface_alignment_and_finite_output() {
        let g=Geometry::from_arrays(vec![0.,0.,0., 0.,1.,0., 0.,0.,1.],vec![0,1,2],vec![0,3],None).unwrap();
        let out=run(g,serde_json::json!({"point_count_method":"fixed","point_count":16,"max_random_cone_angle":0.0,"rotation_around_normal":false}));
        let ns=out.point["N"].as_f32().unwrap();let qs=out.point["orient"].as_f32().unwrap();
        for (normal,q) in ns.chunks_exact(3).zip(qs.chunks_exact(4)) {
            let z=[2.*(q[0]*q[2]+q[3]*q[1]),2.*(q[1]*q[2]-q[3]*q[0]),1.-2.*(q[0]*q[0]+q[1]*q[1])];
            for k in 0..3 {assert!((z[k]-normal[k]).abs()<1e-5);}
        }
        assert!(out.p().iter().chain(ns).chain(qs).all(|v|v.is_finite()));
    }
}
