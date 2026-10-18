use super::*;

#[test]
fn wildcard_promotes_point_group() {
    let mut g=Geometry::from_arrays(vec![0.;12],vec![0,1,2,3],vec![0,4],Some(vec![false])).unwrap();
    g.point.insert("group_tip".into(),Arc::new(Attr::i32(1,vec![1,1,0,0])));
    let p=Params::resolve(NODE.params,serde_json::json!({"name":"t*","to":"primitives"}).as_object()).unwrap();
    let out=(NODE.cook)(&[vec![Arc::new(g)]],&p).unwrap();
    assert_eq!(out.prim["group_tip"].data,crate::geometry::AttrData::I32(vec![0]));
    assert!(!out.point.contains_key("group_tip"));
}
