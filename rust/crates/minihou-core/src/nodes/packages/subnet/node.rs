//! Subnet: a collapsed group of nodes. The inner graph lives in the node's `inner` field and is expanded into
//! `<subnet>/<inner>` nodes when the graph is cooked (see `graph::expand_subnets`); this node just passes the inner output on.
use crate::{error::Result, geometry::Geometry, nodes::{InputSpec, Inputs, NodeSpec}, params::Params};

pub(super) const NODE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "subnet",
    inputs: &[InputSpec { name: "geo", multi: true, optional: false }],
    params: &[],
    cook,
};

fn cook(inputs: &Inputs, _p: &Params) -> Result<Geometry> {
    // A new subnet's unconnected input boundary represents empty geometry.
    Ok(inputs[0].first().map(|g| (**g).clone()).unwrap_or_default())
}

#[cfg(test)]
#[test]
fn unconnected_subnet_boundary_cooks_empty_geometry() {
    let params = Params::resolve(&[], None).unwrap();
    let geometry = cook(&[Vec::new()], &params).unwrap();
    assert_eq!(geometry.npoints, 0);
    geometry.validate().unwrap();
}

/// Parameters declared by an inner input or output boundary are public controls
/// on the enclosing subnet. Each entry names the inner node parameter it drives.
pub(crate) fn public_params(node: &serde_json::Value) -> Vec<&serde_json::Value> {
    let mut entries: Vec<_> = node["promoted_params"].as_array().into_iter().flatten().collect();
    if let Some(inner) = node["inner"]["nodes"].as_object() {
        for boundary in inner.values().filter(|n| matches!(n["type"].as_str(), Some("input" | "output" | "subnet_input" | "subnet_output"))) {
            entries.extend(boundary["custom_params"].as_array().into_iter().flatten());
        }
    }
    entries
}
