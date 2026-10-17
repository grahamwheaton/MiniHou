//! Connection-owned stack evaluators. No access to the proxy session's external inputs.
use super::*;
use minihou_app::{graph_embed, link_interface, link_param_edit};
use minihou_core::{nodes::blender_input::ExternalInput, session::Session, Graph};

type Result<T> = minihou_app::Result<T>;
struct Entry {
    graph: Value,
    session: Session,
    input: Arrays,
    materials: Value,
    sources: BTreeMap<String, Arc<Geometry>>,
    source_meta: BTreeMap<String, Value>,
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
    /// graph_embed_v1: last graph both sides agreed on, per Blender network id.
    embed_base: BTreeMap<String, Value>,
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
            embed_base: BTreeMap::new(),
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
                .is_none_or(|s| !s.eq_ignore_ascii_case("json") && !s.eq_ignore_ascii_case("minihou"))
            {
                return Err("modifier graph must be a .json or .minihou file".into());
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
                    Arc::new(decode_geometry(h, &arrays)?),
                );
            }
            self.sources.insert(id.clone(), sources);
            if self.live.contains_key(&id) {
                self.refresh(&id);
                self.update_live(&id, "live_update", None)?;
            }
            return Ok((json!({"type":"instance_sources_set"}), Arrays::new()));
        }
        if typ == "graph_push" {
            return Ok((self.embed_push(&id, h)?, Arrays::new()));
        }
        if typ == "graph_pull" {
            return Ok((self.embed_pull(&id)?, Arrays::new()));
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
                    source_meta: BTreeMap::new(),
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
        let mut bindings: Vec<(String, String, String)> = entry
            .session
            .graph()
            .nodes
            .iter()
            .filter(|(_, n)| n.node_type == "object_input")
            .map(|(nid, n)| {
                let key = n.params
                    .as_ref()
                    .and_then(|p| p.get("binding"))
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .unwrap_or(nid)
                    .to_string();
                (format!("object_input:{key}"), key, "Choose an object in this modifier's Object input picker in Blender".to_string())
            })
            .collect();
        // Blender Object nodes read their implicit group picker (input:@picker:<id>); empty = waiting, never by name.
        bindings.extend(minihou_app::modifier_inputs::pickers(&graph).into_iter().map(|(node, uid)|
            (minihou_core::nodes::blender_input::picker_external(&node), minihou_app::modifier_inputs::picker_key(&uid), String::new())));
        for (external, key, empty) in bindings {
            // Keyed by store name too: a renamed node needs its feed set even when Blender's source is unchanged.
            let tag = format!("{external}|{key}");
            let incoming = feeds.and_then(|s| s.get(&key));
            let info = h["references"].get(&key).cloned().unwrap_or(Value::Null);
            let unchanged = entry
                .sources
                .get(&key)
                .zip(incoming)
                .is_some_and(|(a, b)| Arc::ptr_eq(a, b)) && entry.source_meta.get(&tag) == Some(&info)
                // Still empty: keep the cached waiting state instead of recooking every frame.
                || (incoming.is_none() && !entry.sources.contains_key(&key) && entry.source_meta.get(&tag) == Some(&info));
            if !unchanged {
                let matrix = info["matrix"].as_array().and_then(|a| (a.len() == 16).then(|| {
                    let values: Vec<_> = a.iter().filter_map(Value::as_f64).collect();
                    (values.len() == 16).then(|| std::array::from_fn(|i| values[i]))
                }).flatten());
                entry.session.set_external(external,incoming.map_or_else(||ExternalInput::Missing(empty),|g|ExternalInput::Object{geo:g.clone(),matrix,info:info.clone()}));
            }
            entry.source_meta.insert(tag, info);
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
        let cook = json!({"ms":start.elapsed().as_secs_f64()*1000.,"cooked":stats.cooked,"cached":stats.cached,"node_ms":stats.node_ms.into_iter().collect::<BTreeMap<_,_>>(),"waiting":stats.waiting.into_iter().collect::<BTreeMap<_,_>>(),"frame":frame,"fps":fps});
        let (mut header, mut output) =
            DiffState::default().build(&link_protocol::geometry_arrays(&result), &result, 1, &cook);
        header["type"] = json!("instance_result");
        header["interface"] = link_interface(&graph)["params"].clone();
        if h["references"].is_object() { header["input_references"] = h["references"].clone(); }
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
        let pending = self.shared.embed_pending.lock().unwrap().contains_key(id);
        let graph = live.graph.clone();
        if !pending {
            self.embed_base.insert(id.to_string(), graph.clone());
        }
        let live = &self.live[id];
        Ok(
            json!({"type":"instance_synced","live":true,"editor":"native","graph":live.graph,"interface":interface,"path":live.path,
                   "hash":graph_embed::graph_hash(&graph),"embed_pending":pending}),
        )
    }
    /// `graph_push`: Blender's embedded graph for network `id`. Replies `graph_ack` with a status:
    /// unchanged | accepted | app_newer (+ the app's graph) | conflict (the app asks the user).
    fn embed_push(&mut self, id: &str, h: &Value) -> Result<Value> {
        use graph_embed::Decision;
        let push = graph_embed::parse_push(h)?;
        validate_graph(&push.graph)?;
        self.refresh(id);
        let app = self.live.get(id).map(|l| l.graph.clone());
        let open_dirty = self.shared.open_dirty.load(Ordering::Relaxed);
        let decision = graph_embed::decide(&push.graph, app.as_ref(), self.embed_base.get(id), open_dirty);
        self.shared.embed_resolutions.lock().unwrap().remove(id);
        self.shared.embed_pending.lock().unwrap().remove(id);
        let mut ack = match decision {
            Decision::Unchanged => graph_embed::state("graph_ack", "unchanged", None),
            Decision::AdoptEmbedded => {
                if let Some(live) = self.live.get_mut(id) {
                    live.graph = push.graph.clone();
                    let path = live.path.clone();
                    self.dispatch(json!({"type":"live_replace","path":path,"graph":push.graph}), Arrays::new(), true)?;
                }
                graph_embed::state("graph_ack", "accepted", None)
            }
            Decision::KeepApp => graph_embed::state("graph_ack", "app_newer", app.as_ref()),
            Decision::Conflict | Decision::OpenDocumentDirty => {
                let kind = if decision == Decision::Conflict { "live" } else { "open_document" };
                let path = self.live.get(id).map(|l| l.path.clone()).unwrap_or_default();
                self.shared.embed_pending.lock().unwrap().insert(id.to_string(), push.graph.clone());
                self.dispatch(json!({"type":"embed_conflict","kind":kind,"path":path,"instance_id":id,
                    "label":push.label,"name":push.name,"graph":push.graph}), Arrays::new(), false)?;
                return Ok(graph_embed::state("graph_ack", "conflict", None));
            }
        };
        self.embed_base.insert(id.to_string(), match decision { Decision::KeepApp => app.unwrap(), _ => push.graph });
        ack["received"] = json!(push.size);
        Ok(ack)
    }
    /// `graph_pull`: the app's current graph for network `id` (`graph_state`), or the outcome of
    /// a pending conflict. Status: pending | resolved (+ resolution) | ok | none.
    fn embed_pull(&mut self, id: &str) -> Result<Value> {
        self.refresh(id);
        let app = self.live.get(id).map(|l| l.graph.clone());
        if self.shared.embed_pending.lock().unwrap().contains_key(id) {
            let Some(resolution) = self.shared.embed_resolutions.lock().unwrap().remove(id) else {
                return Ok(graph_embed::state("graph_state", "pending", None));
            };
            let Some(pushed) = self.shared.embed_pending.lock().unwrap().remove(id) else { return Ok(graph_embed::state("graph_state", "none", None)) };
            let adopt = graph_embed::Resolution::parse(&resolution).is_some_and(|r| r.adopts_embedded());
            match &app {
                Some(graph) => { self.embed_base.insert(id.to_string(), graph.clone()); }
                None if adopt => { self.embed_base.insert(id.to_string(), pushed); }
                None => {}
            }
            let mut out = graph_embed::state("graph_state", "resolved", app.as_ref());
            out["resolution"] = json!(resolution);
            return Ok(out);
        }
        Ok(match app {
            Some(graph) => {
                self.embed_base.insert(id.to_string(), graph.clone());
                graph_embed::state("graph_state", "ok", Some(&graph))
            }
            None => graph_embed::state("graph_state", "none", None),
        })
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
        let h = json!({"type":typ,"path":live.path,"label":live.header["label"],"graph":live.graph,"sources":sources.keys().collect::<Vec<_>>(),"materials":live.header["materials"],"frame":frame,"fps":fps,"overrides":overrides});
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
        self.embed_base.remove(id);
        self.shared.embed_pending.lock().unwrap().remove(id);
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
    let known: Vec<Value> = link_interface(graph)["params"].as_array().into_iter().flatten().map(|p| p["name"].clone()).collect();
    for (name, value) in params {
        // Unknown names stay an error, except a stale Blender Object picker value (an object name sent
        // under the node's old name right after a rename); the add-on resyncs from this cook's interface.
        let stale_picker = value.is_string(); // also after the node was removed
        if !known.iter().any(|k| k == name) && stale_picker { continue; }
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
/// `blender_object` nodes are allowed: instance cooks feed them only from their own group picker
/// (`picker:<node>`), never by object name, and the add-on refuses self/cache picks (feedback loops).
fn validate_graph(v: &Value) -> Result<()> {
    Graph::from_value(v)?.main_output()?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn push(id: &str, graph: &Value) -> Value {
        let text = graph_embed::canonical_text(graph);
        json!({"type":"graph_push","request_id":1,"instance_id":id,"schema":1,"name":"N","label":"Cube / MiniHou",
               "json":text,"hash":graph_embed::sha256_hex(text.as_bytes()),"size":text.len()})
    }

    #[test]
    fn graph_push_acks_validates_and_asks_before_replacing_a_modified_document() {
        let graph = json!({"nodes":{"input":{"type":"modifier_input"},"o":{"type":"output","inputs":{"geo":"input"}}},"display":"o"});
        let doc = Document::parse(graph.to_string()).unwrap();
        let server = LinkServer::bind("127.0.0.1:0", &doc, Path::new("g.json")).unwrap();
        let (sender, requests) = mpsc::sync_channel(4);
        let mut manager = InstanceManager::new(1, server.shared.clone(), sender);
        let (ack, _) = manager.handle(&push("A", &graph), &Arrays::new());
        assert_eq!((ack["type"].as_str(), ack["status"].as_str()), (Some("graph_ack"), Some("accepted")), "{ack}");
        assert_eq!(ack["instance_id"], "A");
        assert_eq!(manager.embed_base["A"]["version"], 1, "migrated to the current schema");
        let (state, _) = manager.handle(&json!({"type":"graph_pull","request_id":2,"instance_id":"A"}), &Arrays::new());
        assert_eq!((state["type"].as_str(), state["status"].as_str()), (Some("graph_state"), Some("none")));
        let mut bad = push("A", &graph);
        bad["hash"] = json!("deadbeef");
        let (err, _) = manager.handle(&bad, &Arrays::new());
        assert_eq!(err["type"], "instance_error");
        assert!(err["message"].as_str().unwrap().contains("hash"));
        assert!(validate_graph(&json!({"display":"b","nodes":{"b":{"type":"blender_object"}}})).is_ok(), "fed by its group picker");

        server.shared.open_dirty.store(true, Ordering::Relaxed);
        let (ack, _) = manager.handle(&push("B", &graph), &Arrays::new());
        assert_eq!(ack["status"], "conflict");
        let conflict = requests.try_recv().unwrap().message;
        assert_eq!((conflict["type"].as_str(), conflict["kind"].as_str()), (Some("embed_conflict"), Some("open_document")));
        assert_eq!(conflict["label"], "Cube / MiniHou");
        let pull = json!({"type":"graph_pull","request_id":3,"instance_id":"B"});
        assert_eq!(manager.handle(&pull, &Arrays::new()).0["status"], "pending");
        server.shared.embed_resolutions.lock().unwrap().insert("B".into(), "keep_mine".into());
        let (state, _) = manager.handle(&pull, &Arrays::new());
        assert_eq!((state["status"].as_str(), state["resolution"].as_str()), (Some("resolved"), Some("keep_mine")));
        assert!(!manager.embed_base.contains_key("B"), "a declined push is not an agreement");
        assert_eq!(manager.handle(&pull, &Arrays::new()).0["status"], "none");
    }

    #[test]
    fn independent_generator_instances_cook_with_empty_inputs_and_survive_relink() {
        let graph = json!({"nodes":{"box":{"type":"box"}},"display":"box",
            "exposed":[{"name":"Size","node":"box","param":"size"}]});
        let doc = Document::parse(graph.to_string()).unwrap();
        let server = LinkServer::bind("127.0.0.1:0", &doc, Path::new("generator.json")).unwrap();
        let (sender, _requests) = mpsc::sync_channel(4);
        let mut manager = InstanceManager::new(1, server.shared.clone(), sender);
        let empty = Geometry::from_arrays(vec![], vec![], vec![0], None).unwrap();
        let empty_arrays = link_protocol::geometry_arrays(&empty);
        let source = Geometry::from_arrays(vec![1., 2., 3.], vec![], vec![0], None).unwrap();
        let source_arrays = link_protocol::geometry_arrays(&source);

        let describe = manager.handle(&json!({"type":"instance_describe","graph":graph}), &Arrays::new());
        assert_eq!(describe.0["type"], "instance_description");
        for (request_id, id, input, size) in [(1, "A", &empty_arrays, 2.),
                                               (2, "B", &empty_arrays, 4.),
                                               (3, "A", &source_arrays, 2.),
                                               (4, "A", &empty_arrays, 2.)] {
            let (header, arrays) = manager.handle(&json!({"type":"instance_cook","request_id":request_id,
                "instance_id":id,"graph":graph,"params":{"Size":[size,2.,2.]}}), input);
            assert_eq!(header["type"], "instance_result", "{header}");
            assert_eq!(header["instance_id"], id);
            let geometry = link_protocol::decode_object(&header, &arrays).unwrap();
            assert!(geometry.npoints > 0, "generator must return geometry for {id}");
            let max_x = geometry.p().chunks_exact(3).map(|p| p[0]).fold(f32::NEG_INFINITY, f32::max);
            assert_eq!(max_x, size as f32 / 2.);
        }
        assert!(manager.entries.contains_key("A"));
        assert!(manager.entries.contains_key("B"));

        let empty_input_graph = json!({"nodes":{"input":{"type":"modifier_input"}},"display":"input"});
        let (header, arrays) = manager.handle(&json!({"type":"instance_cook","request_id":5,
            "instance_id":"empty-input","graph":empty_input_graph,"params":{}}), &empty_arrays);
        assert_eq!(header["type"], "instance_result", "{header}");
        assert_eq!(link_protocol::decode_object(&header, &arrays).unwrap().npoints, 0);
    }
}
