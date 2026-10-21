use crate::{param_kind, selftest, Camera, Document, ParamKind, StandardView};
use minihou_core::nodes;
use serde_json::{json, Value};

const SMALL: &str = include_str!("../tests/fixtures/small.json");

fn is_composite_list(value: &Value) -> bool {
    matches!(value, Value::Array(items) if !items.is_empty() && items.iter().all(|item| item.is_array() || item.is_object()))
}

#[test]
fn parameter_kinds_from_defaults() {
    for (v,k) in [(json!(true),ParamKind::Bool),(json!(2),ParamKind::Integer),(json!(2.5),ParamKind::Float),(json!("x"),ParamKind::String),(json!([1,2.,3]),ParamKind::Vec3),(Value::Null,ParamKind::Unsupported),(json!([1,2]),ParamKind::Unsupported)] {
        assert_eq!(param_kind(&v),k);
    }
    for spec in nodes::registry() {
        // Structured lists use dedicated editors; they are not scalar parameter kinds.
        for param in spec.params { let default = (param.default)(); if (spec.name=="attribute_create" && matches!(param.name, "attributes" | "default")) || is_composite_list(&default) {continue;} assert_ne!(param_kind(&default),ParamKind::Unsupported,"{} {}",spec.name,param.name); }
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
fn display_list_round_trip_and_cooks_merged_outputs() {
    let mut doc = Document::parse(json!({"version":1,"nodes":{"box1":{"type":"box"},"sphere1":{"type":"sphere"}},"display":"box1"}).to_string()).unwrap();
    doc.set_display_nodes(&["box1".into(), "sphere1".into()]).unwrap();
    let restored = Document::parse(doc.text().to_owned()).unwrap();
    assert_eq!(restored.display_nodes(), vec!["box1", "sphere1"]);
    let mut session = restored.session().unwrap();
    let mut target = "box1".to_owned();
    let response = cook_batch(&mut session, &mut target, vec![request(1, Edit::Replace(restored.value().clone()))], |_| ());
    assert!(response.error.is_none(), "{:?}", response.error);
    assert!(response.geometry.unwrap().npoints > session.cook(Some("box1")).unwrap().npoints);
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

#[test]
fn explicit_clip_planes_survive_zoom_and_scene_scale() {
    for radius in [0.001, 1.0, 10000.0] {
        let mut camera = Camera::frame([0.; 3], radius);
        camera.set_clip_planes(0.02, 25000.0);
        camera.set_dist(0.001);
        assert_eq!(camera.clip_planes(), (0.02, 25000.0));
        camera.zoom(-100.0);
        assert_eq!(camera.clip_planes(), (0.02, 25000.0));
        assert!(camera.matrices(1.0).0.iter().flatten().all(|x| x.is_finite()));
    }
}

#[test]
fn invalid_clip_planes_are_sanitized() {
    let mut camera = Camera::frame([0.; 3], 1.0);
    for (near, far) in [(-1., -2.), (10., 1.), (f32::NAN, f32::INFINITY), (f32::MAX, 0.)] {
        camera.set_clip_planes(near, far);
        let (near, far) = camera.clip_planes();
        assert!(near.is_finite() && far.is_finite() && near > 0. && far > near);
    }
}

#[test]
fn point_at_15_radii_in_front_of_camera_inside_frustum() {
    let radius = 10.0;
    let camera = Camera::frame([0., 0., 0.], radius);
    assert_eq!(camera.clip_planes(), (0.01, 1000.0));

    let (near, far) = camera.clip_planes();
    let pt_dist = 15.0 * radius;
    assert!(pt_dist > near && pt_dist < far, "pt_dist {pt_dist} should be between near {near} and far {far}");

    let (view_proj, _) = camera.matrices(1.0);
    let eye = camera.eye();
    let fwd = camera.forward();
    let pt = [eye[0] + fwd[0] * pt_dist, eye[1] + fwd[1] * pt_dist, eye[2] + fwd[2] * pt_dist];
    let clip_z = view_proj[0][2] * pt[0] + view_proj[1][2] * pt[1] + view_proj[2][2] * pt[2] + view_proj[3][2];
    let clip_w = view_proj[0][3] * pt[0] + view_proj[1][3] * pt[1] + view_proj[2][3] * pt[2] + view_proj[3][3];
    assert!(clip_w > 0.0);
    let ndc_z = clip_z / clip_w;
    assert!((0.0..=1.0).contains(&ndc_z), "ndc_z {ndc_z} must be in [0, 1] for point inside frustum");

    let old_far = camera.distance() + radius * 3.0;
    assert!(pt_dist > old_far, "point at 15 radii should be beyond old far {old_far}");
}

#[test]
fn orthographic_uses_same_clip_planes() {
    let mut camera = Camera::frame([0., 0., 0.], 10.0);
    let (p_near, p_far) = camera.clip_planes();
    camera.set_view(StandardView::Front);
    assert!(camera.is_ortho());
    let (o_near, o_far) = camera.clip_planes();
    assert_eq!((p_near, p_far), (o_near, o_far));
}


use crate::{Edit, History, CookRequest, CookState, CookWorker, cook_batch, param_meta, slider_range, clamp_parameter, metadata_kind};
fn columns(value: Value) -> Edit { Edit::Param { node:"g".into(), name:"columns".into(), value } }
fn request(revision: u64, edit: Edit) -> CookRequest { CookRequest { revision, edit } }

#[test]
fn bom_open_save_strips_bom_and_preserves_crlf() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/bom-{}.json", std::process::id()));
    let text = SMALL.replace('\n', "\r\n");
    std::fs::write(&path, format!("\u{feff}{text}")).unwrap();
    let mut doc = Document::open(&path).unwrap();
    doc.edit(&columns(json!(7))).unwrap();
    doc.save(&path).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text.replace("\"columns\":3", "\"columns\":7"));
    std::fs::remove_file(path).unwrap();
}
#[test]
fn metadata_covers_all_node_types_and_parameters() {
    assert!(nodes::registry().len() >= 19);
    for spec in nodes::registry() {
        for param in spec.params {
            let meta = param_meta(spec.name, param.name).unwrap_or_else(|| panic!("missing metadata for {}.{}", spec.name, param.name));
            assert!(meta["kind"].is_string());
            assert!(meta["doc"].is_string());
            let default = (param.default)();
            if !(spec.name=="attribute_create" && matches!(param.name, "attributes" | "default")) && !is_composite_list(&default) { assert_ne!(metadata_kind(meta, &default), ParamKind::Unsupported); }
        }
    }
    assert!(param_meta("missing", "missing").is_none());
}
#[test]
fn metadata_ranges_hard_bounds_and_soft_bounds() {
    let hard = param_meta("grid", "columns").unwrap();
    assert_eq!(slider_range(hard), Some(2.0..=20000.0));
    assert_eq!(clamp_parameter(hard, 30000.), 20000.);
    assert_eq!(clamp_parameter(hard, -1.), 2.);
    let soft = param_meta("grid", "size_x").unwrap();
    assert_eq!(slider_range(soft), Some(0.0..=100.0));
    assert_eq!(clamp_parameter(soft, 200.), 200.);
    assert_eq!(clamp_parameter(soft, -1.), 0.);
    assert!(slider_range(param_meta("noise", "seed").unwrap()).is_none());
    assert_eq!(param_meta("noise", "mode").unwrap()["options"], json!(["scalar", "vector"]));
}
#[test]
fn batch_cooks_only_newest_state_and_retains_independent_edits() {
    let doc = Document::parse(SMALL.into()).unwrap();
    let mut session = doc.session().unwrap();
    let mut target = "o".to_string();
    let batch = vec![request(1, columns(json!("bad"))), request(2, columns(json!(7))),
        request(3, Edit::Param { node:"g".into(), name:"rows".into(), value:json!(4) }),
        request(4, Edit::Display("g".into()))];
    let response = cook_batch(&mut session, &mut target, batch, |_| ());
    assert_eq!(response.revision, 4);
    assert!(response.error.is_none());
    assert_eq!(response.geometry.unwrap().npoints, 28);
    assert_eq!(response.stats.cooked, vec!["g"]);
}
#[test]
fn failed_cook_keeps_last_good_mesh_and_fix_recovers() {
    let doc = Document::parse(SMALL.into()).unwrap();
    let mut session = doc.session().unwrap();
    let mut target = "g".to_string();
    let mut state = CookState { requested:1, ..Default::default() };
    state.accept(cook_batch(&mut session, &mut target, vec![request(1, columns(json!(3)))], |_| ()));
    let good = state.geometry.clone().unwrap();
    state.requested = 2;
    state.accept(cook_batch(&mut session, &mut target, vec![request(2, columns(json!("bad")))], |_| ()));
    assert!(state.error.is_some());
    assert!(std::sync::Arc::ptr_eq(&good, state.geometry.as_ref().unwrap()));
    state.requested = 3;
    state.accept(cook_batch(&mut session, &mut target, vec![request(3, columns(json!(8)))], |_| ()));
    assert!(state.error.is_none());
    assert!(!state.cooking());
    assert_eq!(state.geometry.unwrap().npoints, 24);
}
#[test]
fn stale_response_never_overwrites_newest_request() {
    let doc = Document::parse(SMALL.into()).unwrap();
    let mut session = doc.session().unwrap();
    let mut target = "g".into();
    let mut state = CookState { requested:2, ..Default::default() };
    assert!(state.accept(cook_batch(&mut session, &mut target, vec![request(1, columns(json!(3)))], |_| ())).is_none());
    assert!(state.geometry.is_none());
    assert!(state.cooking());
}
#[test]
fn displayed_downstream_node_carries_nearest_bake_payload() {
    let graph = json!({"nodes": {
        "g": {"type":"grid", "params":{"columns":24,"rows":24}},
        "b": {"type":"lod_bake", "inputs":{"geo":"g"}, "params":{"min_triangles":0}},
        "o": {"type":"output", "inputs":{"geo":"b"}}
    }, "display":"o"});
    let mut session = minihou_core::session::Session::new(minihou_core::Graph::from_value(&graph).unwrap());
    let mut target = "o".to_owned();
    let first = cook_batch(&mut session, &mut target, vec![request(1, Edit::Display("o".into()))], |_| ());
    assert!(first.error.is_none());
    let payload = first.lod.unwrap();
    assert!(!payload.clusters.is_empty());
    let second = cook_batch(&mut session, &mut target, vec![request(2, Edit::Display("o".into()))], |_| ());
    assert!(second.stats.cooked.is_empty());
    assert!(std::sync::Arc::ptr_eq(&payload, &second.lod.unwrap()));

    let changed = json!({"nodes": {
        "g": {"type":"grid", "params":{"columns":24,"rows":24}},
        "b": {"type":"lod_bake", "inputs":{"geo":"g"}, "params":{"min_triangles":0}},
        "t": {"type":"transform", "inputs":{"geo":"b"}, "params":{"translate":[1.0,0.0,0.0]}},
        "o": {"type":"output", "inputs":{"geo":"t"}}
    }, "display":"o"});
    let mut session = minihou_core::session::Session::new(minihou_core::Graph::from_value(&changed).unwrap());
    let response = cook_batch(&mut session, &mut target, vec![request(3, Edit::Display("o".into()))], |_| ());
    assert!(response.error.is_none());
    assert!(response.lod.is_none(), "transformed bounds must not be reused");
}
#[test]
fn undo_redo_restore_exact_json_and_text_including_omitted_members() {
    let mut doc = Document::parse(SMALL.into()).unwrap();
    let original = doc.value().clone();
    let mut history = History::default();
    let edit = Edit::Param { node:"n".into(), name:"frequency".into(), value:json!(2.0) };
    history.edit(&mut doc, &edit).unwrap();
    history.finish(&doc);
    let edited = doc.text().to_owned();
    let mut session = doc.session().unwrap();
    let mut target = "o".into();
    let undo = history.undo(&mut doc).unwrap();
    assert_eq!(doc.value(), &original);
    assert_eq!(doc.text(), SMALL);
    let response = cook_batch(&mut session, &mut target, vec![request(1, undo)], |_| ());
    assert!(response.error.is_none());
    assert!(session.graph().nodes.iter().find(|(id,_)| id == "n").map(|(_,n)| n).unwrap().params.is_none());
    let redo = history.redo(&mut doc).unwrap();
    assert_eq!(doc.text(), edited);
    cook_batch(&mut session, &mut target, vec![request(2, redo)], |_| ());
    assert_eq!(session.graph().nodes.iter().find(|(id,_)| id == "n").map(|(_,n)| n).unwrap().params.as_ref().unwrap()["frequency"], json!(2.0));
}
#[test]
fn drag_is_one_step_and_new_edit_clears_redo() {
    let mut doc = Document::parse(SMALL.into()).unwrap();
    let mut history = History::default();
    for n in 4..20 { history.edit(&mut doc, &columns(json!(n))).unwrap(); }
    history.finish(&doc);
    assert!(history.undo(&mut doc).is_some());
    assert_eq!(doc.text(), SMALL);
    assert!(history.undo(&mut doc).is_none());
    assert!(history.can_redo());
    history.edit(&mut doc, &columns(json!(6))).unwrap();
    history.finish(&doc);
    assert!(!history.can_redo());
}
#[test]
fn bypass_undo_restores_omitted_flag_and_numeric_spelling() {
    let text = SMALL.replace("\"columns\":3", "\"columns\":3.0");
    let mut doc = Document::parse(text.clone()).unwrap();
    let mut history = History::default();
    history.edit(&mut doc, &columns(json!(4))).unwrap();
    history.finish(&doc);
    history.undo(&mut doc).unwrap();
    assert_eq!(doc.text(), text);
    history.edit(&mut doc, &Edit::Bypass { node:"g".into(), flag:true }).unwrap();
    history.finish(&doc);
    history.undo(&mut doc).unwrap();
    assert_eq!(doc.text(), text);
    history.redo(&mut doc).unwrap();
    assert_eq!(doc.value()["nodes"]["g"]["flags"]["bypass"], true);
}
#[test]
fn actual_worker_delivers_final_revision() {
    let doc = Document::parse(SMALL.into()).unwrap();
    let worker = CookWorker::start(&doc, |g| g.npoints).unwrap();
    for n in 1..=20 { worker.send(request(n, columns(json!(n + 2)))).unwrap(); }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if let Some(response) = worker.poll() {
            if response.revision == 20 {
                assert_eq!(response.prepared, Some(66));
                assert!(response.error.is_none());
                break;
            }
        }
        assert!(std::time::Instant::now() < deadline, "worker timed out");
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

#[test]
fn timeline_frame_change_emits_one_time_edit() {
    let edits: Vec<_> = [12, 12].into_iter()
        .scan(1, |frame, next| {
            let edit = crate::timeline_time_edit(*frame, next, 24.);
            *frame = next;
            Some(edit)
        })
        .flatten().collect();
    assert_eq!(edits.len(), 1);
    assert!(matches!(edits[0], Edit::Time { frame: 12., fps: 24. }));
}

#[test]
fn timeline_time_dependent_blend_changes_output() {
    let doc = Document::parse(json!({"display":"blend","nodes":{
        "base":{"type":"point"},
        "target":{"type":"point","params":{"center":[2,0,0]}},
        "blend":{"type":"blend","inputs":{"geo":"base","targets":["target"]},"params":{"weights":"@Time"}}
    }}).to_string()).unwrap();
    let mut session = doc.session().unwrap();
    let mut target = "blend".to_owned();
    let mut at = |session: &mut _, revision, frame| cook_batch(session, &mut target,
        vec![request(revision, Edit::Time { frame, fps: 24. })], |_| ());
    let first = at(&mut session, 1, 1.);
    let second = at(&mut session, 2, 25.);
    assert!(first.error.is_none() && second.error.is_none());
    assert_ne!(first.geometry.unwrap().p()[0], second.geometry.unwrap().p()[0]);
}

#[test]
fn timeline_rapid_scrub_cooks_only_latest_frame() {
    let doc = Document::parse(json!({"display":"blend","nodes":{
        "base":{"type":"point"},
        "target":{"type":"point","params":{"center":[2,0,0]}},
        "blend":{"type":"blend","inputs":{"geo":"base","targets":["target"]},"params":{"weights":"@Frame"}}
    }}).to_string()).unwrap();
    let cooks = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = cooks.clone();
    let worker = CookWorker::start(&doc, move |g| {
        count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        g.p()[0]
    }).unwrap();
    for frame in 1..=80 {
        worker.send(request(frame, Edit::Time { frame: frame as f64, fps: 24. })).unwrap();
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if let Some(response) = worker.poll() {
            if response.revision == 80 {
                assert!(response.error.is_none());
                assert_eq!(response.prepared, Some(160.));
                assert_eq!(cooks.load(std::sync::atomic::Ordering::SeqCst), 1);
                break;
            }
        }
        assert!(std::time::Instant::now() < deadline, "worker timed out");
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

#[test]
fn file_save_as_dirty_title_and_open_reset() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let path = dir.join(format!("save-as-{}.json", std::process::id()));
    let doc = Document::parse(SMALL.into()).unwrap();
    let mut editor = crate::EditorDocument::new(doc, dir.join("original.json"));
    assert!(!editor.dirty());
    assert_eq!(editor.title(), "MiniHou - original.json");
    editor.history.edit(&mut editor.doc, &columns(json!(7))).unwrap();
    editor.history.finish(&editor.doc);
    assert!(editor.dirty());
    assert!(editor.title().ends_with(" *"));
    editor.save_to(path.clone()).unwrap();
    assert_eq!(editor.path, path);
    assert!(!editor.dirty());
    editor.history.undo(&mut editor.doc).unwrap();
    assert!(editor.dirty());
    editor.history.redo(&mut editor.doc).unwrap();
    assert!(!editor.dirty());
    let reopened = crate::EditorDocument::new(Document::open(&path).unwrap(), path.clone());
    assert!(!reopened.history.can_undo());
    assert!(!reopened.history.can_redo());
    assert!(!reopened.dirty());
    assert_eq!(reopened.doc.session().unwrap().cook(None).unwrap().npoints, 21);
    std::fs::remove_file(path).unwrap();
}
#[test]
fn failed_save_as_retains_path_and_dirty_state() {
    let doc = Document::parse(SMALL.into()).unwrap();
    let mut editor = crate::EditorDocument::new(doc, "original.json".into());
    editor.history.edit(&mut editor.doc, &columns(json!(7))).unwrap();
    let directory = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    assert!(editor.save_to(directory).is_err());
    assert_eq!(editor.path, std::path::PathBuf::from("original.json"));
    assert!(editor.dirty());
}

#[test]
fn the_built_in_demo_graph_opens_and_cooks() {
    // main.rs opens this when the app is started without arguments (a double-click).
    let doc = Document::parse(include_str!("test_data/demo_graph.json").to_owned()).expect("default graph parses");
    assert!(selftest(&doc).expect("default graph cooks")["points"].as_u64().unwrap() > 0);
}

#[test]
fn adding_a_node_is_one_undo_step() {
    use crate::History;
    let mut doc = Document::parse(include_str!("test_data/demo_graph.json").to_owned()).unwrap();
    let before = doc.text().to_owned();
    let mut value = doc.value().clone();
    value["nodes"]["extra1"] = json!({"type": "null", "inputs": {"geo": "grid1"}});
    let new = Document::parse(serde_json::to_string_pretty(&value).unwrap()).unwrap();
    let mut history = History::default();
    history.replace(&mut doc, new);
    assert!(doc.value()["nodes"].get("extra1").is_some());
    assert!(history.can_undo());
    assert!(history.undo(&mut doc).is_some());
    assert_eq!(doc.text(), before);
    assert!(history.redo(&mut doc).is_some());
    assert!(doc.value()["nodes"].get("extra1").is_some());
}
