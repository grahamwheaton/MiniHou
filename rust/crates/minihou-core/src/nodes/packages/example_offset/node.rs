use std::sync::Arc;
use crate::{error::{Error, Result}, geometry::Geometry, nodes::{InputSpec, Inputs, NodeSpec}, params::Params};

pub(super) const NODE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None,
    name: "example_offset",
    inputs: &[InputSpec { name: "geo", multi: false, optional: false }],
    params: crate::params!("amount": 1.0; "enabled": true),
    cook,
};

fn cook(inputs: &Inputs, params: &Params) -> Result<Geometry> {
    let source: &Arc<Geometry> = inputs[0].first().ok_or_else(|| Error::new("input 'geo' is not connected"))?;
    let mut output = (**source).clone();
    if params.b("enabled")? {
        let amount = params.f("amount")? as f32;
        let mut positions = output.p().to_vec();
        for point in positions.chunks_exact_mut(3) { point[1] += amount; }
        output.set_p(positions);
    }
    Ok(output)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
