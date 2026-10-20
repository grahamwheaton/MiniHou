//! Fracture tools: assemble and face-contact adjacency (minihou/nodes/fracture_tools.py).
#[cfg(test)]
use std::sync::Arc;

use super::{InputSpec, NodeSpec, GEO};
#[cfg(test)]
use crate::geometry::{Attr, AttrData, Geometry};
#[cfg(test)]
use crate::params::Params;

mod adjacency;

pub(super) use adjacency::{extent, pairs, contacts};
use adjacency::voronoi_adjacency;

pub const VORONOI_ADJACENCY: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "voronoi_adjacency",
    inputs: &[
        InputSpec { name: "geo", multi: false, optional: false },
        InputSpec { name: "points", multi: false, optional: true },
    ],
    params: crate::params!("piece_attrib": "piece"; "max_gap": 0.0; "min_area": 0.0; "strong_area": 0.5;
        "outgrp_connected": false; "outgrp_connected_name": "connected";
        "outgrp_isolated": false; "outgrp_isolated_name": "isolated";
        "outgrp_strong": false; "outgrp_strong_name": "stronglinks"),
    cook: voronoi_adjacency,
};

pub const ASSEMBLE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "assemble",
    inputs: GEO,
    params: crate::params!(
        "source": "attribute";
        "piece_attrib": "piece";
        "groups": "*";
        "output_attrib": "piece";
        "renumber": true;
        "unshare_points": true;
        "pivot": "centroid";
        "name_prefix": "piece";
        "piece_groups": false;
        "remove_source_groups": false;
        "outgrp_unshared": false;
        "outgrp_unshared_name": "splitpoints";
        "outgrp_shared": false;
        "outgrp_shared_name": "sharedpoints"
    ),
    cook: assemble,
};

mod assemble;
use assemble::assemble;

pub const GLUE_CLUSTER: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "glue_cluster",
    inputs: GEO,
    params: crate::params!(
        "piece_attrib": "piece";
        "output": "pieces";
        "strength": 4.0e6;
        "strength_variation": 0.2;
        "seed": 0;
        "max_gap": 0.0;
        "min_area": 0.0;
        "min_share": 0.0;
        "clusters": 0;
        "weak": 0.5;
        "outgrp_glued": false;
        "outgrp_glued_name": "glued";
        "outgrp_loose": false;
        "outgrp_loose_name": "loose";
        "outgrp_interface": false;
        "outgrp_interface_name": "interface";
        "outgrp_internal": false;
        "outgrp_internal_name": "internal";
        "outgrp_external": false;
        "outgrp_external_name": "external";
        "outgrp_weak": false;
        "outgrp_weak_name": "weaklinks"
    ),
    cook: glue_cluster,
};

mod glue;
use glue::glue_cluster;

#[cfg(test)]
mod tests {
    use super::*;

    fn test_geo() -> Geometry {
        // Two disjoint triangles sharing no points
        let p = vec![
            0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0,
            2.0, 0.0, 0.0, 3.0, 0.0, 0.0, 2.0, 1.0, 0.0,
        ];
        let vp = vec![0, 1, 2, 3, 4, 5];
        let offs = vec![0, 3, 6];
        Geometry::from_arrays(p, vp, offs, None).unwrap()
    }

    #[test]
    fn test_assemble_connectivity() {
        let g = test_geo();
        let p = Params::resolve(
            ASSEMBLE.params,
            serde_json::json!({"source": "connectivity", "piece_attrib": ""}).as_object(),
        )
        .unwrap();
        let out = assemble(&[vec![Arc::new(g)]], &p).unwrap();
        assert_eq!(out.prim["piece"].data, AttrData::I32(vec![0, 1]));
        assert_eq!(out.detail["piece_names"].as_array().unwrap().len(), 2);
        assert!(out.prim.contains_key("pivot"));
    }

    #[test]
    fn test_assemble_missing_attrib_error() {
        let g = test_geo();
        let p = Params::resolve(
            ASSEMBLE.params,
            serde_json::json!({"source": "attribute", "piece_attrib": "nope"}).as_object(),
        )
        .unwrap();
        assert!(assemble(&[vec![Arc::new(g)]], &p).is_err());
    }

    fn geom_from_encoded(val: &serde_json::Value) -> Geometry {
        let ints = |v: &serde_json::Value| v.as_array().unwrap().iter().map(|x| x.as_i64().unwrap() as i32).collect::<Vec<_>>();
        let floats = |v: &serde_json::Value| v.as_array().unwrap().iter().map(|x| x.as_f64().unwrap() as f32).collect::<Vec<_>>();
        let p_data = floats(&val["point"]["P"]["data"]);
        let vp = ints(&val["vp"]);
        let offsets = ints(&val["offsets"]);
        let closed: Vec<bool> = val["closed"].as_array().unwrap().iter().map(|x| x.as_bool().unwrap()).collect();
        let mut g = Geometry::from_arrays(p_data, vp, offsets, Some(closed)).unwrap();
        if let Some(pts) = val["point"].as_object() {
            for (name, a) in pts {
                if name == "P" { continue; }
                let width = a["width"].as_u64().unwrap() as usize;
                let dtype = a["dtype"].as_str().unwrap();
                let attr = if dtype == "int32" {
                    Attr::i32(width, ints(&a["data"]))
                } else {
                    Attr::f32(width, floats(&a["data"]))
                };
                g.point.insert(name.clone(), Arc::new(attr));
            }
        }
        if let Some(prims) = val["prim"].as_object() {
            for (name, a) in prims {
                let width = a["width"].as_u64().unwrap() as usize;
                let dtype = a["dtype"].as_str().unwrap();
                let attr = if dtype == "int32" {
                    Attr::i32(width, ints(&a["data"]))
                } else {
                    Attr::f32(width, floats(&a["data"]))
                };
                g.prim.insert(name.clone(), Arc::new(attr));
            }
        }
        g
    }

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol * (1.0 + a.abs().max(b.abs()))
    }

    fn check_parity(name: &str, got: &Geometry, want: &serde_json::Value, tol: f64) {
        let ints = |v: &serde_json::Value| v.as_array().unwrap().iter().map(|x| x.as_i64().unwrap() as i32).collect::<Vec<_>>();
        assert_eq!(got.topology.vertex_point, ints(&want["vp"]), "{name}: vertex_point");
        assert_eq!(got.topology.prim_offsets, ints(&want["offsets"]), "{name}: offsets");
        let closed: Vec<bool> = want["closed"].as_array().unwrap().iter().map(|x| x.as_bool().unwrap()).collect();
        assert_eq!(got.topology.closed_mask(), closed, "{name}: closed");
        for (dom, table) in [("point", &got.point), ("prim", &got.prim)] {
            let w = want[dom].as_object().unwrap();
            let mut names: Vec<&String> = table.keys().collect();
            let mut expected: Vec<&String> = w.keys().collect();
            names.sort();
            expected.sort();
            assert_eq!(names, expected, "{name}: {dom} attribute names");
            for (attr, a) in table {
                let e = &w[attr];
                assert_eq!(a.width as u64, e["width"].as_u64().unwrap(), "{name}: {dom}/{attr} width");
                assert_eq!(a.dtype(), e["dtype"].as_str().unwrap(), "{name}: {dom}/{attr} dtype");
                let data = e["data"].as_array().unwrap();
                match &a.data {
                    AttrData::F32(v) => {
                        assert_eq!(v.len(), data.len(), "{name}: {dom}/{attr} len");
                        for (i, (x, y)) in v.iter().zip(data).enumerate() {
                            let yf = y.as_f64().unwrap();
                            assert!(close(*x as f64, yf, tol), "{name}: {dom}/{attr}[{i}] {x} != {yf}");
                        }
                    }
                    AttrData::I32(v) => assert_eq!(*v, ints(&e["data"]), "{name}: {dom}/{attr}"),
                }
            }
        }
    }

    #[test]
    fn test_glue_cluster_golden_parity() {
        let cases_json: serde_json::Value =
            serde_json::from_str(include_str!("test_data/glue_cluster_golden.json")).unwrap();
        let s = &GLUE_CLUSTER;
        for c in cases_json.as_array().unwrap() {
            let name = c["name"].as_str().unwrap();
            let input_geo = geom_from_encoded(&c["input"]);
            let p = Params::resolve(s.params, c["params"].as_object()).unwrap();
            let got = (s.cook)(&[vec![Arc::new(input_geo)]], &p).unwrap();
            check_parity(name, &got, &c["expected"], 1e-4);
        }
    }

    #[test]
    fn test_glue_cluster_open_curves_error() {
        // An open line should produce an error
        let p = vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0];
        let vp = vec![0, 1];
        let offs = vec![0, 2];
        let g = Geometry::from_arrays(p, vp, offs, Some(vec![false])).unwrap();
        let s = &GLUE_CLUSTER;
        let p = Params::resolve(s.params, None).unwrap();
        let err = (s.cook)(&[vec![Arc::new(g)]], &p).unwrap_err();
        assert!(err.0.contains("glue_cluster works on polygons, but the input has 1 open curve(s)"));
    }

    #[test]
    fn test_glue_cluster_seeded_variation_and_zero_strength() {
        let cases_json: serde_json::Value =
            serde_json::from_str(include_str!("test_data/glue_cluster_golden.json")).unwrap();
        let s = &GLUE_CLUSTER;
        let c0 = &cases_json[0]; // voronoi_constraints
        let input_geo = geom_from_encoded(&c0["input"]);

        // Same seed => same strength
        let p1 = Params::resolve(s.params, serde_json::json!({"output": "constraints", "strength": 1000.0, "strength_variation": 0.5, "seed": 7}).as_object()).unwrap();
        let p2 = Params::resolve(s.params, serde_json::json!({"output": "constraints", "strength": 1000.0, "strength_variation": 0.5, "seed": 7}).as_object()).unwrap();
        let out1 = (s.cook)(&[vec![Arc::new(input_geo.clone())]], &p1).unwrap();
        let out2 = (s.cook)(&[vec![Arc::new(input_geo.clone())]], &p2).unwrap();
        assert_eq!(out1.prim["strength"].as_f32().unwrap(), out2.prim["strength"].as_f32().unwrap());

        // Different seed => different strength
        let p3 = Params::resolve(s.params, serde_json::json!({"output": "constraints", "strength": 1000.0, "strength_variation": 0.5, "seed": 8}).as_object()).unwrap();
        let out3 = (s.cook)(&[vec![Arc::new(input_geo.clone())]], &p3).unwrap();
        assert_ne!(out1.prim["strength"].as_f32().unwrap(), out3.prim["strength"].as_f32().unwrap());

        // Zero strength => all strength is 0.0
        let p_zero = Params::resolve(s.params, serde_json::json!({"output": "constraints", "strength": 0.0}).as_object()).unwrap();
        let out_zero = (s.cook)(&[vec![Arc::new(input_geo.clone())]], &p_zero).unwrap();
        for &st in out_zero.prim["strength"].as_f32().unwrap() {
            assert_eq!(st, 0.0);
        }
    }

    #[test]
    fn test_glue_cluster_filters() {
        let cases_json: serde_json::Value =
            serde_json::from_str(include_str!("test_data/glue_cluster_golden.json")).unwrap();
        let s = &GLUE_CLUSTER;
        let c0 = &cases_json[0];
        let input_geo = geom_from_encoded(&c0["input"]);

        // Regular cook
        let p_base = Params::resolve(s.params, serde_json::json!({"output": "constraints"}).as_object()).unwrap();
        let out_base = (s.cook)(&[vec![Arc::new(input_geo.clone())]], &p_base).unwrap();

        // Huge min_area drops all links
        let p_min_area = Params::resolve(s.params, serde_json::json!({"output": "constraints", "min_area": 1e9}).as_object()).unwrap();
        let out_area = (s.cook)(&[vec![Arc::new(input_geo.clone())]], &p_min_area).unwrap();
        assert_eq!(out_area.nprims(), 0);

        // min_share = 1.0 keeps only the largest link(s)
        let p_share = Params::resolve(s.params, serde_json::json!({"output": "constraints", "min_share": 1.0}).as_object()).unwrap();
        let out_share = (s.cook)(&[vec![Arc::new(input_geo.clone())]], &p_share).unwrap();
        assert!(out_share.nprims() >= 1 && out_share.nprims() < out_base.nprims());
    }

    #[test]
    fn test_glue_cluster_invalid_attrib_error() {
        let g = test_geo();
        let s = &GLUE_CLUSTER;
        let p = Params::resolve(s.params, serde_json::json!({"piece_attrib": "nonexistent"}).as_object()).unwrap();
        let err = (s.cook)(&[vec![Arc::new(g)]], &p).unwrap_err();
        assert!(err.0.contains("no primitive attribute \"nonexistent\""));
    }

    #[test]
    fn test_glue_cluster_empty_and_single_element() {
        let s = &GLUE_CLUSTER;
        let empty = Geometry::points_only(vec![]).unwrap();

        // Empty geometry in pieces mode
        let p_pcs = Params::resolve(s.params, serde_json::json!({"output": "pieces", "piece_attrib": ""}).as_object()).unwrap();
        let out_pcs = (s.cook)(&[vec![Arc::new(empty.clone())]], &p_pcs).unwrap();
        assert_eq!(out_pcs.npoints, 0);
        assert_eq!(out_pcs.nprims(), 0);

        // Empty geometry in constraints mode
        let p_cons = Params::resolve(s.params, serde_json::json!({"output": "constraints", "piece_attrib": ""}).as_object()).unwrap();
        let out_cons = (s.cook)(&[vec![Arc::new(empty)]], &p_cons).unwrap();
        assert_eq!(out_cons.nprims(), 0);

        // Open curve fails
        let curve = Geometry::from_arrays(
            vec![0., 0., 0., 1., 0., 0.],
            vec![0, 1],
            vec![0, 2],
            Some(vec![false]),
        ).unwrap();
        let err_curve = (s.cook)(&[vec![Arc::new(curve)]], &p_pcs).unwrap_err();
        assert!(err_curve.0.contains("open curve"));

        // Single triangle (single piece)
        let tri = Geometry::from_arrays(
            vec![0., 0., 0., 1., 0., 0., 0., 1., 0.],
            vec![0, 1, 2],
            vec![0, 3],
            None,
        ).unwrap();
        let out_tri = (s.cook)(&[vec![Arc::new(tri)]], &p_pcs).unwrap();
        assert_eq!(out_tri.nprims(), 1);
        assert_eq!(out_tri.prim["cluster"].as_i32().unwrap(), &[0]);
    }

    #[test]
    fn test_glue_cluster_invalid_params_and_determinism() {
        let s = &GLUE_CLUSTER;
        let g = test_geo();

        // Invalid output enum
        let p_bad_out = Params::resolve(s.params, serde_json::json!({"output": "everything", "piece_attrib": ""}).as_object()).unwrap();
        assert!((s.cook)(&[vec![Arc::new(g.clone())]], &p_bad_out).is_err());

        // Negative strength or clusters
        let p_neg_str = Params::resolve(s.params, serde_json::json!({"strength": -1.0, "piece_attrib": ""}).as_object()).unwrap();
        assert!((s.cook)(&[vec![Arc::new(g.clone())]], &p_neg_str).is_err());

        // Determinism: seeded random gives identical clusters and strength
        let p1 = Params::resolve(s.params, serde_json::json!({"seed": 42, "clusters": 2, "output": "pieces", "piece_attrib": ""}).as_object()).unwrap();
        let p2 = Params::resolve(s.params, serde_json::json!({"seed": 42, "clusters": 2, "output": "pieces", "piece_attrib": ""}).as_object()).unwrap();
        let out1 = (s.cook)(&[vec![Arc::new(g.clone())]], &p1).unwrap();
        let out2 = (s.cook)(&[vec![Arc::new(g)]], &p2).unwrap();
        assert_eq!(out1.prim["cluster"].data, out2.prim["cluster"].data);
    }
}
