use super::*;
#[test]
fn facet_unique_points() {
    let g = Geometry::from_arrays(vec![0.,0.,0., 1.,0.,0., 1.,1.,0., 0.,1.,0.], vec![0,1,2, 0,2,3], vec![0,3,6], None).unwrap();
    let p = Params::resolve(NODE.params, None).unwrap();
    let out = (NODE.cook)(&[vec![Arc::new(g)]], &p).unwrap();
    assert_eq!(out.npoints, 6);
    assert_eq!(out.topology.vertex_point, vec![0,1,2,3,4,5]);
}
