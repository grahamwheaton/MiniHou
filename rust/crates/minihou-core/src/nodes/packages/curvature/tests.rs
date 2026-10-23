use super::*;
use serde_json::json;

fn run(g: &Geometry, params: serde_json::Value) -> Geometry {
    let p = Params::resolve(NODE.params, params.as_object()).unwrap();
    cook(&[vec![Arc::new(g.clone())]], &p).unwrap()
}

fn grid() -> Geometry {
    let mut positions = Vec::new();
    for y in 0..3 {
        for x in 0..3 {
            positions.extend_from_slice(&[x as f32, y as f32, 0.]);
        }
    }
    Geometry::from_arrays(
        positions,
        vec![0, 1, 4, 3, 1, 2, 5, 4, 3, 4, 7, 6, 4, 5, 8, 7],
        vec![0, 4, 8, 12, 16],
        None,
    )
    .unwrap()
}

fn cube() -> Geometry {
    Geometry::from_arrays(
        vec![
            0., 0., 0., 1., 0., 0., 1., 1., 0., 0., 1., 0., 0., 0., 1., 1., 0., 1., 1., 1., 1., 0.,
            1., 1.,
        ],
        vec![
            0, 3, 2, 1, 4, 5, 6, 7, 0, 1, 5, 4, 1, 2, 6, 5, 2, 3, 7, 6, 3, 0, 4, 7,
        ],
        vec![0, 4, 8, 12, 16, 20, 24],
        None,
    )
    .unwrap()
}

#[test]
fn registers_with_defaults() {
    assert!(crate::nodes::get("curvature").is_some());
    let p = Params::resolve(NODE.params, None).unwrap();
    assert_eq!(p.s("name").unwrap(), "curvature");
    assert_eq!(p.s("mode").unwrap(), "signed");
}

#[test]
fn flat_grid_including_open_boundary_is_zero() {
    let g = grid();
    let out = run(&g, json!({}));
    assert!(out.point["curvature"]
        .as_f32()
        .unwrap()
        .iter()
        .all(|v| v.abs() < 1e-6));
    assert_eq!(out.topology, g.topology);
    assert_eq!(out.p(), g.p());
    out.validate().unwrap();
}

#[test]
fn cube_corners_are_convex_and_geometry_is_preserved() {
    let mut g = cube();
    g.point
        .insert("weight".into(), Arc::new(Attr::f32(1, vec![0.5; 8])));
    let out = run(&g, json!({"name": "bend"}));
    assert!(out.point["bend"].as_f32().unwrap().iter().all(|&v| v > 0.5));
    assert_eq!(out.point["weight"], g.point["weight"]);
    assert_eq!(out.topology, g.topology);
    assert_eq!(out.p(), g.p());
    let convex = run(&g, json!({"mode": "convex"}));
    let concave = run(&g, json!({"mode": "concave"}));
    assert!(convex.point["curvature"]
        .as_f32()
        .unwrap()
        .iter()
        .all(|&v| v > 0.5));
    assert!(concave.point["curvature"]
        .as_f32()
        .unwrap()
        .iter()
        .all(|&v| v == 0.));
}

#[test]
fn inward_winding_exposes_concavity() {
    let mut g = cube();
    let topo = Arc::make_mut(&mut g.topology);
    for face in topo.vertex_point.chunks_exact_mut(4) {
        face.reverse();
    }
    let out = run(&g, json!({"mode": "concave"}));
    assert!(out.point["curvature"]
        .as_f32()
        .unwrap()
        .iter()
        .all(|&v| v > 0.5));
}
