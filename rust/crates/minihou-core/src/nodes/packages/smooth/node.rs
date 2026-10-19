use std::sync::Arc;
use crate::nodes::{Inputs, NodeSpec, GEO};
use crate::error::{Error, Result};
use crate::geometry::{Geometry};
use crate::params::{Params};
use crate::nodes::attrib_tools::{laplacian};
pub(super) const NODE:NodeSpec=NodeSpec{ outputs: &["geometry"], cook_multi: None,name:"smooth",inputs:GEO,params:crate::params!("iterations":10;"strength":0.5;"method":"taubin";"pin_boundary":true;"group":""),cook:smooth};
fn smooth(inputs:&Inputs,p:&Params)->Result<Geometry>{let g=NODE.one(inputs,0)?;let n=p.i("iterations")?;if n<0{return Err(Error::new("iterations must be nonnegative"));}let f=p.f("strength")?;
    let taubin=p.e("method",&["taubin","laplacian"])?=="taubin";let mut factors=Vec::new();for _ in 0..n{factors.push(f);if taubin{factors.push(1./(0.1-1./f.max(1e-6)));}}
    let mut out=laplacian(&g,"P",p,&factors)?;
    if out.point.contains_key("N") && out.nprims()>0{let spec=crate::nodes::get("normal").unwrap();out=(spec.cook)(&[vec![Arc::new(out)]],&Params::resolve(spec.params,None)?)?;}Ok(out)
}


#[cfg(test)]
#[path = "tests.rs"]
mod tests;
