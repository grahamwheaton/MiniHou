//! Surface generation and modeling operations from minihou/nodes/surface.py.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use super::{GEO, Inputs, NodeSpec, linspace};
use crate::{
    error::{Error, Result},
    geometry::{Attr, AttrData, Geometry},
    params::Params,
    topo,
};

mod bridge;
mod convex_hull;
mod revolve;
mod skin;
use bridge::bridge;
use convex_hull::convex_hull;
use revolve::revolve;
use skin::skin;

pub const CONVEX_HULL: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "convex_hull",
    inputs: GEO,
    params: crate::params!(
        "group": "";
        "type": "3d";
        "positions_2d": "fit_plane";
        "shrink_amount": 0.0;
        "preserve_attributes": true;
        "remove_inline_points": true
    ),
    cook: convex_hull,
};

pub const BRIDGE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "bridge",
    inputs: GEO,
    params: crate::params!(
        "align": true;
        "twist": 0;
        "pairing": "mutual_nearest";
        "outgrp_bridge": false;
        "outgrp_bridge_name": "bridgepolys"
    ),
    cook: bridge,
};

pub const REVOLVE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "revolve",
    inputs: GEO,
    params: crate::params!(
        "origin": [0.0, 0.0, 0.0];
        "axis": [0.0, 0.0, 1.0];
        "angle": 360.0;
        "divisions": 32;
        "start_angle": 0.0;
        "end_caps": false;
        "outgrp_endcaps": false;
        "outgrp_endcaps_name": "endcaps"
    ),
    cook: revolve,
};

pub const SKIN: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "skin",
    inputs: GEO,
    params: crate::params!(
        "samples": 0;
        "loop": false;
        "cap": false;
        "outgrp_surface": false;
        "outgrp_surface_name": "skinsurface";
        "outgrp_cap": false;
        "outgrp_cap_name": "skincap"
    ),
    cook: skin,
};

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn len3(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}

fn sub3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_skin_two_lines() {
        let pts = vec![
            0.0, 0.0, 0.0,  1.0, 0.0, 0.0,
            0.0, 1.0, 0.0,  1.0, 1.0, 0.0,
        ];
        let vp = vec![0, 1, 2, 3];
        let offs = vec![0, 2, 4];
        let geo = Geometry::from_arrays(pts, vp, offs, Some(vec![false, false])).unwrap();
        let p = Params::resolve(SKIN.params, None).unwrap();
        let out = skin(&[vec![Arc::new(geo)]], &p).unwrap();
        assert_eq!(out.nprims(), 1);
        assert_eq!(out.topology.prim_offsets, vec![0, 4]);
    }

    #[test]
    fn test_revolve_line() {
        let pts = vec![1.0, 0.0, 0.0, 1.0, 0.0, 1.0];
        let vp = vec![0, 1];
        let offs = vec![0, 2];
        let geo = Geometry::from_arrays(pts, vp, offs, Some(vec![false])).unwrap();
        let p = Params::resolve(REVOLVE.params, serde_json::json!({"divisions": 4}).as_object()).unwrap();
        let out = revolve(&[vec![Arc::new(geo)]], &p).unwrap();
        assert_eq!(out.nprims(), 4);
    }

    #[test]
    fn test_convex_hull_cube() {
        let pts = vec![
            0.0, 0.0, 0.0,
            1.0, 0.0, 0.0,
            1.0, 1.0, 0.0,
            0.0, 1.0, 0.0,
            0.0, 0.0, 1.0,
            1.0, 0.0, 1.0,
            1.0, 1.0, 1.0,
            0.0, 1.0, 1.0,
        ];
        let geo = Arc::new(Geometry::points_only(pts).unwrap());
        // Default params have remove_inline_points = true, merging coplanar triangles into 6 quad faces like Houdini.
        let p_def = Params::resolve(CONVEX_HULL.params, None).unwrap();
        let out_def = convex_hull(&[vec![geo.clone()]], &p_def).unwrap();
        assert_eq!(out_def.nprims(), 6);
        assert_eq!(out_def.npoints, 8);
        assert_eq!(out_def.topology.prim_offsets, vec![0, 4, 8, 12, 16, 20, 24]);

        // With remove_inline_points = false, keeps the 12 triangles.
        let p_tri = Params::resolve(
            CONVEX_HULL.params,
            serde_json::json!({"remove_inline_points": false}).as_object(),
        ).unwrap();
        let out_tri = convex_hull(&[vec![geo]], &p_tri).unwrap();
        assert_eq!(out_tri.nprims(), 12);
        assert_eq!(out_tri.npoints, 8);
    }

    #[test]
    fn test_convex_hull_tetrahedron() {
        let pts = vec![
            0.0, 0.0, 0.0,
            1.0, 0.0, 0.0,
            0.0, 1.0, 0.0,
            0.0, 0.0, 1.0,
            0.2, 0.2, 0.2, // interior point
        ];
        let geo = Arc::new(Geometry::points_only(pts).unwrap());
        let p = Params::resolve(CONVEX_HULL.params, None).unwrap();
        let out = convex_hull(&[vec![geo]], &p).unwrap();
        assert_eq!(out.nprims(), 4);
        assert_eq!(out.npoints, 4);
        assert_eq!(out.topology.prim_offsets, vec![0, 3, 6, 9, 12]);
    }

    #[test]
    fn test_convex_hull_inline_and_coplanar_points_removed() {
        // 3x3x3 grid of points on a cube [0, 2]^3: 27 points total, 8 corners, 12 edge midpoints, 6 face centers, 1 center
        let mut pts = Vec::new();
        for x in 0..3 {
            for y in 0..3 {
                for z in 0..3 {
                    pts.extend_from_slice(&[x as f32, y as f32, z as f32]);
                }
            }
        }
        let geo = Arc::new(Geometry::points_only(pts).unwrap());
        let p = Params::resolve(CONVEX_HULL.params, None).unwrap();
        let out = convex_hull(&[vec![geo.clone()]], &p).unwrap();
        // Inline edge points and coplanar face centers are dropped, leaving only the 8 corners and 6 quad faces
        assert_eq!(out.npoints, 8);
        assert_eq!(out.nprims(), 6);
        assert_eq!(out.topology.prim_offsets, vec![0, 4, 8, 12, 16, 20, 24]);

        // With remove_inline_points = false, edge midpoints and face interior points on the boundary are retained
        let p_keep = Params::resolve(
            CONVEX_HULL.params,
            serde_json::json!({"remove_inline_points": false}).as_object(),
        ).unwrap();
        let out_keep = convex_hull(&[vec![geo]], &p_keep).unwrap();
        assert!(out_keep.npoints > 8);
        assert!(out_keep.nprims() > 6);
    }

    #[test]
    fn test_convex_hull_2d_coplanar() {
        // Square in XY plane with an edge midpoint and an interior point, tilted into 3D for fit_plane as well
        let pts = vec![
            0.0, 0.0, 1.0,
            2.0, 0.0, 1.0,
            2.0, 2.0, 1.0,
            0.0, 2.0, 1.0,
            1.0, 0.0, 1.0, // collinear on bottom edge
            1.0, 1.0, 1.0, // interior point
        ];
        let geo = Arc::new(Geometry::points_only(pts).unwrap());
        for mode in ["fit_plane", "xy"] {
            let p = Params::resolve(
                CONVEX_HULL.params,
                serde_json::json!({"type": "2d", "positions_2d": mode}).as_object(),
            ).unwrap();
            let out = convex_hull(&[vec![geo.clone()]], &p).unwrap();
            assert_eq!(out.nprims(), 1);
            assert_eq!(out.npoints, 4);
            assert_eq!(out.topology.prim_offsets, vec![0, 4]);
        }

        // With remove_inline_points = false in 2D, the collinear edge point (1, 0, 1) is kept (5 vertices)
        let p_keep = Params::resolve(
            CONVEX_HULL.params,
            serde_json::json!({"type": "2d", "positions_2d": "xy", "remove_inline_points": false}).as_object(),
        ).unwrap();
        let out_keep = convex_hull(&[vec![geo]], &p_keep).unwrap();
        assert_eq!(out_keep.nprims(), 1);
        assert_eq!(out_keep.npoints, 5);
    }

    #[test]
    fn test_convex_hull_group_restriction() {
        // Two cubes: points 0..8 at [0,1]^3, points 8..16 at [0,10]^3
        let mut pts = Vec::new();
        for &(x, y, z) in &[
            (0.0, 0.0, 0.0), (1.0, 0.0, 0.0), (1.0, 1.0, 0.0), (0.0, 1.0, 0.0),
            (0.0, 0.0, 1.0), (1.0, 0.0, 1.0), (1.0, 1.0, 1.0), (0.0, 1.0, 1.0),
        ] {
            pts.extend_from_slice(&[x, y, z]);
        }
        for &(x, y, z) in &[
            (0.0, 0.0, 0.0), (10.0, 0.0, 0.0), (10.0, 10.0, 0.0), (0.0, 10.0, 0.0),
            (0.0, 0.0, 10.0), (10.0, 0.0, 10.0), (10.0, 10.0, 10.0), (0.0, 10.0, 10.0),
        ] {
            pts.extend_from_slice(&[x, y, z]);
        }
        let mut geo = Geometry::points_only(pts).unwrap();
        let mut grp = vec![0i32; 16];
        for i in 0..8 {
            grp[i] = 1;
        }
        geo.point.insert("group_small".into(), Arc::new(Attr::i32(1, grp)));
        let geo = Arc::new(geo);

        let p = Params::resolve(
            CONVEX_HULL.params,
            serde_json::json!({"group": "small"}).as_object(),
        ).unwrap();
        let out = convex_hull(&[vec![geo]], &p).unwrap();
        assert_eq!(out.npoints, 8);
        assert_eq!(out.nprims(), 6);
        assert!(out.p().iter().all(|&v| v >= 0.0 && v <= 1.0));
    }

    #[test]
    fn test_convex_hull_attribute_and_group_preservation() {
        let pts = vec![
            0.0, 0.0, 0.0,
            1.0, 0.0, 0.0,
            0.0, 1.0, 0.0,
            0.0, 0.0, 1.0,
            0.2, 0.2, 0.2, // interior point (index 4)
        ];
        let mut geo = Geometry::points_only(pts).unwrap();
        geo.point.insert("id".into(), Arc::new(Attr::i32(1, vec![10, 20, 30, 40, 99])));
        geo.point.insert("group_corners".into(), Arc::new(Attr::i32(1, vec![1, 1, 1, 1, 0])));
        geo.detail.insert("meta".into(), serde_json::json!("kept"));
        let geo = Arc::new(geo);

        // preserve_attributes = true (default)
        let p_on = Params::resolve(CONVEX_HULL.params, None).unwrap();
        let out_on = convex_hull(&[vec![geo.clone()]], &p_on).unwrap();
        assert_eq!(out_on.point.get("id").unwrap().data, AttrData::I32(vec![10, 20, 30, 40]));
        assert_eq!(out_on.point.get("group_corners").unwrap().data, AttrData::I32(vec![1, 1, 1, 1]));
        assert_eq!(out_on.detail.get("meta"), Some(&serde_json::json!("kept")));

        // preserve_attributes = false
        let p_off = Params::resolve(
            CONVEX_HULL.params,
            serde_json::json!({"preserve_attributes": false}).as_object(),
        ).unwrap();
        let out_off = convex_hull(&[vec![geo]], &p_off).unwrap();
        assert!(!out_off.point.contains_key("id"));
        assert!(!out_off.point.contains_key("group_corners"));
        assert!(out_off.detail.is_empty());
    }

    #[test]
    fn test_convex_hull_shrink_stays_inside_original_hull() {
        let mut pts = vec![
            0.0, 0.0, 0.0,
            2.0, 0.0, 0.0,
            2.0, 2.0, 0.0,
            0.0, 2.0, 0.0,
            0.0, 0.0, 2.0,
            2.0, 0.0, 2.0,
            2.0, 2.0, 2.0,
            0.0, 2.0, 2.0,
            // Interior cluster near (0.8, 0.8, 0.8)
            0.8, 0.8, 0.8,
            0.9, 0.7, 0.8,
            0.7, 0.9, 0.8,
        ];
        let geo = Arc::new(Geometry::points_only(std::mem::take(&mut pts)).unwrap());
        let p0 = Params::resolve(CONVEX_HULL.params, None).unwrap();
        let out0 = convex_hull(&[vec![geo.clone()]], &p0).unwrap();

        let p_shrink = Params::resolve(
            CONVEX_HULL.params,
            serde_json::json!({"shrink_amount": 0.5}).as_object(),
        ).unwrap();
        let out_s = convex_hull(&[vec![geo]], &p_shrink).unwrap();
        assert_eq!(out_s.npoints, out0.npoints);
        assert_eq!(out_s.nprims(), out0.nprims());

        // Every shrunk point must lie strictly inside [0, 2]^3 and closer to the interior than the original corners
        for chunk in out_s.p().chunks_exact(3) {
            for &coord in chunk {
                assert!(coord > 0.0 && coord < 2.0, "shrunk coordinate {coord} not strictly inside (0, 2)");
            }
        }
    }
}
