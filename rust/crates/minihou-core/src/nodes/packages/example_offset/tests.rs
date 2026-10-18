use super::*;

#[test]
fn cooks_enabled_and_disabled() {
    let source = Arc::new(Geometry::points_only(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]).unwrap());
    let inputs = vec![vec![source.clone()]];
    let mut values = serde_json::Map::new();
    values.insert("amount".into(), serde_json::json!(2.5));
    let params = Params::resolve(NODE.params, Some(&values)).unwrap();
    assert_eq!((NODE.cook)(&inputs, &params).unwrap().p(), &[1.0, 4.5, 3.0, 4.0, 7.5, 6.0]);
    values.insert("enabled".into(), serde_json::json!(false));
    let params = Params::resolve(NODE.params, Some(&values)).unwrap();
    assert_eq!((NODE.cook)(&inputs, &params).unwrap().p(), source.p());
}
