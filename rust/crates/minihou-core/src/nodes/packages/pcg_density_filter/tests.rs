use super::*;
use std::sync::Arc;
#[test]
fn test_density_filter_keeps_range() {
    let mut g = Geometry::points_only(vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0]).unwrap();
    g.point.insert("density".into(), Arc::new(crate::geometry::Attr::f32(1, vec![0.2, 0.9])));
    let p = Params::resolve(NODE.params, None).unwrap();
    let out = (NODE.cook)(&[vec![Arc::new(g)]], &p).unwrap();
    assert_eq!(out.npoints, 1);
}
