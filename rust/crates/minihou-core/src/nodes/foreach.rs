//! For-each pieces, using native NodeSpec bodies and their parameter validation.
use std::sync::Arc;
use super::{Inputs,NodeSpec,GEO};
use crate::{error::{Error,Result},geometry::{AttrData,Geometry},params::Params};
pub const FOR_EACH:NodeSpec=NodeSpec{ outputs: &["geometry"], cook_multi: None,name:"for_each",inputs:GEO,params:crate::params!(
    "body":"";"params":"{}";"piece":"connectivity";"attribute":"class";"max_pieces":1000),cook:for_each};
thread_local!{static DEPTH:std::cell::Cell<usize>=const{std::cell::Cell::new(0)};}
struct Depth;
impl Drop for Depth{fn drop(&mut self){DEPTH.with(|d|d.set(d.get()-1));}}
fn for_each(inputs:&Inputs,p:&Params)->Result<Geometry>{
    let g=FOR_EACH.one(inputs,0)?;let body=p.s("body")?;
    if body.is_empty(){return Err(Error::new("set 'body' to a node type"));}
    let nt=super::get(&body).ok_or_else(||Error::new(format!("no native node type {body:?}; graph-system bodies are not supported yet")))?;
    if nt.inputs.is_empty(){return Err(Error::new(format!("{body:?} has no geometry input to feed pieces into")));}
    DEPTH.with(|d|{if d.get()>=32{Err(Error::new("for_each body recursion exceeds 32 levels"))}else{d.set(d.get()+1);Ok(())}})?;
    let _depth=Depth;
    let text=p.s("params")?;let overrides:serde_json::Value=serde_json::from_str(if text.is_empty(){"{}"}else{&text}).map_err(|e|Error::new(format!("params must be JSON: {e}")))?;
    let values=Params::resolve(nt.params,Some(overrides.as_object().ok_or_else(||Error::new("params must be a JSON object"))?))?;
    let piece=p.e("piece",&["connectivity","attribute","primitive"])?;let limit=p.i("max_pieces")?;
    if !(1..=10000).contains(&limit){return Err(Error::new("max_pieces must be 1..10000"));}
    let labels:Vec<f64>=match piece.as_str(){
        "connectivity"=>super::attrib_tools::piece_labels(&g).1.into_iter().map(|i|i as f64).collect(),
        "primitive"=>(0..g.nprims()).map(|i|i as f64).collect(),
        _=>{let name=p.s("attribute")?;let a=g.prim.get(&name).ok_or_else(||Error::new(format!("no primitive attribute {name:?}")))?;match &a.data{
            AttrData::F32(v)=>v.chunks(a.width).map(|v|v[0] as f64).collect(),AttrData::I32(v)=>v.chunks(a.width).map(|v|v[0] as f64).collect()}}
    };
    if labels.iter().any(|x|!x.is_finite()){return Err(Error::new("non-finite piece labels are not supported by the Rust backend"));}
    let integer_labels=piece!="attribute" || matches!(g.prim.get(&p.s("attribute")?).map(|a|&a.data),Some(AttrData::I32(_)));
    let mut keys=labels.clone();keys.sort_by(|a,b|a.partial_cmp(b).unwrap());keys.dedup();
    if keys.len()>limit as usize{return Err(Error::new(format!("{} pieces is more than max_pieces ({limit})",keys.len())));}
    let mut outs=Vec::new();
    for (i,&key) in keys.iter().enumerate(){
        let mut part=super::group::keep_prims(&g,&labels.iter().map(|&v|v==key).collect::<Vec<_>>(),true)?;
        part.detail.insert("iteration".into(),serde_json::json!(i));part.detail.insert("numiterations".into(),serde_json::json!(keys.len()));part.detail.insert("value".into(),if integer_labels{serde_json::json!(key as i64)}else{serde_json::json!(key)});
        let mut args=vec![Vec::new();nt.inputs.len()];args[0].push(Arc::new(part));
        for port in &nt.inputs[1..]{if !port.optional{return Err(Error::new(format!("body {body:?} requires additional input {:?}",port.name)));}}
        outs.push(Arc::new((nt.cook)(&args,&values)?));
    }
    if outs.is_empty(){return Ok((*g).clone());}
    let mut out=super::merge::merge(&[outs],&Params::resolve(&[],None)?)?;
    for key in ["iteration","numiterations","value"]{out.detail.remove(key);}Ok(out)
}
#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn primitive_pieces_duplicate_shared_points_and_clear_iteration(){
        let g=Geometry::from_arrays(vec![0.,0.,0.,1.,0.,0.,2.,0.,0.],vec![0,1,1,2],vec![0,2,4],Some(vec![false,false])).unwrap();
        let p=Params::resolve(FOR_EACH.params,serde_json::json!({"body":"transform","piece":"primitive","params":"{\"translate\":[0,0,1]}"}).as_object()).unwrap();
        let out=for_each(&[vec![Arc::new(g)]],&p).unwrap();assert_eq!(out.npoints,4);assert_eq!(out.nprims(),2);assert_eq!(out.p()[2],1.);assert!(!out.detail.contains_key("iteration"));out.validate().unwrap();
    }
    #[test]
    fn bad_body_and_piece_limit_fail(){
        let g=Arc::new(Geometry::from_arrays(vec![0.;6],vec![0,1],vec![0,1,2],None).unwrap());
        for v in [serde_json::json!({"body":"grid"}),serde_json::json!({"body":"null","piece":"primitive","max_pieces":1}),serde_json::json!({"body":"null","params":"[]"})]{
            assert!(for_each(&[vec![g.clone()]],&Params::resolve(FOR_EACH.params,v.as_object()).unwrap()).is_err());
        }
    }
}
