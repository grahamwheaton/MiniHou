use super::*;
use std::sync::Arc;
use crate::geometry::Attr;

#[test]
fn test_bulge_flat_grid_and_zero_strength() {
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
        "center": [0.0, 0.0, 0.0],
        "radius": 2.0,
    }).as_object()).unwrap();
    let out_zero = cook(&[vec![arc_g.clone()]], &p_zero).unwrap();
    assert_eq!(out_zero.p(), arc_g.p());

    // Displaced by expected amount at center
    let p_disp = Params::resolve(NODE.params, serde_json::json!({
        "strength": 0.5,
        "center": [0.0, 0.0, 0.0],
        "radius": 2.0,
    }).as_object()).unwrap();
    let out_disp = cook(&[vec![arc_g.clone()]], &p_disp).unwrap();
    let p = out_disp.p();
    assert!((p[1] - 0.5).abs() < 1e-6);
    assert_eq!(p[0], 0.0);
    assert_eq!(p[2], 0.0);

    // Missing input returns error
    assert!(cook(&[vec![]], &Params::resolve(NODE.params, None).unwrap()).is_err());
}
