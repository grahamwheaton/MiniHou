use minihou_core::{Graph, lod::ClusterLod, session::Session};
use serde_json::json;

fn graph(params: serde_json::Value) -> Graph {
    Graph::from_value(&json!({"nodes": {
        "g": {"type":"grid", "params":{"columns":24,"rows":24}},
        "b": {"type":"lod_bake", "inputs":{"geo":"g"}, "params":params},
        "o": {"type":"output", "inputs":{"geo":"b"}}
    }, "display":"o"})).unwrap()
}

#[test]
fn pass_through_payload_cache_and_invalidation() {
    let mut session = Session::new(graph(json!({"min_triangles":0})));
    let expected = session.graph().cook(Some("g")).unwrap();
    let output = session.cook(None).unwrap();
    assert_eq!(output.npoints, expected.npoints);
    assert_eq!(*output.topology, *expected.topology);
    assert_eq!(output.point, expected.point);
    assert_eq!(output.vertex, expected.vertex);
    assert_eq!(output.prim, expected.prim);
    assert_eq!(output.detail, expected.detail);
    let first = session.side_payload::<ClusterLod>("b").unwrap();
    assert!(!first.clusters.is_empty());
    assert!(session.cache_bytes() > minihou_core::session::geometry_bytes(&output));
    session.cook(None).unwrap();
    assert!(session.last_stats().cooked.is_empty());
    assert!(std::sync::Arc::ptr_eq(&first, &session.side_payload::<ClusterLod>("b").unwrap()));
    session.set_param("g", "columns", json!(25)).unwrap();
    assert!(session.side_payload::<ClusterLod>("b").is_none());
    session.cook(None).unwrap();
    assert!(session.last_stats().cooked.contains(&"b".to_owned()));
    assert!(!std::sync::Arc::ptr_eq(&first, &session.side_payload::<ClusterLod>("b").unwrap()));
    session.clear();
    assert!(session.side_payload::<ClusterLod>("b").is_none());
    assert_eq!(session.cache_bytes(), 0);
    session.cook(None).unwrap();
    session.set_budget_bytes(Some(1));
    assert!(session.side_payload::<ClusterLod>("b").is_none());
    assert_eq!(session.cache_bytes(), 0);
}

#[test]
fn skips_and_parameter_errors() {
    for params in [json!({}), json!({"enabled":false,"min_triangles":0})] {
        let mut session = Session::new(graph(params));
        session.cook(None).unwrap();
        assert!(session.side_payload::<ClusterLod>("b").is_none());
    }
    let mut session = Session::new(graph(json!({"min_triangles":0})));
    session.set_bypass("b", true).unwrap();
    session.cook(None).unwrap();
    assert!(session.side_payload::<ClusterLod>("b").is_none());
    for params in [json!({"cluster_size":4}), json!({"error_px":0}), json!({"min_triangles":-1})] {
        let g=graph(params);
        assert!(g.cook(None).is_err());
        let mut session = Session::new(g);
        let error = session.cook(None).unwrap_err().to_string();
        assert!(error.contains("lod_bake"), "{error}");
    }
}
