use super::*;
use std::sync::Arc;
use serde_json::json;

fn unit_cube() -> Geometry {
    let p = vec![
        -0.5, -0.5, -0.5,
         0.5, -0.5, -0.5,
         0.5,  0.5, -0.5,
        -0.5,  0.5, -0.5,
        -0.5, -0.5,  0.5,
         0.5, -0.5,  0.5,
         0.5,  0.5,  0.5,
        -0.5,  0.5,  0.5,
    ];
    let vp = vec![
        0, 3, 2, 1, // -Z
        4, 5, 6, 7, // +Z
        0, 1, 5, 4, // -Y
        2, 3, 7, 6, // +Y
        0, 4, 7, 3, // -X
        1, 2, 6, 5, // +X
    ];
    let offs = vec![0, 4, 8, 12, 16, 20, 24];
    Geometry::from_arrays(p, vp, offs, None).unwrap()
}

fn run(g: Geometry, opt: serde_json::Value) -> Geometry {
    let params = Params::resolve(NODE.params, opt.as_object()).unwrap();
    (NODE.cook)(&[vec![Arc::new(g)]], &params).unwrap()
}

#[test]
fn test_prim_normals_on_cube() {
    let cube = unit_cube();
    let out = run(cube, json!({"domain": "prim"}));
    assert!(out.prim.contains_key("N"));
    let n = out.prim["N"].as_f32().unwrap();
    assert_eq!(n.len(), 6 * 3);
    // Face 0 is -Z
    assert!((n[0].abs() < 1e-5) && (n[1].abs() < 1e-5) && (n[2] - -1.0).abs() < 1e-5);
    // Face 1 is +Z
    assert!((n[3].abs() < 1e-5) && (n[4].abs() < 1e-5) && (n[5] - 1.0).abs() < 1e-5);
}

#[test]
fn test_point_normals_on_cube() {
    let cube = unit_cube();
    let out = run(cube, json!({"domain": "point"}));
    assert!(out.point.contains_key("N"));
    let n = out.point["N"].as_f32().unwrap();
    assert_eq!(n.len(), 8 * 3);
    // Point 0 is (-0.5, -0.5, -0.5), normal points outward along (-1, -1, -1)/sqrt(3)
    let inv_sqrt3 = 1.0f32 / (3.0f32).sqrt();
    assert!((n[0] - -inv_sqrt3).abs() < 1e-4);
    assert!((n[1] - -inv_sqrt3).abs() < 1e-4);
    assert!((n[2] - -inv_sqrt3).abs() < 1e-4);
}

#[test]
fn test_vertex_normals_hard_edges_cube() {
    let cube = unit_cube();
    // With cusp_angle 60.0 (less than 90 deg cube angle), every corner gets its face normal
    let out = run(cube, json!({"domain": "vertex", "cusp_angle": 60.0}));
    assert!(out.vertex.contains_key("N"));
    let n = out.vertex["N"].as_f32().unwrap();
    assert_eq!(n.len(), 24 * 3);
    // First 4 corners belong to -Z face
    for i in 0..4 {
        assert!((n[i * 3].abs() < 1e-5) && (n[i * 3 + 1].abs() < 1e-5) && (n[i * 3 + 2] - -1.0).abs() < 1e-5);
    }
}

#[test]
fn test_vertex_normals_smooth_cube() {
    let cube = unit_cube();
    // With cusp_angle 180.0, corners at each vertex share the averaged point normal
    let out = run(cube, json!({"domain": "vertex", "cusp_angle": 180.0}));
    assert!(out.vertex.contains_key("N"));
    let n = out.vertex["N"].as_f32().unwrap();
    let inv_sqrt3 = 1.0f32 / (3.0f32).sqrt();
    // Corner 0 is at point 0 (-0.5, -0.5, -0.5)
    assert!((n[0] - -inv_sqrt3).abs() < 1e-4);
    assert!((n[1] - -inv_sqrt3).abs() < 1e-4);
    assert!((n[2] - -inv_sqrt3).abs() < 1e-4);
}

#[test]
fn test_reverse_normals() {
    let cube = unit_cube();
    let out = run(cube, json!({"domain": "prim", "reverse": true}));
    let n = out.prim["N"].as_f32().unwrap();
    // Face 0 was -Z, reversed is +Z
    assert!((n[0].abs() < 1e-5) && (n[1].abs() < 1e-5) && (n[2] - 1.0).abs() < 1e-5);
}
