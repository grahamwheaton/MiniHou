use std::sync::Arc;
use crate::{nodes::{Inputs, NodeSpec, GEO}, error::Result, geometry::{Attr, Geometry}, params::Params, normals};
use crate::nodes::model::{gather, corners};
pub(super) const NODE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None,
    name: "facet",
    inputs: GEO,
    params: crate::params!("unique_points":true;"compute_normals":false),
    cook: facet,
};
fn facet(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = NODE.one(inputs, 0)?;
    let mut out = (*g).clone();
    let unique = p.b("unique_points")?;
    if unique {
        let ix: Vec<_> = g
            .topology
            .vertex_point
            .iter()
            .map(|&v| v as usize)
            .collect();
        out.point = gather(&g.point, &ix);
        out.npoints = ix.len();
        let mut t = (*g.topology).clone();
        t.vertex_point = (0..ix.len() as i32).collect();
        out.topology = Arc::new(t);
    }
    if p.b("compute_normals")? {
        let fn_ = normals::normalize_to_f32(&normals::prim_normals(out.p(), &out.topology));
        if unique {
            let (pv, _, _) = corners(&out);
            out.point.insert(
                "N".into(),
                Arc::new(Attr::f32(
                    3,
                    pv.iter()
                        .flat_map(|&p| fn_[3 * p..3 * p + 3].iter().copied())
                        .collect(),
                )),
            );
        } else {
            out.prim.insert("N".into(), Arc::new(Attr::f32(3, fn_)));
        }
    }
    Ok(out)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
