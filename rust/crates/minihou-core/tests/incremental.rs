//! Incremental cooking and shared geometry: cook counts per node for the Houdini-style chain
//! Grid -> Scatter -> Copy to Points -> Transform, plus structural edits, cancellation and copy-free sharing.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

use minihou_core::session::{Session, SUPERSEDED};
use minihou_core::Graph;
use serde_json::{json, Value};

fn chain() -> Value {
    json!({
        "nodes": {
            "grid": {"type": "grid", "params": {"columns": 60, "rows": 60}},
            "scatter": {"type": "scatter", "inputs": {"geo": "grid"}, "params": {"count": 2000, "seed": 3}},
            "shape": {"type": "box"},
            "copy": {"type": "copy_to_points", "inputs": {"source": "shape", "points": "scatter"}},
            "xform": {"type": "transform", "inputs": {"geo": "copy"}, "params": {"translate": [0.0, 1.0, 0.0]}},
        },
        "display": "xform",
    })
}

fn session() -> Session {
    Session::new(Graph::from_value(&chain()).unwrap())
}

fn cooked(s: &Session) -> Vec<&str> {
    let mut v: Vec<&str> = s.last_stats().cooked.iter().map(String::as_str).collect();
    v.sort();
    v
}

#[test]
fn downstream_edit_does_not_recook_upstream() {
    let mut s = session();
    s.cook(None).unwrap();
    assert_eq!(cooked(&s), ["copy", "grid", "scatter", "shape", "xform"]);
    s.set_param("xform", "translate", json!([0.0, 2.0, 0.0])).unwrap();
    s.cook(None).unwrap();
    assert_eq!(cooked(&s), ["xform"]);
    assert!(s.last_stats().cached.contains(&"copy".to_string()));
}

#[test]
fn upstream_edit_recooks_every_dependent_and_nothing_else() {
    let mut s = session();
    s.cook(None).unwrap();
    s.set_param("grid", "columns", json!(61)).unwrap();
    s.cook(None).unwrap();
    assert_eq!(cooked(&s), ["copy", "grid", "scatter", "xform"]);
}

#[test]
fn unchanged_values_and_repeat_cooks_cook_nothing() {
    let mut s = session();
    s.cook(None).unwrap();
    s.cook(None).unwrap();
    assert!(cooked(&s).is_empty());
    // Re-setting a parameter to its current value (what a UI refresh does) is not a change.
    s.set_param("grid", "columns", json!(60)).unwrap();
    s.cook(None).unwrap();
    assert!(cooked(&s).is_empty());
    // Changing what is displayed reads the cache.
    s.cook(Some("scatter")).unwrap();
    assert!(cooked(&s).is_empty());
}

#[test]
fn structural_edits_and_undo_keep_the_cache() {
    let mut s = session();
    s.cook(None).unwrap();
    let mut g = chain();
    g["nodes"]["extra"] = json!({"type": "transform", "inputs": {"geo": "xform"}});
    g["display"] = json!("extra");
    s.replace_graph(Graph::from_value(&g).unwrap());
    s.cook(None).unwrap();
    assert_eq!(cooked(&s), ["extra"]);
    // undo: the original document comes back as a whole new graph
    s.replace_graph(Graph::from_value(&chain()).unwrap());
    s.cook(None).unwrap();
    assert!(cooked(&s).is_empty());
    // a removed node's entries are dropped with it
    let before = s.cache_len();
    let mut g = chain();
    g["nodes"].as_object_mut().unwrap().remove("xform");
    g["display"] = json!("copy");
    s.replace_graph(Graph::from_value(&g).unwrap());
    assert_eq!(s.cache_len(), before - 1);
}

#[test]
fn a_superseded_cook_stops_and_the_next_one_resumes() {
    let mut s = session();
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    // cancel at the fifth node that has to run (asked in pre-order: xform, copy, shape, scatter, grid)
    s.set_cancel(Some(Arc::new(move || c.fetch_add(1, Ordering::SeqCst) >= 4)));
    let err = s.cook(None).unwrap_err().to_string();
    assert!(err.contains(SUPERSEDED), "{err}");
    let first = s.last_stats().cooked.len();
    assert!(first > 0 && first < 5, "{first}");
    s.set_cancel(None);
    let fresh = Graph::from_value(&chain()).unwrap().cook(None).unwrap();
    let geo = s.cook(None).unwrap();
    assert_eq!(geo.npoints, fresh.npoints);
    assert_eq!(**geo.point.get("P").unwrap(), **fresh.point.get("P").unwrap());
    // the nodes finished before the cancel were not cooked again
    assert_eq!(s.last_stats().cooked.len(), 5 - first);
}

#[test]
fn branches_share_the_upstream_result_without_copying() {
    let g = json!({"nodes": {
        "grid": {"type": "grid", "params": {"columns": 50, "rows": 50}},
        "a": {"type": "transform", "inputs": {"geo": "grid"}, "params": {"translate": [1.0, 0.0, 0.0]}},
        "b": {"type": "transform", "inputs": {"geo": "grid"}, "params": {"translate": [0.0, 1.0, 0.0]}},
        "pass": {"type": "transform", "inputs": {"geo": "grid"}, "flags": {"bypass": true}},
        "m": {"type": "merge", "inputs": {"geos": ["a", "b", "pass"]}},
    }, "display": "m"});
    let mut s = Session::new(Graph::from_value(&g).unwrap());
    s.cook(None).unwrap();
    assert_eq!(s.last_stats().cooked.iter().filter(|n| *n == "grid").count(), 1);
    let grid = s.cached_geometry("grid").unwrap();
    let (a, b) = (s.cached_geometry("a").unwrap(), s.cached_geometry("b").unwrap());
    // the branches changed P only: topology and every other array are the upstream ones, shared
    assert!(Arc::ptr_eq(&grid.topology, &a.topology) && Arc::ptr_eq(&grid.topology, &b.topology));
    for (name, attr) in grid.point.iter().chain(&grid.vertex).chain(&grid.prim) {
        if name == "P" || name == "N" { continue; }
        let same = |g: &minihou_core::Geometry| [&g.point, &g.vertex, &g.prim].iter().any(|t| t.get(name).is_some_and(|x| Arc::ptr_eq(x, attr)));
        assert!(same(&a) && same(&b), "{name} was copied");
    }
    // the upstream result itself is untouched (immutable) and a bypass is the very same geometry
    assert!(!Arc::ptr_eq(grid.point.get("P").unwrap(), a.point.get("P").unwrap()));
    assert!(Arc::ptr_eq(&grid, &s.cached_geometry("pass").unwrap()));
}

/// Not an assertion on speed: prints the cook time of the chain before and after a downstream change.
#[test]
fn bench_downstream_change_timing() {
    let mut s = session();
    let t = Instant::now();
    s.cook(None).unwrap();
    let full = t.elapsed().as_secs_f64() * 1000.0;
    s.set_param("xform", "translate", json!([0.0, 3.0, 0.0])).unwrap();
    let t = Instant::now();
    s.cook(None).unwrap();
    let incremental = t.elapsed().as_secs_f64() * 1000.0;
    s.replace_graph(Graph::from_value(&chain()).unwrap());
    let t = Instant::now();
    s.cook(None).unwrap();
    let undo = t.elapsed().as_secs_f64() * 1000.0;
    println!("BENCH full {full:.2} ms, downstream edit {incremental:.2} ms, undo/replace {undo:.2} ms");
}
