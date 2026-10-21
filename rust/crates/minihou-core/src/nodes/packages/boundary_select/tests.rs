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
fn test_cube_and_open_quad_boundary() {
    // A cube box with all faces selected gives no boundary
    let c = cube();
    let out_cube = run(&c, serde_json::json!({"domain": "faces", "group": ""})).unwrap();
    assert_eq!(edge_group_count(&out_cube, "boundary"), 0);

    // An open quad gives its 4 boundary edges
    let q = open_quad();
    let out_quad = run(&q, serde_json::json!({"domain": "faces", "group": ""})).unwrap();
    assert_eq!(edge_group_count(&out_quad, "boundary"), 4);

    // Selecting 1 face of the cube gives its 4 boundary edges
    let out_cube_one = run(&c, serde_json::json!({"domain": "faces", "group": "0"})).unwrap();
    assert_eq!(edge_group_count(&out_cube_one, "boundary"), 4);

    // Geometry is unchanged except for the new group
    assert_eq!(out_cube.p(), c.p());
    assert_eq!(out_cube.topology.vertex_point, c.topology.vertex_point);
    assert_eq!(out_cube.topology.prim_offsets, c.topology.prim_offsets);
    assert_eq!(out_quad.p(), q.p());
    assert_eq!(out_quad.topology.vertex_point, q.topology.vertex_point);
    assert_eq!(out_quad.topology.prim_offsets, q.topology.prim_offsets);
}

#[test]
fn test_preserve_existing_groups_and_custom_name() {
    let mut c = cube();
    c.prim.insert("group_existing_prim".into(), Arc::new(Attr::i32(1, vec![1; 6])));
    c.point.insert("group_existing_pt".into(), Arc::new(Attr::i32(1, vec![1; 8])));

    let out = run(&c, serde_json::json!({"output_group_name": "my_boundary"})).unwrap();
    assert_eq!(edge_group_count(&out, "my_boundary"), 0);
    assert!(out.prim.contains_key("group_existing_prim"));
    assert!(out.point.contains_key("group_existing_pt"));
}

#[test]
fn test_edge_and_point_domains() {
    // Open quad: 4 edges in a loop. With all 4 edges selected, all 4 are boundary edges of the quad
    let q = open_quad();
    let out_e_all = run(&q, serde_json::json!({"domain": "edges", "group": ""})).unwrap();
    assert_eq!(edge_group_count(&out_e_all, "boundary"), 4);
    assert!(!out_e_all.point.contains_key("group_boundary"));

    // If 1 edge of quad is selected, that edge is on the boundary
    let out_e_one = run(&q, serde_json::json!({"domain": "edges", "group": "0"})).unwrap();
    assert_eq!(edge_group_count(&out_e_one, "boundary"), 1);
    assert!(!out_e_one.point.contains_key("group_boundary"));

    // Points domain: on an open quad, all 4 points are on the mesh boundary, so all 4 boundary edges are output
    let out_pts = run(&q, serde_json::json!({"domain": "points", "group": ""})).unwrap();
    assert_eq!(edge_group_count(&out_pts, "boundary"), 4);
    assert!(!out_pts.point.contains_key("group_boundary"));

    // Default domain is edges: running without parameters selects all 4 boundary edges
    let out_default = run(&q, serde_json::json!({})).unwrap();
    assert_eq!(edge_group_count(&out_default, "boundary"), 4);
    assert!(!out_default.point.contains_key("group_boundary"));
}

#[test]
fn test_domain_options_no_duplicate_faces() {
    let raw = include_str!("param_meta.json");
    let meta: serde_json::Value = serde_json::from_str(raw).unwrap();
    let options = meta["domain"]["options"].as_array().unwrap();
    let opt_strs: Vec<&str> = options.iter().map(|v| v.as_str().unwrap()).collect();
    assert_eq!(opt_strs, vec!["edges", "faces", "points"]);
    let mut deduped = opt_strs.clone();
    deduped.sort_unstable();
    deduped.dedup();
    assert_eq!(deduped.len(), opt_strs.len());
    assert!(!opt_strs.contains(&"primitives"));
}

#[test]
fn test_output_group_is_stored() {
    let q = open_quad();
    // Default output group name is "boundary"
    let out = run(&q, serde_json::json!({})).unwrap();
    assert!(out.edge.contains_key("group_boundary"), "out.edge should contain group_boundary");
    assert!(out.edge.contains_key("edge_group_boundary"), "out.edge should contain edge_group_boundary");
    assert_eq!(out.edge["group_boundary"].rows(), out.nedges());
    assert_eq!(edge_group_count(&out, "boundary"), 4);

    // Custom output_group_name
    let out_custom = run(&q, serde_json::json!({"output_group_name": "border"})).unwrap();
    assert!(out_custom.edge.contains_key("group_border"), "out.edge should contain group_border");
    assert!(out_custom.edge.contains_key("edge_group_border"), "out.edge should contain edge_group_border");
    assert_eq!(edge_group_count(&out_custom, "border"), 4);
    assert_eq!(crate::nodes::edge_tools::values(&out_custom, "group_border").unwrap(), vec![1.0; 4]);
    assert_eq!(crate::nodes::group::flags(&out_custom.edge["group_border"]), vec![true; 4]);

    // Support output_group parameter alias
    let out_alias = run(&q, serde_json::json!({"output_group": "perimeter"})).unwrap();
    assert!(out_alias.edge.contains_key("group_perimeter"), "out.edge should contain group_perimeter");
    assert!(out_alias.edge.contains_key("edge_group_perimeter"), "out.edge should contain edge_group_perimeter");
    assert_eq!(edge_group_count(&out_alias, "perimeter"), 4);
    out_alias.validate().unwrap();
}

#[test]
fn test_errors_and_missing_input() {
    assert!(cook(&[vec![]], &Params::resolve(NODE.params, None).unwrap()).is_err());
    let q = open_quad();
    assert!(run(&q, serde_json::json!({"output_group_name": "123invalid"})).is_err());
    assert!(run(&q, serde_json::json!({"domain": "unknown_domain"})).is_err());
}

