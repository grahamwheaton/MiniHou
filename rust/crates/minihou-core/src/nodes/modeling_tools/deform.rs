use super::*;

fn direction(v: [f64; 3]) -> Result<[f64; 3]> {
    if len(v) < 1e-12 {
        return Err(Error::new("direction must not be the zero vector"));
    }
    Ok(unit(v))
}
pub(super) fn twist(i: &Inputs, p: &Params) -> Result<Geometry> {
    let g = TWIST.one(i, 0)?;
    let d = direction(p.v3("direction")?)?;
    let o = p.v3("origin")?;
    let strength = p.f("strength")?.to_radians();
    apply(&g, &p.s("group")?, |q| {
        let r = sub(q, o);
        let t = dot(r, d);
        let (s, c) = (strength * t).sin_cos();
        add(
            o,
            add(add(mul(r, c), mul(cross(d, r), s)), mul(d, t * (1. - c))),
        )
    })
}
pub(super) fn taper(i: &Inputs, p: &Params) -> Result<Geometry> {
    let g = LINEAR_TAPER.one(i, 0)?;
    let d = direction(p.v3("direction")?)?;
    let up = p.v3("up")?;
    let mut y = sub(up, mul(d, dot(up, d)));
    if len(y) < 1e-9 {
        y = cross(
            d,
            if d[0].abs() < 0.9 {
                [1., 0., 0.]
            } else {
                [0., 1., 0.]
            },
        );
    }
    y = unit(y);
    let x = cross(y, d);
    let o = p.v3("origin")?;
    let length = p.f("length")?;
    let dropoff = p.f("dropoff")?;
    if length < 1e-8 || dropoff < 0.01 {
        return Err(Error::new("invalid taper length or dropoff"));
    }
    let symmetric = p.b("symmetric")?;
    let tx = p.f("taper_x")?;
    let ty = p.f("taper_y")?;
    apply(&g, &p.s("group")?, |q| {
        let r = sub(q, o);
        let t = dot(r, d);
        let u = ((if symmetric { t.abs() } else { t }) / length)
            .max(0.)
            .powf(dropoff);
        add(
            o,
            add(
                add(
                    mul(x, dot(r, x) * (1. + (tx - 1.) * u)),
                    mul(y, dot(r, y) * (1. + (ty - 1.) * u)),
                ),
                mul(d, t),
            ),
        )
    })
}
pub(super) fn squash(i: &Inputs, p: &Params) -> Result<Geometry> {
    let g = SQUASH_STRETCH.one(i, 0)?;
    let d = direction(p.v3("direction")?)?;
    let c = p.v3("center")?;
    let amount = p.f("amount")?;
    if amount < 0.01 {
        return Err(Error::new("amount must be at least 0.01"));
    }
    let side = if p.b("preserve_volume")? {
        1. / amount.sqrt()
    } else {
        1.
    };
    apply(&g, &p.s("group")?, |q| {
        let r = sub(q, c);
        let a = mul(d, dot(r, d));
        add(c, add(mul(a, amount), mul(sub(r, a), side)))
    })
}
fn normal_map(j: [[f64; 3]; 3], n: [f64; 3]) -> [f64; 3] {
    let transpose: [[f64; 3]; 3] = std::array::from_fn(|i| std::array::from_fn(|k| j[k][i]));
    unit32(least_squares(&transpose, &n, 1e-15))
}

fn apply<F: Fn([f64; 3]) -> [f64; 3]>(g: &Geometry, text: &str, map: F) -> Result<Geometry> {
    let pos = positions(g);
    if pos.iter().flatten().any(|x| !x.is_finite()) {
        return Err(Error::new("deformation requires finite positions"));
    }
    let sel = groups::point_mask(g, text)?;
    if !sel.iter().any(|&b| b) {
        return Ok(g.clone());
    }
    let mut out = g.clone();
    let mut newp = Vec::new();
    for (i, &q) in pos.iter().enumerate() {
        let q = if sel[i] { map(q) } else { q };
        if q.iter().any(|x| !x.is_finite()) {
            return Err(Error::new("deformation produced nonfinite positions"));
        }
        newp.extend(q.map(|x| x as f32));
    }
    out.set_p(newp);
    if ![&g.point, &g.vertex, &g.prim]
        .iter()
        .any(|t| t.contains_key("N"))
    {
        return Ok(out);
    }
    let mut extent = 1.0f64;
    for k in 0..3 {
        let lo = pos.iter().map(|q| q[k]).fold(f64::INFINITY, f64::min);
        let hi = pos.iter().map(|q| q[k]).fold(f64::NEG_INFINITY, f64::max);
        extent = extent.max(hi - lo);
    }
    let h = extent * 1e-5;
    let mut jac = vec![[[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]]; g.npoints];
    for (i, &q) in pos.iter().enumerate() {
        if sel[i] {
            for k in 0..3 {
                let mut a = q;
                let mut b = q;
                a[k] += h;
                b[k] -= h;
                let col = mul(sub(map(a), map(b)), 1. / (2. * h));
                for j in 0..3 {
                    jac[i][j][k] = col[j];
                }
            }
        }
    }
    for domain in ["point", "vertex", "prim"] {
        let table = g.table(domain);
        let Some(a) = table.get("N") else {
            continue;
        };
        let data = a
            .as_f32()
            .filter(|_| a.width == 3)
            .ok_or_else(|| Error::new("N must have three components"))?;
        let mut result = data.to_vec();
        for i in 0..a.rows() {
            let (j, active) = match domain {
                "point" => (jac[i], sel[i]),
                "vertex" => {
                    let q = g.topology.vertex_point[i] as usize;
                    (jac[q], sel[q])
                }
                _ => {
                    let mut j = [[0.; 3]; 3];
                    let mut active = false;
                    let r = range(g, i);
                    for v in r.clone() {
                        let q = g.topology.vertex_point[v] as usize;
                        active |= sel[q];
                        for x in 0..3 {
                            for y in 0..3 {
                                j[x][y] += jac[q][x][y] / r.len().max(1) as f64;
                            }
                        }
                    }
                    (j, active)
                }
            };
            if active {
                let n = normal_map(
                    j,
                    [
                        data[3 * i] as f64,
                        data[3 * i + 1] as f64,
                        data[3 * i + 2] as f64,
                    ],
                );
                result[3 * i..3 * i + 3].copy_from_slice(&n.map(|x| x as f32));
            }
        }
        let attr = Arc::new(Attr::f32(3, result));
        match domain {
            "point" => {
                out.point.insert("N".into(), attr);
            }
            "vertex" => {
                out.vertex.insert("N".into(), attr);
            }
            _ => {
                out.prim.insert("N".into(), attr);
            }
        }
    }
    Ok(out)
}
