//! Node `measure_volume`: closed-mesh volume (signed, via divergence theorem), detail attribute volume (float).
use crate::nodes::{InputSpec, Inputs, NodeSpec};
use crate::error::Result;
use crate::geometry::Geometry;
use crate::params::Params;

pub(super) const NODE: NodeSpec = NodeSpec {
    name: "measure_volume",
    inputs: &[InputSpec { name: "geo", multi: false, optional: false }],
    params: crate::params!("group": ""),
    cook: measure_volume,
};

fn measure_volume(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = NODE.one(inputs, 0)?;
    let mut out = (*g).clone();
    let group = p.s("group")?;
    if g.npoints == 0 || g.nprims() == 0 {
        out.detail.insert("volume".into(), serde_json::json!(0.0));
        return Ok(out);
    }
    let sel = if group.trim().is_empty() {
        vec![true; g.nprims()]
    } else {
        crate::nodes::group::mask(&g, "prim", &group).unwrap_or_else(|_| vec![false; g.nprims()])
    };
    let face = crate::normals::prim_normals(g.p(), &g.topology);
    let centres = crate::nodes::group::centroids64(&g);
    let mut volume = 0.0f64;
    for i in 0..g.nprims() {
        if !sel[i] || !g.topology.is_closed(i) {
            continue;
        }
        let s = g.topology.prim_offsets[i] as usize;
        let e = g.topology.prim_offsets[i + 1] as usize;
        if e - s < 3 {
            continue;
        }
        let c = [centres[i * 3], centres[i * 3 + 1], centres[i * 3 + 2]];
        let n = face[i];
        volume += (n[0] * c[0] + n[1] * c[1] + n[2] * c[2]) / 6.0;
    }
    out.detail.insert("volume".into(), serde_json::json!(volume));
    Ok(out)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
