use std::sync::Arc;
use crate::{geometry::{Attr, AttrData, Geometry}, nodes::curves::curve, params::Params};
use super::*;

fn cook(g: Geometry, v: serde_json::Value) -> Geometry {
    let p = Params::resolve(NODE.params, v.as_object()).unwrap();
    (NODE.cook)(&[vec![Arc::new(g)]], &p).unwrap()
}

#[test]
fn resample_interpolation_keep_original_knots() {
    let mut g = curve(vec![0., 0., 0., 0., 0., 1., 0., 0., 2., 0., 0., 3., 0., 0., 4.], false).unwrap();
    g.point.insert("id".into(), Arc::new(Attr::i32(1, vec![10, 11, 12, 13, 14])));
    let r = cook(g, serde_json::json!({"length": 0.5}));
    assert_eq!(r.npoints, 9);
    assert_eq!(r.point["curveu"].as_f32().unwrap(), [0., 0.125, 0.25, 0.375, 0.5, 0.625, 0.75, 0.875, 1.]);
    assert_eq!(r.point["id"].data, AttrData::I32(vec![10, 10, 11, 11, 12, 12, 13, 13, 14]));
}

#[test]
fn closed_resample() {
    let g = curve(vec![0., 0., 0., 1., 0., 0., 1., 1., 0., 0., 1., 0.], true).unwrap();
    let r = cook(g, serde_json::json!({"length": 0.25}));
    assert_eq!(r.npoints, 16);
    assert!(r.topology.is_closed(0));
}
