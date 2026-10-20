use super::{add, cross, dot, len, mul, sub, unit, Key, V};

pub(crate) fn planes(p: &[V], t: &[[usize; 3]]) -> Vec<(V, f64)> {
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
pub(crate) fn candidates(
    pa: &[V],
    ta: &[[usize; 3]],
    pb: &[V],
    tb: &[[usize; 3]],
) -> Vec<(usize, usize)> {
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

pub(crate) fn interval(
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
pub(crate) fn distances(p: &[V], t: [usize; 3], op: &[V], ot: [usize; 3], plane: (V, f64)) -> V {
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

pub(crate) fn inside(p: V, po: &[V], to: &[[usize; 3]], scale: f64) -> bool {
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
