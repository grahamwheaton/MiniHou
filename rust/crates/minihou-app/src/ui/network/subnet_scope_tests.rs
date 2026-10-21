    use super::*;
    use serde_json::json;
    #[test]
    fn network_root_and_inner_scope_follow_subnet_path() {
        let doc=json!({"nodes":{"a":{"type":"box"},"group":{"type":"subnet","inner":{"output":"inner","nodes":{
            "inner":{"type":"box"},"nested":{"type":"subnet","inner":{"nodes":{"deep":{"type":"box"}},"output":"deep"}}
        }}}}});
        let mut state=State::default();
        assert!(state.subnet_path.is_empty());
        assert!(scope_doc(&doc,&state.subnet_path).is_none());
        state.subnet_path.push("group".into());
        let inner=scope_doc(&doc,&state.subnet_path).unwrap();
        assert!(inner["nodes"].get("inner").is_some());
        assert!(inner["nodes"].get("a").is_none());
        assert_eq!(inner["display"],"inner");
        state.subnet_path.push("nested".into());
        assert!(scope_doc(&doc,&state.subnet_path).unwrap()["nodes"].get("deep").is_some());
        state.subnet_path.pop(); reset_scope(&mut state);
        assert_eq!(valid_path(&doc,&state.subnet_path),vec!["group"]);
        state.subnet_path.pop();
        assert!(scope_doc(&doc,&state.subnet_path).is_none());
    }
    #[test]
    fn network_inner_parameter_edit_preserves_root_nodes() {
        let doc=json!({"nodes":{"other":{"type":"box"},"group":{"type":"subnet","inner":{"nodes":{"inner":{"type":"box","params":{"size":[1,1,1]}}},"output":"inner"}}}});
        let action=UiAction::Edit {edit:Edit::Param {node:"inner".into(),name:"size".into(),value:json!([2,2,2])},continuous:false};
        let Some(UiAction::Edit {edit:Edit::Replace(next),..})=scoped_edit(&doc,&["group".into()],action) else {panic!("expected root edit")};
        assert_eq!(next["nodes"]["group"]["inner"]["nodes"]["inner"]["params"]["size"],json!([2,2,2]));
        assert_eq!(next["nodes"]["other"],doc["nodes"]["other"]);
    }
    #[test]
    fn exposing_inner_parameter_creates_a_subnet_input_that_drives_the_inner_node() {
        let doc=json!({"display":"group","nodes":{"group":{"type":"subnet","inner":{"nodes":{
            "point":{"type":"point"},
            "move":{"type":"transform","inputs":{"geo":"point"}}
        },"output":"move"}}}});
        let path=["group".into()];
        let inner=scope_doc(&doc,&path).unwrap();
        let edit=minihou_app::toggle_exposed(&inner,"move","translate");
        let Some(UiAction::Edit {edit:Edit::Replace(mut next),..})=
            scoped_edit(&doc,&path,UiAction::Edit {edit,continuous:false}) else {panic!("expected subnet edit")};
        assert_eq!(next["nodes"]["group"]["promoted_params"][0]["name"],"move_translate");
        assert_eq!(next["nodes"]["group"]["promoted_params"][0]["param"],"translate");
        assert!(minihou_app::is_exposed(&scope_doc(&next,&path).unwrap(),"move","translate"));
        next["nodes"]["group"]["params"]["move_translate"]=json!([3.0,4.0,5.0]);
        let graph=minihou_core::Graph::from_value(&next).unwrap();
        assert_eq!(graph.cook(None).unwrap().p(),&[3.0,4.0,5.0]);
    }
    #[test]
    fn network_inner_output_change_embeds_in_parent_subnet() {
        let doc=json!({"nodes":{"group":{"type":"subnet","inner":{"nodes":{"a":{"type":"box"},"b":{"type":"box"}},"output":"a"}}}});
        let mut inner=scope_doc(&doc,&["group".into()]).unwrap();
        inner["display"]=json!("b");
        let next=embed_scope(&doc,&["group".into()],inner).unwrap();
        assert_eq!(next["nodes"]["group"]["inner"]["output"],"b");
    }
