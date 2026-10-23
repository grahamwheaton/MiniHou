use std::sync::Arc;

use crate::geometry::{Attr, Geometry};
use crate::params::Params;

use super::NODE;

fn cook(geometry: Geometry, settings: serde_json::Value) -> Geometry {
    let params = Params::resolve(NODE.params, settings.as_object()).unwrap();
    (NODE.cook)(&[vec![Arc::new(geometry)]], &params).unwrap()
}

#[test]
fn points_move_toward_nearest_other_point() {
    let geometry = Geometry::points_only(vec![0., 0., 0., 4., 0., 0., 10., 0., 0.]).unwrap();
    let output = cook(
        geometry,
        serde_json::json!({"strength": 0.5, "iterations": 1}),
    );
    assert_eq!(output.p(), &[2., 0., 0., 2., 0., 0., 7., 0., 0.]);
}

#[test]
fn point_at_target_does_not_move() {
    let mut geometry = Geometry::points_only(vec![0., 0., 0., 0., 0., 0., 4., 0., 0.]).unwrap();
    geometry
        .point
        .insert("group_target".into(), Arc::new(Attr::i32(1, vec![1, 0, 0])));
    let output = cook(
        geometry,
        serde_json::json!({"group": "target", "strength": 0.5, "iterations": 2}),
    );
    assert_eq!(output.p(), &[0., 0., 0., 0., 0., 0., 1., 0., 0.]);
}

#[test]
fn empty_group_leaves_points_unchanged() {
    let mut geometry = Geometry::points_only(vec![0., 0., 0., 4., 0., 0.]).unwrap();
    geometry
        .point
        .insert("group_empty".into(), Arc::new(Attr::i32(1, vec![0, 0])));
    let output = cook(
        geometry,
        serde_json::json!({"group": "empty", "strength": 1.0}),
    );
    assert_eq!(output.p(), &[0., 0., 0., 4., 0., 0.]);
}

#[test]
fn primitive_group_uses_its_points_as_targets() {
    let mut geometry = Geometry::from_arrays(
        vec![0., 0., 0., 2., 0., 0., 6., 0., 0.],
        vec![0, 1, 1, 2],
        vec![0, 2, 4],
        Some(vec![false, false]),
    )
    .unwrap();
    geometry
        .prim
        .insert("group_target".into(), Arc::new(Attr::i32(1, vec![1, 0])));
    let output = cook(
        geometry,
        serde_json::json!({"group": "target", "domain": "primitives", "strength": 0.5}),
    );
    assert_eq!(output.p(), &[0., 0., 0., 2., 0., 0., 4., 0., 0.]);
}
