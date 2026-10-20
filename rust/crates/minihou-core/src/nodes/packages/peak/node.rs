//! `peak` from minihou/nodes/transform.py.
use crate::nodes::{GEO, Inputs, NodeSpec};
use crate::error::Result;
use crate::geometry::Geometry;
use crate::params::Params;

pub(super) const NODE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "peak",
    inputs: GEO,
    params: crate::params!(
        "distance": 0.1;
        "group": "";
    ),
    cook: peak,
};

fn peak(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = NODE.one(inputs, 0)?;
    let distance = p.f("distance")? as f32;
    let group = p.s("group")?;
    let sel = crate::nodes::group::mask(&geo, "point", &group)?;

    let has_n = geo.point.get("N").filter(|a| a.width == 3).and_then(|a| a.as_f32()).is_some();
    let normals = if has_n {
        geo.point["N"].as_f32().unwrap().to_vec()
    } else {
        crate::normals::point_normals(geo.p(), &geo.topology, geo.npoints)
    };

    let p_slice = geo.p();
    let mut new_p = p_slice.to_vec();
    for i in 0..geo.npoints {
        if sel[i] {
            new_p[i * 3] += normals[i * 3] * distance;
            new_p[i * 3 + 1] += normals[i * 3 + 1] * distance;
            new_p[i * 3 + 2] += normals[i * 3 + 2] * distance;
        }
    }

    let mut out = (*geo).clone();
    out.set_p(new_p);
    Ok(out)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
