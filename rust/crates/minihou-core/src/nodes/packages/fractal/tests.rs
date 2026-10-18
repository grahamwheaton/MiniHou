use super::*;
use std::sync::Arc;
use crate::geometry::Attr;

#[test]
fn test_fractal_flat_grid_and_zero_strength() {
    let mut g = Geometry::from_arrays(
        vec![
            0.0, 0.0, 0.0,
            1.0, 0.0, 0.0,
            1.0, 0.0, 1.0,
            0.0, 0.0, 1.0,
        ],
        vec![0, 3, 2, 1],
        vec![0, 4],
        None,
    ).unwrap();
    g.point.insert("N".into(), Arc::new(Attr::f32(3, vec![
        0.0, 1.0, 0.0,
        0.0, 1.0, 0.0,
        0.0, 1.0, 0.0,
        0.0, 1.0, 0.0,
    ])));
    let arc_g = Arc::new(g);

    // Zero strength leaves geometry unchanged
    let p_zero = Params::resolve(NODE.params, serde_json::json!({
        "strength": 0.0,
    }).as_object()).unwrap();
    let out_zero = cook(&[vec![arc_g.clone()]], &p_zero).unwrap();
    assert_eq!(out_zero.p(), arc_g.p());

    // Displaced by expected amount along normal
    let p_disp = Params::resolve(NODE.params, serde_json::json!({
        "strength": 0.5,
        "seed": 42,
    }).as_object()).unwrap();
    let out_disp = cook(&[vec![arc_g.clone()]], &p_disp).unwrap();
    let p = out_disp.p();
    let expected_noise = crate::noise::fbm3_one([0.0, 0.0, 0.0], 42, 3, 2.0, 0.5);
    let expected_y = 0.5 * expected_noise;
    assert!((p[1] - expected_y).abs() < 1e-6);
    assert_eq!(p[0], 0.0);
    assert_eq!(p[2], 0.0);

    // Missing input returns error
    assert!(cook(&[vec![]], &Params::resolve(NODE.params, None).unwrap()).is_err());
}
