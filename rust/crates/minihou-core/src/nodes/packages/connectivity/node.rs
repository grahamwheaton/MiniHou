use crate::nodes::{Inputs, NodeSpec, GEO};
use crate::error::{Result};
use crate::geometry::{Attr, Geometry};
use crate::params::{Params};
use crate::nodes::attrib_tools::{domain, set, piece_labels};
pub(super) const NODE:NodeSpec=NodeSpec{ outputs: &["geometry"], cook_multi: None, cook_sidecar: None,name:"connectivity",inputs:GEO,params:crate::params!("domain":"primitives";"name":"class"),cook:connectivity};
fn connectivity(inputs:&Inputs,p:&Params)->Result<Geometry>{let g=NODE.one(inputs,0)?;let dom=p.e("domain",&["primitives","points"])?;let name=p.s("name")?;
    let(a,b)=piece_labels(&g);Ok(set(&g,domain(&dom),name,Attr::i32(1,if dom=="points"{a}else{b})))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
