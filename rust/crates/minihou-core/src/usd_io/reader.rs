use super::*;
use openusd::gf::{vec3d, Mat4d as Matrix4d};
use openusd_schemas::geom::{BBoxCache, Purpose, XformCache};

#[derive(Clone, Debug)]
pub struct ReadOptions {
    pub prim_path: String,
    pub purposes: String,
    pub path_attrib: String,
    pub up_axis: String,
    pub apply_units: bool,
    pub time: Option<f64>,
}
impl Default for ReadOptions {
    fn default() -> Self {
        Self {
            prim_path: String::new(),
            purposes: "default render".into(),
            path_attrib: String::new(),
            up_axis: "z".into(),
            apply_units: true,
            time: None,
        }
    }
}
fn value(p: &usd::Prim, n: &str, t: usd::TimeCode) -> Result<Option<U>> {
    p.attribute(n).get_at::<U>(t).map_err(err)
}
fn json_value(v: U) -> Result<Value> {
    serde_json::to_value(v).map_err(err)
}
fn string(p: &usd::Prim, n: &str, t: usd::TimeCode) -> Result<String> {
    Ok(value(p, n, t)?
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default())
}
fn metadata(p: &usd::Prim) -> Result<Value> {
    p.metadata::<U>("customData")
        .map_err(err)?
        .map(json_value)
        .transpose()
        .map(|v| v.unwrap_or(json!({})))
}
fn attr_meta(a: &usd::Attribute, key: &str) -> Result<Value> {
    a.metadata::<U>(key)
        .map_err(err)?
        .map(json_value)
        .transpose()
        .map(|v| v.unwrap_or(Value::Null))
}
fn flatten(v: &Value, out: &mut Vec<f64>) -> bool {
    if let Some(a) = v.as_array() {
        a.iter().all(|v| flatten(v, out))
    } else if let Some(n) = v.as_f64() {
        out.push(n);
        true
    } else if let Some(b) = v.as_bool() {
        out.push(if b { 1. } else { 0. });
        true
    } else {
        false
    }
}
fn numeric(v: U, esize: usize) -> Result<Option<Attr>> {
    let integer = matches!(
        &v,
        U::Bool(_)
            | U::BoolVec(_)
            | U::Int(_)
            | U::IntVec(_)
            | U::Int64(_)
            | U::Int64Vec(_)
            | U::Uint(_)
            | U::UintVec(_)
            | U::Uint64(_)
            | U::Uint64Vec(_)
            | U::Uchar(_)
            | U::UcharVec(_)
            | U::Vec2i(_)
            | U::Vec3i(_)
            | U::Vec4i(_)
            | U::Vec2iVec(_)
            | U::Vec3iVec(_)
            | U::Vec4iVec(_)
    );
    let j = json_value(v)?;
    let width = j
        .as_array()
        .and_then(|a| a.first())
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(esize.max(1));
    let mut flat = vec![];
    if !flatten(&j, &mut flat) || width == 0 || !flat.len().is_multiple_of(width) {
        return Ok(None);
    }
    Ok(Some(if integer {
        Attr::i32(width, flat.into_iter().map(|x| x as i32).collect())
    } else {
        Attr::f32(width, flat.into_iter().map(|x| x as f32).collect())
    }))
}
fn numbers(p: &usd::Prim, n: &str, t: usd::TimeCode) -> Result<Vec<f64>> {
    let mut out = vec![];
    if let Some(v) = value(p, n, t)? {
        flatten(&json_value(v)?, &mut out);
    }
    Ok(out)
}
fn indices(p: &usd::Prim, n: &str, t: usd::TimeCode) -> Result<Vec<i32>> {
    Ok(numbers(p, n, t)?.into_iter().map(|x| x as i32).collect())
}
fn gather(a: &Attr, rows: &[usize]) -> Result<Attr> {
    if rows.iter().any(|&i| i >= a.rows()) {
        return Err(err("USD primvar index outside its value array"));
    }
    Ok(match &a.data {
        AttrData::F32(v) => Attr::f32(
            a.width,
            rows.iter()
                .flat_map(|&r| v[r * a.width..(r + 1) * a.width].iter().copied())
                .collect(),
        ),
        AttrData::I32(v) => Attr::i32(
            a.width,
            rows.iter()
                .flat_map(|&r| v[r * a.width..(r + 1) * a.width].iter().copied())
                .collect(),
        ),
    })
}
fn copy_row(dst: &mut Attr, d: usize, src: &Attr, s: usize) -> Result<()> {
    if dst.width != src.width {
        return Err(err("USD attribute has mismatched widths across prims"));
    }
    let w = src.width;
    match (&mut dst.data, &src.data) {
        (AttrData::F32(a), AttrData::F32(b)) => {
            a[d * w..(d + 1) * w].copy_from_slice(&b[s * w..(s + 1) * w])
        }
        (AttrData::I32(a), AttrData::I32(b)) => {
            a[d * w..(d + 1) * w].copy_from_slice(&b[s * w..(s + 1) * w])
        }
        _ => return Err(err("USD attribute has mismatched types across prims")),
    }
    Ok(())
}
struct Part {
    geo: Geometry,
    point_index: Option<Vec<i32>>,
    prim_index: Option<Vec<i32>>,
}
fn primvars(
    p: &usd::Prim,
    g: &mut Geometry,
    t: usd::TimeCode,
    notes: &mut Vec<String>,
) -> Result<(Option<Vec<i32>>, Option<Vec<i32>>)> {
    let (mut pi, mut qi) = (None, None);
    // Ancestor constant primvars are inherited by USD consumers.
    let mut ancestors = vec![p.clone()];
    let mut parent = p.parent();
    while let Some(p) = parent {
        parent = p.parent();
        ancestors.push(p);
    }
    ancestors.reverse();
    for owner in ancestors {
        for token in owner.property_names().map_err(err)? {
            let raw = token.as_str();
            let normals = raw == "normals" && owner.path() == p.path();
            if !normals && (!raw.starts_with("primvars:") || raw.ends_with(":indices")) {
                continue;
            }
            let a = owner.attribute(raw);
            let Some(v) = a.get_at::<U>(t).map_err(err)? else {
                continue;
            };
            if raw == "primvars:minihou:point_index" {
                pi = Some(indices(p, raw, t)?);
                continue;
            }
            if raw == "primvars:minihou:prim_index" {
                qi = Some(indices(p, raw, t)?);
                continue;
            }
            if raw.starts_with("primvars:minihou:") {
                continue;
            }
            let interp = attr_meta(&a, "interpolation")?
                .as_str()
                .unwrap_or(if normals { "vertex" } else { "constant" })
                .to_owned();
            if owner.path() != p.path() && interp != "constant" {
                continue;
            }
            let es = attr_meta(&a, "elementSize")?.as_u64().unwrap_or(1) as usize;
            let Some(mut attr) = numeric(v, es)? else {
                notes.push(format!("{}: non-numeric primvar {raw} skipped", p.path()));
                continue;
            };
            let ix = indices(&owner, &format!("{raw}:indices"), t)?;
            if !ix.is_empty() {
                if ix.iter().any(|&i| i < 0) {
                    return Err(err("USD primvar has negative indices"));
                }
                attr = gather(&attr, &ix.iter().map(|&i| i as usize).collect::<Vec<_>>())?;
            }
            let meta = attr_meta(&a, "customData")?;
            let dom = meta["minihou"]["domain"].as_str().unwrap_or("");
            let raw_name = raw.strip_prefix("primvars:").unwrap_or(raw);
            let name = meta["minihou"]["name"]
                .as_str()
                .filter(|s| !s.is_empty())
                .unwrap_or(match raw_name {
                    "displayColor" => "Cd",
                    "displayOpacity" => "Alpha",
                    "st" => "uv",
                    "normals" => "N",
                    s => s,
                })
                .to_owned();
            let (table, n) = match (dom, interp.as_str()) {
                ("point", _) => (&mut g.point, g.npoints),
                ("vertex", _) => (&mut g.vertex, g.topology.nvertices()),
                ("prim", _) => (&mut g.prim, g.topology.nprims()),
                (_, "vertex" | "varying") => (&mut g.point, g.npoints),
                (_, "faceVarying") => (&mut g.vertex, g.topology.nvertices()),
                (_, "uniform") => (&mut g.prim, g.topology.nprims()),
                (_, "constant") => {
                    let n = if g.nprims() > 0 {
                        g.nprims()
                    } else {
                        g.npoints
                    };
                    if attr.rows() == 1 {
                        attr = gather(&attr, &vec![0; n])?;
                    }
                    if g.nprims() > 0 {
                        (&mut g.prim, n)
                    } else {
                        (&mut g.point, n)
                    }
                }
                _ => continue,
            };
            if attr.rows() == n {
                table.insert(name, Arc::new(attr));
            } else {
                notes.push(format!(
                    "{}: primvar {raw} has a size that fits no domain; skipped",
                    p.path()
                ));
            }
        }
    }
    Ok((
        pi.filter(|v| v.len() == g.npoints),
        qi.filter(|v| v.len() == g.nprims()),
    ))
}
fn glob(pattern: &str, text: &str) -> bool {
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
fn hit(path: &str, pat: &str) -> bool {
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
fn detail(p: &usd::Prim, t: usd::TimeCode) -> Result<serde_json::Map<String, Value>> {
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
fn group(p: &usd::Prim) -> Result<Option<usd::Prim>> {
    let mut a = Some(p.clone());
    while let Some(p) = a {
        if metadata(&p)?["minihou"]["counts"].is_object() {
            return Ok(Some(p));
        }
        a = p.parent();
    }
    Ok(None)
}

fn bind_material(stage: &usd::Stage, p: &usd::Prim) -> Result<Option<(String, String)>> {
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
fn groups_materials(
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
fn transform(g: &mut Geometry, m: Matrix4d, unit: f64, mesh: bool, left: bool) -> Result<()> {
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
fn instancer(
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
fn assemble(mut parts: Vec<Part>) -> Result<Geometry> {
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
pub fn read_graph(path: &Path) -> Result<Option<Value>> {
    let layer = sdf::Layer::open(path.to_string_lossy().as_ref()).map_err(err)?;
    let v = layer
        .data()
        .try_field(&sdf::Path::abs_root(), "customLayerData")
        .map_err(err)?
        .map(|v| json_value(v.into_owned()))
        .transpose()?
        .unwrap_or(Value::Null);
    v["minihou"]["graph"]
        .as_str()
        .map(|s| serde_json::from_str(s).map_err(err))
        .transpose()
}
pub fn read_geometry(path: &Path, o: &ReadOptions) -> Result<Geometry> {
    if !["z", "keep"].contains(&o.up_axis.as_str()) {
        return Err(err("up_axis must be 'z' or 'keep'"));
    }
    let purpose_text = o.purposes.to_ascii_lowercase();
    let purposes: Vec<_> = purpose_text
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|s| !s.is_empty())
        .collect();
    for purpose in &purposes {
        if !purposes.contains(&"all")
            && !["default", "render", "proxy", "guide", "all"].contains(purpose)
        {
            return Err(err(format!(
                "unknown purpose '{purpose}': use default, render, proxy, guide or all"
            )));
        }
    }
    let purposes = if purposes.is_empty() {
        vec!["default", "render"]
    } else {
        purposes
    };
    let mut inc = vec![];
    let mut exc = vec![];
    for pat in o.prim_path.split_whitespace() {
        let (exclude, pat) = if pat.starts_with(['^', '-']) {
            (true, pat.trim_start_matches(['^', '-']))
        } else {
            (false, pat)
        };
        if !pat.starts_with('/') {
            return Err(err(format!("prim pattern '{pat}' must start with /")));
        }
        if exclude {
            exc.push(pat)
        } else {
            inc.push(pat)
        }
    }
    let stage = usd::Stage::builder()
        .schema_registry(openusd_schemas::schema_registry())
        .open(path.to_string_lossy().as_ref())
        .map_err(|e| {
            err(format!(
                "cannot open {} as USD: {e}",
                path.file_name().unwrap_or_default().to_string_lossy()
            ))
        })?;
    let root = stage.root_layer();
    let layer = root.data();
    let get = |key: &str| -> Result<Value> {
        layer
            .try_field(&sdf::Path::abs_root(), key)
            .map_err(err)?
            .map(|v| json_value(v.into_owned()))
            .transpose()
            .map(|v| v.unwrap_or(Value::Null))
    };
    let y_up = get("upAxis")?.as_str() == Some("Y");
    let unit = if o.apply_units {
        get("metersPerUnit")?
            .as_f64()
            .filter(|x| *x != 0.)
            .unwrap_or(1.)
    } else {
        1.
    };
    drop(root);
    let t = o
        .time
        .map(usd::TimeCode::new)
        .unwrap_or(usd::TimeCode::EARLIEST);
    let mut xcache = XformCache::new(t);
    let axis = if y_up && o.up_axis == "z" {
        Matrix4d::rotation_x(std::f64::consts::FRAC_PI_2)
    } else {
        Matrix4d::IDENTITY
    };
    let mut paths = vec![];
    stage
        .traverse(
            usd::PrimPredicate::DEFAULT.with_instance_proxies(true),
            |p| paths.push(p.clone()),
        )
        .map_err(err)?;
    if inc.len() == 1
        && exc.is_empty()
        && !inc[0].contains(['*', '?'])
        && inc[0] != "/"
        && !paths.iter().any(|p| p.to_string() == inc[0])
    {
        return Err(err(format!(
            "no prim {:?} in {}",
            inc[0],
            path.file_name().unwrap_or_default().to_string_lossy()
        )));
    }
    let mut groups: BTreeMap<String, Vec<Part>> = BTreeMap::new();
    let mut group_details = BTreeMap::new();
    let mut notes = vec![];
    let mut skipped = vec![];
    let mut prune = vec![];
    let mut count = 0;
    let mut source_paths = vec![];
    let mut instance_names = vec![];
    let mut instance_bounds = vec![];
    for path_ in paths {
        let path_s = path_.to_string();
        if prune
            .iter()
            .any(|s: &String| path_s.starts_with(&format!("{s}/")))
        {
            continue;
        }
        let p = stage.prim(&path_).map_err(err)?;
        let ty = p
            .type_name()
            .map_err(err)?
            .map(|s| s.to_string())
            .unwrap_or_default();
        if ty == "PointInstancer" {
            prune.push(path_s.clone());
        }
        if ty == "Xform" && metadata(&p)?["minihou"]["counts"].is_object() {
            group_details.insert(path_s.clone(), detail(&p, t)?);
        }
        if !["Mesh", "BasisCurves", "Points", "PointInstancer"].contains(&ty.as_str()) {
            if [
                "Cube",
                "Sphere",
                "Capsule",
                "Cylinder",
                "Cone",
                "NurbsCurves",
                "NurbsPatch",
            ]
            .contains(&ty.as_str())
            {
                skipped.push(format!("{path_s} ({ty})"));
            }
            continue;
        }
        if (!inc.is_empty() && !inc.iter().any(|pat| hit(&path_s, pat)))
            || exc.iter().any(|pat| hit(&path_s, pat))
        {
            continue;
        }
        let mut purpose = String::new();
        let mut a = Some(p.clone());
        let mut hidden = false;
        while let Some(p) = a {
            if purpose.is_empty() {
                purpose = string(&p, "purpose", t)?;
                if purpose == "default" {
                    purpose.clear();
                }
            }
            let vis = p.attribute("visibility");
            if vis
                .time_samples()
                .map_err(err)?
                .is_some_and(|s| !s.is_empty())
                && string(&p, "visibility", t)? == "invisible"
            {
                hidden = true;
            }
            a = p.parent();
        }
        if hidden {
            continue;
        }
        if purpose.is_empty() {
            purpose = "default".into();
        }
        if !purposes.contains(&"all") && !purposes.contains(&purpose.as_str()) {
            skipped.push(format!("{path_s} ({purpose} purpose)"));
            continue;
        }
        let pts = numbers(
            &p,
            if ty == "PointInstancer" {
                "positions"
            } else {
                "points"
            },
            t,
        )?;
        if pts.is_empty() {
            notes.push(format!("{path_s}: no points at this time; skipped"));
            continue;
        }
        if !pts.len().is_multiple_of(3) {
            return Err(err("USD points must have three components"));
        }
        let (counts, idx) = if ty == "Mesh" {
            (
                indices(&p, "faceVertexCounts", t)?,
                indices(&p, "faceVertexIndices", t)?,
            )
        } else if ty == "BasisCurves" {
            let counts = indices(&p, "curveVertexCounts", t)?;
            let n: usize = counts.iter().map(|&x| x.max(0) as usize).sum();
            if n > pts.len() / 3 {
                return Err(err("curve counts exceed its points"));
            }
            if string(&p, "type", t)? != "linear" {
                notes.push(format!("{path_s}: cubic curve read as its control polygon"));
            }
            (counts, (0..n as i32).collect())
        } else {
            (vec![], vec![])
        };
        let mut offs = vec![0i32];
        for n in counts {
            if n < 0 {
                return Err(err("negative USD face/curve count"));
            }
            offs.push(
                offs.last()
                    .unwrap()
                    .checked_add(n)
                    .ok_or_else(|| err("USD topology exceeds index limit"))?,
            );
        }
        let nc = offs.len() - 1;
        let mut g = Geometry::from_arrays(
            pts.into_iter().map(|x| x as f32).collect(),
            idx,
            offs,
            Some(vec![ty == "Mesh"; nc]),
        )?;
        let (pi, qi) = primvars(&p, &mut g, t, &mut notes)?;
        let group_prim = if pi.is_some() { group(&p)? } else { None };
        if group_prim.is_some() && unit != 1. {
            // The writer scales this MiniHou attribute with positions. Undo that
            // storage-unit conversion before exact primvars win in the instancer.
            if let Some(scale) = g
                .point
                .get("scale")
                .and_then(|a| a.as_f32().map(|v| (a.width, v.to_vec())))
            {
                g.point.insert(
                    "scale".into(),
                    Arc::new(Attr::f32(
                        scale.0,
                        scale
                            .1
                            .into_iter()
                            .map(|v| (v as f64 * unit) as f32)
                            .collect(),
                    )),
                );
            }
        }
        let key = group_prim
            .as_ref()
            .map(|p| p.path().to_string())
            .unwrap_or_default();
        let d = if let Some(ref p) = group_prim {
            detail(p, t)?
        } else {
            Default::default()
        };
        let stored_materials = d.get("materials").is_some_and(Value::is_array)
            && g.prim.contains_key("material_index");
        if ty == "Mesh" {
            groups_materials(&stage, &p, &mut g, t, path, stored_materials)?;
        }
        let m = xcache.local_to_world_transform(&p).map_err(err)? * axis;
        if ty == "PointInstancer" {
            instancer(
                &stage,
                &p,
                &mut g,
                t,
                m,
                unit,
                &mut instance_names,
                &mut instance_bounds,
                &mut notes,
            )?;
        }
        transform(
            &mut g,
            m,
            unit,
            ty == "Mesh",
            string(&p, "orientation", t)? == "leftHanded",
        )?;
        if !o.path_attrib.is_empty() && g.nprims() > 0 {
            g.prim
                .entry(o.path_attrib.clone())
                .or_insert_with(|| Arc::new(Attr::i32(1, vec![source_paths.len() as i32; nc])));
            source_paths.push(path_s);
        }
        group_details.insert(key.clone(), d);
        groups.entry(key).or_default().push(Part {
            geo: g,
            point_index: pi,
            prim_index: qi,
        });
        count += 1;
    }
    let mut geos = vec![];
    for (key, parts) in groups {
        let mut geo = assemble(parts)?;
        geo.detail
            .extend(group_details.remove(&key).unwrap_or_default());
        geos.push(Arc::new(geo));
    }
    let mut geo = if geos.len() == 1 {
        (*geos[0]).clone()
    } else if geos.is_empty() {
        let mut g = Geometry::points_only(vec![])?;
        for d in group_details.into_values() {
            g.detail.extend(d);
        }
        g
    } else {
        assemble(
            geos.into_iter()
                .map(|g| Part {
                    geo: (*g).clone(),
                    point_index: None,
                    prim_index: None,
                })
                .collect(),
        )?
    };
    if !instance_names.is_empty() {
        geo.detail
            .entry("instance_objects")
            .or_insert(json!(instance_names));
    }
    if instance_bounds.iter().any(|b| !b.is_null()) {
        geo.detail
            .entry("instance_bounds")
            .or_insert(json!(instance_bounds));
    }
    if !source_paths.is_empty() {
        geo.detail.insert("usd_paths".into(), json!(source_paths));
    }
    geo.detail.insert("usd_prims".into(), json!(count));
    if !skipped.is_empty() {
        geo.detail.insert(
            "usd_skipped".into(),
            json!(skipped.into_iter().take(20).collect::<Vec<_>>().join("; ")),
        );
    }
    if !notes.is_empty() {
        notes.sort();
        notes.dedup();
        geo.detail.insert(
            "usd_notes".into(),
            json!(notes.join("; ").chars().take(500).collect::<String>()),
        );
    }
    geo.validate()?;
    Ok(geo)
}
