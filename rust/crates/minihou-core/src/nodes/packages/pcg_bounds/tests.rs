use super::*;
use std::sync::Arc;
#[test]
fn test_pcg_bounds_scale() {
    let pts = Geometry::points_only(vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0]).unwrap();
    let pb = Params::resolve(NODE.params, serde_json::json!({"mode": "scale", "factor": 2.0}).as_object()).unwrap();
    let scaled = (NODE.cook)(&[vec![Arc::new(pts)]], &pb).unwrap();
    assert_eq!(scaled.point["bounds_min"].as_f32().unwrap()[0], -1.0);
}
