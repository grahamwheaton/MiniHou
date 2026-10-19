//! The RBD nodes of `minihou/nodes/fracture_tools.py`: `rbd_constraint_properties` (edit glue link strengths).
//! `rbd_material_fracture` composes voronoi_fracture_points / assemble / glue_cluster / convex_decomposition and is
//! registered only once those are in the Rust registry (see docs/RUST_SOLVER_DESIGN.md).

use std::sync::Arc;

use super::model::outgroup;
use super::np_random::Rng;
use super::sim_common::float;
use super::{InputSpec, Inputs, NodeSpec};
use crate::error::{Error, Result};
use crate::geometry::{Attr, AttrData, Geometry};
use crate::params::Params;

pub const RBD_CONSTRAINT_PROPERTIES: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None,
    name: "rbd_constraint_properties",
    inputs: &[InputSpec { name: "constraints", multi: false, optional: false }],
    params: crate::params!("group": ""; "mode": "set"; "strength": 1.0e6; "per_area": true; "variation": 0.0; "seed": 0;
        "unbreakable": false; "outgrp_changed": false; "outgrp_changed_name": "editedlinks"),
    cook: rbd_constraint_properties,
};

fn column(a: &Attr) -> Vec<f64> {
    match &a.data {
        AttrData::F32(v) => v.iter().step_by(a.width.max(1)).map(|&x| x as f64).collect(),
        AttrData::I32(v) => v.iter().step_by(a.width.max(1)).map(|&x| x as f64).collect(),
    }
}

fn rbd_constraint_properties(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let c = RBD_CONSTRAINT_PROPERTIES.one(inputs, 0)?;
    let group = p.s("group")?;
    let mode = p.e("mode", &["set", "scale"])?;
    let strength = float(p, "strength", Some(0.0), None)?;
    let per_area = p.b("per_area")?;
    let variation = float(p, "variation", Some(0.0), Some(1.0))?;
    let seed = p.i("seed")?;
    let unbreakable = p.b("unbreakable")?;
    let n = c.nprims();
    if n == 0 {
        return Ok((*c).clone());
    }
    let cur = column(c.prim.get("strength").ok_or_else(|| {
        Error::new("these are not glue links: no 'strength' attribute (use glue_cluster output = constraints)")
    })?);
    let sel = super::group::mask(&c, "prim", &group)?;
    let mut rng = Rng::new(&[(seed & 0xFFFF_FFFF) as u64, 53]);
    let spread: Vec<f64> = (0..n).map(|_| (1.0 + variation * (rng.random() * 2.0 - 1.0)).max(0.01)).collect();
    let area = c.prim.get("area").map(|a| column(a));
    let new: Vec<f64> = (0..n).map(|i| {
        if unbreakable {
            0.0
        } else if mode == "scale" {
            cur[i] * strength * spread[i]
        } else {
            let base = match (&area, per_area) {
                (Some(a), true) => strength * a[i],
                _ => strength,
            };
            base * spread[i]
        }
    }).collect();
    let mut out = (*c).clone();
    let values: Vec<f32> = (0..n).map(|i| if sel[i] { new[i] } else { cur[i] } as f32).collect();
    out.prim.insert("strength".into(), Arc::new(Attr::f32(1, values)));
    outgroup(&mut out, p, "changed", "prim", &sel)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn links() -> Geometry {
        let mut g = Geometry::from_arrays(vec![0.0; 12], vec![0, 1, 2, 3], vec![0, 2, 4], Some(vec![false, false])).unwrap();
        g.prim.insert("strength".into(), Arc::new(Attr::f32(1, vec![10.0, 20.0])));
        g.prim.insert("area".into(), Arc::new(Attr::f32(1, vec![0.5, 2.0])));
        g
    }

    fn cook(g: Geometry, params: Value) -> Result<Geometry> {
        let s = &RBD_CONSTRAINT_PROPERTIES;
        (s.cook)(&[vec![Arc::new(g)]], &Params::resolve(s.params, params.as_object())?)
    }

    #[test]
    fn set_scale_variation_and_groups() {
        let set = cook(links(), json!({"strength": 100})).unwrap();
        assert_eq!(set.prim["strength"].as_f32().unwrap(), &[50.0, 200.0]);
        let flat = cook(links(), json!({"strength": 100, "per_area": false, "group": "1", "outgrp_changed": true})).unwrap();
        assert_eq!(flat.prim["strength"].as_f32().unwrap(), &[10.0, 100.0]);
        assert_eq!(flat.prim["group_editedlinks"].data, AttrData::I32(vec![0, 1]));
        let scaled = cook(links(), json!({"mode": "scale", "strength": 2, "variation": 0.5, "seed": 5})).unwrap();
        // np.random.default_rng([5, 53]).random(2) = 0.174991606238858, 0.6089609637200527
        let expect = [10.0 * 2.0 * (1.0 + 0.5 * (0.174991606238858 * 2.0 - 1.0)), 20.0 * 2.0 * (1.0 + 0.5 * (0.6089609637200527 * 2.0 - 1.0))];
        assert_eq!(scaled.prim["strength"].as_f32().unwrap(), &expect.map(|x: f64| x as f32));
        assert_eq!(cook(links(), json!({"unbreakable": true})).unwrap().prim["strength"].as_f32().unwrap(), &[0.0, 0.0]);
        let mut bare = links();
        bare.prim.remove("strength");
        assert!(cook(bare, json!({})).unwrap_err().0.contains("not glue links"));
    }
}
