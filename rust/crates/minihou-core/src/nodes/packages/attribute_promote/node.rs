use std::sync::Arc;
use crate::nodes::{Inputs, NodeSpec, GEO};
use crate::error::{Error, Result};
use crate::geometry::{AttrData, Geometry};
use crate::params::{Params};
use crate::nodes::attrib_tools::{table_mut, domain, size};
pub(super) const NODE:NodeSpec=NodeSpec{ outputs: &["geometry"], cook_multi: None,name:"attribute_promote",inputs:GEO,params:crate::params!("name":"Cd";"to":"points";"method":"average";"keep_original":false),cook:promote};
fn promote(inputs:&Inputs,p:&Params)->Result<Geometry>{
    let g=NODE.one(inputs,0)?;let name=p.s("name")?;let dst=p.e("to",&["points","primitives","vertices","detail"])?;let dst=domain(&dst);
    let method=p.e("method",&["average","min","max","sum","first"])?;let keep=p.b("keep_original")?;
    let mut hits=Vec::new();for src in ["point","prim","vertex"]{for(n,a)in g.table(src){if src!=dst && crate::nodes::attribs::matches(&name,n) && (n!="P" || keep){hits.push((src,n.clone(),a.clone()));}}}
    if hits.is_empty(){return Err(Error::new(format!("no attribute {name:?} on another domain")));}
    let mut pv=Vec::new();for i in 0..g.nprims(){pv.extend(std::iter::repeat_n(i,(g.topology.prim_offsets[i+1]-g.topology.prim_offsets[i]) as usize));}
    let mut out=(*g).clone();
    for(src,n,a)in hits{
        let mut rows=vec![Vec::<usize>::new();if dst=="detail"{1}else{size(&g,dst)}];
        if dst=="detail"{rows[0].extend(0..a.rows());}else{
            for (v,&prim) in pv.iter().enumerate(){let pt=g.topology.vertex_point[v] as usize;let s=match src{"point"=>pt,"prim"=>prim,_=>v};let d=match dst{"point"=>pt,"prim"=>prim,_=>v};rows[d].push(s);}
        }
        let mut result=a.zeros_like(rows.len());
        match (&a.data,&mut result.data){
            (AttrData::F32(v),AttrData::F32(r))=>for(i,ids)in rows.iter().enumerate(){if ids.is_empty(){continue;}for k in 0..a.width{
                r[i*a.width+k]=match method.as_str(){"first"=>v[ids[0]*a.width+k],"min"=>ids.iter().map(|&j|v[j*a.width+k]).fold(f32::INFINITY,f32::min),"max"=>ids.iter().map(|&j|v[j*a.width+k]).fold(f32::NEG_INFINITY,f32::max),"sum"=>ids.iter().fold(0f32,|s,&j|s+v[j*a.width+k]),_=> (ids.iter().map(|&j|v[j*a.width+k] as f64).sum::<f64>()/ids.len() as f64) as f32};
            }},
            (AttrData::I32(v),AttrData::I32(r))=>for(i,ids)in rows.iter().enumerate(){if ids.is_empty(){continue;}for k in 0..a.width{
                r[i*a.width+k]=match method.as_str(){"min"=>ids.iter().map(|&j|v[j*a.width+k]).min().unwrap(),"max"=>ids.iter().map(|&j|v[j*a.width+k]).max().unwrap(),"sum"=>ids.iter().fold(0i32,|s,&j|s.wrapping_add(v[j*a.width+k])),_=>v[ids[0]*a.width+k]};
            }},_=>unreachable!(),
        }
        if !keep{table_mut(&mut out,src).remove(&n);}
        if dst=="detail"{let vals:Vec<_>=match result.data{AttrData::F32(v)=>v.into_iter().map(|v|serde_json::json!(v)).collect(),AttrData::I32(v)=>v.into_iter().map(|v|serde_json::json!(v)).collect()};out.detail.insert(n,if a.width==1{vals[0].clone()}else{serde_json::json!(vals)});}
        else{table_mut(&mut out,dst).insert(n,Arc::new(result));}
    }Ok(out)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
