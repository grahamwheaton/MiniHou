use super::*;
#[test]
fn point_position() {
    let mut m = serde_json::Map::new();
    m.insert("center".into(), serde_json::json!([1.25, -2.0, 3.0]));
    let p = Params::resolve(NODE.params, Some(&m)).unwrap();
    let g = (NODE.cook)(&[], &p).unwrap();
    assert_eq!(g.p(), &[1.25, -2.0, 3.0]);
    assert_eq!(g.nprims(), 0);
}
