use super::*;
use serde_json::json;

fn cube() -> Geometry {
    let spec = crate::nodes::get("box").unwrap();
    let params = Params::resolve(spec.params, None).unwrap();
    (spec.cook)(&[vec![]], &params).unwrap()
}

fn run(g: Geometry, options: serde_json::Value) -> Geometry {
    let params = Params::resolve(NODE.params, options.as_object()).unwrap();
    (NODE.cook)(&[vec![Arc::new(g)]], &params).unwrap()
}

#[test]
fn one_cube_corner_adds_one_face_and_two_points() {
    let g = cube();
    let out = run(g.clone(), json!({"group": "0", "width": 0.2}));
    out.validate().unwrap();
    assert_eq!(out.nprims(), g.nprims() + 1);
    assert_eq!(out.npoints, g.npoints + 2);
    assert!(
        crate::topo::edges(&out.topology, out.npoints)
            .count
            .iter()
            .all(|&n| n == 2)
    );
    assert!(out.detail.get("vertex_bevel_skipped").is_none());
}

#[test]
fn zero_width_preserves_mesh() {
    let g = cube();
    let out = run(g.clone(), json!({"group": "0", "width": 0.0}));
    assert_eq!(out.p(), g.p());
    assert_eq!(out.topology, g.topology);
    assert_eq!(out.nprims(), g.nprims());
}

#[test]
fn boundary_vertex_is_skipped_with_note() {
    let g = Geometry::from_arrays(
        vec![0., 0., 0., 1., 0., 0., 0., 1., 0.],
        vec![0, 1, 2],
        vec![0, 3],
        Some(vec![true]),
    )
    .unwrap();
    let out = run(g.clone(), json!({"group": "0"}));
    assert_eq!(out.topology, g.topology);
    assert_eq!(out.p(), g.p());
    assert_eq!(out.detail["vertex_bevel_skipped"][0]["point"], 0);
    assert!(
        out.detail["vertex_bevel_skipped"][0]["reason"]
            .as_str()
            .unwrap()
            .contains("open")
    );
}

#[test]
fn segmented_profile_keeps_cube_closed() {
    let g = cube();
    let a = run(
        g.clone(),
        json!({"group": "0", "segments": 3, "profile": 0.0}),
    );
    let b = run(g, json!({"group": "0", "segments": 3, "profile": 1.0}));
    a.validate().unwrap();
    b.validate().unwrap();
    assert!(
        crate::topo::edges(&b.topology, b.npoints)
            .count
            .iter()
            .all(|&n| n == 2)
    );
    assert_ne!(a.p(), b.p());
}
