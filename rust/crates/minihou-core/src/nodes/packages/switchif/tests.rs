use std::sync::Arc;
use serde_json::json;
use crate::{
    geometry::Geometry,
    graph::Graph,
    params::Params,
};
use super::*;

fn make_point_geo(count: usize) -> Arc<Geometry> {
    let mut coords = Vec::with_capacity(count * 3);
    for i in 0..count {
        coords.push(i as f32);
        coords.push(0.0);
        coords.push(0.0);
    }
    Arc::new(Geometry::points_only(coords).unwrap())
}

#[test]
fn test_registration_and_defaults() {
    let spec = crate::nodes::get("switchif").expect("switchif should be registered");
    assert_eq!(spec.name, "switchif");
    let p = Params::resolve(spec.params, None).unwrap();
    assert_eq!(p.s("use_second_input_if").unwrap(), "all_conditions_true");
    assert_eq!(p.s("test_input").unwrap(), "first_input");
    assert_eq!(p.i("number_of_tests").unwrap(), 2);
    assert_eq!(p.b("enable").unwrap(), true);
    assert_eq!(p.s("enable_value").unwrap(), "1");
}

#[test]
fn test_default_selects_second_input() {
    let g0 = make_point_geo(1);
    let g1 = make_point_geo(2);
    let p = Params::resolve(NODE.params, None).unwrap();

    let out = cook(&[vec![g0], vec![g1]], &p).unwrap();
    assert_eq!(out.npoints, 2);
}

#[test]
fn test_false_condition_selects_first_input() {
    let g0 = make_point_geo(1);
    let g1 = make_point_geo(2);
    let p = Params::resolve(NODE.params, json!({ "enable_value": "0" }).as_object()).unwrap();

    let out = cook(&[vec![g0], vec![g1]], &p).unwrap();
    assert_eq!(out.npoints, 1);
}

#[test]
fn test_condition_rules() {
    let g0 = make_point_geo(1);
    let g1 = make_point_geo(2);

    // any_condition_true: one true, one false -> true (selects second)
    let p = Params::resolve(NODE.params, json!({
        "use_second_input_if": "any_condition_true",
        "tests": [
            { "enable": true, "enable_value": "0" },
            { "enable": true, "enable_value": "1" }
        ]
    }).as_object()).unwrap();
    let out = cook(&[vec![g0.clone()], vec![g1.clone()]], &p).unwrap();
    assert_eq!(out.npoints, 2);

    // any_condition_true: both false -> false (selects first)
    let p = Params::resolve(NODE.params, json!({
        "use_second_input_if": "any_condition_true",
        "tests": [
            { "enable": true, "enable_value": "0" },
            { "enable": true, "enable_value": "0" }
        ]
    }).as_object()).unwrap();
    let out = cook(&[vec![g0.clone()], vec![g1.clone()]], &p).unwrap();
    assert_eq!(out.npoints, 1);

    // all_conditions_false: both false -> true (selects second)
    let p = Params::resolve(NODE.params, json!({
        "use_second_input_if": "all_conditions_false",
        "tests": [
            { "enable": true, "enable_value": "0" },
            { "enable": true, "enable_value": "0" }
        ]
    }).as_object()).unwrap();
    let out = cook(&[vec![g0.clone()], vec![g1.clone()]], &p).unwrap();
    assert_eq!(out.npoints, 2);

    // all_conditions_false: one true -> false (selects first)
    let p = Params::resolve(NODE.params, json!({
        "use_second_input_if": "all_conditions_false",
        "tests": [
            { "enable": true, "enable_value": "1" },
            { "enable": true, "enable_value": "0" }
        ]
    }).as_object()).unwrap();
    let out = cook(&[vec![g0.clone()], vec![g1.clone()]], &p).unwrap();
    assert_eq!(out.npoints, 1);

    // any_condition_false: one false -> true (selects second)
    let p = Params::resolve(NODE.params, json!({
        "use_second_input_if": "any_condition_false",
        "tests": [
            { "enable": true, "enable_value": "1" },
            { "enable": true, "enable_value": "0" }
        ]
    }).as_object()).unwrap();
    let out = cook(&[vec![g0], vec![g1]], &p).unwrap();
    assert_eq!(out.npoints, 2);
}

#[test]
fn test_test_input_geometry() {
    let g0 = make_point_geo(10);
    let g1 = make_point_geo(2);

    // Testing first_input: @npoints is 10 > 5 -> true -> second input
    let p = Params::resolve(NODE.params, json!({
        "test_input": "first_input",
        "number_of_tests": 1,
        "enable_value": "@npoints > 5"
    }).as_object()).unwrap();
    let out = cook(&[vec![g0.clone()], vec![g1.clone()]], &p).unwrap();
    assert_eq!(out.npoints, 2);

    // Testing second_input: @npoints is 2 > 5 -> false -> first input
    let p = Params::resolve(NODE.params, json!({
        "test_input": "second_input",
        "number_of_tests": 1,
        "enable_value": "@npoints > 5"
    }).as_object()).unwrap();
    let out = cook(&[vec![g0], vec![g1]], &p).unwrap();
    assert_eq!(out.npoints, 10);
}

#[test]
fn test_enable_checkbox_toggles() {
    let g0 = make_point_geo(1);
    let g1 = make_point_geo(2);

    // Disabled test with 0 is skipped, enabled test with 1 makes all active tests true -> second input
    let p = Params::resolve(NODE.params, json!({
        "tests": [
            { "enable": false, "enable_value": "0" },
            { "enable": true, "enable_value": "1" }
        ]
    }).as_object()).unwrap();
    let out = cook(&[vec![g0], vec![g1]], &p).unwrap();
    assert_eq!(out.npoints, 2);
}

#[test]
fn test_all_enabled_rows_must_pass_and_count_limits_rows() {
    let g0 = make_point_geo(1);
    let g1 = make_point_geo(2);
    let rows = json!([
        { "enable": true, "test_type": "expression", "enable_value": "1" },
        { "enable": true, "test_type": "expression", "enable_value": "0" }
    ]);
    let p = Params::resolve(NODE.params, json!({ "tests": rows }).as_object()).unwrap();
    assert_eq!(cook(&[vec![g0.clone()], vec![g1.clone()]], &p).unwrap().npoints, 1);

    let p = Params::resolve(NODE.params, json!({ "number_of_tests": 1, "tests": rows }).as_object()).unwrap();
    assert_eq!(cook(&[vec![g0], vec![g1]], &p).unwrap().npoints, 2);
}

#[test]
fn test_unknown_test_type_is_rejected() {
    let p = Params::resolve(NODE.params, json!({
        "number_of_tests": 1,
        "tests": [{ "enable": true, "test_type": "unsupported", "enable_value": "1" }]
    }).as_object()).unwrap();
    let err = cook(&[vec![make_point_geo(1)], vec![make_point_geo(2)]], &p).unwrap_err();
    assert!(err.to_string().contains("unsupported test type"));
}

#[test]
fn test_time_expressions() {
    let g0 = make_point_geo(1);
    let g1 = make_point_geo(2);

    let p = Params::resolve(NODE.params, json!({
        "number_of_tests": 1,
        "enable_value": "@Frame == 10"
    }).as_object()).unwrap();

    let out_at_10 = crate::timectx::at(Some(10.0), None, || {
        cook(&[vec![g0.clone()], vec![g1.clone()]], &p).unwrap()
    });
    assert_eq!(out_at_10.npoints, 2);

    let out_at_5 = crate::timectx::at(Some(5.0), None, || {
        cook(&[vec![g0], vec![g1]], &p).unwrap()
    });
    assert_eq!(out_at_5.npoints, 1);
}

#[test]
fn test_missing_input_errors() {
    let g = make_point_geo(1);
    let p_default = Params::resolve(NODE.params, None).unwrap();
    let p_false = Params::resolve(NODE.params, json!({ "enable_value": "0" }).as_object()).unwrap();

    // Neither input connected
    let err = cook(&[], &p_default).unwrap_err();
    assert!(err.to_string().contains("connect at least one input"));

    // Condition selects second, but second missing
    let err = cook(&[vec![g.clone()]], &p_default).unwrap_err();
    assert!(err.to_string().contains("second input is not connected"));

    // Condition selects first, but first missing
    let err = cook(&[vec![], vec![g]], &p_false).unwrap_err();
    assert!(err.to_string().contains("first input is not connected"));
}

#[test]
fn test_graph_cook_switchif() {
    let graph_json = r#"{
        "nodes": {
            "box1": { "type": "box", "params": { "size": [1.0, 1.0, 1.0] } },
            "grid1": { "type": "grid", "params": { "size_x": 2.0, "size_y": 2.0, "rows": 2, "columns": 2 } },
            "sw": {
                "type": "switchif",
                "inputs": {
                    "input0": "box1",
                    "input1": "grid1"
                },
                "params": {
                    "number_of_tests": 1,
                    "enable_value": "0"
                }
            },
            "out": {
                "type": "output",
                "inputs": { "geo": "sw" }
            }
        }
    }"#;
    let graph = Graph::from_json(graph_json).unwrap();
    let res = graph.cook(None).unwrap();
    // With enable_value 0, first input (box1) is selected: 8 points
    assert_eq!(res.npoints, 8);
}
