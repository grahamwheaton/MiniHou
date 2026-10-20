//! Python blend.py: additive target deltas in float64, one final float32 cast.
use std::sync::Arc;
use crate::{Attr, Error, Geometry, Result, params::Params};
use super::{InputSpec,Inputs,NodeSpec,scalar_expr::{Expr,Number}};
pub const BLEND:NodeSpec=NodeSpec{ outputs: &["geometry"], cook_multi: None, cook_sidecar: None,name:"blend",inputs:&[
    InputSpec{name:"geo",multi:false,optional:false},InputSpec{name:"targets",multi:true,optional:true}
],params:crate::params!("amount":1.;"weights":"1";"attributes":"P";"mask":"";"group":""),cook:blend};

fn weights(text:&str,count:usize)->Result<Vec<f64>>{
    let mut parts:Vec<&str>=text.split(';').filter(|s|!s.trim().is_empty()).collect();if parts.is_empty(){parts.push("1");}
    let values=parts.iter().map(|s|{
        let e=Expr::parse(s)?;
        let v=e.eval(&mut |name|match name.trim_start_matches('@'){
            "pi"=>Ok(Number::scalar(std::f64::consts::PI)),
            "Frame"=>Ok(Number::scalar(crate::timectx::frame())),
            "Time"=>Ok(Number::scalar(crate::timectx::seconds())),
            "TimeInc"=>Ok(Number::scalar(crate::timectx::time_inc())),
            _=>Err(Error::new(format!("unknown weight name {name:?}")))
        })?.value;
        if !v.is_finite(){return Err(Error::new("weight expression is not finite"));}Ok(v)
    }).collect::<Result<Vec<_>>>()?;
    if values.len()==1{Ok(vec![values[0];count])}else if values.len()!=count{Err(Error::new(format!("{} weights for {count} targets (give one, or one per target separated by ';')",values.len())))}else{Ok(values)}
}
fn blend(inputs:&Inputs,p:&Params)->Result<Geometry>{
    let g=BLEND.one(inputs,0)?;let targets=&inputs[1];if targets.is_empty(){return Ok((*g).clone());}
    let w=weights(&p.s("weights")?,targets.len())?.into_iter().map(|x|Ok(x*p.f("amount")?)).collect::<Result<Vec<_>>>()?;
    for (i,t) in targets.iter().enumerate(){if t.npoints!=g.npoints{return Err(Error::new(format!("target {} has {} points but the base has {}: blending needs the same points",i+1,t.npoints,g.npoints)));}}
    let attributes=p.s("attributes")?;let mut names:Vec<String>=attributes.split_whitespace().map(str::to_owned).collect();
    if names.iter().any(|n|n=="*"){
        names=g.point.iter().filter(|(n,a)|a.as_f32().is_some()&&targets.iter().all(|t|t.point.get(*n).is_some_and(|b|a.width==b.width&&a.rows()==b.rows()))).map(|(n,_)|n.clone()).collect();
    }
    for name in &names{
        let a=g.point.get(name).ok_or_else(||Error::new(format!("the base has no point attribute {name:?}")))?;
        for (i,t) in targets.iter().enumerate(){if !t.point.get(name).is_some_and(|b|b.width==a.width&&b.rows()==a.rows()){return Err(Error::new(format!("target {} lacks point attribute {name:?} with the base's shape",i+1)));}}
        if a.as_f32().is_none(){return Err(Error::new(format!("point attribute {name:?} is not a float attribute; only floats can be blended")));}
    }
    let mask=p.s("mask")?;let mask=mask.trim();
    let per_point=if mask.is_empty(){None}else{
        let a=g.point.get(mask).ok_or_else(||Error::new(format!("the base has no point attribute {mask:?} to use as a mask")))?;
        if a.width!=1{return Err(Error::new(format!("mask attribute {mask:?} must be a single number per point")));}
        Some(super::blender_input::numbers(a))
    };
    let selected=super::groups::point_mask(&g,&p.s("group")?)?;
    let mut out=(*g).clone();
    for name in names{
        let a=&g.point[&name];let base=a.as_f32().unwrap();let mut delta=vec![0f64;base.len()];
        for (wi,t) in w.iter().zip(targets){let target=super::blender_input::numbers(&t.point[&name]);for ((d,&b),v) in delta.iter_mut().zip(base).zip(target){*d+=wi*(v-b as f64);}}
        let values=base.iter().enumerate().map(|(j,&b)|{
            let i=j/a.width;if !selected[i]{b}else{(b as f64+delta[j]*per_point.as_ref().map_or(1.,|m|m[i])) as f32}
        }).collect();out.point.insert(name,Arc::new(Attr::f32(a.width,values)));
    }Ok(out)
}

#[cfg(test)]mod tests{
    use super::*;use serde_json::json;
    #[test]fn additive_deltas_mask_and_groups(){
        let mut g=Geometry::points_only(vec![0.,0.,0.,1.,1.,1.]).unwrap();g.point.insert("mask".into(),Arc::new(Attr::f32(1,vec![0.5,2.])));
        let a=Geometry::points_only(vec![2.,4.,6.,3.,5.,7.]).unwrap();let b=Geometry::points_only(vec![4.,2.,0.,5.,3.,1.]).unwrap();
        let p=Params::resolve(BLEND.params,json!({"weights":"1;0.5","mask":"mask","group":"0"}).as_object()).unwrap();
        let out=blend(&[vec![Arc::new(g)],vec![Arc::new(a),Arc::new(b)]],&p).unwrap();assert_eq!(out.p(),&[2.,2.5,3.,1.,1.,1.]);
    }
    #[test]fn expression_weights(){assert_eq!(weights("sin(pi/2);fit(2,0,4,0,1)",2).unwrap(),vec![1.,0.5]);assert_eq!(weights("; ;",2).unwrap(),vec![1.,1.]);assert!(weights("1;2",3).is_err());crate::timectx::at(Some(25.),Some(24.),||assert_eq!(weights("@Time",1).unwrap(),vec![1.]));}
    #[test]fn no_targets_does_not_parse_unused_weights(){let g=Geometry::points_only(vec![1.,2.,3.]).unwrap();let p=Params::resolve(BLEND.params,json!({"weights":"invalid"}).as_object()).unwrap();assert_eq!(blend(&[vec![Arc::new(g)],vec![]],&p).unwrap().p(),&[1.,2.,3.]);}
    #[test]fn timeline_invalidates_weight_cache(){
        let g=crate::Graph::from_value(&json!({"display":"blend","nodes":{
            "base":{"type":"point"},"target":{"type":"point","params":{"center":[2,0,0]}},
            "blend":{"type":"blend","inputs":{"geo":"base","targets":["target"]},"params":{"weights":"@Time"}}
        }})).unwrap();let mut session=crate::session::Session::new(g);
        session.set_time(1.,24.).unwrap();assert_eq!(session.cook(None).unwrap().p()[0],0.);
        session.set_time(25.,24.).unwrap();assert_eq!(session.cook(None).unwrap().p()[0],2.);
        assert!(session.last_stats().cooked.contains(&"blend".into()));
    }
}
