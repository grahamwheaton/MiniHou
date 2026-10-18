use super::*;
#[test]
fn line_positions_and_topology() {
    let p = Params::resolve(NODE.params, None).unwrap();
    let g = (NODE.cook)(&[], &p).unwrap();
    assert_eq!(g.npoints, 10);
    assert_eq!(g.nprims(), 1);
    assert_eq!(&g.p()[0..3], &[0.0, 0.0, 0.0]);
    assert_eq!(&g.p()[27..30], &[0.0, 0.0, 1.0]);
    assert!(!g.topology.is_closed(0));
}
