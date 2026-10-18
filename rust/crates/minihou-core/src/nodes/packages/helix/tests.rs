use super::*;

#[test]
fn helix_preserves_curve_topology_and_end_radius() {
    let overrides = serde_json::json!({"radius": 2.0, "radius_end": 1.0, "height": 3.0, "turns": 1.0, "points": 5});
    let p = Params::resolve(NODE.params, overrides.as_object()).unwrap();
    let g = (NODE.cook)(&[], &p).unwrap();
    assert_eq!(g.npoints, 5);
    assert_eq!(g.topology.vertex_point, vec![0, 1, 2, 3, 4]);
    assert!(!g.topology.is_closed(0));
    assert_eq!(&g.p()[0..3], &[2.0, 0.0, 0.0]);
    assert!((g.p()[12] - 1.0).abs() < 1e-6);
    assert!(g.p()[13].abs() < 1e-6);
    assert_eq!(g.p()[14], 3.0);
    assert!(g.prim.contains_key("__viewport_curve"));
}
