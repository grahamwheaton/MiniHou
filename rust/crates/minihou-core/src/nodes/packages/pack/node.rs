//! Pack: geometry into packed primitives (see `nodes/packed.rs` for the representation).
use crate::{error::{Error, Result}, geometry::Geometry,
    nodes::{packed::{self, PackMode}, InputSpec, Inputs, NodeSpec}, params::Params};

pub(super) const NODE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "pack",
    inputs: &[InputSpec { name: "geo", multi: false, optional: false }],
    params: crate::params!("mode": "all"; "group": ""; "pivot": "centroid"),
    cook,
};

fn cook(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = inputs[0].first().ok_or_else(|| Error::new("geo is not connected"))?;
    let mode = match p.e("mode", &["all", "group", "connectivity"])?.as_str() {
        "group" => PackMode::Group,
        "connectivity" => PackMode::Connectivity,
        _ => PackMode::All,
    };
    let mask = if mode == PackMode::Group { Some(crate::nodes::group::mask(geo, "prim", &p.s("group")?)?) } else { None };
    let centroid = p.e("pivot", &["centroid", "origin"])? == "centroid";
    packed::pack(geo, mode, mask.as_deref(), centroid)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
