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
    let camera = CameraInfo { projection: [[1.,0.,0.,0.],[0.,1.,0.,0.],[0.,0.,1.,0.],[0.,0.,0.,1.]], ..Default::default() };
    let view = View { graph: &graph, doc: doc.value(), path: std::path::Path::new("demo.json"), dirty: false,
        can_undo: false, can_redo: false, selected: "hidden_transform", display: "output1", cooking: &cooking,
        file_status: "", mode: Mode::Shaded, theme: &theme, editor: &editor, camera,
        bounds: ([0.;3], 1.), selection_center: None, geo: None, blender: &blender, flags: &flags,
        units: crate::units::UnitSettings::default() };
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
            ("translate", [TransformOp::Move([0.5, 0., 0.]); 3], json!([1.5, 0., 0.])),
            ("rotate", [TransformOp::Rotate([0., 0., 15.]); 3], json!([0., 0., 45.])),
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
