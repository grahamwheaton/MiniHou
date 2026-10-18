use super::*;
#[test]
fn test_pcg_surface_sampler() {
    let p = vec![-10.0, -10.0, 0.0, 10.0, -10.0, 0.0, 10.0, 10.0, 0.0, -10.0, 10.0, 0.0];
    let quad = Geometry::from_arrays(p, vec![0, 1, 2, 3], vec![0, 4], None).unwrap();
    let params = Params::resolve(
        NODE.params,
        serde_json::json!({"points_per_m2": 1.0, "extents": [0.8, 0.8, 1.0], "looseness": 0.0, "seed": 0}).as_object(),
    ).unwrap();
    let pts = pcg_surface_sampler(&[vec![Arc::new(quad)]], &params).unwrap();
    assert!((pts.npoints as i32 - 400).abs() <= 40);
    for name in ["rot", "scale", "bounds_min", "bounds_max", "density", "seed", "N"] {
        assert!(pts.point.contains_key(name));
    }
}
