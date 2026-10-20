//! Node `measure_bounds`: bounding box size and centre (detail attributes bbox_size vec3, bbox_center vec3).

use crate::nodes::{InputSpec, Inputs, NodeSpec};
use crate::error::Result;
use crate::geometry::Geometry;
use crate::params::Params;

pub(super) const NODE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "measure_bounds",
    inputs: &[InputSpec { name: "geo", multi: false, optional: false }],
    params: crate::params!("group": ""),
    cook: measure_bounds,
};

fn measure_bounds(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = NODE.one(inputs, 0)?;
    let mut out = (*g).clone();
    let group = p.s("group")?;

    if g.npoints == 0 {
        out.detail.insert("bbox_size".into(), serde_json::json!([0.0, 0.0, 0.0]));
        out.detail.insert("bbox_center".into(), serde_json::json!([0.0, 0.0, 0.0]));
        return Ok(out);
    }

    let sel = if group.trim().is_empty() {
        vec![true; g.npoints]
    } else {
        crate::nodes::group::mask(&g, "point", &group).unwrap_or_else(|_| vec![false; g.npoints])
    };

    if sel.iter().all(|&b| !b) {
        out.detail.insert("bbox_size".into(), serde_json::json!([0.0, 0.0, 0.0]));
        out.detail.insert("bbox_center".into(), serde_json::json!([0.0, 0.0, 0.0]));
        return Ok(out);
    }

    let p_arr = g.p();
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for (i, &selected) in sel.iter().enumerate() {
        if !selected {
            continue;
        }
        for k in 0..3 {
            let val = p_arr[i * 3 + k];
            if val < min[k] {
                min[k] = val;
            }
            if val > max[k] {
                max[k] = val;
            }
        }
    }

    let size = [
        (max[0] - min[0]) as f64,
        (max[1] - min[1]) as f64,
        (max[2] - min[2]) as f64,
    ];
    let center = [
        ((min[0] + max[0]) * 0.5) as f64,
        ((min[1] + max[1]) * 0.5) as f64,
        ((min[2] + max[2]) * 0.5) as f64,
    ];

    out.detail.insert("bbox_size".into(), serde_json::json!(size));
    out.detail.insert("bbox_center".into(), serde_json::json!(center));
    Ok(out)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
