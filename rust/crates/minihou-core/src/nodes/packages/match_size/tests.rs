use super::*;
use serde_json::{json, Map, Value};
use std::sync::Arc;

fn cook(points: Vec<f32>, settings: &[(&str, Value)]) -> Geometry {
    let geo = Geometry::points_only(points).unwrap();
    let given: Map<String, Value> = settings.iter().map(|(k, v)| ((*k).into(), v.clone())).collect();
    let params = Params::resolve(NODE.params, Some(&given)).unwrap();
    (NODE.cook)(&[vec![Arc::new(geo)], vec![]], &params).unwrap()
}

#[test]
fn moves_box_to_target_centre() {
    let out = cook(vec![-1., -1., -1., 1., 1., 1.], &[("target_position", json!([4., 5., 6.]))]);
    assert_eq!(out.p(), &[3., 4., 5., 5., 6., 7.]);
}

#[test]
fn scales_to_fit_target_size() {
    let out = cook(vec![-1., -1., -1., 1., 1., 1.], &[("scale_to_fit", json!(true)), ("target_size", json!([4., 6., 8.]))]);
    assert_eq!(out.p(), &[-2., -3., -4., 2., 3., 4.]);
}

#[test]
fn justifies_min_corner() {
    let out = cook(vec![2., 3., 4., 4., 5., 6.], &[
        ("target_position", json!([10., 10., 10.])),
        ("target_size", json!([4., 4., 4.])),
        ("justify_x", json!("min")), ("justify_y", json!("min")), ("justify_z", json!("min")),
    ]);
    assert_eq!(out.p(), &[8., 8., 8., 10., 10., 10.]);
}
