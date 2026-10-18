use super::*;
#[test]
fn reverse_keeps_corner_rows() {
    let mut g = Geometry::from_arrays(vec![0.,0.,0., 1.,0.,0., 1.,1.,0., 0.,1.,0.], vec![0,1,2,3], vec![0,4], None).unwrap();
    g.vertex.insert("id".into(), Arc::new(crate::geometry::Attr::i32(1, vec![10,11,12,13])));
    let p = Params::resolve(NODE.params, None).unwrap();
    let out = (NODE.cook)(&[vec![Arc::new(g)]], &p).unwrap();
    assert_eq!(out.topology.vertex_point, vec![3,2,1,0]);
    assert_eq!(out.vertex["id"].data, AttrData::I32(vec![13,12,11,10]));
}
