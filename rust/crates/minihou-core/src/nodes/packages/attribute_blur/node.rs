use crate::nodes::{Inputs, NodeSpec, GEO};
use crate::error::{Error, Result};
use crate::geometry::{Geometry};
use crate::params::{Params};
use crate::nodes::attrib_tools::{laplacian};
pub(super) const NODE:NodeSpec=NodeSpec{ outputs: &["geometry"], cook_multi: None,name:"attribute_blur",inputs:GEO,params:crate::params!("names":"P";"iterations":5;"strength":0.5;"pin_boundary":true;"group":""),cook:blur};
fn blur(inputs:&Inputs,p:&Params)->Result<Geometry>{let g=NODE.one(inputs,0)?;let n=p.i("iterations")?;if n<0{return Err(Error::new("iterations must be nonnegative"));}laplacian(&g,&p.s("names")?,p,&vec![p.f("strength")?;n as usize])}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
