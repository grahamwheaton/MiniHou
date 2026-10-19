//! Explicit geometry cache. Session uses the input's cook key to reuse this result.
use crate::{error::{Error, Result}, geometry::Geometry,
    nodes::{InputSpec, Inputs, NodeSpec}, params::Params};

pub(super) const NODE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None,
    name: "cache",
    inputs: &[InputSpec { name: "geo", multi: false, optional: false }],
    params: crate::params!("mode": "auto"),
    cook,
};

fn cook(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    p.e("mode", &["auto", "off"])?;
    Ok((**inputs[0].first().ok_or_else(|| Error::new("geo is not connected"))?).clone())
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
