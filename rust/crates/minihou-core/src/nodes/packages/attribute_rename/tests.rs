use super::*;
use crate::geometry::Attr;
use std::sync::Arc;

#[test]
fn rename_replaces_other_domains() {
    let mut g = Geometry::points_only(vec![0.; 3]).unwrap();
    g.point.insert("weight".into(), Arc::new(Attr::f32(1, vec![2.])));
    g.prim.insert("pscale".into(), Arc::new(Attr::f32(1, vec![])));
    let out = rename(&[vec![Arc::new(g)]], &Params::resolve(NODE.params,
        serde_json::json!({"from_name":"weight","to_name":"pscale"}).as_object()).unwrap()).unwrap();
    assert!(!out.prim.contains_key("pscale"));
    assert_eq!(out.point["pscale"].as_f32().unwrap(), [2.]);
}
