use super::*;

pub(crate) fn ear_clip(xy: &[[f64; 2]]) -> Vec<[usize; 3]> {
    let n = xy.len();
    if n < 3 {
        return Vec::new();
    }
    let sign = if (0..n)
        .map(|i| xy[i][0] * xy[(i + 1) % n][1] - xy[(i + 1) % n][0] * xy[i][1])
        .sum::<f64>()
        >= 0.
    {
        1.
    } else {
        -1.
    };
    let turn = |a: usize, b: usize, c: usize| {
        ((xy[b][0] - xy[a][0]) * (xy[c][1] - xy[a][1])
            - (xy[b][1] - xy[a][1]) * (xy[c][0] - xy[a][0]))
            * sign
    };
    let mut ix: Vec<_> = (0..n).collect();
    let mut out = Vec::new();
    for _ in 0..10 * n {
        if ix.len() <= 3 {
            break;
        }
        let mut found = false;
        for k in 0..ix.len() {
            let a = ix[(k + ix.len() - 1) % ix.len()];
            let b = ix[k];
            let c = ix[(k + 1) % ix.len()];
            if turn(a, b, c) <= 1e-12 {
                continue;
            }
            if ix.iter().any(|&j| {
                j != a
                    && j != b
                    && j != c
                    && turn(a, b, j) >= 0.
                    && turn(b, c, j) >= 0.
                    && turn(c, a, j) >= 0.
            }) {
                continue;
            }
            out.push([a, b, c]);
            ix.remove(k);
            found = true;
            break;
        }
        if !found {
            break;
        }
    }
    for k in 1..ix.len() - 1 {
        out.push([ix[0], ix[k], ix[k + 1]]);
    }
    out
}
fn triangles(g: &Geometry, p: usize, pos: &[[f64; 3]], fn_: [f64; 3]) -> Vec<Vec<usize>> {
    let r = range(g, p);
    let q: Vec<_> = r
        .clone()
        .map(|v| pos[g.topology.vertex_point[v] as usize])
        .collect();
    if q.len() < 3 {
        return Vec::new();
    }
    let n = q.len();
    let concave = (0..n).any(|i| {
        dot(
            cross(sub(q[i], q[(i + n - 1) % n]), sub(q[(i + 1) % n], q[i])),
            fn_,
        ) < -1e-12
    });
    if !concave {
        return (1..n - 1)
            .map(|k| vec![r.start, r.start + k, r.start + k + 1])
            .collect();
    }
    let normal = mul(fn_, 1.0 / len(fn_).max(1e-30));
    let u = unit(cross(
        normal,
        if normal[0].abs() < 0.9 {
            [1., 0., 0.]
        } else {
            [0., 1., 0.]
        },
    ));
    let w = cross(normal, u);
    let xy: Vec<_> = q.iter().map(|&q| [dot(q, u), dot(q, w)]).collect();
    ear_clip(&xy)
        .iter()
        .map(|t| t.iter().map(|&i| r.start + i).collect())
        .collect()
}
pub(crate) fn divide(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = DIVIDE.one(inputs, 0)?;
    let s = prim_mask(&g, &p.s("group")?)?;
    let pos = positions(&g);
    let face_normals = normals::prim_normals(g.p(), &g.topology);
    let mut faces = Vec::new();
    let mut src = Vec::new();
    let mut closed = Vec::new();
    for i in 0..g.nprims() {
        let f = if s[i] && range(&g, i).len() > 3 && g.topology.is_closed(i) {
            triangles(&g, i, &pos, face_normals[i])
        } else {
            vec![range(&g, i).collect()]
        };
        for f in f {
            faces.push(f);
            src.push(i);
            closed.push(g.topology.is_closed(i));
        }
    }
    rebuild(
        &g,
        &faces,
        &src,
        if g.topology.has_open() {
            Some(closed)
        } else {
            None
        },
    )
}

pub(crate) fn clip(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = CLIP.one(inputs, 0)?;
    polygons(&g, "clip")?;
    let raw = p.v3("normal")?;
    let n = mul(raw, 1. / len(raw).max(1e-12));
    let o = p.v3("origin")?;
    let mut d: Vec<_> = positions(&g).iter().map(|&q| dot(sub(q, o), n)).collect();
    let eps = 1e-7 * d.iter().map(|d| d.abs()).fold(1.0, f64::max);
    for x in &mut d {
        if x.abs() < eps {
            *x = 0.;
        }
    }
    let e = topo::edges(&g.topology, g.npoints);
    let (_, nx, _) = corners(&g);
    let vp = &g.topology.vertex_point;
    let mut xindex = vec![0; e.len()];
    let mut ia = Vec::new();
    let mut ib = Vec::new();
    let mut fr = Vec::new();
    for i in 0..e.len() {
        let a = e.points[2 * i] as usize;
        let b = e.points[2 * i + 1] as usize;
        if d[a] * d[b] < 0. {
            xindex[i] = g.npoints + ia.len();
            ia.push(a);
            ib.push(b);
            fr.push(d[a] / (d[a] - d[b]));
        }
    }
    let mut pts = concat(&g.point, &lerp(&g.point, &ia, &ib, &fr)); // P uses a + (b-a)*t rather than B.lerp's a*(1-t)+b*t.
    let mut newp = g.p().to_vec();
    for ((&a, &b), &t) in ia.iter().zip(&ib).zip(&fr) {
        for k in 0..3 {
            newp.push(g.p()[3 * a + k] + (g.p()[3 * b + k] - g.p()[3 * a + k]) * t as f32);
        }
    }
    pts.insert("P".into(), Arc::new(Attr::f32(3, newp)));
    let keep = p.e("keep", &["above", "below", "both"])?;
    let sides = match keep.as_str() {
        "above" => vec![1.],
        "below" => vec![-1.],
        _ => vec![1., -1.],
    };
    let mut vertices = Vec::new();
    let mut va = Vec::new();
    let mut vb = Vec::new();
    let mut vf = Vec::new();
    let mut sizes = Vec::new();
    let mut src = Vec::new();
    let mut above = Vec::new();
    let mut cut = Vec::new();
    for s in sides {
        for i in 0..g.nprims() {
            let mut entries = Vec::new();
            let mut was_cut = false;
            for v in range(&g, i) {
                let a = vp[v] as usize;
                let b = vp[nx[v]] as usize;
                if s * d[a] >= 0. {
                    entries.push((a, v, 0.));
                }
                if d[a] * d[b] < 0. {
                    entries.push((xindex[e.of_vertex[v] as usize], v, d[a] / (d[a] - d[b])));
                    was_cut = true;
                }
            }
            if entries.len() < 3 {
                continue;
            }
            sizes.push(entries.len());
            src.push(i);
            above.push(s > 0.);
            cut.push(was_cut);
            for (q, v, f) in entries {
                vertices.push(q as i32);
                va.push(v);
                vb.push(nx[v]);
                vf.push(f);
            }
        }
    }
    let mut out = make(
        pts,
        vertices,
        &sizes,
        lerp(&g.vertex, &va, &vb, &vf),
        gather(&g.prim, &src),
        g.detail.clone(),
        None,
    )?;
    outgroup(&mut out, p, "above", "prim", &above)?;
    outgroup(
        &mut out,
        p,
        "below",
        "prim",
        &above.iter().map(|b| !b).collect::<Vec<_>>(),
    )?;
    outgroup(&mut out, p, "cutpolys", "prim", &cut)?;
    let on: Vec<_> = d
        .iter()
        .map(|&x| x == 0.)
        .chain(std::iter::repeat_n(true, ia.len()))
        .collect();
    outgroup(&mut out, p, "cutpoints", "point", &on)?;
    let used: BTreeSet<_> = out
        .topology
        .vertex_point
        .iter()
        .map(|&x| x as usize)
        .collect();
    let on: Vec<_> = used.iter().map(|&i| on[i]).collect();
    out = compact(out)?;
    let ed = topo::edges(&out.topology, out.npoints);
    let mask = ed
        .points
        .chunks_exact(2)
        .map(|q| on[q[0] as usize] && on[q[1] as usize])
        .collect::<Vec<_>>();
    outgroup(&mut out, p, "cutedges", "edge", &mask)?;
    Ok(out)
}
