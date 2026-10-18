use super::*;

#[test]
fn one_point_ring() {
    let mut g=Geometry::from_arrays(vec![0.;12],vec![0,1,2,3],vec![0,4],Some(vec![false])).unwrap();
    g.point.insert("group_tip".into(),Arc::new(Attr::i32(1,vec![1,0,0,0])));
    let p=Params::resolve(NODE.params,serde_json::json!({"domain":"points","group":"tip"}).as_object()).unwrap();
    let out=(NODE.cook)(&[vec![Arc::new(g)]],&p).unwrap();
    assert_eq!(out.point["group_tip"].data,crate::geometry::AttrData::I32(vec![1,1,0,0]));
}
