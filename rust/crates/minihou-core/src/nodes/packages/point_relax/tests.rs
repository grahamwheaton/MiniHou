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
