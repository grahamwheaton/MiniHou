//! Delaunay triangulation of points in 2-D, matching nodes/delaunay.py.
use std::sync::Arc;
use super::{GEO, InputSpec, Inputs, NodeSpec};
use crate::{error::{Error, Result}, geometry::Geometry, params::Params};

pub const DELAUNAY: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "delaunay",
    inputs: GEO,
    params: crate::params!("axis": "z"; "max_edge": 0.0),
    cook: delaunay,
};

fn orient(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

fn in_circle(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> bool {
    let (ax, ay) = (a[0] - d[0], a[1] - d[1]);
    let (bx, by) = (b[0] - d[0], b[1] - d[1]);
    let (cx, cy) = (c[0] - d[0], c[1] - d[1]);
    ((ax * ax + ay * ay) * (bx * cy - cx * by)
        - (bx * bx + by * by) * (ax * cy - cx * ay)
        + (cx * cx + cy * cy) * (ax * by - bx * ay))
        > 0.0
}

fn space_filling_order(pts: &[[f64; 2]]) -> Vec<usize> {
    let n = pts.len();
    if n == 0 {
        return Vec::new();
    }
    let mut lo = [pts[0][0], pts[0][1]];
    let mut hi = [pts[0][0], pts[0][1]];
    for p in pts.iter().skip(1) {
        lo[0] = lo[0].min(p[0]);
        lo[1] = lo[1].min(p[1]);
        hi[0] = hi[0].max(p[0]);
        hi[1] = hi[1].max(p[1]);
    }
    let ext_x = (hi[0] - lo[0]).max(1e-300);
    let ext_y = (hi[1] - lo[1]).max(1e-300);
    let strips = ((n as f64 / 2.0).sqrt() as usize).max(1);
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&i, &j| {
        let s_i = (((pts[i][1] - lo[1]) / ext_y * strips as f64).floor() as usize).min(strips - 1);
        let s_j = (((pts[j][1] - lo[1]) / ext_y * strips as f64).floor() as usize).min(strips - 1);
        if s_i != s_j {
            return s_i.cmp(&s_j);
        }
        let x_i = (pts[i][0] - lo[0]) / ext_x;
        let x_j = (pts[j][0] - lo[0]) / ext_x;
        let key_i = if s_i % 2 == 0 { x_i } else { -x_i };
        let key_j = if s_j % 2 == 0 { x_j } else { -x_j };
        key_i.total_cmp(&key_j).then(i.cmp(&j))
    });
    order
}

fn bowyer_watson(pts: &[[f64; 2]]) -> Vec<[usize; 3]> {
    let n = pts.len();
    if n < 3 {
        return Vec::new();
    }
    let mut lo = [pts[0][0], pts[0][1]];
    let mut hi = [pts[0][0], pts[0][1]];
    for p in pts.iter().skip(1) {
        lo[0] = lo[0].min(p[0]);
        lo[1] = lo[1].min(p[1]);
        hi[0] = hi[0].max(p[0]);
        hi[1] = hi[1].max(p[1]);
    }
    let ext = (hi[0] - lo[0]).max(hi[1] - lo[1]).max(1.0);
    let big = 1e5 * ext;

    let mut points: Vec<[f64; 2]> = pts.to_vec();
    points.push([-big, -big]);
    points.push([big, -big]);
    points.push([0.0, big]);

    let mut tris: Vec<[usize; 3]> = vec![[n, n + 1, n + 2]];
    let mut neighbors: Vec<[i32; 3]> = vec![[-1, -1, -1]];
    let mut alive: Vec<bool> = vec![true];
    let mut last = 0usize;

    for pi in space_filling_order(pts) {
        let p = points[pi];
        let mut t = last;
        let mut steps = 0;
        loop {
            steps += 1;
            if steps > 4 * tris.len() + 16 {
                if let Some(k) = (0..tris.len()).find(|&k| {
                    alive[k] && in_circle(points[tris[k][0]], points[tris[k][1]], points[tris[k][2]], p)
                }) {
                    t = k;
                }
                break;
            }
            let mut moved = false;
            for i in 0..3 {
                let a = points[tris[t][i]];
                let b = points[tris[t][(i + 1) % 3]];
                if orient(a, b, p) < 0.0 && neighbors[t][i] >= 0 {
                    t = neighbors[t][i] as usize;
                    moved = true;
                    break;
                }
            }
            if !moved {
                break;
            }
        }

        let mut cavity = std::collections::HashSet::new();
        cavity.insert(t);
        let mut stack = vec![t];
        while let Some(c) = stack.pop() {
            for &nb in &neighbors[c] {
                if nb >= 0 {
                    let unb = nb as usize;
                    if !cavity.contains(&unb)
                        && in_circle(points[tris[unb][0]], points[tris[unb][1]], points[tris[unb][2]], p)
                    {
                        cavity.insert(unb);
                        stack.push(unb);
                    }
                }
            }
        }

        let mut boundary = Vec::new();
        for &c in &cavity {
            for i in 0..3 {
                let nb = neighbors[c][i];
                if nb < 0 || !cavity.contains(&(nb as usize)) {
                    boundary.push((tris[c][i], tris[c][(i + 1) % 3], nb, c));
                }
            }
        }

        for &c in &cavity {
            alive[c] = false;
        }

        let mut by_first = std::collections::HashMap::new();
        let mut by_second = std::collections::HashMap::new();
        let mut new_ids = Vec::with_capacity(boundary.len());

        for (a, b, outer, dead) in boundary {
            let k = tris.len();
            tris.push([a, b, pi]);
            neighbors.push([outer, -1, -1]);
            alive.push(true);
            by_first.insert(a, k);
            by_second.insert(b, k);
            new_ids.push(k);
            if outer >= 0 {
                let o = &mut neighbors[outer as usize];
                if let Some(slot) = o.iter_mut().find(|slot| **slot == dead as i32) {
                    *slot = k as i32;
                }
            }
        }

        for &k in &new_ids {
            let a = tris[k][0];
            let b = tris[k][1];
            neighbors[k][1] = by_first.get(&b).map_or(-1, |&v| v as i32);
            neighbors[k][2] = by_second.get(&a).map_or(-1, |&v| v as i32);
        }

        if let Some(&first_new) = new_ids.first() {
            last = first_new;
        }
    }

    let mut keep = Vec::new();
    for k in 0..tris.len() {
        if alive[k] && tris[k].iter().all(|&v| v < n) {
            keep.push(tris[k]);
        }
    }
    keep
}

pub(crate) fn triangulate(xy: &[[f64; 2]]) -> Vec<[usize; 3]> {
    if xy.len() < 3 {
        return Vec::new();
    }
    let mut indexed: Vec<([f64; 2], usize)> = xy.iter().copied().enumerate().map(|(i, p)| (p, i)).collect();
    indexed.sort_by(|a, b| {
        a.0[0].total_cmp(&b.0[0]).then(a.0[1].total_cmp(&b.0[1])).then(a.1.cmp(&b.1))
    });
    let mut uniq = Vec::new();
    let mut first = Vec::new();
    for item in indexed {
        if let Some(&last_p) = uniq.last() {
            if item.0 == last_p {
                continue;
            }
        }
        uniq.push(item.0);
        first.push(item.1);
    }
    if uniq.len() < 3 {
        return Vec::new();
    }

    let mean_x = uniq.iter().map(|p| p[0]).sum::<f64>() / uniq.len() as f64;
    let mean_y = uniq.iter().map(|p| p[1]).sum::<f64>() / uniq.len() as f64;
    let centered: Vec<[f64; 2]> = uniq.iter().map(|p| [p[0] - mean_x, p[1] - mean_y]).collect();

    let mut tri = bowyer_watson(&centered);
    if tri.is_empty() {
        return Vec::new();
    }

    let mut lo = [centered[0][0], centered[0][1]];
    let mut hi = [centered[0][0], centered[0][1]];
    for p in centered.iter().skip(1) {
        lo[0] = lo[0].min(p[0]);
        lo[1] = lo[1].min(p[1]);
        hi[0] = hi[0].max(p[0]);
        hi[1] = hi[1].max(p[1]);
    }
    let scale = (hi[0] - lo[0]).max(hi[1] - lo[1]).max(1e-300);
    let min_area = 1e-12 * scale * scale;

    let mut valid = Vec::new();
    for mut t in tri {
        let a = centered[t[0]];
        let b = centered[t[1]];
        let c = centered[t[2]];
        let mut area2 = orient(a, b, c);
        if area2 < 0.0 {
            t.swap(1, 2);
            area2 = -area2;
        }
        if area2 > min_area {
            valid.push([first[t[0]], first[t[1]], first[t[2]]]);
        }
    }
    valid
}

fn delaunay(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = DELAUNAY.one(inputs, 0)?;
    let axis = p.e("axis", &["z", "y", "x"])?;
    let max_edge = p.f("max_edge")?;
    if max_edge < 0.0 {
        return Err(Error::new("max_edge must be non-negative"));
    }
    let a = match axis.as_str() {
        "x" => 0,
        "y" => 1,
        "z" => 2,
        _ => unreachable!(),
    };
    let u = (a + 1) % 3;
    let v = (a + 2) % 3;

    let n = geo.npoints;
    let p_slice = geo.p();
    let mut xy = Vec::with_capacity(n);
    for i in 0..n {
        xy.push([p_slice[i * 3 + u] as f64, p_slice[i * 3 + v] as f64]);
    }

    let mut tri = triangulate(&xy);
    if max_edge > 0.0 && !tri.is_empty() {
        tri.retain(|t| {
            for i in 0..3 {
                let p0 = &p_slice[t[i] * 3..t[i] * 3 + 3];
                let p1 = &p_slice[t[(i + 1) % 3] * 3..t[(i + 1) % 3] * 3 + 3];
                let d2 = (0..3).map(|k| (p0[k] as f64 - p1[k] as f64).powi(2)).sum::<f64>();
                if d2.sqrt() > max_edge {
                    return false;
                }
            }
            true
        });
    }

    let ntri = tri.len();
    let mut vp = Vec::with_capacity(ntri * 3);
    for t in &tri {
        vp.push(t[0] as i32);
        vp.push(t[1] as i32);
        vp.push(t[2] as i32);
    }
    let offsets: Vec<i32> = (0..=ntri).map(|i| (i * 3) as i32).collect();

    let mut out = Geometry::from_arrays(geo.p().to_vec(), vp, offsets, None)?;
    out.point = geo.point.clone();
    out.detail = geo.detail.clone();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_delaunay_basic_quad() {
        let pts = vec![
            0.0, 0.0, 0.0,
            1.0, 0.0, 0.0,
            1.0, 1.0, 0.0,
            0.0, 1.0, 0.0,
        ];
        let geo = Geometry::points_only(pts).unwrap();
        let p = Params::resolve(DELAUNAY.params, serde_json::json!({"axis": "z"}).as_object()).unwrap();
        let out = delaunay(&[vec![Arc::new(geo)]], &p).unwrap();
        assert_eq!(out.nprims(), 2);
        assert_eq!(out.npoints, 4);
    }

    #[test]
    fn test_delaunay_max_edge() {
        let pts = vec![
            0.0, 0.0, 0.0,
            1.0, 0.0, 0.0,
            1.0, 1.0, 0.0,
            0.0, 1.0, 0.0,
        ];
        let geo = Geometry::points_only(pts).unwrap();
        let p = Params::resolve(DELAUNAY.params, serde_json::json!({"axis": "z", "max_edge": 0.5}).as_object()).unwrap();
        let out = delaunay(&[vec![Arc::new(geo)]], &p).unwrap();
        assert_eq!(out.nprims(), 0);
    }

    #[test]
    fn test_delaunay_degenerate() {
        let pts = vec![0.0, 0.0, 0.0, 1.0, 1.0, 0.0];
        let geo = Geometry::points_only(pts).unwrap();
        let p = Params::resolve(DELAUNAY.params, None).unwrap();
        let out = delaunay(&[vec![Arc::new(geo)]], &p).unwrap();
        assert_eq!(out.nprims(), 0);
        assert_eq!(out.npoints, 2);
    }
}
