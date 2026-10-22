//! Opt-in subprocess bridge for the `python` node (docs/RUST_PYTHON_NODE.md).
//!
//! This is NOT a sandbox: an allowed snippet runs as the user, with the user's privileges. The engine never runs one unless
//! the cook was given a [`PythonGate`] that is switched on *and* the exact snippet was confirmed for this session. A
//! [`crate::session::Session`] has no gate by default, so headless, MCP-only and Blender-link cooks are denied.
//!
//! Wire format: the live link's MHL1 framing (JSON header + 8-byte aligned little-endian arrays) in two files, `job.mhl`
//! and `out.mhl`, versioned by `header.version` ([`WIRE_VERSION`]). Every point / vertex / primitive attribute (float32 or
//! int32, any width, NaN preserved), the full topology (open curves included) and the whole detail map (materials,
//! material_bindings, anything JSON) round-trip byte for byte.
use std::cell::RefCell;
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};

use crate::error::{Error, Result};
use crate::geometry::{Attr, AttrData, Geometry, Topology};
use crate::params::Params;

pub const WIRE_VERSION: u64 = 1;
pub const RUNNER_SOURCE: &str = include_str!("../../../../tools/python_node_runner.py");
pub const DISABLED_MESSAGE: &str = "Python execution is disabled; enable it in Preferences > Security";
pub const CONFIRM_MESSAGE: &str = "Python snippet waiting for confirmation: choose Run or Skip in the banner";
pub const SKIPPED_MESSAGE: &str = "Python snippet skipped for this session (edit the code or restart to be asked again)";
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const MESSAGE_CAP: usize = 4000;

/// What a gate says about one snippet. Pure: see [`decide`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision { Disabled, NeedsConfirmation, Skipped, Run }

/// The permission rule: off means off; on means each distinct snippet is confirmed (or skipped) once per session.
pub fn decide(allowed: bool, approved: bool, skipped: bool) -> Decision {
    match (allowed, approved, skipped) {
        (false, _, _) => Decision::Disabled,
        (true, true, _) => Decision::Run,
        (true, false, true) => Decision::Skipped,
        (true, false, false) => Decision::NeedsConfirmation,
    }
}

/// Stable short id of a snippet (FNV-1a 64), for display and cache keys. Approval compares the full text, not this.
pub fn code_hash(code: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in code.as_bytes() { h = (h ^ *b as u64).wrapping_mul(0x100000001b3); }
    format!("{h:016x}")
}

/// How to start Python: `program args... -B runner.py job_dir`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Runtime {
    pub program: PathBuf,
    pub args: Vec<String>,
    /// Prepended to PYTHONPATH (the repo root, so `import minihou` works).
    pub python_path: Option<PathBuf>,
    /// Runner source override (tests use tiny fake runners).
    pub runner: Option<String>,
}
impl Runtime {
    pub fn id(&self) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.hash(&mut h);
        RUNNER_SOURCE.hash(&mut h);
        if let Ok(m) = std::fs::metadata(&self.program) { m.len().hash(&mut h); m.modified().ok().hash(&mut h); }
        h.finish()
    }
}

/// The app-owned permission for one editor session. Remote surfaces (HTTP/MCP, Blender link) never receive it.
pub struct PythonGate {
    allowed: AtomicBool,
    timeout_ms: AtomicU64,
    max_output: AtomicU64,
    approved: Mutex<HashSet<String>>,
    skipped: Mutex<HashSet<String>>,
    pending: Mutex<Vec<String>>,
    runtime: Mutex<Option<std::result::Result<Runtime, String>>>,
}
impl Default for PythonGate { fn default() -> Self { Self::denied() } }
impl PythonGate {
    pub fn denied() -> Self {
        Self { allowed: AtomicBool::new(false), timeout_ms: AtomicU64::new(DEFAULT_TIMEOUT.as_millis() as u64),
            max_output: AtomicU64::new(crate::limits::Limits::default().python_output_bytes), approved: Default::default(), skipped: Default::default(),
            pending: Default::default(), runtime: Mutex::new(None) }
    }
    pub fn set_allowed(&self, on: bool) { self.allowed.store(on, Ordering::SeqCst); }
    pub fn allowed(&self) -> bool { self.allowed.load(Ordering::SeqCst) }
    pub fn set_timeout(&self, t: Duration) { self.timeout_ms.store((t.as_millis() as u64).max(1), Ordering::SeqCst); }
    pub fn timeout(&self) -> Duration { Duration::from_millis(self.timeout_ms.load(Ordering::SeqCst)) }
    pub fn set_max_output_bytes(&self, n: u64) { self.max_output.store(n.max(16), Ordering::SeqCst); }
    pub fn max_output_bytes(&self) -> u64 { self.max_output.load(Ordering::SeqCst) }
    pub fn set_runtime(&self, r: Runtime) { *self.runtime.lock().unwrap() = Some(Ok(r)); }
    /// Confirm this exact snippet for the rest of the session.
    pub fn approve(&self, code: &str) {
        self.skipped.lock().unwrap().remove(code);
        self.approved.lock().unwrap().insert(code.to_owned());
        self.pending.lock().unwrap().retain(|c| c != code);
    }
    pub fn skip(&self, code: &str) {
        self.skipped.lock().unwrap().insert(code.to_owned());
        self.pending.lock().unwrap().retain(|c| c != code);
    }
    /// Snippets that asked to run and wait for Run / Skip, oldest first.
    pub fn pending(&self) -> Vec<String> { self.pending.lock().unwrap().clone() }
    pub fn decision(&self, code: &str) -> Decision {
        decide(self.allowed(), self.approved.lock().unwrap().contains(code), self.skipped.lock().unwrap().contains(code))
    }
    /// [`Self::decision`], remembering a snippet that needs confirmation so the UI can ask.
    pub fn check(&self, code: &str) -> Decision {
        let d = self.decision(code);
        if d == Decision::NeedsConfirmation {
            let mut p = self.pending.lock().unwrap();
            if !p.iter().any(|c| c == code) { p.push(code.to_owned()); }
        }
        d
    }
    pub fn runtime(&self) -> std::result::Result<Runtime, String> {
        let mut r = self.runtime.lock().unwrap();
        r.get_or_insert_with(discover).clone()
    }
    /// Cache-key material: changes when the permission, confirmation, runtime or limits change.
    pub fn key(&self, code: &str) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (self.decision(code) as u8).hash(&mut h);
        self.timeout_ms.load(Ordering::SeqCst).hash(&mut h);
        self.max_output_bytes().hash(&mut h);
        if let Some(Ok(r)) = self.runtime.lock().unwrap().as_ref() { r.id().hash(&mut h); }
        h.finish()
    }
}

thread_local! { static GATE: RefCell<Option<Arc<PythonGate>>> = const { RefCell::new(None) }; }

/// Run `f` with `gate` as the cook's Python permission (None: denied).
pub fn with_gate<R>(gate: Option<Arc<PythonGate>>, f: impl FnOnce() -> R) -> R {
    let old = GATE.with(|g| std::mem::replace(&mut *g.borrow_mut(), gate));
    struct Restore(Option<Option<Arc<PythonGate>>>);
    impl Drop for Restore { fn drop(&mut self) { let v = self.0.take().unwrap(); GATE.with(|g| *g.borrow_mut() = v); } }
    let _r = Restore(Some(old));
    f()
}
pub fn current_gate() -> Option<Arc<PythonGate>> { GATE.with(|g| g.borrow().clone()) }
/// Cache-key material for a python node in the current cook.
pub fn key_material(code: &str) -> u64 { current_gate().map_or(0, |g| g.key(code) | 1) }

// ---------------------------------------------------------------- runtime discovery

fn repo_root() -> Option<PathBuf> {
    let ok = |p: &Path| p.join("minihou").join("__init__.py").is_file();
    if let Some(p) = std::env::var_os("MINIHOU_REPO").map(PathBuf::from).filter(|p| ok(p)) { return Some(p); }
    let mut starts = vec![];
    if let Ok(e) = std::env::current_exe() { starts.push(e); }
    if let Ok(d) = std::env::current_dir() { starts.push(d); }
    starts.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")));
    starts.iter().flat_map(|s| s.ancestors()).find(|p| ok(p)).map(Path::to_path_buf)
}

/// MINIHOU_PYTHON, then the repo venv, then `py -3` (Windows), `python3`, `python`; the first that imports numpy wins.
pub fn discover() -> std::result::Result<Runtime, String> {
    let root = repo_root();
    let mut candidates: Vec<(PathBuf, Vec<String>)> = vec![];
    if let Some(p) = std::env::var_os("MINIHOU_PYTHON").filter(|p| !p.is_empty()) { candidates.push((p.into(), vec![])); }
    if let Some(r) = &root {
        for v in [".venv", "venv"] {
            for exe in ["Scripts/python.exe", "bin/python3", "bin/python"] {
                let p = r.join(v).join(exe);
                if p.is_file() { candidates.push((p, vec![])); }
            }
        }
    }
    if cfg!(windows) { candidates.push(("py".into(), vec!["-3".into()])); }
    candidates.push(("python3".into(), vec![]));
    candidates.push(("python".into(), vec![]));
    let mut tried = vec![];
    for (program, args) in candidates {
        let rt = Runtime { program: program.clone(), args, python_path: root.clone(), runner: None };
        let mut cmd = base_command(&rt, Path::new("."));
        cmd.args(["-c", "import numpy"]);
        match spawn_wait(cmd, Duration::from_secs(15)) {
            Ok(o) if o.status_ok => return Ok(rt),
            _ => tried.push(program.display().to_string()),
        }
    }
    Err(format!("no Python 3 with numpy found (tried {}); set MINIHOU_PYTHON to a python executable", tried.join(", ")))
}

// ---------------------------------------------------------------- process control

const ENV_ALLOW: &[&str] = &["PATH", "PATHEXT", "SYSTEMROOT", "SystemRoot", "WINDIR", "TEMP", "TMP", "TMPDIR", "HOME",
    "USERPROFILE", "APPDATA", "LOCALAPPDATA", "LANG", "LC_ALL", "LC_CTYPE", "VIRTUAL_ENV", "CONDA_PREFIX", "PYTHONHOME"];

fn base_command(rt: &Runtime, cwd: &Path) -> Command {
    let mut cmd = Command::new(&rt.program);
    cmd.args(&rt.args).current_dir(cwd).env_clear().stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    for k in ENV_ALLOW { if let Some(v) = std::env::var_os(k) { cmd.env(k, v); } }
    let mut paths: Vec<PathBuf> = rt.python_path.iter().cloned().collect();
    if let Some(p) = std::env::var_os("PYTHONPATH") { paths.extend(std::env::split_paths(&p)); }
    if let Ok(joined) = std::env::join_paths(paths) { if !joined.is_empty() { cmd.env("PYTHONPATH", joined); } }
    cmd.env("PYTHONIOENCODING", "utf-8").env("PYTHONDONTWRITEBYTECODE", "1");
    #[cfg(windows)] { use std::os::windows::process::CommandExt; cmd.creation_flags(0x0800_0000 | 0x0000_0200); }
    #[cfg(unix)] { use std::os::unix::process::CommandExt; cmd.process_group(0); }
    cmd
}

fn kill_tree(pid: u32) {
    #[cfg(windows)] {
        let exe = std::env::var_os("SystemRoot").map(|r| PathBuf::from(r).join("System32").join("taskkill.exe")).unwrap_or_else(|| "taskkill".into());
        let _ = Command::new(exe).args(["/T", "/F", "/PID", &pid.to_string()]).stdout(Stdio::null()).stderr(Stdio::null()).status();
    }
    #[cfg(unix)] { let _ = Command::new("kill").args(["-KILL", "--", &format!("-{pid}")]).stdout(Stdio::null()).stderr(Stdio::null()).status(); }
}

struct Outcome { status_ok: bool, code: Option<i32>, stderr: String, timed_out: bool }

fn capped_reader(mut r: impl Read + Send + 'static) -> mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    let stderr_cap = crate::limits::Limits::default().python_stderr_bytes;
    std::thread::spawn(move || {
        let (mut keep, mut buf) = (Vec::new(), [0u8; 8192]);
        while let Ok(n) = r.read(&mut buf) {
            if n == 0 { break; }
            let room = stderr_cap.saturating_sub(keep.len());
            keep.extend_from_slice(&buf[..n.min(room)]);
        }
        let _ = tx.send(keep);
    });
    rx
}

fn spawn_wait(mut cmd: Command, timeout: Duration) -> std::io::Result<Outcome> {
    let mut child = cmd.spawn()?;
    let out = capped_reader(child.stdout.take().unwrap());
    let err = capped_reader(child.stderr.take().unwrap());
    let start = Instant::now();
    let (status, timed_out) = loop {
        if let Some(s) = child.try_wait()? { break (Some(s), false); }
        if start.elapsed() >= timeout {
            kill_tree(child.id());
            let _ = child.kill();
            let _ = child.wait();
            break (None, true);
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    // A grandchild may still hold the pipes open; never block on it.
    let _ = out.recv_timeout(Duration::from_secs(2));
    let stderr = err.recv_timeout(Duration::from_secs(2)).unwrap_or_default();
    Ok(Outcome { status_ok: status.is_some_and(|s| s.success()), code: status.and_then(|s| s.code()),
        stderr: String::from_utf8_lossy(&stderr).into_owned(), timed_out })
}

struct TempDir(PathBuf);
impl Drop for TempDir { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }
fn temp_dir() -> Result<TempDir> {
    static N: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let dir = std::env::temp_dir().join(format!("minihou-py-{}-{}-{nanos}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    std::fs::create_dir(&dir).map_err(|e| Error::new(format!("python node: cannot create a temp dir: {e}")))?;
    Ok(TempDir(dir))
}

fn tail(s: &str) -> String {
    let s = s.trim();
    if s.len() <= MESSAGE_CAP { return s.to_owned(); }
    let mut i = s.len() - MESSAGE_CAP;
    while !s.is_char_boundary(i) { i += 1; }
    format!("...{}", &s[i..])
}

/// Run one snippet through the runner. The caller has already checked the gate.
pub fn run(gate: &PythonGate, inputs: [Option<&Geometry>; 4], params: &Params) -> Result<Geometry> {
    let rt = gate.runtime().map_err(|e| Error::new(format!("python node: {e}")))?;
    let dir = temp_dir()?;
    let runner = dir.0.join("python_node_runner.py");
    std::fs::write(&runner, rt.runner.as_deref().unwrap_or(RUNNER_SOURCE)).map_err(|e| Error::new(format!("python node: {e}")))?;
    std::fs::write(dir.0.join("job.mhl"), encode_job(inputs, params)?).map_err(|e| Error::new(format!("python node: {e}")))?;
    let mut cmd = base_command(&rt, &dir.0);
    cmd.arg("-B").arg(&runner).arg(&dir.0);
    let o = spawn_wait(cmd, gate.timeout()).map_err(|e| Error::new(format!("python node: cannot start {}: {e}", rt.program.display())))?;
    if o.timed_out {
        return Err(Error::new(format!("python node timed out after {:.1} s; the Python process tree was killed", gate.timeout().as_secs_f64())));
    }
    if !o.status_ok {
        let code = o.code.map_or("killed".to_owned(), |c| c.to_string());
        return Err(Error::new(format!("python node failed (exit {code}):\n{}", tail(&o.stderr))));
    }
    let out = dir.0.join("out.mhl");
    let len = std::fs::metadata(&out).map_err(|_| Error::new("python node: the runner exited without writing a result"))?.len();
    if len > gate.max_output_bytes() {
        return Err(Error::new(format!("python node: the result is {len} bytes, over the {} byte limit", gate.max_output_bytes())));
    }
    let bytes = std::fs::read(&out).map_err(|e| Error::new(format!("python node: {e}")))?;
    decode_result(&bytes).map_err(|e| Error::new(format!("python node: malformed result: {e}")))
}

// ---------------------------------------------------------------- wire format

struct Arr { key: String, dtype: &'static str, shape: Vec<usize>, bytes: Vec<u8> }

pub fn encode_frame(header: &Value, arrays: &[(String, &'static str, Vec<usize>, Vec<u8>)]) -> Vec<u8> {
    let mut blob = Vec::new();
    let mut descs = Vec::new();
    for (key, dtype, shape, bytes) in arrays {
        descs.push(json!({"key":key,"dtype":dtype,"shape":shape,"offset":blob.len(),"nbytes":bytes.len()}));
        blob.extend_from_slice(bytes);
        blob.resize(blob.len().div_ceil(8) * 8, 0);
    }
    let mut header = header.clone();
    if !descs.is_empty() { header["arrays"] = Value::Array(descs); }
    let mut hb = serde_json::to_vec(&header).expect("json");
    hb.resize(hb.len().div_ceil(8) * 8, b' ');
    let mut f = Vec::with_capacity(16 + hb.len() + blob.len());
    f.extend_from_slice(b"MHL1");
    f.extend_from_slice(&(hb.len() as u32).to_le_bytes());
    f.extend_from_slice(&(blob.len() as u64).to_le_bytes());
    f.extend(hb);
    f.extend(blob);
    f
}

fn decode_frame(data: &[u8]) -> std::result::Result<(Value, Vec<Arr>), String> {
    if data.len() < 16 || &data[..4] != b"MHL1" { return Err("bad magic".into()); }
    let hlen = u32::from_le_bytes(data[4..8].try_into().unwrap()) as usize;
    let blen = u64::from_le_bytes(data[8..16].try_into().unwrap());
    if 16u64.checked_add(hlen as u64).and_then(|n| n.checked_add(blen)) != Some(data.len() as u64) { return Err("frame size mismatch".into()); }
    let header: Value = serde_json::from_slice(&data[16..16 + hlen]).map_err(|e| format!("header: {e}"))?;
    let blob = &data[16 + hlen..];
    let mut out = Vec::new();
    for d in header.get("arrays").and_then(Value::as_array).into_iter().flatten() {
        let key = d["key"].as_str().ok_or("array without key")?.to_owned();
        let dtype = match d["dtype"].as_str() { Some("<f4") => "<f4", Some("<i4") => "<i4", Some("|b1") => "|b1", Some("|u1") => "|u1", other => return Err(format!("{key}: unsupported dtype {other:?}")) };
        let num = |v: &Value| v.as_u64().and_then(|n| usize::try_from(n).ok()).ok_or_else(|| format!("{key}: bad number"));
        let shape: Vec<usize> = d["shape"].as_array().ok_or("missing shape")?.iter().map(num).collect::<std::result::Result<_, _>>()?;
        let (off, n) = (num(&d["offset"])?, num(&d["nbytes"])?);
        let item = if dtype.starts_with('|') { 1 } else { 4 };
        let want = shape.iter().try_fold(item, |a: usize, s| a.checked_mul(*s)).ok_or("shape overflow")?;
        let end = off.checked_add(n).ok_or("offset overflow")?;
        if want != n || end > blob.len() || shape.is_empty() || shape.len() > 2 { return Err(format!("{key}: shape/size mismatch")); }
        out.push(Arr { key, dtype, shape, bytes: blob[off..end].to_vec() });
    }
    Ok((header, out))
}

fn le<T: Copy>(v: &[T], f: impl Fn(T) -> [u8; 4]) -> Vec<u8> { v.iter().flat_map(|x| f(*x)).collect() }

fn geometry_arrays(prefix: &str, g: &Geometry) -> Vec<(String, &'static str, Vec<usize>, Vec<u8>)> {
    let mut a = vec![];
    for (dom, table) in [("point", &g.point), ("vertex", &g.vertex), ("prim", &g.prim)] {
        for (name, attr) in table {
            let shape = if attr.width == 1 { vec![attr.rows()] } else { vec![attr.rows(), attr.width] };
            let (dt, bytes) = match &attr.data { AttrData::F32(v) => ("<f4", le(v, f32::to_le_bytes)), AttrData::I32(v) => ("<i4", le(v, i32::to_le_bytes)) };
            a.push((format!("{prefix}{dom}/{name}"), dt, shape, bytes));
        }
    }
    let t = &g.topology;
    a.push((format!("{prefix}topo/vertex_point"), "<i4", vec![t.vertex_point.len()], le(&t.vertex_point, i32::to_le_bytes)));
    a.push((format!("{prefix}topo/prim_offsets"), "<i4", vec![t.prim_offsets.len()], le(&t.prim_offsets, i32::to_le_bytes)));
    if t.has_open() {
        a.push((format!("{prefix}topo/prim_closed"), "|u1", vec![g.nprims()], t.closed_mask().iter().map(|b| u8::from(*b)).collect()));
    }
    a
}

/// The job file: code, parameters, time and up to four input geometries.
pub fn encode_job(inputs: [Option<&Geometry>; 4], p: &Params) -> Result<Vec<u8>> {
    let mut arrays = vec![];
    let mut descs = vec![];
    for (k, g) in inputs.iter().enumerate() {
        match g {
            None => descs.push(Value::Null),
            Some(g) => {
                arrays.extend(geometry_arrays(&format!("in{k}/"), g));
                descs.push(json!({"npoints": g.npoints, "detail": Value::Object(g.detail.clone())}));
            }
        }
    }
    let header = json!({"type":"python_job","version":WIRE_VERSION,
        "params":{"code":p.s("code")?,"a":p.f("a")?,"b":p.f("b")?,"c":p.f("c")?,"d":p.f("d")?,"i":p.i("i")?,"j":p.i("j")?,
            "v":p.v3("v")?,"s":p.s("s")?,"flag":p.b("flag")?,"variables":p.s("variables")?,"animated":p.b("animated")?},
        "time":{"frame":crate::timectx::frame(),"fps":crate::timectx::fps(),"time_inc":crate::timectx::time_inc()},
        "inputs":descs});
    Ok(encode_frame(&header, &arrays))
}

fn attr_of(a: &Arr) -> Attr {
    let w = a.shape.get(1).copied().unwrap_or(1).max(1);
    match a.dtype {
        "<f4" => Attr::f32(w, a.bytes.chunks_exact(4).map(|b| f32::from_le_bytes(b.try_into().unwrap())).collect()),
        "<i4" => Attr::i32(w, a.bytes.chunks_exact(4).map(|b| i32::from_le_bytes(b.try_into().unwrap())).collect()),
        _ => Attr::i32(w, a.bytes.iter().map(|b| i32::from(*b != 0)).collect()),
    }
}

fn geometry_from(arrays: &[Arr], prefix: &str, npoints: usize, detail: Map<String, Value>) -> std::result::Result<Geometry, String> {
    let find = |k: &str| arrays.iter().find(|a| a.key == format!("{prefix}{k}"));
    let ints = |k: &str, d: Vec<i32>| -> std::result::Result<Vec<i32>, String> {
        match find(k) {
            None => Ok(d),
            Some(a) if a.dtype == "<i4" && a.shape.len() == 1 => Ok(a.bytes.chunks_exact(4).map(|b| i32::from_le_bytes(b.try_into().unwrap())).collect()),
            _ => Err(format!("{k} must be a 1-D int32 array")),
        }
    };
    let closed = find("topo/prim_closed").map(|a| a.bytes.iter().map(|b| *b != 0).collect());
    let topology = Topology { vertex_point: ints("topo/vertex_point", vec![])?, prim_offsets: ints("topo/prim_offsets", vec![0])?, closed , ..Default::default()};
    topology.validate(npoints).map_err(|e| e.to_string())?;
    let mut g = Geometry { npoints, topology: Arc::new(topology), detail, ..Default::default() };
    for a in arrays {
        let Some(rest) = a.key.strip_prefix(prefix) else { continue };
        let (dom, name) = rest.split_once('/').ok_or_else(|| format!("bad key {}", a.key))?;
        if name.is_empty() { return Err(format!("empty attribute name in {}", a.key)); }
        let table = match dom { "point" => &mut g.point, "vertex" => &mut g.vertex, "prim" => &mut g.prim, "topo" => continue, _ => return Err(format!("bad domain in {}", a.key)) };
        table.insert(name.to_owned(), Arc::new(attr_of(a)));
    }
    if g.point.get("P").is_none_or(|p| p.width != 3 || p.as_f32().is_none()) { return Err("point/P must be float32 (n, 3)".into()); }
    g.validate().map_err(|e| e.to_string())?;
    Ok(g)
}

pub fn decode_result(data: &[u8]) -> std::result::Result<Geometry, String> {
    let (h, arrays) = decode_frame(data)?;
    if h["type"] != "python_result" { return Err(format!("unexpected message type {}", h["type"])); }
    if h["version"].as_u64() != Some(WIRE_VERSION) { return Err(format!("wire version {} is not {WIRE_VERSION}", h["version"])); }
    let n = h["npoints"].as_u64().and_then(|n| usize::try_from(n).ok()).ok_or("missing npoints")?;
    let detail = match &h["detail"] { Value::Object(m) => m.clone(), Value::Null => Map::new(), _ => return Err("detail must be an object".into()) };
    geometry_from(&arrays, "", n, detail)
}

/// Decode one input of a job file (tests and tools).
pub fn decode_job_input(data: &[u8], slot: usize) -> std::result::Result<Option<Geometry>, String> {
    let (h, arrays) = decode_frame(data)?;
    let d = &h["inputs"][slot];
    if d.is_null() { return Ok(None); }
    let n = d["npoints"].as_u64().ok_or("missing npoints")? as usize;
    geometry_from(&arrays, &format!("in{slot}/"), n, d["detail"].as_object().cloned().unwrap_or_default()).map(Some)
}

#[cfg(test)]
#[path = "python_bridge_tests.rs"]
mod tests;
