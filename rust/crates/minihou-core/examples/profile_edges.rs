//! `cargo run --release --example profile_edges -- 2000`: time the edges kernel on a grid of N x N quads.
use std::time::Instant;

use minihou_core::{topo, Graph};

fn main() {
    let side: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(1000);
    let g = Graph::from_json(&format!(r#"{{"nodes": {{"g": {{"type": "grid", "params": {{"columns": {c}, "rows": {c}}}}}}}, "display": "g"}}"#, c = side + 1))
        .unwrap().cook(None).unwrap();
    for _ in 0..2 {
        topo::edges(&g.topology, g.npoints);
    }
    let t = Instant::now();
    let n = 5;
    for _ in 0..n {
        std::hint::black_box(topo::edges(&g.topology, g.npoints));
    }
    println!("{} quads: edges {:.1} ms", side * side, t.elapsed().as_secs_f64() * 1000.0 / n as f64);
}
