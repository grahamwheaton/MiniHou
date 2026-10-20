use super::*;

pub(super) const TAG: &str = "__pushpull_new";
/// Points strictly inside the face (its triangles' centroids and pulled-in corners).
pub(super) fn face_samples(
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
pub(super) fn exit_depth(
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
pub(super) fn coplanar_target(
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
pub(super) fn cut(
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
pub(super) fn drop_collinear(g: &Geometry, tol: f64) -> Result<Geometry> {
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
pub(super) fn merge_coplanar(g: &Geometry, tol: f64) -> Result<Geometry> {
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
/// Blender-style auto-merge: points created by the inset (index >= `first_new`) that land on another point are welded to it,
/// repeated corners are dropped, and faces left with fewer than 3 corners disappear.
pub fn auto_merge(g: &Geometry, first_new: usize) -> Result<Geometry> {
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
