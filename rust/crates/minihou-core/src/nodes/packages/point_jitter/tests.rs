use super::*;
    #[test]
    fn point_jitter_applies_offset() {
        let g = Arc::new(Geometry::points_only(vec![0.0, 0.0, 0.0]).unwrap());
        let p = Params::resolve(NODE.params, serde_json::json!({"scale": [0.1, 0.0, 0.0]}).as_object()).unwrap();
        let out = point_jitter(&[vec![g]], &p).unwrap();
        assert!(out.p()[0].abs() <= 0.1 && out.p()[0] != 0.0);
        assert_eq!(out.p()[1], 0.0);
        assert_eq!(out.p()[2], 0.0);
    }
