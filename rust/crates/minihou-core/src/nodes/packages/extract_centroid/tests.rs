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
        0, 1, 2, 3, // -Z
        4, 7, 6, 5, // +Z
        0, 4, 5, 1, // -Y
        3, 2, 6, 7, // +Y
        0, 3, 7, 4, // -X
        1, 5, 6, 2, // +X
    ];
    let offs = vec![0, 4, 8, 12, 16, 20, 24];
    Geometry::from_arrays(p, vp, offs, None).unwrap()
}

fn grid_2x2() -> Geometry {
    // 2x2 cells = 9 points, 4 quads
    let p = vec![
        0.0, 0.0, 0.0,
        1.0, 0.0, 0.0,
        2.0, 0.0, 0.0,
        0.0, 1.0, 0.0,
        1.0, 1.0, 0.0,
        2.0, 1.0, 0.0,
        0.0, 2.0, 0.0,
        1.0, 2.0, 0.0,
        2.0, 2.0, 0.0,
    ];
    let vp = vec![
        0, 1, 4, 3,
        1, 2, 5, 4,
        3, 4, 7, 6,
        4, 5, 8, 7,
    ];
    let offs = vec![0, 4, 8, 12, 16];
    Geometry::from_arrays(p, vp, offs, None).unwrap()
}

fn run(g: Geometry, opt: serde_json::Value) -> Geometry {
    let params = Params::resolve(NODE.params, opt.as_object()).unwrap();
    (NODE.cook)(&[vec![Arc::new(g)]], &params).unwrap()
}

#[test]
fn test_detail_centroid_of_unit_cube() {
    let cube = unit_cube();
    let out = run(cube, json!({"domain": "detail", "method": "center_of_mass", "output": "points"}));
    assert_eq!(out.npoints, 1);
    let p = out.p();
    assert!((p[0].abs() < 1e-5) && (p[1].abs() < 1e-5) && (p[2].abs() < 1e-5));
}

#[test]
fn test_detail_bbox_centroid_offset() {
    let mut cube = unit_cube();
    let mut p = cube.p().to_vec();
    for xyz in p.chunks_exact_mut(3) {
        xyz[0] += 5.0;
        xyz[1] += 2.0;
        xyz[2] += -1.0;
    }
    cube.set_p(p);
    let out = run(cube, json!({"domain": "detail", "method": "bounding_box", "output": "points"}));
    assert_eq!(out.npoints, 1);
    let p = out.p();
    assert!((p[0] - 5.0).abs() < 1e-5);
    assert!((p[1] - 2.0).abs() < 1e-5);
    assert!((p[2] - -1.0).abs() < 1e-5);
}

#[test]
fn test_primitive_centroids_grid() {
    let g = grid_2x2();
    let out = run(g, json!({"domain": "primitives", "method": "center_of_mass", "output": "points"}));
    assert_eq!(out.npoints, 4);
    let p = out.p();
    // Cell centers: (0.5, 0.5), (1.5, 0.5), (0.5, 1.5), (1.5, 1.5)
    assert!((p[0] - 0.5).abs() < 1e-5 && (p[1] - 0.5).abs() < 1e-5);
    assert!((p[3] - 1.5).abs() < 1e-5 && (p[4] - 0.5).abs() < 1e-5);
    assert!((p[6] - 0.5).abs() < 1e-5 && (p[7] - 1.5).abs() < 1e-5);
    assert!((p[9] - 1.5).abs() < 1e-5 && (p[10] - 1.5).abs() < 1e-5);
}

#[test]
fn test_output_attribute_mode() {
    let g = grid_2x2();
    let out = run(g.clone(), json!({"domain": "primitives", "output": "attribute", "attribute_name": "cen"}));
    assert_eq!(out.nprims(), 4);
    assert!(out.prim.contains_key("cen"));
    let cen = out.prim["cen"].as_f32().unwrap();
    assert_eq!(cen.len(), 12);
    assert!((cen[0] - 0.5).abs() < 1e-5 && (cen[1] - 0.5).abs() < 1e-5);
}

#[test]
fn test_pieces_mode_by_connectivity() {
    // Two disconnected quads
    let p = vec![
        0.0, 0.0, 0.0,
        1.0, 0.0, 0.0,
        1.0, 1.0, 0.0,
        0.0, 1.0, 0.0,
        10.0, 10.0, 0.0,
        11.0, 10.0, 0.0,
        11.0, 11.0, 0.0,
        10.0, 11.0, 0.0,
    ];
    let vp = vec![0, 1, 2, 3, 4, 5, 6, 7];
    let offs = vec![0, 4, 8];
    let g = Geometry::from_arrays(p, vp, offs, None).unwrap();
    let out = run(g, json!({"domain": "pieces", "output": "points"}));
    assert_eq!(out.npoints, 2);
    let pts = out.p();
    assert!((pts[0] - 0.5).abs() < 1e-5 && (pts[1] - 0.5).abs() < 1e-5);
    assert!((pts[3] - 10.5).abs() < 1e-5 && (pts[4] - 10.5).abs() < 1e-5);
}

#[test]
fn test_transfer_attribute_primitives() {
    let mut g = grid_2x2();
    let prim_class = vec![10, 20, 30, 40];
    g.prim.insert("tag".into(), Arc::new(Attr::i32(1, prim_class)));
    let out = run(g, json!({"domain": "primitives", "transfer_attribute": "tag"}));
    assert_eq!(out.npoints, 4);
    assert!(out.point.contains_key("tag"));
    let tag = out.point["tag"].as_i32().unwrap();
    assert_eq!(tag, &[10, 20, 30, 40]);
}

#[test]
fn test_extract_centroid_parameter_order_and_defaults() {
    let expected = &[
        "group",
        "domain",
        "method",
        "output",
        "attribute_name",
        "transfer_attribute",
    ];
    let names: Vec<&str> = NODE.params.iter().map(|p| p.name).collect();
    assert_eq!(names, expected);

    let cube = unit_cube();
    let out = run(cube, json!({}));
    assert_eq!(out.npoints, 1);
}
