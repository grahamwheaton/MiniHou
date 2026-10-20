use std::sync::Arc;
use crate::nodes::{InputSpec, Inputs, NodeSpec};
use crate::{error::{Error, Result}, geometry::{Attr, Geometry}, params::Params};
pub(super) const NODE:NodeSpec=NodeSpec{ outputs: &["geometry"], cook_multi: None, cook_sidecar: None,name:"bend",inputs:&[InputSpec{name:"geo",multi:false,optional:false},InputSpec{name:"rest",multi:false,optional:true}],params:crate::params!(
    "origin":[0.,0.,0.];"direction":[0.,0.,1.];"length":1.;"up":[0.,1.,0.];"bend":90.;"twist":0.;"taper":1.;"group":""),cook:bend};
fn dot(a:[f64;3],b:[f64;3])->f64{(0..3).map(|i|a[i]*b[i]).sum()}
fn cross(a:[f64;3],b:[f64;3])->[f64;3]{[a[1]*b[2]-a[2]*b[1],a[2]*b[0]-a[0]*b[2],a[0]*b[1]-a[1]*b[0]]}
fn bend(inputs:&Inputs,p:&Params)->Result<Geometry>{
    let g=NODE.one(inputs,0)?;let rest=inputs[1].first();if rest.is_some_and(|r|r.npoints!=g.npoints){return Err(Error::new("bend: the rest geometry must have the same point count as the input"));}
    let capture=rest.map_or(&*g,|r|&**r);let origin=p.v3("origin")?;let d=p.v3("direction")?;let dl=dot(d,d).sqrt().max(1e-12);let d=d.map(|v|v/dl);
    let up=p.v3("up")?;let ud=dot(up,d);let mut u=std::array::from_fn(|k|up[k]-ud*d[k]);
    if dot(u,u).sqrt()<1e-9{u=cross(d,if d[0].abs()<0.9{[1.,0.,0.]}else{[0.,1.,0.]});}
    let ul=dot(u,u).sqrt();if ul==0.{return Err(Error::new("bend: direction is zero"));}let u=u.map(|v|v/ul);let s_axis=cross(u,d);
    let length=p.f("length")?;if length<=0.{return Err(Error::new("length must be positive"));}
    let angle=p.f("bend")?.to_radians();let twist=p.f("twist")?.to_radians();let taper=p.f("taper")?;let sel=crate::nodes::group::mask(&g,"point",&p.s("group")?)?;
    let mut positions=g.p().to_vec();let source_n=g.point.get("N").filter(|a|a.width==3).and_then(|a|a.as_f32());let mut normals=source_n.map(|v|v.to_vec());
    for i in 0..g.npoints{if !sel[i]{continue;}let q=std::array::from_fn(|k|capture.p()[i*3+k] as f64-origin[k]);let(a,s,uval)=(dot(q,d),dot(q,s_axis),dot(q,u));let t=(a/length).clamp(0.,1.);
        let (sn,cs)=(twist*t).sin_cos();let sc=1.+(taper-1.)*t;let(s,uval)=((s*cs-uval*sn)*sc,(s*sn+uval*cs)*sc);
        let(phi_s,phi_c)=(angle*t).sin_cos();
        let (c,tangent,ns)=if angle.abs()<1e-9{(d.map(|v|a*v),d,s_axis)}else{
            let radius=length/angle;let tangent=std::array::from_fn(|k|phi_c*d[k]+phi_s*s_axis[k]);let ns=std::array::from_fn(|k|-phi_s*d[k]+phi_c*s_axis[k]);
            let mut c=std::array::from_fn(|k|radius*phi_s*d[k]+radius*(1.-phi_c)*s_axis[k]);
            if a>length{for k in 0..3{c[k]+=(a-length)*tangent[k];}}if a<0.{c=d.map(|v|a*v);}(c,tangent,ns)
        };
        for k in 0..3{let new=origin[k]+c[k]+s*ns[k]+uval*u[k];positions[i*3+k]=if rest.is_some(){(g.p()[i*3+k] as f64+new-(q[k]+origin[k])) as f32}else{new as f32};}
        if let(Some(src),Some(out))=(source_n,&mut normals){let n=std::array::from_fn(|k|src[i*3+k] as f64);let(na,ns0,nu)=(dot(n,d),dot(n,s_axis),dot(n,u));let(ns0,nu)=(ns0*cs-nu*sn,ns0*sn+nu*cs);let n=std::array::from_fn(|k|na*tangent[k]+ns0*ns[k]+nu*u[k]);let ln=dot(n,n).sqrt();let ln=if ln<1e-20{1.}else{ln};for k in 0..3{out[i*3+k]=(n[k]/ln) as f32;}}
    }
    let mut out=(*g).clone();out.set_p(positions);if let Some(n)=normals{out.point.insert("N".into(),Arc::new(Attr::f32(3,n)));}Ok(out)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
