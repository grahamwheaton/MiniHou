use crate::{Graph, session::Session};
use serde_json::json;

#[test]
fn controller_sections_roundtrip_and_parameter_change_recooks() {
    let value=json!({"version":1,"display":"box1","nodes":{
        "driver":{"type":"controller","controller_sections":[
            {"name":"Shape","parameters":[{"name":"width","kind":"float","default":1.0,"expose_to_blender":true}]},
            {"name":"Options","parameters":[{"name":"enabled","kind":"bool","default":true,"expose_to_blender":false}]}],
            "params":{"width":2.0}},
        "box1":{"type":"box","channels":{"size.x":"ch('../driver/width')"}}
    }});
    let graph=Graph::from_value(&value).unwrap();
    let saved=graph.to_value();
    assert_eq!(saved["nodes"]["driver"]["controller_sections"],value["nodes"]["driver"]["controller_sections"]);
    let mut session=Session::new(Graph::from_json(&saved.to_string()).unwrap());
    let initial = session.resolved_channels();
    assert_eq!(initial.params["box1"]["size"][0],json!(2.0),"{:?}",initial.diagnostics);
    let first=session.cook(None).unwrap();
    let x0=first.p().chunks_exact(3).map(|p|p[0]).fold(f32::NEG_INFINITY,f32::max);
    session.set_param("driver","width",json!(4.0)).unwrap();
    let changed = session.resolved_channels();
    assert_eq!(changed.params["box1"]["size"][0],json!(4.0),"{:?}",changed.diagnostics);
    let second=session.cook(None).unwrap();
    let x1=second.p().chunks_exact(3).map(|p|p[0]).fold(f32::NEG_INFINITY,f32::max);
    assert!(x1>x0+0.5,"controller edit must recook the dependent box: {x0} -> {x1}");
}

#[test]
fn controller_rejects_invalid_sections_and_values() {
    let value=json!({"nodes":{"driver":{"type":"controller","controller_sections":[{"name":"Shape","parameters":[{"name":"width","kind":"float","default":1.0}]}],"params":{"width":"bad"}}}});
    assert!(Graph::from_value(&value).is_err());
}
