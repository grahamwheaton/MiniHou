use super::*;

pub(super) fn soft_peak(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = SOFT_PEAK.one(inputs, 0)?;
    let text = p.s("group")?;
    let sel = groups::point_mask(&g, &text)?;
    if !sel.iter().any(|&b| b) {
        return Ok((*g).clone());
    }
    let radius = p.f("radius")?;
    if radius <= 0. {
        return Err(Error::new("radius must be positive"));
    }
    let falloff = p.e("falloff", &["smooth", "linear", "constant"])?;
    let n = if let Some(a) = g.point.get("N") {
        a.as_f32()
            .filter(|_| a.width == 3)
            .ok_or_else(|| Error::new("N must have three components"))?
            .to_vec()
    } else {
        normals::point_normals(g.p(), &g.topology, g.npoints)
    };
    let pos = positions(&g);
    let sources: Vec<_> = pos
        .iter()
        .zip(&sel)
        .filter(|(_, s)| **s)
        .map(|(&q, _)| q)
        .collect();
    let mut out = (*g).clone();
    let mut pts = Vec::new();
    for (i, &q) in pos.iter().enumerate() {
        let d = if text.trim().is_empty() {
            0.
        } else {
            sources
                .iter()
                .map(|&s| len(sub(q, s)))
                .fold(f64::INFINITY, f64::min)
        };
        let u = (1. - d / radius).clamp(0., 1.);
        let w = match falloff.as_str() {
            "smooth" => u * u * (3. - 2. * u),
            "linear" => u,
            _ => (d <= radius) as u8 as f64,
        };
        for k in 0..3 {
            pts.push((q[k] + n[3 * i + k] as f64 * w * p.f("distance")?) as f32);
        }
    }
    out.set_p(pts);
    Ok(out)
}
pub(super) fn group_range(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = GROUP_RANGE.one(inputs, 0)?;
    let point = p.e("domain", &["primitives", "points"])? == "points";
    let n = if point { g.npoints } else { g.nprims() };
    let start = p.i("start")?;
    let end = p.i("end")?;
    let last = if end < 0 {
        n as i64 - 1
    } else {
        end.min(n as i64 - 1)
    };
    let select = p.i("select")?;
    let every = p.i("every")?;
    if every < 1 || select < 1 || start < 0 {
        return Err(Error::new("invalid group range"));
    }
    let offset = p.i("offset")?;
    let invert = p.b("invert")?;
    let mut mask: Vec<_> = (0..n)
        .map(|i| {
            let i = i as i64;
            (i >= start && i <= last && (i - start + offset).rem_euclid(every) < select.min(every))
                ^ invert
        })
        .collect();
    let name = p.s("name")?;
    group_name(&name)?;
    let old = if point {
        g.point.get(&format!("group_{name}"))
    } else {
        g.prim.get(&format!("group_{name}"))
    };
    let combine = p.e("combine", &["replace", "union", "intersect", "subtract"])?;
    for (i, m) in mask.iter_mut().enumerate() {
        let old = old.is_some_and(|a| match &a.data {
            AttrData::F32(v) => v[i * a.width] != 0.,
            AttrData::I32(v) => v[i * a.width] != 0,
        });
        *m = match combine.as_str() {
            "union" => old || *m,
            "intersect" => old && *m,
            "subtract" => old && !*m,
            _ => *m,
        };
    }
    let mut out = (*g).clone();
    write_group(&mut out, if point { "point" } else { "prim" }, &name, &mask)?;
    Ok(out)
}
pub(super) fn edge_collapse(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = EDGE_COLLAPSE.one(inputs, 0)?;
    let text = p.s("group")?;
    if text.trim().is_empty() {
        return Ok((*g).clone());
    }
    polygons(&g, "edge_collapse")?;
    let e = topo::edges(&g.topology, g.npoints);
    let sel = edges::edge_mask(&g, &text)?;
    if !sel.iter().any(|&b| b) {
        return Ok((*g).clone());
    }
    let mut parent: Vec<_> = (0..g.npoints).collect();
    for (i, q) in e.points.chunks_exact(2).enumerate() {
        if sel[i] {
            let a = edges::root(&mut parent, q[0] as usize);
            let b = edges::root(&mut parent, q[1] as usize);
            parent[a.max(b)] = a.min(b);
        }
    }
    let roots: Vec<_> = (0..g.npoints)
        .map(|i| edges::root(&mut parent, i))
        .collect();
    let unique: Vec<_> = roots
        .iter()
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let labels: Vec<_> = roots
        .iter()
        .map(|r| unique.binary_search(r).unwrap())
        .collect();
    let mut buckets = vec![Vec::new(); unique.len()];
    for (i, &r) in labels.iter().enumerate() {
        buckets[r].push(i);
    }
    let point = mean(&g.point, &buckets);
    let mut faces = Vec::new();
    let mut ps = Vec::new();
    for i in 0..g.nprims() {
        let r = range(&g, i);
        let mut f = Vec::new();
        for v in r.clone() {
            let next = if v + 1 < r.end { v + 1 } else { r.start };
            if labels[g.topology.vertex_point[v] as usize]
                != labels[g.topology.vertex_point[next] as usize]
                || r.len() == 1
            {
                f.push(v);
            }
        }
        if f.len() >= 3 {
            faces.push(f);
            ps.push(i);
        }
    }
    let ix: Vec<_> = faces.iter().flatten().copied().collect();
    let mut out = make(
        point,
        ix.iter()
            .map(|&v| labels[g.topology.vertex_point[v] as usize] as i32)
            .collect(),
        &faces.iter().map(Vec::len).collect::<Vec<_>>(),
        gather(&g.vertex, &ix),
        gather(&g.prim, &ps),
        g.detail.clone(),
        None,
    )?;
    outgroup(
        &mut out,
        p,
        "collapsed",
        "point",
        &buckets.iter().map(|b| b.len() > 1).collect::<Vec<_>>(),
    )?;
    Ok(out)
}
pub(super) fn edge_flip(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = EDGE_FLIP.one(inputs, 0)?;
    let text = p.s("group")?;
    if text.trim().is_empty() {
        return Ok((*g).clone());
    }
    polygons(&g, "edge_flip")?;
    let e = topo::edges(&g.topology, g.npoints);
    let sel = edges::edge_mask(&g, &text)?;
    let tw = twins(&g);
    let (pv, nx, pr) = corners(&g);
    let mut used = vec![false; g.nprims()];
    let mut replace = BTreeMap::new();
    let mut pairs = Vec::new();
    for i in 0..g.nvertices() {
        let Some(j) = tw[i] else {
            continue;
        };
        if j < i || e.of_vertex[i] < 0 || !sel[e.of_vertex[i] as usize] {
            continue;
        }
        let (a, b) = (pv[i], pv[j]);
        if a == b || range(&g, a).len() != 3 || range(&g, b).len() != 3 || used[a] || used[b] {
            continue;
        }
        used[a] = true;
        used[b] = true;
        let (c, d) = (pr[i], pr[j]);
        if g.topology.vertex_point[c] == g.topology.vertex_point[d] {
            continue;
        }
        replace.insert(a, vec![i, d, c]);
        replace.insert(b, vec![d, nx[i], c]);
        pairs.push((
            g.topology.vertex_point[c] as usize,
            g.topology.vertex_point[d] as usize,
        ));
    }
    if replace.is_empty() {
        return Ok((*g).clone());
    }
    let faces: Vec<_> = (0..g.nprims())
        .map(|i| replace.remove(&i).unwrap_or_else(|| range(&g, i).collect()))
        .collect();
    let mut out = edges::rebuild_edges(
        &g,
        &faces,
        &(0..g.nprims()).collect::<Vec<_>>(),
        g.topology.closed.clone(),
    )?;
    let mask = pair_mask(&out, &pairs);
    outgroup(&mut out, p, "flipped", "edge", &mask)?;
    Ok(out)
}
pub(super) fn edge_cusp(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = EDGE_CUSP.one(inputs, 0)?;
    polygons(&g, "edge_cusp")?;
    let text = p.s("group")?;
    let e = topo::edges(&g.topology, g.npoints);
    let hard = if text.trim().is_empty() {
        vec![false; e.len()]
    } else {
        edges::edge_mask(&g, &text)?
    };
    let fn_ = normals::prim_normals(g.p(), &g.topology);
    let un: Vec<_> = fn_.iter().map(|&v| unit32(v)).collect();
    let (pv, nx, _) = corners(&g);
    let tw = twins(&g);
    let mut parent: Vec<_> = (0..g.nvertices()).collect();
    let cos = p.f("cusp_angle")?.to_radians().cos();
    for i in 0..g.nvertices() {
        let Some(j) = tw[i] else {
            continue;
        };
        if e.of_vertex[i] < 0
            || hard[e.of_vertex[i] as usize]
            || dot(un[pv[i]], un[pv[j]]) < cos - 1e-9
        {
            continue;
        }
        for (a, b) in [(i, nx[j]), (j, nx[i])] {
            let a = edges::root(&mut parent, a);
            let b = edges::root(&mut parent, b);
            parent[a.max(b)] = a.min(b);
        }
    }
    let labels: Vec<_> = (0..g.nvertices())
        .map(|i| edges::root(&mut parent, i))
        .collect();
    let mut acc = vec![[0.; 3]; g.nvertices()];
    for v in 0..g.nvertices() {
        acc[labels[v]] = add(acc[labels[v]], fn_[pv[v]]);
    }
    let mut out = (*g).clone();
    out.vertex.insert(
        "N".into(),
        Arc::new(Attr::f32(
            3,
            labels
                .iter()
                .flat_map(|&i| unit32(acc[i]).map(|x| x as f32))
                .collect(),
        )),
    );
    out.point.remove("N");
    out.prim.remove("N");
    Ok(out)
}
