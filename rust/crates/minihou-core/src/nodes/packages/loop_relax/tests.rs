use super::*;
use std::sync::Arc;
use crate::geometry::Geometry;
use crate::params::Params;

fn run_node(geo: Geometry, params: serde_json::Value) -> Geometry {
    let map = match params {
        serde_json::Value::Object(m) => Some(m),
        _ => None,
    };
    let p = Params::resolve(NODE.params, map.as_ref()).unwrap();
    (NODE.cook)(&[vec![Arc::new(geo)]], &p).unwrap()
}

#[test]
fn test_sawtooth_loop_smoothed_over_iterations_with_fixed_open_endpoints() {
    // Sawtooth open chain:
    // pt 0: (0, 0, 0)
    // pt 1: (1, 10, 0)
    // pt 2: (2, -10, 0)
    // pt 3: (3, 10, 0)
    // pt 4: (4, 0, 0)
    let pts = vec![
        0.0, 0.0, 0.0,
        1.0, 10.0, 0.0,
        2.0, -10.0, 0.0,
        3.0, 10.0, 0.0,
        4.0, 0.0, 0.0,
    ];
    let vp = vec![0, 1, 2, 3, 4];
    let offs = vec![0, 5];
    let geo = Geometry::from_arrays(pts, vp, offs, Some(vec![false])).unwrap();

    // Iteration 1:
    let cooked1 = run_node(
        geo.clone(),
        serde_json::json!({
            "group": "",
            "iterations": 1,
            "influence": 1.0
        }),
    );
    let p1_slice = cooked1.p();

    // Endpoints fixed
    assert_eq!(&p1_slice[0..3], &[0.0, 0.0, 0.0]);
    assert_eq!(&p1_slice[12..15], &[4.0, 0.0, 0.0]);

    // Exact Laplacian 1-step midpoints:
    // pt 1: y = (0 + (-10)) / 2 = -5.0
    // pt 2: y = (10 + 10) / 2 = 10.0
    // pt 3: y = (-10 + 0) / 2 = -5.0
    assert!((p1_slice[4] - (-5.0)).abs() < 1e-4);
    assert!((p1_slice[7] - 10.0).abs() < 1e-4);
    assert!((p1_slice[10] - (-5.0)).abs() < 1e-4);

    // Iteration 20: smoothing continues and deviations shrink towards 0
    let cooked20 = run_node(
        geo.clone(),
        serde_json::json!({
            "group": "",
            "iterations": 20,
            "influence": 1.0
        }),
    );
    let p20_slice = cooked20.p();

    // Endpoints still fixed
    assert_eq!(&p20_slice[0..3], &[0.0, 0.0, 0.0]);
    assert_eq!(&p20_slice[12..15], &[4.0, 0.0, 0.0]);

    // Deviations at interior points must be much smaller than initially (max initial was 10.0)
    for i in 1..4 {
        assert!(
            p20_slice[i * 3 + 1].abs() < 2.0,
            "y deviation at pt {i} should be smoothed close to 0, got {}",
            p20_slice[i * 3 + 1]
        );
    }
}

#[test]
fn test_influence_zero_and_half() {
    let pts = vec![
        0.0, 0.0, 0.0,
        1.0, 10.0, 0.0,
        2.0, 0.0, 0.0,
    ];
    let vp = vec![0, 1, 2];
    let offs = vec![0, 3];
    let geo = Geometry::from_arrays(pts, vp, offs, Some(vec![false])).unwrap();

    // influence = 0
    let cooked0 = run_node(
        geo.clone(),
        serde_json::json!({
            "group": "",
            "iterations": 5,
            "influence": 0.0
        }),
    );
    assert_eq!(cooked0.p(), geo.p());

    // influence = 0.5: midpoint for pt 1 after 1 iteration is 0.0; halfway between 10.0 and 0.0 is 5.0
    let cooked_half = run_node(
        geo.clone(),
        serde_json::json!({
            "group": "",
            "iterations": 1,
            "influence": 0.5
        }),
    );
    assert!((cooked_half.p()[4] - 5.0).abs() < 1e-4);
}
