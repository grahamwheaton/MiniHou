//! Blender discovery and the process boundary for the native editor.

#[path = "blender_link/install.rs"]
pub mod install;
#[allow(unused_imports)]
pub use install::*;

pub(crate) static NEXT_TEMP: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

// The UI owns documents/history; socket threads only exchange immutable snapshots and requests.
use minihou_app::{
    link_protocol::{self, Arrays, DiffState},
    Document,
};
use minihou_core::Geometry;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    env, fs,
    io::{self, Write},
    net::{Shutdown, TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
#[derive(Clone)]
struct LinkSnapshot {
    document: Value,
    version: u64,
    interface: Value,
    watch: Value,
    extras: BTreeMap<String, Arc<Geometry>>,
    scene: Vec<(Value, Arrays)>,
    graph_path: String,
    geometry: Option<Arc<Geometry>>,
    seq: u64,
    good_seq: u64,
    cook: Value,
    error: Option<String>,
}
struct Peer {
    name: String,
    protocol: u64,
    replies: mpsc::Sender<Value>,
}
struct LinkShared {
    snapshot: Mutex<LinkSnapshot>,
    image_assets: Mutex<minihou_app::image_assets::Service>,
    asset_updates: Mutex<Vec<String>>,
    peers: Mutex<BTreeMap<u64, Peer>>,
    stop: AtomicBool,
    next_peer: AtomicU64,
    live_documents: Mutex<BTreeMap<String, Value>>,
    live_active: AtomicBool,
    /// graph_embed_v1: the user's conflict-modal answers, keyed by Blender network id.
    embed_resolutions: Mutex<BTreeMap<String, String>>,
    /// Pushed graphs waiting for that answer (shared: a reconnecting add-on still learns the outcome).
    embed_pending: Mutex<BTreeMap<String, Value>>,
    /// The UI's unrelated open document has unsaved edits (ask before a pushed graph takes over).
    open_dirty: AtomicBool,
}
pub struct LinkRequest {
    pub peer: u64,
    pub message: Value,
    pub arrays: Arrays,
    completion: Option<mpsc::Sender<std::result::Result<(), String>>>,
}
pub struct LinkServer {
    shared: Arc<LinkShared>,
    requests: mpsc::Receiver<LinkRequest>,
    listener: Option<thread::JoinHandle<()>>,
    pub frame: f64,
    pub fps: f64,
    saved_editor: Option<minihou_app::EditorDocument>,
    live_editors: BTreeMap<String, minihou_app::EditorDocument>,
    active_live: Option<String>,
    saved_time: Option<(f64, f64)>,
    address: std::net::SocketAddr,
}
impl LinkServer {
    pub fn texture_binding_ids(&self)->Vec<String> {
        self.shared.image_assets.lock().unwrap().manifests.values().flat_map(|m|
            m["bindings"].as_array().into_iter().flatten().filter_map(|v|v.as_str().map(str::to_owned))
        ).collect()
    }
    pub fn texture_status(&self)->Vec<String> {
        let service=self.shared.image_assets.lock().unwrap();
        let mut result=Vec::new();
        for (scope,manifest) in &service.manifests {
            if let Some(diagnostics)=manifest["diagnostics"].as_object() {
                for (slot,reason) in diagnostics {result.push(format!("{scope} / {slot}: unsupported: {}",reason.as_str().unwrap_or("unknown")));}
            }
            if let Some(images)=manifest["images"].as_array() {
                for hash in images.iter().filter_map(Value::as_str) {if !service.has(hash) {result.push(format!("{scope}: pending image {hash}"));}}
            }
        }
        result
    }
    pub fn apply_asset_updates(&self, registry:&mut minihou_app::materials::Registry)->bool {
        let updates=std::mem::take(&mut *self.shared.asset_updates.lock().unwrap());
        if updates.is_empty(){return false;}
        let service=self.shared.image_assets.lock().unwrap();
        for scope in updates {let _=service.bind(&scope,registry);}
        true
    }
    pub fn start(doc: &Document, path: &Path) -> io::Result<Self> {
        Self::bind("127.0.0.1:8765", doc, path)
    }
    pub fn bind(address: &str, doc: &Document, path: &Path) -> io::Result<Self> {
        let listener = TcpListener::bind(address)?;
        listener.set_nonblocking(true)?;
        let address = listener.local_addr()?;
        let shared = Arc::new(LinkShared {
            image_assets: Mutex::new(minihou_app::image_assets::Service::default()),
            asset_updates: Mutex::new(Vec::new()),
            snapshot: Mutex::new(LinkSnapshot {
                document: doc.value().clone(),
                version: 1,
                interface: minihou_app::link_interface(doc.value()),
                watch: link_protocol::watch_objects(doc.value()),
                extras: BTreeMap::new(),
                scene: Vec::new(),
                graph_path: path.to_string_lossy().into_owned(),
                geometry: None,
                seq: 0,
                good_seq: 0,
                cook: json!({}),
                error: None,
            }),
            peers: Mutex::new(BTreeMap::new()),
            stop: AtomicBool::new(false),
            next_peer: AtomicU64::new(1),
            live_documents: Mutex::new(BTreeMap::new()),
            live_active: AtomicBool::new(false),
            embed_resolutions: Mutex::new(BTreeMap::new()),
            embed_pending: Mutex::new(BTreeMap::new()),
            open_dirty: AtomicBool::new(false),
        });
        let (sender, requests) = mpsc::sync_channel(256);
        let state = shared.clone();
        let listener = thread::Builder::new()
            .name("minihou-link-listener".into())
            .spawn(move || {
                let mut clients: Vec<(TcpStream, thread::JoinHandle<()>)> = Vec::new();
                while !state.stop.load(Ordering::Relaxed) {
                    clients.retain(|(_, client)| !client.is_finished());
                    match listener.accept() {
                        Ok((socket, _)) => {
                            let Ok(shutdown) = socket.try_clone() else {
                                continue;
                            };
                            let state = state.clone();
                            let sender = sender.clone();
                            if let Ok(client) = thread::Builder::new()
                                .name("minihou-link-client".into())
                                .spawn(move || {
                                    let _ = serve_client(socket, state, sender);
                                })
                            {
                                clients.push((shutdown, client));
                            }
                        }
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(20))
                        }
                        Err(_) => break,
                    }
                }
                state.stop.store(true, Ordering::Relaxed);
                for (socket, _) in &clients {
                    let _ = socket.shutdown(Shutdown::Both);
                }
                // Release the port and wake every peer before waiting for cooks.
                // Rust cannot cancel a thread executing an arbitrary graph node.
                drop(listener);
                let deadline = Instant::now() + Duration::from_millis(100);
                for (_, client) in clients {
                    join_until(client, deadline);
                }
            })?;
        Ok(Self {
            shared,
            requests,
            listener: Some(listener),
            frame: 1.0,
            fps: 24.0,
            saved_editor: None,
            live_editors: BTreeMap::new(),
            active_live: None,
            saved_time: None,
            address,
        })
    }
    pub fn address(&self) -> std::net::SocketAddr {
        self.address
    }
    pub fn live_active(&self) -> bool {
        self.active_live.is_some()
    }
    pub fn restore_editor(
        &mut self,
        editor: &mut minihou_app::EditorDocument,
    ) -> Vec<minihou_app::Edit> {
        let Some(path) = self.active_live.clone() else {
            return vec![];
        };
        let request = LinkRequest {
            peer: 0,
            message: json!({"type":"live_remove","path":path}),
            arrays: Arrays::new(),
            completion: None,
        };
        self.handle_live(&request, editor).unwrap_or_default()
    }
    /// The UI reports whether its own (non-Blender) document has unsaved edits.
    pub fn set_open_dirty(&self, dirty: bool) {
        self.shared.open_dirty.store(dirty, Ordering::Relaxed);
    }
    /// Apply the user's answer to an `embed_conflict` request (graph_embed_v1). The add-on
    /// learns the outcome through `graph_pull`. Unsaved work is never overwritten here:
    /// `KeepMine` changes nothing, the others replace the bound document through its
    /// undo history (the caller exports the user's version first for `OpenAsNew`).
    pub fn resolve_embed(
        &mut self,
        conflict: &Value,
        resolution: minihou_app::graph_embed::Resolution,
        editor: &mut minihou_app::EditorDocument,
    ) -> io::Result<Vec<minihou_app::Edit>> {
        let id = conflict["instance_id"].as_str().unwrap_or("").to_string();
        let mut edits = Vec::new();
        if resolution.adopts_embedded() && conflict["kind"] == "live" {
            let request = LinkRequest {
                peer: 0,
                message: json!({"type":"live_replace","path":conflict["path"],"graph":conflict["graph"]}),
                arrays: Arrays::new(),
                completion: None,
            };
            edits = self.handle_live(&request, editor)?;
        }
        self.shared.embed_resolutions.lock().unwrap().insert(id, resolution.as_str().into());
        Ok(edits)
    }
    pub fn poll(&self) -> Vec<LinkRequest> {
        self.requests.try_iter().collect()
    }
    pub fn clients(&self) -> Vec<String> {
        self.shared
            .peers
            .lock()
            .unwrap()
            .values()
            .map(|p| format!("{} (protocol {})", p.name, p.protocol))
            .collect()
    }
    pub fn listening(&self) -> bool {
        !self.shared.stop.load(Ordering::Relaxed)
            && self
                .listener
                .as_ref()
                .is_some_and(|listener| !listener.is_finished())
    }
    pub fn reply(&self, peer: u64, message: Value) {
        if let Some(p) = self.shared.peers.lock().unwrap().get(&peer) {
            let _ = p.replies.send(message);
        }
    }
    /// The Blender graph path bound to the editor document: the live modifier's path while one is live.
    pub fn bound_path(&self) -> Option<&str> {
        self.active_live.as_deref()
    }
    pub fn reply_error(&self, peer: u64, where_: &str, message: &str) {
        if let Some(p) = self.shared.peers.lock().unwrap().get(&peer) {
            let _ = p
                .replies
                .send(json!({"type":"error","where":where_,"message":message}));
        }
    }
    pub fn document_changed(&self, doc: &Document, path: &Path) {
        let interface = minihou_app::link_interface(doc.value());
        let watch = link_protocol::watch_objects(doc.value());
        let path = path.to_string_lossy();
        if let Some(key) = &self.active_live {
            self.shared
                .live_documents
                .lock()
                .unwrap()
                .insert(key.clone(), doc.value().clone());
        }
        let mut snapshot = self.shared.snapshot.lock().unwrap();
        if snapshot.document != *doc.value()
            || snapshot.interface != interface
            || snapshot.graph_path.as_str() != path.as_ref()
            || snapshot.watch != watch
        {
            if snapshot.graph_path.as_str() != path.as_ref() {
                snapshot.geometry = None;
                snapshot.error = None;
            }
            snapshot.document = doc.value().clone();
            snapshot.watch = watch;
            snapshot.interface = interface;
            snapshot.graph_path = path.into_owned();
            snapshot.version += 1;
        }
    }
    pub fn cooked<T>(&self, response: &minihou_app::CookResponse<T>) {
        let mut snapshot = self.shared.snapshot.lock().unwrap();
        // Sequence numbers belong to this listener, not a document's worker. File Open
        // starts a new worker whose revision counter can be lower than the previous one.
        snapshot.version += 1;
        snapshot.seq += 1;
        snapshot.error = response.error.clone();
        if response.geometry.is_none() {
            return;
        } // retain the last good result for new subscribers/resync
        snapshot.good_seq = snapshot.seq;
        snapshot.extras = response.extras.clone();
        snapshot.scene = response.scene.clone();
        snapshot.geometry = response.geometry.clone();
        snapshot.cook = json!({"ms":response.milliseconds,"cooked":response.stats.cooked,"cached":response.stats.cached,
            "node_ms":response.stats.node_ms.iter().cloned().collect::<BTreeMap<_,_>>(),"frame":self.frame,"fps":self.fps});
    }
    /// UI-thread boundary for live modifier documents. Original document/history stay in memory.
    /// All feeds are transient worker edits, never saved into the graph.
    pub fn handle_live(
        &mut self,
        request: &LinkRequest,
        editor: &mut minihou_app::EditorDocument,
    ) -> io::Result<Vec<minihou_app::Edit>> {
        let result = (|| -> minihou_app::Result<Vec<minihou_app::Edit>> {
            use minihou_app::Edit;
            use minihou_core::nodes::blender_input::ExternalInput;
            let h = &request.message;
            let key = h["path"].as_str().ok_or("missing live path")?.to_owned();
            let current = self.active_live.as_deref() == Some(&key);
            if h["type"] == "live_remove" {
                self.shared.live_documents.lock().unwrap().remove(&key);
                self.live_editors.remove(&key);
                if current {
                    self.active_live = None;
                    self.shared.live_active.store(false, Ordering::Relaxed);
                    if let Some(original) = self.saved_editor.take() {
                        *editor = original;
                        self.document_changed(&editor.doc, &editor.path);
                        let (frame, fps) = self.saved_time.take().unwrap_or((1., 24.));
                        self.frame = frame;
                        self.fps = fps;
                        return Ok(vec![
                            Edit::Replace(editor.doc.value().clone()),
                            Edit::Time { frame, fps },
                        ]);
                    }
                }
                return Ok(vec![]);
            }
            if h["type"] == "live_replace" {
                // graph_embed_v1: Blender's embedded graph replaces the bound document (undoable).
                let doc = Document::parse(h["graph"].to_string())?;
                self.shared.live_documents.lock().unwrap().insert(key.clone(), doc.value().clone());
                if current {
                    editor.history.replace(&mut editor.doc, doc);
                    self.document_changed(&editor.doc, &editor.path);
                    return Ok(vec![Edit::Replace(editor.doc.value().clone())]);
                }
                if let Some(cached) = self.live_editors.get_mut(&key) {
                    cached.history.replace(&mut cached.doc, doc);
                }
                return Ok(vec![]);
            }
            if h["type"] != "live_activate" && !current {
                if let Some(overrides)=h["overrides"].as_object() {
                    let graph=self.shared.live_documents.lock().unwrap().get(&key).cloned().unwrap_or(h["graph"].clone());
                    let mut doc=Document::parse(graph.to_string())?;
                    for(name,value)in overrides { let edit=minihou_app::link_param_edit(doc.value(),&json!({"name":name,"value":value}))?;doc.edit(&edit)?; }
                    self.shared.live_documents.lock().unwrap().insert(key.clone(),doc.value().clone());
                    if let Some(cached)=self.live_editors.get_mut(&key){cached.history.replace(&mut cached.doc,doc);}
                }
                return Ok(vec![]);
            }
            // Input updates never replace a graph edited in the native app. Merge only
            // explicitly changed Blender controls into the current document on this thread.
            let mut doc=if current {editor.doc.clone()}else{Document::parse(h["graph"].to_string())?};
            if let Some(overrides)=h["overrides"].as_object(){for(name,value)in overrides{
                let edit=minihou_app::link_param_edit(doc.value(),&json!({"name":name,"value":value}))?;doc.edit(&edit)?;
            }}
            // Validate every transient feed before changing the selected document.
            let input = request
                .arrays
                .iter()
                .filter(|(k, _)| !k.starts_with("source:"))
                .map(|(k, a)| (k.clone(), a.clone()))
                .collect();
            let geo = link_protocol::decode_object(h, &input)?;
            let mut edits = vec![Edit::External {
                name: "modifier_input".into(),
                input: ExternalInput::Object {
                    geo: Arc::new(geo),
                    matrix: None,
                    info: Value::Null,
                },
            }];
            for (nid, n) in doc.value()["nodes"].as_object().into_iter().flatten() {
                if n["type"] == "object_input" {
                    let binding = n["params"]["binding"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .unwrap_or(nid);
                    edits.push(Edit::External {
                        name: format!("object_input:{binding}"),
                        input: ExternalInput::Missing(
                            "Choose an object in this modifier's Object input picker in Blender"
                                .into(),
                        ),
                    });
                }
            }
            // Blender Object nodes follow their group picker while the modifier is live (empty: waiting).
            let pickers = minihou_app::modifier_inputs::pickers(doc.value());
            for (node, _) in &pickers {
                edits.push(Edit::External { name: minihou_core::nodes::blender_input::picker_external(node), input: ExternalInput::Missing(String::new()) });
            }
            if let Some(sources) = h["sources"].as_array() {
                for (i, name) in sources.iter().enumerate() {
                    let prefix = format!("source:{i}/");
                    let arrays = request
                        .arrays
                        .iter()
                        .filter_map(|(k, a)| {
                            k.strip_prefix(&prefix).map(|k| (k.to_string(), a.clone()))
                        })
                        .collect();
                    if let Some((node, _)) = pickers.iter().find(|(_, uid)| minihou_app::modifier_inputs::picker_key(uid) == name.as_str().unwrap_or("")) {
                        edits.push(Edit::External {
                            name: minihou_core::nodes::blender_input::picker_external(node),
                            input: ExternalInput::Object { geo: Arc::new(link_protocol::decode_object(&Value::Null, &arrays)?), matrix: minihou_app::modifier_inputs::reference_matrix(&h["references"][name.as_str().unwrap_or("")]), info: h["references"][name.as_str().unwrap_or("")].clone() },
                        });
                    }
                    edits.push(Edit::External {
                        name: format!("object_input:{}", name.as_str().ok_or("bad source name")?),
                        input: ExternalInput::Object {
                            geo: Arc::new(link_protocol::decode_object(h, &arrays)?),
                            matrix: None,
                            info: Value::Null,
                        },
                    });
                }
            }
            let graph_changed = !current || editor.doc.value() != doc.value();
            if !current {
                let mut new = self.live_editors.remove(&key).unwrap_or_else(|| {
                    minihou_app::EditorDocument::new(doc.clone(), PathBuf::from(&key))
                });
                new.history.replace(&mut new.doc, doc);
                if let Some(label) = h["label"].as_str().filter(|s| !s.is_empty()) {
                    new.blender_label = Some(label.chars().take(200).collect());
                }
                let old = std::mem::replace(editor, new);
                if let Some(old_key) = self.active_live.take() {
                    self.live_editors.insert(old_key, old);
                } else if self.saved_editor.is_none() {
                    self.saved_editor = Some(old);
                    self.saved_time = Some((self.frame, self.fps));
                }
            } else {
                editor.history.replace(&mut editor.doc, doc);
            }
            self.active_live = Some(key);
            self.shared.live_active.store(true, Ordering::Relaxed);
            self.document_changed(&editor.doc, &editor.path);
            if graph_changed {
                edits.insert(0, Edit::Replace(editor.doc.value().clone()));
            }
            let (frame, fps) = (
                h["frame"].as_f64().unwrap_or(1.),
                h["fps"].as_f64().unwrap_or(24.),
            );
            self.frame = frame;
            self.fps = fps;
            edits.push(Edit::Time { frame, fps });
            Ok(edits)
        })();
        if let Some(completion) = &request.completion {
            let _ = completion.send(result.as_ref().map(|_| ()).map_err(|e| e.to_string()));
        }
        result.map_err(|e| io::Error::other(e.to_string()))
    }
    pub fn set_frame(&mut self, message: &Value) -> io::Result<bool> {
        let frame = match message.get("frame") {
            Some(v) => v.as_f64().ok_or_else(|| io::Error::other("bad frame"))?,
            None => 1.0,
        };
        let fps = match message.get("fps").filter(|v| !v.is_null()) {
            Some(v) => v.as_f64().ok_or_else(|| io::Error::other("bad fps"))?,
            None => self.fps,
        };
        if !frame.is_finite() || !fps.is_finite() || fps <= 0.0 {
            return Err(io::Error::other("frame must be finite and fps positive"));
        }
        let changed = frame != self.frame || fps != self.fps;
        self.frame = frame;
        self.fps = fps;
        Ok(changed)
    }
}
impl Drop for LinkServer {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        // Release the listening socket before a Restart Link action binds the same port.
        if let Some(listener) = self.listener.take() {
            join_until(listener, Instant::now() + Duration::from_millis(250));
        }
    }
}

/// Join only finished workers, with one shared deadline for a whole shutdown.
/// Dropping an unfinished JoinHandle detaches it; it cannot retain the listener
/// or a usable peer socket, and must never hold up application exit.
fn join_until(worker: thread::JoinHandle<()>, deadline: Instant) {
    while !worker.is_finished() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(2));
    }
    if worker.is_finished() {
        let _ = worker.join();
    }
}

#[path = "link_instances.rs"]
mod link_instances;

fn send_frame(socket: &mut TcpStream, header: &Value, arrays: &Arrays) -> io::Result<()> {
    socket.write_all(&link_protocol::encode_message(header, arrays)?)
}
fn serve_client(
    mut socket: TcpStream,
    state: Arc<LinkShared>,
    requests: mpsc::SyncSender<LinkRequest>,
) -> io::Result<()> {
    // Windows accepted sockets inherit the listener's nonblocking mode. The framing
    // reader uses read_exact, so both handshake and reader thread require blocking I/O.
    socket.set_nonblocking(false)?;
    socket.set_nodelay(true)?;
    socket.set_write_timeout(Some(Duration::from_secs(2)))?;
    // A stalled/partial hello cannot leave a reader behind forever.
    socket.set_read_timeout(Some(Duration::from_secs(10)))?;
    let (hello, _) = link_protocol::read_message(&mut socket, 1024 * 1024)?;
    if hello["type"] != "hello" {
        return Err(io::Error::other("expected hello"));
    }
    if hello["protocol"].as_u64() != Some(link_protocol::PROTOCOL_VERSION as u64) {
        send_frame(
            &mut socket,
            &json!({"type":"error","message":"unsupported protocol version"}),
            &Arrays::new(),
        )?;
        return Ok(());
    }
    let snapshot = state.snapshot.lock().unwrap().clone();
    send_frame(
        &mut socket,
        &json!({"type":"welcome","protocol":link_protocol::PROTOCOL_VERSION,"server":"minihou",
        "capabilities":["instances_v1","live_modifier_v1","object_inputs_v1","scene_objects_v1","image_exchange_v1","viewport_materials_v1","image_assets_v1",minihou_app::graph_embed::CAPABILITY,minihou_app::input_define::CAPABILITY],"editor_url":null,"editor":"native","graph_path":snapshot.graph_path}),
        &Arrays::new(),
    )?;
    let instance_mode = hello["mode"] == "instances";
    if !instance_mode {
        send_frame(&mut socket, &snapshot.watch, &Arrays::new())?;
    }
    socket.set_read_timeout(None)?;
    let mut reader = socket.try_clone()?;
    let (incoming, received) = mpsc::sync_channel(32);
    let reader_state = state.clone();
    let reader_thread = thread::Builder::new()
        .name("minihou-link-reader".into())
        .spawn(move || loop {
            let mut message = link_protocol::read_message(&mut reader, 512 * 1024 * 1024);
            let failed = message.is_err();
            loop {
                if reader_state.stop.load(Ordering::Relaxed) {
                    return;
                }
                match incoming.try_send(message) {
                    Ok(()) => break,
                    Err(mpsc::TrySendError::Disconnected(_)) => return,
                    Err(mpsc::TrySendError::Full(value)) => {
                        message = value;
                        thread::sleep(Duration::from_millis(2));
                    }
                }
            }
            if failed { break; }
        })?;
    let id = state.next_peer.fetch_add(1, Ordering::Relaxed);
    let (replies, reply_rx) = mpsc::channel();
    state.peers.lock().unwrap().insert(
        id,
        Peer {
            name: hello["client"].as_str().unwrap_or("?").into(),
            protocol: hello["protocol"].as_u64().unwrap_or(1),
            replies,
        },
    );
    let mut instances = link_instances::InstanceManager::new(id, state.clone(), requests.clone());
    // object_names_v1: the session connection of a capable add-on answers object-name searches.
    let names_capable = !instance_mode && hello["capabilities"].as_array().is_some_and(|a| a.iter().any(|v| v == minihou_app::object_names::CAPABILITY));
    if names_capable { if let Ok(mut hub) = minihou_app::object_names::HUB.lock() { hub.capable_peers += 1; } }
    let result = (|| -> io::Result<()> {
        let mut diff = DiffState::default();
        let mut version = 0;
        let mut seq = None;
        let mut good_seq = None;
        let mut interface = Value::Null;
        let mut watch = snapshot.watch.clone();
        let mut extra_diffs: BTreeMap<String, DiffState> = BTreeMap::new();
        let mut scene_keys: BTreeMap<String, Value> = BTreeMap::new();
        loop {
            if state.stop.load(Ordering::Relaxed) {
                break;
            }
            match received.recv_timeout(Duration::from_millis(16)) {
                Ok(Ok((h, arrays))) => match h["type"].as_str().unwrap_or("") {
                    "object_names" if names_capable => {
                        if let Ok(mut hub) = minihou_app::object_names::HUB.lock() { if hub.incoming.len() < 64 { hub.incoming.push(h); } }
                    }
                    "material_manifest" if hello["capabilities"].as_array().is_some_and(|a|a.iter().any(|v|v=="viewport_materials_v1")) => {
                        let scope=h["scope"].as_str().unwrap_or("");
                        let scoped=format!("peer{id}:{scope}");
                        let revision=h["revision"].as_u64().unwrap_or(0);
                        let hashes:Vec<String>=h["images"].as_array().into_iter().flatten().filter_map(|v|v.as_str().map(str::to_owned)).collect();
                        if !scope.is_empty() && revision>0 {
                            if h.to_string().len()>1024*1024 || h["descriptors"].as_object().is_none_or(|v|v.len()>4096)
                                || h["bindings"].as_array().is_none_or(|v|v.len()>4096)
                                || hashes.len()>4096 {
                                send_frame(&mut socket,&json!({"type":"error","where":"material_manifest","message":"invalid or oversized material manifest"}),&Arrays::new())?;
                                continue;
                            }
                            let mut service=state.image_assets.lock().unwrap();
                            let mut manifest=h.clone();
                            if let Some(desc)=manifest["descriptors"].as_object_mut() {
                                let renamed=std::mem::take(desc).into_iter().map(|(key,value)|(format!("peer{id}:{key}"),value)).collect();
                                *desc=renamed;
                            }
                            if let Some(bindings)=manifest["bindings"].as_array_mut() {for binding in bindings {if let Some(key)=binding.as_str(){*binding=json!(format!("peer{id}:{key}"));}}}
                            if service.manifest(&scoped,revision,manifest) {
                                let missing=service.missing(&hashes);
                                drop(service);
                                state.asset_updates.lock().unwrap().push(scoped);
                                send_frame(&mut socket,&json!({"type":"image_request","scope":scope,"revision":revision,"hashes":missing}),&Arrays::new())?;
                                if let Some(instance)=scope.strip_prefix("instance:") {
                                    let bindings={state.image_assets.lock().unwrap().manifests
                                        .get(&format!("peer{id}:{scope}")).and_then(|m|m.get("bindings")).cloned()};
                                    if let Some(bindings)=bindings {
                                        let _=instances.rebind_manifest(instance,&bindings);
                                    }
                                }
                            }
                        }
                    }
                    "image_begin" | "image_chunk" | "image_end" | "image_error" if hello["capabilities"].as_array().is_some_and(|a|a.iter().any(|v|v=="image_assets_v1")) => {
                        let id=h["transfer_id"].as_str().unwrap_or("");
                        let scoped_id=format!("peer{id}:{id}");
                        let mut service=state.image_assets.lock().unwrap();
                        let result=match h["type"].as_str().unwrap_or("") {
                            "image_begin"=>service.begin(&scoped_id,h["hash"].as_str().unwrap_or(""),h["total"].as_u64().and_then(|n|usize::try_from(n).ok()).unwrap_or(0)).map(|_|None),
                            "image_chunk"=>arrays.get("data").ok_or_else(||"missing data array".to_string()).and_then(|a|if a.dtype=="|u1" && a.bytes.len()<=minihou_app::image_assets::CHUNK {service.chunk(&scoped_id,h["offset"].as_u64().and_then(|n|usize::try_from(n).ok()).unwrap_or(usize::MAX),&a.bytes)}else{Err("invalid chunk array".into())}).map(|_|None),
                            "image_end"=>service.end(&scoped_id).map(Some),
                            _=>{service.cancel(&scoped_id);Ok(None)},
                        };
                        let affected=if result.as_ref().is_ok_and(Option::is_some) {service.manifests.keys().cloned().collect::<Vec<_>>()} else {Vec::new()};
                        drop(service);
                        match result {
                            Ok(Some(hash))=>{
                                state.asset_updates.lock().unwrap().extend(affected);
                                send_frame(&mut socket,&json!({"type":"image_ack","transfer_id":id,"hash":hash}),&Arrays::new())?;
                            },
                            Ok(None)=>{},
                            Err(message)=>send_frame(&mut socket,&json!({"type":"image_error","transfer_id":id,"message":message}),&Arrays::new())?,
                        }
                    }
                    "ping" => send_frame(
                        &mut socket,
                        &json!({"type":"pong","t":h["t"]}),
                        &Arrays::new(),
                    )?,
                    "resync" => {
                        diff.reset();
                        extra_diffs.clear();
                        scene_keys.clear();
                        version = 0;
                        seq = None;
                        good_seq = None;
                        interface = Value::Null;
                        watch = Value::Null;
                    }
                    typ if typ.starts_with("instance_") || typ.starts_with("graph_") => {
                        let typ=typ.to_owned();
                        let mut h=h;
                        if matches!(typ.as_str(),"instance_cook"|"instance_edit"|"instance_sources") && h.get("manifest_revision").is_some() {
                            let scope=format!("peer{id}:instance:{}",h["instance_id"].as_str().unwrap_or(""));
                            let service=state.image_assets.lock().unwrap();
                            if service.revisions.get(&scope).copied()!=h["manifest_revision"].as_u64() {
                                drop(service);
                                send_frame(&mut socket,&json!({"type":"instance_error","request_id":h["request_id"],"instance_id":h["instance_id"],"message":"geometry manifest revision is unavailable"}),&Arrays::new())?;
                                continue;
                            }
                            if let Some(bindings)=service.manifests.get(&scope).and_then(|v|v.get("bindings")).filter(|b|b.as_array().is_some_and(|a|!a.is_empty())) {
                                h["material_bindings"]=json!({"version":1,"slots":bindings});
                            }
                        }
                        let (header, output) = if instance_mode {
                            instances.handle(&h, &arrays)
                        } else {
                            (
                                json!({"type":"instance_error","request_id":h["request_id"],"instance_id":h["instance_id"],"message":"instance requests require hello mode instances"}),
                                Arrays::new(),
                            )
                        };
                        send_frame(&mut socket, &header, &output)?;
                    }
                    "set_param" | "set_frame" | "object_geo" | "object_missing" | "image_data"
                    | "image_missing" => {
                        let mut h=h;
                        if h["type"]=="object_geo" && h.get("manifest_revision").is_some() {
                            let scope=format!("peer{id}:object:{}",h["object"].as_str().unwrap_or(""));
                            let service=state.image_assets.lock().unwrap();
                            if service.revisions.get(&scope).copied()!=h["manifest_revision"].as_u64() {
                                drop(service);
                                send_frame(&mut socket,&json!({"type":"error","where":"material_manifest","message":"geometry manifest revision is unavailable"}),&Arrays::new())?;
                                continue;
                            }
                            if let Some(bindings)=service.manifests.get(&scope).and_then(|v|v.get("bindings")).filter(|b|b.as_array().is_some_and(|a|!a.is_empty())) {
                                h["material_bindings"]=json!({"version":1,"slots":bindings});
                            }
                        }
                        if state.live_active.load(Ordering::Relaxed) && h["type"] == "set_param" {
                            send_frame(
                                &mut socket,
                                &json!({"type":"error","where":"set_param","message":"Use the selected modifier controls or live editor"}),
                                &Arrays::new(),
                            )?;
                            continue;
                        }
                        if requests
                            .try_send(LinkRequest {
                                peer: id,
                                message: h,
                                arrays,
                                completion: None,
                            })
                            .is_err()
                        {
                            send_frame(
                                &mut socket,
                                &json!({"type":"error","message":"editor request queue is full"}),
                                &Arrays::new(),
                            )?;
                        }
                    }
                    // input_define_v1: applied on the UI thread as an undoable edit; it always answers
                    // with input_define_ack (status ok|error) carrying this request_id/instance_id.
                    "input_define" => {
                        if requests.try_send(LinkRequest { peer: id, message: h.clone(), arrays, completion: None }).is_err() {
                            send_frame(&mut socket, &minihou_app::input_define::error(&h, "busy", "editor request queue is full"), &Arrays::new())?;
                        }
                    }
                    "geo_ack" => {} // TCP needs no acknowledgement; some addon versions send one anyway.
                    _ => send_frame(
                        &mut socket,
                        &json!({"type":"error","message":"unsupported message type"}),
                        &Arrays::new(),
                    )?,
                },
                Ok(Err(e)) => return Err(e),
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            for reply in reply_rx.try_iter() {
                send_frame(&mut socket, &reply, &Arrays::new())?;
            }
            if names_capable {
                let queries = minihou_app::object_names::HUB.lock().map(|mut hub| std::mem::take(&mut hub.outgoing)).unwrap_or_default();
                for query in queries { send_frame(&mut socket, &query, &Arrays::new())?; }
            }
            if instance_mode || state.live_active.load(Ordering::Relaxed) {
                continue;
            }
            let snapshot = state.snapshot.lock().unwrap().clone();
            if version == snapshot.version {
                continue;
            }
            version = snapshot.version;
            if watch != snapshot.watch {
                send_frame(&mut socket, &snapshot.watch, &Arrays::new())?;
                watch = snapshot.watch.clone();
            }
            if interface != snapshot.interface || seq != Some(snapshot.seq) {
                send_frame(&mut socket, &snapshot.interface, &Arrays::new())?;
                interface = snapshot.interface;
            }
            if seq != Some(snapshot.seq) {
                if let Some(geo) = snapshot
                    .geometry
                    .filter(|_| good_seq != Some(snapshot.good_seq))
                {
                    let shown = link_protocol::output_geometry(&geo).map_err(io::Error::other)?;
                    let arrays = link_protocol::geometry_arrays(&shown);
                    let (header, changed) =
                        diff.build(&arrays, &shown, snapshot.good_seq, &snapshot.cook);
                    send_frame(&mut socket, &header, &changed)?;
                    let names: Vec<_> = snapshot.extras.keys().cloned().collect();
                    send_frame(
                        &mut socket,
                        &json!({"type":"outputs","names":names}),
                        &Arrays::new(),
                    )?;
                    extra_diffs.retain(|name, _| snapshot.extras.contains_key(name));
                    for (name, g) in &snapshot.extras {
                        let shown = link_protocol::output_geometry(g).map_err(io::Error::other)?;
                        let (mut h, a) = extra_diffs.entry(name.clone()).or_default().build(
                            &link_protocol::geometry_arrays(&shown),
                            &shown,
                            snapshot.good_seq,
                            &snapshot.cook,
                        );
                        h["output"] = json!(name);
                        send_frame(&mut socket, &h, &a)?;
                    }
                    good_seq = Some(snapshot.good_seq);
                    let nodes: Vec<_> = snapshot
                        .scene
                        .iter()
                        .filter_map(|(h, _)| h["node"].as_str())
                        .collect();
                    send_frame(
                        &mut socket,
                        &json!({"type":"scene_nodes","nodes":nodes}),
                        &Arrays::new(),
                    )?;
                    scene_keys.retain(|node, _| nodes.contains(&node.as_str()));
                    for (h, a) in &snapshot.scene {
                        if let Some(node) = h["node"].as_str() {
                            if scene_keys.get(node) != Some(&h["key"]) {
                                send_frame(&mut socket, h, a)?;
                                scene_keys.insert(node.into(), h["key"].clone());
                            }
                        }
                    }
                }
                if let Some(error) = snapshot.error {
                    send_frame(
                        &mut socket,
                        &json!({"type":"error","where":"cook","seq":snapshot.seq,"message":error}),
                        &Arrays::new(),
                    )?;
                }
                seq = Some(snapshot.seq);
            }
        }
        Ok(())
    })();
    // Wake read_exact and release a full incoming queue before any cleanup.
    let _ = socket.shutdown(Shutdown::Both);
    drop(received);
    join_until(reader_thread, Instant::now() + Duration::from_millis(50));
    instances.clear();
    state.image_assets.lock().unwrap().remove_peer(&format!("peer{id}:"));
    state.peers.lock().unwrap().remove(&id);
    if names_capable { if let Ok(mut hub) = minihou_app::object_names::HUB.lock() { hub.capable_peers = hub.capable_peers.saturating_sub(1); } }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blender_input_reset_transform_uses_live_reference_matrix() {
        let graph = json!({"display":"b","nodes":{"b":{"type":"blender_object"}}});
        let doc = Document::parse(graph.to_string()).unwrap();
        let mut editor = minihou_app::EditorDocument::new(doc.clone(), PathBuf::from("object.json"));
        let mut server = LinkServer::bind("127.0.0.1:0", &doc, Path::new("object.json")).unwrap();
        let source = Geometry::points_only(vec![1., 0., 0.]).unwrap();
        let key = minihou_app::modifier_inputs::picker_key("b");
        let mut arrays = link_protocol::geometry_arrays(&Geometry::default());
        arrays.extend(link_protocol::geometry_arrays(&source).into_iter()
            .map(|(name, array)| (format!("source:0/{name}"), array)));
        let mut references = serde_json::Map::new();
        references.insert(key.clone(), json!({"matrix":[1.,0.,0.,5.,0.,1.,0.,0.,0.,0.,1.,0.,0.,0.,0.,1.]}));
        let message = json!({"type":"live_activate","path":"object.json","graph":graph,
            "sources":[key],"references":references});
        let edits = server.handle_live(&LinkRequest { peer:0, message, arrays, completion:None }, &mut editor).unwrap();
        let mut session = editor.doc.session().unwrap();
        for edit in edits {
            if let minihou_app::Edit::External { name, input } = edit { session.set_external(name, input); }
        }
        assert_eq!(session.cook(None).unwrap().p(), &[6., 0., 0.]);
        let edit = minihou_app::Edit::Param { node:"b".into(), name:"reset_transform".into(), value:json!(true) };
        editor.doc.edit(&edit).unwrap();
        session.set_param("b", "reset_transform", json!(true)).unwrap();
        assert_eq!(session.cook(None).unwrap().p(), &[1., 0., 0.]);
    }

    #[test]
    fn image_transport_requests_verifies_and_reuses_on_reconnect() {
        use sha2::{Digest,Sha256};
        let doc=Document::parse(json!({"version":1,"nodes":{"box":{"type":"box"}},"display":"box"}).to_string()).unwrap();
        let server=LinkServer::bind("127.0.0.1:0",&doc,Path::new("graph.json")).unwrap();
        let mut bytes=Vec::new();
        image::DynamicImage::new_rgba8(2,2).write_to(&mut std::io::Cursor::new(&mut bytes),image::ImageFormat::Png).unwrap();
        let hash=format!("sha256:{:x}",Sha256::digest(&bytes));
        for first in [true,false] {
            let mut socket=TcpStream::connect(server.address).unwrap();
            socket.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            send_frame(&mut socket,&json!({"type":"hello","protocol":1,"client":"test","capabilities":["viewport_materials_v1","image_assets_v1"]}),&Arrays::new()).unwrap();
            let _=receive(&mut socket,"welcome");
            let _=receive(&mut socket,"watch_objects");
            let manifest=json!({"type":"material_manifest","scope":"object:Cube","revision":1,"images":[hash],"descriptors":{},"bindings":[]});
            send_frame(&mut socket,&manifest,&Arrays::new()).unwrap();
            let request=receive(&mut socket,"image_request").0;
            assert_eq!(request["hashes"].as_array().unwrap().len(),usize::from(first));
            if first {
                send_frame(&mut socket,&json!({"type":"image_begin","transfer_id":"t","hash":hash,"total":bytes.len()}),&Arrays::new()).unwrap();
                let mut arrays=Arrays::new();
                arrays.insert("data".into(),link_protocol::Array {dtype:"|u1".into(),shape:vec![bytes.len()],bytes:Arc::new(bytes.clone())});
                send_frame(&mut socket,&json!({"type":"image_chunk","transfer_id":"t","offset":0}),&arrays).unwrap();
                send_frame(&mut socket,&json!({"type":"image_end","transfer_id":"t"}),&Arrays::new()).unwrap();
                assert_eq!(receive(&mut socket,"image_ack").0["hash"],hash);
            }
            drop(socket);
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    #[test]
    fn linked_quad_stays_one_polygon_through_transform_and_output() {
        let graph = json!({"display":"out","nodes":{
            "input":{"type":"modifier_input"},
            "move":{"type":"transform","inputs":{"geo":"input"},"params":{"translate":[1,0,0]}},
            "out":{"type":"output","inputs":{"geo":"move"}}
        }});
        let original = Document::parse(graph.to_string()).unwrap();
        let mut editor = minihou_app::EditorDocument::new(original.clone(), PathBuf::from("quad.json"));
        let mut server = LinkServer::bind("127.0.0.1:0", &original, Path::new("quad.json")).unwrap();
        let quad = Geometry::from_arrays(
            vec![-1.,-1.,0., 1.,-1.,0., 1.,1.,0., -1.,1.,0.],
            vec![0,1,2,3], vec![0,4], None).unwrap();
        let request = LinkRequest { peer:0,
            message:json!({"type":"live_activate","path":"quad.json","graph":graph,"params":{}}),
            arrays:link_protocol::geometry_arrays(&quad), completion:None };
        let edits = server.handle_live(&request, &mut editor).unwrap();
        let mut session = editor.doc.session().unwrap();
        let mut target = session.graph().main_output().unwrap();
        let requests = edits.into_iter().enumerate().map(|(i, edit)| minihou_app::CookRequest { revision:i as u64 + 1, edit }).collect();
        let cooked = minihou_app::cook_batch(&mut session, &mut target, requests, |_| ());
        assert!(cooked.error.is_none(), "{:?}", cooked.error);
        let geo = cooked.geometry.unwrap();
        assert_eq!(geo.nprims(), 1);
        assert_eq!(geo.topology.prim_offsets.as_slice(), [0,4]);
        let display = minihou_core::preview::layout(geo.p(), &geo.topology.vertex_point,
            &geo.topology.prim_offsets, geo.topology.closed.as_deref(), geo.npoints,
            usize::MAX, usize::MAX, 100_000, 8192);
        assert_eq!(display.tri.len() / 3, 2, "rendering may use two fan triangles");
        assert_eq!(display.edge.len() / 2, 4, "display edges must exclude the fan diagonal");
    }
    #[test]
    fn live_modifier_bindings_reach_cooked_document_and_rebind() {
        let graph=json!({"nodes":{"input":{"type":"modifier_input"},"move":{"type":"transform","inputs":{"geo":"input"}},"out":{"type":"output","inputs":{"geo":"move"}}},"display":"out"});
        let doc=Document::parse(graph.to_string()).unwrap();
        let mut editor=minihou_app::EditorDocument::new(doc.clone(),PathBuf::from("plane.json"));
        let mut server=LinkServer::bind("127.0.0.1:0",&doc,Path::new("plane.json")).unwrap();
        let mut plane=Geometry::from_arrays(vec![0.,0.,0.,1.,0.,0.,1.,1.,0.,0.,1.,0.],vec![0,1,2,3],vec![0,4],Some(vec![true])).unwrap();
        plane.prim.insert("material_index".into(),Arc::new(minihou_core::Attr::i32(1,vec![0])));
        plane.vertex.insert("uv".into(),Arc::new(minihou_core::Attr::f32(2,vec![0.,0.,1.,0.,1.,1.,0.,1.])));
        let arrays=link_protocol::geometry_arrays(&plane);
        let mut session=doc.session().unwrap(); let mut target="out".to_owned();
        for (revision,id) in [(1,"peer1:blender:plane:0"),(2,"peer1:blender:plane:new:0")] {
            let edits=server.handle_live(&LinkRequest {peer:1,message:json!({"type":"live_activate","path":"plane.json","graph":graph,"materials":["imagepaste-261017-142014"],"material_bindings":{"version":1,"slots":[id]}}),arrays:arrays.clone(),completion:None},&mut editor).unwrap();
            let response=minihou_app::cook_batch(&mut session,&mut target,edits.into_iter().map(|edit|minihou_app::CookRequest {revision,edit}).collect(),|_|());
            assert!(response.error.is_none(),"{:?}",response.error);
            let cooked=response.geometry.unwrap();
            assert_eq!(cooked.detail["materials"],json!(["imagepaste-261017-142014"]));
            assert_eq!(cooked.detail["material_bindings"]["slots"],json!([id]));
            assert!(cooked.vertex.contains_key("uv"));
        }
    }

    fn receive(socket: &mut TcpStream, kind: &str) -> (Value, Arrays) {
        // Allows interface/geo broadcasts to interleave with solicited pong replies.
        for _ in 0..16 {
            let message = link_protocol::read_message(socket, 1024 * 1024).unwrap();
            if message.0["type"] == kind {
                return message;
            }
        }
        panic!("missing {kind}");
    }
    fn connect(server: &LinkServer) -> TcpStream {
        let mut socket = TcpStream::connect(server.address).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        send_frame(
            &mut socket,
            &json!({"type":"hello","protocol":1,"client":"Blender test","capabilities":["shm_v1"]}),
            &Arrays::new(),
        )
        .unwrap();
        let welcome = receive(&mut socket, "welcome").0;
        assert_eq!(welcome["protocol"], 1);
        assert_eq!(
            welcome["capabilities"],
            json!([
                "instances_v1",
                "live_modifier_v1",
                "object_inputs_v1",
                "scene_objects_v1",
                "image_exchange_v1",
                "viewport_materials_v1",
                "image_assets_v1",
                "graph_embed_v1",
                "input_define_v1"
            ])
        );
        socket
    }
    #[test]
    fn input_define_is_forwarded_to_the_ui_and_answered_with_its_request_id() {
        let doc = Document::parse(r#"{"nodes":{"input":{"type":"modifier_input"}},"display":"input"}"#.into()).unwrap();
        let server = LinkServer::bind("127.0.0.1:0", &doc, Path::new("graph.json")).unwrap();
        let mut a = connect(&server);
        let msg = json!({"type":"input_define","request_id":7,"instance_id":"m","path":"graph.json","op":"add",
                         "input":{"name":"Target","kind":"object"}});
        send_frame(&mut a, &msg, &Arrays::new()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        let request = loop {
            if let Some(r) = server.poll().into_iter().next() { break r; }
            assert!(Instant::now() < deadline, "input_define was not forwarded");
            thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(request.message["type"], "input_define");
        let (edit, entry) = minihou_app::input_define::plan(doc.value(), &request.message).unwrap();
        assert!(matches!(edit, minihou_app::Edit::ModifierInputs { .. }));
        server.reply(request.peer, minihou_app::input_define::ack(&request.message, &entry, doc.value()));
        let ack = receive(&mut a, "input_define_ack").0;
        assert_eq!((ack["request_id"].as_i64(), ack["instance_id"].as_str(), ack["input"]["kind"].as_str()), (Some(7), Some("m"), Some("object")));
        assert_eq!(server.bound_path(), None);
    }
    #[test]
    fn proxy_handshake_requests_resync_errors_and_disconnect() {
        let doc =
            Document::parse(r#"{"nodes":{"b":{"type":"box"}},"display":"b"}"#.into()).unwrap();
        let server = LinkServer::bind("127.0.0.1:0", &doc, Path::new("graph.json")).unwrap();
        let mut session = doc.session().unwrap();
        let mut target = "b".into();
        let response = minihou_app::cook_batch(
            &mut session,
            &mut target,
            vec![minihou_app::CookRequest {
                revision: 1,
                edit: minihou_app::Edit::Replace(doc.value().clone()),
            }],
            |_| (),
        );
        assert!(response.error.is_none());
        server.cooked(&response);
        let mut a = connect(&server);
        assert_eq!(receive(&mut a, "interface").0["params"], json!([]));
        let (geo, arrays) = receive(&mut a, "geo");
        assert_eq!(geo["full"], true);
        assert!(arrays.contains_key("point/P"));
        send_frame(&mut a, &json!({"type":"ping","t":123}), &Arrays::new()).unwrap();
        assert_eq!(receive(&mut a, "pong").0["t"], 123);
        assert_eq!(server.clients(), vec!["Blender test (protocol 1)"]);
        send_frame(
            &mut a,
            &json!({"type":"set_param","node":"b","param":"size","value":[2,2,2]}),
            &Arrays::new(),
        )
        .unwrap();
        let request = server
            .requests
            .recv_timeout(Duration::from_secs(3))
            .unwrap();
        assert_eq!(request.message["param"], "size");
        server.reply_error(request.peer, "set_param", "test error");
        assert_eq!(receive(&mut a, "error").0["message"], "test error");
        send_frame(
            &mut a,
            &json!({"type":"set_frame","frame":12,"fps":30}),
            &Arrays::new(),
        )
        .unwrap();
        assert_eq!(
            server
                .requests
                .recv_timeout(Duration::from_secs(3))
                .unwrap()
                .message["frame"],
            12
        );
        send_frame(&mut a, &json!({"type":"resync"}), &Arrays::new()).unwrap();
        receive(&mut a, "interface");
        assert_eq!(receive(&mut a, "geo").0["full"], true);
        let mut b = connect(&server);
        receive(&mut b, "interface");
        assert_eq!(receive(&mut b, "geo").0["full"], true);
        let error = minihou_app::CookResponse::<()> {
            revision: 2,
            geometry: None,
            preview: None,
            lod: None,
            prepared: None,
            extras: Default::default(),
            scene: Vec::new(),
            display_ranges: Vec::new(),
            stats: Default::default(),
            error: Some("bad cook".into()),
            milliseconds: 1.0,
        };
        server.cooked(&error);
        assert_eq!(receive(&mut a, "error").0["message"], "bad cook");
        assert_eq!(receive(&mut b, "error").0["where"], "cook");
        drop(a);
        drop(b);
        let deadline = Instant::now() + Duration::from_secs(3);
        while !server.clients().is_empty() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        assert!(server.clients().is_empty());
    }
    #[test]
    fn frame_validation_and_change_detection() {
        let doc =
            Document::parse(r#"{"nodes":{"b":{"type":"box"}},"display":"b"}"#.into()).unwrap();
        let mut server = LinkServer::bind("127.0.0.1:0", &doc, Path::new("g.json")).unwrap();
        assert!(server.set_frame(&json!({"frame":10,"fps":30})).unwrap());
        assert!(!server.set_frame(&json!({"frame":10})).unwrap());
        assert!(server.set_frame(&json!({"frame":"bad"})).is_err());
        assert!(server.set_frame(&json!({"frame":11,"fps":0})).is_err());
        assert_eq!((server.frame, server.fps), (10.0, 30.0));
    }

    #[test]
    fn named_outputs_scene_manifests_and_binary_input_requests() {
        let doc = Document::parse(
            json!({"display":"p","nodes":{
                "p":{"type":"point"},
                "extra":{"type":"output","params":{"object":"Separate"},"inputs":{"geo":"p"}},
                "cloud":{"type":"scene_objects","params":{"mode":"points"},"inputs":{"geo":"p"}},
                "source":{"type":"blender_object","params":{"object":"Cube"}}
            }})
            .to_string(),
        )
        .unwrap();
        let server = LinkServer::bind("127.0.0.1:0", &doc, Path::new("g.json")).unwrap();
        let mut session = doc.session().unwrap();
        let mut target = "p".into();
        let response = minihou_app::cook_batch(
            &mut session,
            &mut target,
            vec![minihou_app::CookRequest {
                revision: 1,
                edit: minihou_app::Edit::Replace(doc.value().clone()),
            }],
            |_| (),
        );
        assert!(response.error.is_none());
        server.cooked(&response);
        let mut socket = connect(&server);
        assert_eq!(
            receive(&mut socket, "watch_objects").0["objects"],
            json!(["Cube"])
        );
        let main = receive(&mut socket, "geo").0;
        assert!(main.get("output").is_none());
        assert_eq!(
            receive(&mut socket, "outputs").0["names"],
            json!(["Separate"])
        );
        let named = receive(&mut socket, "geo").0;
        assert_eq!(named["output"], "Separate");
        assert_eq!(named["full"], true);
        assert_eq!(
            receive(&mut socket, "scene_nodes").0["nodes"],
            json!(["cloud"])
        );
        let (manifest, a) = receive(&mut socket, "scene_objects");
        assert_eq!(manifest["mode"], "points");
        assert!(a.contains_key("P"));
        let input = Geometry::points_only(vec![1., 2., 3.]).unwrap();
        let arrays = link_protocol::geometry_arrays(&input);
        send_frame(
            &mut socket,
            &json!({"type":"object_geo","object":"Cube"}),
            &arrays,
        )
        .unwrap();
        let request = server
            .requests
            .recv_timeout(Duration::from_secs(3))
            .unwrap();
        assert_eq!(request.arrays, arrays);
        let edit = link_protocol::external_edit(&request.message, &request.arrays).unwrap();
        let response = minihou_app::cook_batch(
            &mut session,
            &mut "source".into(),
            vec![minihou_app::CookRequest { revision: 2, edit }],
            |_| (),
        );
        assert_eq!(response.geometry.unwrap().p(), &[1., 2., 3.]);
        send_frame(&mut socket, &json!({"type":"resync"}), &Arrays::new()).unwrap();
        receive(&mut socket, "geo");
        receive(&mut socket, "outputs");
        assert_eq!(receive(&mut socket, "geo").0["full"], true);
    }

    #[test]
    fn proxy_waits_for_fragmented_hello_and_control_frames() {
        let doc =
            Document::parse(r#"{"nodes":{"b":{"type":"box"}},"display":"b"}"#.into()).unwrap();
        let server = LinkServer::bind("127.0.0.1:0", &doc, Path::new("g.json")).unwrap();
        let mut socket = TcpStream::connect(server.address).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        // Leave the handler waiting for both the rest of the prefix and the payload.
        for header in [
            json!({"type":"hello","protocol":1}),
            json!({"type":"ping","t":42}),
        ] {
            let frame = link_protocol::encode_message(&header, &Arrays::new()).unwrap();
            socket.write_all(&frame[..5]).unwrap();
            thread::sleep(Duration::from_millis(50));
            socket.write_all(&frame[5..16]).unwrap();
            thread::sleep(Duration::from_millis(50));
            socket.write_all(&frame[16..]).unwrap();
            if header["type"] == "hello" {
                receive(&mut socket, "welcome");
                receive(&mut socket, "watch_objects");
                receive(&mut socket, "interface");
            } else {
                assert_eq!(receive(&mut socket, "pong").0["t"], 42);
            }
        }
    }

    #[test]
    fn occupied_port_returns_bind_error() {
        // Exercise the same bind path as start(), without needing port 8765 to be free.
        let occupied = TcpListener::bind("127.0.0.1:0").unwrap();
        let doc =
            Document::parse(r#"{"nodes":{"b":{"type":"box"}},"display":"b"}"#.into()).unwrap();
        let error = LinkServer::bind(
            &occupied.local_addr().unwrap().to_string(),
            &doc,
            Path::new("g.json"),
        )
        .err()
        .expect("binding an occupied port must fail");
        assert_eq!(error.kind(), io::ErrorKind::AddrInUse);
    }

    #[test]
    fn subscriber_sequences_survive_a_new_documents_worker_revision_counter() {
        let doc=Document::parse(json!({"nodes":{"b":{"type":"box"}},"display":"b"}).to_string()).unwrap();
        let server=LinkServer::bind("127.0.0.1:0",&doc,Path::new("graph.json")).unwrap();
        let mut session=doc.session().unwrap();let mut target="b".to_string();
        let response=minihou_app::cook_batch(&mut session,&mut target,vec![minihou_app::CookRequest{revision:100,edit:minihou_app::Edit::Replace(doc.value().clone())}],|_|());
        server.cooked(&response);
        let mut client=connect(&server);receive(&mut client,"interface");
        let first=receive(&mut client,"geo").0["seq"].as_u64().unwrap();
        let response=minihou_app::cook_batch(&mut session,&mut target,vec![minihou_app::CookRequest{revision:1,edit:minihou_app::Edit::Param{node:"b".into(),name:"size".into(),value:json!([2,2,2])}}],|_|());
        server.cooked(&response);
        let next=receive(&mut client,"geo").0["seq"].as_u64().unwrap();
        assert!(next>first);
    }

    #[test]
    fn dropping_listener_releases_port_for_restart() {
        let doc =
            Document::parse(r#"{"nodes":{"b":{"type":"box"}},"display":"b"}"#.into()).unwrap();
        let server = LinkServer::bind("127.0.0.1:0", &doc, Path::new("graph.json")).unwrap();
        let address = server.address.to_string();
        let _client = connect(&server);
        assert!(server.listening());
        drop(server);
        let _restarted = LinkServer::bind(&address, &doc, Path::new("graph.json")).unwrap();
    }

    #[test]
    fn shutdown_closes_partial_handshake_and_pending_live_rpc_promptly() {
        use std::io::Read;
        let doc = Document::parse(r#"{"nodes":{},"display":null}"#.into()).unwrap();
        let server = LinkServer::bind("127.0.0.1:0", &doc, Path::new("graph.json")).unwrap();
        let address = server.address();
        let mut partial = TcpStream::connect(address).unwrap();
        partial.write_all(b"MHL1\x10").unwrap();
        let mut rpc = TcpStream::connect(address).unwrap();
        rpc.set_read_timeout(Some(Duration::from_millis(500))).unwrap();
        send_frame(&mut rpc, &json!({"type":"hello","protocol":1,"mode":"instances"}), &Arrays::new()).unwrap();
        receive(&mut rpc, "welcome");
        send_frame(&mut rpc, &json!({"type":"instance_edit","request_id":1,"instance_id":"pending",
            "graph":{"nodes":{"i":{"type":"modifier_input"}},"display":"i"},"params":{}}),
            &link_protocol::geometry_arrays(&Geometry::points_only(vec![]).unwrap())).unwrap();
        // Deliberately do not service the UI request: the worker is waiting for it.
        let deadline = Instant::now() + Duration::from_millis(500);
        while server.requests.try_recv().is_err() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(2));
        }
        assert!(Instant::now() < deadline, "live RPC never reached the UI queue");
        let started = Instant::now();
        drop(server);
        assert!(started.elapsed() < Duration::from_millis(300));
        for socket in [&mut partial, &mut rpc] {
            socket.set_read_timeout(Some(Duration::from_millis(300))).unwrap();
            let mut buffer = [0; 4096];
            // Already-buffered welcome/error bytes may precede the terminal EOF/RST.
            loop {
                match socket.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(_) => {},
                    Err(e) if matches!(e.kind(), io::ErrorKind::ConnectionReset | io::ErrorKind::ConnectionAborted) => break,
                    Err(e) => panic!("peer did not receive EOF/RST: {e}"),
                }
            }
        }
        let _restarted = LinkServer::bind(&address.to_string(), &doc, Path::new("graph.json")).unwrap();
    }

    #[test]
    fn shutdown_does_not_join_an_uncooperative_cook_forever() {
        let (release, wait) = mpsc::channel::<()>();
        let worker = thread::spawn(move || { let _ = wait.recv(); });
        let started = Instant::now();
        join_until(worker, started + Duration::from_millis(20));
        assert!(started.elapsed() < Duration::from_millis(100));
        release.send(()).unwrap();
    }

}
