use crate::{nodes::{Inputs, NodeSpec}, error::Result, geometry::Geometry, params::Params};
pub(super) const NODE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "point", inputs: &[], params: crate::params!("center": [0.0, 0.0, 0.0];), cook: point,
};

fn point(_inputs: &Inputs, p: &Params) -> Result<Geometry> {
    Geometry::points_only(p.v3("center")?.map(|v| v as f32).to_vec())
}


#[cfg(test)]
#[path = "tests.rs"]
mod tests;
