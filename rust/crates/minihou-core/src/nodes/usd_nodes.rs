//! Python parameter parity; Rust deliberately writes on cook, as requested.
use super::{blender_input::ExternalInputs, Inputs, NodeSpec};
use crate::{
    params::Params,
    usd_io::{self, Outputs, ReadOptions, WriteOptions},
    Error, Geometry, Result,
};
use serde_json::Value;
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
};
pub const USD_EXPORT: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None,
    name: "usd_export",
    inputs: super::GEO,
    params: crate::params!(
        "file":"exports/scene.usdc";"format":"auto";"name":"";"root":"MiniHou";
        "frames":"current";"start":1;"end":24;"step":1;"up_axis":"z";"units":"meters";
        "materials":true;"embed_graph":true;"overwrite":true
    ),
    cook: usd_export,
};
pub const USD_IMPORT: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None,
    name: "usd_import",
    inputs: &[],
    params: crate::params!(
        "file":"";"missing_file":"error";"time_offset":0.0;"time_scale":1.0;"time":"default";
        "prim_path":"";"purpose":"default render";"path_attrib":"";"up_axis":"z";"apply_units":true;"reload":0
    ),
    cook: usd_import,
};
thread_local! {
    static CONTEXT:RefCell<Option<(Value,ExternalInputs)>>=const{RefCell::new(None)};
    static SUPPRESS:Cell<bool>=const{Cell::new(false)};
}
struct RestoreContext(Option<(Value, ExternalInputs)>);
impl Drop for RestoreContext {
    fn drop(&mut self) {
        CONTEXT.with(|c| *c.borrow_mut() = self.0.take());
    }
}
struct RestoreSuppress(bool);
impl Drop for RestoreSuppress {
    fn drop(&mut self) {
        SUPPRESS.with(|c| c.set(self.0));
    }
}
pub(crate) fn without_exports<R>(f: impl FnOnce() -> R) -> R {
    let old = SUPPRESS.with(|s| s.replace(true));
    let _restore = RestoreSuppress(old);
    f()
}
pub(crate) fn exports_suppressed() -> bool {
    SUPPRESS.with(Cell::get)
}
pub(crate) fn with_context<R>(
    graph: &crate::Graph,
    external: &ExternalInputs,
    f: impl FnOnce() -> R,
) -> R {
    if !graph.nodes.iter().any(|(_, d)| d.node_type == "usd_export") {
        return f();
    }
    let old = CONTEXT.with(|c| c.replace(Some((graph.to_value(), external.clone()))));
    let _restore = RestoreContext(old);
    f()
}
pub fn export_target(p: &Params) -> Result<PathBuf> {
    let file = p.s("file")?;
    if file.is_empty() {
        return Err(Error::new("set 'file' to the USD file to write"));
    }
    let mut path = usd_io::resolve_path(&file);
    let ext = path
        .extension()
        .map(|s| s.to_string_lossy().to_ascii_lowercase());
    let fmt = p.e("format", &["auto", "usdc", "usda"])?;
    if ext
        .as_deref()
        .is_some_and(|s| !["usd", "usda", "usdc"].contains(&s))
    {
        return Err(Error::new(format!(
            "{}: a USD file ends in .usd, .usda, .usdc",
            path.file_name().unwrap_or_default().to_string_lossy()
        )));
    }
    if ext.is_none()
        || (fmt == "usdc" && ext.as_deref() == Some("usda"))
        || (fmt == "usda" && ext.as_deref() != Some("usda"))
    {
        path.set_extension(if fmt == "usda" { "usda" } else { "usdc" });
    }
    Ok(path)
}
pub fn export_frames(p: &Params) -> Result<Option<Vec<f64>>> {
    if p.e("frames", &["current", "range"])? != "range" {
        return Ok(None);
    }
    let (start, end, step) = (p.i("start")?, p.i("end")?, p.i("step")?);
    if step < 1 {
        return Err(Error::new("step must be positive"));
    }
    if end < start {
        return Err(Error::new(format!(
            "the frame range {start}..{end} is empty: 'end' must not be below 'start'"
        )));
    }
    let n = (end as i128 - start as i128) / step as i128 + 1;
    if n > 20000 {
        return Err(Error::new(format!(
            "{n} frames is more than the 20000 one export may write; raise 'step' or shorten the range"
        )));
    }
    Ok(Some(
        (0..n)
            .map(|i| (start as i128 + i * step as i128) as f64)
            .collect(),
    ))
}
pub fn write_options(p: &Params) -> Result<WriteOptions> {
    Ok(WriteOptions {
        up_axis: p.s("up_axis")?,
        units: p.s("units")?,
        root: p.s("root")?,
        materials: p.b("materials")?,
        overwrite: p.b("overwrite")?,
        fps: crate::timectx::fps(),
        ..Default::default()
    })
}
fn usd_export(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    cook_named(inputs, p, "geometry")
}
pub(crate) fn cook_named(inputs: &Inputs, p: &Params, id: &str) -> Result<Geometry> {
    let geo = USD_EXPORT.one(inputs, 0)?;
    if SUPPRESS.with(Cell::get) {
        return Ok((*geo).clone());
    }
    let path = export_target(p)?;
    let frames = export_frames(p)?;
    let mut opts = write_options(p)?;
    if path.exists() && !opts.overwrite {
        return Err(Error::new(format!(
            "{} already exists; turn on 'overwrite' to replace it",
            path.file_name().unwrap_or_default().to_string_lossy()
        )));
    }
    let name = p.s("name")?;
    let name = if name.is_empty() { id.to_owned() } else { name };
    let ctx = CONTEXT.with(|c| c.borrow().clone());
    if p.b("embed_graph")? {
        opts.graph = ctx.as_ref().map(|(g, _)| g.clone());
    }
    if let Some(frames) = frames {
        let Some((graph, external)) = ctx else {
            return Err(Error::new(
                "frame range export requires a graph or Session cook",
            ));
        };
        let graph = crate::Graph::from_value(&graph)?;
        let def = graph
            .node(id)
            .ok_or_else(|| Error::new("export node is not in the cooking graph"))?;
        let source = def
            .inputs
            .iter()
            .find(|(p, _)| p == "geo")
            .and_then(|(_, r)| r.first())
            .ok_or_else(|| Error::new("input 'geo' is not connected"))?
            .clone();
        let mut session = crate::session::Session::new(graph);
        for (k, v) in external {
            session.set_external(k, v);
        }
        usd_io::export_session(&mut session, &path, &[(name, source)], &opts, Some(frames))?;
    } else {
        usd_io::write_stage(
            &path,
            &Outputs::from([(name, geo)]),
            &opts,
            crate::timectx::frame(),
        )?;
    }
    Ok((*USD_EXPORT.one(inputs, 0)?).clone())
}
fn usd_import(_: &Inputs, p: &Params) -> Result<Geometry> {
    let file = p.s("file")?;
    let path = usd_io::resolve_path(&file);
    let missing = p.e("missing_file", &["error", "empty"])?;
    let valid = path.extension().is_some_and(|e| {
        ["usd", "usda", "usdc", "usdz"].contains(&e.to_string_lossy().to_ascii_lowercase().as_str())
    });
    if file.is_empty() || !path.is_file() {
        if missing == "empty" {
            return Geometry::points_only(vec![]);
        }
        if file.is_empty() {
            return Err(Error::new("set 'file' to a USD file"));
        }
        if valid {
            return Err(Error::new(format!("no such file: {}", path.display())));
        }
    }
    if !valid {
        return Err(Error::new(format!(
            "{}: not a USD file (expected .usd, .usda, .usdc, .usdz)",
            path.file_name().unwrap_or_default().to_string_lossy()
        )));
    }
    let path_attrib = p.s("path_attrib")?;
    if !path_attrib.is_empty() && usd_io::valid_name(&path_attrib) != path_attrib {
        return Err(Error::new(format!(
            "path_attrib '{path_attrib}' is not a valid attribute name (letters, digits and _)"
        )));
    }
    let scale = p.f("time_scale")?;
    if !scale.is_finite() || scale < 0.0001 {
        return Err(Error::new("time_scale must be at least 0.0001"));
    }
    usd_io::read_geometry(
        &path,
        &ReadOptions {
            prim_path: p.s("prim_path")?,
            purposes: p.s("purpose")?,
            path_attrib,
            up_axis: p.e("up_axis", &["z", "keep"])?,
            apply_units: p.b("apply_units")?,
            time: if p.e("time", &["default", "frame"])? == "frame" {
                Some((crate::timectx::frame() - p.f("time_offset")?) / scale)
            } else {
                None
            },
        },
    )
}
