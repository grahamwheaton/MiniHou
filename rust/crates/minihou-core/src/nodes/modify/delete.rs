use super::*;

pub(super) fn delete_node(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = DELETE.one(inputs, 0)?;
    let expr = p.s("expr")?;
    let invert = p.b("invert")?;

    if geo.npoints == 0 {
        return Ok((*geo).clone());
    }

    let mut mask = delete_mask(&geo, &expr)?;
    if invert {
        for m in &mut mask {
            *m = !*m;
        }
    }

    if !mask.iter().any(|&m| m) {
        return Ok((*geo).clone());
    }

    let keep: Vec<bool> = mask.iter().map(|&m| !m).collect();
    keep_points(&geo, &keep)
}

fn delete_mask(geo: &Geometry, expr: &str) -> Result<Vec<bool>> {
    if expr.contains(';') || expr.contains('\n') {
        return Err(Error::new("the delete expression must be a single expression, e.g. @P.z < 0"));
    }
    let text = expr.trim();
    if text.is_empty() || text == "0" {
        return Ok(vec![false; geo.npoints]);
    }
    if text == "1" {
        return Ok(vec![true; geo.npoints]);
    }
    let trimmed = text.trim_start_matches('(').trim_end_matches(')').trim();
    if let Some(axis) = trimmed.strip_prefix("@P.") {
        let (k, rest) = match axis.chars().next() {
            Some('x') => (0, &axis[1..]),
            Some('y') => (1, &axis[1..]),
            Some('z') => (2, &axis[1..]),
            _ => return Err(Error::new("delete expression is not supported by the Rust backend")),
        };
        let rest = rest.trim();
        let (op, rhs) = [">=", "<=", "==", "!=", ">", "<"].into_iter()
            .find_map(|op| rest.strip_prefix(op).map(|rhs| (op, rhs.trim())))
            .ok_or_else(|| Error::new("delete expression is not supported by the Rust backend"))?;
        let rhs = rhs.trim_matches(|c| c == '(' || c == ')').trim();
        let value: f32 = rhs.parse().map_err(|_| Error::new("delete expression is not supported by the Rust backend"))?;
        return Ok(geo.p().as_chunks::<3>().0.iter().map(|q| match op {
            ">=" => q[k] >= value,
            "<=" => q[k] <= value,
            "==" => q[k] == value,
            "!=" => q[k] != value,
            ">" => q[k] > value,
            _ => q[k] < value,
        }).collect());
    }
    Err(Error::new("delete expression is not supported by the Rust backend"))
}

pub(in crate::nodes) fn keep_points(geo: &Geometry, keep: &[bool]) -> Result<Geometry> {
    let npoints = geo.npoints;
    let topo = &geo.topology;
    let nverts = topo.nvertices();
    let nprims = topo.nprims();
    let closed = topo.closed_mask();

    let mut remap = vec![-1i32; npoints];
    let mut new_point_count = 0;
    for (i, &k) in keep.iter().enumerate() {
        if k {
            remap[i] = new_point_count as i32;
            new_point_count += 1;
        }
    }


    let vkeep_pt: Vec<bool> = topo.vertex_point.iter().map(|&v| keep[v as usize]).collect();

    let mut dropped_per_prim = vec![0usize; nprims];
    let mut vert_prim = Vec::with_capacity(nverts);
    for (p, drop_count) in dropped_per_prim.iter_mut().enumerate() {
        let start = topo.prim_offsets[p] as usize;
        let end = topo.prim_offsets[p + 1] as usize;
        vert_prim.extend(std::iter::repeat_n(p, end - start));
        for &vk in &vkeep_pt[start..end] {
            if !vk {
                *drop_count += 1;
            }
        }
    }

    let mut vkeep = vec![false; nverts];
    for i in 0..nverts {
        let p = vert_prim[i];
        vkeep[i] = if closed[p] {
            dropped_per_prim[p] == 0
        } else {
            vkeep_pt[i]
        };
    }

    let mut first = vec![false; nverts];
    for p in 0..nprims {
        let start = topo.prim_offsets[p] as usize;
        let end = topo.prim_offsets[p + 1] as usize;
        if end > start {
            first[start] = true;
        }
    }

    let mut starts_run = vec![false; nverts];
    for i in 0..nverts {
        if vkeep[i] {
            let is_first = first[i] || i == 0 || !vkeep[i - 1];
            starts_run[i] = is_first;
        }
    }

    let mut run = vec![-1i32; nverts];
    let mut run_count = 0;
    for i in 0..nverts {
        if starts_run[i] {
            run_count += 1;
        }
        if vkeep[i] {
            run[i] = (run_count - 1) as i32;
        }
    }

    let mut run_size = vec![0i32; run_count];
    for i in 0..nverts {
        if vkeep[i] {
            run_size[run[i] as usize] += 1;
        }
    }

    let mut run_prim = Vec::with_capacity(run_count);
    for i in 0..nverts {
        if starts_run[i] {
            run_prim.push(vert_prim[i]);
        }
    }

    let mut good = vec![false; run_count];
    for r in 0..run_count {
        let p = run_prim[r];
        good[r] = if closed[p] {
            run_size[r] > 0
        } else {
            run_size[r] >= 2
        };
    }

    for i in 0..nverts {
        if vkeep[i] {
            let r = run[i] as usize;
            vkeep[i] = good[r];
        }
    }

    let new_vp: Vec<i32> = vkeep.iter()
        .zip(topo.vertex_point.iter())
        .filter_map(|(&k, &v)| k.then_some(remap[v as usize]))
        .collect();

    let mut new_offs = Vec::with_capacity(run_count + 1);
    new_offs.push(0);
    let mut acc = 0;
    for r in 0..run_count {
        if good[r] {
            acc += run_size[r];
            new_offs.push(acc);
        }
    }

    let src_prim: Vec<usize> = good.iter().enumerate().filter_map(|(r, &g)| g.then_some(run_prim[r])).collect();

    let new_closed = if topo.closed.is_none() {
        None
    } else {
        Some(src_prim.iter().map(|&p| closed[p]).collect())
    };

    Ok(Geometry { packed: Default::default(), edge: Default::default(),
        npoints: new_point_count,
        topology: Arc::new(Topology {
            vertex_point: new_vp,
            prim_offsets: new_offs,
            closed: new_closed,
        }),
        point: slice_table_mask(&geo.point, keep),
        vertex: slice_table_mask(&geo.vertex, &vkeep),
        prim: slice_table_indices(&geo.prim, &src_prim),
        detail: geo.detail.clone(),
    })
}

fn slice_table_mask(table: &AttrTable, mask: &[bool]) -> AttrTable {
    let mut out = AttrTable::new();
    for (name, attr) in table {
        let w = attr.width;
        let data = match &attr.data {
            AttrData::F32(v) => {
                let mut d = Vec::new();
                for (i, &m) in mask.iter().enumerate() {
                    if m {
                        d.extend_from_slice(&v[i * w..(i + 1) * w]);
                    }
                }
                AttrData::F32(d)
            }
            AttrData::I32(v) => {
                let mut d = Vec::new();
                for (i, &m) in mask.iter().enumerate() {
                    if m {
                        d.extend_from_slice(&v[i * w..(i + 1) * w]);
                    }
                }
                AttrData::I32(d)
            }
        };
        out.insert(name.clone(), Arc::new(Attr { width: w, data }));
    }
    out
}

fn slice_table_indices(table: &AttrTable, indices: &[usize]) -> AttrTable {
    let mut out = AttrTable::new();
    for (name, attr) in table {
        let w = attr.width;
        let data = match &attr.data {
            AttrData::F32(v) => {
                let mut d = Vec::with_capacity(indices.len() * w);
                for &idx in indices {
                    d.extend_from_slice(&v[idx * w..(idx + 1) * w]);
                }
                AttrData::F32(d)
            }
            AttrData::I32(v) => {
                let mut d = Vec::with_capacity(indices.len() * w);
                for &idx in indices {
                    d.extend_from_slice(&v[idx * w..(idx + 1) * w]);
                }
                AttrData::I32(d)
            }
        };
        out.insert(name.clone(), Arc::new(Attr { width: w, data }));
    }
    out
}
