use super::*;
#[test]
fn bound_min_max_and_padding() {
    let g = Geometry::points_only(vec![1.0, 2.0, 3.0, 3.0, 4.0, 5.0]).unwrap();
    let mut m = serde_json::Map::new();
    m.insert("padding".into(), json!([0.5, 0.0, 0.25]));
    let p = Params::resolve(NODE.params, Some(&m)).unwrap();
    let out = (NODE.cook)(&[vec![Arc::new(g)]], &p).unwrap();
    assert_eq!(out.detail["bound_min"], json!([0.5, 2.0, 2.75]));
    assert_eq!(out.detail["bound_max"], json!([3.5, 4.0, 5.25]));
}
