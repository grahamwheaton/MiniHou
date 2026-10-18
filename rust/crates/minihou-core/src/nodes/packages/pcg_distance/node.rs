use crate::nodes::pcg::*;
use crate::nodes::{InputSpec, Inputs, NodeSpec};
use crate::{error::{Error, Result}, geometry::{Attr, AttrData, Geometry}, params::Params};
use std::collections::HashMap;
use std::sync::Arc;

pub(super) const NODE: NodeSpec = NodeSpec {
    name: "pcg_distance",
    inputs: &[
        InputSpec {
            name: "points",
            multi: false,
            optional: false,
        },
        InputSpec {
            name: "target",
            multi: false,
            optional: false,
        },
    ],
    params: crate::params!(
        "max_distance": 5.0;
        "mode": "set";
        "invert": false
    ),
    cook: pcg_distance,
};

fn closest_point_on_tri(p: [f64; 3], a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> [f64; 3] {
    let ab = sub(b, a);
    let ac = sub(c, a);
    let ap = sub(p, a);
    let d1 = dot(ab, ap);
    let d2 = dot(ac, ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }

    let bp = sub(p, b);
    let d3 = dot(ab, bp);
    let d4 = dot(ac, bp);
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }

    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return [a[0] + ab[0] * v, a[1] + ab[1] * v, a[2] + ab[2] * v];
    }

    let cp = sub(p, c);
    let d5 = dot(ab, cp);
    let d6 = dot(ac, cp);
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }

    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return [a[0] + ac[0] * w, a[1] + ac[1] * w, a[2] + ac[2] * w];
    }

    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        let bc = sub(c, b);
        return [b[0] + bc[0] * w, b[1] + bc[1] * w, b[2] + bc[2] * w];
    }

    let denom = 1.0 / (va + vb + vc);
    let v = vb * denom;
    let w = vc * denom;
    [
        a[0] + ab[0] * v + ac[0] * w,
        a[1] + ab[1] * v + ac[1] * w,
        a[2] + ab[2] * v + ac[2] * w,
    ]
}

fn distance_to_target(points_p: &[[f64; 3]], target: &Geometry) -> Vec<f64> {
    if is_mesh(target) {
        let tris = fan_triangles(target);
        return points_p
            .iter()
            .map(|&p| {
                let mut best = f64::INFINITY;
                for tri in &tris {
                    let cp = closest_point_on_tri(p, tri[0], tri[1], tri[2]);
                    let d = norm(sub(cp, p));
                    if d < best {
                        best = d;
                    }
                }
                best
            })
            .collect();
    }
    if target.nprims() > 0 {
        let edges = crate::topo::edges(&target.topology, target.npoints);
        let pt = target.p();
        let mut best = vec![f64::INFINITY; points_p.len()];
        for pair in edges.points.chunks_exact(2) {
            let (i0, i1) = (pair[0] as usize, pair[1] as usize);
            let a = [
                pt[i0 * 3] as f64,
                pt[i0 * 3 + 1] as f64,
                pt[i0 * 3 + 2] as f64,
            ];
            let b = [
                pt[i1 * 3] as f64,
                pt[i1 * 3 + 1] as f64,
                pt[i1 * 3 + 2] as f64,
            ];
            let ab = sub(b, a);
            let l2 = dot(ab, ab).max(1e-300);
            for (p_idx, &p) in points_p.iter().enumerate() {
                let pa = sub(p, a);
                let t = (dot(pa, ab) / l2).clamp(0.0, 1.0);
                let q = [a[0] + t * ab[0], a[1] + t * ab[1], a[2] + t * ab[2]];
                let d = norm(sub(p, q));
                if d < best[p_idx] {
                    best[p_idx] = d;
                }
            }
        }
        return best;
    }
    let pt = target.p();
    points_p
        .iter()
        .map(|&p| {
            let mut best = f64::INFINITY;
            for c in pt.chunks_exact(3) {
                let d = norm(sub(p, [c[0] as f64, c[1] as f64, c[2] as f64]));
                if d < best {
                    best = d;
                }
            }
            best
        })
        .collect()
}

fn pcg_distance(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let points = NODE.one(inputs, 0)?;
    let target = NODE.one(inputs, 1)?;
    let max_dist = p.f("max_distance")?;
    let mode = p.e("mode", &["set", "multiply"])?;
    let invert = p.b("invert")?;

    let a = read_pcg(&points);
    let dist = if target.npoints == 0 {
        vec![f64::INFINITY; points.npoints]
    } else {
        distance_to_target(&a.p, &target)
    };

    let mut d_out = Vec::with_capacity(points.npoints);
    for i in 0..points.npoints {
        let mut v = (dist[i] / max_dist).clamp(0.0, 1.0);
        if invert {
            v = 1.0 - v;
        }
        let d = if mode == "set" { v } else { a.density[i] * v };
        d_out.push(d as f32);
    }

    let mut attrs = HashMap::new();
    attrs.insert("density".into(), NewAttrVal::F32(1, d_out));
    cloud(&points, None, Some(attrs))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
