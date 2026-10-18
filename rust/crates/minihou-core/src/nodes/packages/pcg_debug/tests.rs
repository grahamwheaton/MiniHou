use super::*;
#[test]
fn test_pcg_debug() {
    let pts = Geometry::points_only(vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 2.0, 0.0, 0.0]).unwrap();
    let p = Params::resolve(NODE.params, None).unwrap();
    let dbg = pcg_debug(&[vec![Arc::new(pts)]], &p).unwrap();
    assert_eq!(dbg.npoints, 3 * 8);
    assert_eq!(dbg.nprims(), 3 * 6);
}
