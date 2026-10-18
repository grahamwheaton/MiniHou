use super::*;
use std::sync::Arc;

#[test]
fn unpack_node_by_group_and_all() {
    let src = Arc::new(Geometry::from_arrays(vec![0., 0., 0., 1., 0., 0., 0., 1., 0.], vec![0, 1, 2], vec![0, 3], None).unwrap());
    let mut inst = packed::instance_geometry(&[src], &[0, 0], vec![0.0, 0.0, 0.0, 4.0, 0.0, 0.0],
        &[crate::math::IDENTITY; 2], Default::default()).unwrap();
    inst.point.insert("group_pick".into(), Arc::new(crate::geometry::Attr::i32(1, vec![0, 1])));
    let run = |v: serde_json::Value| cook(&[vec![Arc::new(inst.clone())]], &Params::resolve(NODE.params, v.as_object()).unwrap()).unwrap();
    let all = run(serde_json::json!({}));
    assert_eq!((all.npoints, all.nprims(), packed::instance_count(&all)), (6, 2, 0));
    let some = run(serde_json::json!({"group": "pick"}));
    assert_eq!((some.nprims(), packed::instance_count(&some)), (1, 1));
    assert_eq!(&some.p()[3..6], &[4.0, 0.0, 0.0]);
}
