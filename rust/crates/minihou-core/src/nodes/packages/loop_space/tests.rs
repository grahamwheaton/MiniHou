use super::*;
use std::sync::Arc;
use crate::geometry::Geometry;
use crate::params::Params;

#[test]
fn test_unequal_open_grid_edge_gives_uniform_spacing() {
    // 4 points along X at unequal intervals: 0.0, 0.1, 0.8, 1.0
    let pts = vec![
        0.0, 0.0, 0.0,
        0.1, 0.0, 0.0,
        0.8, 0.0, 0.0,
        1.0, 0.0, 0.0,
    ];
    let vp = vec![0, 1, 2, 3];
    let offs = vec![0, 4];
    let geo = Geometry::from_arrays(pts, vp, offs, Some(vec![false])).unwrap();

    let p = Params::resolve(NODE.params, None).unwrap();
    let out = (NODE.cook)(&[vec![Arc::new(geo)]], &p).unwrap();

    let p_out = out.p();
    // Endpoints unchanged
    assert!((p_out[0] - 0.0).abs() < 1e-5);
    assert!((p_out[9] - 1.0).abs() < 1e-5);

    // Intermediate points spaced uniformly at 1/3 and 2/3
    let expected_1 = 1.0 / 3.0;
    let expected_2 = 2.0 / 3.0;
    assert!((p_out[3] - expected_1).abs() < 1e-5, "got {}, expected {}", p_out[3], expected_1);
    assert!((p_out[6] - expected_2).abs() < 1e-5, "got {}, expected {}", p_out[6], expected_2);

    // Segment lengths must be equal
    let d1 = p_out[3] - p_out[0];
    let d2 = p_out[6] - p_out[3];
    let d3 = p_out[9] - p_out[6];
    assert!((d1 - d2).abs() < 1e-5);
    assert!((d2 - d3).abs() < 1e-5);
}

#[test]
fn test_closed_loop_stays_closed() {
    // 4 points around a square with side 2.0, with an initially unequal point at (0.5, 0, 0)
    let pts = vec![
        0.0, 0.0, 0.0,
        0.5, 0.0, 0.0, // unequally spaced along bottom edge
        2.0, 2.0, 0.0,
        0.0, 2.0, 0.0,
    ];
    let vp = vec![0, 1, 2, 3];
    let offs = vec![0, 4];
    let geo = Geometry::from_arrays(pts, vp, offs, Some(vec![true])).unwrap();

    let p = Params::resolve(NODE.params, None).unwrap();
    let out = (NODE.cook)(&[vec![Arc::new(geo)]], &p).unwrap();

    // Loop topology remains closed
    assert_eq!(out.topology.is_closed(0), true);

    // The points should now be uniformly spaced around the perimeter
    // Perimeter = 0.5 + sqrt(1.5^2 + 2^2) + 2 + 2 = 0.5 + 2.5 + 4 = 7.0
    // Segment length = 7.0 / 4 = 1.75
    let p_out = out.p();
    let get_pt = |i: usize| [p_out[i * 3], p_out[i * 3 + 1], p_out[i * 3 + 2]];

    // Verify all 4 points remain connected and moved to uniform arc-length positions
    assert_eq!(get_pt(0), [0.0, 0.0, 0.0]); // Start point fixed
    // Point 1 was at (0.5, 0, 0); target distance is 1.75 along bottom and right edges
    // First segment length is 0.5, second is 2.5, so 1.75 is at 0.5 + 0.5 * (1.5, 2.0) = (1.25, 1.0, 0)
    let p1 = get_pt(1);
    assert!((p1[0] - 1.25).abs() < 1e-4, "p1.x = {}", p1[0]);
    assert!((p1[1] - 1.0).abs() < 1e-4, "p1.y = {}", p1[1]);

    // Check that perimeter is preserved and loop remains closed
    let d0 = dist3(get_pt(0), [0.5, 0.0, 0.0]) + dist3([0.5, 0.0, 0.0], get_pt(1));
    assert!((d0 - 1.75).abs() < 1e-4, "arc length of first segment should be 1.75, got {d0}");
}

#[test]
fn test_influence_zero_is_identity() {
    let pts = vec![
        0.0, 0.0, 0.0,
        0.1, 0.0, 0.0,
        0.8, 0.0, 0.0,
        1.0, 0.0, 0.0,
    ];
    let vp = vec![0, 1, 2, 3];
    let offs = vec![0, 4];
    let geo = Geometry::from_arrays(pts.clone(), vp, offs, Some(vec![false])).unwrap();

    let mut map = serde_json::Map::new();
    map.insert("influence".into(), serde_json::json!(0.0));
    let p = Params::resolve(NODE.params, Some(&map)).unwrap();
    let out = (NODE.cook)(&[vec![Arc::new(geo)]], &p).unwrap();

    assert_eq!(out.p(), pts.as_slice());
}
