use super::*;
use serde_json::json;

fn run(g: &Geometry, params: serde_json::Value) -> Geometry {
    let p = Params::resolve(NODE.params, params.as_object()).unwrap();
    cook(&[vec![Arc::new(g.clone())]], &p).unwrap()
}

fn cube_with_sample(z: f32) -> Geometry {
    let mut g = Geometry::from_arrays(
        vec![
            0., 0., 0., 1., 0., 0., 1., 1., 0., 0., 1., 0., 0., 0., 1., 1., 0., 1., 1., 1., 1., 0.,
            1., 1., 0.1, 0.1, z,
        ],
        vec![
            0, 3, 2, 1, 4, 5, 6, 7, 0, 1, 5, 4, 1, 2, 6, 5, 2, 3, 7, 6, 3, 0, 4, 7,
        ],
        vec![0, 4, 8, 12, 16, 20, 24],
        None,
    )
    .unwrap();
    let mut normals = [0., 0., 1.].repeat(9);
    normals[24..27].copy_from_slice(&[0., 0., 1.]);
    g.point.insert("N".into(), Arc::new(Attr::f32(3, normals)));
    g.point
        .insert("weight".into(), Arc::new(Attr::f32(1, vec![2.; 9])));
    g
}

#[test]
fn registers_with_defaults() {
    assert!(crate::nodes::get("ambient_occlusion").is_some());
    let p = Params::resolve(NODE.params, None).unwrap();
    assert_eq!(p.s("name").unwrap(), "ao");
    assert_eq!(p.i("samples").unwrap(), 32);
}

#[test]
fn flat_plane_and_face_domain_are_unoccluded() {
    let g = Geometry::from_arrays(
        vec![-1., -1., 0., 1., -1., 0., 1., 1., 0., -1., 1., 0.],
        vec![0, 1, 2, 3],
        vec![0, 4],
        None,
    )
    .unwrap();
    let out = run(&g, json!({"samples": 12}));
    assert!(out.point["ao"].as_f32().unwrap().iter().all(|&v| v > 0.95));
    assert_eq!(out.topology, g.topology);
    assert_eq!(out.p(), g.p());
    let faces = run(
        &g,
        json!({"domain": "faces", "name": "face_ao", "samples": 12}),
    );
    assert_eq!(faces.prim["face_ao"].as_f32().unwrap(), &[1.]);
    assert_eq!(faces.point, g.point);
    faces.validate().unwrap();
}

#[test]
fn enclosed_bottom_corner_is_occluded_and_outside_face_is_open() {
    let inside = cube_with_sample(0.);
    let out = run(&inside, json!({"samples": 32, "max_distance": 2.0}));
    let ao = out.point["ao"].as_f32().unwrap()[8];
    assert!(ao < 0.5, "bottom corner AO was {ao}");
    assert_eq!(out.point["weight"], inside.point["weight"]);
    assert_eq!(out.topology, inside.topology);
    assert_eq!(out.p(), inside.p());
    assert_eq!(
        run(&inside, json!({"samples": 32})).point["ao"],
        run(&inside, json!({"samples": 32})).point["ao"]
    );

    let outside = cube_with_sample(1.);
    let out = run(&outside, json!({"samples": 16}));
    assert!(out.point["ao"].as_f32().unwrap()[8] > 0.95);
}

#[test]
fn isolated_point_uses_fallback_normal() {
    let g = Geometry::points_only(vec![0., 0., 0.]).unwrap();
    assert_eq!(
        run(&g, json!({"samples": 4})).point["ao"].as_f32().unwrap(),
        &[1.]
    );
}
