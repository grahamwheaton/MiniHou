use super::*;
pub(super) fn glob(pattern: &str, text: &str) -> bool {
    let (p, s) = (pattern.as_bytes(), text.as_bytes());
    let (mut i, mut j, mut star, mut retry) = (0, 0, None, 0);
    while j < s.len() {
        if i < p.len() && (p[i] == b'?' || p[i] == s[j]) {
            i += 1;
            j += 1;
        } else if i < p.len() && p[i] == b'*' {
            star = Some(i);
            i += 1;
            retry = j;
        } else if let Some(k) = star {
            retry += 1;
            j = retry;
            i = k + 1;
        } else {
            return false;
        }
    }
    while i < p.len() && p[i] == b'*' {
        i += 1;
    }
    i == p.len()
}
pub(super) fn hit(path: &str, pat: &str) -> bool {
    if pat == "/" {
        return true;
    }
    let mut s = path;
    loop {
        if glob(pat, s) {
            return true;
        }
        let Some((parent, _)) = s.rsplit_once('/') else {
            return false;
        };
        if parent.is_empty() {
            return false;
        }
        s = parent;
    }
}
pub(super) fn detail(p: &usd::Prim, t: usd::TimeCode) -> Result<serde_json::Map<String, Value>> {
    let sampled = string(p, "minihou:detail", t)?;
    let m = metadata(p)?;
    let raw = if sampled.is_empty() {
        m["minihou"]["detail"].as_str().unwrap_or("")
    } else {
        &sampled
    };
    Ok(serde_json::from_str::<Value>(raw)
        .ok()
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default())
}
pub(super) fn group(p: &usd::Prim) -> Result<Option<usd::Prim>> {
    let mut a = Some(p.clone());
    while let Some(p) = a {
        if metadata(&p)?["minihou"]["counts"].is_object() {
            return Ok(Some(p));
        }
        a = p.parent();
    }
    Ok(None)
}

pub(super) fn bind_material(stage: &usd::Stage, p: &usd::Prim) -> Result<Option<(String, String)>> {
    let mut a = Some(p.clone());
    while let Some(p) = a {
        if let Some(target) = p
            .relationship("material:binding")
            .forwarded_targets()
            .map_err(err)?
            .first()
        {
            let mat = stage.prim(target).map_err(err)?;
            let m = metadata(&mat)?;
            let name = m["minihou"]["material"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| {
                    target
                        .to_string()
                        .rsplit('/')
                        .next()
                        .unwrap_or("Material")
                        .to_owned()
                });
            return Ok(Some((name, target.to_string())));
        }
        a = p.parent();
    }
    Ok(None)
}
pub(super) fn groups_materials(
    stage: &usd::Stage,
    p: &usd::Prim,
    g: &mut Geometry,
    t: usd::TimeCode,
    file: &Path,
    stored_materials: bool,
) -> Result<()> {
    let mut mats = vec![String::new()];
    let mut sources = serde_json::Map::new();
    let mut slots = vec![0i32; g.nprims()];
    let mut any = false;
    let mut colors = vec![0.5f32; g.nprims() * 3];
    let mut has_color = false;
    let mut bindings = vec![];
    if let Some(b) = bind_material(stage, p)? {
        bindings.push(((0..g.nprims()).collect::<Vec<_>>(), b));
    }
    for child in p.children().map_err(err)? {
        if child
            .type_name()
            .map_err(err)?
            .is_none_or(|s| s.as_str() != "GeomSubset")
        {
            continue;
        }
        let el = string(&child, "elementType", t)?;
        let ids = indices(&child, "indices", t)?;
        let n = if el == "point" { g.npoints } else { g.nprims() };
        let ids: Vec<_> = ids
            .into_iter()
            .filter(|&i| i >= 0 && (i as usize) < n)
            .map(|i| i as usize)
            .collect();
        let m = metadata(&child)?;
        let family = string(&child, "familyName", t)?;
        if family != "materialBind" {
            let name = m["minihou"]["group"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| {
                    format!(
                        "group_{}",
                        valid_name(
                            child
                                .path()
                                .to_string()
                                .rsplit('/')
                                .next()
                                .unwrap_or("subset")
                        )
                    )
                });
            let mut mask = vec![0; n];
            for &i in &ids {
                mask[i] = 1;
            }
            let table = if el == "point" {
                &mut g.point
            } else {
                &mut g.prim
            };
            table
                .entry(name)
                .or_insert_with(|| Arc::new(Attr::i32(1, mask)));
        }
        if el == "face" {
            if let Some(b) = bind_material(stage, &child)? {
                bindings.push((ids, b));
            }
        }
    }
    if !stored_materials {
        for (ids, (name, path)) in bindings {
            let slot = mats.iter().position(|n| n == &name).unwrap_or_else(|| {
                mats.push(name.clone());
                mats.len() - 1
            });
            let material = stage.prim(path.as_str()).map_err(err)?;
            let mut color = None;
            let mut children = material.children().map_err(err)?;
            while let Some(shader) = children.pop() {
                children.extend(shader.children().map_err(err)?);
                if string(&shader, "info:id", t)? == "UsdPreviewSurface" {
                    let values = numbers(&shader, "inputs:diffuseColor", t)?;
                    if values.len() == 3 {
                        color = Some([values[0] as f32, values[1] as f32, values[2] as f32]);
                        break;
                    }
                }
            }
            for i in ids {
                slots[i] = slot as i32;
                any = true;
                if let Some(color) = color {
                    colors[i * 3..i * 3 + 3].copy_from_slice(&color);
                    has_color = true;
                }
            }
            sources.insert(name, json!({"file":file,"prim":path}));
        }
        if any {
            g.prim
                .insert("material_index".into(), Arc::new(Attr::i32(1, slots)));
            g.detail.insert("materials".into(), json!(mats));
            g.detail
                .insert("usd_materials".into(), Value::Object(sources));
        }
        if has_color
            && !g.point.contains_key("Cd")
            && !g.prim.contains_key("Cd")
            && !g.vertex.contains_key("Cd")
        {
            g.prim.insert("Cd".into(), Arc::new(Attr::f32(3, colors)));
        }
    }
    Ok(())
}
pub(super) fn transform(g: &mut Geometry, m: Matrix4d, unit: f64, mesh: bool, left: bool) -> Result<()> {
    let mut points = Vec::with_capacity(g.npoints * 3);
    for p in g.p().chunks_exact(3) {
        let v = m.transform_affine(vec3d(p[0] as f64, p[1] as f64, p[2] as f64));
        points.extend([v.x, v.y, v.z].map(|v| (v as f64 * unit) as f32));
    }
    g.set_p(points);
    if m != Matrix4d::IDENTITY {
        let inv = m.inverse().ok_or_else(|| err("Singular matrix"))?.0;
        for table in [&mut g.point, &mut g.vertex, &mut g.prim] {
            if let Some(a) = table
                .get("N")
                .filter(|a| a.width == 3)
                .and_then(|a| a.as_f32())
            {
                let mut normals = vec![];
                for p in a.chunks_exact(3) {
                    let v: [f64; 3] =
                        std::array::from_fn(|r| (0..3).map(|c| inv[r * 4 + c] * p[c] as f64).sum());
                    let length = v.iter().map(|x| x * x).sum::<f64>().sqrt().max(1e-12);
                    normals.extend(v.map(|x| (x / length) as f32));
                }
                table.insert("N".into(), Arc::new(Attr::f32(3, normals)));
            }
        }
    }
    let a = &m.0;
    let det = a[0] * (a[5] * a[10] - a[6] * a[9]) - a[1] * (a[4] * a[10] - a[6] * a[8])
        + a[2] * (a[4] * a[9] - a[5] * a[8]);
    if mesh && ((det < 0.) ^ left) {
        let mut order = vec![];
        for w in g.topology.prim_offsets.windows(2) {
            if w[0] < w[1] {
                order.push(w[0] as usize);
                order.extend((w[0] as usize + 1..w[1] as usize).rev());
            }
        }
        let topo = Arc::make_mut(&mut g.topology);
        topo.vertex_point = order.iter().map(|&i| topo.vertex_point[i]).collect();
        for a in g.vertex.values_mut() {
            *a = Arc::new(gather(a, &order)?);
        }
    }
    Ok(())
}
