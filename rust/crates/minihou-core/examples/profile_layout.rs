//! `cargo run --release --example profile_layout -- 2000`: time the viewport layout steps on a grid of N x N quads.
use std::time::Instant;

use minihou_core::{preview, topo, Graph};

fn main() {
    let side: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(1000);
    let g = Graph::from_json(&format!(r#"{{"nodes": {{"g": {{"type": "grid", "params": {{"columns": {c}, "rows": {c}}}}}}}, "display": "g"}}"#, c = side + 1))
        .unwrap().cook(None).unwrap();
    let (p, t) = (g.p(), &g.topology);
    let time = |label: &str, f: &mut dyn FnMut()| {
        f();
        let s = Instant::now();
        for _ in 0..3 {
            f();
        }
        println!("{label:>14}: {:7.1} ms", s.elapsed().as_secs_f64() * 1000.0 / 3.0);
    };
    time("fans", &mut || { std::hint::black_box(preview::fan_triangles(&t.vertex_point, &t.prim_offsets, None)); });
    time("edges", &mut || { std::hint::black_box(topo::edges(t, g.npoints)); });
    let (tri, prim) = preview::fan_triangles(&t.vertex_point, &t.prim_offsets, None);
    time("morton sort", &mut || { let (mut a, mut b) = (tri.clone(), prim.clone()); std::hint::black_box(preview::chunk_sort(p, &mut a, &mut b, 8192)); });
    time("layout", &mut || { std::hint::black_box(preview::layout(p, &t.vertex_point, &t.prim_offsets, None, g.npoints, usize::MAX, usize::MAX, 100_000, 8192)); });
}
