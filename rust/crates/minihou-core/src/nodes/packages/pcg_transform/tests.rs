use super::*;
use std::sync::Arc;
#[test]
fn test_pcg_transform_deterministic() {
    let pts = Geometry::points_only(vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 2.0, 0.0, 0.0]).unwrap();
    let p = Params::resolve(NODE.params, serde_json::json!({"seed": 42, "scale_min": [2.0, 2.0, 2.0]}).as_object()).unwrap();
    let a = (NODE.cook)(&[vec![Arc::new(pts.clone())]], &p).unwrap();
    let b = (NODE.cook)(&[vec![Arc::new(pts)]], &p).unwrap();
    assert_eq!(a.p(), b.p());
}
