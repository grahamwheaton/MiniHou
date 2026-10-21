use crate::{error::Result, geometry::Geometry, nodes::{InputSpec, Inputs, NodeSpec}, params::Params};

pub(super) const NODE: NodeSpec = NodeSpec {
    outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "null", inputs: &[InputSpec { name: "geo", multi: false, optional: false }],
    params: crate::params!("label": ""; "size": 1.0), cook: null,
};

fn null(inputs: &Inputs, params: &Params) -> Result<Geometry> {
    // Size is a viewport control; cooking must preserve the input geometry.
    let _ = params.f("size")?;
    Ok((*NODE.one(inputs, 0)?).clone())
}
