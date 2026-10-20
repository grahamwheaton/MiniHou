use crate::nodes::pcg::{cloud, read_pcg, NewAttrVal};
use std::collections::HashMap;
use crate::nodes::{Inputs, NodeSpec, GEO};
use crate::{error::Result, geometry::Geometry, params::Params};

pub(super) const NODE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "pcg_points",
    inputs: GEO,
    params: crate::params!("extents": [0.5, 0.5, 0.5]; "density": 1.0),
    cook: pcg_points,
};

fn pcg_points(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = NODE.one(inputs, 0)?;
    let extents = p.v3("extents")?;
    let density = p.f("density")?;
    let d = read_pcg(&geo);
    let n = geo.npoints;

    let mut new_attrs = HashMap::new();
    if !geo.point.contains_key("rot") {
        new_attrs.insert(
            "rot".into(),
            NewAttrVal::F32(
                3,
                d.rot
                    .iter()
                    .flat_map(|r| [r[0] as f32, r[1] as f32, r[2] as f32])
                    .collect(),
            ),
        );
    }
    if !geo.point.contains_key("scale") {
        new_attrs.insert(
            "scale".into(),
            NewAttrVal::F32(
                3,
                d.scale
                    .iter()
                    .flat_map(|s| [s[0] as f32, s[1] as f32, s[2] as f32])
                    .collect(),
            ),
        );
    }
    if !geo.point.contains_key("bounds_min") {
        let bmin = extents
            .iter()
            .map(|&e| -e as f32)
            .cycle()
            .take(n * 3)
            .collect();
        new_attrs.insert("bounds_min".into(), NewAttrVal::F32(3, bmin));
    }
    if !geo.point.contains_key("bounds_max") {
        let bmax = extents
            .iter()
            .map(|&e| e as f32)
            .cycle()
            .take(n * 3)
            .collect();
        new_attrs.insert("bounds_max".into(), NewAttrVal::F32(3, bmax));
    }
    if !geo.point.contains_key("density") {
        new_attrs.insert(
            "density".into(),
            NewAttrVal::F32(1, vec![density as f32; n]),
        );
    }
    if !geo.point.contains_key("seed") {
        new_attrs.insert("seed".into(), NewAttrVal::I32(1, d.seed));
    }
    cloud(&geo, None, Some(new_attrs))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
