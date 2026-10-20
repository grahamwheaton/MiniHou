use super::*;
pub(super) fn instancer(
    stage: &usd::Stage,
    p: &usd::Prim,
    g: &mut Geometry,
    t: usd::TimeCode,
    m: Matrix4d,
    unit: f64,
    names: &mut Vec<String>,
    bounds: &mut Vec<Value>,
    notes: &mut Vec<String>,
) -> Result<()> {
    let targets = p
        .relationship("prototypes")
        .forwarded_targets()
        .map_err(err)?;
    if !g.point.contains_key("instance") {
        let ids = indices(p, "protoIndices", t)?;
        if ids.len() != g.npoints {
            return Err(err(format!(
                "{}: protoIndices must match positions",
                p.path()
            )));
        }
        g.point.insert(
            "instance".into(),
            Arc::new(Attr::i32(
                1,
                ids.iter().map(|&i| i + 1 + names.len() as i32).collect(),
            )),
        );
        let mut bbox = BBoxCache::new(t, [Purpose::Default, Purpose::Render]);
        for target in targets {
            let proto = stage.prim(&target).map_err(err)?;
            let bound = bbox
                .relative_bound(&proto, p)
                .ok()
                .map(|b| b.aligned_range())
                .filter(|b| !b.is_empty());
            bounds.push(
                bound
                    .map(|b| json!([b.min(), b.max()]))
                    .unwrap_or(Value::Null),
            );
            let meta = metadata(&proto)?;
            names.push(
                meta["minihou"]["blender_object"]
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| target.to_string().rsplit('/').next().unwrap_or("").into()),
            );
        }
    }
    let a = &m.0;
    let det = a[0] * (a[5] * a[10] - a[6] * a[9]) - a[1] * (a[4] * a[10] - a[6] * a[8])
        + a[2] * (a[4] * a[9] - a[5] * a[8]);
    let k = det.abs().cbrt().max(1e-12);
    if !g.point.contains_key("rot") {
        let mut qs = numbers(p, "orientationsf", t)?;
        if qs.is_empty() {
            qs = numbers(p, "orientations", t)?;
        }
        let mut rotation = m;
        for r in 0..3 {
            for c in 0..3 {
                rotation.0[r * 4 + c] /= k;
            }
        }
        let orthogonal = (0..3).all(|i| {
            (0..3).all(|j| {
                ((0..3)
                    .map(|c| rotation.0[i * 4 + c] * rotation.0[j * 4 + c])
                    .sum::<f64>()
                    - if i == j { 1. } else { 0. })
                .abs()
                    < 1e-3
            })
        });
        if !orthogonal {
            notes.push(format!(
                "{}: instance rotations ignore sheared or non-uniform parent scale",
                p.path()
            ));
            rotation = Matrix4d::IDENTITY;
        }
        let mut r = vec![];
        for i in 0..g.npoints {
            let q = if qs.len() == g.npoints * 4 {
                [qs[i * 4], qs[i * 4 + 1], qs[i * 4 + 2], qs[i * 4 + 3]]
            } else {
                [1., 0., 0., 0.]
            };
            let mat = (Matrix4d::from_quat(q) * rotation).0;
            let sy = (-mat[2]).clamp(-1., 1.);
            let y = sy.asin();
            let (x, z) = if y.cos().abs() > 1e-6 {
                (mat[6].atan2(mat[10]), mat[1].atan2(mat[0]))
            } else {
                ((-mat[9]).atan2(mat[5]), 0.)
            };
            r.extend([x, y, z].map(|v| v as f32));
        }
        g.point.insert("rot".into(), Arc::new(Attr::f32(3, r)));
    }
    if !g.point.contains_key("scale") {
        let mut sc = numbers(p, "scales", t)?;
        if sc.len() != g.npoints * 3 {
            sc = vec![1.; g.npoints * 3];
        }
        g.point.insert(
            "scale".into(),
            Arc::new(Attr::f32(
                3,
                sc.into_iter().map(|x| (x * k * unit) as f32).collect(),
            )),
        );
    }
    // MiniHou writes exact primvars. Match Python: these win over schema values.
    for (src, dst, factor) in [
        ("ids", "id", 1.),
        ("velocities", "v", unit),
        ("angularVelocities", "w", std::f64::consts::PI / 180.),
    ] {
        if !g.point.contains_key(dst) {
            if let Some(v) = value(p, src, t)? {
                if let Some(mut attr) = numeric(v, 1)? {
                    if attr.rows() == g.npoints {
                        if let AttrData::F32(v) = &mut attr.data {
                            for x in v {
                                *x = (*x as f64 * factor) as f32;
                            }
                        }
                        g.point.insert(dst.into(), Arc::new(attr));
                    }
                }
            }
        }
    }
    let invis = indices(p, "invisibleIds", t)?;
    if !invis.is_empty() {
        g.point.insert(
            "group_invisible".into(),
            Arc::new(Attr::i32(
                1,
                (0..g.npoints)
                    .map(|i| {
                        i32::from(invis.contains(&(scalar(g.point.get("id"), i, i as f64) as i32)))
                    })
                    .collect(),
            )),
        );
    }
    Ok(())
}
pub(super) fn assemble(mut parts: Vec<Part>) -> Result<Geometry> {
    if parts.is_empty() {
        return Geometry::points_only(vec![]);
    }
    // Materials use per-output name tables. Reconcile them before concatenating;
    // the general merge node deliberately refuses these rich detail attributes.
    let mut materials = Vec::<String>::new();
    let mut sources = serde_json::Map::new();
    for p in &mut parts {
        if let Some(list) = p.geo.detail.get("materials").and_then(Value::as_array) {
            let mut mapping = vec![];
            for name in list {
                let name = name.as_str().unwrap_or("").to_owned();
                let slot = materials
                    .iter()
                    .position(|n| n == &name)
                    .unwrap_or_else(|| {
                        materials.push(name);
                        materials.len() - 1
                    });
                mapping.push(slot as i32);
            }
            for (table, key) in [
                (&mut p.geo.prim, "material_index"),
                (&mut p.geo.point, "material_id"),
            ] {
                if let Some(attr) = table.get(key) {
                    let indices = (0..attr.rows())
                        .map(|i| {
                            mapping
                                .get(scalar(Some(attr), i, 0.) as usize)
                                .copied()
                                .unwrap_or(0)
                        })
                        .collect();
                    table.insert(key.into(), Arc::new(Attr::i32(1, indices)));
                }
            }
        }
        if let Some(map) = p.geo.detail.get("usd_materials").and_then(Value::as_object) {
            sources.extend(map.clone());
        }
    }
    let exact = parts.iter().all(|p| p.point_index.is_some());
    let mut maps = vec![];
    let mut n = 0;
    if exact {
        let ids: BTreeSet<i32> = parts
            .iter()
            .flat_map(|p| p.point_index.as_ref().unwrap().iter().copied())
            .collect();
        if ids.iter().any(|&i| i < 0) {
            return Err(err("negative MiniHou point index"));
        }
        let lookup: BTreeMap<_, _> = ids.into_iter().enumerate().map(|(i, k)| (k, i)).collect();
        n = lookup.len();
        for p in &parts {
            maps.push(
                p.point_index
                    .as_ref()
                    .unwrap()
                    .iter()
                    .map(|id| lookup[id])
                    .collect::<Vec<_>>(),
            );
        }
    } else {
        for p in &parts {
            maps.push((n..n + p.geo.npoints).collect());
            n += p.geo.npoints;
        }
    }
    let mut points = vec![0.; n * 3];
    let mut have = vec![false; n];
    let mut pa: BTreeMap<String, Attr> = BTreeMap::new();
    for p in &parts {
        for (name, a) in &p.geo.point {
            if name != "P" {
                pa.entry(name.clone()).or_insert_with(|| a.zeros_like(n));
            }
        }
    }
    for (part, map) in parts.iter().zip(&maps) {
        for (i, &dst) in map.iter().enumerate() {
            if !have[dst] {
                points[dst * 3..dst * 3 + 3].copy_from_slice(&part.geo.p()[i * 3..i * 3 + 3]);
                for (name, a) in &part.geo.point {
                    if let Some(out) = pa.get_mut(name) {
                        copy_row(out, dst, a, i)?;
                    }
                }
                have[dst] = true;
            }
        }
    }
    let sorted = parts
        .iter()
        .filter(|p| p.geo.nprims() > 0)
        .all(|p| p.prim_index.is_some());
    let mut prims = vec![];
    let mut base = 0;
    for (part, p) in parts.iter().enumerate() {
        for i in 0..p.geo.nprims() {
            prims.push((
                if sorted {
                    p.prim_index.as_ref().unwrap()[i]
                } else {
                    (base + i) as i32
                },
                part,
                i,
            ));
        }
        base += p.geo.nprims();
    }
    prims.sort_by_key(|p| p.0);
    let mut vp = vec![];
    let mut offs = vec![0];
    let mut closed = vec![];
    let mut qa: BTreeMap<String, Attr> = BTreeMap::new();
    let mut ca: BTreeMap<String, Attr> = BTreeMap::new();
    let nv = parts.iter().map(|p| p.geo.nvertices()).sum();
    for p in &parts {
        for (n, a) in &p.geo.prim {
            qa.entry(n.clone())
                .or_insert_with(|| a.zeros_like(prims.len()));
        }
        for (n, a) in &p.geo.vertex {
            ca.entry(n.clone()).or_insert_with(|| a.zeros_like(nv));
        }
    }
    for (dst, (_, part, i)) in prims.iter().enumerate() {
        let g = &parts[*part].geo;
        for c in g.topology.prim_offsets[*i] as usize..g.topology.prim_offsets[*i + 1] as usize {
            let d = vp.len();
            vp.push(maps[*part][g.topology.vertex_point[c] as usize] as i32);
            for (n, a) in &g.vertex {
                copy_row(ca.get_mut(n).unwrap(), d, a, c)?;
            }
        }
        offs.push(vp.len() as i32);
        closed.push(g.topology.is_closed(*i));
        for (n, a) in &g.prim {
            copy_row(qa.get_mut(n).unwrap(), dst, a, *i)?;
        }
    }
    let mut out = Geometry::from_arrays(points, vp, offs, Some(closed))?;
    out.point
        .extend(pa.into_iter().map(|(n, a)| (n, Arc::new(a))));
    out.vertex = ca.into_iter().map(|(n, a)| (n, Arc::new(a))).collect();
    out.prim = qa.into_iter().map(|(n, a)| (n, Arc::new(a))).collect();
    for p in parts {
        out.detail.extend(p.geo.detail);
    }
    if !materials.is_empty() {
        out.detail.insert("materials".into(), json!(materials));
    }
    if !sources.is_empty() {
        out.detail
            .insert("usd_materials".into(), Value::Object(sources));
    }
    out.validate()?;
    Ok(out)
}
