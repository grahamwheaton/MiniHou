use crate::{param_kind, selftest, Camera, Document, ParamKind};
use minihou_core::nodes;
use serde_json::{json, Value};

const SMALL: &str = include_str!("../tests/fixtures/small.json");

#[test]
fn parameter_kinds_from_defaults() {
    for (v,k) in [(json!(true),ParamKind::Bool),(json!(2),ParamKind::Integer),(json!(2.5),ParamKind::Float),(json!("x"),ParamKind::String),(json!([1,2.,3]),ParamKind::Vec3),(Value::Null,ParamKind::Unsupported),(json!([1,2]),ParamKind::Unsupported)] {
        assert_eq!(param_kind(&v),k);
    }
    for spec in nodes::registry() {
        for param in spec.params { assert_ne!(param_kind(&(param.default)()),ParamKind::Unsupported,"{} {}",spec.name,param.name); }
    }
}
#[test]
fn unchanged_save_is_byte_exact_and_edit_only_replaces_value() {
    let original=SMALL.replace("\n","\r\n");
    let mut doc=Document::parse(original.clone()).unwrap();
    let path=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/roundtrip-{}.json",std::process::id()));
    doc.save(&path).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(),original.as_bytes());
    let mut session=doc.session().unwrap();
    doc.set_param(&mut session,"g","columns",json!(7)).unwrap();
    doc.save(&path).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(),original.replace("\"columns\":3","\"columns\":7"));
    let mut expected:Value=serde_json::from_str(&original).unwrap();
    expected["nodes"]["g"]["params"]["columns"]=json!(7);
    assert_eq!(doc.value(),&expected);
    std::fs::remove_file(path).unwrap();
}
#[test]
fn omitted_params_and_flags_preserve_other_members() {
    let mut doc=Document::parse(SMALL.into()).unwrap();
    let mut expected=doc.value().clone();
    let mut session=doc.session().unwrap();
    doc.set_param(&mut session,"n","frequency",json!(2.)).unwrap();
    expected["nodes"]["n"]["params"]=json!({"frequency":2.});
    doc.set_bypass(&mut session,"n",true).unwrap();
    expected["nodes"]["n"]["flags"]["bypass"]=json!(true);
    doc.set_bypass(&mut session,"g",true).unwrap();
    expected["nodes"]["g"]["flags"]=json!({"bypass":true});
    assert_eq!(doc.value(),&expected);
    assert!(session.graph().nodes[1].1.bypass);
    assert_eq!(doc.value()["nodes"]["n"]["flags"]["selected"],true);
}
#[test]
fn escaped_keys_unicode_and_braces_in_strings() {
    let raw = r#" { "nodes": {"a\"?": {"type":"null", "params":{"label":"a } \" b"}}}, "display":"a\"?" } "#;
    let mut doc=Document::parse(raw.into()).unwrap();
    let mut session=doc.session().unwrap();
    doc.set_param(&mut session,"a\"?","label",json!("new")).unwrap();
    assert_eq!(doc.text(),raw.replace(r#""a } \" b""#,r#""new""#));
}
#[test]
fn display_selection_is_saved_and_does_not_change_other_fields() {
    let mut doc=Document::parse(SMALL.into()).unwrap();
    doc.set_display("g").unwrap();
    assert_eq!(doc.text(),SMALL.replace("\"display\":\"o\"","\"display\":\"g\""));
    assert_eq!(doc.session().unwrap().cook(None).unwrap().npoints,9);
}
#[test]
fn invalid_edits_leave_document_unchanged() {
    let mut doc=Document::parse(SMALL.into()).unwrap();
    let mut session=doc.session().unwrap();
    assert!(doc.set_param(&mut session,"g","nonsense",json!(1)).is_err());
    assert!(doc.set_bypass(&mut session,"missing",true).is_err());
    assert!(doc.set_display("missing").is_err());
    assert_eq!(doc.text(),SMALL);
}
#[test]
fn generated_graph_selftest_cooks_incrementally() {
    let doc=Document::parse(SMALL.into()).unwrap();
    let output=selftest(&doc).unwrap();
    assert_eq!(output["nodes"],3);
    assert_eq!(output["points"],9);
    assert_eq!(output["prims"],4);
    assert_eq!(output["cooked_second"],json!(["n","o"]));
    assert!(output["first_cook_ms"].as_f64().unwrap()>=0.);
    assert_eq!(doc.text(),SMALL);
}
#[test]
fn single_node_selftest_bypasses_generator_to_empty() {
    let doc=Document::parse(json!({"nodes":{"g":{"type":"grid"}},"display":"g"}).to_string()).unwrap();
    let output=selftest(&doc).unwrap();
    assert_eq!(output["points"],0);
    assert_eq!(output["prims"],0);
    assert_eq!(output["cooked_second"],json!(["g"]));
}
#[test]
fn cook_failure_names_node_and_can_recover() {
    let mut doc=Document::parse(SMALL.into()).unwrap();
    let mut session=doc.session().unwrap();
    doc.set_param(&mut session,"g","columns",json!("bad")).unwrap();
    assert!(session.cook(None).unwrap_err().to_string().contains("'g'"));
    doc.set_param(&mut session,"g","columns",json!(3)).unwrap();
    assert_eq!(session.cook(None).unwrap().npoints,9);
}
#[test]
fn camera_frame_orbit_pan_zoom_are_finite() {
    let mut camera=Camera::frame([1.,2.,3.],0.);
    let initial=camera.matrices(1.);
    camera.orbit(100.,-10000.); camera.pan(20.,10.); camera.zoom(1e10);
    let next=camera.matrices(1.);
    assert_ne!(initial,next);
    assert!(next.0.iter().flatten().all(|f|f.is_finite()));
    camera.zoom(-1e10);
    assert!(camera.matrices(0.1).0.iter().flatten().all(|f|f.is_finite()));
}

