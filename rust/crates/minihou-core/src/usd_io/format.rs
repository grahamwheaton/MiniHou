use super::*;

pub fn valid_name(s: &str) -> String {
    let mut n: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if n.is_empty() {
        n.push('_');
    }
    if n.as_bytes()[0].is_ascii_digit() {
        n.insert(0, '_');
    }
    n
}
pub(super) fn unique(s: &str, used: &mut BTreeSet<String>, suffix: &str) -> String {
    let base = valid_name(s);
    let mut n = base.clone();
    let mut i = 1;
    while !used.insert(n.clone()) {
        n = if i == 1 {
            format!("{base}__{suffix}")
        } else {
            format!("{base}__{suffix}{i}")
        };
        i += 1;
    }
    n
}
pub(super) fn q(s: &str) -> String {
    serde_json::to_string(s).expect("string")
}
pub(super) fn num(x: f64) -> String {
    if x.is_finite() {
        format!("{x:?}")
    } else {
        "0.0".into()
    }
}
pub(super) fn array(values: impl IntoIterator<Item = String>) -> String {
    format!("[{}]", values.into_iter().collect::<Vec<_>>().join(", "))
}
pub(super) fn ints(v: impl IntoIterator<Item = usize>) -> String {
    array(v.into_iter().map(|x| x.to_string()))
}
pub(super) fn attr_values(a: &Attr, rows: &[usize], factor: f64) -> String {
    let value = |i: usize| match &a.data {
        AttrData::F32(v) => num(v[i] as f64 * factor),
        AttrData::I32(v) => v[i].to_string(),
    };
    array(rows.iter().flat_map(|&r| {
        if (2..=4).contains(&a.width) {
            vec![format!(
                "({})",
                (0..a.width)
                    .map(|c| value(r * a.width + c))
                    .collect::<Vec<_>>()
                    .join(", ")
            )]
        } else {
            (0..a.width).map(|c| value(r * a.width + c)).collect()
        }
    }))
}
#[cfg(test)]
mod invalid_uv_export_tests {
    use super::*;
    #[test]
    fn usd_uv_values_replace_invalid_markers() {
        let uv = Attr::f32(2, vec![f32::NAN, f32::INFINITY, 0.25, 0.75]);
        let encoded = attr_values(&uv, &[0, 1], 1.);
        assert_eq!(encoded, "[(0.0, 0.0), (0.25, 0.75)]");
        assert!(!encoded.contains("NaN"));
    }
}
pub(super) fn attr_type(a: &Attr, name: &str) -> String {
    let base = match &a.data {
        AttrData::F32(_) => "float",
        AttrData::I32(_) => "int",
    };
    let base = match (name, a.width, base) {
        ("Cd", 3, "float") => "color3f",
        ("uv", 2, "float") => "texCoord2f",
        ("N", 3, "float") => "normal3f",
        _ => base,
    };
    if base == "float" || base == "int" {
        format!(
            "{base}{}[]",
            if (2..=4).contains(&a.width) {
                a.width.to_string()
            } else {
                String::new()
            }
        )
    } else {
        format!("{base}[]")
    }
}
pub(super) fn property(out: &mut String, ty: &str, name: &str, value: &str, meta: &str) {
    let _ = writeln!(out, "{ty} {name} = {value} {meta}");
}
pub(super) fn internal(out: &mut String, name: &str, rows: &[usize], interp: &str) {
    property(
        out,
        "int[]",
        &format!("primvars:minihou:{name}"),
        &ints(rows.iter().copied()),
        &format!("(interpolation = {})", q(interp)),
    );
}
pub(super) fn attributes(
    out: &mut String,
    g: &Geometry,
    pts: &[usize],
    corners: &[usize],
    prims: &[usize],
    mesh: bool,
    k: f64,
) {
    let mut used = BTreeSet::new();
    for (table, rows, interp, dom) in [
        (&g.point, pts, "vertex", "point"),
        (
            &g.vertex,
            corners,
            if mesh { "faceVarying" } else { "vertex" },
            "vertex",
        ),
        (&g.prim, prims, "uniform", "prim"),
    ] {
        for (name, a) in table {
            if name == "P" || rows.is_empty() {
                continue;
            }
            let mapped = match name.as_str() {
                "Cd" => "displayColor",
                "Alpha" => "displayOpacity",
                "uv" => "st",
                n => n,
            };
            let normals = mesh
                && name == "N"
                && a.width == 3
                && a.as_f32().is_some()
                && !used.contains("normals");
            let base = if normals {
                "normals".into()
            } else if mapped.starts_with("minihou") {
                format!("attr_{}", valid_name(mapped))
            } else {
                valid_name(mapped)
            };
            let usd_name = unique(&base, &mut used, dom);
            let prop = if normals {
                usd_name
            } else {
                format!("primvars:{usd_name}")
            };
            let element = if a.width > 4 {
                format!("elementSize = {}", a.width)
            } else {
                String::new()
            };
            let meta = format!(
                "(interpolation = {} {element} customData = {{ dictionary minihou = {{ string name = {} string domain = {} }} }})",
                q(interp),
                q(name),
                q(dom)
            );
            property(
                out,
                &attr_type(a, name),
                &prop,
                &attr_values(
                    a,
                    rows,
                    if dom == "point" && name == "scale" {
                        k
                    } else {
                        1.
                    },
                ),
                &meta,
            );
        }
    }
}
pub(super) fn positions(out: &mut String, g: &Geometry, pts: &[usize], k: f64, name: &str) {
    property(
        out,
        "point3f[]",
        name,
        &attr_values(&g.point["P"], pts, k),
        "",
    );
    if !pts.is_empty() {
        let mut lo = [f64::INFINITY; 3];
        let mut hi = [f64::NEG_INFINITY; 3];
        for &p in pts {
            for c in 0..3 {
                let x = g.p()[p * 3 + c] as f64 * k;
                lo[c] = lo[c].min(x);
                hi[c] = hi[c].max(x);
            }
        }
        let value =
            array([lo, hi].map(|v| format!("({}, {}, {})", num(v[0]), num(v[1]), num(v[2]))));
        property(out, "float3[]", "extent", &value, "");
    }
}
pub(super) fn scalar(a: Option<&Arc<Attr>>, i: usize, default: f64) -> f64 {
    a.filter(|a| i < a.rows())
        .map(|a| match &a.data {
            AttrData::F32(v) => v[i * a.width] as f64,
            AttrData::I32(v) => v[i * a.width] as f64,
        })
        .unwrap_or(default)
}
pub(super) fn subset(
    out: &mut String,
    name: &str,
    element: &str,
    family: &str,
    indices: &[usize],
    original: Option<&str>,
    binding: Option<&str>,
) {
    let _ = write!(out, "def GeomSubset {}", q(name));
    if let Some(n) = original {
        let _ = write!(
            out,
            " (customData = {{dictionary minihou = {{string group = {}}}}})",
            q(n)
        );
    } else if binding.is_some() {
        out.push_str(" (prepend apiSchemas = [\"MaterialBindingAPI\"])");
    }
    out.push_str(" {\n");
    property(out, "uniform token", "elementType", &q(element), "");
    property(out, "uniform token", "familyName", &q(family), "");
    property(out, "int[]", "indices", &ints(indices.iter().copied()), "");
    if let Some(p) = binding {
        let _ = writeln!(out, "rel material:binding = <{p}>");
    }
    out.push_str("}\n");
}
pub(super) fn looks(out: &mut String, outputs: &Outputs, top: &str) -> BTreeMap<String, String> {
    let mut names = BTreeMap::new();
    let mut used = BTreeSet::new();
    out.push_str("def Scope \"Looks\" {\n");
    for g in outputs.values() {
        if let Some(list) = g.detail.get("materials").and_then(Value::as_array) {
            for (slot, n) in list.iter().enumerate() {
                let Some(n) = n.as_str().filter(|n| !n.is_empty()) else {
                    continue;
                };
                if names.contains_key(n)
                    || !(0..g.nprims())
                        .any(|i| scalar(g.prim.get("material_index"), i, 0.) as usize == slot)
                {
                    continue;
                }
                let nm = unique(n, &mut used, "mat");
                let path = format!("/{top}/Looks/{nm}");
                let mut color = [0.5; 3];
                let mut sum = [0.; 3];
                let mut count = 0;
                if let Some(cd) = g
                    .prim
                    .get("Cd")
                    .filter(|a| a.width >= 3)
                    .and_then(|a| a.as_f32())
                {
                    let w = g.prim["Cd"].width;
                    for i in 0..g.nprims() {
                        if scalar(g.prim.get("material_index"), i, 0.) as usize == slot {
                            for c in 0..3 {
                                sum[c] += cd[i * w + c] as f64;
                            }
                            count += 1;
                        }
                    }
                    if count > 0 {
                        color = sum.map(|x| (x / count as f64).clamp(0., 1.));
                    }
                } else if let Some(cd) = g
                    .point
                    .get("Cd")
                    .filter(|a| a.width >= 3)
                    .and_then(|a| a.as_f32())
                {
                    let w = g.point["Cd"].width;
                    let points: BTreeSet<_> = (0..g.nprims())
                        .filter(|&i| scalar(g.prim.get("material_index"), i, 0.) as usize == slot)
                        .flat_map(|i| {
                            g.topology.prim_offsets[i] as usize
                                ..g.topology.prim_offsets[i + 1] as usize
                        })
                        .map(|c| g.topology.vertex_point[c] as usize)
                        .collect();
                    if !points.is_empty() {
                        for &i in &points {
                            for c in 0..3 {
                                sum[c] += cd[i * w + c] as f64;
                            }
                        }
                        color = sum.map(|x| (x / points.len() as f64).clamp(0., 1.));
                    }
                }
                if let Some(src) = g.detail.get("usd_materials").and_then(|m| m.get(n)) {
                    if let (Some(file), Some(prim)) = (src["file"].as_str(), src["prim"].as_str()) {
                        if Path::new(file).is_file()
                            && !file.contains('@')
                            && prim.starts_with('/')
                            && !prim.contains('>')
                        {
                            let file = resolve_path(file).to_string_lossy().replace('\\', "/");
                            let _ = writeln!(
                                out,
                                "def Material {} (prepend references = @{file}@<{prim}> customData = {{dictionary minihou = {{string material = {}}}}}) {{}}",
                                q(&nm),
                                q(n)
                            );
                            names.insert(n.to_owned(), path);
                            continue;
                        }
                    }
                }
                let _ = writeln!(
                    out,
                    "def Material {} (customData = {{dictionary minihou = {{string material = {}}}}}) {{",
                    q(&nm),
                    q(n)
                );
                let _ = writeln!(
                    out,
                    "token outputs:surface.connect = <{path}/PreviewSurface.outputs:surface>\ndef Shader \"PreviewSurface\" {{\nuniform token info:id = \"UsdPreviewSurface\"\ncolor3f inputs:diffuseColor = ({}, {}, {})\nfloat inputs:roughness = 0.5\ntoken outputs:surface\n}}\n}}",
                    num(color[0]),
                    num(color[1]),
                    num(color[2])
                );
                names.insert(n.to_owned(), path);
            }
        }
    }
    out.push_str("}\n");
    names
}
