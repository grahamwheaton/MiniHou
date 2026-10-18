use super::*;
use std::sync::Arc;
use crate::geometry::Attr;

#[test]
fn clean_removes_degenerates_and_unused() {
    let p = vec![
        0.0, 0.0, 0.0,
        1.0, 0.0, 0.0,
        1.0, 1.0, 0.0,
        0.0, 1.0, 0.0,
        5.0, 5.0, 5.0,
    ];
    let g = Arc::new(Geometry::from_arrays(p, vec![0, 1, 1, 2, 3, 0, 1, 2, 2, 2], vec![0, 6, 8, 9, 10], None).unwrap());
    let params = Params::resolve(NODE.params, serde_json::json!({"fuse_distance": 0.0, "remove_degenerate": true, "remove_unused_points": true}).as_object()).unwrap();
    let out = clean(&[vec![g]], &params).unwrap();
    assert_eq!(out.npoints, 4);
    assert_eq!(out.nprims(), 1);
    assert_eq!(out.topology.vertex_point, vec![0, 1, 2, 3]);
}

#[test]
fn test_degenerate_triangles_removed() {
    // Valid triangle (0, 1, 2) and degenerate collinear triangle (3, 4, 5) with area 0
    let p = vec![
        0.0, 0.0, 0.0,
        1.0, 0.0, 0.0,
        0.0, 1.0, 0.0,
        0.0, 0.0, 1.0,
        1.0, 0.0, 1.0,
        2.0, 0.0, 1.0, // collinear with (0,0,1) and (1,0,1)
    ];
    let g = Arc::new(Geometry::from_arrays(p, vec![0, 1, 2, 3, 4, 5], vec![0, 3, 6], None).unwrap());
    let params = Params::resolve(NODE.params, serde_json::json!({
        "remove_degenerate": true,
        "degenerate_tolerance": 0.001,
        "remove_unused_points": false,
    }).as_object()).unwrap();
    let out = clean(&[vec![g]], &params).unwrap();
    assert_eq!(out.nprims(), 1, "degenerate collinear triangle must be removed");
    assert_eq!(out.topology.vertex_point, vec![0, 1, 2]);
}

#[test]
fn test_unused_points_removed() {
    let p = vec![
        0.0, 0.0, 0.0,
        1.0, 0.0, 0.0,
        0.0, 1.0, 0.0,
        10.0, 10.0, 10.0, // unused
        20.0, 20.0, 20.0, // unused
    ];
    let g = Arc::new(Geometry::from_arrays(p, vec![0, 1, 2], vec![0, 3], None).unwrap());
    let params = Params::resolve(NODE.params, serde_json::json!({
        "remove_unused_points": true,
        "remove_degenerate": false,
    }).as_object()).unwrap();
    let out = clean(&[vec![g]], &params).unwrap();
    assert_eq!(out.npoints, 3, "unused points must be removed");
    assert_eq!(out.nprims(), 1);
}

#[test]
fn test_nan_points_removed() {
    let p = vec![
        0.0, 0.0, 0.0,
        1.0, 0.0, 0.0,
        0.0, 1.0, 0.0,
        f32::NAN, 0.0, 0.0, // NaN point
        5.0, 5.0, 5.0,
    ];
    // Prim 0: valid triangle (0,1,2); Prim 1: invalid triangle referencing NaN point 3 (3,4,1)
    let g = Arc::new(Geometry::from_arrays(p, vec![0, 1, 2, 3, 4, 1], vec![0, 3, 6], None).unwrap());
    let params = Params::resolve(NODE.params, serde_json::json!({
        "remove_nans": true,
        "remove_degenerate": false,
        "remove_unused_points": true,
    }).as_object()).unwrap();
    let out = clean(&[vec![g]], &params).unwrap();
    assert_eq!(out.nprims(), 1, "primitive with NaN point must be removed");
    assert_eq!(out.npoints, 3, "NaN point and unused point must be removed");
    assert_eq!(out.topology.vertex_point, vec![0, 1, 2]);
}

#[test]
fn test_consolidation_merges_coincident_points() {
    // Points 0 and 1 are almost coincident (distance 0.005)
    let p = vec![
        0.0, 0.0, 0.0,
        0.005, 0.0, 0.0,
        1.0, 0.0, 0.0,
        0.0, 1.0, 0.0,
    ];
    // Two triangles sharing the coincident points
    let g = Arc::new(Geometry::from_arrays(p, vec![0, 2, 3, 1, 2, 3], vec![0, 3, 6], None).unwrap());
    let params = Params::resolve(NODE.params, serde_json::json!({
        "consolidate_points": true,
        "consolidate_distance": 0.01,
        "remove_degenerate": false,
        "remove_unused_points": true,
    }).as_object()).unwrap();
    let out = clean(&[vec![g]], &params).unwrap();
    assert_eq!(out.npoints, 3, "points within consolidate_distance must be merged");
}

#[test]
fn test_attribute_pattern_removal() {
    let p = vec![
        0.0, 0.0, 0.0,
        1.0, 0.0, 0.0,
        0.0, 1.0, 0.0,
    ];
    let mut g = Geometry::from_arrays(p, vec![0, 1, 2], vec![0, 3], None).unwrap();
    g.point.insert("temp".into(), Arc::new(Attr::f32(1, vec![1.0, 2.0, 3.0])));
    g.point.insert("delete_me".into(), Arc::new(Attr::f32(1, vec![4.0, 5.0, 6.0])));
    g.point.insert("keep_me".into(), Arc::new(Attr::f32(1, vec![7.0, 8.0, 9.0])));
    let g = Arc::new(g);

    let params = Params::resolve(NODE.params, serde_json::json!({
        "remove_attributes": "temp delete_*",
        "remove_degenerate": false,
        "remove_unused_points": false,
    }).as_object()).unwrap();
    let out = clean(&[vec![g]], &params).unwrap();
    assert!(out.point.contains_key("P"), "P must never be deleted");
    assert!(out.point.contains_key("keep_me"), "keep_me must not be deleted");
    assert!(!out.point.contains_key("temp"), "temp must be deleted");
    assert!(!out.point.contains_key("delete_me"), "delete_me must be deleted");
}
