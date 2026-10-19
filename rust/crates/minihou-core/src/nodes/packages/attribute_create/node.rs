use crate::nodes::{Inputs, NodeSpec, GEO};
use crate::error::{Error, Result};
use crate::geometry::{Attr, Geometry};
use crate::params::Params;
use crate::nodes::attrib_tools::{domain, size, set};
use serde_json::{json, Value};
pub(super) const NODE:NodeSpec=NodeSpec{name:"attribute_create",inputs:GEO,params:crate::params!("name":"attrib1";"domain":"points";"type":"float";"value":[0.,0.,0.];"group":"";"attributes":Value::Null),cook:create};
fn create(inputs:&Inputs,p:&Params)->Result<Geometry>{
    let g=NODE.one(inputs,0)?;
    let Some(attrs)=p.value("attributes").as_array() else {
        if !p.value("attributes").is_null(){return Err(Error::new("attributes must be an array"));}
        return create_one(&g,&p.s("name")?,&p.s("domain")?,&p.s("type")?,None,None,p.value("value"),&p.s("group")?);
    };
    let mut out=(*g).clone();let group=p.s("group")?;
    for (i,a) in attrs.iter().enumerate(){
        let field=|key:&str|a.get(key).ok_or_else(||Error::new(format!("attribute {} missing {key}",i+1)));
        let string=|key:&str|->Result<&str>{field(key)?.as_str().ok_or_else(||Error::new(format!("attribute {} {key} must be a string",i+1)))};
        let name=string("name")?;let class=string("class")?;let kind=string("type")?;
        out=create_one(&out,name,class,kind,Some(field("size")?),Some(field("default")?),field("value")?,&group)?;
    }
    Ok(out)
}
fn components(value:&Value,w:usize,label:&str)->Result<Vec<f64>>{
    let a=value.as_array().ok_or_else(||Error::new(format!("{label} must be an array")))?;
    if a.len()<w{return Err(Error::new(format!("{label} needs {w} components")));}
    a.iter().take(w).map(|v|v.as_f64().ok_or_else(||Error::new(format!("{label} components must be numbers")))).collect()
}
fn create_one(g:&Geometry,name:&str,class:&str,kind:&str,width:Option<&Value>,default:Option<&Value>,value:&Value,group:&str)->Result<Geometry>{
    crate::nodes::group::valid_name(name)?;
    if !["points","primitives","vertices","detail"].contains(&class){return Err(Error::new(format!("invalid attribute class {class:?}")));}
    let dom=domain(class);
    let natural=match kind{"float"|"int"|"string"=>1,"vector2"=>2,"vector4"=>4,"vector"|"color"=>3,_=>return Err(Error::new(format!("invalid attribute type {kind:?}")))};
    let w=if let Some(width)=width{let w=width.as_u64().ok_or_else(||Error::new("attribute size must be a positive integer"))?;if !(1..=4).contains(&w){return Err(Error::new("attribute size must be between 1 and 4"));}w as usize}else{natural};
    if kind=="string"{
        if dom!="detail"{return Err(Error::new("string attributes are supported on Detail only"));}
        let s=value.as_str().ok_or_else(||Error::new("string attribute value must be text"))?;
        let mut out=g.clone();out.detail.insert(name.into(),json!(s));return Ok(out);
    }
    let v=if width.is_none(){let a=components(value,3,"value")?;vec![a[0],a[1],a[2],1.][..w].to_vec()}else{components(value,w,"value")?};
    let d=if let Some(default)=default{components(default,w,"default")?}else{vec![0.;w]};
    if dom=="detail"{let mut out=g.clone();out.detail.insert(name.into(),if w==1{json!(v[0])}else{json!(v)});return Ok(out);}
    if name=="P" && (dom!="point" || w!=3){return Err(Error::new("P must be a point vector attribute"));}
    let n=size(g,dom);let sel=if dom=="point" || dom=="prim"{crate::nodes::group::mask(g,dom,group)?}else{vec![true;n]};
    let arr=if kind=="int"{let mut data=Vec::with_capacity(n*w);for selected in &sel{for k in 0..w{data.push(if *selected{v[k] as i32}else{d[k] as i32});}}Attr::i32(w,data)}
        else{let mut data=Vec::with_capacity(n*w);for selected in &sel{for k in 0..w{data.push(if *selected{v[k] as f32}else{d[k] as f32});}}Attr::f32(w,data)};
    Ok(set(g,dom,name.into(),arr))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
