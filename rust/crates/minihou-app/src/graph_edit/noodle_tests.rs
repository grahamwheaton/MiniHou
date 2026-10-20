    use super::*;
    use std::collections::BTreeSet;
    #[test]
    fn graph_multi_output_wire_reconnection() {
        let base=json!({"nodes":{
            "box":{"type":"box"},
            "fill":{"type":"flood_fill","inputs":{"geometry":"box"}},
            "pass":{"type":"null"},
            "out":{"type":"output"}
        }});
        assert!(connect_output(&base,"fill",2,"out",Some("geo")).is_none());
        let connected=connect_output(&base,"fill",1,"out",Some("geo")).unwrap();
        assert_eq!(connected["nodes"]["out"]["inputs"]["geo"],json!({"node":"fill","output":1}));
        let spliced=insert_existing(&connected,"pass",&wire("fill","out","geo")).unwrap();
        assert_eq!(spliced["nodes"]["pass"]["inputs"]["geo"],json!({"node":"fill","output":1}));
        assert_eq!(spliced["nodes"]["out"]["inputs"]["geo"],"pass");
        let cut=disconnect_wires(&connected,&BTreeSet::from([wire("fill","out","geo")])).unwrap();
        assert!(cut["nodes"]["out"]["inputs"].get("geo").is_none());
    }
    fn wire(from: &str, to: &str, port: &str) -> WireRef {
        WireRef { from: from.into(), to: to.into(), port: port.into() }
    }
    #[test]
    fn transform_drop_splices_exact_edge_once() {
        let base = json!({"nodes": {
            "grid1":{"type":"grid"},
            "noise1":{"type":"noise","inputs":{"geo":"grid1"}},
            "transform1":{"type":"transform","inputs":{"geo":"noise1"}},
            "normal1":{"type":"normal","inputs":{"geo":"transform1"}},
            "transform2":{"type":"transform","inputs":{"geo":"normal1"}},
            "output1":{"type":"output","inputs":{"geo":"transform2"}}
        },"display":"output1"});
        let (added, id) = add(&base, "transform", None).unwrap();
        assert_eq!(id, "transform3");
        let next = insert_existing(&added, &id, &wire("transform2", "output1", "geo")).unwrap();
        let graph = minihou_core::Graph::from_value(&next).unwrap();
        let edges: Vec<_> = graph.nodes.iter().flat_map(|(to, def)| def.inputs.iter().flat_map(move |(port, sources)|
            sources.iter().map(move |from| (from.as_str(), to.as_str(), port.as_str())))).collect();
        assert_eq!(edges.iter().filter(|(from, to, _)| *from == "transform2" && *to == "output1").count(), 0);
        assert_eq!(edges.iter().filter(|(from, to, _)| *from == "transform2" && *to == "transform3").count(), 1);
        assert_eq!(edges.iter().filter(|(from, to, _)| *from == "transform3" && *to == "output1").count(), 1);
        assert_eq!(next["nodes"]["transform3"]["inputs"], json!({"geo":"transform2"}));
        assert_eq!(next["nodes"]["output1"]["inputs"], json!({"geo":"transform3"}));
        let duplicate = json!({"nodes": {
            "grid1":{"type":"grid"},
            "merge1":{"type":"merge","inputs":{"geos":["grid1","grid1"]}},
            "transform1":{"type":"transform"}
        }});
        let deduped = insert_existing(&duplicate, "transform1", &wire("grid1", "merge1", "geos")).unwrap();
        assert_eq!(deduped["nodes"]["merge1"]["inputs"]["geos"], json!(["transform1"]));
    }
    #[test]
    fn noodle_insert_exact_edge_and_position_are_one_history_edit() {
        let base = json!({"nodes": {
            "a":{"type":"box"}, "b":{"type":"box"},
            "loose":{"type":"transform","pos":[0,0]},
            "m":{"type":"merge","inputs":{"geos":["a","b"]}},
            "out":{"type":"output","inputs":{"geo":"m"}}
        },"display":"out"});
        let mut moved = base.clone(); moved["nodes"]["loose"]["pos"] = json!([50,80]);
        let next = insert_existing(&moved,"loose",&wire("a","m","geos")).unwrap();
        assert_eq!(next["nodes"]["m"]["inputs"]["geos"],json!(["loose","b"]));
        assert_eq!(next["nodes"]["loose"]["inputs"]["geo"],"a");
        assert_eq!(next["display"],"out");
        let mut doc = validated(base.clone()).unwrap();
        let mut history = minihou_app::History::default();
        history.replace(&mut doc,validated(next.clone()).unwrap());
        history.undo(&mut doc).unwrap(); assert_eq!(doc.value(),&base);
        assert!(history.undo(&mut doc).is_none());
        history.redo(&mut doc).unwrap(); assert_eq!(doc.value(),&next);
        let terminal = insert_existing(&moved,"loose",&wire("m","out","geo")).unwrap();
        assert_eq!(terminal["nodes"]["out"]["inputs"]["geo"],"loose");
        for edge in [wire("loose","m","geos"),wire("a","loose","geo"),wire("b","out","geo"),wire("gone","m","geos")] {
            assert!(insert_existing(&moved,"loose",&edge).is_none());
        }
        assert!(insert_existing(&next,"loose",&wire("b","m","geos")).is_none());
        assert!(insert_existing(&moved,"out",&wire("a","m","geos")).is_none());
        let mut generator = moved.clone(); generator["nodes"]["point"] = json!({"type":"point"});
        assert!(insert_existing(&generator,"point",&wire("a","m","geos")).is_none());
        let mut cyclic = moved.clone(); cyclic["nodes"]["a"]["inputs"]["geo"] = json!("loose");
        assert!(insert_existing(&cyclic,"loose",&wire("a","m","geos")).is_none());
    }
    #[test]
    fn noodle_pickup_preview_cancel_release_and_undo() {
        let base = json!({"nodes": {"a":{"type":"box"},"b":{"type":"box"},
            "m":{"type":"merge","inputs":{"geos":["a","b"]}},
            "t":{"type":"transform"},"out":{"type":"output","inputs":{"geo":"m"}}}});
        assert!(WirePickup::begin(&base,"t","geo").is_none());
        let pickup = WirePickup::begin(&base,"m","geos").unwrap();
        assert_eq!(pickup.original,wire("b","m","geos"));
        // Cancelling simply drops the preview: the original document never changed.
        drop(pickup.clone()); assert_eq!(base["nodes"]["m"]["inputs"]["geos"],json!(["a","b"]));
        assert!(pickup.release(&base,Some(("m","geos"))).is_none());
        assert!(pickup.release(&base,Some(("b","geo"))).is_none());
        assert!(pickup.release(&base,Some(("t","missing"))).is_none());
        let next = pickup.release(&base,Some(("t","geo"))).unwrap();
        assert_eq!(next["nodes"]["t"]["inputs"]["geo"],"b");
        assert_eq!(next["nodes"]["m"]["inputs"]["geos"],json!(["a"]));
        let disconnected = pickup.release(&base,None).unwrap();
        assert_eq!(disconnected["nodes"]["m"]["inputs"]["geos"],json!(["a"]));
        let mut doc = validated(base.clone()).unwrap(); let mut history = minihou_app::History::default();
        history.replace(&mut doc,validated(next.clone()).unwrap());
        history.undo(&mut doc).unwrap(); assert_eq!(doc.value(),&base);
        assert!(history.undo(&mut doc).is_none());
        history.redo(&mut doc).unwrap(); assert_eq!(doc.value(),&next);
        let cycle = WirePickup::begin(&base,"out","geo").unwrap();
        assert!(cycle.release(&base,Some(("m","geos"))).is_none());
    }
    #[test]
    fn noodle_cut_is_one_undoable_edit_and_never_dissolves() {
        let before = json!({"nodes": {
            "a":{"type":"box"}, "b":{"type":"box"},
            "merge":{"type":"merge", "inputs":{"geos":["a","b"]}},
            "out":{"type":"output", "inputs":{"geo":"merge"}}
        }, "output":"out"});
        let cuts=BTreeSet::from([wire("a","merge","geos"),wire("merge","out","geo")]);
        let next=disconnect_wires(&before,&cuts).unwrap();
        assert_eq!(next["nodes"]["merge"]["inputs"]["geos"],json!(["b"]));
        assert!(next["nodes"]["out"]["inputs"].get("geo").is_none());
        assert_eq!(next["nodes"].as_object().unwrap().len(),4);
        assert_eq!(next["output"],before["output"]);
        let mut doc=validated(before.clone()).unwrap();
        let mut history=minihou_app::History::default();
        history.replace(&mut doc,validated(next.clone()).unwrap());
        history.undo(&mut doc).unwrap();
        assert_eq!(doc.value(),&before);
        assert!(history.undo(&mut doc).is_none());
        history.redo(&mut doc).unwrap();
        assert_eq!(doc.value(),&next);
    }
    #[test]
    fn noodle_cut_preserves_order_other_ports_and_ignores_stale_edges() {
        let doc=json!({"nodes":{"m":{"inputs":{"geos":["a","b","c"],"other":"a"}}}});
        let cuts=BTreeSet::from([wire("b","m","geos"),wire("missing","m","geos"),wire("a","gone","geo")]);
        let next=disconnect_wires(&doc,&cuts).unwrap();
        assert_eq!(next["nodes"]["m"]["inputs"]["geos"],json!(["a","c"]));
        assert_eq!(next["nodes"]["m"]["inputs"]["other"],json!("a"));
        assert!(disconnect_wires(&next,&cuts).is_none());
        let next=disconnect_wires(&next,&BTreeSet::from([wire("a","m","geos"),wire("c","m","geos")])).unwrap();
        assert!(next["nodes"]["m"]["inputs"].get("geos").is_none());
    }
