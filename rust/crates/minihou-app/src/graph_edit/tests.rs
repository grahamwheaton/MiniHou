    use super::*;
    #[test] fn viewport_insertion_splices_branches_pushes_stream_and_undoes_once() {
        let base = json!({"nodes": {
            "source":{"type":"box","pos":[96,32]},
            "middle":{"type":"transform","inputs":{"geo":"source"},"pos":[96,160]},
            "out":{"type":"output","inputs":{"geo":"middle"},"pos":[96,288]},
            "branch":{"type":"normal","inputs":{"geo":"middle"},"pos":[416,288]}
        },"display":["out","branch"]});
        for source in ["out", "middle", "source"] {
            for ty in ["transform", "polyextrude", "inset", "polybevel", "pushpull", "mirror", "grow", "loop_cut"] {
                let (next, id) = add(&base, ty, Some(source)).unwrap();
                let anchor = if source == "out" { "middle" } else { source };
                assert_eq!(next["nodes"][&id]["inputs"]["geo"], anchor, "{source} {ty}");
                assert_eq!(next["nodes"][&id]["pos"][0].as_f64(), base["nodes"][if source == "out" { "out" } else { source }]["pos"][0].as_f64());
                if source == "out" { assert_eq!(next["nodes"]["out"]["inputs"]["geo"], id); }
                else {
                    for consumer in ["middle", "out", "branch"] {
                        if base["nodes"][consumer]["inputs"]["geo"] == source { assert_eq!(next["nodes"][consumer]["inputs"]["geo"], id); }
                    }
                }
                assert_eq!(next["display"], base["display"]);
                let origin_y = base["nodes"][if source == "out" { "out" } else { source }]["pos"][1].as_f64().unwrap();
                assert_eq!(next["nodes"][&id]["pos"][1].as_f64().unwrap(), origin_y + if source == "out" { 0.0 } else { 128.0 });
                if source == "out" { assert_eq!(next["nodes"]["out"]["pos"], json!([96.,416.])); }
                if source == "middle" { assert_eq!(next["nodes"]["out"]["pos"], json!([96.,416.])); }
                let mut doc = validated(base.clone()).unwrap();
                let mut history = minihou_app::History::default();
                history.replace(&mut doc, validated(next).unwrap());
                history.undo(&mut doc).unwrap();
                assert_eq!(doc.value(), &base);
            }
        }
    }
    #[test] fn output_insertion_materializes_unpositioned_stream() {
        let base = json!({"nodes":{"box1":{"type":"box"},"out":{"type":"output","inputs":{"geo":"box1"}}},"display":"out"});
        let (next,id) = add(&base,"transform",Some("out")).unwrap();
        assert_eq!(next["nodes"][&id]["pos"], json!([96.,160.]));
        assert_eq!(next["nodes"]["out"]["pos"], json!([96.,288.]));
        assert_eq!(next["nodes"]["out"]["inputs"]["geo"], id);
    }
    #[test] fn tab_boolean_wires_first_selected_object_as_the_cutter_and_cooks() {
        let base = json!({"version":1,"display":["cutter","subject"],"nodes":{
            "cutter":{"type":"box","params":{"size":[1.,1.,1.]}},
            "subject":{"type":"box","params":{"size":[2.,2.,2.]}}
        }});
        // `subject` is active because it was selected after `cutter`.
        let selected = std::collections::BTreeSet::from(["cutter".to_owned(), "subject".to_owned()]);
        assert_eq!(boolean_input_order(&selected, "subject"), Some(["subject".into(), "cutter".into()]));
        let (next, id) = add_selected_boolean(&base, &selected, "subject").unwrap();
        assert_eq!(next["nodes"][&id]["inputs"], json!({"a":"subject", "b":"cutter"}));
        assert_eq!(next["nodes"][&id]["params"]["operation"], "subtract");
        assert!(minihou_core::Graph::from_value(&next).unwrap().cook(Some(&id)).is_ok());
    }
    #[test] fn repairs_operator_hanging_after_output() {
        let mut bad = json!({"nodes":{"source":{"type":"box","pos":[96,32]},
            "out":{"type":"output","inputs":{"geo":"source"},"pos":[96,160]},
            "transform1":{"type":"transform","inputs":{"geo":"out"},"pos":[96,288]}},"display":"out"});
        assert_eq!(repair_output_hangs(&mut bad), 1);
        assert_eq!(bad["nodes"]["transform1"]["inputs"]["geo"], "source");
        assert_eq!(bad["nodes"]["out"]["inputs"]["geo"], "transform1");
        assert_eq!(bad["nodes"]["transform1"]["pos"], json!([96.,160.]));
        assert_eq!(bad["nodes"]["out"]["pos"], json!([96.,288.]));
    }
    #[test] fn measurement_deletion_and_clear_are_undoable_document_edits() {
        let mut base=graph(); base["nodes"]["box1"]["measurement_identity"]=json!("stable");
        base["measurements"]=json!([{"id":"one","points":[[0,0,0],[1,0,0]],"refs":[{"node":"box1","identity":"stable","point":0,"point_count":8},null],"a":[0,0,0],"b":[1,0,0]},{"a":[0,0,0],"b":[2,0,0]}]);
        let mut doc=validated(base.clone()).unwrap();
        let mut history=minihou_app::History::default();
        let one=delete_measurement(doc.value(),0).unwrap();
        assert_eq!(one["measurements"].as_array().unwrap().len(),1);
        history.replace(&mut doc,validated(one).unwrap());
        let empty=validated(clear_measurements(doc.value()).unwrap()).unwrap();
        history.replace(&mut doc,empty);
        assert!(doc.value()["measurements"].as_array().unwrap().is_empty());
        history.undo(&mut doc).unwrap();
        assert_eq!(doc.value()["measurements"].as_array().unwrap().len(),1);
        history.undo(&mut doc).unwrap();
        assert_eq!(doc.value()["measurements"].as_array().unwrap().len(),2);
    }
    #[test] fn renaming_a_blender_object_pins_its_picker_id() {
        let g=json!({"display":"a","nodes":{"a":{"type":"blender_object"}}});
        let once=rename(&g,"a","b").unwrap();
        assert_eq!(once["nodes"]["b"]["picker_id"],"a");
        let twice=rename(&once,"b","c").unwrap();
        assert_eq!(twice["nodes"]["c"]["picker_id"],"a");
        assert_eq!(minihou_app::modifier_inputs::pickers(&twice),vec![("c".to_string(),"a".to_string())]);
        assert!(rename(&json!({"nodes":{"x":{"type":"box"}}}),"x","y").unwrap()["nodes"]["y"].get("picker_id").is_none());
    }
    #[test] fn rename_and_paste_keep_reference_targets() {
        let doc=json!({"nodes":{"driver":{"type":"box","params":{"size":[2,3,4]}},"target":{"type":"box","channels":{"size.x":"ch('../driver/size.y')"}},"external":{"type":"box","channels":{"size.x":"ch('../target/size.x')"}}},"display":"target"});
        let renamed=rename(&doc,"driver","source").unwrap();
        assert_eq!(renamed["nodes"]["target"]["channels"]["size.x"],json!("ch('../source/size.y')"));
        let clip=copy(&renamed,&["source".into(),"target".into()]).unwrap();
        let (pasted,ids)=paste(&renamed,&clip).unwrap();
        let new_target=ids.iter().find(|id|pasted["nodes"][*id]["channels"].is_object()).unwrap();
        assert!(crate::value_link_ui::links(&pasted).iter().any(|l|l.target==*new_target && l.source!= "source" && !l.broken));
        assert_eq!(pasted["nodes"]["external"]["channels"]["size.x"],renamed["nodes"]["external"]["channels"]["size.x"]);
        let deleted=delete(&renamed,"source").unwrap();
        assert_eq!(deleted["nodes"]["target"]["channels"]["size.x"],json!("ch('../source/size.y')"));
    }
    #[test] fn noodle_add_connects_in_both_directions_and_records_position() {
        let base = graph();
        let (next, id) = add_connected(&base, "transform", &AddConnection::Output("box1".into()), [320., 192.]).unwrap();
        assert_eq!(next["nodes"][&id]["inputs"]["geo"], "box1");
        assert_eq!(next["nodes"][&id]["pos"], json!([320., 192.]));
        let mut doc = validated(base.clone()).unwrap();
        let mut history = minihou_app::History::default();
        history.replace(&mut doc, validated(next.clone()).unwrap());
        history.undo(&mut doc).unwrap();
        assert_eq!(doc.value(), &base);
        history.redo(&mut doc).unwrap();
        assert_eq!(doc.value(), &next);
        let (next, id) = add_connected(&base, "box", &AddConnection::Input { node: "merge1".into(), port: "geos".into() }, [-32., 64.]).unwrap();
        assert_eq!(next["nodes"]["merge1"]["inputs"]["geos"], json!(["box1", "box2", id]));
        assert_eq!(next["display"], base["display"]);
        validated(next).unwrap();
    }
    #[test] fn noodle_add_handles_optional_inputs_and_terminal_output() {
        let base = json!({"version":1,"display":"output1","nodes":{"box1":{"type":"box"},"output1":{"type":"output","inputs":{"geo":"box1"}}}});
        for ty in ["transform", "curve"] {
            let (next, id) = add_connected(&base, ty, &AddConnection::Output("output1".into()), [32., 64.]).unwrap();
            assert_eq!(next["nodes"][&id]["inputs"]["geo"], "box1");
            assert_eq!(next["nodes"]["output1"]["inputs"]["geo"], id.as_str());
            assert_eq!(next["display"], "output1");
            validated(next).unwrap();
        }
        let (next, id) = add_connected(&base, "curve", &AddConnection::Output("box1".into()), [0., 0.]).unwrap();
        assert_eq!(next["nodes"][&id]["inputs"]["geo"], "box1");
    }
    #[test] fn noodle_add_replaces_exact_input_and_rejects_invalid_contexts() {
        let base = json!({"version":1,"nodes":{"a":{"type":"box"},"b":{"type":"transform","inputs":{"geo":"a"}}}});
        let input = AddConnection::Input { node: "b".into(), port: "geo".into() };
        let (next, id) = add_connected(&base, "box", &input, [0., 0.]).unwrap();
        assert_eq!(next["nodes"]["b"]["inputs"]["geo"], id.as_str());
        assert!(next.get("display").is_none());
        assert!(add_connected(&base, "output", &input, [0., 0.]).is_none());
        assert!(add_connected(&base, "point", &AddConnection::Output("a".into()), [0., 0.]).is_none());
        assert!(add_connected(&base, "transform", &AddConnection::Output("missing".into()), [0., 0.]).is_none());
        assert!(add_connected(&base, "box", &AddConnection::Input { node: "b".into(), port: "missing".into() }, [0., 0.]).is_none());
        assert!(add_connected(&base, "box", &input, [f32::NAN, 0.]).is_none());
        assert_eq!(base["nodes"].as_object().unwrap().len(), 2);
    }
    #[test] fn reverse_noodle_preserves_other_named_inputs() {
        let base = json!({"nodes":{"a":{"type":"box"},"b":{"type":"ray","inputs":{"geo":"a","target":"a"}}}});
        let (next, id) = add_connected(&base, "sphere", &AddConnection::Input { node: "b".into(), port: "target".into() }, [32., 64.]).unwrap();
        assert_eq!(next["nodes"]["b"]["inputs"]["geo"], "a");
        assert_eq!(next["nodes"]["b"]["inputs"]["target"], id.as_str());
        validated(next).unwrap();
    }
    #[test] fn detach_keeps_card_and_bypasses_all_consumers() {
        let v = json!({"version":1,"display":"b","nodes":{
            "a":{"type":"box"}, "other":{"type":"box"},
            "b":{"type":"transform","inputs":{"geo":"a"},"params":{"translate":[1,2,3]},"pos":[123,456]},
            "c":{"type":"normal","inputs":{"geo":"b"}},
            "d":{"type":"merge","inputs":{"geos":["other","b","a"]}}
        }});
        let next = detach(&v, "b").unwrap();
        let mut card = v["nodes"]["b"].clone();
        card.as_object_mut().unwrap().remove("inputs");
        assert_eq!(next["nodes"]["b"], card);
        assert_eq!(next["nodes"]["c"]["inputs"]["geo"], "a");
        assert_eq!(next["nodes"]["d"]["inputs"]["geos"], json!(["other","a"]));
        assert_eq!(next["display"], "a");
        validated(next).unwrap();
    }

    fn graph() -> Value { json!({"version": 1, "display": "merge1", "nodes": {
        "box1": {"type": "box"}, "box2": {"type": "box"},
        "merge1": {"type": "merge", "inputs": {"geos": ["box1", "box2"]}}
    }}) }
    #[test] fn joining_two_objects_keeps_selection_order_and_displays_one_result() {
        let base = json!({"version":1,"display":["box1","box2"],"nodes":{
            "box1":{"type":"box"},"box2":{"type":"sphere"}
        }});
        let selected = ["box2".to_owned(), "box1".to_owned()];
        let (joined, id) = join_objects(&base, &selected).unwrap();
        assert_eq!(joined["nodes"][&id]["type"], "merge");
        assert_eq!(joined["nodes"][&id]["inputs"]["geos"], json!(["box2", "box1"]));
        assert_eq!(joined["display"], json!(id));
        assert_eq!(minihou_app::display_nodes(&joined), vec![id]);
        validated(joined).unwrap();
        assert!(join_objects(&base, &["box1".to_owned()]).is_none());
    }
    #[test] fn adding_after_middle_splices_scalar_and_multi_consumers() {
        let base = json!({"version":1,"display":"out","nodes":{
            "a":{"type":"box"}, "b":{"type":"normal","inputs":{"geo":"a"}},
            "c":{"type":"normal","inputs":{"geo":"b"}},
            "merge":{"type":"merge","inputs":{"geos":["a","b"]}},
            "out":{"type":"output","inputs":{"geo":"c"}}
        }});
        let (next, id) = add(&base, "normal", Some("b")).unwrap();
        assert_eq!(next["nodes"][&id]["inputs"]["geo"], "b");
        assert_eq!(next["nodes"]["c"]["inputs"]["geo"], id.as_str());
        assert_eq!(next["nodes"]["merge"]["inputs"]["geos"], json!(["a",id]));
        assert_eq!(next["nodes"]["out"]["inputs"], base["nodes"]["out"]["inputs"]);
        assert_eq!(next["nodes"]["out"]["pos"], json!([96., 544.]));
        assert_eq!(next["display"], base["display"]);
        validated(next).unwrap();
        let (next, _) = add_connected(&base, "normal", &AddConnection::Output("b".into()), [0.,0.]).unwrap();
        assert_eq!(next["nodes"]["c"], base["nodes"]["c"]);
        assert_eq!(next["nodes"]["merge"], base["nodes"]["merge"]);
    }
    #[test] fn add_and_duplicate_find_unused_ids() {
        let (v, id) = add(&graph(), "box", Some("box1")).unwrap();
        assert_eq!(id, "box3"); assert_eq!(minihou_app::display_nodes(&v), vec!["merge1", "box3"]);
        assert!(v["nodes"][&id].get("inputs").is_none()); validated(v.clone()).unwrap();
        let (v, next) = duplicate(&v, "box1").unwrap();
        assert_eq!(next, "box4"); assert_eq!(v["nodes"]["box4"], v["nodes"]["box1"]); validated(v).unwrap();
    }
    #[test] fn adding_after_an_output_inserts_before_it() {
        let base = json!({"version":1,"nodes":{"box1":{"type":"box"},"output1":{"type":"output","inputs":{"geo":"box1"}}},"display":"output1"});
        let (v, id) = add(&base, "transform", Some("output1")).unwrap();
        assert_eq!(v["nodes"][&id]["inputs"]["geo"], "box1");
        assert_eq!(v["nodes"]["output1"]["inputs"]["geo"], id.as_str());
        assert_eq!(minihou_app::display_nodes(&v), vec!["output1"]);
        validated(v).unwrap();
    }
    #[test] fn displayed_set_adds_generators_and_replaces_modifier_input() {
        let base = json!({"version":1,"nodes":{"box1":{"type":"box"}},"display":"box1"});
        let (two, sphere) = add(&base, "sphere", Some("box1")).unwrap();
        assert_eq!(minihou_app::display_nodes(&two), vec!["box1", sphere.as_str()]);
        assert!(two["nodes"][&sphere].get("inputs").is_none());
        let (modified, transform) = add(&two, "transform", Some(&sphere)).unwrap();
        assert_eq!(minihou_app::display_nodes(&modified), vec!["box1", transform.as_str()]);
        assert_eq!(modified["nodes"][&transform]["inputs"]["geo"], sphere);
        let removed = set_display(&modified, "box1", true).unwrap();
        assert_eq!(minihou_app::display_nodes(&removed), vec![transform.as_str()]);
        let only = set_display(&modified, "box1", false).unwrap();
        assert_eq!(minihou_app::display_nodes(&only), vec!["box1"]);
        assert_eq!(set_display(&only, "box1", true).unwrap()["display"], "box1");
    }
    #[test] fn only_required_first_inputs_are_wired_on_add() {
        let (v, id) = add(&graph(), "transform", Some("box1")).unwrap();
        assert_eq!(v["nodes"][&id]["inputs"]["geo"], "box1");
        let (v, id) = add(&v, "box", Some("transform1")).unwrap();
        assert!(v["nodes"][&id].get("inputs").is_none());
        validated(v).unwrap();
    }
    #[test] fn viewport_cuts_cook_and_undo_as_one_edit() {
        let base = json!({"version":1,"display":"out","nodes":{"box1":{"type":"box"},"out":{"type":"output","inputs":{"geo":"box1"}}}});
        let original = minihou_core::Graph::from_value(&base).unwrap().cook(None).unwrap();
        for (ty,params) in [
            ("knife",json!({"cuts":json!([{"a":[0.,-2.],"b":[0.,2.],"matrix":[1.,0.,0.,0.,0.,1.,0.,0.,0.,0.,1.,0.,0.,0.,0.,1.],"through":false}]).to_string()})),
            ("loop_cut",json!({"split_edges":"0","mode":"edge_loop","loops":1,"position":0.5})),
            ("polysplit",json!({"split_edges":"0","mode":"edge_loop","loops":1,"position":0.5})),
            ("edge_slide",json!({"group":"p0-1","factor":0.25})),
        ] {
            let (next,id) = add_viewport_cut(&base,"out",ty,params).unwrap();
            assert_eq!(next["nodes"][&id]["inputs"]["geo"],"box1");
            assert_eq!(next["nodes"]["out"]["inputs"]["geo"],id.as_str());
            assert_eq!(next["display"],"out");
            let cut = minihou_core::Graph::from_value(&next).unwrap().cook(None).unwrap();
            if ty == "edge_slide" {
                // Sliding an existing edge changes positions, unlike the three cutting nodes which add points.
                assert_eq!(cut.npoints, original.npoints);
                assert_ne!(cut.p(), original.p());
            } else {
                assert!(cut.npoints > original.npoints, "{ty}: {} -> {} points", original.npoints, cut.npoints);
            }
            let mut doc = validated(base.clone()).unwrap(); let mut history = minihou_app::History::default();
            history.replace(&mut doc,validated(next.clone()).unwrap());
            history.undo(&mut doc).unwrap(); assert_eq!(doc.value(),&base);
            history.redo(&mut doc).unwrap(); assert_eq!(doc.value(),&next);
        }
        assert!(add_viewport_cut(&base,"box1","loop_cut",json!({})).is_none());
        assert!(add_viewport_cut(&base,"out","transform",json!({})).is_none());
    }
    #[test] fn viewport_cut_splices_only_active_output_with_three_displays() {
        let base=json!({"version":1,"display":["a","b","c"],"nodes":{
            "box_a":{"type":"box"},"box_b":{"type":"box"},"box_c":{"type":"box"},
            "a":{"type":"output","inputs":{"geo":"box_a"}},
            "b":{"type":"output","inputs":{"geo":"box_b"}},
            "c":{"type":"output","inputs":{"geo":"box_c"}}}});
        let (next,id)=add_viewport_cut(&base,"b","edge_slide",json!({"group":"p0-1","factor":0.25})).unwrap();
        assert_eq!(next["nodes"]["b"]["inputs"]["geo"],id);
        assert_eq!(next["nodes"]["a"],base["nodes"]["a"]);
        assert_eq!(next["nodes"]["c"],base["nodes"]["c"]);
        assert_eq!(next["display"],base["display"]);
    }
    #[test] fn sketch_wires_after_a_cooked_node_and_can_stand_alone() {
        let points = [[0.,0.,0.], [1.,0.,0.], [1.,1.,0.]];
        let (v, id) = add_sketch(&graph(), "Polygon", &points, true, "box1").unwrap();
        assert_eq!(v["nodes"][&id]["inputs"]["geo"], "box1");
        assert_eq!(v["nodes"][&id]["params"]["points"], "0,0,0 1,0,0 1,1,0");
        assert_eq!(v["nodes"][&id]["params"]["closed"], true);
        assert_eq!(minihou_app::display_nodes(&v), vec!["merge1", id.as_str()]);
        validated(v).unwrap();
        let (v, id) = add_sketch(&graph(), "Line", &points[..2], false, "missing").unwrap();
        assert!(v["nodes"][&id].get("inputs").is_none());
        validated(v).unwrap();
        assert!(add_sketch(&graph(), "Bad", &[[f32::NAN,0.,0.]; 2], false, "box1").is_none());
    }
    #[test] fn sketch_batch_follows_stream_primitive_type_and_undoes_once() {
        let strokes: Vec<(&str, &[[f32;3]], bool)> = vec![
            ("Line", &[[0.,0.,0.],[1.,0.,0.]], false),
            ("Polygon", &[[0.,0.,1.],[1.,0.,1.],[0.,1.,1.]], true),
        ];
        for (base, selected, target) in [
            (json!({"version":1,"display":"out","nodes":{"box1":{"type":"box"},"out":{"type":"output","inputs":{"geo":"box1"}}}}), "out", SketchTarget::Mesh),
            (json!({"version":1,"display":"out","nodes":{"curve1":{"type":"curve","params":{"points":"0,0,0 1,0,0"}},"out":{"type":"output","inputs":{"geo":"curve1"}}}}), "out", SketchTarget::Curve),
            (json!({"version":1,"nodes":{}}), "", SketchTarget::New),
        ] {
            let actual = if selected.is_empty() { SketchTarget::New } else {
                let g = minihou_core::Graph::from_value(&base).unwrap().cook(Some(selected)).unwrap();
                sketch_target(Some(&g))
            };
            assert_eq!(actual, target);
            let (next, id, ids) = add_sketch_batch(&base, &strokes, selected, actual).unwrap();
            assert_eq!(ids.len(), 2);
            if target == SketchTarget::Mesh {
                assert_eq!(next["nodes"][&id]["type"], "merge");
                assert_eq!(next["nodes"]["out"]["inputs"]["geo"], id);
                assert_eq!(next["display"], "out");
                assert_eq!(next["nodes"][&ids[0]]["params"]["as_mesh"], true);
            } else if target == SketchTarget::Curve {
                assert_eq!(next["nodes"][&ids[0]]["inputs"]["geo"], "curve1");
                assert_eq!(next["nodes"][&ids[1]]["inputs"]["geo"], ids[0]);
                assert_eq!(next["display"], "out");
            } else { assert_eq!(next["display"], id); }
            let cooked = minihou_core::Graph::from_value(&next).unwrap().cook(None).unwrap();
            assert!(cooked.nprims() >= 2, "{target:?}: {} primitives", cooked.nprims());
            let mut doc = validated(base.clone()).unwrap();
            let mut history = minihou_app::History::default();
            history.replace(&mut doc, validated(next).unwrap());
            history.undo(&mut doc).unwrap();
            assert_eq!(doc.value(), &base);
            assert!(history.undo(&mut doc).is_none());
        }
    }
    #[test] fn curve_blast_inserts_before_output_and_cleans_points() {
        let base = json!({
            "version": 1,
            "display": "out",
            "nodes": {
                "c": {"type": "curve", "params": {"points": "0,0,0 1,0,0 2,0,0 3,0,0", "closed": false}},
                "out": {"type": "output", "inputs": {"geo": "c"}}
            }
        });
        let original = minihou_core::Graph::from_value(&base).unwrap().cook(None).unwrap();
        assert_eq!(original.npoints, 4);

        let (next, blast_id) = add_curve_blast(&base, "out", &[1, 2]).unwrap();
        assert_eq!(next["nodes"]["out"]["inputs"]["geo"], blast_id.as_str());
        assert_eq!(next["nodes"][&blast_id]["inputs"]["geo"], "c");
        assert_eq!(next["nodes"][&blast_id]["params"]["domain"], "points");
        assert_eq!(next["nodes"][&blast_id]["params"]["group"], "1,2");
        assert_eq!(next["display"], "out");

        let cooked = minihou_core::Graph::from_value(&next).unwrap().cook(None).unwrap();
        assert_eq!(cooked.npoints, 2);

        assert!(add_curve_blast(&base, "out", &[]).is_none());
        let mesh_base = json!({"version": 1, "display": "out", "nodes": {"b": {"type": "box"}, "out": {"type": "output", "inputs": {"geo": "b"}}}});
        assert!(add_curve_blast(&mesh_base, "out", &[0]).is_none());
    }
    #[test] fn bezier_sketch_creates_bezier_node_with_controls() {
        let points = [[0., 0., 0.], [1., 1., 0.], [2., 0., 0.]];
        let (v, id) = add_bezier_sketch(&graph(), "Bezier", &points, false, "box1").unwrap();
        assert_eq!(v["nodes"][&id]["type"], "bezier");
        assert_eq!(v["nodes"][&id]["inputs"]["geo"], "box1");
        assert_eq!(v["nodes"][&id]["params"]["closed"], false);
        let controls: Value = serde_json::from_str(v["nodes"][&id]["params"]["controls"].as_str().unwrap()).unwrap();
        assert_eq!(controls.as_array().unwrap().len(), 3);
        validated(v.clone()).unwrap();

        let cooked = minihou_core::Graph::from_value(&v).unwrap().cook(None).unwrap();
        assert!(cooked.npoints >= 3);
    }
    #[test] fn delete_cleans_wires_and_display() {
        let v = delete(&graph(), "box1").unwrap();
        assert_eq!(v["nodes"]["merge1"]["inputs"]["geos"], json!(["box2"])); validated(v).unwrap();
        let v = delete(&graph(), "merge1").unwrap();
        assert_ne!(v["display"], "merge1"); validated(v).unwrap();
    }
    #[test] fn connect_multi_and_disconnect() {
        let v = connect(&graph(), "box1", "merge1", Some("geos")).unwrap();
        assert_eq!(v["nodes"]["merge1"]["inputs"]["geos"], json!(["box1", "box2"]));
        let v = disconnect(&v, "merge1", "geos").unwrap();
        assert!(v["nodes"]["merge1"]["inputs"].get("geos").is_none()); validated(v).unwrap();
    }
    #[test] fn single_input_replaces_its_wire_and_invalid_commit_is_rejected() {
        let mut v = graph();
        v["nodes"]["transform1"] = json!({"type": "transform", "inputs": {"geo": "box1"}});
        let v = connect(&v, "box2", "transform1", Some("geo")).unwrap();
        assert_eq!(v["nodes"]["transform1"]["inputs"]["geo"], "box2");
        assert!(connect(&v, "missing", "transform1", None).is_none());
        assert!(connect(&v, "box2", "transform1", Some("missing")).is_none());
        validated(v).unwrap();
        let mut invalid = graph();
        invalid["nodes"] = json!([]);
        assert!(validated(invalid).is_err());
    }
    #[test] fn paste_remaps_only_internal_wires_and_ids_remain_unique() {
        let source = graph();
        let clip = copy(&source, &["box1".into(), "merge1".into()]).unwrap();
        assert_eq!(clip["nodes"]["merge1"]["inputs"]["geos"], json!(["box1"]));
        let (v, ids) = paste(&source, &clip).unwrap();
        assert_eq!(ids, ["box3", "merge2"]);
        assert_eq!(v["nodes"]["merge2"]["inputs"]["geos"], json!(["box3"]));
        let (v, ids) = paste(&v, &clip).unwrap();
        assert_eq!(ids, ["box4", "merge3"]); validated(v).unwrap();
    }
    #[test] fn dissolve_chain_branch_and_display() {
        let v = json!({"version":1,"display":"transform1","nodes":{
            "box1":{"type":"box"},
            "transform1":{"type":"transform","inputs":{"geo":"box1"}},
            "transform2":{"type":"transform","inputs":{"geo":"transform1"}},
            "transform3":{"type":"transform","inputs":{"geo":"transform1"}}
        }});
        let next = dissolve_set(&v, &["transform1".into()]).unwrap();
        assert!(next["nodes"].get("transform1").is_none());
        assert_eq!(next["nodes"]["transform2"]["inputs"]["geo"], "box1");
        assert_eq!(next["nodes"]["transform3"]["inputs"]["geo"], "box1");
        assert_eq!(next["display"], "box1");
        validated(next).unwrap();
    }
    #[test] fn dissolve_multi_input_and_generator() {
        let next = dissolve_set(&graph(), &["merge1".into()]).unwrap();
        assert_eq!(next["display"], "box1");
        let next = dissolve_set(&graph(), &["box1".into()]).unwrap();
        assert_eq!(next["nodes"]["merge1"]["inputs"]["geos"], json!(["box2"]));
        validated(next).unwrap();
        let mut v = graph();
        v["nodes"]["transform1"] = json!({"type":"transform","inputs":{"geo":"box1"}});
        v["nodes"]["merge1"]["inputs"]["geos"] = json!(["transform1","box2","box1"]);
        let next = dissolve_set(&v, &["transform1".into()]).unwrap();
        assert_eq!(next["nodes"]["merge1"]["inputs"]["geos"], json!(["box1","box2"]));
        validated(next).unwrap();
    }
    #[test] fn dissolve_selected_chain_and_delete_set_are_atomic_values() {
        let v = json!({"version":1,"display":"transform2","nodes":{
            "box1":{"type":"box"},
            "transform1":{"type":"transform","inputs":{"geo":"box1"}},
            "transform2":{"type":"transform","inputs":{"geo":"transform1"}},
            "transform3":{"type":"transform","inputs":{"geo":"transform2"}}
        }});
        let ids = ["transform1".into(), "transform2".into()];
        let next = dissolve_set(&v, &ids).unwrap();
        assert_eq!(next["nodes"]["transform3"]["inputs"]["geo"], "box1");
        assert_eq!(next["display"], "box1");
        validated(next).unwrap();
        let reversed = dissolve_set(&v, &["transform2".into(), "transform1".into()]).unwrap();
        assert_eq!(reversed["nodes"]["transform3"]["inputs"]["geo"], "box1");
        assert_eq!(reversed["display"], "box1");
        validated(reversed).unwrap();
        let deleted = delete_set(&v, &ids).unwrap();
        assert!(deleted["nodes"]["transform3"]["inputs"].as_object().unwrap().is_empty());
        validated(deleted).unwrap();
    }
    #[test] fn disconnect_all_keeps_node_and_clears_both_sides() {
        let v = json!({"version":1,"display":"transform2","nodes":{
            "box1":{"type":"box"},
            "transform1":{"type":"transform","inputs":{"geo":"box1"}},
            "transform2":{"type":"transform","inputs":{"geo":"transform1"}}
        }});
        let next = disconnect_all(&v, "transform1").unwrap();
        assert!(next["nodes"]["transform1"].get("inputs").is_none());
        assert!(next["nodes"]["transform2"]["inputs"].as_object().unwrap().is_empty());
        validated(next).unwrap();
    }
    #[test] fn delete_last_node_leaves_valid_empty_graph() {
        let v = json!({"version":1,"display":"box1","nodes":{"box1":{"type":"box"}}});
        let next = delete_set(&v, &["box1".into()]).unwrap();
        assert!(next.get("display").is_none());
        assert!(next["nodes"].as_object().unwrap().is_empty());
        validated(next).unwrap();
    }
    #[test] fn dissolve_reconnects_input_to_consumers() {
        let mut v = graph();
        v["nodes"]["transform1"] = json!({"type": "transform", "inputs": {"geo": "box1"}});
        v["nodes"]["merge1"]["inputs"]["geos"] = json!(["transform1", "box2"]);
        v["display"] = json!("transform1");
        let v = dissolve(&v, "transform1").unwrap();
        assert!(v["nodes"].get("transform1").is_none());
        assert_eq!(v["nodes"]["merge1"]["inputs"]["geos"], json!(["box1", "box2"]));
        assert_eq!(v["display"], "box1");
        validated(v).unwrap();
    }
