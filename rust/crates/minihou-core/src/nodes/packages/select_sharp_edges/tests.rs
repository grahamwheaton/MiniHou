use super::*;
use std::sync::Arc;

fn cube() -> Geometry {
    Geometry::from_arrays(
        vec![
            0., 0., 0., 1., 0., 0., 1., 1., 0., 0., 1., 0.,
            0., 0., 1., 1., 0., 1., 1., 1., 1., 0., 1., 1.,
        ],
        vec![
            0, 3, 2, 1, 4, 5, 6, 7, 0, 1, 5, 4, 1, 2, 6, 5, 2, 3, 7, 6, 3, 0, 4, 7,
        ],
        vec![0, 4, 8, 12, 16, 20, 24],
        None,
    )
    .unwrap()
}

fn open_quad() -> Geometry {
    Geometry::from_arrays(
        vec![0., 0., 0., 1., 0., 0., 1., 1., 0., 0., 1., 0.],
        vec![0, 1, 2, 3],
        vec![0, 4],
        None,
    )
    .unwrap()
}

fn run(geo: &Geometry, params: serde_json::Value) -> Result<Geometry> {
    let p = Params::resolve(NODE.params, params.as_object())?;
    cook(&[vec![Arc::new(geo.clone())]], &p)
}

fn edge_group_count(g: &Geometry, name: &str) -> usize {
    crate::nodes::edge_tools::values(g, &format!("edge_group_{name}"))
        .unwrap()
        .into_iter()
        .filter(|&x| x != 0.)
        .count()
}

#[test]
fn test_cube_sharp_edges_thresholds() {
    let c = cube();

    // Sharp edges on a cube at 30 degrees give all 12 edges
    let out_30 = run(&c, serde_json::json!({"angle": 30.0})).unwrap();
    assert_eq!(edge_group_count(&out_30, "sharp_edges"), 12);

    // Sharp edges on a cube at 100 degrees give none
    let out_100 = run(&c, serde_json::json!({"angle": 100.0})).unwrap();
    assert_eq!(edge_group_count(&out_100, "sharp_edges"), 0);

    // Geometry is unchanged except for the new group
    assert_eq!(out_30.p(), c.p());
    assert_eq!(out_30.topology.vertex_point, c.topology.vertex_point);
    assert_eq!(out_30.topology.prim_offsets, c.topology.prim_offsets);
}

#[test]
fn test_coplanar_and_open_mesh() {
    // 2 coplanar quads side by side: dihedral angle is 0 degrees, so threshold 30 gives none
    let g = Geometry::from_arrays(
        vec![
            0., 0., 0., 1., 0., 0., 1., 1., 0., 0., 1., 0.,
            2., 0., 0., 2., 1., 0.,
        ],
        vec![0, 1, 2, 3, 1, 4, 5, 2],
        vec![0, 4, 8],
        None,
    )
    .unwrap();
    let out_coplanar = run(&g, serde_json::json!({"angle": 30.0})).unwrap();
    assert_eq!(edge_group_count(&out_coplanar, "sharp_edges"), 0);

    // An open quad has no adjacent face pairs, so 0 sharp edges
    let q = open_quad();
    let out_q = run(&q, serde_json::json!({"angle": 30.0})).unwrap();
    assert_eq!(edge_group_count(&out_q, "sharp_edges"), 0);
}

#[test]
fn test_domain_fixed_to_edges_and_errors() {
    let c = cube();
    // domain must be edges
    assert!(run(&c, serde_json::json!({"domain": "faces"})).is_err());
    assert!(run(&c, serde_json::json!({"domain": "points"})).is_err());
    assert!(run(&c, serde_json::json!({"angle": -5.0})).is_err());
    assert!(run(&c, serde_json::json!({"output_group_name": "123invalid"})).is_err());
    assert!(cook(&[vec![]], &Params::resolve(NODE.params, None).unwrap()).is_err());
}
