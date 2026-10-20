//! Unpack: packed primitives and native instances back into real geometry (see `nodes/packed.rs`).
use crate::{error::{Error, Result}, geometry::Geometry,
    nodes::{packed, InputSpec, Inputs, NodeSpec}, params::Params};

pub(super) const NODE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "unpack",
    inputs: &[InputSpec { name: "geo", multi: false, optional: false }],
    params: crate::params!("group": ""; "recursive": false),
    cook,
};

fn cook(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = inputs[0].first().ok_or_else(|| Error::new("geo is not connected"))?;
    let pattern = p.s("group")?;
    let mask = if pattern.trim().is_empty() { None } else { Some(crate::nodes::groups::point_mask(geo, &pattern)?) };
    packed::unpack(geo, mask.as_deref(), p.b("recursive")?)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
