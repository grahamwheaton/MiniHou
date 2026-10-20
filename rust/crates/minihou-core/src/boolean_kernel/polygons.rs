use crate::error::{Error, Result};
use super::{area2, cross2, V2};

pub(crate) fn inside_poly(p: V2, poly: &[usize], xy: &[V2]) -> bool {
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
