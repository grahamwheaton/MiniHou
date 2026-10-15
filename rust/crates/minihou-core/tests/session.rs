//! The incremental evaluator against a fresh cook of the same (edited) graph.

use std::sync::Arc;

use minihou_core::session::Session;
use minihou_core::{Geometry, Graph};
use serde_json::{json, Value};

fn chain() -> Value {
    json!({
        "nodes": {
            "g": {"type": "grid", "params": {"columns": 40, "rows": 30}},
            "n": {"type": "noise", "inputs": {"geo": "g"}, "params": {"amplitude": [0.0, 0.0, 1.0], "frequency": 1.5}},
            "t": {"type": "transform", "inputs": {"geo": "n"}, "params": {"translate": [1.0, 2.0, 3.0]}},
            "nm": {"type": "normal", "inputs": {"geo": "t"}},
            "o": {"type": "output", "inputs": {"geo": "nm"}},
        },
        "display": "o",
    })
}

fn same(a: &Geometry, b: &Geometry) {
    assert_eq!(a.npoints, b.npoints);
    assert_eq!(*a.topology, *b.topology);
    for (x, y) in [(&a.point, &b.point), (&a.vertex, &b.vertex), (&a.prim, &b.prim)] {
        assert_eq!(x.keys().collect::<Vec<_>>(), y.keys().collect::<Vec<_>>());
        for (name, attr) in x {
            assert_eq!(**attr, *y[name], "attribute {name}");
        }
    }
    assert_eq!(a.detail, b.detail);
}

/// Apply `edits` (node, param, value) to the graph JSON, as a fresh cook would read it.
fn edited(mut g: Value, edits: &[(&str, &str, Value)]) -> Value {
    for (node, name, value) in edits {
        let n = g["nodes"][node].as_object_mut().unwrap();
        let p = n.entry("params").or_insert_with(|| json!({}));
        p[name] = value.clone();
    }
    g
}

fn fresh(g: &Value) -> Arc<Geometry> {
    Graph::from_value(g).unwrap().cook(None).unwrap()
}

fn names(v: &[String]) -> Vec<&str> {
    v.iter().map(String::as_str).collect()
}

#[test]
fn a_session_cook_equals_a_fresh_cook_and_the_second_one_is_all_cache() {
    let g = chain();
    let mut s = Session::new(Graph::from_value(&g).unwrap());
    same(&s.cook(None).unwrap(), &fresh(&g));
    assert_eq!(names(&s.last_stats().cooked), ["g", "n", "t", "nm", "o"]);
    assert!(s.last_stats().cached.is_empty());
    same(&s.cook(None).unwrap(), &fresh(&g));
    assert!(s.last_stats().cooked.is_empty());
    assert_eq!(names(&s.last_stats().cached), ["o"]);
}

#[test]
fn an_edit_recooks_the_edited_node_and_what_reads_it() {
    let mut s = Session::new(Graph::from_value(&chain()).unwrap());
    s.cook(None).unwrap();
    s.set_param("n", "frequency", json!(3.0)).unwrap();
    let got = s.cook(None).unwrap();
    assert_eq!(names(&s.last_stats().cooked), ["n", "t", "nm", "o"]);
    assert_eq!(names(&s.last_stats().cached), ["g"]);
    same(&got, &fresh(&edited(chain(), &[("n", "frequency", json!(3.0))])));
    // editing the grid recooks everything
    s.set_param("g", "columns", json!(41)).unwrap();
    let got = s.cook(None).unwrap();
    assert_eq!(names(&s.last_stats().cooked), ["g", "n", "t", "nm", "o"]);
    same(&got, &fresh(&edited(chain(), &[("n", "frequency", json!(3.0)), ("g", "columns", json!(41))])));
}

#[test]
fn going_back_to_an_earlier_value_is_a_cache_hit() {
    let mut s = Session::new(Graph::from_value(&chain()).unwrap());
    s.cook(None).unwrap();
    s.set_param("n", "frequency", json!(3.0)).unwrap();
    s.cook(None).unwrap();
    s.set_param("n", "frequency", json!(1.5)).unwrap();
    let got = s.cook(None).unwrap();
    assert!(s.last_stats().cooked.is_empty(), "{:?}", s.last_stats());
    same(&got, &fresh(&chain()));
}

#[test]
fn bypass_is_part_of_the_key() {
    let mut s = Session::new(Graph::from_value(&chain()).unwrap());
    s.cook(None).unwrap();
    s.set_bypass("n", true).unwrap();
    let got = s.cook(None).unwrap();
    let mut g = chain();
    g["nodes"]["n"]["flags"] = json!({"bypass": true});
    same(&got, &fresh(&g));
    s.set_bypass("n", false).unwrap();
    same(&s.cook(None).unwrap(), &fresh(&chain()));
}

#[test]
fn errors_leave_the_cache_usable() {
    let mut s = Session::new(Graph::from_value(&chain()).unwrap());
    assert!(s.set_param("nope", "x", json!(1)).is_err());
    assert!(s.set_param("n", "nope", json!(1)).is_err());
    s.cook(None).unwrap();
    s.set_param("t", "proportional_enabled", json!(true)).unwrap();
    s.set_param("t", "proportional_falloff", json!("random")).unwrap(); // refused at cook time
    let e = s.cook(None).unwrap_err();
    assert!(e.0.contains("Rust backend"), "{e}");
    s.set_param("t", "proportional_enabled", json!(false)).unwrap();
    s.set_param("t", "proportional_falloff", json!("smooth")).unwrap();
    same(&s.cook(None).unwrap(), &fresh(&chain()));
}

#[test]
fn a_tiny_budget_still_gives_correct_cooks_and_keeps_the_cache_small() {
    let mut s = Session::new(Graph::from_value(&chain()).unwrap());
    s.set_budget_bytes(Some(1));
    let mut freq = 1.5;
    for _ in 0..3 {
        let got = s.cook(None).unwrap();
        same(&got, &fresh(&edited(chain(), &[("n", "frequency", json!(freq))])));
        assert_eq!(s.cache_bytes(), 0);
        freq += 1.0;
        s.set_param("n", "frequency", json!(freq)).unwrap();
    }
    s.set_budget_bytes(None);
    s.cook(None).unwrap();
    assert!(s.cache_bytes() > 0);
    s.clear();
    assert_eq!(s.cache_bytes(), 0);
}

#[test]
fn identical_nodes_share_one_cache_entry() {
    let g = json!({
        "nodes": {
            "a": {"type": "grid", "params": {"columns": 8, "rows": 8}},
            "b": {"type": "grid", "params": {"rows": 8, "columns": 8}},
            "m": {"type": "merge", "inputs": {"geos": ["a", "b"]}},
        },
        "display": "m",
    });
    let mut s = Session::new(Graph::from_value(&g).unwrap());
    same(&s.cook(None).unwrap(), &fresh(&g));
    // the same recipe written in another key order is one cook, the other one a cache hit
    assert_eq!(s.last_stats().cooked.iter().filter(|n| *n == "a" || *n == "b").count(), 1);
    assert_eq!(s.last_stats().cached.iter().filter(|n| *n == "a" || *n == "b").count(), 1);
}

#[test]
fn a_slider_drag_does_not_pile_up_versions() {
    let mut s = Session::new(Graph::from_value(&chain()).unwrap());
    s.cook(None).unwrap();
    let one = s.cache_bytes();
    for i in 0..20 {
        s.set_param("n", "frequency", json!(2.0 + i as f64 * 0.1)).unwrap();
        s.cook(None).unwrap();
    }
    // the grid once, the three downstream nodes twice (current + previous), not twenty times
    assert!(s.cache_bytes() < one * 2, "{} vs {}", s.cache_bytes(), one);
    // the previous value is still a cache hit
    s.set_param("n", "frequency", json!(2.0 + 18.0 * 0.1)).unwrap();
    s.cook(None).unwrap();
    assert!(s.last_stats().cooked.is_empty(), "{:?}", s.last_stats());
    // and one version per node when asked
    s.set_keep_versions(1);
    assert!(s.cache_bytes() <= one);
}
