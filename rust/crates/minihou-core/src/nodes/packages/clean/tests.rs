use super::*;
use std::sync::Arc;
    #[test]
    fn clean_removes_degenerates_and_unused() {
        let p = vec![
            0.0, 0.0, 0.0,
            1.0, 0.0, 0.0,
            1.0, 1.0, 0.0,
            0.0, 1.0, 0.0,
            5.0, 5.0, 5.0,
        ];
        let g = Arc::new(Geometry::from_arrays(p, vec![0, 1, 1, 2, 3, 0, 1, 2, 2, 2], vec![0, 6, 8, 9, 10], None).unwrap());
        let params = Params::resolve(NODE.params, serde_json::json!({"fuse_distance": 0.0, "remove_degenerate": true, "remove_unused_points": true}).as_object()).unwrap();
        let out = clean(&[vec![g]], &params).unwrap();
        assert_eq!(out.npoints, 4);
        assert_eq!(out.nprims(), 1);
        assert_eq!(out.topology.vertex_point, vec![0, 1, 2, 3]);
    }
