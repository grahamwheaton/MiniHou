use super::*;
use std::sync::Arc;
use crate::geometry::Attr;

#[test]
fn test_waves_flat_grid_and_zero_strength() {
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

    // Displaced by expected amount (sin(pi/2) = 1.0, moves along Y by strength)
    let p_disp = Params::resolve(NODE.params, serde_json::json!({
        "strength": 0.5,
        "axis": "x",
        "frequency": 0.0,
        "offset": std::f32::consts::FRAC_PI_2,
    }).as_object()).unwrap();
    let out_disp = cook(&[vec![arc_g.clone()]], &p_disp).unwrap();
    let p = out_disp.p();
    for i in 0..4 {
        assert!((p[i * 3 + 1] - 0.5).abs() < 1e-6);
    }

    // Missing input returns error
    assert!(cook(&[vec![]], &Params::resolve(NODE.params, None).unwrap()).is_err());
}
