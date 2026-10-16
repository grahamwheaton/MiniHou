//! Queries execute on the existing cook worker, retaining live Blender inputs and time.
use crate::{
    http_api::{error_detail, Response},
    Result,
};
use minihou_core::{
    geometry::AttrData,
    session::Session,
    Geometry,
};
use serde_json::{json, Value};
use std::{path::PathBuf, sync::mpsc};

pub enum QueryKind {
    Inspect {
        node: String,
        detail: bool,
    },
    Export {
        path: PathBuf,
        node: Option<String>,
        graph: Option<Value>,
        overwrite: bool,
        options: Value,
        root: PathBuf,
    },
}
pub struct Query {
    pub kind: QueryKind,
    pub revision: u64,
    pub reply: mpsc::Sender<Response>,
}
pub fn run(session: &mut Session, target: &str, q: Query) {
    let result = match q.kind {
        QueryKind::Inspect { node, detail } => {
            let start = std::time::Instant::now();
            match session.cook(Some(&node)) {
                Ok(g) => {
                    let mut out = summarize(&g, detail);
                    out["node"] = json!(node);
                    out["ok"] = json!(true);
                    out["rev"] = json!(q.revision);
                    out["ms"] = json!(start.elapsed().as_secs_f64() * 1000.);
                    out["cooked"] = json!(session.last_stats().cooked);
                    out["cached"] = json!(session.last_stats().cached);
                    Response::ok(out)
                }
                Err(e) => Response::ok(
                    json!({"ok":false,"node":node,"rev":q.revision,"error":error_detail(&e.to_string())}),
                ),
            }
        }
        QueryKind::Export {
            path,
            node,
            graph,
            overwrite,
            options,
            root,
        } => match export(session, target, path, node, graph, overwrite, options, root) {
            Ok(r) => r,
            Err(e) => Response::error(400, e),
        },
    };
    let _ = q.reply.send(result);
}
pub fn summarize(g: &Geometry, detail: bool) -> Value {
    let mut out = json!({"points":g.npoints,"vertices":g.nvertices(),"prims":g.nprims()});
    if g.nprims() > 0 {
        let sizes = g
            .topology
            .prim_offsets
            .windows(2)
            .map(|w| w[1] - w[0])
            .collect::<Vec<_>>();
        out["verts_per_prim"] = json!([sizes.iter().min(), sizes.iter().max()]);
    }
    if g.npoints > 0 {
        let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for p in g.p().chunks_exact(3) {
            for i in 0..3 {
                lo[i] = lo[i].min(p[i] as f64);
                hi[i] = hi[i].max(p[i] as f64);
            }
        }
        out["bounds"] = json!({"min":lo,"max":hi,"size":([0,1,2].map(|i|hi[i]-lo[i])),"center":([0,1,2].map(|i|(lo[i]+hi[i])/2.))});
    }
    let mut attrs = vec![];
    for (domain, table) in [
        ("point", &g.point),
        ("vertex", &g.vertex),
        ("prim", &g.prim),
    ] {
        for (name, a) in table {
            let mut info =
                json!({"name":name,"domain":domain,"type":a.dtype(),"components":a.width});
            if detail && a.rows() > 0 {
                let data = match &a.data {
                    AttrData::F32(v) => v.iter().map(|&x| x as f64).collect::<Vec<_>>(),
                    AttrData::I32(v) => v.iter().map(|&x| x as f64).collect(),
                };
                let (mut lo, mut hi, mut sum) = (
                    vec![f64::INFINITY; a.width],
                    vec![f64::NEG_INFINITY; a.width],
                    vec![0.; a.width],
                );
                for row in data.chunks_exact(a.width) {
                    for i in 0..a.width {
                        lo[i] = lo[i].min(row[i]);
                        hi[i] = hi[i].max(row[i]);
                        sum[i] += row[i];
                    }
                }
                info["min"] = json!(lo);
                info["max"] = json!(hi);
                info["mean"] = json!(sum.iter().map(|x| x / a.rows() as f64).collect::<Vec<_>>());
                info["first"] = Value::Array(
                    data.chunks_exact(a.width)
                        .take(3)
                        .map(|r| if a.width == 1 { json!(r[0]) } else { json!(r) })
                        .collect(),
                );
            }
            attrs.push(info);
        }
    }
    for (name, value) in &g.detail {
        attrs.push(json!({"name":name,"domain":"detail","value":if value.is_array()||value.is_object(){json!(value.to_string())}else{value.clone()}}));
    }
    out["attributes"] = json!(attrs);
    out
}
fn export(
    session: &mut Session,
    target: &str,
    path: PathBuf,
    node: Option<String>,
    graph: Option<Value>,
    overwrite: bool,
    options: Value,
    _root: PathBuf,
) -> Result<Response> {
    if path.exists() && !overwrite {
        return Ok(Response::error(
            409,
            "file exists; set overwrite to replace it",
        ));
    }
    let targets = if let Some(node) = node {
        vec![(
            options["name"]
                .as_str()
                .filter(|s| !s.is_empty())
                .unwrap_or(&node)
                .to_owned(),
            node,
        )]
    } else {
        let mut t = vec![(target.to_owned(), target.to_owned())];
        t.extend(session.graph().extra_outputs());
        t
    };
    let (frame, fps) = session.time();
    let animated = options["frames"] == "range";
    let frames = if animated {
        let start = options["start"]
            .as_i64()
            .ok_or("start must be an integer")?;
        let end = options["end"].as_i64().ok_or("end must be an integer")?;
        let step = options["step"]
            .as_i64()
            .filter(|s| *s > 0)
            .ok_or("step must be positive")?;
        if end < start {
            return Err("the frame range is empty: end must not be below start".into());
        }
        let n = end.checked_sub(start).ok_or("frame range overflow")? / step + 1;
        if n > 20000 {
            return Err("more than 20000 frames: raise step or shorten the range".into());
        }
        (0..n)
            .map(|i| (start + i * step) as f64)
            .collect::<Vec<_>>()
    } else {
        vec![frame]
    };
    let opts = minihou_core::usd_io::WriteOptions {
        up_axis: options["up_axis"].as_str().unwrap_or("z").into(),
        units: options["units"].as_str().unwrap_or("meters").into(),
        root: options["root"].as_str().filter(|s| !s.is_empty()).unwrap_or("MiniHou").into(),
        materials: options["materials"] != false,
        overwrite, graph, fps,
    };
    let mut report = minihou_core::usd_io::export_session(
        session, &path, &targets, &opts, animated.then_some(frames),
    )?;
    report["frame"] = json!(frame);
    Ok(Response::ok(report))
}
