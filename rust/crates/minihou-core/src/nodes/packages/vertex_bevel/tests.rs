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
    assert!(crate::topo::edges(&out.topology, out.npoints)
        .count
        .iter()
        .all(|&n| n == 2));
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
    assert!(out.detail["vertex_bevel_skipped"][0]["reason"]
        .as_str()
        .unwrap()
        .contains("open"));
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
    assert!(crate::topo::edges(&b.topology, b.npoints)
        .count
        .iter()
        .all(|&n| n == 2));
    assert_ne!(a.p(), b.p());
}

#[test]
fn box_corner_matches_blender_grid_face_counts() {
    // Blender's M_ADJ vertex grid has n * floor(s/2) * ceil(s/2)
    // quads, plus a center ngon when s is odd (n = 3 here).
    for (segments, corner_faces) in [(1, 1), (2, 3), (3, 7), (4, 12), (5, 19)] {
        let out = run(
            cube(),
            json!({"group": "0", "width": 0.2, "segments": segments}),
        );
        out.validate().unwrap();
        assert_eq!(out.nprims(), 6 + corner_faces, "segments {segments}");
        let edges = crate::topo::edges(&out.topology, out.npoints);
        assert!(edges.count.iter().all(|&n| n == 2), "segments {segments}");
        let mut directed = std::collections::HashSet::new();
        for f in 0..out.nprims() {
            let r = range(&out, f);
            for v in r.clone() {
                let next = if v + 1 == r.end { r.start } else { v + 1 };
                assert!(directed.insert((
                    out.topology.vertex_point[v],
                    out.topology.vertex_point[next]
                )));
            }
        }
    }
}

#[test]
fn odd_segment_corner_has_one_even_triangular_cap() {
    let out = run(cube(), json!({"group": "0", "width": 0.2, "segments": 3}));
    let cap = out.nprims() - 1;
    let r = range(&out, cap);
    assert_eq!(r.len(), 3);
    let p = out.p();
    let vp = &out.topology.vertex_point;
    let ids: Vec<_> = r.map(|v| vp[v] as usize).collect();
    let lengths: Vec<f64> = (0..3)
        .map(|i| {
            (0..3)
                .map(|axis| {
                    let d = p[3 * ids[i] + axis] as f64 - p[3 * ids[(i + 1) % 3] + axis] as f64;
                    d * d
                })
                .sum::<f64>()
                .sqrt()
        })
        .collect();
    assert!(lengths[0] > 1e-5);
    assert!(lengths.iter().all(|&l| (l - lengths[0]).abs() < 1e-5));
    assert!(newell(&out, cap).iter().any(|&x| x.abs() > 1e-8));
}

#[test]
fn entire_box_has_blender_vertex_mode_face_count() {
    for (segments, faces) in [(1, 14), (2, 30), (3, 62), (4, 102)] {
        let out = run(cube(), json!({"width": 0.2, "segments": segments}));
        out.validate().unwrap();
        assert_eq!(out.nprims(), faces, "segments {segments}");
        assert!(crate::topo::edges(&out.topology, out.npoints)
            .count
            .iter()
            .all(|&n| n == 2));
    }
}

fn newell(g: &Geometry, f: usize) -> [f64; 3] {
    let p = g.p();
    let vp = &g.topology.vertex_point;
    let r = crate::nodes::model::range(g, f);
    let pts: Vec<[f64; 3]> = r
        .map(|v| {
            let i = vp[v] as usize * 3;
            [p[i] as f64, p[i + 1] as f64, p[i + 2] as f64]
        })
        .collect();
    let mut n = [0.; 3];
    for i in 0..pts.len() {
        let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
        n[0] += (a[1] - b[1]) * (a[2] + b[2]);
        n[1] += (a[2] - b[2]) * (a[0] + b[0]);
        n[2] += (a[0] - b[0]) * (a[1] + b[1]);
    }
    n
}

#[test]
fn bevel_faces_wind_outward_and_edges_are_oriented_consistently() {
    for segments in [1, 3] {
        let out = run(
            cube(),
            json!({"group": "0", "width": 0.2, "segments": segments}),
        );
        out.validate().unwrap();
        let vp = &out.topology.vertex_point;
        let mut directed = std::collections::HashSet::new();
        for f in 0..out.nprims() {
            let r = crate::nodes::model::range(&out, f);
            for v in r.clone() {
                let nx = if v + 1 == r.end { r.start } else { v + 1 };
                assert!(
                    directed.insert((vp[v], vp[nx])),
                    "directed edge used twice: inconsistent winding"
                );
            }
            // cube is centred on the origin: every face normal must point away from it
            let n = newell(&out, f);
            let c: Vec<f64> = (0..3)
                .map(|k| {
                    crate::nodes::model::range(&out, f)
                        .map(|v| out.p()[vp[v] as usize * 3 + k] as f64)
                        .sum::<f64>()
                })
                .collect();
            assert!(
                n[0] * c[0] + n[1] * c[1] + n[2] * c[2] > 0.0,
                "face {f} points inward"
            );
        }
    }
}

#[test]
fn valence_three_corner_matches_blender_vertex_mesh_counts() {
    // Blender's vertex mesh: ring j holds n * (s - 2j) points while that is
    // positive, plus one shared centre point when s is even; the original
    // vertex is removed. Faces: n * floor(s/2) * ceil(s/2) quads, plus a
    // centre n-gon when s is odd (M_POLY for s = 1).
    for (segments, points, faces) in [(1, 3, 1), (2, 7, 3), (3, 12, 7), (4, 19, 12)] {
        let g = cube();
        let out = run(g.clone(), json!({"group": "0", "width": 0.2, "segments": segments}));
        out.validate().unwrap();
        assert_eq!(out.npoints, g.npoints - 1 + points, "segments {segments}");
        assert_eq!(out.nprims(), g.nprims() + faces, "segments {segments}");
        assert!(crate::topo::edges(&out.topology, out.npoints)
            .count
            .iter()
            .all(|&n| n == 2));
        // No sliver triangles at the corner once the viewport fans each
        // polygon from its first point (trimmed faces start at a far point).
        let corner = [g.p()[0] as f64, g.p()[1] as f64, g.p()[2] as f64];
        let (tris, _) = crate::preview::fan_triangles(
            &out.topology.vertex_point,
            &out.topology.prim_offsets,
            out.topology.closed.as_deref(),
        );
        let p = out.p();
        let at = |i: u32| {
            let i = i as usize * 3;
            [p[i] as f64, p[i + 1] as f64, p[i + 2] as f64]
        };
        for t in tris.chunks(3) {
            let (a, b, c) = (at(t[0]), at(t[1]), at(t[2]));
            if [a, b, c].iter().any(|&q| len(sub(q, corner)) > 0.5) {
                continue;
            }
            for (o, u, v) in [(a, b, c), (b, c, a), (c, a, b)] {
                let (u, v) = (sub(u, o), sub(v, o));
                let cos = (u[0] * v[0] + u[1] * v[1] + u[2] * v[2]) / (len(u) * len(v));
                assert!(cos.acos().to_degrees() > 5.0, "segments {segments}: sliver {t:?}");
            }
        }
    }
}