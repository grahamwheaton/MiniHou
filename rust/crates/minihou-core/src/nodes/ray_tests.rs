use super::*;
use serde_json::{json, Value};

fn run(g: Geometry, t: Geometry, values: Value) -> Geometry {
    let p = Params::resolve(RAY.params, values.as_object()).unwrap();
    let out = ray(&[vec![Arc::new(g)], vec![Arc::new(t)]], &p).unwrap();
    out.validate().unwrap();
    out
}
fn points(p: &[[f32; 3]]) -> Geometry {
    Geometry::points_only(p.iter().flatten().copied().collect()).unwrap()
}
fn plane(z: f32) -> Geometry {
    Geometry::from_arrays(
        vec![-10., -10., z, 10., -10., z, 10., 10., z, -10., 10., z],
        vec![0, 1, 2, 3],
        vec![0, 4],
        None,
    )
    .unwrap()
}
fn layers() -> Geometry {
    Geometry::from_arrays(
        vec![
            -10., -10., 0., 10., -10., 0., 0., 10., 0., -10., -10., -2., 10., -10., -2., 0., 10.,
            -2.,
        ],
        vec![0, 1, 2, 3, 4, 5],
        vec![0, 3, 6],
        None,
    )
    .unwrap()
}
fn scalar(g: &Geometry, name: &str, i: usize) -> f64 {
    match &g.point[name].data {
        AttrData::F32(v) => v[i] as f64,
        AttrData::I32(v) => v[i] as f64,
    }
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 2e-5, "{a} != {b}");
}
fn sphere() -> Geometry {
    let graph = crate::Graph::from_value(&json!({"nodes":{"s":{"type":"sphere","params":{"radius":1.,"rows":32,"columns":48}}},"display":"s"})).unwrap();
    (*graph.cook(None).unwrap()).clone()
}

#[test]
fn ray_flat_plane_defaults_and_legacy_lift() {
    let source = points(&[[0., 0., 2.], [20., 0., 2.]]);
    let out = run(source.clone(), plane(0.), json!({}));
    near(out.p()[2] as f64, 0.);
    near(scalar(&out, "hitdist", 0), 2.);
    assert_eq!(scalar(&out, "hitprim", 1), -1.);
    assert_eq!(scalar(&out, "group_rayHit", 0), 1.);
    assert_eq!(out.p()[5], 2.);
    assert!(out.detail.is_empty());
    let out = run(source, plane(0.), json!({"lift":0.25,"scale":0.5}));
    near(out.p()[2] as f64, 1.25);
}

#[test]
fn ray_sphere_scale_lift_bias_and_distance_toggle() {
    let source = points(&[[0., 0., 3.]]);
    let target = sphere();
    let params = json!({"method":"project_rays","direction_from":"specified_vector","scale":0.5,"lift":0.2,"bias":0.3,"point_intersection_normal":true,"point_intersection_distance":true});
    let out = run(source.clone(), target.clone(), params);
    let d = scalar(&out, "hitdist", 0);
    assert!((1.99..2.02).contains(&d));
    let nz = scalar(&out, "hitN", 2);
    near(out.p()[2] as f64, 3. - d * 0.5 - 0.2 + 0.3 * nz);
    near(scalar(&out, "dist", 0), d);
    assert!(scalar(&out, "N", 2) > 0.98);
    let miss = run(source.clone(), target.clone(), json!({"max_distance":1.9}));
    assert_eq!(miss.p()[2], 3.);
    let hit = run(
        source,
        target,
        json!({"max_distance":0.,"max_distance_enabled":false}),
    );
    assert!(hit.p()[2] < 1.01);
}

#[test]
fn ray_farthest_reverse_and_both_ways() {
    let g = points(&[[0., 0., 2.]]);
    let out = run(
        g.clone(),
        layers(),
        json!({"intersect_farthest_surface":true}),
    );
    near(out.p()[2] as f64, -2.);
    near(scalar(&out, "hitdist", 0), 4.);
    let out = run(
        g.clone(),
        layers(),
        json!({"intersect_farthest_surface":true,"max_distance":3.}),
    );
    near(out.p()[2] as f64, 0.);
    let out = run(
        g.clone(),
        layers(),
        json!({"direction":[0.,0.,1.],"reverse_rays":true}),
    );
    near(out.p()[2] as f64, 0.);
    let out = run(
        g,
        layers(),
        json!({"direction":[0.,0.,1.],"both_ways":true}),
    );
    near(out.p()[2] as f64, 0.);
    let out = run(
        points(&[[0., 0., 3.]]),
        sphere(),
        json!({"intersect_farthest_surface":true}),
    );
    near(out.p()[2] as f64, -1.);
}

#[test]
fn ray_minimum_distance_ignores_normals_and_zero_direction() {
    let g = points(&[[0.25, 0.25, 2.]]);
    let out = run(
        g.clone(),
        plane(0.),
        json!({"method":"minimum_distance","direction":[0.,0.,0.]}),
    );
    assert_eq!(out.p(), &[0.25, 0.25, 0.]);
    near(scalar(&out, "hitdist", 0), 2.);
    let miss = run(
        g,
        plane(0.),
        json!({"method":"minimum_distance","max_distance":1.}),
    );
    assert_eq!(miss.p()[2], 2.);
    let out = run(
        points(&[[0., 0., 3.]]),
        sphere(),
        json!({"method":"minimum_distance"}),
    );
    near(out.p()[2] as f64, 1.);
}

#[test]
fn ray_specified_attribute_and_generated_normal_directions() {
    let mut source = plane(2.);
    // Source normal points up: reverse must send it back toward the collision plane.
    let out = run(
        source.clone(),
        plane(0.),
        json!({"method":"project_rays","reverse_rays":true}),
    );
    assert!(out.p().chunks_exact(3).all(|p| p[2].abs() < 1e-5));
    source.point.insert(
        "aim".into(),
        Arc::new(Attr::f32(3, [0., 0., -7.].repeat(4))),
    );
    let out = run(
        source.clone(),
        plane(0.),
        json!({"method":"project_rays","direction_from":"attribute","direction_attribute":"aim"}),
    );
    assert_eq!(out.p()[2], 0.);
    let p = Params::resolve(RAY.params, json!({"method":"normal"}).as_object()).unwrap();
    assert!(ray(&[vec![Arc::new(source)], vec![Arc::new(plane(0.))]], &p).is_err());
}

#[test]
fn ray_primitive_selection_and_original_collision_primitive_numbers() {
    let mut collision = layers();
    collision
        .prim
        .insert("group_back".into(), Arc::new(Attr::i32(1, vec![0, 1])));
    let source = Geometry::from_arrays(
        vec![0., 0., 2., 1., 0., 2., 0., 1., 2., 2., 2., 2.],
        vec![0, 1, 2],
        vec![0, 3],
        None,
    )
    .unwrap();
    let out = run(
        source,
        collision,
        json!({"entity":"primitives","group":"0","collision_group":"back"}),
    );
    assert_eq!(out.p()[2], -2.);
    assert_eq!(out.p()[11], 2.);
    assert_eq!(scalar(&out, "hitprim", 0), 1.);
    assert_eq!(scalar(&out, "hitprim", 3), -1.);
}

#[test]
fn ray_hit_outputs_transform_off_and_barycentric_coordinates() {
    let t = Geometry::from_arrays(
        vec![0., 0., 0., 2., 0., 0., 0., 2., 0.],
        vec![0, 1, 2],
        vec![0, 3],
        None,
    )
    .unwrap();
    let out = run(
        points(&[[0.5, 1., 2.], [3., 3., 2.]]),
        t,
        json!({"transform_points":false,"create_point_group":true,"ray_hit_group":"landed","prim_num_attribute":"primid","prim_uvw_attribute_enabled":true,"prim_uvw_attribute":"coords","point_intersection_normal":true,"point_intersection_distance":true}),
    );
    assert_eq!(out.p()[2], 2.);
    near(scalar(&out, "coords", 0), 0.25);
    near(scalar(&out, "coords", 1), 0.5);
    assert_eq!(scalar(&out, "primid", 0), 0.);
    assert_eq!(scalar(&out, "primid", 1), -1.);
    assert_eq!(scalar(&out, "group_landed", 0), 1.);
    assert_eq!(scalar(&out, "group_landed", 1), 0.);
    assert!(!out.point.contains_key("hitprim"));
    assert!(!out.point.contains_key("group_rayHit"));
    near(scalar(&out, "dist", 0), 2.);
    near(scalar(&out, "N", 2), 1.);
}

#[test]
fn ray_optional_outputs_off_and_misses_preserve_input_normals() {
    let mut g = points(&[[0., 0., 2.], [20., 0., 2.]]);
    g.point.insert(
        "N".into(),
        Arc::new(Attr::f32(3, vec![1., 0., 0., 1., 0., 0.])),
    );
    let out = run(
        g,
        plane(0.),
        json!({"prim_num_attribute_enabled":false,"create_point_group":false,"point_intersection_normal":true}),
    );
    assert!(!out.point.contains_key("hitprim"));
    assert!(!out.point.contains_key("group_rayHit"));
    assert!(!out.point.contains_key("hitprimuv"));
    assert_eq!(&out.point["N"].as_f32().unwrap()[3..], &[1., 0., 0.]);
}

#[test]
fn ray_import_all_domains_and_preserve_misses() {
    let mut t = Geometry::from_arrays(
        vec![0., 0., 0., 2., 0., 0., 0., 2., 0.],
        vec![0, 1, 2],
        vec![0, 3],
        None,
    )
    .unwrap();
    t.point.insert(
        "Cd".into(),
        Arc::new(Attr::f32(3, vec![1., 0., 0., 0., 1., 0., 0., 0., 1.])),
    );
    t.point
        .insert("N".into(), Arc::new(Attr::f32(3, [0., 0., 2.].repeat(3))));
    t.vertex.insert(
        "uv".into(),
        Arc::new(Attr::f32(2, vec![0., 0., 1., 0., 0., 1.])),
    );
    t.prim.insert("id".into(), Arc::new(Attr::i32(1, vec![17])));
    t.detail.insert("temperature".into(), json!(42.));
    let mut g = points(&[[0.5, 1., 2.], [4., 4., 2.]]);
    g.point
        .insert("Cd".into(), Arc::new(Attr::f32(3, vec![9.; 6])));
    let out = run(
        g,
        t,
        json!({"import_attributes_from_hits":true,"point_attributes":"*","vertex_attributes":"uv","primitive_attributes":"id","detail_attributes":"temperature","point_intersection_normal":true}),
    );
    assert_eq!(out.p()[2], 0.); // P is never imported.
    for (i, v) in [0.25, 0.25, 0.5, 9., 9., 9.].iter().enumerate() {
        near(scalar(&out, "Cd", i), *v);
    }
    near(scalar(&out, "uv", 0), 0.25);
    near(scalar(&out, "uv", 1), 0.5);
    assert_eq!(scalar(&out, "id", 0), 17.);
    near(scalar(&out, "N", 2), 2.);
    near(scalar(&out, "temperature", 0), 42.);
    near(scalar(&out, "temperature", 1), 0.);
}

#[test]
fn ray_tolerance_accepts_near_edges_but_not_large_misses() {
    let t = Geometry::from_arrays(
        vec![0., 0., 0., 1., 0., 0., 0., 1., 0.],
        vec![0, 1, 2],
        vec![0, 3],
        None,
    )
    .unwrap();
    let g = points(&[[-0.005, 0.5, 1.], [-0.02, 0.5, 1.]]);
    let out = run(
        g.clone(),
        t.clone(),
        json!({"method":"project_rays","direction_from":"specified_vector"}),
    );
    assert_eq!(scalar(&out, "group_rayHit", 0), 1.);
    assert_eq!(scalar(&out, "group_rayHit", 1), 0.);
    let out = run(
        g,
        t,
        json!({"method":"project_rays","direction_from":"specified_vector","ray_tolerance":0.}),
    );
    assert_eq!(scalar(&out, "group_rayHit", 0), 0.);
}

#[test]
fn ray_seeded_jitter_and_combiners() {
    let g = points(&[[0., 0., 2.]]);
    // z = x/2: jitter changes both ray length and the sampled x attribute.
    let mut t = plane(0.);
    let mut p = t.p().to_vec();
    for q in p.chunks_exact_mut(3) {
        q[2] = q[0] * 0.5;
    }
    t.set_p(p);
    t.point.insert(
        "x".into(),
        Arc::new(Attr::f32(1, vec![-10., 10., 10., -10.])),
    );
    let params = json!({"method":"project_rays","direction_from":"specified_vector","sample":17,"jitter_scale":0.4,"seed":9,"import_attributes_from_hits":true,"point_attributes":"x"});
    let a = run(g.clone(), t.clone(), params.clone());
    let b = run(g.clone(), t.clone(), params.clone());
    assert_eq!(a.point, b.point);
    let mut different = params.clone();
    different["seed"] = json!(10);
    let c = run(g.clone(), t.clone(), different);
    assert_ne!(a.p(), c.p());
    let mut lengths = vec![];
    for mode in ["shortest", "median", "longest"] {
        let mut p = params.clone();
        p["ray_combiner"] = json!(mode);
        let out = run(g.clone(), t.clone(), p);
        lengths.push(scalar(&out, "hitdist", 0));
        // All combiners move along the original base ray, not sideways along a jitter ray.
        near(out.p()[0] as f64, 0.);
        near(out.p()[1] as f64, 0.);
    }
    assert!(lengths[0] < lengths[1] && lengths[1] < lengths[2]);
    let avg = scalar(&a, "hitdist", 0);
    assert!(lengths[0] < avg && avg < lengths[2]);
    // Average imported attributes must average sampled surface positions, not resample the combined point.
    assert!(scalar(&a, "x", 0).abs() > 1e-4);
    let mut zero = params;
    zero["jitter_scale"] = json!(0.);
    let out = run(g, t, zero);
    near(out.p()[2] as f64, 0.);
    near(scalar(&out, "x", 0), 0.);
}

#[test]
fn ray_empty_target_and_delete_only_selected_misses() {
    let g = points(&[[0., 0., 2.], [1., 1., 2.]]);
    let out = run(
        g.clone(),
        points(&[]),
        json!({"group":"0","delete_misses":true}),
    );
    assert_eq!(out.npoints, 1);
    assert_eq!(out.p(), &[1., 1., 2.]);
    let out = run(
        g,
        plane(0.),
        json!({"collision_group":"99","delete_misses":true}),
    );
    assert_eq!(out.npoints, 0);
}

#[test]
fn ray_guides_do_not_change_topology_or_positions() {
    let g = points(&[[0., 0., 2.]]);
    let out = run(g, plane(0.), json!({"show_guide_geometry":true}));
    assert_eq!((out.npoints, out.nprims()), (1, 0));
    assert_eq!(
        out.detail["__ray_guide_segments"].as_array().unwrap().len(),
        4
    );
    let out = run(out, plane(0.), json!({}));
    assert!(!out.detail.contains_key("__ray_guide_segments"));
}

#[test]
fn ray_bvh_matches_exhaustive_triangle_queries() {
    let t = sphere();
    let grid = Target::new(&t, &vec![true; t.nprims()]);
    assert!(grid.branches.len() > 1);
    let mut rng = 34;
    for _ in 0..30 {
        let o = [
            random01(&mut rng) * 4. - 2.,
            random01(&mut rng) * 4. - 2.,
            2.,
        ];
        let d = unit([random01(&mut rng) - 0.5, random01(&mut rng) - 0.5, -1.]);
        for far in [false, true] {
            let a = grid.cast(o, d, 10., far, 0.);
            let b = grid
                .triangles
                .iter()
                .enumerate()
                .filter_map(|(id, tri)| {
                    let single = Target {
                        triangles: vec![*tri],
                        branches: vec![Branch {
                            lo: [-10.; 3],
                            hi: [10.; 3],
                            left: None,
                            right: None,
                            tris: vec![0],
                        }],
                        root: Some(0),
                    };
                    single.cast(o, d, 10., far, 0.).map(|mut h| {
                        h.tri = id;
                        h
                    })
                })
                .reduce(|a, b| {
                    if (far && b.dist > a.dist) || (!far && b.dist < a.dist) {
                        b
                    } else {
                        a
                    }
                });
            assert_eq!(a.is_some(), b.is_some());
            if let (Some(a), Some(b)) = (a, b) {
                near(a.dist, b.dist);
            }
        }
        let a = grid.closest(o, 10.).unwrap();
        let b = grid
            .triangles
            .iter()
            .map(|t| len(sub(closest_on_tri(o, t.p), o)))
            .fold(f64::INFINITY, f64::min);
        near(a.dist, b);
    }
}

#[test]
fn ray_metadata_covers_every_registered_parameter() {
    let meta: Value =
        serde_json::from_str(include_str!("../../../minihou-app/src/param_meta.json")).unwrap();
    for p in RAY.params {
        let m = &meta["ray"][p.name];
        assert!(!m.is_null(), "missing ray metadata: {}", p.name);
        assert!(
            crate::params::same_value(&m["default"], &(p.default)()),
            "default {}",
            p.name
        );
    }
}

#[test]
fn ray_zero_distance_lift_and_scale_overshoot() {
    let out = run(
        points(&[[0., 0., 0.]]),
        plane(0.),
        json!({"method":"project_rays","direction_from":"specified_vector","lift":0.2}),
    );
    near(out.p()[2] as f64, -0.2);
    let out = run(points(&[[0., 0., 2.]]), plane(0.), json!({"scale":1.5}));
    near(out.p()[2] as f64, -1.);
}

#[test]
fn ray_import_replaces_mismatched_types_and_respects_exclusions() {
    let mut g = points(&[[0., 0., 2.]]);
    g.point
        .insert("value".into(), Arc::new(Attr::i32(1, vec![7])));
    let mut t = plane(0.);
    t.point
        .insert("value".into(), Arc::new(Attr::f32(1, vec![0.5; 4])));
    t.point
        .insert("skip".into(), Arc::new(Attr::f32(1, vec![2.; 4])));
    let out = run(
        g,
        t,
        json!({"import_attributes_from_hits":true,"point_attributes":"* ^skip"}),
    );
    assert_eq!(out.point["value"].dtype(), "float32");
    near(scalar(&out, "value", 0), 0.5);
    assert!(!out.point.contains_key("skip"));
}

#[test]
fn ray_degenerate_target_is_a_miss_for_closest_point() {
    let t = Geometry::from_arrays(
        vec![0., 0., 0., 1., 0., 0., 2., 0., 0.],
        vec![0, 1, 2],
        vec![0, 3],
        None,
    )
    .unwrap();
    let out = run(
        points(&[[0., 0., 2.]]),
        t,
        json!({"method":"minimum_distance"}),
    );
    assert_eq!(scalar(&out, "hitdist", 0), -1.);
    assert_eq!(out.p()[2], 2.);
}
