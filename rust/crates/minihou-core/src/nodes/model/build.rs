use super::*;

pub(crate) fn positions(g: &Geometry) -> Vec<[f64; 3]> {
    g.p()
        .chunks_exact(3)
        .map(|p| [p[0] as f64, p[1] as f64, p[2] as f64])
        .collect()
}
pub(crate) fn range(g: &Geometry, p: usize) -> std::ops::Range<usize> {
    g.topology.prim_offsets[p] as usize..g.topology.prim_offsets[p + 1] as usize
}
pub(crate) fn corners(g: &Geometry) -> (Vec<usize>, Vec<usize>, Vec<usize>) {
    let mut pv = vec![0; g.nvertices()];
    let mut nx = pv.clone();
    let mut pr = pv.clone();
    for p in 0..g.nprims() {
        let r = range(g, p);
        for v in r.clone() {
            pv[v] = p;
            nx[v] = if v + 1 < r.end { v + 1 } else { r.start };
            pr[v] = if v > r.start { v - 1 } else { r.end - 1 };
        }
    }
    (pv, nx, pr)
}
pub(crate) fn polygons(g: &Geometry, what: &str) -> Result<()> {
    if g.topology.has_open() {
        return Err(Error::new(format!(
            "{what} works on polygons, but the input has open curves; remove them first"
        )));
    }
    Ok(())
}
pub(crate) fn gather(t: &AttrTable, ix: &[usize]) -> AttrTable {
    t.iter()
        .map(|(n, a)| {
            let w = a.width;
            let b = match &a.data {
                AttrData::F32(v) => Attr::f32(
                    w,
                    ix.iter()
                        .flat_map(|&i| v[i * w..(i + 1) * w].iter().copied())
                        .collect(),
                ),
                AttrData::I32(v) => Attr::i32(
                    w,
                    ix.iter()
                        .flat_map(|&i| v[i * w..(i + 1) * w].iter().copied())
                        .collect(),
                ),
            };
            (n.clone(), Arc::new(b))
        })
        .collect()
}

/// A merged mesh has a corner UV set if any input does. Materialize point UV
/// fallback on the other inputs before concatenating their vertex tables.
pub(crate) fn promote_corner_uv_sets(geos: &mut [Geometry]) {
    let names: BTreeSet<_> = geos.iter().flat_map(|g| g.vertex.iter())
        .filter(|(name, a)| !matches!(name.as_str(), "N" | "Cd" | "P") && matches!(a.width, 2 | 3) && a.as_f32().is_some())
        .map(|(name, _)| name.clone()).collect();
    for g in geos {
        for name in &names {
            if g.vertex.contains_key(name) { continue; }
            if let Some((width, values)) = g.point.get(name).filter(|a| matches!(a.width, 2 | 3))
                .and_then(|a| a.as_f32().map(|v| (a.width, v))) {
                let data = g.topology.vertex_point.iter().flat_map(|&p| values[p as usize * width..(p as usize + 1) * width].iter().copied()).collect();
                g.vertex.insert(name.clone(), Arc::new(Attr::f32(width, data)));
            }
        }
    }
}
pub(crate) fn concat(a: &AttrTable, b: &AttrTable) -> AttrTable {
    a.iter()
        .filter_map(|(n, x)| {
            let y = b.get(n)?;
            let data = match (&x.data, &y.data) {
                (AttrData::F32(v), AttrData::F32(w)) => {
                    AttrData::F32(v.iter().chain(w).copied().collect())
                }
                (AttrData::I32(v), AttrData::I32(w)) => {
                    AttrData::I32(v.iter().chain(w).copied().collect())
                }
                _ => return None,
            };
            Some((
                n.clone(),
                Arc::new(Attr {
                    width: x.width,
                    data,
                }),
            ))
        })
        .collect()
}
pub(crate) fn lerp(t: &AttrTable, ia: &[usize], ib: &[usize], fr: &[f64]) -> AttrTable {
    t.iter()
        .map(|(n, a)| {
            let w = a.width;
            let b = match &a.data {
                AttrData::F32(v) => Attr::f32(
                    w,
                    ia.iter()
                        .zip(ib)
                        .zip(fr)
                        .flat_map(|((&i, &j), &f)| {
                            let f = f as f32;
                            (0..w).map(move |k| v[i * w + k] * (1.0 - f) + v[j * w + k] * f)
                        })
                        .collect(),
                ),
                AttrData::I32(v) => Attr::i32(
                    w,
                    ia.iter()
                        .flat_map(|&i| v[i * w..(i + 1) * w].iter().copied())
                        .collect(),
                ),
            };
            (n.clone(), Arc::new(b))
        })
        .collect()
}
pub(crate) fn mean(t: &AttrTable, buckets: &[Vec<usize>]) -> AttrTable {
    t.iter()
        .map(|(n, a)| {
            let w = a.width;
            let b = match &a.data {
                AttrData::F32(v) => Attr::f32(
                    w,
                    buckets
                        .iter()
                        .flat_map(|ix| {
                            (0..w).map(move |k| {
                                (ix.iter().map(|&i| v[i * w + k] as f64).sum::<f64>()
                                    / ix.len().max(1) as f64) as f32
                            })
                        })
                        .collect(),
                ),
                AttrData::I32(v) => Attr::i32(
                    w,
                    buckets
                        .iter()
                        .flat_map(|ix| {
                            (0..w).map(move |k| ix.iter().map(|&i| v[i * w + k]).min().unwrap_or(0))
                        })
                        .collect(),
                ),
            };
            (n.clone(), Arc::new(b))
        })
        .collect()
}
pub(crate) fn make(
    point: AttrTable,
    vp: Vec<i32>,
    sizes: &[usize],
    vertex: AttrTable,
    prim: AttrTable,
    detail: serde_json::Map<String, serde_json::Value>,
    closed: Option<Vec<bool>>,
) -> Result<Geometry> {
    let mut offsets = vec![0];
    for &s in sizes {
        let next = offsets.last().copied().unwrap() as usize + s;
        if next > i32::MAX as usize {
            return Err(Error::new("too many vertices"));
        }
        offsets.push(next as i32);
    }
    let npoints = point.get("P").map_or(0, |a| a.rows());
    let g = Geometry { packed: Default::default(), edge: Default::default(),
        npoints,
        topology: Arc::new(Topology {
            vertex_point: vp,
            prim_offsets: offsets,
            closed,
        ..Default::default()}),
        point,
        vertex,
        prim,
        detail,
    };
    g.validate()?;
    Ok(g)
}
pub(crate) fn rebuild(
    g: &Geometry,
    faces: &[Vec<usize>],
    sources: &[usize],
    closed: Option<Vec<bool>>,
) -> Result<Geometry> {
    let ix: Vec<_> = faces.iter().flatten().copied().collect();
    make(
        g.point.clone(),
        ix.iter().map(|&v| g.topology.vertex_point[v]).collect(),
        &faces.iter().map(Vec::len).collect::<Vec<_>>(),
        gather(&g.vertex, &ix),
        gather(&g.prim, sources),
        g.detail.clone(),
        closed,
    )
}
pub(crate) fn compact(g: Geometry) -> Result<Geometry> {
    let used: BTreeSet<_> = g
        .topology
        .vertex_point
        .iter()
        .map(|&x| x as usize)
        .collect();
    if used.len() == g.npoints {
        return Ok(g);
    }
    let ix: Vec<_> = used.into_iter().collect();
    let mut map = vec![0; g.npoints];
    for (i, &p) in ix.iter().enumerate() {
        map[p] = i as i32;
    }
    let mut out = g.clone();
    out.npoints = ix.len();
    out.point = gather(&g.point, &ix);
    let mut t = (*g.topology).clone();
    for v in &mut t.vertex_point {
        *v = map[*v as usize];
    }
    out.topology = Arc::new(t);
    Ok(out)
}
pub(crate) fn prim_mask(g: &Geometry, text: &str) -> Result<Vec<bool>> {
    // Reuse the core selection parser on primitive rows, including conversion of point groups by all corners.
    let mut proxy = Geometry::points_only(vec![0.0; g.nprims() * 3])?;
    proxy.point = g.prim.clone();
    let pos = positions(g);
    let mut cent = Vec::new();
    for p in 0..g.nprims() {
        let r = range(g, p);
        let mut c = [0.; 3];
        for v in r.clone() {
            c = add(c, pos[g.topology.vertex_point[v] as usize]);
        }
        cent.extend(mul(c, 1.0 / r.len().max(1) as f64).map(|x| x as f32));
    }
    proxy.set_p(cent);
    for (n, a) in &g.point {
        if !n.starts_with("group_") || proxy.point.contains_key(n) {
            continue;
        }
        let mut data = vec![0; g.nprims()];
        for p in 0..g.nprims() {
            data[p] = (!range(g, p).is_empty()
                && range(g, p).all(|v| match &a.data {
                    AttrData::F32(x) => x[g.topology.vertex_point[v] as usize * a.width] != 0.,
                    AttrData::I32(x) => x[g.topology.vertex_point[v] as usize * a.width] != 0,
                })) as i32;
        }
        proxy.point.insert(n.clone(), Arc::new(Attr::i32(1, data)));
    }
    groups::point_mask(&proxy, text)
}
pub(crate) fn group_name(name: &str) -> Result<()> {
    if name.is_empty()
        || !name.chars().next().unwrap().is_ascii_alphabetic() && !name.starts_with('_')
        || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Err(Error::new("invalid group name"));
    }
    Ok(())
}
pub(crate) fn write_group(g: &mut Geometry, domain: &str, name: &str, mask: &[bool]) -> Result<()> {
    group_name(name)?;
    let a = Arc::new(Attr::i32(1, mask.iter().map(|&v| v as i32).collect()));
    if domain == "point" {
        g.point.insert(format!("group_{name}"), a);
    } else {
        g.prim.insert(format!("group_{name}"), a);
    }
    Ok(())
}
pub(crate) fn outgroup(
    g: &mut Geometry,
    p: &Params,
    key: &str,
    domain: &str,
    mask: &[bool],
) -> Result<()> {
    if p.b(&format!("outgrp_{key}"))? {
        let name = p.s(&format!("outgrp_{key}_name"))?;
        if !name.trim().is_empty() {
            if domain == "edge" {
                group_name(name.trim())?;
                super::super::edge_tools::write_values(
                    g,
                    &format!("edge_group_{}", name.trim()),
                    &mask.iter().map(|&x| x as u8 as f32).collect::<Vec<_>>(),
                    true,
                )?;
            } else {
                write_group(g, domain, name.trim(), mask)?;
            }
        }
    }
    Ok(())
}

