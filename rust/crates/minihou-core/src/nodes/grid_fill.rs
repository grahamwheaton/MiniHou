//! `grid_fill` (Blender Grid Fill, `mesh.fill_grid`): fills closed boundary loops with a quad grid. Spec: docs/RUST_MESH_NODES.md.
use std::collections::{BTreeMap, BTreeSet};

use super::mesh_common::{append, out_group, refresh_corner_normals, NewPoint};
use super::{edge_tools, model, Inputs, NodeSpec, GEO};
use crate::error::{Error, Result};
use crate::geometry::Geometry;
use crate::params::Params;
use crate::topo;

pub const GRID_FILL: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, name: "grid_fill", inputs: GEO, params: crate::params!(
    "group": ""; "span": 1; "auto_span": true; "offset": 0; "use_interp_simple": false;
    "output_group": true; "output_group_name": "grid_fill"), cook: grid_fill };

struct Loop {
    pts: Vec<usize>,
    /// The corner of the existing face that runs along each loop edge (used for attribute sources).
    corners: Vec<usize>,
}

/// Selected boundary edges as directed loops in the direction the new faces must use (opposite to the existing face).
fn loops(g: &Geometry, pattern: &str) -> Result<Vec<Loop>> {
    let e = topo::edges(&g.topology, g.npoints);
    let sel = if pattern.trim().is_empty() { (0..e.len()).map(|i| e.count[i] == 1).collect() } else { edge_tools::edge_mask(g, pattern)? };
    // new-face direction: from the face's second point to its first
    let mut next: BTreeMap<usize, (usize, usize)> = BTreeMap::new();
    let mut indeg: BTreeSet<usize> = BTreeSet::new();
    for p in 0..g.nprims() {
        let r = model::range(g, p);
        for v in r.clone() {
            let w = if v + 1 == r.end { r.start } else { v + 1 };
            let id = e.of_vertex[v];
            if id < 0 || !sel[id as usize] { continue; }
            let (a, b) = (g.topology.vertex_point[v] as usize, g.topology.vertex_point[w] as usize);
            if e.count[id as usize] != 1 {
                return Err(Error::new(format!("grid_fill: the edge between points {a} and {b} is not a boundary edge (it is shared by {} faces)", e.count[id as usize])));
            }
            if next.insert(b, (a, v)).is_some() || !indeg.insert(a) {
                return Err(Error::new(format!("grid_fill: the selected boundary branches or touches itself near point {a}")));
            }
        }
    }
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for &start in next.keys() {
        if seen.contains(&start) { continue; }
        let (mut pts, mut corners) = (vec![], vec![]);
        let mut cur = start;
        loop {
            seen.insert(cur);
            let Some(&(to, corner)) = next.get(&cur) else {
                return Err(Error::new(format!("grid_fill: the selected edges form an open chain ending at point {cur}, not a closed loop")));
            };
            pts.push(cur);
            corners.push(corner);
            cur = to;
            if cur == start { break; }
            if seen.contains(&cur) { return Err(Error::new("grid_fill: the selected boundary branches or touches itself")); }
        }
        if pts.len() % 2 != 0 {
            return Err(Error::new(format!("grid_fill: the boundary loop through point {start} has {} edges; Grid Fill needs an even number of edges", pts.len())));
        }
        if pts.len() < 4 {
            return Err(Error::new(format!("grid_fill: the boundary loop through point {start} has only {} edges; at least 4 are needed", pts.len())));
        }
        let k = (0..pts.len()).min_by_key(|&i| pts[i]).unwrap();
        pts.rotate_left(k);
        corners.rotate_left(k);
        out.push(Loop { pts, corners });
    }
    Ok(out)
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] { [a[0] - b[0], a[1] - b[1], a[2] - b[2]] }
fn dist(a: [f64; 3], b: [f64; 3]) -> f64 { let d = sub(a, b); (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() }
fn angle(a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> f64 {
    let (u, v) = (sub(a, b), sub(c, b));
    let (lu, lv) = (dist(a, b), dist(c, b));
    if lu < 1e-12 || lv < 1e-12 { return std::f64::consts::FRAC_PI_2; }
    ((u[0] * v[0] + u[1] * v[1] + u[2] * v[2]) / (lu * lv)).clamp(-1., 1.).acos()
}

/// Picks (span, offset) so the four grid corners fall on the sharpest loop corners and the grid is nearly square.
fn auto_layout(pos: &[[f64; 3]]) -> (usize, usize) {
    let n = pos.len();
    let ang: Vec<f64> = (0..n).map(|i| angle(pos[(i + n - 1) % n], pos[i], pos[(i + 1) % n])).collect();
    let arc = |from: usize, len: usize| -> f64 { (0..len).map(|k| dist(pos[(from + k) % n], pos[(from + k + 1) % n])).sum::<f64>().max(1e-12) };
    let mut best = (f64::INFINITY, 1, 0);
    for s in 1..n / 2 {
        let m = n / 2 - s;
        for o in 0..n {
            let corners = [o, (o + s) % n, (o + s + m) % n, (o + 2 * s + m) % n];
            let mut score = 0.;
            for (i, a) in ang.iter().enumerate() {
                score += (a - if corners.contains(&i) { std::f64::consts::FRAC_PI_2 } else { std::f64::consts::PI }).abs();
            }
            score += 0.5 * ((arc(o, s) / arc((o + s) % n, m)).ln().abs() + (arc((o + s + m) % n, s) / arc((o + 2 * s + m) % n, m)).ln().abs());
            if score < best.0 - 1e-9 { best = (score, s, o); }
        }
    }
    (best.1, best.2)
}

/// Normalised cumulative parameter along a polyline (arc length, or the index when `simple`).
fn params_along(c: &[[f64; 3]], simple: bool) -> Vec<f64> {
    let mut a = vec![0.];
    for k in 1..c.len() { a.push(a[k - 1] + if simple { 1. } else { dist(c[k - 1], c[k]) }); }
    let tot = a.last().copied().unwrap().max(1e-12);
    a.iter().map(|x| x / tot).collect()
}

fn grid_fill(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = GRID_FILL.one(inputs, 0)?;
    let span = p.i("span")?;
    if span < 1 { return Err(Error::new("grid_fill: span must be >= 1")); }
    let (auto, simple, offset) = (p.b("auto_span")?, p.b("use_interp_simple")?, p.i("offset")?);
    model::polygons(&g, "grid_fill")?;
    let ls = loops(&g, &p.s("group")?)?;
    let nold = g.nprims();
    if ls.is_empty() {
        let mut out = (*g).clone();
        out_group(&mut out, p, &vec![false; nold])?;
        return Ok(out);
    }
    let pos = model::positions(&g);
    let (mut pts, mut faces, mut srcs): (Vec<NewPoint>, Vec<Vec<usize>>, Vec<Vec<Vec<usize>>>) = (vec![], vec![], vec![]);
    for l in &ls {
        let n = l.pts.len();
        let lp: Vec<[f64; 3]> = l.pts.iter().map(|&i| pos[i]).collect();
        let (s, o) = if auto {
            let (s, o) = auto_layout(&lp);
            (s, (o as i64 + offset).rem_euclid(n as i64) as usize)
        } else {
            if span as usize >= n / 2 {
                return Err(Error::new(format!("grid_fill: span {span} is too large for a loop of {n} edges (maximum {})", n / 2 - 1)));
            }
            (span as usize, offset.rem_euclid(n as i64) as usize)
        };
        let m = n / 2 - s;
        let at = |k: usize| (o + k) % n;
        let bidx = |i: usize, j: usize| -> Option<usize> {
            if j == 0 { Some(at(i)) } else if i == s { Some(at(s + j)) } else if j == m { Some(at(s + m + (s - i))) }
            else if i == 0 { Some(at(2 * s + m + (m - j))) } else { None }
        };
        let b: Vec<[f64; 3]> = (0..=s).map(|i| lp[at(i)]).collect();
        let r: Vec<[f64; 3]> = (0..=m).map(|j| lp[at(s + j)]).collect();
        let t: Vec<[f64; 3]> = (0..=s).map(|i| lp[at(s + m + (s - i))]).collect();
        let lf: Vec<[f64; 3]> = (0..=m).map(|j| lp[at(2 * s + m + (m - j))]).collect();
        let (ub, ut, vr, vl) = (params_along(&b, simple), params_along(&t, simple), params_along(&r, simple), params_along(&lf, simple));
        let cs = [at(0), at(s), at(s + m), at(2 * s + m)];
        let (c00, c10, c11, c01) = (lp[cs[0]], lp[cs[1]], lp[cs[2]], lp[cs[3]]);
        let mut grid = vec![vec![usize::MAX; m + 1]; s + 1];
        let mut gsrc: Vec<Vec<Vec<usize>>> = vec![vec![vec![]; m + 1]; s + 1];
        for i in 0..=s {
            for j in 0..=m {
                if let Some(bi) = bidx(i, j) {
                    grid[i][j] = l.pts[bi];
                    gsrc[i][j] = vec![l.corners[bi]];
                    continue;
                }
                let (u0, v0) = (i as f64 / s as f64, j as f64 / m as f64);
                let (u, v) = if simple { (u0, v0) } else { ((1. - v0) * ub[i] + v0 * ut[i], (1. - u0) * vl[j] + u0 * vr[j]) };
                let mut q = [0.; 3];
                for c in 0..3 {
                    q[c] = (1. - v) * b[i][c] + v * t[i][c] + (1. - u) * lf[j][c] + u * r[j][c]
                        - ((1. - u) * (1. - v) * c00[c] + u * (1. - v) * c10[c] + u * v * c11[c] + (1. - u) * v * c01[c]);
                }
                let w = [(1. - u) * (1. - v), u * (1. - v), u * v, (1. - u) * v];
                pts.push(NewPoint { pos: q, src: cs.iter().zip(w).map(|(&k, w)| (l.pts[k], w)).collect() });
                grid[i][j] = g.npoints + pts.len() - 1;
                gsrc[i][j] = cs.iter().map(|&k| l.corners[k]).collect();
            }
        }
        for i in 0..s {
            for j in 0..m {
                faces.push(vec![grid[i][j], grid[i + 1][j], grid[i + 1][j + 1], grid[i][j + 1]]);
                srcs.push(vec![gsrc[i][j].clone(), gsrc[i + 1][j].clone(), gsrc[i + 1][j + 1].clone(), gsrc[i][j + 1].clone()]);
            }
        }
    }
    let mut out = append(&g, &pts, &faces, &srcs)?;
    refresh_corner_normals(&mut out, nold);
    let mut mask = vec![false; nold];
    mask.extend(std::iter::repeat_n(true, faces.len()));
    out_group(&mut out, p, &mask)?;
    Ok(out)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::Arc;

    /// A closed ring of `n` quads around the z axis (radius 1, height 1), no caps.
    pub(crate) fn ring(n: usize) -> Geometry {
        let mut p = vec![];
        for z in [0., 1.] {
            for i in 0..n {
                let a = i as f32 / n as f32 * std::f32::consts::TAU;
                p.extend([a.cos(), a.sin(), z]);
            }
        }
        let mut vp = vec![];
        for i in 0..n { let j = (i + 1) % n; vp.extend([i, j, n + j, n + i].map(|x| x as i32)); }
        Geometry::from_arrays(p, vp, (0..=n as i32).map(|i| i * 4).collect(), None).unwrap()
    }
    fn run(g: Geometry, kv: serde_json::Value) -> Result<Geometry> {
        let p = Params::resolve(GRID_FILL.params, kv.as_object()).unwrap();
        grid_fill(&[vec![Arc::new(g)]], &p)
    }
    /// (V - E + F, boundary edges, non-manifold edges)
    pub(crate) fn topo_stats(g: &Geometry) -> (i64, usize, usize) {
        let e = topo::edges(&g.topology, g.npoints);
        let used: BTreeSet<_> = g.topology.vertex_point.iter().collect();
        ((used.len() + g.nprims()) as i64 - e.len() as i64, e.count.iter().filter(|&&c| c == 1).count(), e.count.iter().filter(|&&c| c > 2).count())
    }
    /// Every shared edge is traversed in opposite directions by its two faces.
    pub(crate) fn consistent(g: &Geometry) -> bool {
        let mut seen = BTreeSet::new();
        for p in 0..g.nprims() {
            let r = model::range(g, p);
            for v in r.clone() {
                let w = if v + 1 == r.end { r.start } else { v + 1 };
                if !seen.insert((g.topology.vertex_point[v], g.topology.vertex_point[w])) { return false; }
            }
        }
        true
    }

    #[test]
    fn fills_open_tube_end_with_consistent_grid() {
        let g = ring(8);
        let out = run(g.clone(), serde_json::json!({"group": "", "auto_span": false, "span": 2})).unwrap();
        // two loops of 8 edges, each 2x2 grid: 8 new quads, 2 new points
        assert_eq!(out.nprims(), g.nprims() + 8);
        assert_eq!(out.npoints, g.npoints + 2);
        assert_eq!(topo_stats(&out), (2, 0, 0));
        assert!(consistent(&out));
        assert!(out.p().iter().all(|x| x.is_finite()));
        assert_eq!(out.prim["group_grid_fill"].as_i32().unwrap().iter().sum::<i32>(), 8);
        let again = run(g, serde_json::json!({"auto_span": false, "span": 2})).unwrap();
        assert_eq!(out.p(), again.p());
    }
    #[test]
    fn auto_span_gives_square_grid() {
        let out = run(ring(8), serde_json::json!({})).unwrap();
        assert_eq!(out.nprims(), 8 + 8);
    }
    #[test]
    fn normals_point_outward_of_a_closed_cylinder() {
        let out = run(ring(8), serde_json::json!({})).unwrap();
        let n = crate::normals::prim_normals(out.p(), &out.topology);
        // the bottom cap faces down, the top cap faces up
        for (i, f) in n.iter().enumerate().skip(8) {
            let z = out.p()[out.topology.vertex_point[out.topology.prim_offsets[i] as usize] as usize * 3 + 2];
            assert!(f[2] * (if z > 0.5 { 1. } else { -1. }) > 0., "face {i}");
        }
    }
    #[test]
    fn odd_loop_is_rejected_with_a_clear_message() {
        let e = run(ring(7), serde_json::json!({})).unwrap_err().to_string();
        assert!(e.contains("even number"), "{e}");
    }
    #[test]
    fn span_too_large_and_empty_inputs() {
        assert!(run(ring(8), serde_json::json!({"auto_span": false, "span": 4})).is_err());
        let closed_free = Geometry::points_only(vec![]).unwrap();
        assert_eq!(run(closed_free, serde_json::json!({})).unwrap().nprims(), 0);
    }
    #[test]
    fn preserves_cd_groups_and_marks_new_uvs() {
        use crate::geometry::Attr;
        let mut g = ring(8);
        g.vertex.insert("Cd".into(), Arc::new(Attr::f32(3, vec![0.5; 32 * 3])));
        g.vertex.insert("uv".into(), Arc::new(Attr::f32(2, vec![0.25; 32 * 2])));
        g.prim.insert("material_index".into(), Arc::new(Attr::i32(1, vec![0; 8])));
        let out = run(g, serde_json::json!({})).unwrap();
        out.validate().unwrap();
        let uv = out.vertex["uv"].as_f32().unwrap();
        assert_eq!(&uv[..64], &[0.25; 64][..]);
        assert!(uv[64..].iter().all(|x| x.is_nan()));
        assert!(out.vertex["Cd"].as_f32().unwrap().iter().all(|x| x.is_finite()));
        assert_eq!(out.prim["material_index"].as_i32().unwrap().len(), out.nprims());
    }
    #[test]
    fn one_loop_selected_by_edge_pattern_leaves_the_other() {
        let g = ring(8);
        // only the bottom loop: points 0..8
        let sel: Vec<String> = (0..8).map(|i| format!("p{}-{}", i.min((i + 1) % 8), i.max((i + 1) % 8))).collect();
        let out = run(g, serde_json::json!({"group": sel.join(" ")})).unwrap();
        assert_eq!(topo_stats(&out).1, 8);
    }
}
