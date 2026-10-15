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
fn bound_wraps_its_input_in_a_box() {
    let g = cook(json!({"nodes": {"b": {"type": "box"}, "x": {"type": "bound", "inputs": {"geo": "b"}}}, "display": "x"})).unwrap();
    assert_eq!((g.npoints, g.nprims()), (8, 6));
    assert_eq!(g.detail["bound_min"], json!([-0.5, -0.5, -0.5]));
    assert_eq!(g.detail["bound_max"], json!([0.5, 0.5, 0.5]));
    assert_eq!(g.detail["bound_size"], json!([1.0, 1.0, 1.0]));
    let pad = cook(json!({"nodes": {"b": {"type": "box"}, "x": {"type": "bound", "inputs": {"geo": "b"},
        "params": {"padding": [0.1, 0.2, 0.3]}}}, "display": "x"})).unwrap();
    let size: Vec<f64> = pad.detail["bound_size"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
    for (got, want) in size.iter().zip([1.2, 1.4, 1.6]) {
        assert!((got - want).abs() < 1e-12, "{got} != {want}");
    }
    // an empty input is a plain unit box with no detail attributes
    let empty = cook(json!({"nodes": {"g": {"type": "grid", "flags": {"bypass": true}},
        "x": {"type": "bound", "inputs": {"geo": "g"}}}, "display": "x"})).unwrap();
    assert_eq!((empty.npoints, empty.nprims()), (1, 6));
    assert!(empty.detail.is_empty());
    // oriented is refused loudly
    let err = cook(json!({"nodes": {"b": {"type": "box"}, "x": {"type": "bound", "inputs": {"geo": "b"},
        "params": {"oriented": true}}}, "display": "x"})).unwrap_err();
    assert!(err.0.contains("not supported by the Rust backend"), "{err}");
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
fn additional_proportional_falloffs_cook_and_random_is_refused() {
    let graph = |falloff| json!({"nodes": {
        "a": {"type": "line", "params": {"points": 3, "length": 2}},
        "t": {"type": "transform", "inputs": {"geo": "a"}, "params": {"group": "0",
            "proportional_enabled": true, "proportional_radius": 3,
            "proportional_falloff": falloff, "translate": [1,0,0]}}
    }, "display": "t"});
    let inverse = cook(graph("inverse_square")).unwrap();
    assert!(inverse.p()[3] > 0.0 && inverse.p()[3] < 1.0);
    let err = cook(graph("random")).unwrap_err();
    assert!(err.0.contains("not supported by the Rust backend"));
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
    assert!(nodes::get("grid").is_some() && nodes::get("delete").is_some() && nodes::get("wrangle").is_none());
}

#[test]
fn delete_points_and_dependent_primitives() {
    let graph = |expr: &str, invert: bool| json!({
        "nodes": {
            "g": {"type": "grid", "params": {"columns": 4, "rows": 4}},
            "d": {"type": "delete", "inputs": {"geo": "g"}, "params": {"expr": expr, "invert": invert}},
        },
        "display": "d"
    });
    let d = cook(graph("@P.x > 0", false)).unwrap();
    assert_eq!((d.npoints, d.nprims(), d.nvertices()), (8, 3, 12));
    for q in d.p().as_chunks::<3>().0 {
        assert!(q[0] <= 0.0);
    }

    let kept = cook(graph("@P.x > 0", true)).unwrap();
    assert_eq!((kept.npoints, kept.nprims(), kept.nvertices()), (8, 3, 12));
    for q in kept.p().as_chunks::<3>().0 {
        assert!(q[0] > 0.0);
    }

    let none = cook(graph("@P.x > 1000", false)).unwrap();
    assert_eq!((none.npoints, none.nprims(), none.nvertices()), (16, 9, 36));

    let empty = cook(graph("", false)).unwrap();
    assert_eq!((empty.npoints, empty.nprims(), empty.nvertices()), (16, 9, 36));

    let all = cook(graph("1", false)).unwrap();
    assert_eq!((all.npoints, all.nprims(), all.nvertices()), (0, 0, 0));

    let curve_graph = |expr: &str| json!({
        "nodes": {
            "c": {"type": "curve", "params": {"points": "0,0,0 1,0,0 2,0,0 3,0,0 4,0,0", "closed": false}},
            "d": {"type": "delete", "inputs": {"geo": "c"}, "params": {"expr": expr}},
        },
        "display": "d"
    });
    let split = cook(curve_graph("@P.x == 2")).unwrap();
    assert_eq!((split.npoints, split.nprims(), split.nvertices()), (4, 2, 4));

    let err = cook(graph("rand(@ptnum, 1) < 0.5", false)).unwrap_err();
    assert!(err.0.contains("not supported by the Rust backend"));
}

fn extents(g: &minihou_core::Geometry) -> ([f32; 3], [f32; 3]) {
    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    for q in g.p().as_chunks::<3>().0 {
        for k in 0..3 {
            lo[k] = lo[k].min(q[k]);
            hi[k] = hi[k].max(q[k]);
        }
    }
    (lo, hi)
}

fn close(a: [f32; 3], b: [f32; 3]) -> bool {
    (0..3).all(|k| (a[k] - b[k]).abs() < 1e-6)
}

#[test]
fn match_size_places_and_scales() {
    // a unit box scaled (non-uniformly) to size [2, 1, 4] with its centre at [1, 2, 3]
    let g = cook(json!({"nodes": {
        "b": {"type": "box"},
        "m": {"type": "match_size", "inputs": {"geo": "b"},
              "params": {"translate": true, "center": [1.0, 2.0, 3.0], "scale": true,
                         "size": [2.0, 1.0, 4.0], "uniform": false}}}, "display": "m"})).unwrap();
    let (lo, hi) = extents(&g);
    assert!(close(lo, [0.0, 1.5, 1.0]) && close(hi, [2.0, 2.5, 5.0]), "{lo:?} {hi:?}");

    // uniform keeps proportions: the largest fitting scale of [2, 2, 4] on a unit box is 2
    let u = cook(json!({"nodes": {
        "b": {"type": "box"},
        "m": {"type": "match_size", "inputs": {"geo": "b"},
              "params": {"center": [0.0, 0.0, 0.0], "scale": true, "size": [2.0, 2.0, 4.0], "uniform": true}}}, "display": "m"})).unwrap();
    let (lo, hi) = extents(&u);
    assert!(close(lo, [-1.0, -1.0, -1.0]) && close(hi, [1.0, 1.0, 1.0]), "{lo:?} {hi:?}");

    // justify_z = min sits the box bottom on the target z
    let j = cook(json!({"nodes": {
        "b": {"type": "box"},
        "m": {"type": "match_size", "inputs": {"geo": "b"},
              "params": {"center": [0.0, 0.0, 5.0], "justify_z": "min"}}}, "display": "m"})).unwrap();
    let (lo, hi) = extents(&j);
    assert!((lo[2] - 5.0).abs() < 1e-6 && (hi[2] - 6.0).abs() < 1e-6, "{lo:?} {hi:?}");
}

#[test]
fn match_size_uses_the_second_input_as_the_target() {
    // the target curve spans [0, 0, 0] to [2, 4, 6]: size [2, 4, 6], centre [1, 2, 3]
    let g = cook(json!({"nodes": {
        "b": {"type": "box"},
        "c": {"type": "curve", "params": {"points": "0,0,0 2,4,6"}},
        "m": {"type": "match_size", "inputs": {"geo": "b", "target": "c"},
              "params": {"scale": true, "size": [1.0, 1.0, 1.0], "uniform": false}}}, "display": "m"})).unwrap();
    let (lo, hi) = extents(&g);
    assert!(close(lo, [0.0, 0.0, 0.0]) && close(hi, [2.0, 4.0, 6.0]), "{lo:?} {hi:?}");
}

