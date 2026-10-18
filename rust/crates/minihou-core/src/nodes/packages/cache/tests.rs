use super::*;
use crate::{graph::Graph, session::Session};
use serde_json::json;
use std::sync::Arc;

fn session() -> Session {
    Session::new(Graph::from_value(&json!({"nodes": {
        "source": {"type": "box", "params": {"size": [1, 1, 1]}},
        "held": {"type": "cache", "inputs": {"geo": "source"}}
    }, "display": "held"})).unwrap())
}

#[test]
fn pass_through_identity_and_off_mode() {
    let mut s = session();
    let output = s.cook(None).unwrap();
    assert!(Arc::ptr_eq(&output, &s.cached_geometry("source").unwrap()));
    s.set_param("held", "mode", json!("off")).unwrap();
    let output = s.cook(None).unwrap();
    assert!(Arc::ptr_eq(&output, &s.cached_geometry("source").unwrap()));
    assert!(!s.last_stats().cooked.contains(&"source".into()));
    s.cook(None).unwrap();
    assert_eq!(s.last_stats().cooked, ["held"]);
    assert!(!s.last_stats().cached.contains(&"held".into()));
    assert!(cook(&[vec![]], &Params::resolve(NODE.params, None).unwrap()).is_err());
}

#[test]
fn unchanged_upstream_hits_without_recooking_it() {
    let mut s = session();
    s.cook(None).unwrap();
    s.cook(None).unwrap();
    assert!(s.last_stats().cooked.is_empty());
    assert_eq!(s.last_stats().cached, ["held"]);
    assert_eq!(s.last_stats().nodes.len(), 1);
    assert!(s.last_stats().nodes[0].cache_hit);
}

#[test]
fn upstream_parameter_change_misses() {
    let mut s = session();
    s.cook(None).unwrap();
    s.set_param("source", "size", json!([2, 1, 1])).unwrap();
    s.cook(None).unwrap();
    assert_eq!(s.last_stats().cooked, ["source", "held"]);
    assert!(s.last_stats().nodes.iter().all(|d| !d.cache_hit));
}

#[test]
fn diagnostics_report_counts_for_cooks_and_hits() {
    let mut s = session();
    let geo = s.cook(None).unwrap();
    for d in &s.last_stats().nodes {
        assert!(!d.cache_hit);
        assert!(d.cook_ms >= 0.0);
        assert_eq!((d.point_count, d.primitive_count), (geo.npoints, geo.nprims()));
    }
    assert_eq!(s.last_stats().nodes.iter().map(|d| d.node_id.as_str()).collect::<Vec<_>>(), ["source", "held"]);
    s.cook(None).unwrap();
    let d = &s.last_stats().nodes[0];
    assert_eq!(d.node_id, "held");
    assert!(d.cache_hit);
    assert!(d.cook_ms >= 0.0);
    assert_eq!((d.point_count, d.primitive_count), (geo.npoints, geo.nprims()));
}
