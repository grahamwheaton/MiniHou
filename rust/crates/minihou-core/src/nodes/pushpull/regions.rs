use super::*;

pub(super) fn solid(g: &Geometry, label: &str) -> Result<()> {
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
/// Selected faces that are edge-connected and coplanar are pushed as one region, so
/// a face split around a hole (or several adjacent faces) gives one closed prism.
pub(super) struct Region {
    pub(super) prims: Vec<usize>,
    /// Point ids of each face, in face winding order.
    pub(super) loops: Vec<Vec<usize>>,
    /// Region-boundary half-edges (point ids) and the face that owns each.
    pub(super) boundary: Vec<(usize, usize, usize)>,
    pub(super) n: [f64; 3],
}
pub(super) fn face_normal(
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
pub(super) fn regions(
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
pub(super) fn extent_of(pos: &[[f64; 3]]) -> f64 {
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
