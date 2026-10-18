//! Node `measure_area`: face area (prim attribute area) and total (detail total_area).

use std::sync::Arc;

use crate::nodes::{InputSpec, Inputs, NodeSpec};
use crate::error::Result;
use crate::geometry::{Attr, Geometry};
use crate::params::Params;

pub(super) const NODE: NodeSpec = NodeSpec {
    name: "measure_area",
    inputs: &[InputSpec { name: "geo", multi: false, optional: false }],
    params: crate::params!("group": ""),
    cook: measure_area,
};

fn measure_area(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = NODE.one(inputs, 0)?;
    let mut out = (*g).clone();
    let group = p.s("group")?;

    if g.nprims() == 0 {
        out.point.remove("area");
        out.vertex.remove("area");
        out.prim.insert("area".into(), Arc::new(Attr::f32(1, vec![])));
        out.detail.insert("total_area".into(), serde_json::json!(0.0));
        return Ok(out);
    }

    let sel = if group.trim().is_empty() {
        vec![true; g.nprims()]
    } else {
        crate::nodes::group::mask(&g, "prim", &group).unwrap_or_else(|_| vec![false; g.nprims()])
    };

    let face = crate::normals::prim_normals(g.p(), &g.topology);
    let mut areas = Vec::with_capacity(g.nprims());
    let mut total_area: f64 = 0.0;

    for i in 0..g.nprims() {
        if !sel[i] || !g.topology.is_closed(i) {
            areas.push(0.0f32);
            continue;
        }
        let s = g.topology.prim_offsets[i] as usize;
        let e = g.topology.prim_offsets[i + 1] as usize;
        if e - s < 3 {
            areas.push(0.0f32);
            continue;
        }
        let n = face[i];
        let norm = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        let a = norm / 2.0;
        areas.push(a as f32);
        total_area += a;
    }

    out.point.remove("area");
    out.vertex.remove("area");
    out.prim.insert("area".into(), Arc::new(Attr::f32(1, areas)));
    out.detail.insert("total_area".into(), serde_json::json!(total_area));
    Ok(out)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
