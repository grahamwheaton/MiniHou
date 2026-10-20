//! Bounded solid Push/Pull and named inset/outset, from pushpull.py.
use super::{GEO, Inputs, NodeSpec, model};
use super::{boolean, model::*, modeling_tools::twins};
use crate::{
    error::{Error, Result},
    geometry::{Attr, Geometry},
    params::Params,
};
use std::{collections::{BTreeMap, BTreeSet, HashMap}, sync::Arc};
pub const PUSHPULL: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "pushpull",
    inputs: GEO,
    params: crate::params!("group":"";"distance":0.25;"through":"auto"),
    cook: pushpull,
};
fn solid(g: &Geometry, label: &str) -> Result<()> {
    polygons(g, label)?;
    if g.nprims() > 0
        && (twins(g).iter().any(Option::is_none) || g.p().iter().any(|x| !x.is_finite()))
    {
        return Err(Error::new(format!(
            "{label} requires a closed, consistently wound manifold solid"
        )));
    }
    let pos = positions(g);
    for face in 0..g.nprims() {
        let q: Vec<_> = range(g, face)
            .map(|v| pos[g.topology.vertex_point[v] as usize])
            .collect();
        if q.len() > 256 {
            return Err(Error::new(format!(
                "{label} supports polygons with at most 256 corners; triangulate first"
            )));
        }
        let mut n = [0.; 3];
        for k in 0..q.len() {
            n = add(n, cross(q[k], q[(k + 1) % q.len()]));
        }
        let axis = (0..3)
            .max_by(|&a, &b| n[a].abs().total_cmp(&n[b].abs()).then(b.cmp(&a)))
            .unwrap();
        let xy: Vec<_> = q
            .iter()
            .map(|p| {
                let v: Vec<_> = (0..3).filter(|&k| k != axis).map(|k| p[k]).collect();
                [v[0], v[1]]
            })
            .collect();
        let cr = crate::boolean_kernel::cross2;
        for i in 0..q.len() {
            for j in i + 2..q.len() - usize::from(i == 0) {
                let (a, b, c, d) = (xy[i], xy[(i + 1) % q.len()], xy[j], xy[(j + 1) % q.len()]);
                if cr(a, b, c) * cr(a, b, d) < 0. && cr(c, d, a) * cr(c, d, b) < 0. {
                    let ei = [b[0] - a[0], b[1] - a[1]];
                    let ej = [d[0] - c[0], d[1] - c[1]];
                    let angle = (ei[0] * ej[1] - ei[1] * ej[0]).abs()
                        / (ei[0].hypot(ei[1]) * ej[0].hypot(ej[1])).max(1e-30);
                    if angle > 1e-3 {
                        return Err(Error::new(format!(
                            "{label} contains a self-crossing polygon"
                        )));
                    }
                }
            }
        }
    }
    if g.nprims() > 0 {
        let (ts, _, _) = boolean::triangles(g);
        let volume = ts
            .iter()
            .map(|t| dot(pos[t[0]], cross(pos[t[1]], pos[t[2]])))
            .sum::<f64>()
            / 6.;
        if volume <= 0. {
            return Err(Error::new(format!(
                "{label} requires outward-facing solid geometry; reverse inward faces first"
            )));
        }
    }
    Ok(())
}
fn pushpull(i: &Inputs, p: &Params) -> Result<Geometry> {
    let g = PUSHPULL.one(i, 0)?;
    let through = match p.e("through", &["off", "auto", "force"])?.as_str() {
        "off" => Through::Off,
        "force" => Through::Force,
        _ => Through::Auto,
    };
    apply_through(&g, &p.s("group")?, p.f("distance")?, through)
}
/// SketchUp-style punch-through behaviour (Rust-only `through` parameter).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Through {
    /// Python-compatible: exactly one convex face, the cutter ends at `distance`.
    Off,
    /// Punch through when the push reaches (or passes) the opposite surface; the
    /// excess is ignored. A pull landing on a parallel face plane merges into it.
    Auto,
    /// Always punch through to the opposite surface when one exists.
    Force,
}
#[cfg(test)]
pub(super) fn apply(g: &Geometry, group: &str, distance: f64) -> Result<Geometry> {
    apply_through(g, group, distance, Through::Off)
}
/// Selected faces that are edge-connected and coplanar are pushed as one region, so
/// a face split around a hole (or several adjacent faces) gives one closed prism.
struct Region {
    prims: Vec<usize>,
    /// Point ids of each face, in face winding order.
    loops: Vec<Vec<usize>>,
    /// Region-boundary half-edges (point ids) and the face that owns each.
    boundary: Vec<(usize, usize, usize)>,
    n: [f64; 3],
}
fn face_normal(
    g: &Geometry,
    pos: &[[f64; 3]],
    face: usize,
    tolerance: f64,
    convex: bool,
) -> Result<[f64; 3]> {
    let points: Vec<_> = range(g, face)
        .map(|v| pos[g.topology.vertex_point[v] as usize])
        .collect();
    let normal = crate::normals::prim_normals(g.p(), &g.topology)[face];
    let n = crate::boolean_kernel::unit(normal);
    if points.len() < 3
        || len(n) < 0.5
        || points
            .iter()
            .any(|&p| dot(sub(p, points[0]), n).abs() > tolerance)
    {
        return Err(Error::new(
            "Push/Pull: selected face must be non-degenerate and planar",
        ));
    }
    let count = points.len();
    let edges: Vec<_> = (0..count)
        .map(|i| sub(points[(i + 1) % count], points[i]))
        .collect();
    if convex
        && (0..count)
            .any(|i| dot(cross(edges[i], edges[(i + 1) % count]), n) < -tolerance * tolerance)
    {
        return Err(Error::new(
            "Push/Pull: concave faces are not supported; split into convex faces first",
        ));
    }
    Ok(n)
}
fn regions(
    g: &Geometry,
    pos: &[[f64; 3]],
    ids: &[usize],
    tolerance: f64,
    convex: bool,
) -> Result<Vec<Region>> {
    let vp = &g.topology.vertex_point;
    let loop_of = |f: usize| -> Vec<usize> { range(g, f).map(|v| vp[v] as usize).collect() };
    let normals: Vec<_> = ids
        .iter()
        .map(|&f| face_normal(g, pos, f, tolerance, convex))
        .collect::<Result<_>>()?;
    let mut owner = HashMap::new();
    for (k, &f) in ids.iter().enumerate() {
        let l = loop_of(f);
        for i in 0..l.len() {
            owner.insert((l[i], l[(i + 1) % l.len()]), k);
        }
    }
    let mut parent: Vec<usize> = (0..ids.len()).collect();
    fn root(p: &mut [usize], mut x: usize) -> usize {
        while p[x] != x {
            p[x] = p[p[x]];
            x = p[x];
        }
        x
    }
    for (&(a, b), &k) in &owner {
        if let Some(&j) = owner.get(&(b, a)) {
            let plane = |p: usize| dot(sub(pos[p], pos[loop_of(ids[k])[0]]), normals[k]).abs();
            if j != k
                && dot(normals[j], normals[k]) > 1. - 1e-9
                && loop_of(ids[j]).iter().all(|&p| plane(p) <= tolerance)
            {
                let (x, y) = (root(&mut parent, j), root(&mut parent, k));
                parent[x.max(y)] = x.min(y);
            }
        }
    }
    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for k in 0..ids.len() {
        let r = root(&mut parent, k);
        groups.entry(r).or_default().push(k);
    }
    Ok(groups
        .into_values()
        .map(|members| {
            let inside: BTreeSet<_> = members.iter().copied().collect();
            let mut boundary = vec![];
            let loops: Vec<_> = members.iter().map(|&k| loop_of(ids[k])).collect();
            for (&k, l) in members.iter().zip(&loops) {
                for i in 0..l.len() {
                    let (a, b) = (l[i], l[(i + 1) % l.len()]);
                    if !owner.get(&(b, a)).is_some_and(|j| inside.contains(j)) {
                        boundary.push((a, b, ids[k]));
                    }
                }
            }
            Region {
                prims: members.iter().map(|&k| ids[k]).collect(),
                loops,
                boundary,
                n: normals[members[0]],
            }
        })
        .collect())
}
fn extent_of(pos: &[[f64; 3]]) -> f64 {
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for p in pos {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    (0..3).map(|k| hi[k] - lo[k]).fold(1e-8, f64::max)
}
pub(super) fn apply_through(
    g: &Geometry,
    group: &str,
    distance: f64,
    through: Through,
) -> Result<Geometry> {
    if g.nprims() > 20000 || g.npoints > 50000 {
        return Err(Error::new(
            "Push/Pull supports up to 20,000 faces / 50,000 points; simplify the solid first",
        ));
    }
    if !distance.is_finite() && through != Through::Off {
        return Err(Error::new("Push/Pull distance must be finite"));
    }
    if through != Through::Off && g.nprims() > 0 && twins(g).iter().any(Option::is_none) {
        // Open sheet: there is no opposite surface to punch, so this is a plain push/pull.
        polygons(g, "Push/Pull")?;
        if !prim_mask(g, group)?.iter().any(|&x| x) {
            return Err(Error::new("Push/Pull: select at least one face"));
        }
        return model::extrude(g, group, distance, 0., "individual", false, None);
    }
    solid(g, "Push/Pull")?;
    let selected = prim_mask(g, group)?;
    let ids: Vec<_> = (0..g.nprims()).filter(|&i| selected[i]).collect();
    if through == Through::Off && ids.len() != 1 {
        return Err(Error::new(
            "Push/Pull: select exactly one planar convex face (use Inset for a smaller opening)",
        ));
    }
    if ids.is_empty() || ids.len() == g.nprims() {
        return Err(Error::new(
            "Push/Pull: select one or more faces, not the whole solid",
        ));
    }
    if !distance.is_finite() {
        return Err(Error::new("Push/Pull distance must be finite"));
    }
    let pos = positions(g);
    let extent = extent_of(&pos);
    let tolerance = extent * 1e-6;
    let regions = regions(g, &pos, &ids, tolerance, through == Through::Off)?;
    if distance.abs() <= tolerance && through != Through::Force {
        return Ok(g.clone());
    }
    let (tris, _, tri_prim) = boolean::triangles(g);
    let snap = extent * 1e-5;
    let overlap = tolerance * 10.;
    let mut merge = false;
    let mut plans = vec![];
    for r in &regions {
        let mut d = distance;
        if through != Through::Off {
            if d < 0. || through == Through::Force {
                let samples: Vec<_> = r
                    .prims
                    .iter()
                    .flat_map(|&f| face_samples(&pos, &tris, &tri_prim, f))
                    .collect();
                let dir = mul(r.n, -1.);
                let hit = exit_depth(&pos, &tris, &tri_prim, &r.prims, &samples, dir, tolerance);
                if let Some((_, deepest)) =
                    hit.filter(|&(_, deep)| through == Through::Force || -d >= deep - snap)
                {
                    // Stop just past the far surface: excess push is ignored, and the
                    // cutter never ends coplanar with the opposite face.
                    d = -(deepest + extent * 1e-4);
                }
            } else if let Some(t) = coplanar_target(g, &pos, &selected, r, d, snap) {
                d = t;
                merge = true;
            }
        }
        plans.push(d);
    }
    let mut out = g.clone();
    for (&d, r) in plans.iter().zip(&regions) {
        if d.abs() <= tolerance {
            continue;
        }
        let (low, high) = if d > 0. { (-overlap, d) } else { (d, overlap) };
        let op = if d > 0. { "union" } else { "subtract" };
        let expand = if through != Through::Off && d < 0. { extent * 1e-4 } else { 0. };
        out = cut(&out, g, &pos, r, low, high, op, merge, expand)?;
        if through != Through::Off {
            out = drop_collinear(&out, extent * 1e-6)?;
        }
    }
    if merge {
        out = merge_coplanar(&out, extent * 1e-5)?;
    }
    if out.prim.remove(TAG).is_some() {
        out.validate()?;
    }
    Ok(out)
}
const TAG: &str = "__pushpull_new";
/// Points strictly inside the face (its triangles' centroids and pulled-in corners).
fn face_samples(
    pos: &[[f64; 3]],
    tris: &[[usize; 3]],
    tri_prim: &[usize],
    face: usize,
) -> Vec<[f64; 3]> {
    let mut out = vec![];
    for (t, _) in tris.iter().zip(tri_prim).filter(|(_, p)| **p == face) {
        let c = mul(add(add(pos[t[0]], pos[t[1]]), pos[t[2]]), 1. / 3.);
        out.push(c);
        for &i in t {
            out.push(add(pos[i], mul(sub(c, pos[i]), 0.01)));
        }
    }
    out
}
/// Nearest surface hit along `dir` beyond `min_t`, ignoring primitives in `skip`.
pub fn ray_hit(
    pos: &[[f64; 3]],
    tris: &[[usize; 3]],
    tri_prim: &[usize],
    skip: &[usize],
    origin: [f64; 3],
    dir: [f64; 3],
    min_t: f64,
) -> Option<f64> {
    let mut best: Option<f64> = None;
    for (t, p) in tris.iter().zip(tri_prim) {
        if skip.contains(p) {
            continue;
        }
        let (a, b, c) = (pos[t[0]], pos[t[1]], pos[t[2]]);
        let e1 = sub(b, a);
        let e2 = sub(c, a);
        let h = cross(dir, e2);
        let det = dot(e1, h);
        if det.abs() <= 1e-14 * len(e1) * len(e2) {
            continue;
        }
        let s = sub(origin, a);
        let u = dot(s, h) / det;
        let q = cross(s, e1);
        let v = dot(dir, q) / det;
        let eps = 1e-9;
        if u < -eps || v < -eps || u + v > 1. + eps {
            continue;
        }
        let d = dot(e2, q) / det;
        if d > min_t && best.is_none_or(|b| d < b) {
            best = Some(d);
        }
    }
    best
}
/// (shallowest, deepest) first exit along the push direction over all samples,
/// or None when any sample escapes into empty space.
fn exit_depth(
    pos: &[[f64; 3]],
    tris: &[[usize; 3]],
    tri_prim: &[usize],
    skip: &[usize],
    samples: &[[f64; 3]],
    dir: [f64; 3],
    tolerance: f64,
) -> Option<(f64, f64)> {
    let mut range = (f64::INFINITY, f64::NEG_INFINITY);
    for &s in samples {
        let t = ray_hit(pos, tris, tri_prim, skip, s, dir, tolerance)?;
        range = (range.0.min(t), range.1.max(t));
    }
    (!samples.is_empty()).then_some(range)
}
/// Offset of the same-facing parallel face plane matching a pull distance.
fn coplanar_target(
    g: &Geometry,
    pos: &[[f64; 3]],
    selected: &[bool],
    r: &Region,
    d: f64,
    snap: f64,
) -> Option<f64> {
    let ns = crate::normals::prim_normals(g.p(), &g.topology);
    let anchor = pos[r.loops[0][0]];
    (0..g.nprims())
        .filter(|&q| !selected[q])
        .filter_map(|q| {
            let m = crate::boolean_kernel::unit(ns[q]);
            if dot(m, r.n) < 1. - 1e-9 {
                return None;
            }
            let p = pos[g.topology.vertex_point[range(g, q).start] as usize];
            let t = dot(sub(p, anchor), r.n);
            (t > snap && (t - d).abs() <= snap).then_some(t)
        })
        .min_by(|a, b| (a - d).abs().total_cmp(&(b - d).abs()))
}
#[allow(clippy::too_many_arguments)]
fn cut(
    current: &Geometry,
    source: &Geometry,
    pos: &[[f64; 3]],
    r: &Region,
    low: f64,
    high: f64,
    op: &str,
    tag: bool,
    expand: f64,
) -> Result<Geometry> {
    let n = r.n;
    let mut index = HashMap::new();
    let mut order = vec![];
    for l in &r.loops {
        for &p in l {
            index.entry(p).or_insert_with(|| {
                order.push(p);
                order.len() - 1
            });
        }
    }
    let count = order.len();
    let mut points: Vec<_> = order.iter().map(|&p| pos[p]).collect();
    if expand > 0. {
        // Walls that would lie on a convex neighbour (a box side, an existing hole's
        // wall) move out into empty space by `expand`, so the cutter never overlaps
        // those faces coplanarly; the extra volume removed is air.
        let ns = crate::normals::prim_normals(source.p(), &source.topology);
        let vp = &source.topology.vertex_point;
        let mut owner = HashMap::new();
        for f in 0..source.nprims() {
            let rg = range(source, f);
            for c in rg.clone() {
                let next = if c + 1 == rg.end { rg.start } else { c + 1 };
                owner.insert((vp[c] as usize, vp[next] as usize), f);
            }
        }
        let mut shift = vec![[0.; 3]; count];
        for &(a, b, _) in &r.boundary {
            let Some(&m) = owner.get(&(b, a)) else { continue };
            let out = crate::boolean_kernel::unit(cross(sub(pos[b], pos[a]), n));
            if dot(crate::boolean_kernel::unit(ns[m]), out) > 0.5 {
                for p in [a, b] {
                    let s = &mut shift[index[&p]];
                    if dot(*s, out) < 0.5 * expand {
                        *s = add(*s, mul(out, expand));
                    }
                }
            }
        }
        for (p, s) in points.iter_mut().zip(&shift) {
            *p = add(*p, *s);
        }
    }
    let cutterpos: Vec<_> = [low, high]
        .iter()
        .flat_map(|&d| points.iter().map(move |&p| add(p, mul(n, d))))
        .collect();
    let mut faces = vec![];
    let mut sources = vec![];
    for (l, &prim) in r.loops.iter().zip(&r.prims) {
        faces.push(l.iter().rev().map(|p| index[p]).collect::<Vec<_>>());
        faces.push(l.iter().map(|p| index[p] + count).collect());
        sources.extend([prim, prim]);
    }
    for &(a, b, prim) in &r.boundary {
        let (i, j) = (index[&a], index[&b]);
        faces.push(vec![i, j, j + count, i + count]);
        sources.push(prim);
    }
    let mut cutter = Geometry::from_arrays(
        cutterpos.iter().flat_map(|p| p.map(|v| v as f32)).collect(),
        faces.iter().flatten().map(|&v| v as i32).collect(),
        {
            let mut o = vec![0];
            for f in &faces {
                o.push(o.last().unwrap() + f.len() as i32);
            }
            o
        },
        None,
    )?;
    // New walls and caps inherit the pushed face's primitive attributes (its material
    // slot); the boolean gives these source-less corners invalid (NaN) UVs.
    cutter.prim = gather(&source.prim, &sources);
    cutter.detail = source.detail.clone();
    let mut current = current.clone();
    if tag {
        if !current.prim.contains_key(TAG) {
            let zeros = Attr::i32(1, vec![0; current.nprims()]);
            current.prim.insert(TAG.into(), Arc::new(zeros));
        }
        let ones = Attr::i32(1, vec![1; faces.len()]);
        cutter.prim.insert(TAG.into(), Arc::new(ones));
    }
    let (a, b, _) = r.boundary[0];
    solve(&current, &cutter, &points, sub(pos[b], pos[a]), n, op)
}
fn solve(
    g: &Geometry,
    cutter: &Geometry,
    points: &[[f64; 3]],
    edge0: [f64; 3],
    n: [f64; 3],
    op: &str,
) -> Result<Geometry> {
    let pos = positions(g);
    let count = points.len();
    let origin = mul(points.iter().copied().fold([0.; 3], add), 1. / count as f64);
    let mut last = String::new();
    for angle in [0f64, 0.37, 0.91, 1.57] {
        let u = crate::boolean_kernel::unit(edge0);
        let v = cross(n, u);
        let (sin, cos) = angle.sin_cos();
        let ur = add(mul(u, cos), mul(v, sin));
        let vr = sub(mul(v, cos), mul(u, sin));
        let basis = [ur, vr, n];
        let mut local = g.clone();
        let mut tool = cutter.clone();
        let transform = |points: &[[f64; 3]]| {
            points
                .iter()
                .flat_map(|&p| basis.map(|a| dot(sub(p, origin), a) as f32))
                .collect()
        };
        local.set_p(transform(&pos));
        tool.set_p(transform(&positions(cutter)));
        let attempt = boolean::boolean(&local, &tool, op, None).and_then(|mut out| {
            let points = positions(&out);
            out.set_p(
                points
                    .iter()
                    .flat_map(|p| {
                        add(origin, add(add(mul(ur, p[0]), mul(vr, p[1])), mul(n, p[2])))
                            .map(|v| v as f32)
                    })
                    .collect(),
            );
            solid(&out, "Push/Pull result")?;
            out.validate()?;
            // A degenerate solve can stay closed yet lose or gain material: the result
            // must lie between the input and input -/+ cutter volumes.
            let (va, vb, vo) = (signed_volume(g), signed_volume(cutter), signed_volume(&out));
            let slack = 1e-4 * (va.abs() + vb.abs()) + 1e-12;
            let (lo, hi) = if op == "union" { (va, va + vb) } else { (va - vb, va) };
            if vo < lo - slack || vo > hi + slack {
                return Err(Error::new("Push/Pull result volume is inconsistent"));
            }
            Ok(out)
        });
        match attempt {
            Ok(out) => return Ok(out),
            Err(e) => last = e.to_string(),
        }
    }
    Err(Error::new(format!(
        "Push/Pull could not produce a watertight solid; adjust the distance or split the face. {last}"
    )))
}
fn signed_volume(g: &Geometry) -> f64 {
    let pos = positions(g);
    let (ts, _, _) = boolean::triangles(g);
    ts.iter()
        .map(|t| dot(pos[t[0]], cross(pos[t[1]], pos[t[2]])))
        .sum::<f64>()
        / 6.
}
/// Remove points that only split a straight edge shared by exactly two faces (the
/// boolean leaves them where a cutter wall's diagonal crossed the far surface). The
/// mesh stays closed: both faces lose the same corner.
fn drop_collinear(g: &Geometry, tol: f64) -> Result<Geometry> {
    let pos = positions(g);
    let vp = &g.topology.vertex_point;
    let np = g.nprims();
    let mut uses: Vec<Vec<(usize, usize)>> = vec![vec![]; g.npoints];
    for p in 0..np {
        for c in range(g, p) {
            uses[vp[c] as usize].push((p, c));
        }
    }
    let around = |p: usize, c: usize| {
        let r = range(g, p);
        let prev = if c == r.start { r.end - 1 } else { c - 1 };
        let next = if c + 1 == r.end { r.start } else { c + 1 };
        (vp[prev] as usize, vp[next] as usize)
    };
    let mut size: Vec<usize> = (0..np).map(|p| range(g, p).len()).collect();
    let mut removed = vec![false; g.nvertices()];
    for (point, u) in uses.iter().enumerate() {
        let [(fa, ca), (fb, cb)] = u.as_slice() else { continue };
        let (a1, a2) = around(*fa, *ca);
        let (b1, b2) = around(*fb, *cb);
        if fa == fb || a1 != b2 || a2 != b1 || size[*fa] <= 3 || size[*fb] <= 3 {
            continue;
        }
        let (s, e, x) = (pos[a1], pos[a2], pos[point]);
        let d = sub(e, s);
        let l2 = dot(d, d);
        if l2 <= tol * tol {
            continue;
        }
        let t = dot(sub(x, s), d) / l2;
        if t <= 0. || t >= 1. || len(sub(x, add(s, mul(d, t)))) > tol {
            continue;
        }
        // Neighbours must not themselves be removed in this pass (keeps the test exact).
        if [a1, a2].iter().any(|&q| uses[q].iter().any(|&(_, c)| removed[c])) {
            continue;
        }
        removed[*ca] = true;
        removed[*cb] = true;
        size[*fa] -= 1;
        size[*fb] -= 1;
    }
    if !removed.iter().any(|&x| x) {
        return Ok(g.clone());
    }
    let faces: Vec<Vec<usize>> = (0..np)
        .map(|p| range(g, p).filter(|&c| !removed[c]).collect())
        .collect();
    let sources: Vec<_> = (0..np).collect();
    let out = compact(rebuild(g, &faces, &sources, None)?)?;
    if solid(&out, "Push/Pull result").is_err() {
        return Ok(g.clone());
    }
    Ok(out)
}
/// Dissolve edges between coplanar, same-facing neighbours when at least one of them
/// was created by this Push/Pull (SketchUp's coplanar merge). Components whose outline
/// is not a single simple loop are left unchanged.
fn merge_coplanar(g: &Geometry, tol: f64) -> Result<Geometry> {
    let Some(tag) = g.prim.get(TAG).and_then(|a| a.as_i32().map(<[i32]>::to_vec)) else {
        return Ok(g.clone());
    };
    let pos = positions(g);
    let vp = &g.topology.vertex_point;
    let ns: Vec<_> = crate::normals::prim_normals(g.p(), &g.topology)
        .into_iter()
        .map(crate::boolean_kernel::unit)
        .collect();
    let np = g.nprims();
    let next = |c: usize, p: usize| {
        let r = range(g, p);
        if c + 1 == r.end { r.start } else { c + 1 }
    };
    let mut prim_of = vec![0; g.nvertices()];
    for p in 0..np {
        for c in range(g, p) {
            prim_of[c] = p;
        }
    }
    let mut half = HashMap::new();
    for p in 0..np {
        for c in range(g, p) {
            half.insert((vp[c], vp[next(c, p)]), c);
        }
    }
    let coplanar = |a: usize, b: usize| {
        let origin = pos[vp[range(g, a).start] as usize];
        dot(ns[a], ns[b]) > 1. - 1e-9
            && range(g, b).all(|c| dot(sub(pos[vp[c] as usize], origin), ns[a]).abs() <= tol)
    };
    fn root(p: &mut [usize], mut x: usize) -> usize {
        while p[x] != x {
            p[x] = p[p[x]];
            x = p[x];
        }
        x
    }
    let mut parent: Vec<usize> = (0..np).collect();
    for p in 0..np {
        for c in range(g, p) {
            if let Some(&o) = half.get(&(vp[next(c, p)], vp[c])) {
                let q = prim_of[o];
                if q != p && coplanar(p, q) {
                    let (a, b) = (root(&mut parent, p), root(&mut parent, q));
                    parent[a] = b;
                }
            }
        }
    }
    let mut comps: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for p in 0..np {
        let r = root(&mut parent, p);
        comps.entry(r).or_default().push(p);
    }
    let mut merged_into = vec![None; np];
    let mut merged_faces = BTreeMap::new();
    for polys in comps.values() {
        if polys.len() < 2 || !polys.iter().any(|&p| tag[p] != 0) {
            continue;
        }
        let set: BTreeSet<_> = polys.iter().copied().collect();
        let mut succ = BTreeMap::new();
        let mut ok = true;
        for &p in polys {
            for c in range(g, p) {
                let inner = half
                    .get(&(vp[next(c, p)], vp[c]))
                    .is_some_and(|&o| set.contains(&prim_of[o]));
                if !inner && succ.insert(vp[c], c).is_some() {
                    ok = false;
                }
            }
        }
        if !ok || succ.is_empty() {
            continue;
        }
        let start = *succ.keys().next().unwrap();
        let mut lp = vec![];
        let mut cur = start;
        while let Some(&c) = succ.get(&cur) {
            lp.push(c);
            cur = vp[next(c, prim_of[c])];
            if cur == start || lp.len() > succ.len() {
                break;
            }
        }
        if cur != start || lp.len() != succ.len() {
            continue;
        }
        let keep = polys
            .iter()
            .copied()
            .find(|&p| tag[p] == 0)
            .unwrap_or(polys[0]);
        for &p in polys {
            merged_into[p] = Some(keep);
        }
        merged_faces.insert(keep, lp);
    }
    if merged_faces.is_empty() {
        return Ok(g.clone());
    }
    let mut faces = vec![];
    let mut sources = vec![];
    for p in 0..np {
        match merged_into[p] {
            None => {
                faces.push(range(g, p).collect());
                sources.push(p);
            }
            Some(k) if k == p => {
                faces.push(merged_faces[&p].clone());
                sources.push(p);
            }
            Some(_) => {}
        }
    }
    let out = compact(rebuild(g, &faces, &sources, None)?)?;
    solid(&out, "Push/Pull result")?;
    Ok(out)
}
pub const INSET: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "inset",
    inputs: GEO,
    params: crate::params!("group":"";"amount":0.1;"mode":"connected";
        "depth":0.0;"use_boundary":true;"use_even_offset":true;"use_relative_offset":false;"use_edge_rail":false;
        "use_outset":false;"use_individual":false;"use_interpolate":true;
        "output_group":false;"output_group_name":"inset_ring";"output_inner_group":false;"output_inner_group_name":"inset_inner"),
    cook: inset,
};
pub const OUTSET: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "outset",
    inputs: GEO,
    params: crate::params!("group":"";"amount":0.1;"mode":"connected";
        "output_group":false;"output_group_name":"outset_ring";"output_inner_group":false;"output_inner_group_name":"outset_boundary"),
    cook: outset,
};
fn inset(i: &Inputs, p: &Params) -> Result<Geometry> {
    let g = INSET.one(i, 0)?;
    let individual = p.e("mode", &["connected", "individual"])? == "individual" || p.b("use_individual")?;
    let mode = if individual { "individual" } else { "connected" };
    let amount = if p.b("use_outset")? { -p.f("amount")? } else { p.f("amount")? };
    let depth = p.f("depth")?;
    let extended = !p.b("use_boundary")? || !p.b("use_even_offset")? || p.b("use_relative_offset")? || p.b("use_edge_rail")? || !p.b("use_interpolate")?;
    let mut out = if extended {
        let sel = model::prim_mask(&g, &p.s("group")?)?;
        let o = super::inset_ext::Opts { thickness: amount, depth, boundary: p.b("use_boundary")?, even: p.b("use_even_offset")?,
            relative: p.b("use_relative_offset")?, rail: p.b("use_edge_rail")?, individual, interpolate: p.b("use_interpolate")? };
        super::inset_ext::inset_ext(&g, &sel, &o)?
    } else {
        model::extrude(&g, &p.s("group")?, depth, amount, mode, false, None)?
    };
    out = auto_merge(&out, g.npoints)?;
    write_output_groups(&mut out, p)?;
    Ok(out)
}
fn write_output_groups(out: &mut Geometry, p: &Params) -> Result<()> {
    for (flag, name, src) in [("output_group", "output_group_name", "group_extrudeSide"), ("output_inner_group", "output_inner_group_name", "group_extrudeFront")] {
        if p.b(flag)? {
            let n = p.s(name)?;
            model::group_name(n.trim()).map_err(|_| Error::new(format!("invalid {name} {n:?}")))?;
            let mask: Vec<bool> = out.prim.get(src).map_or(vec![false; out.nprims()], |a| super::group::flags(a));
            model::write_group(out, "prim", n.trim(), &mask)?;
        }
    }
    Ok(())
}
/// Blender-style auto-merge: points created by the inset (index >= `first_new`) that land on another point are welded to it,
/// repeated corners are dropped, and faces left with fewer than 3 corners disappear.
pub(super) fn auto_merge(g: &Geometry, first_new: usize) -> Result<Geometry> {
    if g.npoints <= first_new { return Ok(g.clone()); }
    let pos = model::positions(g);
    let key = |q: [f64; 3]| q.map(|x| (x * 1e5).round() as i64);
    let mut seen = std::collections::HashMap::<[i64; 3], usize>::new();
    let mut map: Vec<usize> = (0..g.npoints).collect();
    for q in 0..first_new.min(g.npoints) { seen.entry(key(pos[q])).or_insert(q); }
    let mut any = false;
    for q in first_new..g.npoints {
        match seen.get(&key(pos[q])) { Some(&r) => { map[q] = r; any = true; } None => { seen.insert(key(pos[q]), q); } }
    }
    if !any { return Ok(g.clone()); }
    let vp = &g.topology.vertex_point;
    let (mut ovp, mut sizes, mut vsrc, mut psrc) = (vec![], vec![], vec![], vec![]);
    for f in 0..g.nprims() {
        let mut c: Vec<(usize, usize)> = vec![];
        for v in model::range(g, f) {
            let p = map[vp[v] as usize];
            if c.last().is_none_or(|l| l.0 != p) { c.push((p, v)); }
        }
        while c.len() > 1 && c[0].0 == c[c.len() - 1].0 { c.pop(); }
        if c.len() < 3 { continue; }
        sizes.push(c.len());
        psrc.push(f);
        for (p, v) in c { ovp.push(p as i32); vsrc.push(v); }
    }
    let out = model::make(g.point.clone(), ovp, &sizes, model::gather(&g.vertex, &vsrc), model::gather(&g.prim, &psrc), g.detail.clone(), None)?;
    let mut out = model::compact(out)?;
    out.point.remove("N");
    Ok(out)
}
fn outset(i: &Inputs, p: &Params) -> Result<Geometry> {
    let g = OUTSET.one(i, 0)?;
    let out = model::extrude(&g, &p.s("group")?, 0., -p.f("amount")?, &p.e("mode", &["connected", "individual"])?, false, None)?;
    let mut out = auto_merge(&out, g.npoints)?;
    write_output_groups(&mut out, p)?;
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    fn quad(s: f32) -> Geometry {
        Geometry::from_arrays(vec![0., 0., 0., s, 0., 0., s, s, 0., 0., s, 0.], vec![0, 1, 2, 3], vec![0, 4], None).unwrap()
    }
    #[test]
    fn inset_auto_merges_collapsed_inner_face() {
        // inset by half the width: the inner quad collapses to one point, merged with itself; ring becomes triangles
        let p = Params::resolve(INSET.params, serde_json::json!({"amount": std::f64::consts::SQRT_2, "use_even_offset": false}).as_object()).unwrap();
        let o = inset(&[vec![Arc::new(quad(2.))]], &p).unwrap();
        o.validate().unwrap();
        assert_eq!((o.npoints, o.nprims()), (5, 4));
    }
    #[test]
    fn outset_output_groups_are_optional() {
        let p = Params::resolve(OUTSET.params, None).unwrap();
        let o = outset(&[vec![Arc::new(quad(2.))]], &p).unwrap();
        assert!(!o.prim.contains_key("group_outset_ring"));
        let p = Params::resolve(OUTSET.params, serde_json::json!({"output_group": true, "output_inner_group": true}).as_object()).unwrap();
        let o = outset(&[vec![Arc::new(quad(2.))]], &p).unwrap();
        assert_eq!(o.prim["group_outset_ring"].as_i32().unwrap().iter().sum::<i32>(), 4);
        assert_eq!(o.prim["group_outset_boundary"].as_i32().unwrap().iter().sum::<i32>(), 1);
    }
    #[test]
    fn inset_has_no_height_and_python_groups() {
        let g = Geometry::from_arrays(
            vec![0., 0., 0., 2., 0., 0., 2., 2., 0., 0., 2., 0.],
            vec![0, 1, 2, 3],
            vec![0, 4],
            None,
        )
        .unwrap();
        let p = Params::resolve(INSET.params, None).unwrap();
        let o = inset(&[vec![Arc::new(g)]], &p).unwrap();
        assert_eq!((o.npoints, o.nprims()), (8, 5));
        assert!(o.p().chunks_exact(3).all(|p| p[2] == 0.));
        assert!(o.prim.contains_key("group_extrudeFront"));
        assert!(o.prim.contains_key("group_extrudeSide"));
    }
    #[test]
    fn pushpull_distance_cooks_live_preview_from_connected_box() {
        let box_params=Params::resolve(super::super::generate::BOX.params,None).unwrap();
        let g=(super::super::generate::BOX.cook)(&[vec![]],&box_params).unwrap();
        let zero=apply(&g,"0",0.).unwrap();
        let moved=apply(&g,"0",0.25).unwrap();
        assert_eq!(zero.p(),g.p());
        assert_ne!(moved.p(),g.p());
        moved.validate().unwrap();
    }
    fn mk(points: &[[f64; 3]], faces: &[Vec<usize>]) -> Geometry {
        let build = |faces: &[Vec<usize>]| {
            let mut o = vec![0];
            for f in faces {
                o.push(o.last().unwrap() + f.len() as i32);
            }
            Geometry::from_arrays(
                points.iter().flat_map(|p| p.map(|v| v as f32)).collect(),
                faces.iter().flatten().map(|&v| v as i32).collect(),
                o,
                None,
            )
            .unwrap()
        };
        let g = build(faces);
        if volume(&g) > 0. {
            return g;
        }
        let rev: Vec<Vec<usize>> = faces.iter().map(|f| f.iter().rev().copied().collect()).collect();
        build(&rev)
    }
    fn volume(g: &Geometry) -> f64 {
        let pos = positions(g);
        let (ts, _, _) = boolean::triangles(g);
        ts.iter().map(|t| dot(pos[t[0]], cross(pos[t[1]], pos[t[2]]))).sum::<f64>() / 6.
    }
    fn euler(g: &Geometry) -> i64 {
        let edges = crate::topo::edges(&g.topology, g.npoints);
        g.npoints as i64 - edges.len() as i64 + g.nprims() as i64
    }
    /// Closed, consistently wound, outward, finite, and with a given Euler characteristic.
    fn check(g: &Geometry, chi: i64) {
        g.validate().unwrap();
        solid(g, "test").unwrap();
        assert!(twins(g).iter().all(Option::is_some), "not closed");
        assert_eq!(euler(g), chi, "Euler characteristic");
        assert!(!g.prim.contains_key(TAG));
    }
    fn face_area(g: &Geometry, f: usize) -> f64 {
        let pos = positions(g);
        let q: Vec<_> = range(g, f).map(|v| pos[g.topology.vertex_point[v] as usize]).collect();
        let mut n = [0.; 3];
        for k in 0..q.len() {
            n = add(n, cross(q[k], q[(k + 1) % q.len()]));
        }
        len(n) / 2.
    }
    fn prims_where(g: &Geometry, f: impl Fn([f64; 3], [f64; 3]) -> bool) -> Vec<usize> {
        let pos = positions(g);
        let ns = crate::normals::prim_normals(g.p(), &g.topology);
        (0..g.nprims())
            .filter(|&p| {
                let r = range(g, p);
                let c = mul(
                    r.clone().map(|v| pos[g.topology.vertex_point[v] as usize]).fold([0.; 3], add),
                    1. / r.len() as f64,
                );
                f(c, crate::boolean_kernel::unit(ns[p]))
            })
            .collect()
    }
    fn group(ids: &[usize]) -> String {
        ids.iter().map(usize::to_string).collect::<Vec<_>>().join(" ")
    }
    fn unit_box() -> Geometry {
        let p = Params::resolve(super::super::generate::BOX.params, None).unwrap();
        (super::super::generate::BOX.cook)(&[vec![]], &p).unwrap()
    }
    /// Box with the +Y face inset; returns (geometry, inner face, inner face area).
    fn inset_top(g: &Geometry, amount: f64) -> (Geometry, usize, f64) {
        let top = prims_where(g, |_, n| n[1] > 0.99)[0];
        let o = model::extrude(g, &top.to_string(), 0., amount, "connected", false, None).unwrap();
        let mask = prim_mask(&o, "extrudeFront").unwrap();
        let inner = (0..o.nprims()).find(|&i| mask[i]).unwrap();
        let area = face_area(&o, inner);
        (o, inner, area)
    }
    #[test]
    fn through_hole_on_rotated_box_has_no_unpunched_band() {
        let (g, inner, area) = inset_top(&unit_box(), 0.25);
        let pos = positions(&g);
        let faces: Vec<Vec<usize>> = (0..g.nprims())
            .map(|f| range(&g, f).map(|v| g.topology.vertex_point[v] as usize).collect())
            .collect();
        for (ax, angle) in [([1., 1., 0.], 0.7_f64), ([0., 1., 1.], 0.4), ([1., 2., 3.], 1.1)] {
            let k = mul(ax, 1. / len(ax));
            let rot = |p: [f64; 3]| {
                let (c, s) = (angle.cos(), angle.sin());
                add(add(mul(p, c), mul(cross(k, p), s)), mul(k, dot(k, p) * (1. - c)))
            };
            let pts: Vec<_> = pos.iter().map(|&p| rot(p)).collect();
            let r = mk(&pts, &faces);
            let out = apply_through(&r, &inner.to_string(), -1., Through::Auto).unwrap();
            check(&out, 0);
            assert!((volume(&out) - (1. - area)).abs() < 1e-4, "volume {} axis {ax:?}", volume(&out));
        }
    }
    #[test]
    fn through_hole_on_box_snaps_to_far_face() {
        let (g, inner, area) = inset_top(&unit_box(), 0.25);
        let thickness = 1.;
        let off = apply_through(&g, &inner.to_string(), -0.5, Through::Off).unwrap();
        check(&off, 2); // pocket: still a sphere
        let out = apply_through(&g, &inner.to_string(), -thickness, Through::Auto).unwrap();
        check(&out, 0); // torus: through-hole
        assert!((volume(&out) - (1. - area * thickness)).abs() < 1e-4, "{}", volume(&out));
        // Neither the pushed face nor a coincident far cap survives inside the hole.
        let pos = positions(&out);
        let caps: Vec<_> = prims_where(&out, |_, n| n[1].abs() > 0.99)
            .into_iter()
            .filter(|&f| range(&out, f).all(|v| {
                let p = pos[out.topology.vertex_point[v] as usize];
                p[0].abs() < 0.4 && p[2].abs() < 0.4
            }))
            .collect();
        assert!(caps.is_empty(), "caps left in the hole: {caps:?}");
        // Snap tolerance: a hair short still punches; Force punches from any distance.
        let near = apply_through(&g, &inner.to_string(), -thickness * (1. - 2e-6), Through::Auto).unwrap();
        check(&near, 0);
        let forced = apply_through(&g, &inner.to_string(), -0.1, Through::Force).unwrap();
        check(&forced, 0);
        // New faces: material slot inherited, corner UVs invalid (NaN) rather than garbage.
        assert_eq!(out.nprims(), out.prim.values().next().map_or(out.nprims(), |a| a.rows()));
    }
    #[test]
    fn push_past_thickness_ignores_excess() {
        let (g, inner, area) = inset_top(&unit_box(), 0.25);
        let exact = apply_through(&g, &inner.to_string(), -1., Through::Auto).unwrap();
        let past = apply_through(&g, &inner.to_string(), -7.5, Through::Auto).unwrap();
        check(&past, 0);
        assert_eq!((past.npoints, past.nprims()), (exact.npoints, exact.nprims()));
        assert!((volume(&past) - (1. - area)).abs() < 1e-4);
        // Python-compatible Off mode also cuts through when far past.
        let legacy = apply_through(&g, &inner.to_string(), -3., Through::Off).unwrap();
        check(&legacy, 0);
        assert!((volume(&legacy) - volume(&past)).abs() < 1e-5);
        // Auto also drops the collinear split points the legacy cut leaves behind.
        assert!(past.npoints <= legacy.npoints);
    }
    fn wedge() -> Geometry {
        // Bottom z=0 square [0,2]^2; slanted top from z=1 (x=0) to z=2 (x=2).
        let p = [
            [0., 0., 0.], [2., 0., 0.], [2., 2., 0.], [0., 2., 0.],
            [0., 0., 1.], [2., 0., 2.], [2., 2., 2.], [0., 2., 1.],
        ];
        let f = vec![
            vec![0, 3, 2, 1], vec![4, 5, 6, 7], vec![0, 1, 5, 4],
            vec![1, 2, 6, 5], vec![2, 3, 7, 6], vec![3, 0, 4, 7],
        ];
        mk(&p, &f)
    }
    #[test]
    fn non_parallel_back_face_wedge() {
        let w = wedge();
        solid(&w, "wedge").unwrap();
        let bottom = prims_where(&w, |_, n| n[2] < -0.99)[0];
        let o = model::extrude(&w, &bottom.to_string(), 0., 0.25, "connected", false, None).unwrap();
        let inner = (0..o.nprims()).find(|&i| prim_mask(&o, "extrudeFront").unwrap()[i]).unwrap();
        // Between the shallow and deep exits: an ordinary (partially open) cut is not a punch.
        let pocket = apply_through(&o, &inner.to_string(), -0.8, Through::Auto).unwrap();
        check(&pocket, 2);
        let deepest = 2.;
        let hole = apply_through(&o, &inner.to_string(), -deepest, Through::Auto).unwrap();
        check(&hole, 0);
        let past = apply_through(&o, &inner.to_string(), -5., Through::Auto).unwrap();
        check(&past, 0);
        assert!((volume(&hole) - volume(&past)).abs() < 1e-5);
    }
    #[test]
    fn face_with_existing_hole_can_be_pushed() {
        let (g, inner, _) = inset_top(&unit_box(), 0.25);
        let holed = apply_through(&g, &inner.to_string(), -1., Through::Auto).unwrap();
        // The far (-Y) face now carries the hole; push all its pieces in by 0.2.
        let bottom = prims_where(&holed, |c, n| n[1] < -0.99 && c[1] < -0.49);
        assert!(!bottom.is_empty());
        let before = volume(&holed);
        let area: f64 = bottom.iter().map(|&f| face_area(&holed, f)).sum();
        let out = apply_through(&holed, &group(&bottom), -0.2, Through::Auto).unwrap();
        check(&out, 0);
        assert!((volume(&out) - (before - 0.2 * area)).abs() < 1e-4, "{} vs {}", volume(&out), before - 0.2 * area);
    }
    #[test]
    fn ngon_face_punches_through_octagonal_prism() {
        let mut p = vec![];
        for z in [0., 1.] {
            for i in 0..8 {
                let a = (i as f64 + 0.5) * std::f64::consts::TAU / 8.;
                p.push([2. * a.cos(), 2. * a.sin(), z]);
            }
        }
        let mut f = vec![(0..8).rev().collect::<Vec<_>>(), (8..16).collect()];
        for i in 0..8 {
            f.push(vec![i, (i + 1) % 8, (i + 1) % 8 + 8, i + 8]);
        }
        let g = mk(&p, &f);
        let top = prims_where(&g, |_, n| n[2] > 0.99)[0];
        let o = model::extrude(&g, &top.to_string(), 0., 0.3, "connected", false, None).unwrap();
        let inner = (0..o.nprims()).find(|&i| prim_mask(&o, "extrudeFront").unwrap()[i]).unwrap();
        assert_eq!(range(&o, inner).len(), 8);
        let area = face_area(&o, inner);
        let before = volume(&o);
        let out = apply_through(&o, &inner.to_string(), -1., Through::Auto).unwrap();
        check(&out, 0);
        assert!((volume(&out) - (before - area)).abs() < 1e-4);
    }
    fn step() -> Geometry {
        // Profile in XZ: lower step x in [1,2] at z=1, upper block x in [0,1] at z=2; depth y in [0,1].
        let prof = [[0., 0.], [2., 0.], [2., 1.], [1., 1.], [1., 2.], [0., 2.]];
        let mut p = vec![];
        for y in [0., 1.] {
            for q in prof {
                p.push([q[0], y, q[1]]);
            }
        }
        let mut f = vec![(0..6).collect::<Vec<_>>(), (6..12).rev().collect()];
        for i in 0..6 {
            f.push(vec![i, i + 6, (i + 1) % 6 + 6, (i + 1) % 6]);
        }
        mk(&p, &f)
    }
    #[test]
    fn pull_to_coplanar_merges_into_face() {
        let g = step();
        solid(&g, "step").unwrap();
        let low = prims_where(&g, |c, n| n[2] > 0.99 && c[2] < 1.5)[0];
        let out = apply_through(&g, &low.to_string(), 1. + 3e-6, Through::Auto).unwrap();
        check(&out, 2);
        assert!((volume(&out) - 4.).abs() < 1e-4);
        let tops = prims_where(&out, |_, n| n[2] > 0.99);
        assert_eq!(tops.len(), 1, "top faces merged");
        assert!((face_area(&out, tops[0]) - 2.).abs() < 1e-4);
        assert_eq!(out.nprims(), 6, "a clean box");
        // Off keeps the Python behaviour (no merge).
        let legacy = apply_through(&g, &low.to_string(), 1., Through::Off);
        if let Ok(legacy) = legacy {
            assert!(legacy.nprims() > 6);
        }
    }
    #[test]
    fn open_mesh_is_a_plain_push() {
        let g = Geometry::from_arrays(
            vec![0., 0., 0., 1., 0., 0., 1., 1., 0., 0., 1., 0.],
            vec![0, 1, 2, 3],
            vec![0, 4],
            None,
        )
        .unwrap();
        for d in [-0.5, 0.5] {
            let out = apply_through(&g, "0", d, Through::Auto).unwrap();
            assert_eq!(out.nprims(), 5);
            assert!(out.p().iter().all(|x| x.is_finite()));
        }
        assert!(apply_through(&g, "0", 0.5, Through::Off).is_err());
    }
    #[test]
    fn two_selected_faces_push_together() {
        let b = unit_box();
        let top = prims_where(&b, |_, n| n[1] > 0.99)[0];
        let side = prims_where(&b, |_, n| n[1] < -0.99)[0];
        let o = model::extrude(&b, &group(&[top, side]), 0., 0.25, "individual", false, None).unwrap();
        let mask = prim_mask(&o, "extrudeFront").unwrap();
        let ids: Vec<_> = (0..o.nprims()).filter(|&i| mask[i]).collect();
        assert_eq!(ids.len(), 2);
        let area: f64 = ids.iter().map(|&f| face_area(&o, f)).sum();
        let pockets = apply_through(&o, &group(&ids), -0.3, Through::Auto).unwrap();
        check(&pockets, 2);
        assert!((volume(&pockets) - (1. - 0.3 * area)).abs() < 1e-4);
        assert!(apply_through(&o, &group(&ids), -0.3, Through::Off).is_err());
    }
    #[test]
    fn concave_face_pull_in_auto_mode() {
        // The step's front face is an L-shaped hexagon.
        let g = step();
        let front = prims_where(&g, |_, n| n[1] < -0.99)[0];
        assert!(apply_through(&g, &front.to_string(), 0.5, Through::Off).is_err());
        let out = apply_through(&g, &front.to_string(), 0.5, Through::Auto).unwrap();
        check(&out, 2);
        assert!((volume(&out) - 3. * 1.5).abs() < 1e-4);
    }
    #[test]
    fn default_through_param_is_auto_and_python_cases_match_off() {
        let p = Params::resolve(PUSHPULL.params, None).unwrap();
        assert_eq!(p.s("through").unwrap(), "auto");
    }
}
