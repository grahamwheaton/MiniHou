use super::*;
use std::sync::Arc;

#[test]
fn pack_node_modes() {
    let mut g = Geometry::from_arrays(vec![0., 0., 0., 1., 0., 0., 0., 1., 0., 5., 0., 0., 6., 0., 0., 5., 1., 0.],
        vec![0, 1, 2, 3, 4, 5], vec![0, 3, 6], None).unwrap();
    g.prim.insert("group_a".into(), Arc::new(crate::geometry::Attr::i32(1, vec![1, 0])));
    let run = |m: &str| cook(&[vec![Arc::new(g.clone())]],
        &Params::resolve(NODE.params, serde_json::json!({"mode": m, "group": "a"}).as_object()).unwrap()).unwrap();
    assert_eq!(packed::instance_count(&run("all")), 1);
    assert_eq!((packed::instance_count(&run("group")), run("group").nprims()), (1, 1));
    assert_eq!((packed::instance_count(&run("connectivity")), run("connectivity").nprims()), (2, 0));
    assert!(cook(&[vec![]], &Params::resolve(NODE.params, None).unwrap()).is_err());
}
