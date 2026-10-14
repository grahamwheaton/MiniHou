//! `cargo run --release --example profile_normals -- 2000`: time point normals on a grid of N x N quads.
use std::time::Instant;

use minihou_core::{normals, Graph};

fn main() {
    let side: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(1000);
    let g = Graph::from_json(&format!(r#"{{"nodes": {{"g": {{"type": "grid", "params": {{"columns": {c}, "rows": {c}}}}}}}, "display": "g"}}"#, c = side + 1))
        .unwrap().cook(None).unwrap();
    normals::point_normals(g.p(), &g.topology, g.npoints);
    let s = Instant::now();
    std::hint::black_box(normals::point_normals(g.p(), &g.topology, g.npoints));
    println!("point normals {:.1} ms", s.elapsed().as_secs_f64() * 1000.0);
}
