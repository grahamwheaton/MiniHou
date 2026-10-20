use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use super::VORONOI_ADJACENCY;
use super::super::{Inputs};
use super::super::model::{add, sub, mul, dot, len, positions, range, polygons, outgroup};
use crate::error::{Error, Result};
use crate::geometry::{Attr, AttrData, Geometry};
use crate::params::Params;

pub(in crate::nodes) fn extent(g: &Geometry) -> f64 {
    if g.npoints == 0 { return 1.0; }
    let p = positions(g);
    (0..3).map(|k| {
        let lo = p.iter().map(|p| p[k]).fold(f64::INFINITY, f64::min);
        let hi = p.iter().map(|p| p[k]).fold(f64::NEG_INFINITY, f64::max);
        hi - lo
    }).fold(0.0, f64::max)
}

/// Radius pairs, ordered by original indices. Sweep X to avoid scanning all distant points.
pub(in crate::nodes) fn pairs(p: &[[f64; 3]], radius: f64) -> Vec<(usize, usize)> {
    let mut order: Vec<_> = (0..p.len()).collect();
    order.sort_by(|&a, &b| p[a][0].total_cmp(&p[b][0]).then(a.cmp(&b)));
    let mut out = Vec::new();
    for (at, &a) in order.iter().enumerate() {
        for &b in &order[at + 1..] {
            if p[b][0] - p[a][0] > radius { break; }
            let d = sub(p[b], p[a]);
            if dot(d, d) <= radius * radius { out.push((a.min(b), a.max(b))); }
        }
    }
    out.sort_unstable();
    out
}

/// Opposed cut faces belonging to different pieces, with their shared contact area.
pub(in crate::nodes) fn contacts(g: &Geometry, pieces: &[i32], max_gap: f64) -> Vec<(usize, usize, f64)> {
    let p = positions(g);
    let normals = crate::normals::prim_normals(g.p(), &g.topology);
    let area: Vec<_> = normals.iter().map(|&n| len(n) * 0.5).collect();
    let unit: Vec<_> = normals.iter().map(|&n| mul(n, 1.0 / len(n).max(1e-30))).collect();
    let centres: Vec<_> = (0..g.nprims()).map(|i| {
        let r = range(g, i);
        let n = r.len().max(1) as f64;
        mul(r.fold([0.0; 3], |sum, v| add(sum, p[g.topology.vertex_point[v] as usize])), 1.0 / n)
    }).collect();
    let tol = 1e-5 * extent(g).max(1e-9);
    let rad: Vec<_> = area.iter().map(|a| (a / std::f64::consts::PI).sqrt()).collect();
    let radius = if max_gap <= 0.0 { tol * 5.0 } else {
        max_gap + 2.0 * rad.iter().copied().fold(0.0, f64::max) + tol
    };
    pairs(&centres, radius).into_iter().filter_map(|(a, b)| {
        if pieces[a] == pieces[b] || dot(unit[a], unit[b]) >= -0.999 { return None; }
        let d = sub(centres[b], centres[a]);
        let along = dot(d, unit[a]);
        let side = len(sub(d, mul(unit[a], along)));
        let ok = if max_gap <= 0.0 {
            along.abs() <= tol && side <= tol * 4.0
                && area[a].min(area[b]) / area[a].max(area[b]).max(1e-300) >= 1.0 - 1e-3
        } else { along.abs() <= max_gap + tol && side <= rad[a] + rad[b] };
        ok.then(|| if pieces[a] < pieces[b] { (a, b, area[a].min(area[b])) }
            else { (b, a, area[a].min(area[b])) })
    }).collect()
}
pub(super) fn voronoi_adjacency(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = VORONOI_ADJACENCY.one(inputs, 0)?;
    polygons(&g, "voronoi_adjacency")?;
    let name = p.s("piece_attrib")?;
    let pieces = if name.is_empty() { super::super::attrib_tools::piece_labels(&g).1 } else {
        let attr = g.prim.get(&name).ok_or_else(|| Error::new(format!("no primitive attribute {name:?}: fracture first, or leave piece attribute empty to use connectivity")))?;
        match &attr.data {
            AttrData::I32(v) if attr.width == 1 => v.clone(),
            _ => return Err(Error::new(format!("{name:?} must be a scalar integer primitive attribute"))),
        }
    };
    let ids: Vec<_> = pieces.iter().copied().collect::<BTreeSet<_>>().into_iter().collect();
    let slot: BTreeMap<_, _> = ids.iter().enumerate().map(|(i, &id)| (id, i)).collect();
    let mut centres = vec![[0.0; 3]; ids.len()];
    if let Some(points) = inputs.get(1).and_then(|i| i.first()).filter(|g| g.npoints > 0) {
        let pos = positions(points);
        for (i, &id) in ids.iter().enumerate() {
            if id < 0 || id as usize >= points.npoints {
                return Err(Error::new("the points input has fewer points than there are pieces"));
            }
            centres[i] = pos[id as usize];
        }
    } else {
        let pos = positions(&g);
        let mut counts = vec![0usize; ids.len()];
        for (i, &piece) in pieces.iter().enumerate() {
            let k = slot[&piece];
            for v in range(&g, i) {
                centres[k] = add(centres[k], pos[g.topology.vertex_point[v] as usize]);
                counts[k] += 1;
            }
        }
        for (c, n) in centres.iter_mut().zip(counts) { *c = mul(*c, 1.0 / n.max(1) as f64); }
    }
    let max_gap = p.f("max_gap")?;
    let min_area = p.f("min_area")?;
    let strong = p.f("strong_area")?;
    if !max_gap.is_finite() || max_gap < 0.0 || !min_area.is_finite() || min_area < 0.0 || !(0.0..=1.0).contains(&strong) {
        return Err(Error::new("max_gap/min_area must be nonnegative and strong_area must be 0..1"));
    }
    let mut summed = BTreeMap::<(i32, i32), f64>::new();
    for (a, b, area) in contacts(&g, &pieces, max_gap) {
        *summed.entry((pieces[a], pieces[b])).or_default() += area;
    }
    let mut vp = Vec::new();
    let mut piece_a = Vec::new();
    let mut piece_b = Vec::new();
    let mut areas = Vec::new();
    let mut degree = vec![0i32; ids.len()];
    for ((a, b), area) in summed {
        if area < min_area.max(1e-300) { continue; }
        let (a_slot, b_slot) = (slot[&a], slot[&b]);
        vp.extend([a_slot as i32, b_slot as i32]);
        degree[a_slot] += 1;
        degree[b_slot] += 1;
        piece_a.push(a);
        piece_b.push(b);
        areas.push(area);
    }
    let n = areas.len();
    let mut out = Geometry::from_arrays(centres.into_iter().flatten().map(|v| v as f32).collect(), vp,
        (0..=n).map(|i| (i * 2) as i32).collect(), Some(vec![false; n]))?;
    out.point.insert("piece".into(), Arc::new(Attr::i32(1, ids)));
    out.point.insert("neighbors".into(), Arc::new(Attr::i32(1, degree.clone())));
    out.prim.insert("piece_a".into(), Arc::new(Attr::i32(1, piece_a)));
    out.prim.insert("piece_b".into(), Arc::new(Attr::i32(1, piece_b)));
    out.prim.insert("area".into(), Arc::new(Attr::f32(1, areas.iter().map(|&a| a as f32).collect())));
    outgroup(&mut out, p, "connected", "point", &degree.iter().map(|&d| d > 0).collect::<Vec<_>>())?;
    outgroup(&mut out, p, "isolated", "point", &degree.iter().map(|&d| d == 0).collect::<Vec<_>>())?;
    let biggest = areas.iter().copied().fold(0.0, f64::max);
    outgroup(&mut out, p, "strong", "prim", &areas.iter().map(|&a| a >= strong * biggest).collect::<Vec<_>>())?;
    Ok(out)
}
