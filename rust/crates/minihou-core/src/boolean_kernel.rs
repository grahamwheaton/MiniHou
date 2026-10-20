//! Symbolic surface intersections, planar cut reconstruction and welding.
//! Port of `minihou/boolean.py`; point and polygon order is deliberately stable.
use crate::error::{Error, Result};
use std::collections::{BTreeMap, BTreeSet, HashMap};
pub(crate) type V = [f64; 3];
pub(crate) type V2 = [f64; 2];
pub(crate) type Key = [usize; 4];
pub(crate) fn add(a: V, b: V) -> V {
    std::array::from_fn(|i| a[i] + b[i])
}
pub(crate) fn sub(a: V, b: V) -> V {
    std::array::from_fn(|i| a[i] - b[i])
}
pub(crate) fn mul(a: V, s: f64) -> V {
    a.map(|v| v * s)
}
pub(crate) fn dot(a: V, b: V) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
pub(crate) fn cross(a: V, b: V) -> V {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
pub(crate) fn len(a: V) -> f64 {
    dot(a, a).sqrt()
}
pub(crate) fn unit(a: V) -> V {
    mul(a, 1. / len(a).max(1e-300))
}
pub(crate) fn basis(n: V) -> (V, V) {
    let u = unit(cross(
        n,
        if n[0].abs() < 0.9 {
            [1., 0., 0.]
        } else {
            [0., 1., 0.]
        },
    ));
    (u, cross(n, u))
}
pub(crate) fn cross2(a: V2, b: V2, c: V2) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}
pub(crate) fn area2(poly: &[usize], xy: &[V2]) -> f64 {
    (0..poly.len())
        .map(|k| {
            let a = xy[poly[(k + poly.len() - 1) % poly.len()]];
            let b = xy[poly[k]];
            a[0] * b[1] - b[0] * a[1]
        })
        .sum()
}

mod polygons;
pub(crate) use polygons::{earclip, holes_into};
mod cuts;
pub(crate) use cuts::{inner_point, split_triangle};

#[derive(Clone)]
pub(crate) struct Piece {
    pub v: Vec<usize>,
    pub bary: Vec<V>,
    pub owner: usize,
    pub src: usize,
    pub whole: bool,
}
pub(crate) struct Output {
    pub p: Vec<V>,
    pub pieces: Vec<Piece>,
    pub keys: Vec<Key>,
    pub edge_t: Vec<f64>,
    pub remap: Vec<usize>,
}
pub(crate) struct Dsu(Vec<usize>);
impl Dsu {
    pub fn new(n: usize) -> Self {
        Self((0..n).collect())
    }
    pub fn root(&mut self, i: usize) -> usize {
        if self.0[i] != i {
            self.0[i] = self.root(self.0[i]);
        }
        self.0[i]
    }
    pub fn join(&mut self, a: usize, b: usize) {
        let a = self.root(a);
        let b = self.root(b);
        self.0[a.max(b)] = a.min(b);
    }
}
mod intersections;
pub(crate) use intersections::{candidates, distances, inside, interval, orient3d, planes};

pub(crate) fn boolean(
    pa: &[V],
    ta: &[[usize; 3]],
    pb: &[V],
    tb: &[[usize; 3]],
    op: &str,
    weld: f64,
) -> Result<Output> {
    let na = pa.len();
    let nb = pb.len();
    let planes_a = planes(pa, ta);
    let planes_b = planes(pb, tb);
    let mut segments = vec![];
    let mut keys = BTreeSet::new();
    for (i, j) in candidates(pa, ta, pb, tb) {
        let da = distances(pa, ta[i], pb, tb[j], planes_b[j]);
        let db = distances(pb, tb[j], pa, ta[i], planes_a[i]);
        if da.iter().all(|&d| d > 0.)
            || da.iter().all(|&d| d < 0.)
            || db.iter().all(|&d| d > 0.)
            || db.iter().all(|&d| d < 0.)
        {
            continue;
        }
        if da.contains(&0.) || db.contains(&0.) {
            return Err(Error::new(
                "a point lies exactly on the other surface; move one input very slightly",
            ));
        }
        let dir = cross(planes_a[i].0, planes_b[j].0);
        let (a0, a1, ak0, ak1) = interval(pa, ta[i], da, dir, 0, j);
        let (b0, b1, bk0, bk1) = interval(pb, tb[j], db, dir, 1, i);
        if a0.max(b0) < a1.min(b1) {
            let k1 = if a0 >= b0 { ak0 } else { bk0 };
            let k2 = if a1 <= b1 { ak1 } else { bk1 };
            keys.insert(k1);
            keys.insert(k2);
            segments.push((i, j, k1, k2));
        }
    }
    let keys: Vec<_> = keys.into_iter().collect();
    let key_ids: BTreeMap<_, _> = keys.iter().enumerate().map(|(i, &k)| (k, i)).collect();
    let mut p = pa.to_vec();
    p.extend(pb);
    let mut edge_t = vec![];
    for &[side, u, v, f] in &keys {
        let (points, plane) = if side == 0 {
            (pa, planes_b[f])
        } else {
            (pb, planes_a[f])
        };
        let du = dot(points[u], plane.0) + plane.1;
        let dv = dot(points[v], plane.0) + plane.1;
        let t = du / (du - dv);
        p.push(add(points[u], mul(sub(points[v], points[u]), t)));
        edge_t.push(t);
    }
    let mut pieces = vec![];
    let mut cuts = BTreeSet::new();
    for (side, t, base, off) in [(0, ta, 0, 0), (1, tb, na, ta.len())] {
        let mut cut = vec![vec![]; t.len()];
        for (s, &(a, b, _, _)) in segments.iter().enumerate() {
            cut[if side == 0 { a } else { b }].push(s);
        }
        for (i, tri) in t.iter().enumerate() {
            if cut[i].is_empty() {
                pieces.push(Piece {
                    v: tri.iter().map(|i| i + base).collect(),
                    bary: vec![[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
                    owner: side,
                    src: i + off,
                    whole: true,
                });
            }
        }
        for (i, tri) in t.iter().enumerate() {
            if cut[i].is_empty() {
                continue;
            }
            let c = tri.map(|i| p[i + base]);
            let e1 = sub(c[1], c[0]);
            let e2 = sub(c[2], c[0]);
            let (g00, g01, g11) = (dot(e1, e1), dot(e1, e2), dot(e2, e2));
            let det = g00 * g11 - g01 * g01;
            if det <= 1e-24 * g00.max(g01).max(g11).max(1e-300).powi(2) {
                return Err(Error::new(
                    "an input has a zero-area face where the surfaces cross; clean it first",
                ));
            }
            let mut globals: Vec<_> = tri.iter().map(|i| i + base).collect();
            let mut ids: BTreeMap<_, _> =
                globals.iter().enumerate().map(|(i, &g)| (g, i)).collect();
            let mut xy = vec![[0., 0.], [1., 0.], [0., 1.]];
            let mut edge_pts = [vec![], vec![], vec![]];
            let mut segs = vec![];
            for &s in &cut[i] {
                let (_, _, k1, k2) = segments[s];
                let mut pair = [0; 2];
                let ki = [key_ids[&k1], key_ids[&k2]];
                for k in 0..2 {
                    let kk = ki[k];
                    let g = na + nb + kk;
                    let id = if let Some(&id) = ids.get(&g) {
                        id
                    } else {
                        let id = xy.len();
                        let q = sub(p[g], c[0]);
                        let (x, y) = (dot(q, e1), dot(q, e2));
                        xy.push([(g11 * x - g01 * y) / det, (g00 * y - g01 * x) / det]);
                        globals.push(g);
                        ids.insert(g, id);
                        let key = keys[kk];
                        if key[0] == side {
                            for e in 0..3 {
                                if tri[e].min(tri[(e + 1) % 3]) == key[1]
                                    && tri[e].max(tri[(e + 1) % 3]) == key[2]
                                {
                                    edge_pts[e].push(id);
                                }
                            }
                        }
                        id
                    };
                    pair[k] = id;
                }
                segs.push(pair);
                cuts.insert((na + nb + ki[0].min(ki[1]), na + nb + ki[0].max(ki[1])));
            }
            let mut boundary = vec![];
            for e in 0..3 {
                boundary.push(e);
                let a = xy[e];
                let b = xy[(e + 1) % 3];
                let along = |i: usize| {
                    (xy[i][0] - a[0]) * (b[0] - a[0]) + (xy[i][1] - a[1]) * (b[1] - a[1])
                };
                edge_pts[e].sort_by(|&i, &j| along(i).total_cmp(&along(j)));
                boundary.extend(&edge_pts[e]);
            }
            for poly in split_triangle(&xy, boundary, &segs)? {
                pieces.push(Piece {
                    v: poly.iter().map(|&i| globals[i]).collect(),
                    bary: poly
                        .iter()
                        .map(|&i| [1. - xy[i][0] - xy[i][1], xy[i][0], xy[i][1]])
                        .collect(),
                    owner: side,
                    src: i + off,
                    whole: false,
                });
            }
        }
    }
    let mut dsu = Dsu::new(pieces.len());
    let mut edges: BTreeMap<(usize, usize, usize), usize> = BTreeMap::new();
    for (i, q) in pieces.iter().enumerate() {
        for k in 0..q.v.len() {
            let a = q.v[k];
            let b = q.v[(k + 1) % q.v.len()];
            let e = (a.min(b), a.max(b));
            if !cuts.contains(&e) {
                if let Some(&j) = edges.get(&(q.owner, e.0, e.1)) {
                    dsu.join(i, j);
                }
                edges.insert((q.owner, e.0, e.1), i);
            }
        }
    }
    let mut reps = BTreeMap::new();
    let mut roots = vec![];
    for (i, q) in pieces.iter().enumerate() {
        let r = dsu.root(i);
        roots.push(r);
        let area = len((0..q.v.len())
            .map(|k| cross(p[q.v[k]], p[q.v[(k + 1) % q.v.len()]]))
            .fold([0.; 3], add));
        let e = reps.entry(r).or_insert((-1., i));
        if area > e.0 {
            *e = (area, i);
        }
    }
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for x in &p {
        for k in 0..3 {
            lo[k] = lo[k].min(x[k]);
            hi[k] = hi[k].max(x[k]);
        }
    }
    let scale = (0..3).map(|k| hi[k] - lo[k]).fold(0., f64::max);
    let mut is_in = BTreeMap::new();
    for (&r, &(_, i)) in &reps {
        let q = &pieces[i];
        let pts: Vec<_> = q.v.iter().map(|&i| p[i]).collect();
        let sample = inner_point(&pts);
        is_in.insert(
            r,
            if q.owner == 0 {
                inside(sample, pb, tb, scale)
            } else {
                inside(sample, pa, ta, scale)
            },
        );
    }
    pieces = pieces
        .into_iter()
        .zip(roots)
        .filter_map(|(mut q, r)| {
            let inside = is_in[&r];
            let keep = match op {
                "intersect" => inside,
                "subtract" if q.owner == 1 => inside,
                _ => !inside,
            };
            if !keep {
                return None;
            }
            if op == "subtract" && q.owner == 1 {
                q.v.reverse();
                q.bary.reverse();
            }
            Some(q)
        })
        .collect();
    let mut remap: Vec<_> = (0..p.len()).collect();
    if weld > 0. && !pieces.is_empty() {
        weld_pieces(&p, &mut pieces, &mut remap, weld, na + nb);
    }
    Ok(Output {
        p,
        pieces,
        keys,
        edge_t,
        remap,
    })
}
fn clean(pieces: Vec<Piece>) -> Vec<Piece> {
    let mut loops = vec![];
    for mut q in pieces {
        let keep: Vec<_> = (0..q.v.len())
            .filter(|&i| q.v[i] != q.v[(i + 1) % q.v.len()])
            .collect();
        q.whole &= keep.len() == q.v.len();
        q.v = keep.iter().map(|&i| q.v[i]).collect();
        q.bary = keep.iter().map(|&i| q.bary[i]).collect();
        let pinched = q.v.iter().copied().collect::<BTreeSet<_>>().len() != q.v.len();
        let mut stack = vec![(0..q.v.len()).collect::<Vec<_>>()];
        while let Some(cur) = stack.pop() {
            let mut seen = BTreeMap::new();
            let mut split = false;
            for (k, &ix) in cur.iter().enumerate() {
                if let Some(&i0) = seen.get(&q.v[ix]) {
                    stack.push(cur[i0..k].to_vec());
                    let mut other = cur[..i0].to_vec();
                    other.extend(&cur[k..]);
                    stack.push(other);
                    split = true;
                    break;
                }
                seen.insert(q.v[ix], k);
            }
            if !split && cur.len() >= 3 {
                let mut r = q.clone();
                r.v = cur.iter().map(|&i| q.v[i]).collect();
                r.bary = cur.iter().map(|&i| q.bary[i]).collect();
                r.whole &= !pinched;
                loops.push(r);
            }
        }
    }
    let sig = |q: &Piece| {
        let mut s = q.v.clone();
        s.sort_unstable();
        s
    };
    let mut counts = BTreeMap::new();
    for q in &loops {
        *counts.entry(sig(q)).or_insert(0usize) += 1;
    }
    loops.into_iter().filter(|q| counts[&sig(q)] == 1).collect()
}
fn weld_pieces(p: &[V], pieces: &mut Vec<Piece>, remap: &mut [usize], tol: f64, first_cut: usize) {
    let used: BTreeSet<_> = pieces.iter().flat_map(|q| q.v.iter().copied()).collect();
    let mut dsu = Dsu::new(p.len());
    let mut grid: HashMap<[i64; 3], Vec<usize>> = HashMap::new();
    for &i in &used {
        let cell = p[i].map(|x| (x / tol).floor() as i64);
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    if let Some(ids) = grid.get(&[cell[0] + dx, cell[1] + dy, cell[2] + dz]) {
                        for &j in ids {
                            if dot(sub(p[i], p[j]), sub(p[i], p[j])) <= tol * tol {
                                dsu.join(i, j);
                            }
                        }
                    }
                }
            }
        }
        grid.entry(cell).or_default().push(i);
    }
    for &i in &used {
        remap[i] = dsu.root(i);
    }
    for _ in 0..16 {
        for q in pieces.iter_mut() {
            for v in &mut q.v {
                *v = remap[*v];
            }
        }
        *pieces = clean(std::mem::take(pieces));
        let mut tips = BTreeMap::new();
        for q in pieces.iter() {
            let n = q.v.len();
            let mut area = [0.; 3];
            let mut longest = 0f64;
            for k in 0..n {
                area = add(area, cross(p[q.v[k]], p[q.v[(k + 1) % n]]));
                longest = longest.max(len(sub(p[q.v[(k + 1) % n]], p[q.v[k]])));
            }
            if len(area) <= 1e-6 * longest * longest {
                continue;
            }
            for k in 0..n {
                let (a, v, b) = (q.v[(k + n - 1) % n], q.v[k], q.v[(k + 1) % n]);
                let e1 = sub(p[v], p[a]);
                let e2 = sub(p[b], p[v]);
                let (l1, l2) = (len(e1), len(e2));
                if v >= first_cut
                    && dot(e1, e2) < 0.
                    && len(cross(e1, e2)) / (l1 * l2).max(1e-300) < 1e-4
                {
                    tips.entry(v).or_insert(if l1 <= l2 { a } else { b });
                }
            }
        }
        if tips.is_empty() {
            break;
        }
        let mut step: Vec<_> = (0..p.len()).collect();
        for (v, to) in tips {
            step[v] = to;
        }
        for _ in 0..8 {
            step = step.iter().map(|&i| step[i]).collect();
        }
        for r in remap.iter_mut() {
            *r = step[*r];
        }
    }
}
