use std::sync::Arc;
use crate::{geometry::Geometry, nodes::curves::curve, params::Params};
use super::*;

fn cook(g: Geometry, v: serde_json::Value) -> Geometry {
    let p = Params::resolve(NODE.params, v.as_object()).unwrap();
    (NODE.cook)(&[vec![Arc::new(g)]], &p).unwrap()
}

#[test]
fn fillet_keeps_ends() {
    let g = curve(vec![0., 0., 0., 1., 0., 0., 1., 1., 0.], false).unwrap();
    let out = cook(g, serde_json::json!({"radius": 0.2, "divisions": 2}));
    assert_eq!(out.npoints, 5);
    assert_eq!(&out.p()[..3], &[0., 0., 0.]);
    assert_eq!(&out.p()[12..], &[1., 1., 0.]);
    assert!((out.p()[3] - 0.8).abs() < 1e-6);
    assert!((out.p()[10] - 0.2).abs() < 1e-6);
    out.validate().unwrap();
}
