//! Batched quadric edge decimation, ported from minihou/nodes/reduce.py.
use super::{GEO, Inputs, NodeSpec, model::*};
use crate::{
    error::{Error, Result},
    geometry::{Attr, AttrData, Geometry},
    params::Params,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
type V = [f64; 3];
type Q = [f64; 10];
// numpy.einsum's contiguous three-component dot reduces its even SIMD lanes
// before its odd lane: (x + z) + y. Norms use add.reduce instead (x + y) + z.
// Preserve the distinction: a last-bit cost tie can choose a different edge.
fn einsum(a: V, b: V) -> f64 {
    (a[0] * b[0] + a[2] * b[2]) + a[1] * b[1]
}
pub const POLYREDUCE: NodeSpec = NodeSpec {
    name: "polyreduce",
    inputs: GEO,
    params: crate::params!("percentage":25.;"target":0;"keep_borders":true),
    cook: cook,
};
fn quadric(n: V, d: f64, w: f64) -> Q {
    let [a, b, c] = n;
    [
        a * a,
        a * b,
        a * c,
        a * d,
        b * b,
        b * c,
        b * d,
        c * c,
        c * d,
        d * d,
    ]
    .map(|x| x * w)
}
fn error(q: Q, v: V) -> f64 {
    let [x, y, z] = v;
    q[0] * x * x
        + 2. * q[1] * x * y
        + 2. * q[2] * x * z
        + 2. * q[3] * x
        + q[4] * y * y
        + 2. * q[5] * y * z
        + 2. * q[6] * y
        + q[7] * z * z
        + 2. * q[8] * z
        + q[9]
}
fn sum(a: Q, b: Q) -> Q {
    std::array::from_fn(|k| a[k] + b[k])
}
/// Partial-pivot elimination follows LAPACK's 3x3 solve; singular cases use
/// the first minimum of [end A, end B, midpoint], exactly as Python.
fn solve(q: Q, a: V, b: V) -> V {
    let mut m = [[q[0], q[1], q[2]], [q[1], q[4], q[5]], [q[2], q[5], q[7]]];
    let mut rhs = [-q[3], -q[6], -q[8]];
    let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    let scale = m
        .iter()
        .flatten()
        .map(|v| v.abs())
        .fold(0., f64::max)
        .powi(3)
        .max(1e-300);
    if det.abs() > 1e-9 * scale {
        for k in 0..3 {
            let mut best = k;
            for i in k + 1..3 {
                if m[i][k].abs() > m[best][k].abs() {
                    best = i;
                }
            }
            m.swap(k, best);
            rhs.swap(k, best);
            for i in k + 1..3 {
                let s = m[i][k] / m[k][k];
                for j in k + 1..3 {
                    m[i][j] -= s * m[k][j];
                }
                rhs[i] -= s * rhs[k];
            }
        }
        let mut x = [0.; 3];
        for k in (0..3).rev() {
            x[k] = (rhs[k] - (k + 1..3).map(|j| m[k][j] * x[j]).sum::<f64>()) / m[k][k];
        }
        x
    } else {
        let c = [a, b, mul(add(a, b), 0.5)];
        let mut best = 0;
        for k in 1..3 {
            if error(q, c[k]) < error(q, c[best]) {
                best = k;
            }
        }
        c[best]
    }
}
fn edges(t: &[[usize; 3]]) -> (Vec<[usize; 2]>, Vec<[usize; 3]>, Vec<usize>) {
    let mut set = BTreeSet::new();
    for tri in t {
        for k in 0..3 {
            let (a, b) = (tri[k], tri[(k + 1) % 3]);
            set.insert([a.min(b), a.max(b)]);
        }
    }
    let e: Vec<_> = set.into_iter().collect();
    let map: BTreeMap<_, _> = e.iter().enumerate().map(|(i, &e)| (e, i)).collect();
    let mut count = vec![0; e.len()];
    let side = t
        .iter()
        .map(|tri| {
            std::array::from_fn(|k| {
                let (a, b) = (tri[k], tri[(k + 1) % 3]);
                let i = map[&[a.min(b), a.max(b)]];
                count[i] += 1;
                i
            })
        })
        .collect();
    (e, side, count)
}
fn cook(i: &Inputs, p: &Params) -> Result<Geometry> {
    let g = POLYREDUCE.one(i, 0)?;
    reduce(&g, p.f("percentage")?, p.i("target")?, p.b("keep_borders")?)
}
pub(super) fn reduce(
    g: &Geometry,
    percentage: f64,
    target: i64,
    keep_borders: bool,
) -> Result<Geometry> {
    polygons(g, "polyreduce")?;
    if !percentage.is_finite() || !(0. ..=100.).contains(&percentage) || target < 0 {
        return Err(Error::new(
            "percentage must be between 0 and 100 and target must be nonnegative",
        ));
    }
    if g.p().iter().any(|x| !x.is_finite()) {
        return Err(Error::new("polyreduce requires finite point positions"));
    }
    let vp = &g.topology.vertex_point;
    let mut t = vec![];
    let mut corner_src = vec![];
    let mut tri_prim = vec![];
    for prim in 0..g.nprims() {
        let r = range(g, prim);
        for k in 1..r.len().saturating_sub(1) {
            let v = [r.start, r.start + k, r.start + k + 1];
            t.push(v.map(|v| vp[v] as usize));
            corner_src.push(v);
            tri_prim.push(prim);
        }
    }
    let want = if target > 0 {
        target as usize
    } else {
        (t.len() as f64 * percentage / 100.).ceil() as usize
    };
    let mut pos = positions(g);
    let n = g.npoints;
    let mut live: Vec<_> = (0..t.len()).collect();
    let mut quadrics = vec![[0.; 10]; n];
    for tri in &t {
        let normal = cross(sub(pos[tri[1]], pos[tri[0]]), sub(pos[tri[2]], pos[tri[0]]));
        let area2 = len(normal);
        let normal = normal.map(|x| x / area2.max(1e-300));
        let q = quadric(normal, -einsum(normal, pos[tri[0]]), area2 / 2.);
        for &v in tri {
            quadrics[v] = sum(quadrics[v], q);
        }
    }
    if keep_borders {
        let (_, sides, count) = edges(&t);
        let mut border_planes = Vec::new();
        for (tri, side) in t.iter().zip(sides) {
            for k in 0..3 {
                if count[side[k]] != 1 {
                    continue;
                }
                let (a, b) = (tri[k], tri[(k + 1) % 3]);
                let normal = cross(sub(pos[tri[1]], pos[tri[0]]), sub(pos[tri[2]], pos[tri[0]]));
                let e = sub(pos[b], pos[a]);
                let normal = cross(e, normal);
                let length = len(normal).max(1e-300);
                let normal = normal.map(|x| x / length);
                let q = quadric(
                    normal,
                    -einsum(normal, pos[a]),
                    einsum(e, e).max(1e-30) * 1e3,
                );
                border_planes.push((a, b, q));
            }
        }
        // numpy.bincount receives all start points, then all end points; add
        // the accumulated border quadrics to Q only after summing that list.
        let mut border_q = vec![[0.; 10]; n];
        for end in 0..2 {
            for &(a, b, q) in &border_planes {
                let i = if end == 0 { a } else { b };
                border_q[i] = sum(border_q[i], q);
            }
        }
        for i in 0..n {
            quadrics[i] = sum(quadrics[i], border_q[i]);
        }
    }
    let mut floats: BTreeMap<String, (usize, Vec<f64>)> = g
        .point
        .iter()
        .filter_map(|(name, a)| {
            if name != "P" {
                if let AttrData::F32(v) = &a.data {
                    Some((
                        name.clone(),
                        (a.width, v.iter().map(|&v| v as f64).collect()),
                    ))
                } else {
                    None
                }
            } else {
                None
            }
        })
        .collect();
    let mut blocked = BTreeSet::new();
    let mut progressed = false;
    let mut widen = 1usize;
    for _ in 0..2000 {
        if live.len() <= want {
            break;
        }
        let tl: Vec<_> = live.iter().map(|&i| t[i]).collect();
        let (e, _, count) = edges(&tl);
        if e.is_empty() {
            break;
        }
        let mut border = vec![false; n];
        let mut neighbours = vec![BTreeSet::new(); n];
        for (i, &[a, b]) in e.iter().enumerate() {
            if count[i] == 1 {
                border[a] = true;
                border[b] = true;
            }
            neighbours[a].insert(b);
            neighbours[b].insert(a);
        }
        let qe: Vec<_> = e
            .iter()
            .map(|&[a, b]| sum(quadrics[a], quadrics[b]))
            .collect();
        let v: Vec<_> = e
            .iter()
            .enumerate()
            .map(|(i, &[a, b])| solve(qe[i], pos[a], pos[b]))
            .collect();
        let cost: Vec<_> = e
            .iter()
            .enumerate()
            .map(|(i, &ends)| {
                if blocked.contains(&ends) {
                    f64::INFINITY
                } else {
                    error(qe[i], v[i]).max(0.)
                }
            })
            .collect();
        let tie = |i: usize| {
            let [a, b] = e[i];
            ((a as u64 * 0x9E3779B1 + b as u64 * 0x85EBCA77) & 0xFFFFFFFF) ^ (a as u64 >> 3)
        };
        let mut order: Vec<_> = (0..e.len()).collect();
        order.sort_by(|&a, &b| {
            let rounded = |x: f64| (x * 1e12).round_ties_even() / 1e12;
            rounded(cost[a])
                .total_cmp(&rounded(cost[b]))
                .then(tie(a).cmp(&tie(b)))
                .then(a.cmp(&b))
        });
        let mut rank = vec![0; e.len()];
        let mut best = vec![usize::MAX; n];
        for (r, &i) in order.iter().enumerate() {
            rank[i] = r;
            for &v in &e[i] {
                best[v] = best[v].min(r);
            }
        }
        let need = ((live.len() - want + 1) / 2).max(1);
        let mut finite: Vec<_> = cost.iter().copied().filter(|v| v.is_finite()).collect();
        finite.sort_by(f64::total_cmp);
        let k = (2 * need).saturating_mul(widen).min(finite.len());
        let threshold = if k > 0 { finite[k - 1] } else { -1. };
        let mut cand: Vec<_> = (0..e.len())
            .filter(|&i| {
                best[e[i][0]] == rank[i]
                    && best[e[i][1]] == rank[i]
                    && cost[i] <= threshold * (1. + 1e-9)
            })
            .collect();
        if cand.is_empty() {
            if progressed && !blocked.is_empty() {
                blocked.clear();
                progressed = false;
                continue;
            }
            if k < finite.len() {
                widen = widen.saturating_mul(4);
                continue;
            }
            break;
        }
        cand.sort_by(|&a, &b| cost[a].total_cmp(&cost[b]));
        cand.truncate(need);
        cand.retain(|&i| {
            let [a, b] = e[i];
            let common = neighbours[a].intersection(&neighbours[b]).count();
            let bd = count[i] == 1;
            let ok = common == if bd { 1 } else { 2 } && (bd || !(border[a] && border[b]));
            if !ok {
                blocked.insert(e[i]);
            }
            ok
        });
        if cand.is_empty() {
            continue;
        }
        let old: Vec<_> = tl
            .iter()
            .map(|tri| cross(sub(pos[tri[1]], pos[tri[0]]), sub(pos[tri[2]], pos[tri[0]])))
            .collect();
        while !cand.is_empty() {
            let mut newp = pos.clone();
            let mut to: Vec<_> = (0..n).collect();
            let mut owner = vec![None; n];
            for (k, &i) in cand.iter().enumerate() {
                let [a, b] = e[i];
                newp[a] = v[i];
                newp[b] = v[i];
                to[b] = a;
                owner[a] = Some(k);
                owner[b] = Some(k);
            }
            let mut reject = vec![false; cand.len()];
            for (tri, &old) in tl.iter().zip(&old) {
                let tn = tri.map(|v| to[v]);
                if tn[0] == tn[1] || tn[1] == tn[2] || tn[0] == tn[2] {
                    continue;
                }
                let nn = cross(sub(newp[tn[1]], newp[tn[0]]), sub(newp[tn[2]], newp[tn[0]]));
                if einsum(old, nn) <= 0.2 * len(old) * len(nn) {
                    for &v in tri {
                        if let Some(o) = owner[v] {
                            reject[o] = true;
                        }
                    }
                }
            }
            if !reject.iter().any(|&r| r) {
                break;
            }
            for (k, &i) in cand.iter().enumerate() {
                if reject[k] {
                    blocked.insert(e[i]);
                }
            }
            cand = cand
                .into_iter()
                .enumerate()
                .filter(|(k, _)| !reject[*k])
                .map(|(_, i)| i)
                .collect();
        }
        if cand.is_empty() {
            continue;
        }
        let mut to: Vec<_> = (0..n).collect();
        for i in cand {
            let [a, b] = e[i];
            let edge = sub(pos[b], pos[a]);
            let f =
                (einsum(sub(v[i], pos[a]), edge) / einsum(edge, edge).max(1e-300)).clamp(0., 1.);
            for (w, values) in floats.values_mut() {
                for k in 0..*w {
                    values[a * *w + k] = values[a * *w + k] * (1. - f) + values[b * *w + k] * f;
                }
            }
            pos[a] = v[i];
            quadrics[a] = sum(quadrics[a], quadrics[b]);
            to[b] = a;
        }
        for &i in &live {
            t[i] = t[i].map(|v| to[v]);
        }
        live.retain(|&i| t[i][0] != t[i][1] && t[i][1] != t[i][2] && t[i][0] != t[i][2]);
        progressed = true;
        widen = 1;
    }
    let used: BTreeSet<_> = live.iter().flat_map(|&i| t[i]).collect();
    let used: Vec<_> = used.into_iter().collect();
    let mut index = vec![0; n];
    for (i, &v) in used.iter().enumerate() {
        index[v] = i as i32;
    }
    let mut point = gather(&g.point, &used);
    point.insert(
        "P".into(),
        Arc::new(Attr::f32(
            3,
            used.iter()
                .flat_map(|&i| pos[i].map(|x| x as f32))
                .collect(),
        )),
    );
    for (name, (w, values)) in floats {
        let mut data = vec![];
        for &i in &used {
            let row = &values[i * w..(i + 1) * w];
            let norm = if name == "N" {
                row.iter().map(|v| v * v).sum::<f64>().sqrt().max(1e-30)
            } else {
                1.
            };
            data.extend(row.iter().map(|v| (v / norm) as f32));
        }
        point.insert(name, Arc::new(Attr::f32(w, data)));
    }
    let corners: Vec<_> = live.iter().flat_map(|&i| corner_src[i]).collect();
    let prims: Vec<_> = live.iter().map(|&i| tri_prim[i]).collect();
    make(
        point,
        live.iter().flat_map(|&i| t[i].map(|v| index[v])).collect(),
        &vec![3; live.len()],
        gather(&g.vertex, &corners),
        gather(&g.prim, &prims),
        g.detail.clone(),
        None,
    )
}
