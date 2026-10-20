//! Node `measure_length`: curve or polyline length per primitive (prim attribute length) and total (detail total_length).

use std::sync::Arc;

use crate::nodes::{InputSpec, Inputs, NodeSpec};
use crate::error::Result;
use crate::geometry::{Attr, Geometry};
use crate::params::Params;

pub(super) const NODE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "measure_length",
    inputs: &[InputSpec { name: "geo", multi: false, optional: false }],
    params: crate::params!("group": ""),
    cook: measure_length,
};

fn measure_length(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = NODE.one(inputs, 0)?;
    let mut out = (*g).clone();
    let group = p.s("group")?;

    if g.nprims() == 0 {
        out.point.remove("length");
        out.vertex.remove("length");
        out.prim.insert("length".into(), Arc::new(Attr::f32(1, vec![])));
        out.detail.insert("total_length".into(), serde_json::json!(0.0));
        return Ok(out);
    }

    let sel = if group.trim().is_empty() {
        vec![true; g.nprims()]
    } else {
        crate::nodes::group::mask(&g, "prim", &group).unwrap_or_else(|_| vec![false; g.nprims()])
    };

    let p_arr = g.p();
    let mut lengths = Vec::with_capacity(g.nprims());
    let mut total_length: f64 = 0.0;

    for i in 0..g.nprims() {
        if !sel[i] {
            lengths.push(0.0f32);
            continue;
        }
        let s = g.topology.prim_offsets[i] as usize;
        let e = g.topology.prim_offsets[i + 1] as usize;
        let ids = &g.topology.vertex_point[s..e];
        let mut len = 0.0f64;
        let n = if g.topology.is_closed(i) {
            ids.len()
        } else {
            ids.len().saturating_sub(1)
        };
        for j in 0..n {
            let a = ids[j] as usize * 3;
            let b = ids[(j + 1) % ids.len()] as usize * 3;
            let dx = p_arr[a] as f64 - p_arr[b] as f64;
            let dy = p_arr[a + 1] as f64 - p_arr[b + 1] as f64;
            let dz = p_arr[a + 2] as f64 - p_arr[b + 2] as f64;
            len += (dx * dx + dy * dy + dz * dz).sqrt();
        }
        lengths.push(len as f32);
        total_length += len;
    }

    out.point.remove("length");
    out.vertex.remove("length");
    out.prim.insert("length".into(), Arc::new(Attr::f32(1, lengths)));
    out.detail.insert("total_length".into(), serde_json::json!(total_length));
    Ok(out)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
