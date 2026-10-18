use crate::nodes::{Inputs, NodeSpec, GEO};
use crate::error::{Error, Result};
use crate::geometry::{Attr, Geometry};
use crate::params::{Params};
use crate::nodes::attrib_tools::{domain, size, set};
pub(super) const NODE:NodeSpec=NodeSpec{name:"attribute_create",inputs:GEO,params:crate::params!("name":"attrib1";"domain":"points";"type":"float";"value":[0.,0.,0.];"group":""),cook:create};
fn create(inputs:&Inputs,p:&Params)->Result<Geometry>{
    let g=NODE.one(inputs,0)?;let name=p.s("name")?;crate::nodes::group::valid_name(&name)?;
    let dom=p.e("domain",&["points","primitives","vertices","detail"])?;let dom=domain(&dom);
    let kind=p.e("type",&["float","int","vector","vector2","color","vector4"])?;let w=match kind.as_str(){"float"|"int"=>1,"vector2"=>2,"vector4"=>4,_=>3};
    let v=p.v3("value")?;let v=[v[0],v[1],v[2],1.];
    if dom=="detail"{let mut out=(*g).clone();out.detail.insert(name,if w==1{serde_json::json!(v[0])}else{serde_json::json!(&v[..w])});return Ok(out);}
    if name=="P" && (dom!="point" || w!=3){return Err(Error::new("P must be a point vector attribute"));}
    let n=size(&g,dom);let sel=if dom=="point" || dom=="prim"{crate::nodes::group::mask(&g,dom,&p.s("group")?)?}else{vec![true;n]};
    let arr=if kind=="int"{Attr::i32(w,(0..n).flat_map(|i|(0..w).map({let sel=&sel;move|k|if sel[i]{v[k] as i32}else{0}})).collect())}
        else{Attr::f32(w,(0..n).flat_map(|i|(0..w).map({let sel=&sel;move|k|if sel[i]{v[k] as f32}else{0.}})).collect())};
    Ok(set(&g,dom,name,arr))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
