use super::*;
pub(super) fn crease(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = CREASE.one(inputs, 0)?;
    let mut out = (*g).clone();
    let text = p.s("group")?;
    let sharp = p.f("sharpness")? as f32;
    match p.e("domain", &["edges", "points"])?.as_str() {
        "edges" => {
            let mut a = values(&g, "creaseweight")?;
            for (x, m) in a.iter_mut().zip(edge_mask(&g, &text)?) {
                if m {
                    *x = sharp;
                }
            }
            write_values(&mut out, "creaseweight", &a, false)?;
        }
        _ => {
            let mut a = vec![0.; g.npoints];
            if let Some(old) = g.point.get("cornerweight") {
                if old.width != 1 {
                    return Err(Error::new("cornerweight must be a scalar point attribute"));
                }
                a = match &old.data {
                    AttrData::F32(v) => v.clone(),
                    AttrData::I32(v) => v.iter().map(|&x| x as f32).collect(),
                };
            }
            for (x, m) in a.iter_mut().zip(node_groups::point_mask(&g, &text)?) {
                if m {
                    *x = sharp;
                }
            }
            out.point
                .insert("cornerweight".into(), Arc::new(Attr::f32(1, a)));
        }
    }
    Ok(out)
}
pub(super) fn edge_group(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = EDGE_GROUP.one(inputs, 0)?;
    let name = p.s("name")?;
    group_name(&name)?;
    let pattern = p.s("pattern")?;
    let e = topo::edges(&g.topology, g.npoints);
    let method = p.e(
        "method",
        &[
            "pattern",
            "boundary",
            "points",
            "primitives",
            "bounding_object",
        ],
    )?;
    let mut mask = match method.as_str() {
        "pattern" => edge_mask(&g, &pattern)?,
        "boundary" => e.count.iter().map(|&x| x == 1).collect(),
        "points" => {
            let sel = node_groups::point_mask(&g, &pattern)?;
            e.points
                .chunks_exact(2)
                .map(|p| sel[p[0] as usize] && sel[p[1] as usize])
                .collect()
        }
        "primitives" => {
            let sel = prim_mask(&g, &pattern)?;
            let (pv, _, _) = corners(&g);
            let mut m = vec![false; e.len()];
            for v in 0..g.nvertices() {
                if e.of_vertex[v] >= 0 && sel[pv[v]] {
                    m[e.of_vertex[v] as usize] = true;
                }
            }
            m
        }
        _ => {
            let b = inputs
                .get(1)
                .and_then(|v| v.first())
                .filter(|b| b.npoints > 0)
                .ok_or_else(|| {
                    Error::new("edge_group: connect the bounding object to the second input")
                })?;
            let mut lo = [f64::INFINITY; 3];
            let mut hi = [f64::NEG_INFINITY; 3];
            for q in positions(b) {
                for k in 0..3 {
                    lo[k] = lo[k].min(q[k]);
                    hi[k] = hi[k].max(q[k]);
                }
            }
            let inside: Vec<_> = positions(&g)
                .iter()
                .map(|q| (0..3).all(|k| q[k] >= lo[k] - 1e-9 && q[k] <= hi[k] + 1e-9))
                .collect();
            e.points
                .chunks_exact(2)
                .map(|p| inside[p[0] as usize] && inside[p[1] as usize])
                .collect()
        }
    };
    let key = format!("edge_group_{name}");
    let old = values(&g, &key)?;
    let combine = p.e("combine", &["replace", "union", "intersect", "subtract"])?;
    for (m, x) in mask.iter_mut().zip(old) {
        *m = match combine.as_str() {
            "union" => *m || x != 0.,
            "intersect" => *m && x != 0.,
            "subtract" => x != 0. && !*m,
            _ => *m,
        };
    }
    let mut out = (*g).clone();
    write_values(
        &mut out,
        &key,
        &mask.iter().map(|&b| b as u8 as f32).collect::<Vec<_>>(),
        true,
    )?;
    Ok(out)
}
pub(super) fn promote(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = EDGE_GROUP_PROMOTE.one(inputs, 0)?;
    let m = edge_mask(&g, &p.s("pattern")?)?;
    let e = topo::edges(&g.topology, g.npoints);
    let points = p.e("to", &["points", "primitives"])? == "points";
    let mut sel = vec![false; if points { g.npoints } else { g.nprims() }];
    if points {
        for i in 0..e.len() {
            if m[i] {
                sel[e.points[2 * i] as usize] = true;
                sel[e.points[2 * i + 1] as usize] = true;
            }
        }
    } else {
        let (pv, _, _) = corners(&g);
        for v in 0..g.nvertices() {
            if e.of_vertex[v] >= 0 && m[e.of_vertex[v] as usize] {
                sel[pv[v]] = true;
            }
        }
    }
    let mut out = (*g).clone();
    write_group(
        &mut out,
        if points { "point" } else { "prim" },
        &p.s("name")?,
        &sel,
    )?;
    Ok(out)
}
