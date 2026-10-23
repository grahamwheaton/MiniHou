    use super::*;
    use crate::ui::testing::Fixture;
    fn fixture() -> (Fixture, State, WireRef) {
        let mut f=Fixture::new();
        f.doc=crate::graph_edit::validated(serde_json::json!({"nodes":{
            "a":{"type":"box", "pos":[100,100]},
            "out":{"type":"output", "inputs":{"geo":"a"}, "pos":[100,400]}
        },"display":"out"})).unwrap();
        f.graph=minihou_core::Graph::from_value(f.doc.value()).unwrap();
        f.selected="out".into(); f.selected_set=BTreeSet::from(["out".into()]);
        let state=State { physics:false, ..Default::default() };
        (f,state,WireRef { from:"a".into(),to:"out".into(),port:"geo".into() })
    }
    fn frame(ctx: &egui::Context, f: &Fixture, state: &mut State, at: Pos2,
        button: Option<(PointerButton,bool)>, modifiers: egui::Modifiers) -> Vec<UiAction> {
        let r=Rect::from_min_size(Pos2::ZERO,vec2(1000.,800.));
        let mut events=vec![egui::Event::PointerMoved(at)];
        if let Some((button,pressed))=button {
            events.push(egui::Event::PointerButton { pos:at,button,pressed,modifiers });
        }
        let input=egui::RawInput { screen_rect:Some(r),events,modifiers,..Default::default() };
        let mut out=Actions::default();
        let _=ctx.run(input,|ctx| { egui::CentralPanel::default().show(ctx,|ui| {
            canvas_ui(ui,r,&f.view(),&mut out,state,Id::new("noodle_canvas_test"));
        }); });
        out.0
    }
    #[test]
    fn noodle_pickup_canvas_disconnect_reconnect_and_rmb_cancel() {
        for mode in 0..4 {
            let (mut f, mut state, wire) = fixture();
            let mut doc = f.doc.value().clone();
            doc["nodes"]["t"] = serde_json::json!({"type":"transform","pos":[400,400]});
            f.doc = crate::graph_edit::validated(doc).unwrap();
            f.graph = minihou_core::Graph::from_value(f.doc.value()).unwrap();
            let ctx = egui::Context::default(); let input = pos2(210.,400.);
            frame(&ctx,&f,&mut state,input,None,egui::Modifiers::NONE);
            frame(&ctx,&f,&mut state,input,Some((PointerButton::Primary,true)),egui::Modifiers::NONE);
            assert_eq!(state.pickup.as_ref().unwrap().original,wire);
            assert_eq!(state.wire,Some(AddConnection::Output("a".into())));
            let end = if mode == 1 { pos2(510.,400.) } else { pos2(700.,300.) };
            frame(&ctx,&f,&mut state,end,None,egui::Modifiers::NONE);
            if mode == 2 {
                let actions = frame(&ctx,&f,&mut state,end,Some((PointerButton::Secondary,true)),egui::Modifiers::NONE);
                assert!(actions.is_empty()); assert!(state.menu.is_none()); assert!(state.pickup.is_none());
            }
            if mode == 3 {
                let input = egui::RawInput { events: vec![egui::Event::Key { key: egui::Key::Escape,
                    physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }], ..Default::default() };
                let mut out = Actions::default();
                let _ = ctx.run(input, |ctx| { egui::CentralPanel::default().show(ctx, |ui| {
                    canvas_ui(ui,Rect::from_min_size(Pos2::ZERO,vec2(1000.,800.)),&f.view(),&mut out,&mut state,Id::new("noodle_canvas_test"));
                }); });
                assert!(out.0.is_empty()); assert!(state.pickup.is_none());
            }
            let actions = frame(&ctx,&f,&mut state,end,Some((PointerButton::Primary,false)),egui::Modifiers::NONE);
            if mode >= 2 { assert!(actions.is_empty()); }
            else {
                assert!(matches!(actions.as_slice(),[UiAction::NetworkGesture { before, next, .. }]
                    if before == f.doc.value() && next["nodes"]["out"]["inputs"].get("geo").is_none()
                    && (mode == 0 || next["nodes"]["t"]["inputs"]["geo"] == "a")));
                assert!(state.menu.is_none());
            }
        }
    }
    #[test]
    fn noodle_drop_canvas_move_insert_and_shift_suppression() {
        for physics in [false,true] {
            let (mut f, mut state, _) = fixture(); state.snap = false;
            let mut doc = f.doc.value().clone();
            doc["nodes"]["t"] = serde_json::json!({"type":"transform","pos":[400,240]});
            f.doc = crate::graph_edit::validated(doc).unwrap();
            f.graph = minihou_core::Graph::from_value(f.doc.value()).unwrap();
            f.selected = "t".into(); f.selected_set = BTreeSet::from(["t".into()]);
            let ctx = egui::Context::default(); let start = pos2(510.,268.); let end = pos2(210.,268.);
            frame(&ctx,&f,&mut state,start,None,egui::Modifiers::NONE);
            frame(&ctx,&f,&mut state,start,Some((PointerButton::Primary,true)),egui::Modifiers::NONE);
            frame(&ctx,&f,&mut state,end,None,egui::Modifiers::NONE);
            assert!(state.drag.is_some());
            state.drag_physics = physics;
            let actions = frame(&ctx,&f,&mut state,end,Some((PointerButton::Primary,false)),egui::Modifiers::NONE);
            let edits: Vec<_> = actions.iter().filter(|a| matches!(a,UiAction::NetworkGesture { .. })).collect();
            if physics { assert!(edits.is_empty()); }
            else { assert!(matches!(edits.as_slice(),[UiAction::NetworkGesture { before,next,.. }]
                if before["nodes"]["t"]["pos"] == serde_json::json!([400.,240.])
                && next["nodes"]["out"]["inputs"]["geo"] == "t" && next["nodes"]["t"]["inputs"]["geo"] == "a")); }
        }
    }
    #[test]
    fn noodle_shelf_and_tab_payload_drop_dispatch_position() {
        let (f,mut state,_) = fixture(); let ctx = egui::Context::default();
        let at = pos2(100.,240.);
        frame(&ctx,&f,&mut state,at,None,egui::Modifiers::NONE);
        frame(&ctx,&f,&mut state,at,Some((PointerButton::Primary,true)),egui::Modifiers::NONE);
        state.menu = Some(network_menu::MenuState::new(pos2(700.,500.),None,true));
        egui::DragAndDrop::set_payload(&ctx,super::super::ToolDrag("transform".into()));
        frame(&ctx,&f,&mut state,at,None,egui::Modifiers::NONE);
        let actions = frame(&ctx,&f,&mut state,at,Some((PointerButton::Primary,false)),egui::Modifiers::NONE);
        assert!(matches!(actions.as_slice(),[UiAction::AddNetworkNode { node_type,pos, .. }]
            if node_type == "transform" && *pos == [0.,224.]), "actions: {actions:?}");
        assert!(state.menu.is_none());
    }
    #[test]
    fn noodle_shelf_drop_splices_even_when_collision_moves_card_past_wire() {
        let (f, mut state, expected_wire) = fixture();
        let ctx = egui::Context::default();
        let at = pos2(210., 360.);
        frame(&ctx, &f, &mut state, at, None, egui::Modifiers::NONE);
        frame(&ctx, &f, &mut state, at, Some((PointerButton::Primary, true)), egui::Modifiers::NONE);
        egui::DragAndDrop::set_payload(&ctx, super::super::ToolDrag("transform".into()));
        let actions = frame(&ctx, &f, &mut state, at, Some((PointerButton::Primary, false)), egui::Modifiers::NONE);
        let [UiAction::AddNetworkNode { pos, wire, .. }] = actions.as_slice() else { panic!("actions: {actions:?}"); };
        assert_eq!(wire.as_ref(), Some(&expected_wire));
        let (mut next, id) = crate::graph_edit::add(f.doc.value(), "transform", None).unwrap();
        next["nodes"][&id]["pos"] = serde_json::json!(pos);
        assert!(insert_added(&ctx, Path::new("drop.mhou"), f.doc.value(), &mut next, &id, *pos, wire.as_ref()));
        assert_eq!(next["nodes"]["out"]["inputs"]["geo"], id);
        assert_eq!(next["nodes"][&id]["inputs"]["geo"], "a");
    }
    #[test]
    fn noodle_insertion_picks_closest_centre_and_shelf_add_splices() {
        let (f,_,wire) = fixture();
        let (mut value,id) = crate::graph_edit::add(f.doc.value(),"transform",None).unwrap();
        let near = wire.clone(); let mut far = wire.clone(); far.to = "other".into();
        value["nodes"]["other"] = serde_json::json!({"type":"transform","inputs":{"geo":"a"}});
        let rect = Rect::from_min_max(pos2(100.,240.),pos2(320.,296.));
        let wires = vec![(far,vec![pos2(110.,200.),pos2(110.,350.)]),(near.clone(),vec![pos2(210.,200.),pos2(210.,350.)])];
        assert_eq!(insertion_candidate(&value,&id,rect,&wires),Some(near));
        let ctx = egui::Context::default();
        assert!(!insert_added(&ctx,Path::new("drop.mhou"),f.doc.value(),&mut value,&id,[100.,240.],None));
        assert_eq!(value["nodes"]["out"]["inputs"]["geo"],"a");
        assert_eq!(value["display"],id.as_str());
    }
    #[test]
    fn noodle_ctrl_rmb_cuts_on_release_and_plain_rmb_opens_menu() {
        let (f,mut state,wire)=fixture(); let ctx=egui::Context::default();
        let start=pos2(180.,278.); let end=pos2(240.,278.);
        frame(&ctx,&f,&mut state,start,None,egui::Modifiers::CTRL);
        assert!(frame(&ctx,&f,&mut state,start,Some((PointerButton::Secondary,true)),egui::Modifiers::CTRL).is_empty());
        assert!(state.menu.is_none());
        assert!(frame(&ctx,&f,&mut state,end,None,egui::Modifiers::CTRL).is_empty());
        assert!(state.cut.as_ref().unwrap().hits.contains(&wire));
        let actions=frame(&ctx,&f,&mut state,end,Some((PointerButton::Secondary,false)),egui::Modifiers::CTRL);
        assert!(matches!(actions.as_slice(),[UiAction::DisconnectWires(wires)] if wires==&BTreeSet::from([wire])));
        assert!(state.menu.is_none());
        frame(&ctx,&f,&mut state,start,Some((PointerButton::Secondary,true)),egui::Modifiers::NONE);
        assert!(state.menu.is_some());
    }
    #[test]
    fn noodle_ctrl_rmb_subthreshold_is_a_context_click() {
        let (f,mut state,_)=fixture(); let ctx=egui::Context::default(); let at=pos2(180.,278.);
        frame(&ctx,&f,&mut state,at,None,egui::Modifiers::CTRL);
        frame(&ctx,&f,&mut state,at,Some((PointerButton::Secondary,true)),egui::Modifiers::CTRL);
        let actions=frame(&ctx,&f,&mut state,at+vec2(2.,0.),Some((PointerButton::Secondary,false)),egui::Modifiers::CTRL);
        assert!(actions.is_empty()); assert!(state.menu.is_some()); assert!(state.cut.is_none());
    }
    #[test]
    fn noodle_marquee_and_click_selection_modifiers() {
        for modifiers in [egui::Modifiers::NONE,egui::Modifiers::CTRL,egui::Modifiers::CTRL] {
            let (f,mut state,wire)=fixture(); let ctx=egui::Context::default();
            let other=WireRef { from:"other".into(),..wire.clone() };
            state.selected_wires=BTreeSet::from([wire.clone(),other.clone()]);
            let start=pos2(180.,260.); let end=pos2(240.,300.);
            frame(&ctx,&f,&mut state,start,None,modifiers);
            frame(&ctx,&f,&mut state,start,Some((PointerButton::Primary,true)),modifiers);
            frame(&ctx,&f,&mut state,end,None,modifiers);
            let actions=frame(&ctx,&f,&mut state,end,Some((PointerButton::Primary,false)),modifiers);
            let expected=if modifiers.ctrl { BTreeSet::from([other]) }
                else if modifiers.shift { BTreeSet::from([wire.clone(),other]) }
                else { BTreeSet::from([wire.clone()]) };
            assert_eq!(state.selected_wires,expected);
            assert!(actions.iter().any(|action| matches!(action,UiAction::SelectSet { ids,extend:false }
                if if modifiers.shift || modifiers.ctrl { ids==&vec!["out".to_owned()] } else { ids.is_empty() })));
        }
        let (f,mut state,wire)=fixture(); let ctx=egui::Context::default(); let at=pos2(210.,278.);
        frame(&ctx,&f,&mut state,at,None,egui::Modifiers::NONE);
        frame(&ctx,&f,&mut state,at,Some((PointerButton::Primary,true)),egui::Modifiers::NONE);
        let actions=frame(&ctx,&f,&mut state,at,Some((PointerButton::Primary,false)),egui::Modifiers::NONE);
        assert_eq!(state.selected_wires,BTreeSet::from([wire]));
        assert!(matches!(actions.as_slice(),[UiAction::SelectSet { ids,extend:false }] if ids.is_empty()));
    }

    #[test]
    fn noodle_cut_cancels_on_escape_focus_loss_and_pointer_loss() {
        for cancellation in 0..3 {
            let (f,mut state,_)=fixture(); let ctx=egui::Context::default();
            let start=pos2(180.,278.); let end=pos2(240.,278.);
            frame(&ctx,&f,&mut state,start,None,egui::Modifiers::CTRL);
            frame(&ctx,&f,&mut state,start,Some((PointerButton::Secondary,true)),egui::Modifiers::CTRL);
            frame(&ctx,&f,&mut state,end,None,egui::Modifiers::CTRL);
            assert!(state.cut.as_ref().unwrap().active);
            let event=if cancellation==0 { egui::Event::Key { key:egui::Key::Escape,
                physical_key:None,pressed:true,repeat:false,modifiers:egui::Modifiers::CTRL } }
                else { egui::Event::PointerGone };
            let input=egui::RawInput { focused:cancellation!=1,events:vec![event],..Default::default() };
            let mut out=Actions::default();
            let _=ctx.run(input,|ctx| { egui::CentralPanel::default().show(ctx,|ui| {
                canvas_ui(ui,Rect::from_min_size(Pos2::ZERO,vec2(1000.,800.)),&f.view(),&mut out,&mut state,Id::new("noodle_canvas_test"));
            }); });
            assert!(state.cut.is_none()); assert!(out.0.is_empty());
            let actions=frame(&ctx,&f,&mut state,end,Some((PointerButton::Secondary,false)),egui::Modifiers::SHIFT);
            assert!(!actions.iter().any(|a| matches!(a,UiAction::DisconnectWires(_))));
        }
    }
    #[test]
    fn noodle_ctrl_marquee_subtracts_nodes_and_wires_together() {
        let (f,mut state,wire)=fixture(); let ctx=egui::Context::default();
        state.selected_wires.insert(wire);
        let start=pos2(80.,260.); let end=pos2(340.,500.); let mods=egui::Modifiers::CTRL;
        frame(&ctx,&f,&mut state,start,None,mods);
        frame(&ctx,&f,&mut state,start,Some((PointerButton::Primary,true)),mods);
        frame(&ctx,&f,&mut state,end,None,mods);
        let actions=frame(&ctx,&f,&mut state,end,Some((PointerButton::Primary,false)),mods);
        assert!(state.selected_wires.is_empty());
        assert!(actions.iter().any(|a| matches!(a,UiAction::SelectSet { ids,extend:false } if ids.is_empty())));
    }
    #[test]
    fn noodle_selection_is_ui_only_and_clears_on_graph_change() {
        let ctx=egui::Context::default();
        let path=Path::new("wire-selection-test.mhou");
        let doc=serde_json::json!({"nodes":{"a":{"type":"box"}}});
        let mut state=State::default();
        sync_wire_selection(&mut state,&doc);
        let wire=WireRef { from:"a".into(),to:"out".into(),port:"geo".into() };
        state.selected_wires.insert(wire.clone());
        state.cut=Some(Cut::new(Pos2::ZERO));
        sync_wire_selection(&mut state,&doc);
        assert!(state.selected_wires.contains(&wire));
        ctx.data_mut(|d| d.insert_persisted(state_id(path),state.clone()));
        assert_eq!(take_selected_wires(&ctx,path,&doc),BTreeSet::from([wire.clone()]));
        assert!(take_selected_wires(&ctx,path,&doc).is_empty());
        let changed=serde_json::json!({"nodes":{"b":{"type":"box"}}});
        sync_wire_selection(&mut state,&changed);
        assert!(state.selected_wires.is_empty());
        assert!(state.cut.is_none());
        ctx.data_mut(|d| d.insert_persisted(state_id(path),state));
        assert!(take_selected_wires(&ctx,path,&doc).is_empty());
    }
