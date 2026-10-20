//! Subnet: a collapsed group of nodes. The inner graph lives in the node's `inner` field and is expanded into
//! `<subnet>/<inner>` nodes when the graph is cooked (see `graph::expand_subnets`); this node just passes the inner output on.
use crate::{error::{Error, Result}, geometry::Geometry, nodes::{InputSpec, Inputs, NodeSpec}, params::Params};

pub(super) const NODE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "subnet",
    inputs: &[InputSpec { name: "geo", multi: true, optional: false }],
    params: &[],
    cook,
};

fn cook(inputs: &Inputs, _p: &Params) -> Result<Geometry> {
    inputs[0].first().map(|g| (**g).clone()).ok_or_else(|| Error::new("the subnet has no output"))
}
