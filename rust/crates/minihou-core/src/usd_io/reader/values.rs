use super::*;
pub(super) fn value(p: &usd::Prim, n: &str, t: usd::TimeCode) -> Result<Option<U>> {
    p.attribute(n).get_at::<U>(t).map_err(err)
}
pub(super) fn json_value(v: U) -> Result<Value> {
    serde_json::to_value(v).map_err(err)
}
pub(super) fn string(p: &usd::Prim, n: &str, t: usd::TimeCode) -> Result<String> {
    Ok(value(p, n, t)?
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default())
}
pub(super) fn metadata(p: &usd::Prim) -> Result<Value> {
    p.metadata::<U>("customData")
        .map_err(err)?
        .map(json_value)
        .transpose()
        .map(|v| v.unwrap_or(json!({})))
}
pub(super) fn attr_meta(a: &usd::Attribute, key: &str) -> Result<Value> {
    a.metadata::<U>(key)
        .map_err(err)?
        .map(json_value)
        .transpose()
        .map(|v| v.unwrap_or(Value::Null))
}
pub(super) fn flatten(v: &Value, out: &mut Vec<f64>) -> bool {
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
pub(super) fn numeric(v: U, esize: usize) -> Result<Option<Attr>> {
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
pub(super) fn numbers(p: &usd::Prim, n: &str, t: usd::TimeCode) -> Result<Vec<f64>> {
    let mut out = vec![];
    if let Some(v) = value(p, n, t)? {
        flatten(&json_value(v)?, &mut out);
    }
    Ok(out)
}
pub(super) fn indices(p: &usd::Prim, n: &str, t: usd::TimeCode) -> Result<Vec<i32>> {
    Ok(numbers(p, n, t)?.into_iter().map(|x| x as i32).collect())
}
pub(super) fn gather(a: &Attr, rows: &[usize]) -> Result<Attr> {
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
pub(super) fn copy_row(dst: &mut Attr, d: usize, src: &Attr, s: usize) -> Result<()> {
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
pub(super) struct Part {
    pub(super) geo: Geometry,
    pub(super) point_index: Option<Vec<i32>>,
    pub(super) prim_index: Option<Vec<i32>>,
}
pub(super) fn primvars(
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
