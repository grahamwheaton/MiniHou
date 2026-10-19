//! Python output.py: named outputs use the same validated pass-through cook as the main output.
use super::{InputSpec,Inputs,NodeSpec};
use crate::{Geometry,Error,Result,params::Params};
pub const OUTPUT:NodeSpec=NodeSpec{ outputs: &["geometry"], cook_multi: None,name:"output",inputs:super::GEO,params:crate::params!("object":""),cook:output};
pub const INPUT:NodeSpec=NodeSpec{ outputs: &["geometry"], cook_multi: None,name:"input",inputs:&[InputSpec{name:"preview",multi:false,optional:true}],params:crate::params!("index":0;"name":"geo"),cook:input};
fn input(inputs:&Inputs,p:&Params)->Result<Geometry>{
    let index=p.f("index")?;
    if index.fract()!=0. || !(0.0..=15.0).contains(&index){return Err(Error::new("index must be an integer from 0 to 15"));}
    p.s("name")?;
    inputs[0].first().map(|g|Ok((**g).clone())).unwrap_or_else(||Geometry::points_only(Vec::new()))
}
fn output(inputs:&Inputs,_:&Params)->Result<Geometry>{inputs[0].first().map(|g|(**g).clone()).ok_or_else(||Error::new("input 'geo' is not connected"))}
#[cfg(test)]mod tests{
    use crate::Graph;
    use serde_json::json;
    #[test]fn input_preview_and_empty(){
        for preview in [false,true]{
            let source=if preview{json!("p")}else{json!(null)};
            let g=Graph::from_value(&json!({"display":"in","nodes":{
                "p":{"type":"point"},"in":{"type":"input","inputs":{"preview":source}}
            }})).unwrap();
            let out=g.cook(None).unwrap();assert_eq!(out.npoints,usize::from(preview));out.validate().unwrap();
        }
    }
    #[test]fn extra_names_trim_deduplicate_and_exclude_display(){
        let g=Graph::from_value(&json!({"display":"main","nodes":{
            "main":{"type":"output","params":{"object":"Main"}},
            "a":{"type":"output","params":{"object":" Cube "}},
            "b":{"type":"output","params":{"object":"Cube"}},
            "c":{"type":"output","params":{"object":"  "}}
        }})).unwrap();assert_eq!(g.extra_outputs(),vec![("Cube".into(),"a".into())]);
    }
}
