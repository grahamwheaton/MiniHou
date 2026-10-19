//! Pass on one of several inputs (Houdini's Switch).
use std::sync::Arc;
use crate::{
    error::{Error, Result},
    geometry::Geometry,
    nodes::{InputSpec, Inputs, NodeSpec},
    params::Params,
};

pub const INPUTS: &[InputSpec] = &[
    InputSpec { name: "input0", multi: true, optional: true },
    InputSpec { name: "input1", multi: true, optional: true },
    InputSpec { name: "input2", multi: true, optional: true },
    InputSpec { name: "input3", multi: true, optional: true },
    InputSpec { name: "geos", multi: true, optional: true },
];

pub(super) const NODE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None,
    name: "switch",
    inputs: INPUTS,
    params: crate::params!(
        "index": 0;
        "select": 0;
    ),
    cook,
};

fn cook(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let idx = if let Ok(s) = p.i("select") {
        if s != 0 || p.i("index").unwrap_or(0) == 0 {
            s
        } else {
            p.i("index").unwrap_or(s)
        }
    } else {
        p.i("index")?
    };

    let non_empty: Vec<(usize, &[Arc<Geometry>])> = inputs
        .iter()
        .enumerate()
        .filter(|(_, slot)| !slot.is_empty())
        .map(|(i, slot)| (i, slot.as_slice()))
        .collect();

    if non_empty.is_empty() {
        return Err(Error::new("connect at least one input"));
    }

    // Existing single-input behaviour: if only one slot is connected and it holds multiple geometries
    if non_empty.len() == 1 && non_empty[0].1.len() > 1 {
        let geos = non_empty[0].1;
        let clamped = idx.clamp(0, (geos.len() - 1) as i64) as usize;
        return Ok((*geos[clamped]).clone());
    }

    // Direct slot index if within bounds and non-empty
    if idx >= 0 && (idx as usize) < inputs.len() && !inputs[idx as usize].is_empty() {
        return Ok((*inputs[idx as usize][0]).clone());
    }

    // Otherwise clamp among the available non-empty slots
    let clamped_slot = idx.clamp(0, (non_empty.len() - 1) as i64) as usize;
    Ok((*non_empty[clamped_slot].1[0]).clone())
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
