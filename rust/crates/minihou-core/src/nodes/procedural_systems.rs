//! Architecture and procedural systems (minihou/nodes/procedural_systems.py).
use std::sync::Arc;

use super::{InputSpec, Inputs, NodeSpec, GEO};
use crate::error::{Error, Result};
use crate::geometry::{Attr, AttrData, Geometry};
use crate::params::Params;

pub const BUILDING_RULES: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "building_rules",
    inputs: GEO,
    params: crate::params!(
        "storeys": 3;
        "storey_height": 3.0;
        "bay_width": 2.5;
        "window_width": 0.55;
        "window_height": 0.55;
        "balcony_probability": 0.25;
        "balcony_depth": 0.7;
        "roof": "flat";
        "roof_height": 1.5;
        "seed": 0;
        "rules": "[]"
    ),
    cook: building_rules,
};

mod building;
use building::building_rules;

pub const REGION_IDENTIFY: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "region_identify",
    inputs: GEO,
    params: crate::params!(
        "mode": "connectivity";
        "attribute": "class";
        "seed": 0
    ),
    cook: region_identify,
};

pub const REGION_TRANSFORM: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "region_transform",
    inputs: GEO,
    params: crate::params!(
        "translate": [0.0, 0.0, 0.0];
        "rotate": [0.0, 0.0, 0.0];
        "scale": [1.0, 1.0, 1.0];
        "random_translate": [0.0, 0.0, 0.0];
        "random_rotate": [0.0, 0.0, 0.0];
        "random_scale": 0.0;
        "extrude": 0.0;
        "random_extrude": 0.0;
        "seed": 0
    ),
    cook: region_transform,
};

mod regions;
use regions::{region_identify, region_transform};

#[cfg(test)]
mod tests {
    use super::*;

    fn quad() -> Geometry {
        let p = vec![0.0, 0.0, 0.0, 8.0, 0.0, 0.0, 8.0, 6.0, 0.0, 0.0, 6.0, 0.0];
        let vp = vec![0, 1, 2, 3];
        let offs = vec![0, 4];
        Geometry::from_arrays(p, vp, offs, None).unwrap()
    }

    #[test]
    fn test_building_rules_counts_and_groups() {
        let q = quad();
        let p = Params::resolve(
            BUILDING_RULES.params,
            serde_json::json!({
                "storeys": 3,
                "bay_width": 2.0,
                "balcony_probability": 0.0
            })
            .as_object(),
        )
        .unwrap();
        let out = building_rules(&[vec![Arc::new(q)]], &p).unwrap();
        let crate::geometry::AttrData::I32(comp) = &out.prim["component"].data else {
            panic!("component must be an integer attribute");
        };
        assert_eq!(comp.iter().filter(|&&c| c == 0).count(), 42);
        assert_eq!(comp.iter().filter(|&&c| c == 1).count(), 42);
        assert_eq!(comp.iter().filter(|&&c| c == 2).count(), 0);
        assert_eq!(comp.iter().filter(|&&c| c == 3).count(), 1);
        assert_eq!(comp.iter().filter(|&&c| c == 4).count(), 1);

        let max_z = out.p().chunks_exact(3).map(|c| c[2]).fold(f32::NEG_INFINITY, f32::max);
        assert!((max_z - 9.0).abs() < 1e-5);
    }

    #[test]
    fn test_region_identify_and_transform() {
        let q = quad();
        let p_id = Params::resolve(
            REGION_IDENTIFY.params,
            serde_json::json!({"mode": "primitive"}).as_object(),
        )
        .unwrap();
        let marked = region_identify(&[vec![Arc::new(q.clone())]], &p_id).unwrap();
        assert_eq!(marked.nprims(), 1);
        let AttrData::I32(region_id) = &marked.prim["region_id"].data else {
            panic!("region_id must be I32");
        };
        assert_eq!(region_id, &[0]);
        assert!(marked.prim.contains_key("region_random"));
        assert!(marked.prim.contains_key("region_center"));
        assert!(marked.prim.contains_key("Cd"));

        let p_xform = Params::resolve(
            REGION_TRANSFORM.params,
            serde_json::json!({"translate": [1.0, 2.0, 3.0]}).as_object(),
        )
        .unwrap();
        let xformed = region_transform(&[vec![Arc::new(marked)]], &p_xform).unwrap();
        assert_eq!(xformed.npoints, 4);
        assert!((xformed.p()[0] - 1.0).abs() < 1e-5);
        assert!((xformed.p()[1] - 2.0).abs() < 1e-5);
        assert!((xformed.p()[2] - 3.0).abs() < 1e-5);
    }

    #[test]
    fn test_region_transform_extrude() {
        let q = quad();
        let p_xform = Params::resolve(
            REGION_TRANSFORM.params,
            serde_json::json!({"extrude": 0.5}).as_object(),
        )
        .unwrap();
        let xformed = region_transform(&[vec![Arc::new(q)]], &p_xform).unwrap();
        // Quad extruded with back=true produces 6 faces (1 front, 1 back, 4 sides)
        assert_eq!(xformed.nprims(), 6);
    }
}
