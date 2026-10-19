use crate::{error::Result, geometry::Geometry, nodes::{InputSpec, Inputs, NodeSpec}, params::Params};

pub(super) const NODE: NodeSpec = NodeSpec {
    name: "controller",
    inputs: &[InputSpec { name: "geo", multi: false, optional: true }],
    params: &[],
    cook,
};

fn cook(inputs: &Inputs, _p: &Params) -> Result<Geometry> {
    Ok(inputs[0].first().map_or_else(|| Geometry::points_only(Vec::new()).expect("empty geometry"), |g| (**g).clone()))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
