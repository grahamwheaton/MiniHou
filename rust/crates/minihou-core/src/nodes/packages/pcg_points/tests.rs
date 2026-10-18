use super::*;
use std::sync::Arc;
fn sample_points() -> Geometry {
    Geometry::points_only(vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 2.0, 0.0, 0.0]).unwrap()
}
#[test]
fn test_pcg_points_fills_attributes() {
    let p = Params::resolve(NODE.params, None).unwrap();
    let out = (NODE.cook)(&[vec![Arc::new(sample_points())]], &p).unwrap();
    assert_eq!(out.npoints, 3);
    for k in ["rot", "scale", "bounds_min", "bounds_max", "density", "seed"] {
        assert!(out.point.contains_key(k));
    }
}
