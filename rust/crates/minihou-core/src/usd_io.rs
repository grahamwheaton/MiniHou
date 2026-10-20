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
mod format;
pub use format::valid_name;
use format::*;

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
