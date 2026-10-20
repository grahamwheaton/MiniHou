    use super::*;
    use crate::ui::testing::Fixture;

    #[test] fn shelf_drop_centers_card_and_only_snaps_when_enabled() {
        let canvas = Rect::from_min_size(pos2(100., 80.), vec2(1000., 700.));
        let pan = vec2(-40., 25.);
        let zoom = 0.5;
        let pointer = canvas.min + pan + vec2(645., -61.) * zoom;
        assert_eq!(shelf_drop_position(pointer, canvas, pan, zoom, true, false), pos2(544., -96.));
        assert_eq!(shelf_drop_position(pointer, canvas, pan, zoom, false, false), pos2(535., -89.));
        assert_eq!(shelf_drop_position(pointer, canvas, pan, zoom, true, true), pos2(535., -89.));
    }

    #[test] fn shelf_payload_release_on_canvas_creates_a_standalone_node_at_pointer() {
        let f = Fixture::new();
        let view = f.view();
        let ctx = egui::Context::default();
        let canvas = Rect::from_min_size(pos2(100., 80.), vec2(1000., 700.));
        let mut state = State { physics: false, zoom: 0.5, pan: vec2(-40., 25.), ..Default::default() };
        let at = canvas.min + state.pan + vec2(645., 323.) * state.zoom;
        let mut out = Actions::default();
        for pressed in [true, false] {
            let input = egui::RawInput { events: vec![egui::Event::PointerMoved(at),
                egui::Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() }],
                ..Default::default() };
            let _ = ctx.run(input, |ctx| {
                if pressed { egui::DragAndDrop::set_payload(ctx, super::super::ToolDrag("transform".into())); }
                egui::CentralPanel::default().show(ctx, |ui| {
                    canvas_ui(ui, canvas, &view, &mut out, &mut state, Id::new("shelf_drop_test"));
                });
            });
        }
        assert!(matches!(out.0.as_slice(), [UiAction::AddNetworkNode { node_type, pos, .. }]
            if node_type == "transform" && *pos == [544., 288.]));
        assert!(!egui::DragAndDrop::has_payload_of_type::<super::super::ToolDrag>(&ctx));
    }

    #[test]
    fn stream_add_uses_cached_layout_and_undo_redo_restores_positions() {
        let ctx = egui::Context::default();
        let path = Path::new("insertion-test.json");
        let old = serde_json::json!({"version":1,"display":"out","nodes":{
            "a":{"type":"normal"},"out":{"type":"output","inputs":{"geo":"a"}}
        }});
        for anchor in ["out", "a"] {
            let mut state = State::default();
            state.moved = HashMap::from([("a".into(),pos2(320.,96.)),("out".into(),pos2(320.,256.))]);
            ctx.data_mut(|d| d.insert_persisted(state_id(path), state));
            let (mut next, id) = crate::graph_edit::add(&old, "normal", Some(anchor)).unwrap();
            let mut doc = position_add(&ctx, path, &old, &mut next, Some(anchor)).unwrap();
            assert_eq!(next["nodes"][&id]["pos"], serde_json::json!([320.,256.]));
            assert_eq!(next["nodes"]["out"]["pos"], serde_json::json!([320.,416.]));
            assert_eq!(next["nodes"]["out"]["inputs"]["geo"], id.as_str());
            let mut history = minihou_app::History::default();
            history.replace(&mut doc, crate::graph_edit::validated(next).unwrap());
            restore_positions(&ctx, path, doc.value());
            let cached = || ctx.data_mut(|d| d.get_persisted::<State>(state_id(path)).unwrap());
            assert_eq!(cached().moved["out"], pos2(320.,416.));
            assert!(history.undo(&mut doc).is_some());
            restore_positions(&ctx, path, doc.value());
            assert_eq!(cached().moved["out"], pos2(320.,256.));
            assert_eq!(cached().moved["a"], pos2(320.,96.));
            assert!(!cached().moved.contains_key(&id));
            assert!(!history.can_undo());
            assert!(history.redo(&mut doc).is_some());
            restore_positions(&ctx, path, doc.value());
            assert_eq!(cached().moved[&id], pos2(320.,256.));
            assert_eq!(cached().moved["out"], pos2(320.,416.));
        }
    }

    #[test]
    fn stream_add_without_live_override_pushes_output_node_down() {
        let ctx = egui::Context::default();
        let path = Path::new("downstream-push-test.json");
        let old = serde_json::json!({"version":1,"display":"out","nodes":{
            "a":{"type":"normal","pos":[320.,96.]},"out":{"type":"output","inputs":{"geo":"a"},"pos":[320.,256.]}
        }});
        let (mut next, id) = crate::graph_edit::add(&old, "normal", Some("out")).unwrap();
        let _ = position_add(&ctx, path, &old, &mut next, Some("out")).unwrap();
        assert_eq!(next["nodes"][&id]["pos"], serde_json::json!([320.,256.]));
        assert_eq!(next["nodes"]["out"]["pos"], serde_json::json!([320.,416.]));
    }

    #[test]
    fn panel_runs_and_status_precedence() {
        let f = Fixture::new();
        assert!(f.run(show).is_empty());
        let mut light = Fixture::new();
        light.theme = crate::ui::theme::Theme::light();
        assert!(light.run(show).is_empty());
        let v = f.view();
        assert_eq!(node_status(&v, "grid1", true, false, false).0, "muted");
        assert_eq!(node_status(&v, "grid1", false, true, false).0, "unsupported");
        assert_eq!(node_status(&v, "grid1", false, false, true).0, "failed");
    }
    #[test]
    fn frame_centers_content() {
        let mut state = State::default();
        let canvas = Rect::from_min_size(Pos2::ZERO, vec2(400.0, 600.0));
        frame_all(&mut state, canvas, Rect::from_min_size(pos2(90.0, 34.0), vec2(220.0, 456.0)));
        assert!((0.25..=2.0).contains(&state.zoom));
        assert!((screen(canvas, &state, pos2(200.0, 262.0)).x - canvas.center().x).abs() < 0.01);
    }
    #[test]
    fn flag_cells_request_the_expected_actions() {
        assert!(matches!(flag_action(0, "a", false), UiAction::Edit { edit: Edit::Bypass { node, flag: true }, continuous: false } if node == "a"));
        assert!(matches!(flag_action(1, "a", false), UiAction::ToggleLocked(node) if node == "a"));
        assert!(matches!(flag_action(2, "a", false), UiAction::ToggleTemplate(node) if node == "a"));
        assert!(matches!(flag_action(3, "a", false), UiAction::SetDisplay(node) if node == "a"));
    }
    #[test]
    fn grid_snap_and_ctrl_inversion() {
        let at = pos2(47.0, -17.0);
        assert_eq!(snap(at, 32.0, true, false), pos2(32.0, -32.0));
        assert_eq!(snap(at, 32.0, true, true), at);
        assert_eq!(snap(at, 32.0, false, false), at);
        assert_eq!(snap(at, 32.0, false, true), pos2(32.0, -32.0));
    }
    #[test] fn wire_direction_requires_opposite_ports_on_distinct_nodes() {
        let output = AddConnection::Output("a".into());
        let input = AddConnection::Input { node: "b".into(), port: "geo".into() };
        for (a, b) in [(&output, &input), (&input, &output)] {
            assert!(matches!(wire_connection(a, b), Some(UiAction::ConnectNodes { from, to, port: Some(port) }) if from == "a" && to == "b" && port == "geo"));
        }
        assert!(wire_connection(&output, &output).is_none());
        assert!(wire_connection(&input, &input).is_none());
        assert!(wire_connection(&AddConnection::Output("b".into()), &input).is_none());
    }

    #[test] fn noodle_placement_survives_tidied_canvas_without_moving_peers() {
        let ctx = egui::Context::default();
        let path = Path::new("tidied_noodle.json");
        let mut state = State { tidied: true, ..Default::default() };
        state.moved.insert("a".into(), pos2(96., 32.));
        ctx.data_mut(|d| d.insert_persisted(state_id(path), state));
        place_connected(&ctx, path, "new", [640., 320.]);
        let state = ctx.data_mut(|d| d.get_persisted::<State>(state_id(path)).unwrap());
        let doc = serde_json::json!({"nodes":{"a":{"type":"box"},"new":{"type":"transform","inputs":{"geo":"a"},"pos":[640,320]}}});
        let graph = minihou_core::Graph::from_value(&doc).unwrap();
        let positions: HashMap<_, _> = positions_for(&state, &graph, &doc).into_iter().collect();
        assert_eq!(positions["a"], pos2(96., 32.));
        assert_eq!(positions["new"], pos2(640., 320.));
    }

    #[test] fn headless_noodle_release_add_connect_and_cancel() {
        let f = Fixture::new();
        let doc = serde_json::json!({"nodes": {"a":{"type":"box"},"b":{"type":"transform"}}});
        let graph = minihou_core::Graph::from_value(&doc).unwrap();
        let mut view = f.view(); view.doc = &doc; view.graph = &graph;
        let canvas = Rect::from_min_size(Pos2::ZERO, vec2(1000., 700.));
        let button = |at, pressed| egui::Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
        let key = |key| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
        for reverse in [false, true] {
            for finish in [0, 1, 2, 3] { // choose, Escape, click away, direct connection
                let ctx = egui::Context::default();
                let mut state = State { physics: false, zoom: 0.75, pan: vec2(20., 30.), ..Default::default() };
                state.moved = [("a".into(), pos2(96., 96.)), ("b".into(), pos2(96., 256.))].into();
                let source = if reverse { AddConnection::Input { node: "b".into(), port: "geo".into() } } else { AddConnection::Output("a".into()) };
                let ports = canvas_ports(&view, &node_positions(&state, &view));
                let start = screen(canvas, &state, ports.iter().find(|(p, _)| p == &source).unwrap().1);
                let end = if finish == 3 {
                    let target = if reverse { AddConnection::Output("a".into()) } else { AddConnection::Input { node: "b".into(), port: "geo".into() } };
                    screen(canvas, &state, ports.iter().find(|(p, _)| p == &target).unwrap().1)
                } else { pos2(550., 350.) };
                let mut out = Actions::default();
                let frame = |events: Vec<egui::Event>, state: &mut State, out: &mut Actions| {
                    let input = egui::RawInput { events, screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280., 800.))), ..Default::default() };
                    let _ = ctx.run(input, |ctx| { egui::CentralPanel::default().show(ctx, |ui| {
                        canvas_ui(ui, canvas, &view, out, state, Id::new("noodle_test"));
                    }); });
                };
                frame(vec![egui::Event::PointerMoved(start)], &mut state, &mut out);
                frame(vec![button(start, true)], &mut state, &mut out);
                assert_eq!(state.wire.as_ref(), Some(&source));
                frame(vec![egui::Event::PointerMoved(end)], &mut state, &mut out);
                frame(vec![button(end, false)], &mut state, &mut out);
                assert!(state.wire.is_none());
                if finish == 3 {
                    assert!(state.menu.is_none());
                    assert!(matches!(out.0.as_slice(), [UiAction::ConnectNodes { from, to, port: Some(port) }] if from == "a" && to == "b" && port == "geo"));
                    continue;
                }
                assert!(out.0.is_empty());
                let (connection, at) = state.menu.as_ref().unwrap().connect_to.as_ref().unwrap();
                assert_eq!(connection, &source);
                assert_eq!(*at, pos2(608., 384.));
                match finish {
                    0 => {
                        frame(vec![egui::Event::Text("transform".into())], &mut state, &mut out);
                        frame(vec![key(egui::Key::Enter)], &mut state, &mut out);
                        assert!(matches!(out.0.as_slice(), [UiAction::AddConnectedNode { node_type, connection, pos }] if node_type == "transform" && connection == &source && *pos == [608., 384.]));
                    }
                    1 => frame(vec![key(egui::Key::Escape)], &mut state, &mut out),
                    _ => frame(vec![egui::Event::PointerMoved(pos2(20., 650.)), button(pos2(20., 650.), true)], &mut state, &mut out),
                }
                assert!(state.menu.is_none());
                assert!(state.wire.is_none());
                if finish != 0 { assert!(out.0.is_empty()); }
            }
        }
    }
    #[test]
    fn card_guides_snap_left_center_right_and_top() {
        let size = vec2(220.0, 56.0);
        let peers = [pos2(100.0, 80.0)];
        assert_eq!(align_to_cards(pos2(105.0, 75.0), &peers, size, 6.0).0, pos2(100.0, 80.0));
        assert_eq!(align_to_cards(pos2(96.0, 90.0), &peers, size, 6.0).1.x, Some(100.0));
        assert_eq!(align_to_cards(pos2(319.0, 90.0), &peers, size, 6.0), (pos2(320.0, 90.0), Guides { x: Some(320.0), y: None }));
        assert_eq!(align_to_cards(pos2(-9.0, 90.0), &peers, size, 6.0).0.x, -10.0);
        assert_eq!(align_to_cards(pos2(107.0, 80.0), &peers, size, 6.0).1.x, None);
    }
    #[test]
    fn breadcrumbs_are_ready_for_nested_paths() {
        let nested = vec!["subnet".to_owned(), "inner".to_owned()];
        assert_eq!(crumbs(Path::new("C:/work/scene.json"), &nested, "box1", true), vec!["scene", "subnet", "inner", "box1"]);
        assert_eq!(crumbs(Path::new("scene.json"), &[], "gone", false), vec!["scene"]);
    }
    #[test]
    fn tidy_keeps_locked_authored_manual_and_automatic_positions() {
        let ctx=egui::Context::default(); let path=Path::new("locked_tidy.json");
        let doc=serde_json::json!({"nodes":{
            "authored":{"type":"box","pos":[13.,17.]},
            "manual":{"type":"box"},"automatic":{"type":"box"},
            "free":{"type":"box","pos":[111.,333.]}
        }});
        let graph=minihou_core::Graph::from_value(&doc).unwrap();
        let mut state=State::default();
        state.moved.insert("manual".into(),pos2(-123.,345.));
        let before:HashMap<_,_>=positions_for(&state,&graph,&doc).into_iter().collect();
        let mut flags=HashMap::new();
        for id in ["authored","manual","automatic"] {flags.insert(id.into(),super::super::NodeFlags {locked:true,..Default::default()});}
        ctx.data_mut(|d| d.insert_persisted(state_id(path),state));
        for _ in 0..2 {
            tidy(&ctx,path,&doc,&flags);
            let state=ctx.data_mut(|d|d.get_persisted::<State>(state_id(path)).unwrap());
            let positions:HashMap<_,_>=positions_for(&state,&graph,&doc).into_iter().collect();
            for id in ["authored","manual","automatic"] {assert_eq!(positions[id],before[id],"locked {id} moved");}
            assert_ne!(positions["free"],before["free"]);
            let free=Rect::from_min_size(positions["free"],CARD);
            for id in ["authored","manual","automatic"] {assert!(!free.intersects(Rect::from_min_size(positions[id],CARD)));}
        }
        flags.clear(); tidy(&ctx,path,&doc,&flags);
        let state=ctx.data_mut(|d|d.get_persisted::<State>(state_id(path)).unwrap());
        let positions:HashMap<_,_>=positions_for(&state,&graph,&doc).into_iter().collect();
        assert_ne!(positions["authored"],before["authored"],"unlocking should allow tidy to move the node");
    }
    #[test]
    fn header_has_icon_toggles_without_list_or_zoom_text() {
        let f=Fixture::new(); let ctx=egui::Context::default();
        let mut state=State::default(); let mut actions=Actions::default();
        let r=Rect::from_min_size(Pos2::ZERO,vec2(800.,32.));
        let frame=|events, state:&mut State, actions:&mut Actions| ctx.run(egui::RawInput {events,screen_rect:Some(Rect::from_min_size(Pos2::ZERO,vec2(800.,400.))),..Default::default()},|ctx| {
            egui::CentralPanel::default().show(ctx,|ui|header_ui(ui,r,&f.view(),actions,state));
        });
        let output=frame(vec![],&mut state,&mut actions);
        for shape in output.shapes {
            if let egui::Shape::Text(text)=shape.shape {
                assert!(!["List","Snap","Links"].contains(&text.galley.text()));
                assert!(!text.galley.text().ends_with('%'));
            }
        }
        for (pos,links) in [(pos2(588.,16.),true),(pos2(620.,16.),false)] {
            for pressed in [true,false] {
                frame(vec![egui::Event::PointerMoved(pos),egui::Event::PointerButton {pos,button:PointerButton::Primary,pressed,modifiers:Default::default()}],&mut state,&mut actions);
            }
            if links {assert!(!state.show_links);assert!(actions.0.iter().any(|a|matches!(a,UiAction::LinkVisibility(false))));}
            else {assert!(!state.snap);}
        }
    }
    #[test]
    fn tidy_clears_manual_positions() {
        let mut state = State::default();
        state.moved.insert("box1".into(), pos2(123.0, 45.0));
        tidy_state(&mut state);
        assert!(state.moved.is_empty() && state.tidied && state.zoom < 0.0);
    }
    #[test]
    fn tidy_preserves_current_source_order_across_repeated_requests() {
        let f = Fixture::new();
        let doc = serde_json::json!({"nodes": {
            "grid":{"type":"grid","pos":[-500,20]},
            "noise":{"type":"noise","inputs":{"geo":"grid"}},
            "torus":{"type":"torus","pos":[500,20]},
            "transform":{"type":"transform","inputs":{"geo":"torus"}}
        }});
        let graph = minihou_core::Graph::from_value(&doc).unwrap();
        let mut view = f.view(); view.doc = &doc; view.graph = &graph;
        let mut state = State::default();
        state.moved.insert("grid".into(), pos2(600.,250.));
        state.moved.insert("torus".into(), pos2(-200.,450.));
        tidy_state(&mut state);
        // Header and dispatcher can both request Tidy before the next canvas frame.
        tidy_state(&mut state);
        let positions: HashMap<_, _> = node_positions(&state, &view).into_iter().collect();
        assert!(positions["torus"].x < positions["grid"].x);
        assert_eq!(positions["grid"].x, positions["noise"].x);
        assert_eq!(positions["torus"].x, positions["transform"].x);
        assert_eq!(positions["grid"].y, positions["torus"].y);
    }
    #[test]
    fn tidy_snaps_even_with_snap_disabled_and_overrides_authored_positions() {
        let f = Fixture::new();
        let mut doc = f.doc.value().clone();
        doc["nodes"][&f.graph.nodes[0].0]["pos"] = serde_json::json!([13.,17.]);
        let mut view = f.view();
        view.doc = &doc;
        let mut state = State::default();
        state.snap = false;
        assert_eq!(node_positions(&state, &view)[0].1, pos2(13.,17.));
        state.moved.insert(f.graph.nodes[0].0.clone(), pos2(45.,77.));
        tidy_state(&mut state);
        for (_, at) in node_positions(&state, &view) {
            assert_eq!(at, snap(at, 32.0, true, false));
        }
    }
    #[test]
    fn viewport_primitive_add_reserves_space_without_shifting_existing_cards() {
        let f = Fixture::new();
        let mut state = State::default();
        state.moved = node_positions(&state, &f.view()).into_iter().collect();
        let before = state.moved.clone();
        let (doc, _) = crate::graph_edit::add_at(f.doc.value(), "box", None, [0.; 3]).unwrap();
        let graph = minihou_core::Graph::from_value(&doc).unwrap();
        let mut view = f.view();
        view.doc = &doc;
        view.graph = &graph;
        let positions = node_positions(&state, &view);
        for (id, at) in &before { assert_eq!(positions.iter().find(|(node, _)| node == id).unwrap().1, *at); }
        for (id, at) in &positions {
            if before.contains_key(id) { continue; }
            for (peer, peer_at) in &positions {
                if peer != id { assert!(!Rect::from_min_size(*at, CARD).intersects(Rect::from_min_size(*peer_at, CARD))); }
            }
        }
    }
    #[test]
    fn wiggling_detaches_once_with_or_without_shift_and_leaves_card_at_drop() {
        for shift in [false, true] {
            let mut f = Fixture::new();
            f.selected = f.graph.nodes[0].0.clone();
            f.selected_set = std::collections::BTreeSet::from([f.selected.clone()]);
            let ctx = egui::Context::default();
            let mut state = State::default();
            state.snap = false;
            for (i, (id, _)) in f.graph.nodes.iter().enumerate() {
                state.moved.insert(id.clone(), pos2(96., 96. + i as f32 * 96.));
            }
            let initial = state.moved[&f.selected];
            let modifiers = egui::Modifiers { shift, ..Default::default() };
            let mut out = Actions::default();
            // Warm up, press, cross the drag threshold, then reverse rapidly three times.
            for (frame, x) in [206., 206., 256., 206., 256., 226., 226.].into_iter().enumerate() {
                let at = pos2(x, 124.);
                let mut events = vec![egui::Event::PointerMoved(at)];
                if frame == 1 || frame == 6 { events.push(egui::Event::PointerButton {
                    pos: at, button: PointerButton::Primary, pressed: frame == 1, modifiers,
                }); }
                let input = egui::RawInput { events, modifiers, time: Some(frame as f64 * 0.05),
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.,800.))), ..Default::default() };
                let _ = ctx.run(input, |ctx| { egui::CentralPanel::default().show(ctx, |ui| {
                    canvas_ui(ui, ui.max_rect(), &f.view(), &mut out, &mut state, Id::new("wiggle_regression"));
                }); });
            }
            assert_ne!(state.moved[&f.selected], initial, "gesture must actually drag a card");
            assert_eq!(out.0.iter().filter(|a| matches!(a, UiAction::DetachNode(id) if id == &f.selected)).count(), 1);
            assert!(state.drag.is_none());
            assert!(state.springs.is_none());
            assert!((state.moved[&f.selected].x - initial.x - 20.).abs() < 0.01);
        }
    }
    #[test]
    fn wiggle_ignores_jitter_slow_reversals_and_ordinary_dragging() {
        for samples in [vec![(0.,0.),(100.,0.1),(90.,0.2),(110.,0.3),(100.,0.4)],
            vec![(0.,0.),(50.,0.3),(0.,0.6),(50.,0.9),(0.,1.2)]] {
            let mut wiggle = Wiggle::default();
            for (x, time) in samples { assert!(!wiggle.update(x, time)); }
        }
        for sign in [-1., 1.] {
            let mut wiggle = Wiggle::default();
            for (frame, x) in [0.,50.,0.,50.,0.,50.].into_iter().enumerate() {
                assert_eq!(wiggle.update(x * sign, frame as f64 * 0.05), frame == 4);
            }
        }
    }
    #[test]
    fn physics_toggle_survives_persisted_state_and_tidy() {
        let ctx = egui::Context::default();
        let path = Path::new("physics_toggle.json");
        let mut state = State::default();
        assert!(state.physics, "connected drag physics is on by default");
        state.physics = false;
        ctx.data_mut(|d| d.insert_persisted(state_id(path), state));
        let f=Fixture::new();
        tidy(&ctx, path, f.doc.value(), &HashMap::new());
        assert!(!ctx.data_mut(|d| d.get_persisted::<State>(state_id(path)).unwrap().physics));
    }
    #[test]
    fn canvas_shift_drag_drives_springs_and_continues_after_release() {
        for (physics, shift) in [(true, true), (false, true), (true, false)] {
            let f = Fixture::new();
            let doc = serde_json::json!({"nodes": {
                "a":{"type":"grid"}, "b":{"type":"normal","inputs":{"geo":"a"}},
                "c":{"type":"output","inputs":{"geo":"b"}}, "alone":{"type":"torus"}
            }});
            let graph = minihou_core::Graph::from_value(&doc).unwrap();
            let selected = std::collections::BTreeSet::from(["a".to_owned()]);
            let mut view = f.view();
            view.doc = &doc; view.graph = &graph; view.selected = "a"; view.selected_set = &selected;
            let ctx = egui::Context::default();
            let mut state = State::default();
            state.snap = false; state.physics = physics;
            state.moved = [("a", pos2(96.,96.)), ("b", pos2(96.,224.)),
                ("c", pos2(96.,352.)), ("alone", pos2(800.,96.))]
                .into_iter().map(|(id, at)| (id.to_owned(), at)).collect();
            let modifiers = egui::Modifiers { shift, ..Default::default() };
            let mut out = Actions::default();
            let mut release_x = 0.0;
            for frame in 0..126 {
                let x = [206.,206.,270.,334.,334.,334.].get(frame).copied().unwrap_or(334.);
                let at = pos2(x,124.);
                let mut events = vec![egui::Event::PointerMoved(at)];
                if frame == 1 || frame == 5 { events.push(egui::Event::PointerButton {
                    pos: at, button: PointerButton::Primary, pressed: frame == 1, modifiers,
                }); }
                let input = egui::RawInput { events, modifiers, time: Some(frame as f64 / 60.),
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.,800.))), ..Default::default() };
                let _ = ctx.run(input, |ctx| { egui::CentralPanel::default().show(ctx, |ui| {
                    canvas_ui(ui, ui.max_rect(), &view, &mut out, &mut state, Id::new("spring_drag"));
                }); });
                if frame == 5 {
                    release_x = state.moved["b"].x;
                    assert!(state.drag.is_none());
                    assert_eq!(state.springs.is_some(), physics && shift);
                }
            }
            assert!(state.moved["a"].x > 96., "test must drag the anchor");
            if physics && shift {
                assert!(state.moved["b"].x > release_x + 1., "neighbours must keep following after release");
                assert!(state.moved["c"].x > 96.);
            } else {
                assert_eq!(state.moved["b"], pos2(96.,224.));
                assert_eq!(state.moved["c"], pos2(96.,352.));
            }
            assert_eq!(state.moved["alone"], pos2(800.,96.));
            assert!(!out.0.iter().any(|a| matches!(a, UiAction::Edit { .. } | UiAction::DisconnectAll(_) | UiAction::DetachNode(_)
                | UiAction::DisconnectInput { .. } | UiAction::ConnectNodes { .. })));
        }
    }
    #[test]
    fn exposure_badge_uses_document_interface() {
        let doc = serde_json::json!({"exposed":[{"node":"box1","param":"size","name":"Size"}]});
        assert!(exposed_to_blender(&doc, "box1"));
        assert!(!exposed_to_blender(&doc, "box2"));
        assert!(!exposed_to_blender(&serde_json::json!({}), "box1"));
    }
    #[test]
    fn open_add_menu_no_deadlock() {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let ctx = egui::Context::default();
            open_add_menu(&ctx, Path::new("scene.json"));
            tx.send(()).unwrap();
        });
        assert!(rx.recv_timeout(std::time::Duration::from_millis(500)).is_ok(), "open_add_menu deadlocked on nested ctx lock");
    }
    #[test]
    fn repeated_tab_does_not_close_menu() {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let ctx = egui::Context::default();
            let path = Path::new("fifty_cycles.json");
            let id = state_id(path);
            for i in 0..50 {
                open_add_menu(&ctx, path);
                let is_open = ctx.data_mut(|d| d.get_persisted::<State>(id).and_then(|s| s.menu).is_some());
                assert!(is_open, "Failed to open menu on iteration {i}");
                open_add_menu(&ctx, path);
                let is_open = ctx.data_mut(|d| d.get_persisted::<State>(id).and_then(|s| s.menu).is_some());
                assert!(is_open, "Repeated Tab closed menu on iteration {i}");
            }
            tx.send(()).unwrap();
        });
        assert!(rx.recv_timeout(std::time::Duration::from_secs(5)).is_ok(), "Menu 50 open/close cycles timed out!");
    }
    #[test]
    fn viewport_tab_in_edit_mode_opens_the_add_menu() {
        use crate::ui::keymap::{Command, KeyContext};
        // Tab is the node menu in edit mode too; the object / edit toggle is Ctrl+Tab (Command::ToggleEdit)
        let kctx_edit = KeyContext { viewport_hovered: true, network_hovered: false, in_edit_mode: true, selected: Some("box1") };
        assert!(matches!(Command::AddMenu.to_action(&kctx_edit), Some(UiAction::Command(cmd)) if cmd == "add_menu"));
        assert!(matches!(Command::ToggleEdit.to_action(&kctx_edit), Some(UiAction::Command(cmd)) if cmd == "toggle_edit"));

        let kctx_obj = KeyContext { viewport_hovered: true, network_hovered: false, in_edit_mode: false, selected: Some("box1") };
        assert!(matches!(Command::AddMenu.to_action(&kctx_obj), Some(UiAction::Command(cmd)) if cmd == "add_menu"));

        let kctx_net = KeyContext { viewport_hovered: false, network_hovered: true, in_edit_mode: true, selected: Some("box1") };
        assert!(matches!(Command::AddMenu.to_action(&kctx_net), Some(UiAction::Command(cmd)) if cmd == "add_menu"));
    }
    #[test]
    fn headless_tab_opens_menu_filters_adds_node_and_closes() {
        use crate::ui::keymap::{KeyContext, Keymap};
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let f = Fixture::new();
            let ctx = egui::Context::default();
            f.theme.apply(&ctx);
            let path = std::path::Path::new("untitled.json");
            let keymap = Keymap::blender();

            // Frame 1: Tab over network area opens menu without deadlocking
            let mut out = Actions::default();
            let p_network = pos2(200.0, 300.0);
            let input1 = egui::RawInput {
                events: vec![
                    egui::Event::PointerMoved(p_network),
                    egui::Event::Key { key: egui::Key::Tab, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() },
                ],
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1280., 800.))),
                ..Default::default()
            };
            let v = f.view();
            let _ = ctx.run(input1, |ctx| {
                let kctx = KeyContext { viewport_hovered: false, network_hovered: true, in_edit_mode: false, selected: Some(&f.selected) };
                for a in keymap.poll(ctx, &kctx) { out.push(a); }
                egui::SidePanel::left("network").default_width(400.).show(ctx, |ui| {
                    show(ui, &v, &mut out);
                });
            });
            for a in &out.0 {
                if let UiAction::Command(cmd) = a {
                    if cmd == "add_menu" { open_add_menu(&ctx, path); }
                }
            }
            let id = state_id(path);
            let menu_is_open = ctx.data_mut(|d| d.get_persisted::<State>(id).and_then(|s| s.menu).is_some());
            assert!(menu_is_open, "Tab should open menu");

            // Frame 2: Show menu so TextEdit requests focus
            let input2 = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1280., 800.))), ..Default::default() };
            let _ = ctx.run(input2, |ctx| {
                egui::SidePanel::left("network").default_width(400.).show(ctx, |ui| {
                    show(ui, &v, &mut Actions::default());
                });
            });

            // Frame 3: Type "transform" into the focused TextEdit to filter
            let input3 = egui::RawInput {
                events: vec![
                    egui::Event::Text("transform".into()),
                ],
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1280., 800.))),
                ..Default::default()
            };
            let _ = ctx.run(input3, |ctx| {
                egui::SidePanel::left("network").default_width(400.).show(ctx, |ui| {
                    show(ui, &v, &mut Actions::default());
                });
            });
            let query = ctx.data_mut(|d| d.get_persisted::<State>(id).and_then(|s| s.menu).map(|m| m.query().to_owned())).unwrap_or_default();
            assert_eq!(query, "transform");

            // Frame 4: Press Enter to add the filtered node
            let mut out4 = Actions::default();
            let input4 = egui::RawInput {
                events: vec![
                    egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() },
                ],
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1280., 800.))),
                ..Default::default()
            };
            let _ = ctx.run(input4, |ctx| {
                egui::SidePanel::left("network").default_width(400.).show(ctx, |ui| {
                    show(ui, &v, &mut out4);
                });
            });
            assert!(out4.0.iter().any(|a| matches!(a, UiAction::AddNode { node_type, .. } if node_type == "transform")));
            let menu_is_open = ctx.data_mut(|d| d.get_persisted::<State>(id).and_then(|s| s.menu).is_some());
            assert!(!menu_is_open, "Enter should close menu");

            // Frame 4: Open menu again, then close with Tab
            open_add_menu(&ctx, path);
            assert!(ctx.data_mut(|d| d.get_persisted::<State>(id).and_then(|s| s.menu).is_some()));
            let input4a = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1280., 800.))), ..Default::default() };
            let _ = ctx.run(input4a, |ctx| {
                egui::SidePanel::left("network").default_width(400.).show(ctx, |ui| {
                    show(ui, &v, &mut Actions::default());
                });
            });
            let mut out4b = Actions::default();
            let input4b = egui::RawInput {
                events: vec![
                    egui::Event::Key { key: egui::Key::Tab, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() },
                ],
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1280., 800.))),
                ..Default::default()
            };
            let _ = ctx.run(input4b, |ctx| {
                let kctx = KeyContext { viewport_hovered: false, network_hovered: true, in_edit_mode: false, selected: Some(&f.selected) };
                for a in keymap.poll(ctx, &kctx) { out4b.push(a); }
                egui::SidePanel::left("network").default_width(400.).show(ctx, |ui| {
                    show(ui, &v, &mut out4b);
                });
            });
            for a in &out4b.0 {
                if let UiAction::Command(cmd) = a {
                    if cmd == "add_menu" { open_add_menu(&ctx, path); }
                }
            }
            assert!(ctx.data_mut(|d| d.get_persisted::<State>(id).and_then(|s| s.menu).is_some()), "Tab should leave open menu focused");

            // Frame 5: Open menu again, then close with Escape
            open_add_menu(&ctx, path);
            assert!(ctx.data_mut(|d| d.get_persisted::<State>(id).and_then(|s| s.menu).is_some()));
            let mut out5 = Actions::default();
            let input5 = egui::RawInput {
                events: vec![
                    egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() },
                ],
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1280., 800.))),
                ..Default::default()
            };
            let _ = ctx.run(input5, |ctx| {
                let kctx = KeyContext { viewport_hovered: false, network_hovered: true, in_edit_mode: false, selected: Some(&f.selected) };
                for a in keymap.poll(ctx, &kctx) { out5.push(a); }
                egui::SidePanel::left("network").default_width(400.).show(ctx, |ui| {
                    show(ui, &v, &mut out5);
                });
            });
            assert!(ctx.data_mut(|d| d.get_persisted::<State>(id).and_then(|s| s.menu).is_none()), "Escape should close menu");

            tx.send(()).unwrap();
        });
        assert!(rx.recv_timeout(std::time::Duration::from_secs(5)).is_ok(), "Test timed out!");
    }

    #[test]
    fn tab_menu_node_placed_at_mouse_position_when_no_selection_and_unchanged_with_selection() {
        let ctx = egui::Context::default();
        let path = Path::new("tab_placement_test.json");
        let canvas = Rect::from_min_size(pos2(100., 100.), vec2(800., 600.));
        let old = serde_json::json!({
            "version": 1,
            "nodes": {
                "a": { "type": "box", "pos": [320., 96.] }
            }
        });

        // 1. Without selection: mouse at (390., 510.) on screen with pan (50., 50.) and zoom 2.0.
        // In graph coordinates: ((390 - 100 - 50) / 2.0, (510 - 100 - 50) / 2.0) = (120., 180.)
        let mut state = State::default();
        state.canvas = Some(canvas);
        state.pan = vec2(50., 50.);
        state.zoom = 2.0;
        ctx.data_mut(|d| d.insert_persisted(state_id(path), state));

        let mouse_screen = pos2(390., 510.);
        let _ = ctx.run(egui::RawInput {
            events: vec![egui::Event::PointerMoved(mouse_screen)],
            ..Default::default()
        }, |_| {});
        open_add_menu(&ctx, path);

        let stored_graph_pos = ctx.data_mut(|d| d.get_persisted::<State>(state_id(path)).unwrap().add_pos);
        assert_eq!(stored_graph_pos, Some(pos2(120., 180.)));

        // With no selection (after = None), node added from menu lands at the stored mouse position
        let (mut next_no_sel, id_no_sel) = crate::graph_edit::add(&old, "sphere", None).unwrap();
        let _ = position_add(&ctx, path, &old, &mut next_no_sel, None).unwrap();
        assert_eq!(next_no_sel["nodes"][&id_no_sel]["pos"], serde_json::json!([120., 180.]));

        // 2. With selection: same pointer position captured when menu opens
        let mut state2 = State::default();
        state2.canvas = Some(canvas);
        state2.pan = vec2(50., 50.);
        state2.zoom = 2.0;
        ctx.data_mut(|d| d.insert_persisted(state_id(path), state2));
        open_add_menu(&ctx, path);

        // With a selection (after = Some("a")), placement is unchanged by mouse position
        let (mut next_sel, id_sel) = crate::graph_edit::add(&old, "sphere", Some("a")).unwrap();
        let _ = position_add(&ctx, path, &old, &mut next_sel, Some("a")).unwrap();
        assert_ne!(next_sel["nodes"][&id_sel]["pos"], serde_json::json!([120., 180.]));
        assert_eq!(next_sel["nodes"][&id_sel]["pos"], serde_json::json!([320., 224.]));
    }
