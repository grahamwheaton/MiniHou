//! Analytic falloffs. Mesh distance requires the unported triangle spatial index.
use std::sync::Arc;
use super::{InputSpec,Inputs,NodeSpec};
use crate::{error::{Error,Result},geometry::{Attr,Geometry},params::Params};
pub const FALLOFF:NodeSpec=NodeSpec{name:"falloff",inputs:&[InputSpec{name:"geo",multi:false,optional:false},InputSpec{name:"shape_mesh",multi:false,optional:true}],params:crate::params!(
    "name":"mask";"domain":"points";"shape":"sphere";"center":[0.,0.,0.];"direction":[0.,0.,1.];"size":1.;"blend":1.;"offset":0.;"narrowness":1.;"every":0;"range":10.;"inside":false;"smooth":true;"mirror":false;"flip":false;"min":0.;"max":1.;"group":""),cook:falloff};
fn dot(a:[f64;3],b:[f64;3])->f64{(0..3).map(|i|a[i]*b[i]).sum()}
fn falloff(inputs:&Inputs,p:&Params)->Result<Geometry>{
    let g=FALLOFF.one(inputs,0)?;let name=p.s("name")?;super::group::valid_name(&name)?;
    let domain=p.e("domain",&["points","primitives"])?;let dom=if domain=="points"{"point"}else{"prim"};
    let shape=p.e("shape",&["sphere","directional","radial","wave","index","mesh"])?;
    if shape=="mesh"{return Err(Error::new("falloff shape 'mesh' requires a triangle closest-point/inside index, not supported by the Rust backend yet"));}
    if name=="P"{return Err(Error::new("P must remain a point vector attribute"));}
    let center=p.v3("center")?;let d=p.v3("direction")?;let len=dot(d,d).sqrt();
    if ["directional","radial","wave"].contains(&shape.as_str()) && len<1e-12{return Err(Error::new("falloff: the direction is zero"));}
    let d=if len>0.{d.map(|x|x/len)}else{[0.;3]};
    let pos=if dom=="point"{g.p().iter().map(|&x|x as f64).collect()}else{super::group::centroids64(&g)};let n=pos.len()/3;
    let size=p.f("size")?;let blend=p.f("blend")?;let offset=p.f("offset")?;let range=p.f("range")?;let every=p.i("every")?;let narrowness=p.f("narrowness")?;
    if size<=0. || range<=0. || every<0{return Err(Error::new("size/range must be positive and every nonnegative"));}
    let mirror=p.b("mirror")?;let smooth=p.b("smooth")?;let flip=p.b("flip")?;let min=p.f("min")?;let max=p.f("max")?;let sel=super::group::mask(&g,dom,&p.s("group")?)?;
    let reference=if d[0].abs()<0.9{[1.,0.,0.]}else{[0.,1.,0.]};let proj=dot(reference,d);let a=std::array::from_fn(|k|reference[k]-d[k]*proj);let al=dot(a,a).sqrt();let a=a.map(|v|v/al.max(1e-30));let b=[d[1]*a[2]-d[2]*a[1],d[2]*a[0]-d[0]*a[2],d[0]*a[1]-d[1]*a[0]];
    let mut values=Vec::with_capacity(n);
    for i in 0..n{let rel=std::array::from_fn(|k|pos[i*3+k] as f64-center[k]);let x=dot(rel,d)-offset;
        let mut w=match shape.as_str(){
            "sphere"=>{let dist=dot(rel,rel).sqrt();if blend>0.{((size-dist)/(size*blend)).clamp(0.,1.)}else{if dist<size{1.}else{0.}}},
            "directional"=>if mirror{1.-(x.abs()/size).clamp(0.,1.)}else{(x/size).clamp(0.,1.)},
            "radial"=>{let t=(dot(rel,b).atan2(dot(rel,a)).to_degrees()-offset).rem_euclid(360.)/360.;if mirror{1.-(2.*t-1.).abs()}else{t}},
            "wave"=>(1.-(x/size).abs()).clamp(0.,1.).powf(narrowness),
            _=>{let mut j=i as f64-offset;if every!=0{j=j.rem_euclid(every as f64);}let t=(j/range).clamp(0.,1.);if mirror{1.-(2.*t-1.).abs()}else{t}},
        };
        if smooth{w=w*w*(3.-2.*w);}if flip{w=1.-w;}values.push(if sel[i]{(min+w*(max-min)) as f32}else{min as f32});
    }
    let mut out=(*g).clone();if dom=="point"{out.point.insert(name,Arc::new(Attr::f32(1,values)));}else{out.prim.insert(name,Arc::new(Attr::f32(1,values)));}Ok(out)
}
#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn sphere_directional_and_group_range(){
        let g=Arc::new(Geometry::points_only(vec![0.,0.,0.,0.,0.,0.5,0.,0.,1.]).unwrap());
        let p=Params::resolve(FALLOFF.params,None).unwrap();let out=falloff(&[vec![g.clone()],vec![]],&p).unwrap();assert_eq!(out.point["mask"].as_f32().unwrap(),[1.,0.5,0.]);
        let p=Params::resolve(FALLOFF.params,serde_json::json!({"shape":"directional","mirror":true,"smooth":false,"group":"1","min":2.,"max":4.}).as_object()).unwrap();
        let out=falloff(&[vec![g],vec![]],&p).unwrap();assert_eq!(out.point["mask"].as_f32().unwrap(),[2.,3.,2.]);
    }
    #[test]
    fn unsupported_mesh_fails_loudly(){let p=Params::resolve(FALLOFF.params,serde_json::json!({"shape":"mesh"}).as_object()).unwrap();assert!(falloff(&[vec![Arc::new(Geometry::points_only(vec![]).unwrap())],vec![]],&p).is_err());}
}
