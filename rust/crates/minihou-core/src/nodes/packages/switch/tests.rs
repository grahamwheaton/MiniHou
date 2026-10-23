use super::*;
use crate::Graph;
use serde_json::json;

#[test]
fn package_registers_with_defaults() {
    let spec = crate::nodes::get("switch").unwrap();
    assert_eq!(spec.name, NODE.name);
    Params::resolve(NODE.params, None).unwrap();
}

#[test]
fn test_switch_selects_and_clamps() {
    let g0 = Geometry::points_only(vec![0.0, 0.0, 0.0]).unwrap();
    let g1 = Geometry::points_only(vec![1.0, 0.0, 0.0, 2.0, 0.0, 0.0]).unwrap();
    let inputs = vec![vec![Arc::new(g0), Arc::new(g1)]];

    let p0 = Params::resolve(NODE.params, json!({"index": 0}).as_object()).unwrap();
    let out0 = (NODE.cook)(&inputs, &p0).unwrap();
    assert_eq!(out0.npoints, 1);

    let p1 = Params::resolve(NODE.params, json!({"index": 1}).as_object()).unwrap();
    let out1 = (NODE.cook)(&inputs, &p1).unwrap();
    assert_eq!(out1.npoints, 2);

    let p_clamp = Params::resolve(NODE.params, json!({"index": 99}).as_object()).unwrap();
    let out_clamp = (NODE.cook)(&inputs, &p_clamp).unwrap();
    assert_eq!(out_clamp.npoints, 2);
}

#[test]
fn multiple_input_slots_selected_by_index() {
    let g0 = Geometry::points_only(vec![0.0, 0.0, 0.0]).unwrap();
    let g1 = Geometry::points_only(vec![1.0, 0.0, 0.0, 2.0, 0.0, 0.0]).unwrap();
    let g2 = Geometry::points_only(vec![1.0, 1.0, 1.0, 2.0, 2.0, 2.0, 3.0, 3.0, 3.0]).unwrap();
    let inputs = vec![
        vec![Arc::new(g0)],
        vec![Arc::new(g1)],
        vec![Arc::new(g2)],
    ];

    let p0 = Params::resolve(NODE.params, json!({"index": 0}).as_object()).unwrap();
    assert_eq!((NODE.cook)(&inputs, &p0).unwrap().npoints, 1);

    let p1 = Params::resolve(NODE.params, json!({"index": 1}).as_object()).unwrap();
    assert_eq!((NODE.cook)(&inputs, &p1).unwrap().npoints, 2);

    let p2 = Params::resolve(NODE.params, json!({"index": 2}).as_object()).unwrap();
    assert_eq!((NODE.cook)(&inputs, &p2).unwrap().npoints, 3);

    let p_clamp = Params::resolve(NODE.params, json!({"index": 99}).as_object()).unwrap();
    assert_eq!((NODE.cook)(&inputs, &p_clamp).unwrap().npoints, 3);

    let p_neg = Params::resolve(NODE.params, json!({"index": -1}).as_object()).unwrap();
    assert_eq!((NODE.cook)(&inputs, &p_neg).unwrap().npoints, 1);
}

#[test]
fn empty_inputs_error() {
    let inputs: Vec<Vec<Arc<Geometry>>> = vec![vec![], vec![]];
    let p = Params::resolve(NODE.params, None).unwrap();
    let err = (NODE.cook)(&inputs, &p).unwrap_err();
    assert!(err.0.contains("connect at least one input"));
}

#[test]
fn switch_graph_cook_multi_slot_and_single_input() {
    let graph = json!({
        "nodes": {
            "b": {"type": "box"},
            "s": {"type": "sphere"},
            "sw": {"type": "switch", "inputs": {"input0": "b", "input1": "s"}, "params": {"index": 1}},
        },
        "display": "sw"
    });
    let cooked = Graph::from_value(&graph).unwrap().cook(None).unwrap();
    assert_eq!(cooked.npoints, 482);

    let graph_legacy = json!({
        "nodes": {
            "b": {"type": "box"},
            "s": {"type": "sphere"},
            "sw": {"type": "switch", "inputs": {"geos": ["b", "s"]}, "params": {"index": 0}},
        },
        "display": "sw"
    });
    let cooked_legacy = Graph::from_value(&graph_legacy).unwrap().cook(None).unwrap();
    assert_eq!(cooked_legacy.npoints, 8);
}

#[test]
fn legacy_select_value_is_ignored() {
    let g0 = Geometry::points_only(vec![0.0, 0.0, 0.0]).unwrap();
    let g1 = Geometry::points_only(vec![1.0, 0.0, 0.0, 2.0, 0.0, 0.0]).unwrap();
    let inputs = vec![vec![Arc::new(g0)], vec![Arc::new(g1)]];
    let p = Params::resolve_node("switch", NODE.params, json!({"index": 1, "select": 0}).as_object()).unwrap();
    assert_eq!((NODE.cook)(&inputs, &p).unwrap().npoints, 2);
    let p = Params::resolve_node("switch", NODE.params, json!({"select": 1}).as_object()).unwrap();
    assert_eq!((NODE.cook)(&inputs, &p).unwrap().npoints, 1);
}
