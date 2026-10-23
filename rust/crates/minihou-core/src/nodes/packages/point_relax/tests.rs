use super::*;
    #[test]
    fn projects_relaxed_points_onto_surface() {
        let points = Arc::new(Geometry::points_only(vec![0., 0., 1., 0.1, 0., 1.]).unwrap());
        let surface = Arc::new(Geometry::from_arrays(vec![-1., -1., 0., 1., -1., 0., 0., 1., 0.], vec![0, 1, 2], vec![0, 3], None).unwrap());
        let p = Params::resolve(NODE.params, serde_json::json!({"iterations": 1, "radius": 1.0}).as_object()).unwrap();
        let out = (NODE.cook)(&[vec![points], vec![surface]], &p).unwrap();
        assert_eq!(out.p()[2], 0.);
        assert_eq!(out.p()[5], 0.);
        assert_eq!(out.point["N"].as_f32().unwrap(), &[0., 0., 1., 0., 0., 1.]);
    }

    #[test]
    fn group_moves_only_selected_points() {
        let points = Arc::new(Geometry::points_only(vec![0., 0., 0., 0.2, 0., 0., 3., 0., 0.]).unwrap());
        let p = Params::resolve(NODE.params, serde_json::json!({
            "iterations": 1, "radius": 1.0, "group": "0", "domain": "points"
        }).as_object()).unwrap();
        let out = (NODE.cook)(&[vec![points.clone()], vec![]], &p).unwrap();
        assert!(out.p()[0] < 0.0);
        assert_eq!(&out.p()[3..], &points.p()[3..]);
    }

    #[test]
    fn missing_group_parameters_keep_legacy_all_points_behavior() {
        let points = Arc::new(Geometry::points_only(vec![0., 0., 0., 0.2, 0., 0.]).unwrap());
        let legacy = Params::resolve(NODE.params, serde_json::json!({
            "iterations": 1, "radius": 1.0
        }).as_object()).unwrap();
        let explicit = Params::resolve(NODE.params, serde_json::json!({
            "iterations": 1, "radius": 1.0, "group": "", "domain": "points"
        }).as_object()).unwrap();
        let inputs = [vec![points], vec![]];
        let out = (NODE.cook)(&inputs, &legacy).unwrap();
        assert!(out.p()[0] < 0.0 && out.p()[3] > 0.2);
        assert_eq!(out.p(), (NODE.cook)(&inputs, &explicit).unwrap().p());
    }

    #[test]
    fn edge_and_primitive_groups_select_their_points() {
        let points = Arc::new(Geometry::from_arrays(
            vec![0., 0., 0., 0.2, 0., 0., 3., 0., 0., 5., 0., 0., 6., 0., 0.],
            vec![0, 1, 2, 2, 3, 4], vec![0, 3, 6], None,
        ).unwrap());
        let inputs = [vec![points.clone()], vec![]];
        for (domain, group) in [("edges", "p0-1"), ("primitives", "0")] {
            let p = Params::resolve(NODE.params, serde_json::json!({
                "iterations": 1, "radius": 1.0, "domain": domain, "group": group
            }).as_object()).unwrap();
            let out = (NODE.cook)(&inputs, &p).unwrap();
            assert!(out.p()[0] < 0.0, "{domain}");
            assert_eq!(&out.p()[9..], &points.p()[9..], "{domain}");
        }
    }
