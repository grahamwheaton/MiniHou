//! Blender discovery and the process boundary for the native editor.

#[path = "link_instances.rs"]
pub(crate) mod link_instances;

#[path = "blender_link/install.rs"]
pub mod install;
#[allow(unused_imports)]
pub use install::*;

#[path = "blender_link/connection.rs"]
pub mod connection;
pub(crate) use connection::{join_until, send_frame, serve_client};

#[path = "blender_link/scene_sync.rs"]
pub mod scene_sync;

#[path = "blender_link/status.rs"]
pub mod status;

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
pub(crate) struct LinkSnapshot {
    pub(crate) document: Value,
    pub(crate) version: u64,
    pub(crate) interface: Value,
    pub(crate) watch: Value,
    pub(crate) extras: BTreeMap<String, Arc<Geometry>>,
    pub(crate) scene: Vec<(Value, Arrays)>,
    pub(crate) graph_path: String,
    pub(crate) geometry: Option<Arc<Geometry>>,
    pub(crate) seq: u64,
    pub(crate) good_seq: u64,
    pub(crate) cook: Value,
    pub(crate) error: Option<String>,
}
pub(crate) struct Peer {
    pub(crate) name: String,
    pub(crate) protocol: u64,
    pub(crate) replies: mpsc::Sender<Value>,
}
pub(crate) struct LinkShared {
    pub(crate) snapshot: Mutex<LinkSnapshot>,
    pub(crate) image_assets: Mutex<minihou_app::image_assets::Service>,
    pub(crate) asset_updates: Mutex<Vec<String>>,
    pub(crate) peers: Mutex<BTreeMap<u64, Peer>>,
    pub(crate) stop: AtomicBool,
    pub(crate) next_peer: AtomicU64,
    pub(crate) live_documents: Mutex<BTreeMap<String, Value>>,
    pub(crate) live_active: AtomicBool,
    /// graph_embed_v1: the user's conflict-modal answers, keyed by Blender network id.
    pub(crate) embed_resolutions: Mutex<BTreeMap<String, String>>,
    /// Pushed graphs waiting for that answer (shared: a reconnecting add-on still learns the outcome).
    pub(crate) embed_pending: Mutex<BTreeMap<String, Value>>,
    /// The UI's unrelated open document has unsaved edits (ask before a pushed graph takes over).
    pub(crate) open_dirty: AtomicBool,
}
pub struct LinkRequest {
    pub peer: u64,
    pub message: Value,
    pub arrays: Arrays,
    pub(crate) completion: Option<mpsc::Sender<std::result::Result<(), String>>>,
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
    pub fn texture_binding_ids(&self) -> Vec<String> {
        status::texture_binding_ids(self)
    }
    pub fn texture_status(&self) -> Vec<String> {
        status::texture_status(self)
    }
    pub fn apply_asset_updates(&self, registry: &mut minihou_app::materials::Registry) -> bool {
        status::apply_asset_updates(self, registry)
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
                // Give connected peers a brief chance to receive the normal
                // bye frame. A partial handshake still needs a forced wakeup.
                let grace = Instant::now() + Duration::from_millis(50);
                while clients.iter().any(|(_, client)| !client.is_finished()) && Instant::now() < grace {
                    thread::sleep(Duration::from_millis(2));
                }
                for (socket, client) in &clients {
                    if !client.is_finished() { let _ = socket.shutdown(Shutdown::Both); }
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
        scene_sync::restore_editor(self, editor)
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
        scene_sync::resolve_embed(self, conflict, resolution, editor)
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
        status::reply(self, peer, message);
    }
    /// The Blender graph path bound to the editor document: the live modifier's path while one is live.
    pub fn bound_path(&self) -> Option<&str> {
        self.active_live.as_deref()
    }
    pub fn reply_error(&self, peer: u64, where_: &str, message: &str) {
        status::reply_error(self, peer, where_, message);
    }
    pub fn document_changed(&self, doc: &Document, path: &Path) {
        scene_sync::document_changed(self, doc, path);
    }
    pub fn cooked<T>(&self, response: &minihou_app::CookResponse<T>) {
        status::cooked(self, response);
    }
    /// UI-thread boundary for live modifier documents. Original document/history stay in memory.
    /// All feeds are transient worker edits, never saved into the graph.
    pub fn handle_live(
        &mut self,
        request: &LinkRequest,
        editor: &mut minihou_app::EditorDocument,
    ) -> io::Result<Vec<minihou_app::Edit>> {
        scene_sync::handle_live(self, request, editor)
    }
    pub fn set_frame(&mut self, message: &Value) -> io::Result<bool> {
        status::set_frame(self, message)
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
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_start_has_no_blender_geometry_update() {
        let doc = Document::parse(include_str!("default_graph.json").to_owned()).unwrap();
        let server = LinkServer::bind("127.0.0.1:0", &doc, Path::new("untitled.json")).unwrap();
        let snapshot = server.shared.snapshot.lock().unwrap();
        assert!(snapshot.document["nodes"].as_object().unwrap().is_empty());
        assert!(snapshot.geometry.is_none());
        assert_eq!(snapshot.seq, 0);
    }

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
            output_range_count: 0,
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
