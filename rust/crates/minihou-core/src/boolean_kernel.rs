//! Symbolic surface intersections, planar cut reconstruction and welding.
//! Port of `minihou/boolean.py`; point and polygon order is deliberately stable.
use crate::error::{Error, Result};
use std::collections::{BTreeMap, BTreeSet, HashMap};
pub(crate) type V = [f64; 3];
pub(crate) type V2 = [f64; 2];
type Key = [usize; 4];
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
fn inside_poly(p: V2, poly: &[usize], xy: &[V2]) -> bool {
    let mut c = false;
    for k in 0..poly.len() {
        let a = xy[poly[(k + poly.len() - 1) % poly.len()]];
        let b = xy[poly[k]];
        if (a[1] > p[1]) != (b[1] > p[1])
            && a[0] + (p[1] - a[1]) * (b[0] - a[0]) / (b[1] - a[1]) > p[0]
        {
            c = !c;
        }
    }
    c
}
pub(crate) fn earclip(poly: &[usize], xy: &[V2]) -> Vec<[usize; 3]> {
    let mut ix = poly.to_vec();
    let mut out = vec![];
    for _ in 0..4 * poly.len() + 16 {
        if ix.len() <= 3 {
            break;
        }
        let m = ix.len();
        let mut best = None;
        let mut found = 0;
        for k in 0..m {
            let (i0, i1, i2) = (ix[(k + m - 1) % m], ix[k], ix[(k + 1) % m]);
            let (a, b, c) = (xy[i0], xy[i1], xy[i2]);
            let area = cross2(a, b, c);
            if area <= 0. {
                continue;
            }
            if ix.iter().any(|&j| {
                j != i0
                    && j != i1
                    && j != i2
                    && xy[j] != a
                    && xy[j] != b
                    && xy[j] != c
                    && cross2(a, b, xy[j]) >= 0.
                    && cross2(b, c, xy[j]) >= 0.
                    && cross2(c, a, xy[j]) >= 0.
            }) {
                continue;
            }
            let d = |a: V2, b: V2| (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2);
            let q = area / d(a, b).max(d(b, c)).max(d(c, a)).max(1e-300);
            if best.is_none_or(|(s, _): (f64, usize)| q > s) {
                best = Some((q, k));
            }
            found += 1;
            if found >= 8 {
                break;
            }
        }
        let Some((_, k)) = best else {
            break;
        };
        out.push([ix[(k + m - 1) % m], ix[k], ix[(k + 1) % m]]);
        ix.remove(k);
    }
    if ix.len() == 3 {
        out.push([ix[0], ix[1], ix[2]]);
    } else if ix.len() > 3 {
        for k in 1..ix.len() - 1 {
            out.push([ix[0], ix[k], ix[k + 1]]);
        }
    }
    out
}
fn eberly(poly: &[usize], h: usize, xy: &[V2]) -> Option<usize> {
    let [hx, hy] = xy[h];
    let n = poly.len();
    let mut hit = None;
    let mut best = f64::INFINITY;
    for k in 0..n {
        let a = xy[poly[k]];
        let b = xy[poly[(k + 1) % n]];
        if (a[1] > hy) != (b[1] > hy) {
            let x = a[0] + (hy - a[1]) * (b[0] - a[0]) / (b[1] - a[1]);
            if hx <= x && x < hx + best {
                best = x - hx;
                hit = Some((k, x));
            }
        }
    }
    let (edge, x) = hit?;
    let mut pp = if xy[poly[edge]][0] >= xy[poly[(edge + 1) % n]][0] {
        edge
    } else {
        (edge + 1) % n
    };
    let [px, py] = xy[poly[pp]];
    if [px, py] != [x, hy] {
        let mut tri = [[hx, hy], [x, hy], [px, py]];
        if cross2(tri[0], tri[1], tri[2]) < 0. {
            tri.swap(1, 2);
        }
        let mut key = None;
        for j in 0..n {
            let v = xy[poly[j]];
            if cross2(xy[poly[(j + n - 1) % n]], v, xy[poly[(j + 1) % n]]) >= 0. || v == [hx, hy] {
                continue;
            }
            if cross2(tri[0], tri[1], v) > 0.
                && cross2(tri[1], tri[2], v) > 0.
                && cross2(tri[2], tri[0], v) > 0.
            {
                let dx = v[0] - hx;
                let dy = v[1] - hy;
                let k = (dy.abs() / dx.hypot(dy).max(1e-300), dx * dx + dy * dy);
                if key.is_none_or(|old| k < old) {
                    key = Some(k);
                    pp = j;
                }
            }
        }
    }
    Some(pp)
}
fn two_bridges(
    poly: &[usize],
    hole: &[usize],
    xy: &[V2],
    d: V2,
) -> Option<(usize, usize, usize, usize)> {
    let r: Vec<_> = xy
        .iter()
        .map(|p| [p[0] * d[0] + p[1] * d[1], -p[0] * d[1] + p[1] * d[0]])
        .collect();
    let mut hr = 0;
    let mut hl = 0;
    let key = |k: usize| (r[hole[k]][0], -r[hole[k]][1]);
    for k in 1..hole.len() {
        if key(k) > key(hr) {
            hr = k;
        }
        if key(k) < key(hl) {
            hl = k;
        }
    }
    let pr = eberly(poly, hole[hr], &r)?;
    let rev: Vec<_> = poly.iter().rev().copied().collect();
    let r: Vec<_> = r.iter().map(|p| [-p[0], p[1]]).collect();
    let pl = poly.len() - 1 - eberly(&rev, hole[hl], &r)?;
    (pr != pl && hr != hl).then_some((pr, pl, hr, hl))
}
fn crosses(p: V2, q: V2, a: V2, b: V2) -> bool {
    cross2(p, q, a) * cross2(p, q, b) < 0. && cross2(a, b, p) * cross2(a, b, q) < 0.
}
pub(crate) fn holes_into(
    poly: Vec<usize>,
    mut holes: Vec<Vec<usize>>,
    xy: &[V2],
) -> Result<Vec<Vec<usize>>> {
    let maxx = |h: &Vec<usize>| {
        h.iter()
            .map(|&i| xy[i][0])
            .fold(f64::NEG_INFINITY, f64::max)
    };
    holes.sort_by(|a, b| maxx(b).total_cmp(&maxx(a)));
    let mut pieces = vec![poly];
    let directions = [
        [1., 0.],
        [0., 1.],
        [
            std::f64::consts::FRAC_1_SQRT_2,
            std::f64::consts::FRAC_1_SQRT_2,
        ],
        [
            std::f64::consts::FRAC_1_SQRT_2,
            -std::f64::consts::FRAC_1_SQRT_2,
        ],
        [0.9238795325112867, 0.3826834323650898],
        [0.3826834323650898, -0.9238795325112867],
    ];
    for (hi, hole) in holes.iter().enumerate() {
        let host = pieces
            .iter()
            .position(|q| inside_poly(xy[hole[0]], q, xy))
            .ok_or_else(|| Error::new("a hole lies outside its polygon"))?;
        let q = pieces[host].clone();
        let mut done = false;
        for d in directions {
            let Some((pr, pl, hr, hl)) = two_bridges(&q, hole, xy, d) else {
                continue;
            };
            if holes[hi + 1..].iter().any(|h| {
                (0..h.len()).any(|k| {
                    [(hole[hr], q[pr]), (hole[hl], q[pl])]
                        .iter()
                        .any(|&(a, b)| crosses(xy[a], xy[b], xy[h[k]], xy[h[(k + 1) % h.len()]]))
                })
            }) {
                continue;
            }
            let n = q.len();
            let m = hole.len();
            let p1: Vec<_> = (0..=(pl + n - pr) % n)
                .map(|k| q[(pr + k) % n])
                .chain((0..=(hl + m - hr) % m).map(|k| hole[(hl + m - k) % m]))
                .collect();
            let p2: Vec<_> = (0..=(pr + n - pl) % n)
                .map(|k| q[(pl + k) % n])
                .chain((0..=(hr + m - hl) % m).map(|k| hole[(hr + m - k) % m]))
                .collect();
            let (a1, a2, aq, ah) = (
                area2(&p1, xy),
                area2(&p2, xy),
                area2(&q, xy),
                area2(hole, xy),
            );
            if a1 > 0. && a2 > 0. && (a1 + a2 - (aq - ah)).abs() <= 1e-9 * aq.abs() {
                pieces[host] = p1;
                pieces.push(p2);
                done = true;
                break;
            }
        }
        if done {
            continue;
        }
        let rest: Vec<_> = holes[hi..]
            .iter()
            .filter(|h| inside_poly(xy[h[0]], &q, xy))
            .collect();
        let mut slit = q.clone();
        for h in &rest {
            let mut hr = 0;
            for k in 1..h.len() {
                if (xy[h[k]][0], -xy[h[k]][1]) > (xy[h[hr]][0], -xy[h[hr]][1]) {
                    hr = k;
                }
            }
            let pr = eberly(&slit, h[hr], xy)
                .ok_or_else(|| Error::new("could not bridge a hole in a cut triangle"))?;
            let mut s = slit[..pr + 1].to_vec();
            s.extend((0..h.len()).map(|k| h[(hr + h.len() - k) % h.len()]));
            s.extend([h[hr], slit[pr]]);
            s.extend(&slit[pr + 1..]);
            slit = s;
        }
        pieces.splice(
            host..host + 1,
            earclip(&slit, xy).into_iter().map(|t| t.to_vec()),
        );
        for h in &holes[hi + 1..] {
            if rest.contains(&h) {
                continue;
            }
            let k = pieces
                .iter()
                .position(|q| inside_poly(xy[h[0]], q, xy))
                .ok_or_else(|| Error::new("a hole lies outside its polygon"))?;
            let sub = holes_into(pieces[k].clone(), vec![h.clone()], xy)?;
            pieces.splice(k..k + 1, sub);
        }
        return Ok(pieces);
    }
    Ok(pieces)
}
fn split_triangle(xy: &[V2], boundary: Vec<usize>, segs: &[[usize; 2]]) -> Result<Vec<Vec<usize>>> {
    // Vec preserves Python dict insertion order (important for the bridge and ear policies).
    let mut order = vec![];
    let mut adj = vec![vec![]; xy.len()];
    for &[a, b] in segs {
        for (a, b) in [(a, b), (b, a)] {
            if adj[a].is_empty() {
                order.push(a);
            }
            adj[a].push(b);
        }
    }
    let on: BTreeSet<_> = boundary.iter().copied().collect();
    let mut seen = BTreeSet::new();
    let mut chains = vec![];
    let mut loops = vec![];
    for &start in &order {
        if on.contains(&start) && !seen.contains(&start) && adj[start].len() == 1 {
            let mut chain = vec![start];
            let mut prev = usize::MAX;
            let mut cur = start;
            seen.insert(start);
            while let Some(&next) = adj[cur].iter().find(|&&n| n != prev) {
                prev = cur;
                cur = next;
                chain.push(cur);
                if !seen.insert(cur) || on.contains(&cur) {
                    break;
                }
            }
            chains.push(chain);
        }
    }
    for &start in &order {
        if seen.contains(&start) {
            continue;
        }
        let mut lp = vec![start];
        let mut prev = usize::MAX;
        let mut cur = start;
        seen.insert(start);
        while let Some(&next) = adj[cur]
            .iter()
            .find(|&&n| n != prev && (!seen.contains(&n) || n == start))
        {
            prev = cur;
            cur = next;
            if cur == start {
                break;
            }
            lp.push(cur);
            seen.insert(cur);
        }
        if lp.len() >= 3 {
            loops.push(lp);
        }
    }
    let mut polys = vec![boundary];
    for ch in chains {
        let p = ch[0];
        let q = *ch.last().unwrap();
        let Some(k) = polys.iter().position(|v| v.contains(&p) && v.contains(&q)) else {
            return Err(Error::new("could not place a cut inside its triangle"));
        };
        let mut poly = polys[k].clone();
        let r = poly.iter().position(|&i| i == p).unwrap();
        poly.rotate_left(r);
        let s = poly.iter().position(|&i| i == q).unwrap();
        let inner = &ch[1..ch.len() - 1];
        let mut a = poly[..s + 1].to_vec();
        a.extend(inner.iter().rev());
        let mut b = poly[s..].to_vec();
        b.push(p);
        b.extend(inner);
        polys[k] = a;
        polys.push(b);
    }
    loops.sort_by(|a, b| area2(b, xy).abs().total_cmp(&area2(a, xy).abs()));
    let mut holes: BTreeMap<usize, Vec<Vec<usize>>> = BTreeMap::new();
    for mut lp in loops {
        if area2(&lp, xy) < 0. {
            lp.reverse();
        }
        let mut host = None;
        for (k, poly) in polys.iter().enumerate() {
            if inside_poly(xy[lp[0]], poly, xy)
                && host.is_none_or(|h: usize| area2(poly, xy).abs() < area2(&polys[h], xy).abs())
            {
                host = Some(k);
            }
        }
        let h = host.ok_or_else(|| Error::new("a closed cut lies outside its triangle"))?;
        holes.entry(h).or_default().push(lp.clone());
        polys.push(lp);
    }
    let mut out = vec![];
    for (k, p) in polys.into_iter().enumerate() {
        if let Some(h) = holes.remove(&k) {
            out.extend(holes_into(p, h, xy)?);
        } else {
            out.push(p);
        }
    }
    Ok(out)
}
pub(crate) fn inner_point(q: &[V]) -> V {
    let mut n = [0.; 3];
    for k in 0..q.len() {
        n = add(n, cross(q[k], q[(k + 1) % q.len()]));
    }
    let (u, w) = basis(unit(n));
    let xy: Vec<_> = q.iter().map(|&p| [dot(p, u), dot(p, w)]).collect();
    let ts = earclip(&(0..q.len()).collect::<Vec<_>>(), &xy);
    let mut best = None;
    for t in ts {
        let a = cross2(xy[t[0]], xy[t[1]], xy[t[2]]).abs();
        if best.is_none_or(|(s, _): (f64, [usize; 3])| a > s) {
            best = Some((a, t));
        }
    }
    if let Some((_, t)) = best {
        mul(add(add(q[t[0]], q[t[1]]), q[t[2]]), 1. / 3.)
    } else {
        mul(
            q.iter().copied().fold([0.; 3], add),
            1. / q.len().max(1) as f64,
        )
    }
}
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
fn planes(p: &[V], t: &[[usize; 3]]) -> Vec<(V, f64)> {
    t.iter()
        .map(|v| {
            let n = unit(cross(sub(p[v[1]], p[v[0]]), sub(p[v[2]], p[v[0]])));
            (n, -dot(n, p[v[0]]))
        })
        .collect()
}
fn bounds(p: &[V], t: &[[usize; 3]]) -> Vec<(V, V)> {
    t.iter()
        .map(|t| {
            let mut lo = [f64::INFINITY; 3];
            let mut hi = [f64::NEG_INFINITY; 3];
            for &i in t {
                for k in 0..3 {
                    lo[k] = lo[k].min(p[i][k]);
                    hi[k] = hi[k].max(p[i][k]);
                }
            }
            (lo, hi)
        })
        .collect()
}
/// Sweep broad phase; same lexicographic candidate order as numpy.unique, no quadratic allocation.
fn candidates(pa: &[V], ta: &[[usize; 3]], pb: &[V], tb: &[[usize; 3]]) -> Vec<(usize, usize)> {
    let a = bounds(pa, ta);
    let b = bounds(pb, tb);
    let mut order: Vec<_> = (0..b.len()).collect();
    order.sort_by(|&i, &j| b[i].0[0].total_cmp(&b[j].0[0]).then(i.cmp(&j)));
    let mut out = vec![];
    for (i, (lo, hi)) in a.iter().enumerate() {
        let mut js = vec![];
        for &j in &order {
            if b[j].0[0] > hi[0] {
                break;
            }
            if (0..3).all(|k| lo[k] <= b[j].1[k] && b[j].0[k] <= hi[k]) {
                js.push(j);
            }
        }
        js.sort_unstable();
        out.extend(js.into_iter().map(|j| (i, j)));
    }
    out
}
fn interval(
    p: &[V],
    t: [usize; 3],
    dist: V,
    dir: V,
    side: usize,
    other: usize,
) -> (f64, f64, Key, Key) {
    let sign = |d: f64| {
        if d > 0. {
            1
        } else if d < 0. {
            -1
        } else {
            0
        }
    };
    let lone = if sign(dist[0]) == sign(dist[1]) {
        2
    } else if sign(dist[0]) == sign(dist[2]) {
        1
    } else {
        0
    };
    let mut ends = vec![];
    for k in [(lone + 1) % 3, (lone + 2) % 3] {
        let u = t[lone];
        let v = t[k];
        let x = add(
            p[u],
            mul(sub(p[v], p[u]), dist[lone] / (dist[lone] - dist[k])),
        );
        ends.push((dot(x, dir), [side, u.min(v), u.max(v), other]));
    }
    if ends[0].0 > ends[1].0 {
        ends.swap(0, 1);
    }
    (ends[0].0, ends[1].0, ends[0].1, ends[1].1)
}
/// The reference's float64 plane distances are retained. Exact orient3d is used only
/// when cancellation makes their sign indeterminate; it never changes a reliable sign.
fn distances(p: &[V], t: [usize; 3], op: &[V], ot: [usize; 3], plane: (V, f64)) -> V {
    t.map(|i| {
        let d = dot(p[i], plane.0) + plane.1;
        let magnitude = (0..3).map(|k| (p[i][k] * plane.0[k]).abs()).sum::<f64>() + plane.1.abs();
        if d.abs() > 16. * f64::EPSILON * magnitude {
            return d;
        }
        let exact = orient3d(op[ot[0]], op[ot[1]], op[ot[2]], p[i]);
        if exact == 0. {
            0.
        } else if d != 0. && d.signum() == (-exact).signum() {
            d
        } else {
            -exact / len(cross(sub(op[ot[1]], op[ot[0]]), sub(op[ot[2]], op[ot[0]]))).max(1e-300)
        }
    })
}
// Expansion arithmetic fallback: error-free two-sum, two-diff and FMA product.
// Low terms precede high terms. All input positions are finite float32-derived
// coordinates; products here are comfortably inside binary64's exponent range.
fn grow(e: &[f64], b: f64) -> Vec<f64> {
    let mut q = b;
    let mut out = vec![];
    for &a in e {
        let s = q + a;
        let bv = s - q;
        let err = (q - (s - bv)) + (a - bv);
        if err != 0. {
            out.push(err);
        }
        q = s;
    }
    if q != 0. || out.is_empty() {
        out.push(q);
    }
    out
}
fn esum(a: &[f64], b: &[f64]) -> Vec<f64> {
    let mut r = a.to_vec();
    for &x in b {
        r = grow(&r, x);
    }
    r
}
fn eprod(a: &[f64], b: &[f64]) -> Vec<f64> {
    let mut r = vec![0.];
    for &x in a {
        for &y in b {
            let p = x * y;
            r = esum(&r, &[x.mul_add(y, -p), p]);
        }
    }
    r
}
fn ediff(a: f64, b: f64) -> Vec<f64> {
    let x = a - b;
    let bv = a - x;
    let av = x + bv;
    let br = bv - b;
    let ar = a - av;
    vec![ar + br, x]
}
pub(crate) fn orient3d(a: V, b: V, c: V, d: V) -> f64 {
    let ad = sub(a, d);
    let bd = sub(b, d);
    let cd = sub(c, d);
    let det = dot(ad, cross(bd, cd));
    let permanent = (bd[1] * cd[2]).abs() + (bd[2] * cd[1]).abs();
    let permanent = permanent * ad[0].abs()
        + ((bd[2] * cd[0]).abs() + (bd[0] * cd[2]).abs()) * ad[1].abs()
        + ((bd[0] * cd[1]).abs() + (bd[1] * cd[0]).abs()) * ad[2].abs();
    if det.abs() > 8. * f64::EPSILON * permanent {
        return det;
    }
    let a: Vec<_> = (0..3).map(|k| ediff(a[k], d[k])).collect();
    let b: Vec<_> = (0..3).map(|k| ediff(b[k], d[k])).collect();
    let c: Vec<_> = (0..3).map(|k| ediff(c[k], d[k])).collect();
    let mut out = vec![0.];
    for k in 0..3 {
        let i = (k + 1) % 3;
        let j = (k + 2) % 3;
        let negative = eprod(&b[j], &c[i]).iter().map(|x| -x).collect::<Vec<_>>();
        out = esum(&out, &eprod(&a[k], &esum(&eprod(&b[i], &c[j]), &negative)));
    }
    let estimate = out.iter().sum::<f64>();
    if estimate != 0. {
        estimate
    } else {
        *out.last().unwrap_or(&0.)
    }
}
fn inside(p: V, po: &[V], to: &[[usize; 3]], scale: f64) -> bool {
    let eps = 1e-7 * scale.max(1e-12);
    let mut votes = 0;
    for d in [
        [0.5774, 0.4122, 0.7046],
        [-0.3313, 0.8661, 0.3742],
        [0.7421, -0.5167, -0.4271],
    ] {
        let d = unit(d);
        let mut o = p;
        let mut parity = false;
        for _ in 0..1024 {
            let mut best = f64::INFINITY;
            for t in to {
                let a = po[t[0]];
                let e1 = sub(po[t[1]], a);
                let e2 = sub(po[t[2]], a);
                let pv = cross(d, e2);
                let det = dot(e1, pv);
                if det.abs() <= 1e-14 * (len(e1) * len(e2)).max(1e-300) {
                    continue;
                }
                let inv = 1. / det;
                let tv = sub(o, a);
                let u = dot(tv, pv) * inv;
                let qv = cross(tv, e1);
                let v = dot(d, qv) * inv;
                let dist = dot(e2, qv) * inv;
                if u >= -1e-12 && v >= -1e-12 && u + v <= 1. + 1e-12 && dist >= 0. {
                    best = best.min(dist);
                }
            }
            if !best.is_finite() {
                break;
            }
            parity = !parity;
            o = add(o, mul(d, best + eps));
        }
        votes += parity as usize;
    }
    votes >= 2
}
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
