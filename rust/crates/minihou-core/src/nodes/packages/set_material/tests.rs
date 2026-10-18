use super::*;

#[test]
fn test_set_material_basic() {
    let pts = vec![0.0; 12];
    let vp = vec![0, 1, 2, 0, 2, 3];
    let offs = vec![0, 3, 6];
    let geo = Geometry::from_arrays(pts, vp, offs, None).unwrap();

    let p = Params::resolve(NODE.params, serde_json::json!({"material": "Wood"}).as_object()).unwrap();
    let out = set_material(&[vec![Arc::new(geo)]], &p).unwrap();

    let names = get_names(&out);
    assert_eq!(names, vec!["", "Wood"]);
    let idx = get_indices(&out, "prim");
    assert_eq!(idx, vec![1, 1]);
}

#[test]
fn test_set_material_index_attrib() {
    let pts = vec![0.0; 12];
    let vp = vec![0, 1, 2, 0, 2, 3];
    let offs = vec![0, 3, 6];
    let mut geo = Geometry::from_arrays(pts, vp, offs, None).unwrap();
    geo.prim.insert("piece".to_string(), Arc::new(Attr::i32(1, vec![0, 1])));

    let p = Params::resolve(NODE.params, serde_json::json!({
        "index_attrib": "piece",
        "names": "Wood, Metal, Glass"
    }).as_object()).unwrap();
    let out = set_material(&[vec![Arc::new(geo)]], &p).unwrap();

    let names = get_names(&out);
    assert_eq!(names, vec!["", "Wood", "Metal", "Glass"]);
    let idx = get_indices(&out, "prim");
    assert_eq!(idx, vec![1, 2]);
}
