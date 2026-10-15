use super::*;
use minihou_core::preview;
use std::time::Instant;

fn cooked(worker: &CookWorker<Scene>, state: &mut CookState) -> ([f32; 3], [f32; 3], Vec<f32>) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(response) = worker.poll() {
            let revision = response.revision;
            if let Some(scene) = state.accept(response) {
                let geo = state.geometry.as_ref().unwrap();
                assert_eq!(scene.positions(), geo.p());
                let (lo, hi) = preview::bounds(scene.positions());
                assert!(state.error.is_none(), "revision {revision}: {:?}", state.error);
                return (lo, hi, scene.positions().to_vec());
            }
        }
        assert!(Instant::now() < deadline, "cook timed out at revision {}", state.requested);
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn send(worker: &CookWorker<Scene>, state: &mut CookState, edit: Edit) {
    state.requested += 1;
    worker.send(CookRequest { revision: state.requested, edit }).unwrap();
}

#[test]
fn gizmo_axis_drag_emits_move_steps_and_finishes() {
    use egui::{pos2, Event, PointerButton, RawInput, Rect};
    let mut demo: Value = serde_json::from_str(include_str!("default_graph.json")).unwrap();
    demo["nodes"]["hidden_transform"] = json!({"type": "transform", "inputs": {"geo": "grid1"}});
    let doc = Document::parse(demo.to_string()).unwrap();
    let graph = Graph::from_value(doc.value()).unwrap();
    let editor = EditorState { tool: ui::Tool::Move, ..Default::default() };
    let cooking = CookState::default();
    let theme = Theme::dark();
    let blender = BlenderStatus::default();
    let flags = HashMap::new();
    let selected_set = std::collections::BTreeSet::from(["hidden_transform".to_owned()]);
    let camera = CameraInfo { projection: [[1.,0.,0.,0.],[0.,1.,0.,0.],[0.,0.,1.,0.],[0.,0.,0.,1.]], ..Default::default() };
    let view = View { graph: &graph, doc: doc.value(), path: std::path::Path::new("demo.json"), dirty: false,
        can_undo: false, can_redo: false, selected: "hidden_transform", selected_set: &selected_set, display: "output1", cooking: &cooking,
        file_status: "", mode: Mode::Shaded, theme: &theme, editor: &editor, camera,
        bounds: ([0.;3], 1.), selection_center: None, normal_frame: [[1.,0.,0.],[0.,1.,0.],[0.,0.,1.]], geo: None, blender: &blender, flags: &flags,
        units: crate::units::UnitSettings::default(), repeat_label: None };
    let ctx = egui::Context::default();
    let frame = |events| {
        let mut actions = Actions::default();
        let mut rect = Rect::NOTHING;
        let _ = ctx.run(RawInput { events, ..Default::default() }, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                (rect, _) = ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
                ui::viewport_ui::overlay(ui, rect, &view, &mut actions);
                ui::transform_gizmo::show(ui, rect, &view, &mut actions);
            });
        });
        (actions.0, rect)
    };
    let (_, rect) = frame(Vec::new());
    let tip = pos2(rect.center().x + 58., rect.center().y);
    let (pressed, _) = frame(vec![Event::PointerMoved(tip), Event::PointerButton { pos: tip, button: PointerButton::Primary, pressed: true, modifiers: Default::default() }]);
    assert!(pressed.is_empty());
    let dragged_at = tip + egui::vec2(40., 0.);
    let (dragged, _) = frame(vec![Event::PointerMoved(dragged_at)]);
    assert!(dragged.iter().any(|a| matches!(a, UiAction::TransformNode { op: TransformOp::Move(v) } if v[0] > 0.)));
    // A selected transform can be unrelated to the viewport output. The old
    // action path would edit that hidden node and leave output1 unchanged.
    let mut wrong_target = doc.value().clone();
    assert_eq!(ensure_transform_target(&mut wrong_target, view.selected, true, None).as_deref(), Some("hidden_transform"));
    assert_eq!(wrong_target["display"], "output1");
    // The emitted action must move the displayed mesh through its new transform.
    let op = dragged.iter().find_map(|a| match a { UiAction::TransformNode { op } => Some(*op), _ => None }).unwrap();
    let mut transformed = doc.clone();
    let worker = CookWorker::start(&doc, Scene::new).unwrap();
    let mut state = CookState::default();
    send(&worker, &mut state, Edit::Replace(doc.value().clone()));
    let (_, original_hi, _) = cooked(&worker, &mut state);
    let mut value = transformed.value().clone();
    let target = ensure_transform_target(&mut value, view.display, true, None).unwrap();
    assert_eq!(value["nodes"][&target]["inputs"]["geo"], "output1");
    let (name, param) = transform_parameter(&value, &doc, &target, op);
    value["nodes"][&target]["params"] = json!({"group": "", "pivot": [0.,0.,0.]});
    value["nodes"][&target]["params"][name] = param.clone();
    for edit in apply_transform_step(&mut transformed, true, value, &target, name, param.clone()).unwrap() {
        send(&worker, &mut state, edit);
    }
    let (_, moved_hi, _) = cooked(&worker, &mut state);
    assert!((moved_hi[0] - original_hi[0] - param[0].as_f64().unwrap() as f32).abs() < 1e-3);
    let (released, _) = frame(vec![Event::PointerButton { pos: dragged_at, button: PointerButton::Primary, pressed: false, modifiers: Default::default() }]);
    assert!(released.iter().any(|a| matches!(a, UiAction::FinishTransform)));
}

#[test]
fn viewport_drag_cooks_move_rotate_scale_for_object_and_point_groups() {
    for point_mode in [false, true] {
        for (name, steps, expected) in [
            ("translate", [TransformOp::Move([0.5, 0., 0.]), TransformOp::Move([1.0, 0., 0.]), TransformOp::Move([1.5, 0., 0.])], json!([1.5, 0., 0.])),
            ("rotate", [TransformOp::Rotate([0., 0., 15.]), TransformOp::Rotate([0., 0., 30.]), TransformOp::Rotate([0., 0., 45.])], json!([0., 0., 45.])),
            ("scale", [TransformOp::Scale([1.2, 1., 1.]), TransformOp::Scale([1.5, 1., 1.]), TransformOp::Scale([2., 1., 1.])], json!([2., 1., 1.])),
        ] {
            let mut doc = Document::parse(include_str!("default_graph.json").to_owned()).unwrap();
            let before = doc.clone();
            let worker = CookWorker::start(&doc, Scene::new).unwrap();
            let mut state = CookState::default();
            send(&worker, &mut state, Edit::Replace(doc.value().clone()));
            let (original_lo, original_hi, original_points) = cooked(&worker, &mut state);
            let source = doc.display_nodes()[0].clone();
            let mut value = doc.value().clone();
            let target = ensure_transform_target(&mut value, &source, !point_mode, None).unwrap();
            let corners: Vec<u32> = original_points.as_chunks::<3>().0.iter().enumerate().filter_map(|(i, p)|
                ((p[0] - original_lo[0]).abs() < 1e-4 || (p[0] - original_hi[0]).abs() < 1e-4)
                .then_some((i, p)).filter(|(_, p)| (p[1] - original_lo[1]).abs() < 1e-4 || (p[1] - original_hi[1]).abs() < 1e-4)
                .map(|(i, _)| i as u32)).collect();
            assert_eq!(corners.len(), 4);
            let group = if point_mode { group_text(&corners) } else { String::new() };
            let pivot = if point_mode { [0, 1, 2].map(|axis| corners.iter().map(|id| original_points[*id as usize * 3 + axis]).sum::<f32>() / 4.) }
                else { [0, 1, 2].map(|axis| (original_lo[axis] + original_hi[axis]) * 0.5) };
            let mut replacement_count = 0;
            for (index, op) in steps.into_iter().enumerate() {
                let mut value = doc.value().clone();
                assert_eq!(ensure_transform_target(&mut value, &source, !point_mode, (index > 0).then_some(target.as_str())), Some(target.clone()));
                let (param_name, param) = transform_parameter(&value, &before, &target, op);
                assert_eq!(param_name, name);
                let params = &mut value["nodes"][&target]["params"];
                if !params.is_object() { *params = json!({}); }
                params[name] = param.clone();
                if index == 0 { params["group"] = json!(group); params["pivot"] = json!(pivot); }
                let edits = apply_transform_step(&mut doc, index == 0, value, &target, name, param.clone()).unwrap();
                assert_eq!(edits.len(), if index == 0 { 2 } else { 1 });
                for edit in edits { if matches!(edit, Edit::Replace(_)) { replacement_count += 1; } send(&worker, &mut state, edit); }
                let (lo, hi, points) = cooked(&worker, &mut state);
                assert!(points != original_points, "{name} step {index} point_mode={point_mode} did not move");
                if index > 0 {
                    assert!(state.stats.cached.contains(&source), "{name} step {index} had no upstream cache hit: {:?}", state.stats);
                    assert!(state.stats.cooked.contains(&target), "{name} step {index} did not recook target: {:?}", state.stats);
                }
                if name == "translate" {
                    let amount = param[0].as_f64().unwrap() as f32;
                    assert!((hi[0] - original_hi[0] - amount).abs() < 1e-3);
                    if !point_mode { assert!((lo[0] - original_lo[0] - amount).abs() < 1e-3); }
                } else { assert!(hi[0] > original_hi[0] + 1e-3, "{name} step {index} point_mode={point_mode} bounds did not change"); }
            }
            assert_eq!(replacement_count, 1);
            assert_eq!(doc.value()["nodes"][&target]["params"][name], expected);
            let mut history = minihou_app::History::default();
            history.record(before.clone(), &doc);
            let undo = history.undo(&mut doc).unwrap();
            assert_eq!(doc.value(), before.value());
            send(&worker, &mut state, undo);
            let (_, _, undone_points) = cooked(&worker, &mut state);
            assert_eq!(undone_points, original_points);
            assert!(history.undo(&mut doc).is_none());
        }
    }
}

#[test]
fn viewport_transform_replaces_only_its_entry_in_a_display_set() {
    let mut value: Value = serde_json::from_str(include_str!("default_graph.json")).unwrap();
    value["display"] = json!(["output1", "grid1"]);
    let doc = Document::parse(value.to_string()).unwrap();
    let worker = CookWorker::start(&doc, Scene::new).unwrap();
    let mut state = CookState::default();
    send(&worker, &mut state, Edit::Replace(doc.value().clone()));
    let (_, original_hi, _) = cooked(&worker, &mut state);
    let target = ensure_transform_target(&mut value, "output1", true, None).unwrap();
    assert_eq!(value["display"], json!([target.clone(), "grid1"]));
    value["nodes"][&target]["params"] = json!({"translate": [2., 0., 0.]});
    send(&worker, &mut state, Edit::Replace(value));
    send(&worker, &mut state, Edit::Param { node: target, name: "translate".into(), value: json!([2., 0., 0.]) });
    let (_, hi, _) = cooked(&worker, &mut state);
    assert!((hi[0] - original_hi[0] - 2.).abs() < 1e-3);
}

#[test]
fn modal_g_x_r_y_numeric_and_s_cage_cook_through_incremental_edits() {
    use ui::modal_transform::{CageHandle, Kind, Modal};
    use egui::{pos2, vec2, Key};
    let source_doc = Document::parse(include_str!("default_graph.json").to_owned()).unwrap();
    let camera = CameraInfo { ray_right:[1.,0.,0.], ray_up:[0.,1.,0.], ray_forward:[0.,0.,-1.], ..Default::default() };
    let frame=[[1.,0.,0.],[0.,1.,0.],[0.,0.,1.]];
    let initial_worker=CookWorker::start(&source_doc,Scene::new).unwrap();
    let mut initial_state=CookState::default();
    send(&initial_worker,&mut initial_state,Edit::Replace(source_doc.value().clone()));
    let (lo,hi,original)=cooked(&initial_worker,&mut initial_state);
    let pivot=[0,1,2].map(|i|(lo[i]+hi[i])*0.5);
    for (kind,axis,number) in [(Kind::Move,0,2),(Kind::Rotate,1,9)] {
        let mut modal=Modal::new(kind,pos2(100.,100.),pivot,(lo,hi),ui::SnapTo::Increment,false,1.);
        modal.toggle_axis(axis,false);
        assert!(modal.type_key(if number==2 {Key::Num2} else {Key::Num9},false));
        if kind==Kind::Rotate { modal.type_key(Key::Num0,false); }
        let (values,at)=modal.desired(pos2(100.,100.),100.,frame,camera,false,0.25,false);
        let op=modal.step(values);
        let mut doc=source_doc.clone(); let mut graph=doc.value().clone();
        let target=ensure_transform_target(&mut graph,&source_doc.display_nodes()[0],true,None).unwrap();
        let (name,param)=transform_parameter(&graph,&source_doc,&target,op);
        graph["nodes"][&target]["params"]=json!({"pivot":at,"group":""});
        graph["nodes"][&target]["params"][name]=param.clone();
        let worker=CookWorker::start(&source_doc,Scene::new).unwrap(); let mut state=CookState::default();
        for edit in apply_transform_step(&mut doc,true,graph,&target,name,param).unwrap() { send(&worker,&mut state,edit); }
        let (_,_,points)=cooked(&worker,&mut state);
        assert_ne!(points,original);
        assert_eq!(doc.value()["nodes"][&target]["type"],"transform");
        if kind==Kind::Move { assert!((points[0]-original[0]-2.).abs()<1e-3); }
        else { assert!((doc.value()["nodes"][&target]["params"]["rotate"][1].as_f64().unwrap()-90.).abs()<1e-3); }
        let mut history=minihou_app::History::default();
        history.record(source_doc.clone(),&doc);
        let undo=history.undo(&mut doc).unwrap();
        send(&worker,&mut state,undo);
        assert_eq!(cooked(&worker,&mut state).2,original);
        assert!(history.undo(&mut doc).is_none());
    }
    let mut modal=Modal::new(Kind::Scale,pos2(100.,100.),pivot,(lo,hi),ui::SnapTo::Increment,false,1.);
    modal.cage=true; modal.cage_handle=Some(CageHandle::Face(0,true)); modal.cage_start=pos2(100.,100.);
    modal.pointer=pos2(100.,100.)+vec2(100.,0.);
    let (values,cage_pivot)=modal.desired(pos2(100.,100.),100.,frame,camera,false,0.25,false);
    assert_eq!(cage_pivot[0],lo[0]); assert!(values[0]>1.);
    let op=modal.step(values);
    let mut doc=source_doc.clone(); let mut graph=doc.value().clone();
    let target=ensure_transform_target(&mut graph,&source_doc.display_nodes()[0],true,None).unwrap();
    let (name,param)=transform_parameter(&graph,&source_doc,&target,op);
    graph["nodes"][&target]["params"]=json!({"pivot":cage_pivot,"group":""});
    graph["nodes"][&target]["params"][name]=param.clone();
    let worker=CookWorker::start(&source_doc,Scene::new).unwrap(); let mut state=CookState::default();
    for edit in apply_transform_step(&mut doc,true,graph,&target,name,param).unwrap() { send(&worker,&mut state,edit); }
    let (scaled_lo,scaled_hi,_)=cooked(&worker,&mut state);
    assert!((scaled_lo[0]-lo[0]).abs()<1e-3);
    assert!(scaled_hi[0]>hi[0]);
    assert!(minihou_core::nodes::get("match_size").is_some());
    let (center,size)=ui::modal_transform::cage_target((lo,hi),cage_pivot,values);
    let mut matched=doc.value().clone();
    matched["nodes"][&target]["type"]=json!("match_size");
    matched["nodes"][&target]["params"]=json!({"translate":true,"center":center,"justify_z":"center",
        "scale":true,"size":size,"uniform":false});
    send(&worker,&mut state,Edit::Replace(matched));
    let (matched_lo,matched_hi,_)=cooked(&worker,&mut state);
    assert!((matched_lo[0]-scaled_lo[0]).abs()<1e-3);
    assert!((matched_hi[0]-scaled_hi[0]).abs()<1e-3);
}

#[test]
fn repeat_move_three_times_cooks_incrementally_with_single_transform_node() {
    let source_doc = Document::parse(include_str!("default_graph.json").to_owned()).unwrap();
    let worker = CookWorker::start(&source_doc, Scene::new).unwrap();
    let mut state = CookState::default();
    send(&worker, &mut state, Edit::Replace(source_doc.value().clone()));
    let (_, original_hi, original_points) = cooked(&worker, &mut state);

    // Initial operator: Move by [1.0, 0.0, 0.0]
    let mut doc = source_doc.clone();
    let mut graph = doc.value().clone();
    let source = doc.display_nodes()[0].clone();
    let target = ensure_transform_target(&mut graph, &source, true, None).unwrap();
    let op = LastOperator::Move {
        delta: [1.0, 0.0, 0.0],
        constraint: None,
        orientation: ui::Orientation::Global,
        proportional: None,
    };
    let initial_op = TransformOp::Move([1.0, 0.0, 0.0]);
    let (name, param) = transform_parameter(&graph, &source_doc, &target, initial_op);
    graph["nodes"][&target]["params"] = json!({"pivot": [0., 0., 0.], "group": ""});
    graph["nodes"][&target]["params"][name] = param.clone();

    let mut history = minihou_app::History::default();
    let before_initial = doc.clone();
    for edit in apply_transform_step(&mut doc, true, graph, &target, name, param).unwrap() {
        send(&worker, &mut state, edit);
    }
    history.record(before_initial, &doc);
    history.finish(&doc);

    let (_, hi1, points1) = cooked(&worker, &mut state);
    assert!((hi1[0] - original_hi[0] - 1.0).abs() < 1e-3);
    assert_ne!(points1, original_points);

    // 3 repeats of the move operator
    for i in 1..=3 {
        let before_repeat = doc.clone();
        let edit = repeat::apply_repeat_transform(&mut doc, &target, &op).unwrap();
        assert!(matches!(edit, Edit::Param { .. }));
        send(&worker, &mut state, edit);
        history.record(before_repeat, &doc);
        history.finish(&doc);

        let (_, hi, _) = cooked(&worker, &mut state);
        let expected_delta = 1.0 + (i as f32); // 2.0, 3.0, 4.0
        assert!((hi[0] - original_hi[0] - expected_delta).abs() < 1e-3, "step {i}: hi[0]={} vs expected={}", hi[0], original_hi[0] + expected_delta);
        assert!(state.stats.cached.contains(&source), "repeat step {i} had no upstream cache hit: {:?}", state.stats);
        assert!(state.stats.cooked.contains(&target), "repeat step {i} did not cook target: {:?}", state.stats);
    }

    // Verify only a SINGLE new transform node was created (reused across all 3 repeats)
    let initial_transforms = source_doc.value()["nodes"]
        .as_object()
        .unwrap()
        .values()
        .filter(|n| n["type"] == "transform")
        .count();
    let transform_node_count = doc.value()["nodes"]
        .as_object()
        .unwrap()
        .values()
        .filter(|n| n["type"] == "transform")
        .count();
    assert_eq!(transform_node_count, initial_transforms + 1);
    assert_eq!(doc.value()["nodes"][&target]["params"]["translate"], json!([4.0, 0.0, 0.0]));

    // Undo walkback: undo step 3 (back to 3.0), step 2 (back to 2.0), step 1 (back to 1.0), initial (back to 0.0)
    for expected_delta in [3.0, 2.0, 1.0, 0.0] {
        let undo = history.undo(&mut doc).unwrap();
        send(&worker, &mut state, undo);
        let (_, hi, points) = cooked(&worker, &mut state);
        assert!((hi[0] - original_hi[0] - expected_delta).abs() < 1e-3);
        if expected_delta == 0.0 {
            assert_eq!(points, original_points);
        }
    }
    assert!(history.undo(&mut doc).is_none());
}

#[test]
fn composed_global_local_view_sequence_cooks_expected_bounds() {
    use transform_math::{affine_about_pivot, axis_angle_matrix, compose_rotate_axis,
        compose_scale, compose_translate_axis, euler_xyz, mul4, Frame, ScaleResult, TrsParams};
    let source = Document::parse(json!({"version":1,"display":"box1",
        "nodes":{"box1":{"type":"box"}}}).to_string()).unwrap();
    let worker = CookWorker::start(&source, Scene::new).unwrap();
    let mut state = CookState::default();
    send(&worker, &mut state, Edit::Replace(source.value().clone()));
    let (_, _, points) = cooked(&worker, &mut state);
    let mut trs = TrsParams { pivot: [0.25, -0.5, 0.1], axis_rotation: [0., 0., 25.],
        uniform_scale: 1.2, ..Default::default() };
    let mut truth = trs.to_matrix();
    let r = axis_angle_matrix([0., 0., 1.], 90.);
    truth = mul4(&affine_about_pivot(r, trs.world_pivot()), &truth);
    trs = compose_rotate_axis(&trs, Frame::global(), 2, 90., false);
    let local = Frame::from_rotation(&trs.world_rotation());
    let direction = local.axis(0);
    let mut move_delta = transform_math::IDENTITY_4;
    for i in 0..3 { move_delta[i][3] = direction[i] * 2.; }
    truth = mul4(&move_delta, &truth);
    trs = compose_translate_axis(&trs, local, 0, 2.);
    let r = axis_angle_matrix([1., 0., 0.], 45.);
    truth = mul4(&affine_about_pivot(r, trs.world_pivot()), &truth);
    trs = compose_rotate_axis(&trs, Frame::global(), 0, 45., false);
    let view = Frame::from_rotation(&euler_xyz([25., -15., 10.]));
    let mut scale = transform_math::IDENTITY_3;
    for (i, row) in scale.iter_mut().enumerate() { row[i] = 1.5; }
    truth = mul4(&affine_about_pivot(scale, trs.world_pivot()), &truth);
    trs = match compose_scale(&trs, view, [1.5; 3], false, 1e-6) {
        ScaleResult::Direct(trs) => trs,
        _ => panic!("uniform scale must fit one node"),
    };
    let mut value = source.value().clone();
    let target = ensure_transform_target(&mut value, "box1", true, None).unwrap();
    value["nodes"][&target]["params"] = json!({"pivot":trs.pivot,
        "translate":trs.translate,"rotate":trs.rotate,"scale":trs.scale,
        "uniform_scale":trs.uniform_scale,"axis_rotation":trs.axis_rotation});
    send(&worker, &mut state, Edit::Replace(value));
    let (lo, hi, _) = cooked(&worker, &mut state);
    let mut expected = Vec::new();
    for point in points.as_chunks::<3>().0 {
        for row in truth.iter().take(3) { expected.push((row[3] + (0..3).map(|j| row[j] * point[j] as f64).sum::<f64>()) as f32); }
    }
    let (expected_lo, expected_hi) = preview::bounds(&expected);
    for i in 0..3 {
        assert!((lo[i] - expected_lo[i]).abs() < 1e-4, "lower bound axis {i}");
        assert!((hi[i] - expected_hi[i]).abs() < 1e-4, "upper bound axis {i}");
    }
}

#[test]
fn shearing_scale_appends_one_transform_node_and_cooks() {
    use transform_math::{compose_scale, Frame, ScaleResult, TrsParams};
    let source = Document::parse(json!({"version":1,"display":"transform1",
        "nodes":{"box1":{"type":"box"},"transform1":{"type":"transform",
            "inputs":{"geo":"box1"},"params":{"rotate":[0.,0.,45.]}}}}).to_string()).unwrap();
    let current = TrsParams::from_params(&source.value()["nodes"]["transform1"]["params"]);
    let ScaleResult::ShearNeedsSecondNode { frozen_current, second_node } =
        compose_scale(&current, Frame::global(), [2., 1., 1.], false, 1e-6) else {
            panic!("global nonuniform scale after rotation must need a second node");
        };
    assert_eq!(frozen_current, current);
    let mut value = source.value().clone();
    let target = ensure_transform_target(&mut value, "transform1", false, None).unwrap();
    assert_ne!(target, "transform1");
    assert_eq!(value["nodes"][&target]["inputs"]["geo"], "transform1");
    value["nodes"][&target]["params"] = json!({"translate":second_node.translate,
        "rotate":second_node.rotate,"scale":second_node.scale,"uniform_scale":second_node.uniform_scale,
        "pivot":second_node.pivot,"axis_rotation":second_node.axis_rotation});
    let worker = CookWorker::start(&source, Scene::new).unwrap();
    let mut state = CookState::default();
    send(&worker, &mut state, Edit::Replace(value));
    let (lo, hi, _) = cooked(&worker, &mut state);
    assert!((lo[0] + hi[0]).abs() < 1e-4);
    assert!((hi[0] - lo[0] - 2.0_f32.sqrt() * 2.).abs() < 1e-4);
}
