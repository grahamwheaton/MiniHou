//! Blender discovery and the process boundary for the native editor.
use std::{
    env,
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant},
};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

#[path = "addon_archive.rs"]
mod addon_archive;

fn version(path: &Path) -> Option<(u32, u32)> {
    let text = path.to_string_lossy();
    text.split(['/', '\\']).find_map(|part| {
        let suffix = part
            .strip_prefix("Blender ")
            .or_else(|| part.strip_prefix("Blender-"))?;
        let (major, minor) = suffix.split_once('.')?;
        Some((major.parse().ok()?, minor.parse().ok()?))
    })
}

fn label(path: &Path) -> String {
    match version(path) {
        Some((major, minor)) => format!("Blender {major}.{minor}"),
        None => "Blender ?".to_owned(),
    }
}

fn add_exe(found: &mut Vec<(String, PathBuf)>, path: PathBuf) {
    if path.is_file() && !found.iter().any(|(_, p)| p == &path) {
        found.push((label(&path), path));
    }
}

/// Scan explicit executables and the installation roots used by the Python editor.
/// Keeping the roots injectable avoids depending on the machine's Blender installation in tests.
pub fn find_installs_in(roots: &[PathBuf]) -> Vec<(String, PathBuf)> {
    let mut found = Vec::new();
    for root in roots {
        add_exe(&mut found, root.clone());
        add_exe(&mut found, root.join("blender.exe"));
        add_exe(&mut found, root.join("blender"));
        for parent in [
            root.join("Blender Foundation"),
            root.join("Programs/Blender Foundation"),
        ] {
            if let Ok(entries) = fs::read_dir(parent) {
                let mut paths: Vec<_> = entries
                    .flatten()
                    .map(|e| e.path().join("blender.exe"))
                    .collect();
                paths.sort_by_key(|p| std::cmp::Reverse(version(p)));
                for path in paths {
                    add_exe(&mut found, path);
                }
            }
        }
        add_exe(
            &mut found,
            root.join("Steam/steamapps/common/Blender/blender.exe"),
        );
    }
    found
}

pub fn find_installs() -> Vec<(String, PathBuf)> {
    let mut roots = Vec::new();
    for name in ["BLENDER", "BLENDER_EXE"] {
        if let Some(path) = env::var_os(name) {
            roots.push(PathBuf::from(path));
        }
    }
    if let Some(path) = env::var_os("PATH") {
        roots.extend(env::split_paths(&path));
    }
    #[cfg(windows)]
    for name in [
        "ProgramFiles",
        "ProgramW6432",
        "LOCALAPPDATA",
        "ProgramFiles(x86)",
    ] {
        if let Some(path) = env::var_os(name) {
            roots.push(PathBuf::from(path));
        }
    }
    #[cfg(not(windows))]
    roots.extend([
        PathBuf::from("/usr/bin"),
        PathBuf::from("/snap/bin"),
        PathBuf::from("/opt"),
    ]);
    find_installs_in(&roots)
}

fn addon_zip_in(dirs: &[PathBuf]) -> Option<PathBuf> {
    for dir in dirs {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        let mut zips: Vec<_> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
                    return false;
                };
                let name = name.to_ascii_lowercase();
                p.is_file()
                    && name.ends_with(".zip")
                    && (name.starts_with("minihou_link") || name.starts_with("minihou-link"))
            })
            .collect();
        zips.sort();
        if let Some(zip) = zips.pop() {
            return Some(zip);
        }
    }
    None
}

pub fn addon_zip() -> Option<PathBuf> {
    let exe = env::current_exe().ok()?;
    let dir = exe.parent()?;
    let mut dirs = vec![dir.to_path_buf(), dir.join("dist")];
    if let Some(root) = Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(3) {
        dirs.push(root.join("dist"));
    }
    addon_zip_in(&dirs)
}

fn addon_source_in(roots: &[PathBuf]) -> Option<PathBuf> {
    roots
        .iter()
        .map(|root| root.join("blender_addon/minihou_link"))
        .find(|path| path.join("__init__.py").is_file())
}

/// Keep generated archives in the OS temp directory for Blender startup and manual installation.
pub fn prepare_addon_zip() -> io::Result<PathBuf> {
    let mut roots = Vec::new();
    for start in [
        env::current_dir().ok(),
        env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf)),
        Some(PathBuf::from(env!("CARGO_MANIFEST_DIR"))),
    ]
    .into_iter()
    .flatten()
    {
        roots.extend(start.ancestors().map(Path::to_path_buf));
    }
    // A checkout must package its current source, even if an older release ZIP is nearby.
    let Some(source) = addon_source_in(&roots) else {
        return addon_zip().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound,
            "No MiniHou_Link*.zip or blender_addon/minihou_link/__init__.py found beside the app or in its source tree"));
    };
    let temp = TempDir::new()?;
    let zip = temp.0.join("MiniHou_Link.zip");
    addon_archive::build(&source, &zip)?;
    // These archives also back the Save ZIP action and must outlive the helper process.
    std::mem::forget(temp);
    Ok(zip)
}

/// Blender's startup script loads the packaged add-on, sets the graph and connects to the
/// same default endpoint as tools/launch_blender.py. The native host must provide that endpoint.
fn open_args(graph: &Path, zip: Option<&Path>, report: &Path) -> Vec<OsString> {
    let graph =
        serde_json::to_string(graph.to_string_lossy().as_ref()).expect("path is serializable");
    let zip = zip.map(|p| {
        serde_json::to_string(p.to_string_lossy().as_ref()).expect("path is serializable")
    });
    let source = match zip {
        Some(zip) => format!(
            r#"import bpy, importlib, sys, tempfile, zipfile
from pathlib import Path
d = Path(tempfile.mkdtemp(prefix='minihou_link_'))
with zipfile.ZipFile({zip}) as z: z.extractall(d)
sys.path.insert(0, str(d))
import minihou_link
try: minihou_link.unregister()
except Exception: pass
importlib.reload(minihou_link)
minihou_link.register()
bpy.context.scene.minihou_graph_path = {graph}
bpy.context.window_manager.minihou_host = '127.0.0.1'
bpy.context.window_manager.minihou_port = 8765
def minihou_connect():
    from minihou_link import runtime
    runtime.get_runtime().connect('127.0.0.1', 8765)
    return None
bpy.app.timers.register(minihou_connect, first_interval=0.5)
"#
        ),
        None => format!(
            r#"import bpy, addon_utils
addon_utils.enable('minihou_link', default_set=False)
bpy.context.scene.minihou_graph_path = {graph}
bpy.context.window_manager.minihou_host = '127.0.0.1'
bpy.context.window_manager.minihou_port = 8765
def minihou_connect():
    from minihou_link import runtime
    runtime.get_runtime().connect('127.0.0.1', 8765)
    return None
bpy.app.timers.register(minihou_connect, first_interval=0.5)
"#
        ),
    };
    let source = source.replace(
        "bpy.app.timers.register(minihou_connect,",
        "bpy.app.timers.register(lambda: minihou_guard(minihou_connect),",
    );
    let source = serde_json::to_string(&source).expect("script is serializable");
    let report =
        serde_json::to_string(report.to_string_lossy().as_ref()).expect("path is serializable");
    let guarded = format!(
        r#"import json, time, traceback
from pathlib import Path
def minihou_report(ok, error=''):
    path = Path({report})
    temporary = path.with_suffix('.tmp')
    temporary.write_text(json.dumps(dict(ok=ok, error=error)), encoding='utf-8')
    temporary.replace(path)
def minihou_guard(callback):
    try: return callback()
    except Exception:
        minihou_report(False, traceback.format_exc())
        return None
def minihou_bootstrap():
    exec({source}, globals())
    deadline = time.monotonic() + 30
    def check():
        if Path({report}).exists(): return None
        from minihou_link import runtime
        rt = runtime.get_runtime()
        if rt.state == 'connected':
            minihou_report(True)
            return None
        if time.monotonic() >= deadline:
            minihou_report(False, rt.detail or rt.last_error or 'Timed out connecting to 127.0.0.1:8765')
            return None
        return 0.25
    bpy.app.timers.register(lambda: minihou_guard(check), first_interval=1.0)
minihou_guard(minihou_bootstrap)
"#
    );
    vec![OsString::from("--python-expr"), OsString::from(guarded)]
}

/// Version of the bundled Blender add-on (bl_info in blender_addon/minihou_link/__init__.py), e.g. "0.1.5".
pub fn addon_version() -> String {
    const SOURCE: &str = include_str!("../../../../blender_addon/minihou_link/__init__.py");
    SOURCE.lines().find(|l| l.trim_start().starts_with("\"version\"")).and_then(|l| {
        let inner = l.split('(').nth(1)?.split(')').next()?;
        Some(inner.split(',').map(|n| n.trim()).filter(|n| !n.is_empty()).collect::<Vec<_>>().join("."))
    }).unwrap_or_else(|| "unknown".into())
}

pub fn open_blender(exe: &Path, graph: &Path) -> io::Result<Child> {
    if !graph.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "graph file does not exist",
        ));
    }
    let zip = prepare_addon_zip()?;
    let temp = TempDir::new()?;
    let report = temp.0.join("launch.json");
    let log = temp.0.join("blender.log");
    let output = fs::File::create(&log)?;
    let mut command = Command::new(exe);
    command
        .args(open_args(graph, Some(&zip), &report))
        .stdin(Stdio::null())
        .stderr(output.try_clone()?)
        .stdout(output);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0000_0008 | 0x0000_0200);
    }
    let mut child = command.spawn()?;
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if let Some(status) = child.try_wait()? {
            return Err(io::Error::other(format!(
                "Blender exited with {status}: {}",
                fs::read_to_string(&log).unwrap_or_default()
            )));
        }
        if report.is_file() {
            let result: serde_json::Value = serde_json::from_slice(&fs::read(&report)?)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            if result["ok"] == true {
                return Ok(child);
            }
            let error = result["error"]
                .as_str()
                .unwrap_or("Blender startup failed")
                .to_owned();
            thread::spawn(move || {
                let _ = child.wait();
            });
            return Err(io::Error::other(error));
        }
        if Instant::now() >= deadline {
            thread::spawn(move || {
                let _ = child.wait();
            });
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Blender did not report a connection within 60 s",
            ));
        }
        thread::sleep(Duration::from_millis(100));
    }
}

/// A launch snapshot lives until the launched Blender exits; it never changes the editor's save point.
pub struct LaunchGraph {
    pub path: PathBuf,
    _temporary: Option<TempDir>,
}
pub fn prepare_launch_graph(doc: &Document, path: &Path, dirty: bool) -> io::Result<LaunchGraph> {
    if path.is_file() && !dirty {
        return Ok(LaunchGraph {
            path: path.canonicalize()?,
            _temporary: None,
        });
    }
    let temporary = TempDir::new()?;
    let path = temporary.0.join("current-document.json");
    fs::write(&path, doc.text())?;
    Ok(LaunchGraph {
        path,
        _temporary: Some(temporary),
    })
}

const INSTALL_SCRIPT: &str = r#"import json, os, shutil, sys, zipfile, traceback
import bpy
zip_path, out_path = sys.argv[sys.argv.index('--') + 1:][:2]
res = {'ok': False, 'blender': bpy.app.version_string}
try:
    import addon_utils
    with zipfile.ZipFile(zip_path) as z:
        if 'minihou_link/__init__.py' not in z.namelist():
            raise RuntimeError('ZIP must contain minihou_link/__init__.py')
    base = bpy.utils.user_resource('SCRIPTS', path='addons', create=True)
    target = os.path.join(base, 'minihou_link')
    addon_utils.disable('minihou_link', default_set=True)
    if os.path.isdir(target): shutil.rmtree(target)
    with zipfile.ZipFile(zip_path) as z: z.extractall(base)
    bpy.utils.refresh_script_paths()
    addon_utils.modules(refresh=True)
    errors = []
    mod = addon_utils.enable('minihou_link', default_set=True, persistent=True,
        handle_error=lambda _ex: errors.append(traceback.format_exc()))
    bpy.ops.wm.save_userpref()
    res.update(ok=mod is not None, path=target)
    if mod is None: res['error'] = '\n'.join(errors) or 'Blender did not enable minihou_link'
except Exception as e:
    res['error'] = repr(e)
with open(out_path, 'w', encoding='utf-8') as f: json.dump(res, f)
"#;

struct TempDir(PathBuf);
impl TempDir {
    fn new() -> io::Result<Self> {
        for _ in 0..10 {
            let n = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let path =
                env::temp_dir().join(format!("minihou_install_{}_{}", std::process::id(), n));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not create temporary install directory",
        ))
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn install_args(zip: &Path, result: &Path) -> Vec<OsString> {
    vec![
        "--background".into(),
        "--python-exit-code".into(),
        "1".into(),
        "--python-expr".into(),
        INSTALL_SCRIPT.into(),
        "--".into(),
        zip.into(),
        result.into(),
    ]
}

fn blender_running() -> bool {
    #[cfg(windows)]
    let output = {
        use std::os::windows::process::CommandExt;
        Command::new("tasklist")
            .args(["/FI", "IMAGENAME eq blender.exe", "/FO", "CSV", "/NH"])
            .creation_flags(0x0800_0000)
            .output()
    };
    #[cfg(not(windows))]
    let output = Command::new("pgrep").args(["-x", "blender"]).output();
    output.is_ok_and(|out| {
        out.status.success() && {
            #[cfg(windows)]
            {
                String::from_utf8_lossy(&out.stdout)
                    .to_ascii_lowercase()
                    .contains("\"blender.exe\"")
            }
            #[cfg(not(windows))]
            {
                !out.stdout.is_empty()
            }
        }
    })
}

pub fn install_addon(exe: &Path, zip: &Path) -> io::Result<String> {
    let zip = zip.canonicalize()?;
    let temp = TempDir::new()?;
    let result = temp.0.join("result.json");
    let log = temp.0.join("blender.log");
    let output = fs::File::create(&log)?;
    let mut command = Command::new(exe);
    command
        .args(install_args(&zip, &result))
        .stdin(Stdio::null())
        .stderr(output.try_clone()?)
        .stdout(output);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let mut child = command.spawn()?;
    let deadline = Instant::now() + Duration::from_secs(180);
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Blender did not finish within 180 s",
            ));
        }
        thread::sleep(Duration::from_millis(100));
    };
    let diagnostics = || fs::read_to_string(&log).unwrap_or_default();
    if !status.success() {
        return Err(io::Error::other(format!(
            "Blender exited with {status}: {}",
            diagnostics()
        )));
    }
    let report: serde_json::Value = serde_json::from_slice(&fs::read(&result).map_err(|_| {
        io::Error::other(format!(
            "Blender ran but did not report a result: {}",
            diagnostics()
        ))
    })?)
    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    if report["ok"] == true {
        let running = blender_running();
        Ok(format!(
            "Installed to {} and enabled for Blender {}. {}",
            report["path"].as_str().unwrap_or("?"),
            report["blender"].as_str().unwrap_or(""),
            if running {
                "Blender is already running. Enable MiniHou Link in Preferences (reload/restart Blender for this update), then use Connect in the MiniHou N-panel or leave Auto-connect on."
            } else {
                "Open Blender to connect automatically, or press Connect in its MiniHou N-panel."
            }
        ))
    } else {
        Err(io::Error::other(
            report["error"].as_str().unwrap_or("installation failed"),
        ))
    }
}

// The UI owns documents/history; socket threads only exchange immutable snapshots and requests.
use minihou_app::{
    link_protocol::{self, Arrays, DiffState},
    Document,
};
use minihou_core::Geometry;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    io::Write,
    net::{Shutdown, TcpListener, TcpStream},
    sync::{atomic::AtomicBool, mpsc, Arc, Mutex},
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
    pub fn start(doc: &Document, path: &Path) -> io::Result<Self> {
        Self::bind("127.0.0.1:8765", doc, path)
    }
    pub fn bind(address: &str, doc: &Document, path: &Path) -> io::Result<Self> {
        let listener = TcpListener::bind(address)?;
        listener.set_nonblocking(true)?;
        let address = listener.local_addr()?;
        let shared = Arc::new(LinkShared {
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
                    edits.push(Edit::External {
                        name: format!("object_input:{}", name.as_str().ok_or("bad source name")?),
                        input: ExternalInput::Object {
                            geo: Arc::new(link_protocol::decode_object(&Value::Null, &arrays)?),
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
        "capabilities":["instances_v1","live_modifier_v1","object_inputs_v1","scene_objects_v1","image_exchange_v1",minihou_app::graph_embed::CAPABILITY],"editor_url":null,"editor":"native","graph_path":snapshot.graph_path}),
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
                    let arrays = link_protocol::geometry_arrays(&geo);
                    let (header, changed) =
                        diff.build(&arrays, &geo, snapshot.good_seq, &snapshot.cook);
                    send_frame(&mut socket, &header, &changed)?;
                    let names: Vec<_> = snapshot.extras.keys().cloned().collect();
                    send_frame(
                        &mut socket,
                        &json!({"type":"outputs","names":names}),
                        &Arrays::new(),
                    )?;
                    extra_diffs.retain(|name, _| snapshot.extras.contains_key(name));
                    for (name, g) in &snapshot.extras {
                        let (mut h, a) = extra_diffs.entry(name.clone()).or_default().build(
                            &link_protocol::geometry_arrays(g),
                            g,
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
    state.peers.lock().unwrap().remove(&id);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

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
                "graph_embed_v1"
            ])
        );
        socket
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
    fn scans_install_roots_newest_first_and_deduplicates() {
        let temp = TempDir::new().unwrap();
        for version in ["Blender 4.2", "Blender 5.1"] {
            let dir = temp.0.join("Blender Foundation").join(version);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("blender.exe"), "").unwrap();
        }
        let found = find_installs_in(&[temp.0.clone(), temp.0.clone()]);
        assert_eq!(
            found.iter().map(|x| x.0.as_str()).collect::<Vec<_>>(),
            ["Blender 5.1", "Blender 4.2"]
        );
    }

    #[test]
    fn launch_arguments_contain_graph_and_connection_endpoint() {
        let args = open_args(
            Path::new(r"C:\graphs\my file.json"),
            Some(Path::new("addon's ZIP.zip")),
            Path::new("launch.json"),
        );
        assert_eq!(args[0], "--python-expr");
        let source = args[1].to_string_lossy();
        assert!(source.contains("my file.json"));
        assert!(source.contains("127.0.0.1"));
        assert!(source.contains("8765"));
        assert!(source.contains("addon's ZIP.zip"));
        assert!(source.contains("launch.json"));
        assert!(source.contains("traceback.format_exc()"));
    }

    #[test]
    fn launch_graph_snapshots_untitled_and_dirty_documents_without_saving_editor() {
        let doc =
            Document::parse(r#"{"nodes":{"b":{"type":"box"}},"display":"b"}"#.into()).unwrap();
        let temp = TempDir::new().unwrap();
        let original = temp.0.join("untitled.json");
        let snapshot = prepare_launch_graph(&doc, &original, false).unwrap();
        assert!(!original.exists());
        assert_eq!(fs::read_to_string(&snapshot.path).unwrap(), doc.text());
        let snapshot_path = snapshot.path.clone();
        drop(snapshot);
        assert!(!snapshot_path.exists());

        fs::write(&original, "previous saved document").unwrap();
        let snapshot = prepare_launch_graph(&doc, &original, true).unwrap();
        assert_ne!(snapshot.path, original);
        assert_eq!(fs::read_to_string(&snapshot.path).unwrap(), doc.text());
        assert_eq!(
            fs::read_to_string(&original).unwrap(),
            "previous saved document"
        );
        let saved = prepare_launch_graph(&doc, &original, false).unwrap();
        assert_eq!(saved.path, original.canonicalize().unwrap());
        drop(saved);
        assert!(original.exists());
    }

    #[test]
    fn install_arguments_keep_paths_separate_and_enable_addon() {
        let zip = Path::new(r"C:\a folder\Bob's addon.zip");
        let result = Path::new(r"C:\temp dir\result.json");
        let args = install_args(zip, result);
        assert_eq!(args[0], "--background");
        assert_eq!(args[3], "--python-expr");
        assert_eq!(args[5], "--");
        assert_eq!(args[6], zip.as_os_str());
        assert_eq!(args[7], result.as_os_str());
        let source = args[4].to_string_lossy();
        assert!(source.contains("addon_utils.modules(refresh=True)"));
        assert!(source.contains("default_set=True, persistent=True"));
        assert!(source.contains("save_userpref()"));
        assert!(!source.contains("graph"));
    }

    #[test]
    fn source_discovery_requires_addon_entrypoint() {
        let temp = TempDir::new().unwrap();
        let source = temp.0.join("blender_addon/minihou_link");
        fs::create_dir_all(&source).unwrap();
        assert!(addon_source_in(&[temp.0.clone()]).is_none());
        fs::write(source.join("__init__.py"), "").unwrap();
        assert_eq!(
            addon_source_in(&[temp.0.join("missing"), temp.0.clone()]),
            Some(source)
        );
    }

    #[test]
    fn addon_zip_has_top_level_folder_valid_headers_and_no_python_cache() {
        let temp = TempDir::new().unwrap();
        let source = temp.0.join("source");
        fs::create_dir_all(source.join("nested")).unwrap();
        fs::create_dir_all(source.join("__pycache__")).unwrap();
        fs::write(source.join("__init__.py"), b"123456789").unwrap();
        fs::write(source.join("nested/data.txt"), b"payload").unwrap();
        fs::write(source.join("__pycache__/junk.pyc"), b"ignore").unwrap();
        fs::write(source.join("stale.pyc"), b"ignore").unwrap();
        let zip = temp.0.join("addon.zip");
        addon_archive::build(&source, &zip).unwrap();
        let bytes = fs::read(&zip).unwrap();
        let u16_at = |i| u16::from_le_bytes(bytes[i..i + 2].try_into().unwrap());
        let u32_at = |i| u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap());
        let end = bytes.len() - 22;
        assert_eq!(u32_at(end), 0x06054b50);
        assert_eq!(u16_at(end + 10), 2);
        let mut central = u32_at(end + 16) as usize;
        assert_eq!(central + u32_at(end + 12) as usize, end);
        for (name, payload, crc) in [
            (
                "minihou_link/__init__.py",
                b"123456789".as_slice(),
                0xcbf43926,
            ),
            (
                "minihou_link/nested/data.txt",
                b"payload".as_slice(),
                0x422c6a15,
            ),
        ] {
            assert_eq!(u32_at(central), 0x02014b50);
            let local = u32_at(central + 42) as usize;
            assert_eq!(u32_at(local), 0x04034b50);
            assert_eq!(u16_at(local + 6), 0x800); // UTF-8 names
            assert_eq!(u16_at(local + 8), 0); // stored
            assert_eq!(u32_at(local + 14), crc);
            assert_eq!(u32_at(central + 16), crc);
            assert_eq!(u16_at(local + 26) as usize, name.len());
            assert_eq!(&bytes[local + 30..local + 30 + name.len()], name.as_bytes());
            assert_eq!(
                &bytes[central + 46..central + 46 + name.len()],
                name.as_bytes()
            );
            let data = local + 30 + name.len();
            assert_eq!(u32_at(local + 18) as usize, payload.len());
            assert_eq!(&bytes[data..data + payload.len()], payload);
            central += 46 + name.len();
        }
        assert_eq!(central, end);
        addon_archive::build(&source, &zip).unwrap();
        assert_eq!(fs::read(zip).unwrap(), bytes); // deterministic output
        assert!(addon_archive::build(&temp.0.join("missing"), &temp.0.join("bad.zip")).is_err());
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

    #[test]
    fn zip_lookup_skips_missing_directory_and_accepts_packaged_name() {
        let temp = TempDir::new().unwrap();
        let zip = temp.0.join("MiniHou_Link-0.1.zip");
        fs::write(&zip, b"zip placeholder").unwrap();
        assert_eq!(
            addon_zip_in(&[temp.0.join("absent"), temp.0.clone()]),
            Some(zip)
        );
    }
}

#[cfg(test)]
mod addon_version_tests {
    #[test] fn bundled_addon_version_is_parsed() {
        let v = super::addon_version();
        assert!(v.split('.').count() >= 2 && v.split('.').all(|n| n.parse::<u32>().is_ok()), "got {v}");
    }
}
