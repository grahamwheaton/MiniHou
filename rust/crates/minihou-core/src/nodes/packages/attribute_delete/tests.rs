use super::*;
use crate::geometry::Attr;
use std::sync::Arc;

#[test]
fn wildcard_and_protected_position() {
    assert!(matches("tmp_[!a-c]?", "tmp_z0"));
    assert!(!matches("tmp_[!a-c]?", "tmp_b0"));
    assert!(matches("[", "["));
    let mut g = Geometry::points_only(vec![0.; 3]).unwrap();
    g.point.insert("N".into(), Arc::new(Attr::f32(3, vec![1.; 3])));
    g.detail.insert("P".into(), serde_json::json!(9));
    let out = delete(&[vec![Arc::new(g)]], &Params::resolve(NODE.params,
        serde_json::json!({"names":"*"}).as_object()).unwrap()).unwrap();
    assert_eq!(out.point.len(), 1);
    assert!(out.point.contains_key("P"));
    assert!(out.detail.is_empty());
}
