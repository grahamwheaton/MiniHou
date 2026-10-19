use super::*;
use std::sync::Arc;
use serde_json::json;

fn two_adjacent_quads_shared_points() -> Geometry {
    // 6 points, 2 quads sharing edge (1-2)
    // Points 1 and 4 have duplicate positions (1.0, 0.0, 0.0), Points 2 and 5 have duplicate positions (1.0, 1.0, 0.0)
    let p = vec![
        0.0, 0.0, 0.0, // 0
        1.0, 0.0, 0.0, // 1
        1.0, 1.0, 0.0, // 2
        0.0, 1.0, 0.0, // 3
        1.0, 0.0, 0.0, // 4 (duplicate of 1)
        2.0, 0.0, 0.0, // 5
        2.0, 1.0, 0.0, // 6
        1.0, 1.0, 0.0, // 7 (duplicate of 2)
    ];
    let vp = vec![
        0, 1, 2, 3,
        4, 5, 6, 7,
    ];
    let offs = vec![0, 4, 8];
    Geometry::from_arrays(p, vp, offs, None).unwrap()
}

fn run(geo: Geometry, targets: Option<Geometry>, opt: serde_json::Value) -> Geometry {
    let params = Params::resolve(NODE.params, opt.as_object()).unwrap();
    let inputs = if let Some(tgt) = targets {
        vec![vec![Arc::new(geo)], vec![Arc::new(tgt)]]
    } else {
        vec![vec![Arc::new(geo)], vec![]]
    };
    (NODE.cook)(&inputs, &params).unwrap()
}

#[test]
fn test_fuse_welds_coincident_points() {
    let g = two_adjacent_quads_shared_points();
    assert_eq!(g.npoints, 8);
    let out = run(g, None, json!({"distance": 0.01}));
    assert_eq!(out.npoints, 6);
    assert_eq!(out.nprims(), 2);
}

#[test]
fn test_fuse_snap_only_mode() {
    let p = vec![
        0.0, 0.0, 0.0,
        0.0001, 0.0, 0.0,
    ];
    let g = Geometry::points_only(p).unwrap();
    let out = run(g, None, json!({"distance": 0.01, "fuse_points": false, "output_positions": "average"}));
    assert_eq!(out.npoints, 2);
    let pts = out.p();
    assert_eq!(pts[0], pts[3]);
}

#[test]
fn test_fuse_output_groups() {
    let g = two_adjacent_quads_shared_points();
    let out = run(g, None, json!({
        "distance": 0.01,
        "outgrp_fused": true,
        "outgrp_fused_name": "fusedpts",
        "outgrp_unique": true,
        "outgrp_unique_name": "uniquepts"
    }));
    assert!(out.point.contains_key("group_fusedpts"));
    assert!(out.point.contains_key("group_uniquepts"));
    let fused = out.point["group_fusedpts"].as_i32().unwrap();
    let unique = out.point["group_uniquepts"].as_i32().unwrap();
    // 2 fused points, 4 unique points
    assert_eq!(fused.iter().filter(|&&x| x == 1).count(), 2);
    assert_eq!(unique.iter().filter(|&&x| x == 1).count(), 4);
}

#[test]
fn test_fuse_snap_to_targets() {
    let p1 = vec![0.05, 0.0, 0.0, 10.0, 0.0, 0.0];
    let g1 = Geometry::points_only(p1).unwrap();

    let p2 = vec![0.0, 0.0, 0.0];
    let targets = Geometry::points_only(p2).unwrap();

    let out = run(g1, Some(targets), json!({"distance": 0.1, "fuse_points": false}));
    let pts = out.p();
    // Point 0 snapped to target 0
    assert!((pts[0] - 0.0).abs() < 1e-5);
    // Point 1 too far, kept at 10.0
    assert!((pts[3] - 10.0).abs() < 1e-5);
}

#[test]
fn test_fuse_grid_mode() {
    let p = vec![
        0.01, 0.01, 0.0,
        0.02, 0.02, 0.0,
        5.0, 5.0, 0.0,
    ];
    let g = Geometry::points_only(p).unwrap();
    let out = run(g, None, json!({"distance": 0.1, "snap_to": "grid"}));
    assert_eq!(out.npoints, 2);
}
