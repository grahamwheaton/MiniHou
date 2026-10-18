use super::*;
#[test]
fn test_pcg_spawner() {
    let pts = Geometry::points_only(vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 2.0, 0.0, 0.0]).unwrap();
    let p = Params::resolve(NODE.params, serde_json::json!({"objects": "Tree:3 Rock:1"}).as_object()).unwrap();
    let s = pcg_spawner(&[vec![Arc::new(pts)]], &p).unwrap();
    assert!(s.point.contains_key("instance"));
    assert!(s.point.contains_key("variant"));
    assert!(s.detail.contains_key("instance_objects"));
}
