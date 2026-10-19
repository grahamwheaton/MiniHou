//! Topology-preserving miter offsets, matching minihou/nodes/planar_offset.py.
use super::model::*;
use crate::{
    error::{Error, Result},
    geometry::Geometry,
    topo,
};
use std::collections::{BTreeMap, BTreeSet};
fn cross2(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}
fn sub2(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] - b[0], a[1] - b[1]]
}
fn dot2(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}
fn len2(a: [f64; 2]) -> f64 {
    dot2(a, a).sqrt()
}
fn validate(loops: &[Vec<[f64; 2]>], tol: f64) -> Result<()> {
    let segments: Vec<_> = loops
        .iter()
        .flat_map(|q| (0..q.len()).map(|i| (q[i], q[(i + 1) % q.len()])))
        .collect();
    for (i, &(a, b)) in segments.iter().enumerate() {
        let ab = sub2(b, a);
        if len2(ab) <= tol {
            return Err(Error::new(
                "Inset offset collapses a boundary edge; reduce the amount",
            ));
        }
        for &(c, d) in &segments[i + 1..] {
            if [a, b]
                .iter()
                .any(|&x| [c, d].iter().any(|&y| len2(sub2(x, y)) <= tol))
            {
                continue;
            }
            let cd = sub2(d, c);
            let den = cross2(ab, cd);
            if den.abs() <= tol * tol {
                if cross2(sub2(c, a), ab).abs() <= tol * len2(ab) {
                    let t = dot2(sub2(c, a), ab) / dot2(ab, ab);
                    let u = dot2(sub2(d, a), ab) / dot2(ab, ab);
                    if t.max(u).min(1.) - t.min(u).max(0.) > tol / len2(ab) {
                        return Err(Error::new(
                            "Inset boundary overlaps itself; reduce the amount",
                        ));
                    }
                }
                continue;
            }
            let t = cross2(sub2(c, a), cd) / den;
            let u = cross2(sub2(c, a), ab) / den;
            if t >= -tol / len2(ab)
                && t <= 1. + tol / len2(ab)
                && u >= -tol / len2(cd)
                && u <= 1. + tol / len2(cd)
            {
                return Err(Error::new(
                    "Inset boundary self-intersects; reduce the amount or split the region",
                ));
            }
        }
    }
    Ok(())
}
pub(super) fn offset_selected(
    g: &Geometry,
    sel: &[bool],
    amount: f64,
    individual: bool,
    even: bool,
) -> Result<Vec<[f64; 3]>> {
    let p = positions(g);
    let vp = &g.topology.vertex_point;
    let (pv, nx, _) = corners(g);
    let e = topo::edges(&g.topology, g.npoints);
    let mut result: Vec<_> = vp.iter().map(|&q| p[q as usize]).collect();
    let mut adj = vec![BTreeSet::new(); g.nprims()];
    if !individual {
        let mut owners = vec![Vec::new(); e.len()];
        for v in 0..g.nvertices() {
            if sel[pv[v]] {
                owners[e.of_vertex[v] as usize].push(pv[v]);
            }
        }
        for owners in owners {
            if owners.len() > 2 {
                return Err(Error::new("Inset requires a manifold selected region"));
            }
            if owners.len() == 2 {
                adj[owners[0]].insert(owners[1]);
                adj[owners[1]].insert(owners[0]);
            }
        }
    }
    let mut pending: BTreeSet<_> = (0..g.nprims()).filter(|&i| sel[i]).collect();
    while let Some(&seed) = pending.first() {
        pending.remove(&seed);
        let mut region = BTreeSet::from([seed]);
        let mut todo = vec![seed];
        while let Some(f) = todo.pop() {
            for &nb in &adj[f] {
                if pending.remove(&nb) {
                    region.insert(nb);
                    todo.push(nb);
                }
            }
        }
        let cs: Vec<_> = region.iter().flat_map(|&i| range(g, i)).collect();
        let qs: Vec<_> = range(g, seed).map(|v| p[vp[v] as usize]).collect();
        if qs.len() < 3 {
            return Err(Error::new("Inset requires non-degenerate planar faces"));
        }
        let mut lo = [f64::INFINITY; 3];
        let mut hi = [f64::NEG_INFINITY; 3];
        for &v in &cs {
            for k in 0..3 {
                lo[k] = lo[k].min(p[vp[v] as usize][k]);
                hi[k] = hi[k].max(p[vp[v] as usize][k]);
            }
        }
        let extent = (0..3).map(|k| hi[k] - lo[k]).fold(1e-12, f64::max);
        let tol = (extent * 1e-7).max(1e-12);
        let mut n = [0.; 3];
        for i in 0..qs.len() {
            n = add(
                n,
                cross(sub(qs[i], qs[0]), sub(qs[(i + 1) % qs.len()], qs[0])),
            );
        }
        if len(n) <= tol * tol {
            return Err(Error::new("Inset requires non-degenerate planar faces"));
        }
        n = unit(n);
        if cs
            .iter()
            .any(|&v| dot(sub(p[vp[v] as usize], qs[0]), n).abs() > tol)
        {
            return Err(Error::new(
                "Inset connected regions must be planar; use individual faces or split the selection",
            ));
        }
        let x = mul(sub(qs[1], qs[0]), 1. / len(sub(qs[1], qs[0])).max(1e-30));
        if len(x) < 0.5 {
            return Err(Error::new("Inset boundary contains a zero-length edge"));
        }
        let y = cross(n, x);
        let project = |q: [f64; 3]| {
            let q = sub(q, qs[0]);
            [dot(q, x), dot(q, y)]
        };
        let mut cnt = vec![0; e.len()];
        for &v in &cs {
            cnt[e.of_vertex[v] as usize] += 1;
        }
        let boundary: Vec<_> = cs
            .iter()
            .copied()
            .filter(|&v| cnt[e.of_vertex[v] as usize] == 1)
            .collect();
        if boundary.is_empty() {
            return Err(Error::new("Inset requires an open planar region boundary"));
        }
        let mut succ = BTreeMap::new();
        for v in boundary {
            if succ.insert(vp[v], vp[nx[v]]).is_some() {
                return Err(Error::new(
                    "Inset boundary branches at a vertex; split the region",
                ));
            }
        }
        let mut remaining: BTreeSet<_> = succ.keys().copied().collect();
        let mut loops = Vec::new();
        while let Some(&start) = remaining.first() {
            let mut at = start;
            let mut q = Vec::new();
            while remaining.remove(&at) {
                q.push(at);
                at = *succ.get(&at).ok_or_else(|| {
                    Error::new("Inset requires closed, consistently wound boundary loops")
                })?;
            }
            if at != start || q.len() < 3 {
                return Err(Error::new(
                    "Inset requires closed, consistently wound boundary loops",
                ));
            }
            loops.push(q);
        }
        let old: Vec<Vec<_>> = loops
            .iter()
            .map(|q| q.iter().map(|&i| project(p[i as usize])).collect())
            .collect();
        validate(&old, tol)?;
        let mut new = Vec::new();
        let mut moved = BTreeMap::new();
        for (ids, xy) in loops.iter().zip(&old) {
            let n = xy.len();
            let vectors: Vec<_> = (0..n).map(|i| sub2(xy[(i + 1) % n], xy[i])).collect();
            let mut dirs = Vec::new();
            for v in vectors {
                let l = len2(v);
                if l <= tol {
                    return Err(Error::new("Inset boundary contains a zero-length edge"));
                }
                dirs.push([v[0] / l, v[1] / l]);
            }
            let inward: Vec<_> = dirs.iter().map(|v| [-v[1], v[0]]).collect();
            let mut shifted = Vec::new();
            for i in 0..n {
                let a = inward[(i + n - 1) % n];
                let b = inward[i];
                let den = 1. + dot2(a, b);
                if den < 1e-10 {
                    return Err(Error::new("Inset boundary has an unstable reversing miter"));
                }
                let d = if even { [amount * (a[0] + b[0]) / den, amount * (a[1] + b[1]) / den] }
                    else {let v=[a[0]+b[0],a[1]+b[1]];let l=len2(v).max(1e-12);[amount*v[0]/l,amount*v[1]/l]};
                if len2(d) > (amount.abs() * 1000.).max(tol) {
                    return Err(Error::new(
                        "Inset boundary miter is too sharp; bevel the corner first",
                    ));
                }
                shifted.push([xy[i][0] + d[0], xy[i][1] + d[1]]);
            }
            if (0..n).any(|i| dot2(sub2(shifted[(i + 1) % n], shifted[i]), dirs[i]) <= tol) {
                return Err(Error::new(
                    "Inset offset collapses or reverses an edge; reduce the amount",
                ));
            }
            for (&id, q) in ids.iter().zip(&shifted) {
                moved.insert(id, add(qs[0], add(mul(x, q[0]), mul(y, q[1]))));
            }
            new.push(shifted);
        }
        validate(&new, tol)?;
        for &v in &cs {
            if let Some(&q) = moved.get(&vp[v]) {
                result[v] = q;
            }
        }
        for &f in &region {
            let q: Vec<_> = range(g, f).map(|v| project(result[v])).collect();
            validate(std::slice::from_ref(&q), tol)?;
            if (0..q.len())
                .map(|i| cross2(q[i], q[(i + 1) % q.len()]))
                .sum::<f64>()
                <= tol * tol
            {
                return Err(Error::new(
                    "Inset offset folds an interior face; reduce amount or dissolve interior edges",
                ));
            }
        }
    }
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn perpendicular_offset_and_collapse() {
        let g = Geometry::from_arrays(
            vec![0., 0., 0., 2., 0., 0., 2., 2., 0., 0., 2., 0.],
            vec![0, 1, 2, 3],
            vec![0, 4],
            None,
        )
        .unwrap();
        let q = offset_selected(&g, &[true], 0.25, false, true).unwrap();
        assert_eq!(
            q,
            vec![
                [0.25, 0.25, 0.],
                [1.75, 0.25, 0.],
                [1.75, 1.75, 0.],
                [0.25, 1.75, 0.]
            ]
        );
        assert!(offset_selected(&g, &[true], 1., false, true).is_err());
    }
}
