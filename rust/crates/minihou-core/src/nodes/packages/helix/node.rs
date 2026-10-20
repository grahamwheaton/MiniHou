use crate::{nodes::{Inputs, NodeSpec}, error::{Error, Result}, geometry::Geometry, params::Params};

pub(super) const NODE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None, name: "helix", inputs: &[], params: crate::params!(
    "radius": 1.0; "radius_end": -1.0; "height": 2.0; "turns": 3.0; "points": 120), cook: helix };
fn helix(_: &Inputs,p: &Params) -> Result<Geometry> {
    let n=p.i("points")?; if n<2 { return Err(Error::new("points must be at least 2")); } crate::nodes::curves::count(n as usize)?;
    let (r,h,turns)=(p.f("radius")?,p.f("height")?,p.f("turns")?);
    let end=p.f("radius_end")?; let end=if end<0.0 {r} else {end};
    let mut out=Vec::new();
    for t in crate::nodes::linspace(0.0,1.0,n as usize) {
        let rad=r+(end-r)*t; let a=t*turns*2.0*std::f64::consts::PI;
        out.extend([(rad*a.cos()) as f32,(rad*a.sin()) as f32,(t*h) as f32]);
    }
    crate::nodes::curves::curve(out,false)
}
#[cfg(test)]
#[path = "tests.rs"]
mod tests;
