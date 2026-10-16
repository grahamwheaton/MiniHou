//! UI-thread adapter: the HTTP service uses the same history/worker/link path as UI edits.
use crate::*;
use minihou_app::{
    http_api::{error_detail, HttpServer, Request, Response},
    http_geometry::{Query, QueryKind},
    http_ops,
};
use std::{
    collections::VecDeque,
    path::{Component, Path},
    sync::mpsc,
};

#[derive(Default)]
pub(crate) struct HttpUi {
    server: Option<HttpServer>,
    started: bool,
    queue: VecDeque<Request>,
    active: Option<Active>,
}
enum Active {
    Wait {
        request: Request,
        body: Option<Value>,
        deadline: Instant,
    },
    Query {
        reply: mpsc::Sender<Response>,
        receiver: mpsc::Receiver<Response>,
    },
}
pub(crate) fn workspace() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .unwrap()
        .to_path_buf()
}
fn resolve(root: &Path, text: &str, exts: &[&str]) -> Result<PathBuf> {
    let p = Path::new(text);
    let mut p = if p.is_absolute() {
        p.to_path_buf()
    } else {
        root.join(p)
    };
    if !exts.contains(
        &p.extension()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase()
            .as_str(),
    ) {
        return Err(format!("file extension must be one of {exts:?}").into());
    }
    // Canonicalise the closest existing ancestor, so junctions and '..' cannot escape root.
    let mut tail = vec![];
    while !p.exists() {
        let name = p.file_name().ok_or("invalid file path")?.to_owned();
        tail.push(name);
        p = p.parent().ok_or("invalid file path")?.to_owned();
    }
    p = p.canonicalize()?;
    for name in tail.into_iter().rev() {
        p.push(name);
    }
    if p.components().any(|c| matches!(c, Component::ParentDir))
        || !p.starts_with(root.canonicalize()?)
    {
        return Err("path must be inside the MiniHou workspace".into());
    }
    Ok(p)
}
fn files(root: &Path) -> Vec<String> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
        if out.len() >= 500 {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut entries = entries.flatten().collect::<Vec<_>>();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.')
                || [
                    "rust",
                    "node_modules",
                    "__pycache__",
                    "target",
                    "venv",
                    "build",
                    "dist",
                ]
                .contains(&name.as_str())
            {
                continue;
            }
            let Ok(ty) = e.file_type() else { continue };
            if ty.is_symlink() {
                continue;
            }
            let p = e.path();
            if ty.is_dir() {
                walk(root, &p, out);
            } else if p.extension().is_some_and(|e| e == "json") && out.len() < 500 {
                if std::fs::read_to_string(&p)
                    .ok()
                    .and_then(|s| serde_json::from_str::<Value>(&s).ok())
                    .is_some_and(|v| v["nodes"].is_object())
                {
                    out.push(
                        p.strip_prefix(root)
                            .unwrap()
                            .to_string_lossy()
                            .replace('\\', "/"),
                    );
                }
            }
        }
    }
    let mut out = vec![];
    walk(root, root, &mut out);
    out
}
fn library(root: &Path) -> Value {
    let dir = root.join("nodes");
    let files=std::fs::read_dir(&dir).ok().into_iter().flatten().flatten().filter(|e|e.path().extension().is_some_and(|e|e=="py"||e=="json")).map(|e|json!({"file":e.path(),"nodes":[],"error":"Python node-library loading is not available in the Rust core"})).collect::<Vec<_>>();
    json!({"dirs":[dir],"files":files,"backend":"rust","supported":false})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn http_paths_allow_new_workspace_files_and_reject_escapes() {
        let root = workspace();
        let p = resolve(&root, "graphs/_mcp_path_test_/new.json", &["json"]).unwrap();
        assert!(p.starts_with(root.canonicalize().unwrap()));
        assert!(p.ends_with("graphs/_mcp_path_test_/new.json"));
        assert!(resolve(&root, "../outside.json", &["json"]).is_err());
        assert!(resolve(&root, "graphs/../../outside.json", &["json"]).is_err());
        assert!(resolve(&root, "graphs/test.txt", &["json"]).is_err());
        assert!(resolve(
            &root,
            &std::env::temp_dir().join("outside.json").to_string_lossy(),
            &["json"]
        )
        .is_err());
    }
    #[test]
    fn http_library_explicitly_reports_unavailable_python_files() {
        let listing = library(&workspace());
        assert_eq!(listing["supported"], false);
        for entry in listing["files"].as_array().unwrap() {
            assert!(entry["error"]
                .as_str()
                .unwrap()
                .contains("not available in the Rust core"));
        }
    }
}
impl App {
    fn http_sync_selection(&mut self) {
        let nodes = &self.editor.doc.value()["nodes"];
        self.display = Graph::from_value(self.editor.doc.value())
            .ok()
            .and_then(|g| g.main_output().ok())
            .unwrap_or_default();
        self.selected_set.retain(|id| nodes.get(id).is_some());
        if nodes.get(&self.selected).is_none() {
            self.selected = self.display.clone();
            if !self.selected.is_empty() {
                self.selected_set.insert(self.selected.clone());
            }
        }
        self.selection_dirty = true;
    }
    pub(crate) fn poll_http(&mut self, ctx: &egui::Context) {
        if !self.http.started {
            self.http.started = true;
            let wake = ctx.clone();
            match HttpServer::start("127.0.0.1:8766".parse().unwrap(), move || {
                wake.request_repaint()
            }) {
                Ok(server) => {
                    self.http.server = Some(server);
                    self.file_status = "MCP HTTP listening on 127.0.0.1:8766".into();
                }
                Err(e) => {
                    self.blender.message=format!("MCP HTTP unavailable on 127.0.0.1:8766: {e}. Another engine may be using the port; close it and reopen this app to enable MCP.");
                    self.blender_bar = true;
                }
            }
        }
        if let Some(server) = &self.http.server {
            self.http.queue.extend(server.poll());
        }
        if let Some(active) = self.http.active.take() {
            match active {
                Active::Wait {
                    request,
                    body,
                    deadline,
                } => {
                    if self.cooking.cooking() && Instant::now() < deadline {
                        self.http.active = Some(Active::Wait {
                            request,
                            body,
                            deadline,
                        });
                    } else {
                        let mut body = body.unwrap_or_else(|| self.http_status());
                        if body.get("results").is_some() {
                            body["status"] = self.http_status();
                            if let Err(e) = self.editor.save_to(self.editor.path.clone()) {
                                body["save_error"] = json!(e.to_string());
                            }
                        }
                        let _ = request.reply.send(Response::ok(body));
                    }
                }
                Active::Query { reply, receiver } => match receiver.try_recv() {
                    Ok(r) => {
                        let _ = reply.send(r);
                    }
                    Err(mpsc::TryRecvError::Empty) => {
                        self.http.active = Some(Active::Query { reply, receiver })
                    }
                    Err(_) => {
                        let _ = reply.send(Response::error(503, "cook worker stopped"));
                    }
                },
            }
        }
        // Status/alive probes must remain responsive during a waiting edit or
        // export. In particular, MCP must not mistake a heavy cook for a dead
        // Rust app and start a competing Python engine on the same ports.
        if self.http.active.is_some() {
            for request in std::mem::take(&mut self.http.queue) {
                let waits_for_cook = request.path.starts_with("/api/status?")
                    && request.path.contains("wait=1")
                    && self.cooking.cooking();
                if request.method == "GET" && !waits_for_cook {
                    match self.http_request(&request) {
                        Ok(Some(response)) => {
                            let _ = request.reply.send(response);
                        }
                        Err(e) => {
                            let _ = request.reply.send(Response::error(400, e));
                        }
                        Ok(None) => {}
                    }
                } else {
                    self.http.queue.push_back(request);
                }
            }
        }
        if self.http.active.is_none() {
            if let Some(request) = self.http.queue.pop_front() {
                match self.http_request(&request) {
                    Ok(Some(response)) => {
                        let _ = request.reply.send(response);
                    }
                    Ok(None) => {}
                    Err(e) => {
                        let message = e.to_string();
                        let mut response = Response::error(400, &message);
                        if request.path.split('?').next() == Some("/api/edit") {
                            if let Some(index) = message
                                .strip_prefix("op ")
                                .and_then(|s| s.split_whitespace().next()?.parse::<usize>().ok())
                            {
                                response.body["index"] = json!(index);
                            }
                        }
                        let _ = request.reply.send(response);
                    }
                }
            }
        }
        if self.http.active.is_some() || !self.http.queue.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(10));
        }
    }
    fn http_snapshot(&self) -> Value {
        let path =
            std::path::absolute(&self.editor.path).unwrap_or_else(|_| self.editor.path.clone());
        let live = if self.link.as_ref().is_some_and(|link| link.live_active()) {
            json!({"label":"Blender live modifier","connected":!self.blender.clients.is_empty()})
        } else {
            Value::Null
        };
        json!({"rev":self.cooking.requested,"document_id":path.to_string_lossy(),"path":path,"graph":self.editor.doc.value(),"can_undo":self.editor.history.can_undo(),"can_redo":self.editor.history.can_redo(),"live_modifier":live,"dirty":self.editor.dirty()})
    }
    fn http_proxy(&self) -> Value {
        let p = self.editor_state.proxy;
        json!({"level":p.level,"auto":p.auto,"link_tris":p.link_tris,"cooked":1.0,"mode":"viewport","note":"Rust cooks full geometry; proxy reduces viewport and link triangles"})
    }
    fn http_status(&self) -> Value {
        let c = &self.cooking;
        let (frame, fps) = self
            .link
            .as_ref()
            .map(|l| (l.frame, l.fps))
            .unwrap_or(self.http_time);
        let mut out = json!({"engine":"rust","rev":c.requested,"cooked_rev":c.completed,"busy":c.cooking(),"ok":if c.completed==0{Value::Null}else{json!(c.error.is_none())},"frame":frame,"fps":fps,"proxy":self.http_proxy(),"cook":{"ms":c.milliseconds,"cooked":c.stats.cooked,"cached":c.stats.cached}});
        if let Some(g) = &c.geometry {
            out["points"] = json!(g.npoints);
            out["prims"] = json!(g.nprims());
            out["vertices"] = json!(g.nvertices());
        }
        if let Some(e) = &c.error {
            out["error"] = error_detail(e);
        }
        out
    }
    fn http_wait(&mut self, r: &Request, body: Option<Value>) {
        let wait = r.body["timeout"]
            .as_f64()
            .unwrap_or_else(|| {
                r.path
                    .split("timeout=")
                    .nth(1)
                    .and_then(|s| s.split('&').next()?.parse().ok())
                    .unwrap_or(30.)
            })
            .clamp(0., 110.);
        self.http.active = Some(Active::Wait {
            request: Request {
                method: r.method.clone(),
                path: r.path.clone(),
                body: r.body.clone(),
                reply: r.reply.clone(),
            },
            body,
            deadline: Instant::now() + Duration::from_secs_f64(wait),
        });
    }
    fn http_query(&mut self, r: &Request, kind: QueryKind) -> Result<()> {
        let (tx, receiver) = mpsc::channel();
        self.worker.query(Query {
            kind,
            revision: self.cooking.requested,
            reply: tx,
        })?;
        self.http.active = Some(Active::Query {
            reply: r.reply.clone(),
            receiver,
        });
        Ok(())
    }
    fn http_request(&mut self, r: &Request) -> Result<Option<Response>> {
        let path = r.path.split('?').next().unwrap_or("");
        let b = &r.body;
        let root = workspace();
        // Read/modify requests observe a fully accepted worker revision. Never wait on the UI thread.
        if self.cooking.cooking()
            && matches!(
                (r.method.as_str(), path),
                (
                    "POST",
                    "/api/inspect" | "/api/export_usd" | "/api/node_action"
                )
            )
        {
            self.http.queue.push_front(Request {
                method: r.method.clone(),
                path: r.path.clone(),
                body: b.clone(),
                reply: r.reply.clone(),
            });
            return Ok(None);
        }
        let out = match (r.method.as_str(), path) {
            ("GET", "/api/status") => {
                if r.path.contains("wait=1") && self.cooking.cooking() {
                    self.http_wait(r, None);
                    return Ok(None);
                }
                self.http_status()
            }
            ("GET", "/api/graph") => self.http_snapshot(),
            ("GET", "/api/nodes") => minihou_app::http_api::node_types(),
            ("GET", "/api/state") => {
                let mut s = self.http_snapshot();
                s["status"] = self.http_status();
                s["nodes"] = minihou_app::http_api::node_types();
                s["library"] = library(&root);
                s["root"] = json!(root);
                s["ops"] = json!(http_ops::OPS);
                s["link"] = json!({"port":if self.link.is_some(){Some(8765)}else{None},"clients":self.blender.clients});
                s["engine"] = json!({"engine":"rust","backend":"rust","pid":std::process::id(),"version":"0.1.0 (rust)","python_nodes":false,"capabilities":["instances_v1","scene_objects_v1"]});
                s
            }
            ("GET", "/api/files") => {
                json!({"files":files(&root),"current":self.http_snapshot()["path"],"root":root})
            }
            ("GET", "/api/proxy") => self.http_proxy(),
            ("POST", "/api/proxy") => {
                let mut p = self.editor_state.proxy;
                if let Some(v) = b.get("level") {
                    let n = v
                        .as_f64()
                        .filter(|n| (0.001..=1.).contains(n))
                        .ok_or("level must be between 0.001 and 1")?;
                    p.level = n as f32;
                }
                if let Some(v) = b.get("auto") {
                    p.auto = v.as_bool().ok_or("auto must be boolean")?;
                }
                if let Some(v) = b.get("link_tris") {
                    p.link_tris = u32::try_from(
                        v.as_u64()
                            .ok_or("link_tris must be a nonnegative integer")?,
                    )?;
                }
                p.clustered = false;
                self.editor_state.proxy = p;
                self.http_proxy()
            }
            ("POST", "/api/edit") => {
                let before = self.editor.doc.value();
                let (value, results) = http_ops::apply_ops(before, &b["ops"])?;
                let changed = before != &value;
                if changed && !self.commit_document(value, "apply MCP edits") {
                    return Err(self.file_status.clone().into());
                }
                if changed {
                    self.http_sync_selection();
                }
                let out = json!({"changed":changed,"rev":self.cooking.requested,"results":results,"graph":self.http_snapshot(),"status":self.http_status()});
                if b["wait"] == true {
                    self.http_wait(r, Some(out));
                    return Ok(None);
                }
                out
            }
            ("POST", "/api/undo" | "/api/redo") => {
                let edit = if path.ends_with("/undo") {
                    self.editor.history.undo(&mut self.editor.doc)
                } else {
                    self.editor.history.redo(&mut self.editor.doc)
                };
                let ok = edit.is_some();
                if let Some(edit) = edit {
                    self.send(edit);
                    self.http_sync_selection();
                }
                json!({"ok":ok,"graph":self.http_snapshot()})
            }
            ("POST", "/api/open") => {
                if self.link.as_ref().is_some_and(|link| link.live_active()) {
                    return Err(
                        "Disconnect the live Blender modifier before opening another document."
                            .into(),
                    );
                }
                let text = b["path"].as_str().ok_or("path must be a graph file path")?;
                let mut p = resolve(&root, text, &["json"])?;
                if !p.exists() && b["create"] == true {
                    if p.parent() == Some(root.canonicalize()?.as_path()) {
                        p = root.join("graphs").join(p.file_name().unwrap());
                    }
                    if let Some(parent) = p.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    let doc=Document::parse(json!({"version":1,"nodes":{"box1":{"type":"box","pos":[0,0]},"output1":{"type":"output","inputs":{"geo":"box1"},"pos":[0,150]}},"display":"output1"}).to_string())?;
                    doc.save(&p)?;
                }
                let doc = Document::open(&p)?;
                http_ops::validate(doc.value())?;
                // Same Document::open / App::open replacement as the File menu. Preserve the listener across it.
                if !self.open(p.clone()) {
                    return Err(self.file_status.clone().into());
                }
                self.http_snapshot()
            }
            ("POST", "/api/frame") => {
                let frame = b["frame"]
                    .as_f64()
                    .filter(|f| f.abs() < 1e7)
                    .ok_or("frame must be a number in range")?;
                let fps = match b.get("fps").filter(|v| !v.is_null()) {
                    Some(v) => v
                        .as_f64()
                        .filter(|f| *f > 0. && *f < 1000.)
                        .ok_or("fps out of range")?,
                    None => self.link.as_ref().map_or(self.http_time.1, |l| l.fps),
                };
                if let Some(link) = &mut self.link {
                    link.set_frame(&json!({"frame":frame,"fps":fps}))?;
                }
                self.http_time = (frame, fps);
                self.send(Edit::Time { frame, fps });
                json!({"frame":frame,"fps":fps})
            }
            ("POST", "/api/inspect") => {
                self.http_query(
                    r,
                    QueryKind::Inspect {
                        node: b["node"].as_str().ok_or("node must be a node id")?.into(),
                        detail: b["detail"].as_bool().unwrap_or(false),
                    },
                )?;
                return Ok(None);
            }
            ("POST", "/api/export_usd" | "/api/node_action") => {
                let mut options = b.get("options").cloned().unwrap_or_else(|| json!({}));
                let (mut node, mut embed, mut overwrite) = (
                    b["node"].as_str().map(str::to_owned),
                    b["graph"] != false,
                    b["overwrite"] == true,
                );
                let text = if path == "/api/node_action" {
                    let id = node.as_deref().ok_or("node must be a node id")?;
                    let n = self.editor.doc.value()["nodes"]
                        .get(id)
                        .ok_or("no such node")?;
                    let ty = n["type"].as_str().unwrap_or("");
                    if ty == "usd_import" && b["action"] == "reload" {
                        let reload = n["params"]["reload"].as_i64().unwrap_or(0) + 1;
                        let id = id.to_owned();
                        self.edit(Edit::Param { node: id, name: "reload".into(), value: json!(reload) }, false);
                        return Ok(Some(Response::ok(json!({"ok":true,"message":"Reloaded USD input"}))));
                    }
                    if ty != "usd_export" || b["action"] != "save" {
                        return Ok(Some(Response::error(
                            501,
                            format!(
                                "node type '{ty}' button '{}' is not available in the Rust core",
                                b["action"].as_str().unwrap_or("")
                            ),
                        )));
                    }
                    for p in minihou_core::nodes::get(ty).unwrap().params {
                        options[p.name] = n["params"]
                            .get(p.name)
                            .cloned()
                            .unwrap_or_else(|| (p.default)());
                    }
                    embed = options["embed_graph"] != false;
                    overwrite = options["overwrite"] == true;
                    options["file"]
                        .as_str()
                        .ok_or("file must be a path")?
                        .to_owned()
                } else {
                    b["path"]
                        .as_str()
                        .ok_or("path must be a USD file path")?
                        .to_owned()
                };
                if b.get("node").is_some() && node.is_none() {
                    return Err("node must be a node id".into());
                }
                let mut file = PathBuf::from(&text);
                if path == "/api/node_action" {
                    let fmt = options["format"].as_str().unwrap_or("auto");
                    if !["auto", "usda", "usdc"].contains(&fmt) {
                        return Err("invalid USD format".into());
                    }
                    let ext = file.extension().map(|e| e.to_string_lossy().to_lowercase());
                    if ext
                        .as_deref()
                        .is_some_and(|e| !["usd", "usda", "usdc"].contains(&e))
                    {
                        return Err("a USD file ends in .usd, .usda or .usdc".into());
                    }
                    if ext.is_none()
                        || (fmt == "usdc" && ext.as_deref() == Some("usda"))
                        || (fmt == "usda" && ext.as_deref() != Some("usda"))
                    {
                        file.set_extension(if fmt == "usda" { "usda" } else { "usdc" });
                    }
                }
                let p = resolve(&root, &file.to_string_lossy(), &["usd", "usda", "usdc"])?;
                self.http_query(
                    r,
                    QueryKind::Export {
                        path: p,
                        node: node.take(),
                        graph: embed.then(|| self.editor.doc.value().clone()),
                        overwrite,
                        options,
                        root,
                    },
                )?;
                return Ok(None);
            }
            _ => {
                return Ok(Some(Response::error(
                    404,
                    format!("no such endpoint: {path}"),
                )))
            }
        };
        Ok(Some(Response::ok(out)))
    }
}
