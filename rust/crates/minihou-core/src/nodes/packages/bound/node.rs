use std::sync::Arc;
use serde_json::json;
use crate::{nodes::{InputSpec, Inputs, NodeSpec}, error::Result, geometry::Geometry, params::Params};
pub(super) const NODE: NodeSpec = NodeSpec {
    name: "bound",
    inputs: &[InputSpec { name: "geo", multi: false, optional: false }],
    params: crate::params!("padding": [0.0, 0.0, 0.0]; "oriented": false, unsupported;),
    cook: bound,
};

/// The world-axis box around the input (`minihou/nodes/transform.py`), in float64 like the numpy body, back to float32.
/// `oriented` (a principal-axis fit through `np.linalg.svd`) is refused: the Rust core has no SVD and the box's detail
/// attributes depend on the sign convention LAPACK happens to pick, which the box points do not.
fn bound(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = NODE.one(inputs, 0)?;
    let padding = p.v3("padding")?;
    if geo.npoints == 0 {
        return box_sized([0.0, 0.0, 0.0]);
    }
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for q in geo.p().as_chunks::<3>().0 {
        for k in 0..3 {
            let v = q[k] as f64;
            if v.is_nan() {
                lo[k] = v;
                hi[k] = v;
            } else {
                if v < lo[k] { lo[k] = v; }
                if v > hi[k] { hi[k] = v; }
            }
        }
    }
    for k in 0..3 {
        lo[k] -= padding[k];
        hi[k] += padding[k];
    }
    let size = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
    let center = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0, (lo[2] + hi[2]) / 2.0];
    let mut out = box_sized(size)?;
    let positions = out.p().as_chunks::<3>().0.iter().flat_map(|q| (0..3).map(move |k| (q[k] as f64 + center[k]) as f32)).collect();
    out.set_p(positions);
    out.detail.insert("bound_min".into(), json!(lo));
    out.detail.insert("bound_max".into(), json!(hi));
    out.detail.insert("bound_size".into(), json!(size));
    Ok(out)
}

/// A default `box` (one division, welded) of the given size at the origin, as `bound` builds it.
fn box_sized(size: [f64; 3]) -> Result<Geometry> {
    let mut given = serde_json::Map::new();
    given.insert("size".into(), json!(size));
    let params = Params::resolve(crate::nodes::generate::BOX.params, Some(&given))?;
    let empty: [Vec<Arc<Geometry>>; 1] = [Vec::new()];
    (crate::nodes::generate::BOX.cook)(&empty, &params)
}


#[cfg(test)]
#[path = "tests.rs"]
mod tests;
