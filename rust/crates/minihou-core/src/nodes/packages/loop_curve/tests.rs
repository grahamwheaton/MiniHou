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
fn test_deviated_line_points_straightened() {
    // Open chain from (0,0,0) to (10,0,0) with interior points perturbed in y and z
    let pts = vec![
        0.0, 0.0, 0.0,      // Pt 0 (endpoint)
        2.0, 3.0, -1.0,     // Pt 1
        5.0, -4.0, 2.0,     // Pt 2
        7.0, 1.5, 0.5,      // Pt 3
        10.0, 0.0, 0.0,     // Pt 4 (endpoint)
    ];
    let vp = vec![0, 1, 2, 3, 4];
    let offs = vec![0, 5];
    let geo = Geometry::from_arrays(pts, vp, offs, Some(vec![false])).unwrap();

    let cooked = run_node(
        geo,
        serde_json::json!({
            "group": "",
            "influence": 1.0
        }),
    );
    let p_slice = cooked.p();

    // Endpoints unchanged
    assert_eq!(&p_slice[0..3], &[0.0, 0.0, 0.0]);
    assert_eq!(&p_slice[12..15], &[10.0, 0.0, 0.0]);

    // Interior points have y and z projected to 0, x coordinates preserved
    assert!((p_slice[3] - 2.0).abs() < 1e-4);
    assert!(p_slice[4].abs() < 1e-4);
    assert!(p_slice[5].abs() < 1e-4);

    assert!((p_slice[6] - 5.0).abs() < 1e-4);
    assert!(p_slice[7].abs() < 1e-4);
    assert!(p_slice[8].abs() < 1e-4);

    assert!((p_slice[9] - 7.0).abs() < 1e-4);
    assert!(p_slice[10].abs() < 1e-4);
    assert!(p_slice[11].abs() < 1e-4);
}

#[test]
fn test_influence_zero_and_half() {
    let pts = vec![
        0.0, 0.0, 0.0,
        5.0, 4.0, 0.0,
        10.0, 0.0, 0.0,
    ];
    let vp = vec![0, 1, 2];
    let offs = vec![0, 3];
    let geo = Geometry::from_arrays(pts, vp, offs, Some(vec![false])).unwrap();

    // influence = 0: identity
    let cooked0 = run_node(
        geo.clone(),
        serde_json::json!({
            "group": "",
            "influence": 0.0
        }),
    );
    assert_eq!(cooked0.p(), geo.p());

    // influence = 0.5: y is halved to 2.0
    let cooked_half = run_node(
        geo.clone(),
        serde_json::json!({
            "group": "",
            "influence": 0.5
        }),
    );
    assert!((cooked_half.p()[4] - 2.0).abs() < 1e-4);
    assert!((cooked_half.p()[3] - 5.0).abs() < 1e-4);
}

#[test]
fn test_closed_loop_is_ignored() {
    let pts = vec![
        0.0, 0.0, 0.0,
        1.0, 1.0, 0.0,
        0.0, 1.0, 0.0,
    ];
    let vp = vec![0, 1, 2];
    let offs = vec![0, 3];
    let geo = Geometry::from_arrays(pts, vp, offs, Some(vec![true])).unwrap();

    let cooked = run_node(
        geo.clone(),
        serde_json::json!({
            "group": "",
            "influence": 1.0
        }),
    );
    assert_eq!(cooked.p(), geo.p());
}
