use super::*;
#[test]
fn package_registers_with_defaults() {
    let spec = crate::nodes::get("attribute_create").unwrap();
    assert_eq!(spec.name, NODE.name);
    Params::resolve(NODE.params, None).unwrap();
}

#[test]
fn multiple_attributes_respect_class_size_default_and_group() {
    let g=Geometry::from_arrays(vec![0.;9],vec![0,1,2],vec![0,3],None).unwrap();
    let attrs=json!([
        {"name":"weight","class":"points","type":"float","size":1,"default":[-1],"value":[2]},
        {"name":"uv","class":"vertices","type":"float","size":2,"default":[0,0],"value":[0.25,0.75]},
        {"name":"tag","class":"detail","type":"string","size":1,"default":"","value":"sample"}
    ]);
    let p=Params::resolve(NODE.params,json!({"group":"1","attributes":attrs}).as_object()).unwrap();
    let out=(NODE.cook)(&[vec![std::sync::Arc::new(g)]],&p).unwrap();
    assert_eq!(out.point["weight"].as_f32().unwrap(),&[-1.,2.,-1.]);
    assert_eq!(out.vertex["uv"].width,2);
    assert_eq!(out.vertex["uv"].as_f32().unwrap(),&[0.25,0.75,0.25,0.75,0.25,0.75]);
    assert_eq!(out.detail["tag"],"sample");
    out.validate().unwrap();
}

#[test]
fn empty_list_clears_attributes_and_legacy_still_cooks() {
    let g=Geometry::points_only(vec![0.;6]).unwrap();
    let p=Params::resolve(NODE.params,json!({"attributes":[]}).as_object()).unwrap();
    assert!(!(NODE.cook)(&[vec![std::sync::Arc::new(g.clone())]],&p).unwrap().point.contains_key("attrib1"));
    let p=Params::resolve(NODE.params,json!({"value":[3,0,0]}).as_object()).unwrap();
    assert_eq!((NODE.cook)(&[vec![std::sync::Arc::new(g)]],&p).unwrap().point["attrib1"].as_f32().unwrap(),&[3.,3.]);
}
