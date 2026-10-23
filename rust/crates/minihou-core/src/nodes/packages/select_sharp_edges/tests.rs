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

#[test]
fn test_sharp_edges_tags_and_group_filter() {
    let c = cube();
    let out = run(&c, serde_json::json!({"angle": 30.0, "output_group_name": "sharp_edges"})).unwrap();
    assert_eq!(edge_group_count(&out, "sharp_edges"), 12);
    // Tags also written to edge_group_sharp
    assert_eq!(edge_group_count(&out, "sharp"), 12);
    // Group in out.edge
    assert!(out.edge.contains_key("group_sharp_edges"));

    // Filter to only 1 edge via group pattern
    let out_filt = run(&c, serde_json::json!({"angle": 30.0, "group": "0"})).unwrap();
    assert_eq!(edge_group_count(&out_filt, "sharp_edges"), 1);
}

fn flat_grid(n: usize) -> Geometry {
    let mut p = Vec::new();
    for y in 0..=n {
        for x in 0..=n {
            p.extend([x as f32, y as f32, 0.0]);
        }
    }
    let (mut vp, mut offs) = (Vec::new(), vec![0]);
    let w = (n + 1) as i32;
    for y in 0..n as i32 {
        for x in 0..n as i32 {
            let a = y * w + x;
            vp.extend([a, a + 1, a + w + 1, a + w]);
            offs.push(vp.len() as i32);
        }
    }
    Geometry::from_arrays(p, vp, offs, None).unwrap()
}

#[test]
fn test_flat_grid_has_no_sharp_edges() {
    let g = flat_grid(3);
    // 3x3 grid: 24 edges, 12 interior (two faces) and 12 boundary (one face); every dihedral angle is 0 degrees
    assert_eq!(crate::topo::edges(&g.topology, g.npoints).len(), 24);
    for angle in [0.0, 30.0] {
        let out = run(&g, serde_json::json!({"angle": angle})).unwrap();
        assert_eq!(edge_group_count(&out, "sharp_edges"), 0, "angle {angle}");
    }
}
