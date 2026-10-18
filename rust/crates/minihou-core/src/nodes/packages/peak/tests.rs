use super::*;
use std::sync::Arc;
use crate::geometry::Attr;

    #[test]
    fn test_peak_displaces_along_normals() {
        let pts = vec![
            0.0, 0.0, 0.0,
            1.0, 0.0, 0.0,
            1.0, 1.0, 0.0,
            0.0, 1.0, 0.0,
        ];
        let vp = vec![0, 1, 2, 3];
        let offs = vec![0, 4];
        let mut geo = Geometry::from_arrays(pts, vp, offs, None).unwrap();
        geo.point.insert("N".to_string(), Arc::new(Attr::f32(3, [0.0, 0.0, 1.0].repeat(4))));

        let p = Params::resolve(NODE.params, serde_json::json!({"distance": 2.0}).as_object()).unwrap();
        let out = (NODE.cook)(&[vec![Arc::new(geo)]], &p).unwrap();
        let p_out = out.p();
        assert_eq!(p_out[2], 2.0);
        assert_eq!(p_out[5], 2.0);
        assert_eq!(p_out[8], 2.0);
        assert_eq!(p_out[11], 2.0);
    }
