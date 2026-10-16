//! Connection-owned stack evaluators. No access to the proxy session's external inputs.
use super::*;
use minihou_app::{link_interface, link_param_edit};
use minihou_core::{nodes::blender_input::ExternalInput, session::Session, Graph};

type Result<T> = minihou_app::Result<T>;
struct Entry {
    graph: Value,
    session: Session,
    input: Arrays,
    materials: Value,
    sources: BTreeMap<String, Arc<Geometry>>,
}
struct Live {
    graph: Value,
    path: String,
    sent: Value,
    input: Arrays,
    header: Value,
}
pub(super) struct InstanceManager {
    peer: u64,
    shared: Arc<LinkShared>,
    requests: mpsc::SyncSender<LinkRequest>,
    entries: BTreeMap<String, Entry>,
    sources: BTreeMap<String, BTreeMap<String, Arc<Geometry>>>,
    live: BTreeMap<String, Live>,
}
impl InstanceManager {
    pub fn new(
        peer: u64,
        shared: Arc<LinkShared>,
        requests: mpsc::SyncSender<LinkRequest>,
    ) -> Self {
        Self {
            peer,
            shared,
            requests,
            entries: BTreeMap::new(),
            sources: BTreeMap::new(),
            live: BTreeMap::new(),
        }
    }
    pub fn handle(&mut self, h: &Value, arrays: &Arrays) -> (Value, Arrays) {
        let result = self.request(h, arrays);
        let (mut reply, arrays) = result.unwrap_or_else(|e| {
            (
                json!({"type":"instance_error","message":e.to_string()}),
                Arrays::new(),
            )
        });
        reply["request_id"] = h["request_id"].clone();
        reply["instance_id"] = h["instance_id"].clone();
        reply["seq"] = h["request_id"].clone();
        (reply, arrays)
    }
    fn graph(&self, h: &Value) -> Result<Value> {
        let data = if !h["graph"].is_null() {
            h["graph"].clone()
        } else {
            let path = PathBuf::from(
                h["graph_path"]
                    .as_str()
                    .filter(|p| !p.is_empty())
                    .ok_or("choose a graph file or supply a saved graph")?,
            );
            if path
                .extension()
                .is_none_or(|s| !s.eq_ignore_ascii_case("json"))
            {
                return Err("modifier graph must be a .json file".into());
            }
            let snapshot = self.shared.snapshot.lock().unwrap();
            let same = path
                .canonicalize()
                .ok()
                .zip(Path::new(&snapshot.graph_path).canonicalize().ok())
                .is_some_and(|(a, b)| a == b);
            if same {
                snapshot.document.clone()
            } else {
                Document::open(&path)?.value().clone()
            }
        };
        validate_graph(&data)?;
        Ok(data)
    }
    fn request(&mut self, h: &Value, arrays: &Arrays) -> Result<(Value, Arrays)> {
        let typ = h["type"].as_str().unwrap_or("");
        if typ == "instance_describe" {
            let graph = self.graph(h)?;
            return Ok((
                json!({"type":"instance_description","interface":link_interface(&graph)["params"],"graph":graph}),
                Arrays::new(),
            ));
        }
        let id = h["instance_id"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 256)
            .ok_or("invalid modifier identity")?
            .to_string();
        if typ == "instance_remove" {
            self.remove(&id)?;
            return Ok((json!({"type":"instance_removed"}), Arrays::new()));
        }
        if typ == "instance_sources" {
            let names = h["sources"]
                .as_array()
                .filter(|s| s.len() <= 64)
                .ok_or("invalid source inputs")?;
            if !self.sources.contains_key(&id) && self.sources.len() >= 256 {
                return Err("too many object inputs".into());
            }
            let mut sources = BTreeMap::new();
            for (i, name) in names.iter().enumerate() {
                let name = name
                    .as_str()
                    .filter(|s| s.len() <= 128)
                    .ok_or("invalid source name")?;
                let prefix = format!("{i}/");
                let arrays = arrays
                    .iter()
                    .filter_map(|(k, a)| {
                        k.strip_prefix(&prefix).map(|k| (k.to_string(), a.clone()))
                    })
                    .collect();
                sources.insert(
                    name.into(),
                    Arc::new(decode_geometry(&Value::Null, &arrays)?),
                );
            }
            self.sources.insert(id.clone(), sources);
            if self.live.contains_key(&id) {
                self.refresh(&id);
                self.update_live(&id, "live_update", None)?;
            }
            return Ok((json!({"type":"instance_sources_set"}), Arrays::new()));
        }
        if typ == "instance_sync" {
            return Ok((self.sync(&id, h)?, Arrays::new()));
        }
        if !["instance_cook", "instance_edit"].contains(&typ) {
            return Err("unsupported instance request".into());
        }
        let (frame, fps) = time(h)?;
        let geometry = Arc::new(decode_geometry(h, arrays)?);
        let mut graph = self.graph(h)?;
        apply_params(&mut graph, &h["params"])?;
        if typ == "instance_edit" {
            if !self.live.contains_key(&id) {
                if self.live.len() >= 256 {
                    return Err("maximum 256 live documents per connection".into());
                }
                let path = env::temp_dir().join(format!(
                    "minihou-live-{}-{}-{}.json",
                    std::process::id(),
                    self.peer,
                    NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
                ));
                self.live.insert(
                    id.clone(),
                    Live {
                        graph: graph.clone(),
                        path: path.to_string_lossy().into_owned(),
                        sent: json!({}),
                        input: arrays.clone(),
                        header: h.clone(),
                    },
                );
            }
            // Re-selecting an existing editor must preserve its edits.
            let live = self.live.get_mut(&id).unwrap();
            live.input = arrays.clone();
            live.header = h.clone();
            self.refresh(&id);
            self.update_live(&id, "live_activate", None)?;
            return Ok((self.sync(&id, &json!({}))?, Arrays::new()));
        }
        if self.live.contains_key(&id) {
            self.refresh(&id);
            let live = self.live.get_mut(&id).unwrap();
            if live.input != *arrays
                || live.header["materials"] != h["materials"]
                || time(&live.header)? != (frame, fps)
            {
                live.input = arrays.clone();
                live.header = h.clone();
                self.refresh(&id);
                self.update_live(&id, "live_update", None)?;
            }
        }
        if !self.entries.contains_key(&id) {
            if self.entries.len() >= 256 {
                return Err("maximum 256 modifier instances per connection".into());
            }
            let mut session = Session::new(Graph::from_value(&graph)?);
            session.set_budget_bytes(Some(16 * 1024 * 1024));
            self.entries.insert(
                id.clone(),
                Entry {
                    graph: graph.clone(),
                    session,
                    input: Arrays::new(),
                    materials: Value::Null,
                    sources: BTreeMap::new(),
                },
            );
        }
        let entry = self.entries.get_mut(&id).unwrap();
        if entry.graph != graph {
            entry.session.replace_graph(Graph::from_value(&graph)?);
            entry.graph = graph.clone();
        }
        if entry.input != *arrays || entry.materials != h["materials"] {
            entry.session.set_external(
                "modifier_input".into(),
                ExternalInput::Object {
                    geo: geometry,
                    matrix: None,
                    info: Value::Null,
                },
            );
            entry.input = arrays.clone();
            entry.materials = h["materials"].clone();
        }
        let feeds = self.sources.get(&id);
        let bindings: Vec<_> = entry
            .session
            .graph()
            .nodes
            .iter()
            .filter(|(_, n)| n.node_type == "object_input")
            .map(|(nid, n)| {
                n.params
                    .as_ref()
                    .and_then(|p| p.get("binding"))
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .unwrap_or(nid)
                    .to_string()
            })
            .collect();
        for key in bindings {
            let incoming = feeds.and_then(|s| s.get(&key));
            let unchanged = entry
                .sources
                .get(&key)
                .zip(incoming)
                .is_some_and(|(a, b)| Arc::ptr_eq(a, b));
            if !unchanged {
                entry.session.set_external(format!("object_input:{key}"),incoming.map_or_else(||ExternalInput::Missing("Choose an object in this modifier's Object input picker in Blender".into()),|g|ExternalInput::Object{geo:g.clone(),matrix:None,info:Value::Null}));
            }
        }
        entry.sources = feeds.cloned().unwrap_or_default();
        entry.session.set_time(frame, fps)?;
        let target = if h["render"] == true {
            graph["render"].as_str().map(str::to_string)
        } else {
            None
        };
        let start = Instant::now();
        let result = entry.session.cook(target.as_deref())?;
        if result.p().iter().any(|x| !x.is_finite()) {
            return Err("cooked point positions must be finite".into());
        }
        let stats = entry.session.last_stats().clone();
        let cook = json!({"ms":start.elapsed().as_secs_f64()*1000.,"cooked":stats.cooked,"cached":stats.cached,"node_ms":stats.node_ms.into_iter().collect::<BTreeMap<_,_>>(),"frame":frame,"fps":fps});
        let (mut header, mut output) =
            DiffState::default().build(&link_protocol::geometry_arrays(&result), &result, 1, &cook);
        header["type"] = json!("instance_result");
        header["interface"] = link_interface(&graph)["params"].clone();
        let sinks: Vec<_> = entry
            .session
            .graph()
            .nodes
            .iter()
            .filter(|(_, n)| {
                ["scene_objects", "image_out"].contains(&n.node_type.as_str()) && !n.bypass
            })
            .map(|(id, n)| (id.clone(), n.node_type.clone(), n.params.clone()))
            .collect();
        let mut scene = Vec::new();
        let mut errors = Vec::new();
        for (nid, typ, params) in sinks {
            let manifest = (|| -> Result<_> {
                let geo = entry.session.cook(Some(&nid))?;
                let p = minihou_core::params::Params::resolve(
                    minihou_core::nodes::get(&typ).unwrap().params,
                    params.as_ref(),
                )?;
                link_protocol::sink_manifest(&typ, &nid, &p, &geo, "")
            })();
            match manifest {
                Ok((h, a)) => {
                    scene.push(h);
                    output.extend(a.into_iter().map(|(k, a)| (format!("scene:{nid}:{k}"), a)));
                }
                Err(e) => errors.push(json!({"node":nid,"message":e.to_string()})),
            }
        }
        header["scene"] = json!(scene);
        header["scene_errors"] = json!(errors);
        Ok((header, output))
    }
    fn refresh(&mut self, id: &str) {
        if let Some(live) = self.live.get_mut(id) {
            if let Some(graph) = self.shared.live_documents.lock().unwrap().get(&live.path) {
                live.graph = graph.clone();
            }
        }
    }
    fn sync(&mut self, id: &str, h: &Value) -> Result<Value> {
        self.refresh(id);
        let Some(live) = self.live.get_mut(id) else {
            return Ok(json!({"type":"instance_synced","live":false}));
        };
        let mut graph = live.graph.clone();
        let mut changed = serde_json::Map::new();
        if !h["params"].is_null() {
            let params = h["params"].as_object().ok_or("params must be an object")?;
            for (name, value) in params {
                if let Some(old) = live.sent.get(name) {
                    if !same_param(old, value)
                        && link_interface(&graph)["params"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|p| p["name"] == *name)
                    {
                        apply_params(&mut graph, &json!({name:value}))?;
                        changed.insert(name.clone(), value.clone());
                    }
                }
            }
        }
        if !changed.is_empty() {
            live.graph = graph;
            self.update_live(id, "live_update", Some(Value::Object(changed)))?;
        }
        let live = self.live.get_mut(id).unwrap();
        let interface = link_interface(&live.graph)["params"].clone();
        live.sent = Value::Object(
            interface
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|p| Some((p["name"].as_str()?.to_owned(), p["value"].clone())))
                .collect(),
        );
        Ok(
            json!({"type":"instance_synced","live":true,"editor":"native","graph":live.graph,"interface":interface,"path":live.path}),
        )
    }
    fn update_live(&mut self, id: &str, typ: &str, overrides: Option<Value>) -> Result<()> {
        let live = &self.live[id];
        let mut arrays = live.input.clone();
        let sources = self.sources.get(id).cloned().unwrap_or_default();
        for (i, g) in sources.values().enumerate() {
            arrays.extend(
                link_protocol::geometry_arrays(g)
                    .into_iter()
                    .map(|(k, a)| (format!("source:{i}/{k}"), a)),
            );
        }
        let (frame, fps) = time(&live.header)?;
        let h = json!({"type":typ,"path":live.path,"graph":live.graph,"sources":sources.keys().collect::<Vec<_>>(),"materials":live.header["materials"],"frame":frame,"fps":fps,"overrides":overrides});
        self.shared
            .live_documents
            .lock()
            .unwrap()
            .entry(live.path.clone())
            .or_insert_with(|| live.graph.clone());
        self.dispatch(h, arrays, true)?;
        self.refresh(id);
        let live = &self.live[id];
        // Persist the graph accepted by the UI, including intervening native edits.
        fs::write(&live.path, serde_json::to_vec_pretty(&live.graph)?)?;
        Ok(())
    }
    fn dispatch(&self, message: Value, arrays: Arrays, wait: bool) -> Result<()> {
        let (tx, rx) = mpsc::channel();
        self.requests
            .try_send(LinkRequest {
                peer: self.peer,
                message,
                arrays,
                completion: wait.then_some(tx),
            })
            .map_err(|_| "editor request queue is full")?;
        if wait {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                if self.shared.stop.load(Ordering::Relaxed) {
                    return Err("native editor closed".into());
                }
                match rx.recv_timeout(Duration::from_millis(20)) {
                    Ok(result) => {
                        result.map_err(io::Error::other)?;
                        break;
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) if Instant::now() < deadline => {}
                    _ => return Err("native editor did not service live modifier request".into()),
                }
            }
        }
        Ok(())
    }
    fn remove(&mut self, id: &str) -> Result<()> {
        if let Some(live) = self.live.remove(id) {
            self.dispatch(
                json!({"type":"live_remove","path":live.path}),
                Arrays::new(),
                true,
            )?;
        }
        self.entries.remove(id);
        self.sources.remove(id);
        Ok(())
    }
    pub fn clear(&mut self) {
        for (_, live) in std::mem::take(&mut self.live) {
            let _ = self.dispatch(
                json!({"type":"live_remove","path":live.path}),
                Arrays::new(),
                false,
            );
        }
        self.entries.clear();
        self.sources.clear();
    }
}
fn apply_params(graph: &mut Value, params: &Value) -> Result<()> {
    if params.is_null() {
        return Ok(());
    }
    let params = params
        .as_object()
        .ok_or("params must be an object of exposed parameter values")?;
    let mut doc = Document::parse(graph.to_string())?;
    for (name, value) in params {
        let edit = link_param_edit(doc.value(), &json!({"name":name,"value":value}))?;
        doc.edit(&edit)?;
    }
    *graph = doc.value().clone();
    Ok(())
}
fn time(h: &Value) -> Result<(f64, f64)> {
    let number = |key: &str, default: f64| -> Result<f64> {
        Ok(match h.get(key) {
            None => default,
            Some(v) => v.as_f64().ok_or("frame and fps must be numeric")?,
        })
    };
    let (f, p) = (number("frame", 1.)?, number("fps", 24.)?);
    if !f.is_finite() || !p.is_finite() || p <= 0. {
        return Err("frame must be finite and fps positive".into());
    }
    Ok((f, p))
}
fn decode_geometry(h: &Value, a: &Arrays) -> Result<Geometry> {
    let geo = link_protocol::decode_object(h, a)?;
    if geo.p().iter().any(|p| !p.is_finite()) {
        return Err("input point positions must be finite".into());
    }
    Ok(geo)
}
fn validate_graph(v: &Value) -> Result<()> {
    fn forbidden(v: &Value) -> bool {
        match v {
            Value::Object(o) => {
                o.get("type").is_some_and(|s| s == "blender_object") || o.values().any(forbidden)
            }
            Value::Array(a) => a.iter().any(forbidden),
            _ => false,
        }
    }
    if forbidden(v) {
        return Err("modifier graphs cannot read blender_object; use modifier_input to prevent feedback loops".into());
    }
    let graph = Graph::from_value(v)?;
    validate_target(v, &graph.main_output()?)
}
fn validate_target(v: &Value, target: &str) -> Result<()> {
    let graph = Graph::from_value(v)?;
    let mut queue = vec![target.to_string()];
    let mut seen = std::collections::HashSet::new();
    let mut input = false;
    while let Some(id) = queue.pop() {
        if !seen.insert(id.clone()) {
            continue;
        }
        let n = graph
            .nodes
            .iter()
            .find(|(n, _)| *n == id)
            .map(|(_, n)| n)
            .ok_or("no such output node")?;
        input |= n.node_type == "modifier_input";
        for (_, ids) in &n.inputs {
            queue.extend(ids.iter().cloned());
        }
    }
    if !input {
        return Err("modifier graphs require a top-level modifier_input node; replacement-only generators are not stack modifiers".into());
    }
    Ok(())
}
fn same_param(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(a), Value::Number(b)) if a.is_f64() || b.is_f64() => {
            a.as_f64().map(|x| x as f32) == b.as_f64().map(|x| x as f32)
        }
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same_param(a, b))
        }
        _ => a == b,
    }
}
