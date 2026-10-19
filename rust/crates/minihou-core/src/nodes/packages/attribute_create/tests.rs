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

#[test]
fn advanced_attribute_create_options() {
    let g = Geometry::from_arrays(vec![0.; 9], vec![0, 1, 2], vec![0, 3], None).unwrap();

    // 1. encode_invalid
    let p = Params::resolve(NODE.params, json!({
        "name": "my attrib#1",
        "encode_invalid": true,
        "value": [5.0, 0.0, 0.0]
    }).as_object()).unwrap();
    let out = (NODE.cook)(&[vec![std::sync::Arc::new(g.clone())]], &p).unwrap();
    assert!(out.point.contains_key("my_attrib_1"));
    assert_eq!(out.point["my_attrib_1"].as_f32().unwrap(), &[5.0, 5.0, 5.0]);

    // 2. enable: false skips creation
    let p = Params::resolve(NODE.params, json!({
        "name": "disabled_attrib",
        "enable": false
    }).as_object()).unwrap();
    let out = (NODE.cook)(&[vec![std::sync::Arc::new(g.clone())]], &p).unwrap();
    assert!(!out.point.contains_key("disabled_attrib"));

    // 3. existing: "keep" preserves prior attribute
    let mut g_with_val = g.clone();
    g_with_val.point.insert("existing_val".into(), std::sync::Arc::new(Attr::f32(1, vec![42.0, 42.0, 42.0])));
    let p = Params::resolve(NODE.params, json!({
        "name": "existing_val",
        "value": [99.0, 0.0, 0.0],
        "existing": "keep"
    }).as_object()).unwrap();
    let out = (NODE.cook)(&[vec![std::sync::Arc::new(g_with_val)]], &p).unwrap();
    assert_eq!(out.point["existing_val"].as_f32().unwrap(), &[42.0, 42.0, 42.0]);

    // 4. guess_from_name
    let p = Params::resolve(NODE.params, json!({
        "name": "id",
        "guess_from_name": true,
        "value": [7.0, 0.0, 0.0]
    }).as_object()).unwrap();
    let out = (NODE.cook)(&[vec![std::sync::Arc::new(g.clone())]], &p).unwrap();
    assert_eq!(out.point["id"].as_i32().unwrap(), &[7, 7, 7]);

    // 5. write_values: false fills with default
    let attrs = json!([
        {"name": "test_def", "class": "points", "type": "float", "size": 1, "default": [12.0], "value": [88.0], "write_values": false}
    ]);
    let p = Params::resolve(NODE.params, json!({"attributes": attrs}).as_object()).unwrap();
    let out = (NODE.cook)(&[vec![std::sync::Arc::new(g.clone())]], &p).unwrap();
    assert_eq!(out.point["test_def"].as_f32().unwrap(), &[12.0, 12.0, 12.0]);

    // 6. group_type: "primitives" applied to point attribute
    let p = Params::resolve(NODE.params, json!({
        "name": "from_prim_group",
        "domain": "points",
        "group": "0",
        "group_type": "primitives",
        "value": [1.0, 0.0, 0.0]
    }).as_object()).unwrap();
    let out = (NODE.cook)(&[vec![std::sync::Arc::new(g)]], &p).unwrap();
    assert_eq!(out.point["from_prim_group"].as_f32().unwrap(), &[1.0, 1.0, 1.0]);
}

#[test]
fn single_attribute_size_default_and_precision() {
    let g = Geometry::points_only(vec![0.; 9]).unwrap();
    let run = |args: Value| {
        let p = Params::resolve(NODE.params, args.as_object()).unwrap();
        (NODE.cook)(&[vec![std::sync::Arc::new(g.clone())]], &p)
    };
    let legacy = run(json!({"value":[3.,0.,0.]})).unwrap();
    assert_eq!(legacy.point["attrib1"].width, 1);
    assert_eq!(legacy.point["attrib1"].as_f32().unwrap(), &[3.,3.,3.]);
    let sized = run(json!({"name":"test","size":2,"default":[-1.,-2.],
        "value":[4.,5.,6.],"group":"1","precision":"32"})).unwrap();
    assert_eq!(sized.point["test"].width, 2);
    assert_eq!(sized.point["test"].as_f32().unwrap(), &[-1.,-2.,4.,5.,-1.,-2.]);
    let defaults = run(json!({"name":"test","size":2,"default":[7.,8.],
        "value":[4.,5.,6.],"write_values":false})).unwrap();
    assert_eq!(defaults.point["test"].as_f32().unwrap(), &[7.,8.,7.,8.,7.,8.]);
    assert!(run(json!({"precision":"64"})).unwrap_err().to_string().contains("storage type"));
    let primitive = Geometry::from_arrays(vec![0.;9], vec![0,1,2], vec![0,3], None).unwrap();
    let p = Params::resolve(NODE.params, json!({"name":"flag","class":"primitives","type":"int",
        "value":[5,0,0]}).as_object()).unwrap();
    let out = (NODE.cook)(&[vec![std::sync::Arc::new(primitive)]], &p).unwrap();
    assert_eq!(out.prim["flag"].as_i32().unwrap(), &[5]);
    let locals = run(json!({"name":"index","value":["$PT",0,0]})).unwrap();
    assert_eq!(locals.point["index"].as_f32().unwrap(), &[0.,1.,2.]);
    assert!(run(json!({"value":["$PT",0,0],"allow_local":false})).unwrap_err().to_string().contains("disabled"));
}
