use super::*;
pub(crate) fn root(parent: &mut [usize], mut i: usize) -> usize {
    while parent[i] != i {
        parent[i] = parent[parent[i]];
        i = parent[i];
    }
    i
}
pub(super) fn dissolve(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = DISSOLVE.one(inputs, 0)?;
    let pattern = p.s("group")?;
    if pattern.trim().is_empty() {
        return Ok((*g).clone());
    }
    polygons(&g, "dissolve")?;
    let e = topo::edges(&g.topology, g.npoints);
    let chosen = edge_mask(&g, &pattern)?;
    let (pv, nx, _) = corners(&g);
    let vp = &g.topology.vertex_point;
    let fn_ = normals::prim_normals(g.p(), &g.topology);
    let mut parent: Vec<_> = (0..g.nprims()).collect();
    for eid in 0..e.len() {
        if !chosen[eid] {
            continue;
        }
        if e.count[eid] != 2 {
            return Err(Error::new("dissolve requires manifold interior edges"));
        }
        let vv: Vec<_> = (0..g.nvertices())
            .filter(|&v| e.of_vertex[v] == eid as i64)
            .collect();
        let (u, v) = (vv[0], vv[1]);
        let (a, b) = (pv[u], pv[v]);
        if a == b || vp[u] != vp[nx[v]] || vp[v] != vp[nx[u]] {
            return Err(Error::new(
                "dissolve requires distinct consistently wound faces",
            ));
        }
        if len(fn_[a]).min(len(fn_[b])) < 1e-12
            || dot(unit(fn_[a]), unit(fn_[b])) < p.f("angle")?.to_radians().cos() - 1e-9
        {
            return Err(Error::new(
                "dissolve face angle exceeds limit or face is degenerate",
            ));
        }
        if p.b("preserve_seams")? {
            for (name, attr) in &g.prim {
                if !name.starts_with("group_") && !equal_rows(attr, a, b, 0.) {
                    return Err(Error::new(format!(
                        "dissolve crosses primitive attribute seam: {name}"
                    )));
                }
            }
            for (name, attr) in &g.vertex {
                if !name.starts_with("edge_group_")
                    && name != "creaseweight"
                    && (!equal_rows(attr, u, nx[v], 1e-6) || !equal_rows(attr, v, nx[u], 1e-6))
                {
                    return Err(Error::new(format!(
                        "dissolve crosses corner attribute seam: {name}"
                    )));
                }
            }
        }
        let ra = root(&mut parent, a);
        let rb = root(&mut parent, b);
        parent[rb] = ra;
    }
    let mut regions = BTreeMap::<usize, Vec<usize>>::new();
    for i in 0..g.nprims() {
        let r = root(&mut parent, i);
        regions.entry(r).or_default().push(i);
    }
    let mut regions: Vec<_> = regions.into_values().collect();
    regions.sort_by_key(|r| r[0]);
    let mut faces = Vec::new();
    let mut sources = Vec::new();
    for r in &regions {
        let vs: Vec<_> = (0..g.nvertices())
            .filter(|&v| r.contains(&pv[v]) && !chosen[e.of_vertex[v] as usize])
            .collect();
        let f = if r.len() == 1 {
            vs
        } else {
            let mut by_start = BTreeMap::new();
            for &v in &vs {
                if by_start.insert(vp[v], v).is_some() {
                    return Err(Error::new(
                        "dissolve region has a hole, repeated corner or branching boundary",
                    ));
                }
            }
            let mut f = Vec::new();
            let mut at = vs.first().copied();
            while let Some(v) = at {
                if f.contains(&v) {
                    break;
                }
                f.push(v);
                at = by_start.get(&vp[nx[v]]).copied();
            }
            if f.len() < 3 || f.len() != vs.len() || at != f.first().copied() {
                return Err(Error::new(
                    "dissolve region must have one simple closed boundary",
                ));
            }
            f
        };
        faces.push(f);
        sources.push(r[0]);
    }
    let mut out = rebuild_edges(&g, &faces, &sources, None)?;
    for (name, a) in &g.prim {
        if name.starts_with("group_") {
            let w = a.width;
            let attr = match &a.data {
                AttrData::F32(v) => Attr::f32(
                    w,
                    regions
                        .iter()
                        .flat_map(|r| {
                            (0..w).map(move |k| {
                                r.iter()
                                    .map(|&i| v[i * w + k])
                                    .fold(f32::NEG_INFINITY, f32::max)
                            })
                        })
                        .collect(),
                ),
                AttrData::I32(v) => Attr::i32(
                    w,
                    regions
                        .iter()
                        .flat_map(|r| {
                            (0..w).map(move |k| r.iter().map(|&i| v[i * w + k]).max().unwrap_or(0))
                        })
                        .collect(),
                ),
            };
            out.prim.insert(name.clone(), Arc::new(attr));
        }
    }
    Ok(out)
}
fn equal_rows(a: &Attr, i: usize, j: usize, tol: f64) -> bool {
    let w = a.width;
    (0..w).all(|k| match &a.data {
        AttrData::F32(v) => (v[i * w + k] as f64 - v[j * w + k] as f64).abs() <= tol,
        AttrData::I32(v) => v[i * w + k] == v[j * w + k],
    })
}
