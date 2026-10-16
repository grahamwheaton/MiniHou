//! Native USD interchange. OpenUSD's Rust implementation supplies the file formats
//! and composition; this module supplies MiniHou's geometry and metadata conventions.
use crate::{Attr, AttrData, Error, Geometry, Result};
use openusd::{
    sdf::{self, AbstractData, Value as U},
    usd,
};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
    path::{Path, PathBuf},
    sync::Arc,
};

mod reader;
pub use reader::{read_geometry, read_graph, ReadOptions};

pub const UNITS: &[(&str, f64)] = &[
    ("meters", 1.),
    ("centimeters", 0.01),
    ("millimeters", 0.001),
    ("inches", 0.0254),
    ("feet", 0.3048),
];
#[derive(Clone, Debug)]
pub struct WriteOptions {
    pub up_axis: String,
    pub units: String,
    pub root: String,
    pub materials: bool,
    pub overwrite: bool,
    pub graph: Option<Value>,
    pub fps: f64,
}
impl Default for WriteOptions {
    fn default() -> Self {
        Self {
            up_axis: "z".into(),
            units: "meters".into(),
            root: "MiniHou".into(),
            materials: true,
            overwrite: false,
            graph: None,
            fps: 24.,
        }
    }
}
pub type Outputs = BTreeMap<String, Arc<Geometry>>;
pub(crate) fn err(e: impl std::fmt::Display) -> Error {
    Error::new(e.to_string())
}
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
fn unique(s: &str, used: &mut BTreeSet<String>, suffix: &str) -> String {
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
fn q(s: &str) -> String {
    serde_json::to_string(s).expect("string")
}
fn num(x: f64) -> String {
    if x.is_finite() {
        format!("{x:?}")
    } else {
        "0.0".into()
    }
}
fn array(values: impl IntoIterator<Item = String>) -> String {
    format!("[{}]", values.into_iter().collect::<Vec<_>>().join(", "))
}
fn ints(v: impl IntoIterator<Item = usize>) -> String {
    array(v.into_iter().map(|x| x.to_string()))
}
fn attr_values(a: &Attr, rows: &[usize], factor: f64) -> String {
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
fn attr_type(a: &Attr, name: &str) -> String {
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
fn property(out: &mut String, ty: &str, name: &str, value: &str, meta: &str) {
    let _ = writeln!(out, "{ty} {name} = {value} {meta}");
}
fn internal(out: &mut String, name: &str, rows: &[usize], interp: &str) {
    property(
        out,
        "int[]",
        &format!("primvars:minihou:{name}"),
        &ints(rows.iter().copied()),
        &format!("(interpolation = {})", q(interp)),
    );
}
fn attributes(
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
fn positions(out: &mut String, g: &Geometry, pts: &[usize], k: f64, name: &str) {
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
fn scalar(a: Option<&Arc<Attr>>, i: usize, default: f64) -> f64 {
    a.filter(|a| i < a.rows())
        .map(|a| match &a.data {
            AttrData::F32(v) => v[i * a.width] as f64,
            AttrData::I32(v) => v[i * a.width] as f64,
        })
        .unwrap_or(default)
}
fn subset(
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
fn looks(out: &mut String, outputs: &Outputs, top: &str) -> BTreeMap<String, String> {
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
fn geometry(
    out: &mut String,
    g: &Geometry,
    name: &str,
    k: f64,
    looks: &BTreeMap<String, String>,
) -> Result<Value> {
    g.validate()?;
    let mut mesh = vec![];
    let mut curves = vec![];
    let mut used = vec![false; g.npoints];
    let mut dropped = [0usize; 2];
    for i in 0..g.nprims() {
        let n = (g.topology.prim_offsets[i + 1] - g.topology.prim_offsets[i]) as usize;
        let closed = g.topology.is_closed(i);
        if closed && n >= 3 {
            mesh.push(i)
        } else if !closed && n >= 2 {
            curves.push(i)
        } else {
            dropped[usize::from(!closed)] += 1;
            continue;
        }
        for &v in &g.topology.vertex_point
            [g.topology.prim_offsets[i] as usize..g.topology.prim_offsets[i + 1] as usize]
        {
            used[v as usize] = true;
        }
    }
    let _ = writeln!(
        out,
        "def Xform {} (customData = {{ dictionary minihou = {{ string detail = {} dictionary counts = {{ int points = {} int prims = {} }} dictionary dropped = {{ int faces = {} int curves = {} }} }} }}) {{",
        q(name),
        q(&Value::Object(g.detail.clone()).to_string()),
        g.npoints,
        g.nprims(),
        dropped[0],
        dropped[1]
    );
    property(out, "token", "visibility", "\"inherited\"", "");
    // Also an attribute, so animation can sample details without changing metadata.
    property(
        out,
        "custom string",
        "minihou:detail",
        &q(&Value::Object(g.detail.clone()).to_string()),
        "",
    );
    for (prims, kind) in [(&mesh, "mesh"), (&curves, "curves")] {
        if prims.is_empty() {
            continue;
        }
        let corners: Vec<usize> = prims
            .iter()
            .flat_map(|&i| {
                g.topology.prim_offsets[i] as usize..g.topology.prim_offsets[i + 1] as usize
            })
            .collect();
        let pts: Vec<usize> = if kind == "mesh" {
            corners
                .iter()
                .map(|&c| g.topology.vertex_point[c] as usize)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect()
        } else {
            corners
                .iter()
                .map(|&c| g.topology.vertex_point[c] as usize)
                .collect()
        };
        let _ = writeln!(
            out,
            "def {} {} (prepend apiSchemas = [\"MaterialBindingAPI\"]) {{",
            if kind == "mesh" {
                "Mesh"
            } else {
                "BasisCurves"
            },
            q(kind)
        );
        property(out, "token", "visibility", "\"inherited\"", "");
        positions(out, g, &pts, k, "points");
        let counts = ints(
            prims
                .iter()
                .map(|&i| (g.topology.prim_offsets[i + 1] - g.topology.prim_offsets[i]) as usize),
        );
        property(
            out,
            "int[]",
            if kind == "mesh" {
                "faceVertexCounts"
            } else {
                "curveVertexCounts"
            },
            &counts,
            "",
        );
        if kind == "mesh" {
            let local: BTreeMap<_, _> = pts.iter().enumerate().map(|(i, &p)| (p, i)).collect();
            property(
                out,
                "int[]",
                "faceVertexIndices",
                &ints(
                    corners
                        .iter()
                        .map(|&c| local[&(g.topology.vertex_point[c] as usize)]),
                ),
                "",
            );
            property(out, "uniform token", "subdivisionScheme", "\"none\"", "");
            property(out, "uniform token", "orientation", "\"rightHanded\"", "");
        } else {
            property(out, "uniform token", "type", "\"linear\"", "");
            property(out, "uniform token", "wrap", "\"nonperiodic\"", "");
            property(
                out,
                "float[]",
                "widths",
                "[0.01]",
                "(interpolation = \"constant\")",
            );
        }
        internal(out, "point_index", &pts, "vertex");
        internal(out, "prim_index", prims, "uniform");
        attributes(out, g, &pts, &corners, prims, kind == "mesh", k);
        if kind == "mesh" {
            let mut used = BTreeSet::new();
            for (table, rows, el, family) in [
                (&g.prim, prims.as_slice(), "face", "minihou_face_groups"),
                (&g.point, pts.as_slice(), "point", "minihou_point_groups"),
            ] {
                for (n, a) in table
                    .iter()
                    .filter(|(n, a)| n.starts_with("group_") && a.width == 1)
                {
                    let selected: Vec<_> = rows
                        .iter()
                        .enumerate()
                        .filter_map(|(i, &r)| (scalar(Some(a), r, 0.) != 0.).then_some(i))
                        .collect();
                    if !selected.is_empty() {
                        subset(
                            out,
                            &unique(&format!("g_{}", &n[6..]), &mut used, el),
                            el,
                            family,
                            &selected,
                            Some(n),
                            None,
                        );
                    }
                }
                property(
                    out,
                    "uniform token",
                    &format!("subsetFamily:{family}:familyType"),
                    "\"unrestricted\"",
                    "",
                );
            }
            if let Some(mats) = g.detail.get("materials").and_then(Value::as_array) {
                for (slot, n) in mats.iter().enumerate() {
                    if let Some(path) = n.as_str().and_then(|n| looks.get(n)) {
                        let faces: Vec<_> = prims
                            .iter()
                            .enumerate()
                            .filter_map(|(i, &p)| {
                                (scalar(g.prim.get("material_index"), p, 0.) as usize == slot)
                                    .then_some(i)
                            })
                            .collect();
                        if faces.len() == prims.len() {
                            let _ = writeln!(out, "rel material:binding = <{path}>");
                        } else if !faces.is_empty() {
                            subset(
                                out,
                                &format!("mat_{slot}"),
                                "face",
                                "materialBind",
                                &faces,
                                None,
                                Some(path),
                            );
                        }
                    }
                }
                property(
                    out,
                    "uniform token",
                    "subsetFamily:materialBind:familyType",
                    "\"nonOverlapping\"",
                    "",
                );
            }
        }
        out.push_str("}\n");
    }
    let names: Vec<_> = g
        .detail
        .get("instance_objects")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|n| n.as_str().unwrap_or("").to_owned())
                .collect()
        })
        .unwrap_or_default();
    let (placed, loose): (Vec<_>, Vec<_>) = (0..g.npoints).filter(|&i| !used[i]).partition(|&i| {
        let n = scalar(g.point.get("instance"), i, 0.);
        n >= 1. && n <= names.len() as f64
    });
    if !placed.is_empty() {
        out.push_str("def PointInstancer \"instances\" {\n");
        property(out, "token", "visibility", "\"inherited\"", "");
        positions(out, g, &placed, k, "positions");
        property(
            out,
            "int[]",
            "protoIndices",
            &ints(
                placed
                    .iter()
                    .map(|&i| scalar(g.point.get("instance"), i, 1.) as usize - 1),
            ),
            "",
        );
        let mut proto_names = BTreeSet::new();
        let proto_names: Vec<_> = names
            .iter()
            .map(|n| unique(n, &mut proto_names, "proto"))
            .collect();
        // Relative relationship targets are resolved against this prim by USD.
        let _ = writeln!(
            out,
            "rel prototypes = {}",
            array(proto_names.iter().map(|n| format!("<Prototypes/{n}>")))
        );
        let scales = g
            .point
            .get("scale")
            .filter(|a| a.width == 3)
            .cloned()
            .unwrap_or_else(|| Arc::new(Attr::f32(3, vec![1.; g.npoints * 3])));
        property(
            out,
            "float3[]",
            "scales",
            &attr_values(&scales, &placed, k),
            "",
        );
        let rotations = array(placed.iter().map(|&i| {
            let r = g
                .point
                .get("rot")
                .filter(|a| a.width == 3)
                .and_then(|a| a.as_f32())
                .map(|a| [a[i * 3] as f64, a[i * 3 + 1] as f64, a[i * 3 + 2] as f64])
                .unwrap_or([0.; 3]);
            let (sx, cx) = (r[0] * 0.5).sin_cos();
            let (sy, cy) = (r[1] * 0.5).sin_cos();
            let (sz, cz) = (r[2] * 0.5).sin_cos();
            format!(
                "({}, {}, {}, {})",
                num(cx * cy * cz + sx * sy * sz),
                num(sx * cy * cz - cx * sy * sz),
                num(cx * sy * cz + sx * cy * sz),
                num(cx * cy * sz - sx * sy * cz)
            )
        }));
        property(out, "quatf[]", "orientationsf", &rotations, "");
        if let Some(a) = g.point.get("id") {
            property(
                out,
                "int64[]",
                "ids",
                &array(placed.iter().map(|&i| scalar(Some(a), i, 0.).to_string())),
                "",
            );
        }
        for (src, dst, factor) in [
            ("v", "velocities", 1.),
            ("w", "angularVelocities", 180. / std::f64::consts::PI),
        ] {
            if let Some(a) = g.point.get(src).filter(|a| a.width == 3) {
                property(out, "vector3f[]", dst, &attr_values(a, &placed, factor), "");
            }
        }
        if let Some(invis) = g
            .point
            .get("group_invisible")
            .or_else(|| g.point.get("invisible"))
        {
            property(
                out,
                "int64[]",
                "invisibleIds",
                &array(
                    placed
                        .iter()
                        .enumerate()
                        .filter(|(_, i)| scalar(Some(invis), **i, 0.) != 0.)
                        .map(|(r, &i)| scalar(g.point.get("id"), i, r as f64).to_string()),
                ),
                "",
            );
        }
        internal(out, "point_index", &placed, "vertex");
        attributes(out, g, &placed, &[], &[], false, k);
        out.push_str("def Scope \"Prototypes\" {\n");
        for (n, p) in names.iter().zip(proto_names) {
            let _ = writeln!(
                out,
                "def Xform {} (customData = {{dictionary minihou = {{string blender_object = {}}}}}) {{}}",
                q(&p),
                q(n)
            );
        }
        out.push_str("}\n}\n");
    }
    if !loose.is_empty() {
        out.push_str("def Points \"points\" {\n");
        property(out, "token", "visibility", "\"inherited\"", "");
        positions(out, g, &loose, k, "points");
        property(
            out,
            "float[]",
            "widths",
            "[0.02]",
            "(interpolation = \"constant\")",
        );
        internal(out, "point_index", &loose, "vertex");
        attributes(out, g, &loose, &[], &[], false, k);
        out.push_str("}\n");
    }
    out.push_str("}\n");
    Ok(
        json!({"faces":mesh.len(),"curves":curves.len(),"points":loose.len(),"instances":placed.len(),"dropped":{"faces":dropped[0],"curves":dropped[1]}}),
    )
}
fn document(outputs: &Outputs, o: &WriteOptions, frame: f64) -> Result<(sdf::Data, Value)> {
    let mpu = UNITS
        .iter()
        .find(|(n, _)| *n == o.units)
        .map(|(_, v)| *v)
        .ok_or_else(|| {
            err("units must be one of meters, centimeters, millimeters, inches, feet")
        })?;
    if !["z", "y"].contains(&o.up_axis.as_str()) {
        return Err(err("up_axis must be 'z' or 'y'"));
    }
    let top = valid_name(if o.root.is_empty() {
        "MiniHou"
    } else {
        &o.root
    });
    let graph = o
        .graph
        .as_ref()
        .map(|g| format!("string graph = {}", q(&g.to_string())))
        .unwrap_or_default();
    let mut text = format!(
        "#usda 1.0\n(defaultPrim = {} upAxis = {} metersPerUnit = {} documentation = \"Exported from MiniHou (Rust)\" customLayerData = {{dictionary minihou = {{ int format = 1 string outputs = {} double frame = {} {graph} }} }})\ndef Xform {} {{\n",
        q(&top),
        q(&o.up_axis.to_uppercase()),
        num(mpu),
        q(&json!(outputs.keys().collect::<Vec<_>>()).to_string()),
        num(frame),
        q(&top)
    );
    if o.up_axis == "y" {
        text.push_str("float3 xformOp:rotateXYZ = (-90, 0, 0)\nuniform token[] xformOpOrder = [\"xformOp:rotateXYZ\"]\n");
    }
    let looks = if o.materials {
        looks(&mut text, outputs, &top)
    } else {
        BTreeMap::new()
    };
    let mut used = BTreeSet::from(["Looks".into()]);
    let mut reports = serde_json::Map::new();
    for (name, g) in outputs {
        let nm = unique(name, &mut used, "out");
        let mut report = geometry(&mut text, g, &nm, 1. / mpu, &looks)?;
        report["prim"] = json!(format!("/{top}/{nm}"));
        reports.insert(name.clone(), report);
    }
    text.push_str("}\n");
    let data = openusd::usda::parse(&text).map_err(err)?;
    Ok((data, Value::Object(reports)))
}

/// Write a single stage through the same encoder used by animation, nodes and the editor.
fn merge_prototypes(data: &mut sdf::Data, merged: &sdf::Data) -> Result<()> {
    for path in data
        .spec_paths()
        .into_iter()
        .filter(|p| p.to_string().ends_with(".prototypes"))
    {
        let Some(new) = data.try_field(&path, "targetPaths").map_err(err)? else {
            continue;
        };
        let U::PathListOp(new) = new.into_owned() else {
            continue;
        };
        let old = merged
            .try_field(&path, "targetPaths")
            .map_err(err)?
            .map(|v| v.into_owned());
        let mut targets = match old {
            Some(U::PathListOp(v)) => v.explicit_items().to_vec(),
            _ => vec![],
        };
        let mut mapping = vec![];
        for p in new.explicit_items() {
            let index = targets.iter().position(|t| t == p).unwrap_or_else(|| {
                targets.push(p.clone());
                targets.len() - 1
            });
            mapping.push(index as i32);
        }
        let index_path = sdf::Path::new(
            &(path.to_string().trim_end_matches("prototypes").to_owned() + "protoIndices"),
        )
        .map_err(err)?;
        let values = data
            .try_field(&index_path, "default")
            .map_err(err)?
            .map(|v| v.into_owned());
        if let Some(U::IntVec(v)) = values {
            let remapped = v
                .into_iter()
                .map(|i| {
                    mapping
                        .get(i as usize)
                        .copied()
                        .ok_or_else(|| err("invalid protoIndices"))
                })
                .collect::<Result<Vec<_>>>()?;
            data.set_field(&index_path, "default", U::IntVec(remapped));
        }
        data.set_field(
            &path,
            "targetPaths",
            U::PathListOp(sdf::PathListOp::explicit(targets)),
        );
    }
    Ok(())
}

pub fn write_stage(path: &Path, outputs: &Outputs, o: &WriteOptions, frame: f64) -> Result<Value> {
    write_animation(
        path,
        std::iter::once(Ok((frame, outputs.clone()))),
        o,
        false,
    )
}
/// Consumes samples incrementally. Cooked geometry is released after each frame;
/// only the authored USD layer and its samples remain in memory.
pub fn write_animation(
    path: &Path,
    frames: impl IntoIterator<Item = Result<(f64, Outputs)>>,
    o: &WriteOptions,
    animated: bool,
) -> Result<Value> {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !["usd", "usda", "usdc", "usdz"].contains(&ext.as_str()) {
        return Err(err("a USD file must end in .usd, .usda or .usdc"));
    }
    if path.exists() && !o.overwrite {
        return Err(err(format!(
            "{} already exists",
            path.file_name().unwrap_or_default().to_string_lossy()
        )));
    }
    if !o.fps.is_finite() || o.fps <= 0. {
        return Err(err("fps must be positive and finite"));
    }
    let mut merged = sdf::Data::new();
    let mut times = vec![];
    let mut report = json!({});
    for sample in frames {
        let (frame, outputs) = sample?;
        if !frame.is_finite() || times.last().is_some_and(|&t| frame <= t) {
            return Err(err(
                "animation frames must be finite and strictly increasing",
            ));
        }
        let (mut data, r) = document(&outputs, o, frame)?;
        if times.is_empty() {
            report = r;
        }
        merge_prototypes(&mut data, &merged)?;
        for p in data.spec_paths() {
            let new = !merged.has_spec(&p);
            if new {
                merged.create_spec(p.clone(), data.spec_type(&p).unwrap());
            }
            for key in data.list_fields(&p).unwrap_or_default() {
                let v = data.field(&p, &key).map_err(err)?.into_owned();
                if animated
                    && key == "default"
                    && data
                        .try_field(&p, "variability")
                        .map_err(err)?
                        .is_none_or(|v| v.as_ref() != &U::Variability(sdf::Variability::Uniform))
                {
                    // Shading consumers commonly query defaults even for an animated stage.
                    if new && p.to_string().contains("/Looks/") {
                        merged.set_field(&p, "default", v.clone());
                    }
                    let mut samples = match merged.try_field(&p, "timeSamples").map_err(err)? {
                        Some(v) => match v.into_owned() {
                            U::TimeSamples(v) => v,
                            _ => vec![],
                        },
                        None => vec![],
                    };
                    if new && !times.is_empty() {
                        samples.push((
                            times[0],
                            if p.to_string().ends_with(".visibility") {
                                U::token("invisible")
                            } else {
                                U::ValueBlock
                            },
                        ));
                    }
                    samples.push((frame, v));
                    merged.set_field(&p, "timeSamples", U::TimeSamples(samples));
                } else if new || !merged.has_field(&p, &key) {
                    merged.set_field(&p, &key, v);
                } else if key == "targetPaths" && p.to_string().ends_with(".prototypes") {
                    merged.set_field(&p, &key, v);
                } else if key == "typeName" && merged.field(&p, &key).map_err(err)?.as_ref() != &v {
                    return Err(err(format!("{p}: attribute type changes between frames")));
                } else if key == "primChildren" || key == "propertyChildren" {
                    // union children/properties when topology introduces a new prim
                    if let U::TokenVec(mut v) = v {
                        if let U::TokenVec(old) = merged.field(&p, &key).map_err(err)?.as_ref() {
                            for n in old {
                                if !v.contains(n) {
                                    v.push(n.clone());
                                }
                            }
                        }
                        merged.set_field(&p, &key, U::TokenVec(v));
                    }
                }
            }
        }
        if animated {
            for p in merged.spec_paths() {
                if merged.spec_type(&p) == Some(sdf::SpecType::Attribute) && !data.has_spec(&p) {
                    let mut samples = match merged.try_field(&p, "timeSamples").map_err(err)? {
                        Some(v) => match v.into_owned() {
                            U::TimeSamples(v) => v,
                            _ => continue,
                        },
                        None => continue,
                    };
                    samples.push((
                        frame,
                        if p.to_string().ends_with(".visibility") {
                            U::token("invisible")
                        } else {
                            U::ValueBlock
                        },
                    ));
                    merged.set_field(&p, "timeSamples", U::TimeSamples(samples));
                }
            }
        }
        times.push(frame);
    }
    if times.is_empty() {
        return Err(err("no frames to export"));
    }
    if animated {
        let root = sdf::Path::abs_root();
        for (k, v) in [
            ("startTimeCode", times[0]),
            ("endTimeCode", *times.last().unwrap()),
            ("timeCodesPerSecond", o.fps),
            ("framesPerSecond", o.fps),
        ] {
            merged.set_field(&root, k, U::Double(v));
        }
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent).map_err(err)?;
    // Layer::export uses the library's atomic replacement writer (including Windows).
    let mut bytes = vec![];
    openusd::usda::TextWriter::write(&merged, &mut bytes).map_err(err)?;
    let layer = sdf::Layer::from_bytes("minihou.usda", bytes).map_err(err)?;
    layer.export(path.to_string_lossy().as_ref()).map_err(err)?;
    Ok(
        json!({"ok":true,"path":path,"outputs":report,"frames":if animated{Some(json!([times[0],times.last(),times.len()]))}else{None},"frame":times[0],"message":format!("Wrote {}",path.display())}),
    )
}

/// Relative file node paths start at the workspace set by the application, or cwd.
pub fn resolve_path(file: &str) -> PathBuf {
    let p = PathBuf::from(file);
    if p.is_absolute() {
        p
    } else {
        let root = std::env::var_os("MINIHOU_WORKSPACE")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                let cwd = std::env::current_dir().unwrap_or_default();
                cwd.ancestors()
                    .find(|p| p.join("minihou").is_dir() && p.join("rust/Cargo.toml").is_file())
                    .map(Path::to_owned)
                    .unwrap_or(cwd)
            });
        root.join(p)
    }
}

/// Full-resolution worker/session export, preserving live inputs and restoring time on failure.
pub fn export_session(
    session: &mut crate::session::Session,
    path: &Path,
    targets: &[(String, String)],
    o: &WriteOptions,
    frames: Option<Vec<f64>>,
) -> Result<Value> {
    let (frame, fps) = session.time();
    let animated = frames.is_some();
    let frames = frames.unwrap_or_else(|| vec![frame]);
    let result = crate::nodes::usd_nodes::without_exports(|| {
        write_animation(
            path,
            frames.into_iter().map(|f| {
                session.set_time(f, fps)?;
                let mut outputs = Outputs::new();
                for (name, id) in targets {
                    outputs.insert(name.clone(), session.cook(Some(id))?);
                }
                Ok((f, outputs))
            }),
            o,
            animated,
        )
    });
    session.set_time(frame, fps)?;
    result
}
