use super::{
    add, area2, basis, cross, cross2, dot, mul,
    polygons::{earclip, holes_into, inside_poly},
    unit, V, V2,
};
use crate::error::{Error, Result};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn split_triangle(
    xy: &[V2],
    boundary: Vec<usize>,
    segs: &[[usize; 2]],
) -> Result<Vec<Vec<usize>>> {
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
