//! Self-contained checks of the core (the comparison with Python is `tests/test_rust_parity.py` in the repository root).

use minihou_core::{nodes, topo, Graph};
use serde_json::json;

fn cook(graph: serde_json::Value) -> minihou_core::Result<std::sync::Arc<minihou_core::Geometry>> {
    Graph::from_value(&graph)?.cook(None)
}

#[test]
fn grid_has_the_python_layout() {
    let g = cook(json!({"nodes": {"g": {"type": "grid", "params": {"columns": 3, "rows": 2, "size_x": 2.0, "size_y": 1.0}}}, "display": "g"})).unwrap();
    assert_eq!((g.npoints, g.nprims(), g.nvertices()), (6, 2, 8));
    assert_eq!(g.topology.vertex_point, vec![0, 1, 4, 3, 1, 2, 5, 4]);
    assert_eq!(&g.p()[..6], &[-1.0, -0.5, 0.0, 0.0, -0.5, 0.0]);
    assert_eq!(g.detail["grid_columns"], json!(3));
}

#[test]
fn box_is_eight_welded_points_and_six_quads() {
    let g = cook(json!({"nodes": {"b": {"type": "box"}}, "display": "b"})).unwrap();
    assert_eq!((g.npoints, g.nprims()), (8, 6));
    let t = cook(json!({"nodes": {"b": {"type": "box", "params": {"connectivity": "triangles", "divisions": 3}}}, "display": "b"})).unwrap();
    assert_eq!((t.npoints, t.nprims()), (6 * 9 + 2, 6 * 9 * 2));
}

#[test]
fn edges_of_a_closed_box_are_shared_twice_and_sorted() {
    let g = cook(json!({"nodes": {"b": {"type": "box", "params": {"divisions": 4}}}, "display": "b"})).unwrap();
    let e = topo::edges(&g.topology, g.npoints);
    assert!(e.count.iter().all(|&c| c == 2));
    // Euler: V - E + F = 2
    assert_eq!(g.npoints as i64 - e.len() as i64 + g.nprims() as i64, 2);
    let pairs: Vec<(i64, i64)> = e.points.chunks(2).map(|p| (p[0], p[1])).collect();
    assert!(pairs.windows(2).all(|w| w[0] < w[1]));
    assert!(e.of_vertex.iter().all(|&x| x >= 0));
}

#[test]
fn open_curves_have_no_closing_edge() {
    let topo = minihou_core::Topology { vertex_point: vec![0, 1, 2], prim_offsets: vec![0, 3], closed: Some(vec![false]) };
    let e = topo::edges(&topo, 3);
    assert_eq!(e.points, vec![0, 1, 1, 2]);
    assert_eq!(e.of_vertex, vec![0, 1, -1]);
}

#[test]
fn transform_moves_and_merge_concatenates() {
    let g = cook(json!({"nodes": {
        "a": {"type": "grid", "params": {"columns": 2, "rows": 2}},
        "t": {"type": "transform", "inputs": {"geo": "a"}, "params": {"translate": [0, 0, 5]}},
        "m": {"type": "merge", "inputs": {"geos": ["a", "t"]}},
        "out": {"type": "output", "inputs": {"geo": "m"}}}})).unwrap();
    assert_eq!((g.npoints, g.nprims()), (8, 2));
    assert_eq!(g.p()[3 * 4 + 2], 5.0);
    assert_eq!(g.topology.vertex_point[4..], [4, 5, 7, 6]);
}

#[test]
fn supported_group_and_unsupported_node() {
    let g = cook(json!({"nodes": {"a": {"type": "line", "params": {"points": 3}},
        "t": {"type": "transform", "inputs": {"geo": "a"}, "params": {"group": "1", "translate": [1,0,0]}}}, "display": "t"})).unwrap();
    assert_eq!(g.p(), &[0.0,0.0,0.0,1.0,0.0,0.5,0.0,0.0,1.0]);
    let err = cook(json!({"nodes": {"w": {"type": "wrangle"}}, "display": "w"})).unwrap_err();
    assert!(err.0.contains("not in the Rust backend"), "{err}");
}

#[test]
fn curve_line_circle_and_proportional() {
    let point = cook(json!({"nodes": {"p": {"type": "point", "params": {"center": [1,2,3]}}}, "display": "p"})).unwrap();
    assert_eq!((point.npoints, point.nprims()), (1, 0));
    assert_eq!(point.p(), &[1.0, 2.0, 3.0]);
    let curve = cook(json!({"nodes": {"c": {"type": "curve", "params": {"points": "0,0,0 1,0,0 1,1,0", "closed": true}}}, "display": "c"})).unwrap();
    assert_eq!(curve.topology.vertex_point, [0,1,2]);
    assert_eq!(curve.topology.closed, Some(vec![true]));
    let circle = cook(json!({"nodes": {"c": {"type": "circle", "params": {"divisions": 4}}}, "display": "c"})).unwrap();
    assert_eq!((circle.npoints, circle.nprims()), (4,1));
    let g = cook(json!({"nodes": {"a": {"type": "line", "params": {"points": 3, "length": 2}},
        "t": {"type": "transform", "inputs": {"geo": "a"}, "params": {"group": "0", "proportional_enabled": true,
            "proportional_radius": 2, "proportional_falloff": "linear", "translate": [1,0,0]}}}, "display": "t"})).unwrap();
    assert_eq!(g.p(), &[1.0,0.0,0.0,0.5,0.0,1.0,0.0,0.0,2.0]);
}

#[test]
fn sphere_tube_torus_cook() {
    let sphere = cook(json!({"nodes": {"s": {"type": "sphere", "params": {"rows": 4, "columns": 6}}}, "display": "s"})).unwrap();
    assert_eq!((sphere.npoints, sphere.nprims()), (20, 24));

    let tube = cook(json!({"nodes": {"t": {"type": "tube", "params": {"rows": 2, "columns": 4, "caps": true}}}, "display": "t"})).unwrap();
    assert_eq!((tube.npoints, tube.nprims()), (12, 10));

    let cone = cook(json!({"nodes": {"t": {"type": "tube", "params": {"radius_bottom": 0.0, "radius_top": 1.0, "rows": 2, "columns": 4, "caps": true}}}, "display": "t"})).unwrap();
    assert_eq!((cone.npoints, cone.nprims()), (9, 9));

    let torus = cook(json!({"nodes": {"t": {"type": "torus", "params": {"rows": 3, "columns": 4}}}, "display": "t"})).unwrap();
    assert_eq!((torus.npoints, torus.nprims()), (12, 12));
}

#[test]
fn errors_name_the_node_and_cycles_are_caught() {
    let err = cook(json!({"nodes": {"t": {"type": "transform"}}, "display": "t"})).unwrap_err();
    assert_eq!(err.0, "node 't' (transform): input 'geo' is not connected");
    let err = cook(json!({"nodes": {"a": {"type": "null", "inputs": {"geo": "b"}}, "b": {"type": "null", "inputs": {"geo": "a"}}},
                          "display": "a"})).unwrap_err();
    assert!(err.0.contains("cycle"), "{err}");
}

#[test]
fn noise_is_deterministic_and_seeded() {
    let pos: Vec<f32> = (0..300).map(|i| i as f32 * 0.137).collect();
    let a = minihou_core::noise::fbm3(&pos, 3, 4, 2.0, 0.5);
    assert_eq!(a, minihou_core::noise::fbm3(&pos, 3, 4, 2.0, 0.5));
    assert_ne!(a, minihou_core::noise::fbm3(&pos, 4, 4, 2.0, 0.5));
    assert!(a.iter().all(|x| x.abs() <= 1.5));
}

#[test]
fn every_node_declares_defaults() {
    for n in nodes::registry() {
        for p in n.params {
            let _ = (p.default)();
        }
    }
    assert!(nodes::get("grid").is_some() && nodes::get("wrangle").is_none());
}
