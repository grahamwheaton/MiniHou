//! Window-independent editor state and lossless JSON edits.
use minihou_core::{session::Session, Graph};
use serde_json::{json, Value};
use std::{collections::HashMap, ops::Range, path::Path, time::Instant};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Debug, PartialEq, Eq)]
pub enum ParamKind { Bool, Integer, Float, String, Vec3, Unsupported }
pub fn param_kind(default: &Value) -> ParamKind {
    match default {
        Value::Bool(_) => ParamKind::Bool,
        Value::Number(n) if n.is_i64() || n.is_u64() => ParamKind::Integer,
        Value::Number(_) => ParamKind::Float,
        Value::String(_) => ParamKind::String,
        Value::Array(a) if a.len() == 3 && a.iter().all(Value::is_number) => ParamKind::Vec3,
        _ => ParamKind::Unsupported,
    }
}

/// Keeps original spelling, whitespace and unknown fields. Edits replace only
/// the corresponding JSON token (or insert an omitted member).
#[derive(Clone)]
pub struct Document { value: Value, text: String }
impl Document {
    pub fn parse(text: String) -> Result<Self> {
        let text = text.strip_prefix('\u{feff}').unwrap_or(&text).to_owned();
        let value = serde_json::from_str(&text)?;
        Graph::from_value(&value)?;
        Ok(Self { value, text })
    }
    pub fn open(path: &Path) -> Result<Self> { Self::parse(std::fs::read_to_string(path)?) }
    pub fn value(&self) -> &Value { &self.value }
    pub fn text(&self) -> &str { &self.text }
    pub fn save(&self, path: &Path) -> Result<()> { Ok(std::fs::write(path, &self.text)?) }
    pub fn session(&self) -> Result<Session> { Ok(Session::new(Graph::from_value(&self.value)?)) }
    pub fn set_param(&mut self, session: &mut Session, node: &str, name: &str, value: Value) -> Result<()> {
        session.set_param(node, name, value.clone())?;
        self.set(&["nodes", node, "params", name], value);
        Ok(())
    }
    pub fn set_bypass(&mut self, session: &mut Session, node: &str, flag: bool) -> Result<()> {
        session.set_bypass(node, flag)?;
        self.set(&["nodes", node, "flags", "bypass"], json!(flag));
        Ok(())
    }
    pub fn set_display(&mut self, node: &str) -> Result<()> {
        self.set_display_nodes(&[node.to_owned()])
    }
    pub fn display_nodes(&self) -> Vec<String> { display_nodes(&self.value) }
    pub fn set_display_nodes(&mut self, nodes: &[String]) -> Result<()> {
        if nodes.is_empty() { return Err("display set cannot be empty".into()); }
        for node in nodes {
            if self.value["nodes"].get(node).is_none() { return Err(format!("no such node '{node}'").into()); }
        }
        self.set(&["display"], if nodes.len() == 1 { json!(nodes[0]) } else { json!(nodes) }); Ok(())
    }
    fn set(&mut self, path: &[&str], value: Value) {
        let mut spans = HashMap::new();
        scan(self.text.as_bytes(), &mut 0, &mut Vec::new(), &mut spans);
        let mut keys = Vec::new();
        for (i, key) in path.iter().enumerate() {
            let parent = keys.clone();
            keys.push((*key).to_owned());
            if let Some(span) = spans.get(&keys) {
                if i + 1 == path.len() {
                    self.text.replace_range(span.clone(), &value.to_string()); break;
                }
                if !self.value_at(&keys).is_object() {
                    let nested = nest(&path[i + 1..], value);
                    self.text.replace_range(span.clone(), &nested.to_string()); break;
                }
            } else {
                let span = &spans[&parent];
                let comma = if self.value_at(&parent).as_object().is_some_and(|m| !m.is_empty()) { "," } else { "" };
                let nested = nest(&path[i + 1..], value);
                self.text.insert_str(span.end - 1, &format!("{comma}{}:{nested}", json!(key)));
                break;
            }
        }
        self.value = serde_json::from_str(&self.text).expect("validated JSON token replacement");
    }
    fn value_at(&self, keys: &[String]) -> &Value {
        keys.iter().fold(&self.value, |v,k| &v[k])
    }
}
pub fn display_nodes(value: &Value) -> Vec<String> {
    match &value["display"] {
        Value::String(s) => vec![s.clone()],
        Value::Array(xs) => xs.iter().filter_map(Value::as_str).map(str::to_owned).collect(),
        _ => Vec::new(),
    }
}
fn nest(keys: &[&str], mut value: Value) -> Value {
    for key in keys.iter().rev() { let mut map = serde_json::Map::new(); map.insert((*key).into(), value); value = Value::Object(map); }
    value
}
// Input has already passed serde_json validation.
fn scan(b: &[u8], i: &mut usize, path: &mut Vec<String>, spans: &mut HashMap<Vec<String>, Range<usize>>) {
    fn ws(b: &[u8], i: &mut usize) { while *i < b.len() && b[*i].is_ascii_whitespace() { *i += 1; } }
    fn string(b: &[u8], i: &mut usize) { *i += 1; while b[*i] != b'"' { if b[*i] == b'\\' { *i += 1; } *i += 1; } *i += 1; }
    ws(b,i); let start = *i;
    match b[*i] {
        b'{' => {
            *i += 1; ws(b,i);
            while b[*i] != b'}' {
                let k = *i; string(b,i);
                let key: String = serde_json::from_slice(&b[k..*i]).unwrap();
                ws(b,i); *i += 1; path.push(key); scan(b,i,path,spans); path.pop(); ws(b,i);
                if b[*i] == b',' { *i += 1; ws(b,i); }
            }
            *i += 1;
        }
        b'[' => {
            *i += 1; ws(b,i); let mut index = 0;
            while b[*i] != b']' {
                path.push(index.to_string()); scan(b,i,path,spans); path.pop(); index += 1; ws(b,i);
                if b[*i] == b',' { *i += 1; ws(b,i); }
            }
            *i += 1;
        }
        b'"' => string(b,i),
        _ => { while *i < b.len() && !b",]} \t\r\n".contains(&b[*i]) { *i += 1; } }
    }
    spans.insert(path.clone(), start..*i);
}
pub fn selftest(doc: &Document) -> Result<Value> {
    let mut session = doc.session()?;
    let target = session.graph().main_output()?;
    let t = Instant::now(); session.cook(Some(&target))?;
    let first = t.elapsed().as_secs_f64() * 1000.0;
    let nodes = session.graph().nodes.len();
    let id = session.graph().nodes.get(1).or_else(|| session.graph().nodes.first()).ok_or("graph has no nodes")?.0.clone();
    session.set_bypass(&id, true)?;
    let t = Instant::now(); let geo = session.cook(Some(&target))?;
    Ok(json!({"nodes":nodes,"points":geo.npoints,"prims":geo.topology.nprims(),"first_cook_ms":first,"second_cook_ms":t.elapsed().as_secs_f64()*1000.0,"cooked_second":session.last_stats().cooked}))
}
type M4 = [[f32; 4]; 4];

fn mul(a: &M4, b: &M4) -> M4 {
    let mut r = [[0.0; 4]; 4];
    for c in 0..4 {
        for row in 0..4 {
            r[c][row] = (0..4).map(|k| a[k][row] * b[c][k]).sum();
        }
    }
    r
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn norm(a: [f32; 3]) -> [f32; 3] {
    let l = dot(a, a).sqrt().max(1e-20);
    [a[0] / l, a[1] / l, a[2] / l]
}

fn look_at(eye: [f32; 3], at: [f32; 3], up: [f32; 3]) -> M4 {
    let f = norm(sub(at, eye));
    let s = norm(cross(f, up));
    let u = cross(s, f);
    [
        [s[0], u[0], -f[0], 0.0],
        [s[1], u[1], -f[1], 0.0],
        [s[2], u[2], -f[2], 0.0],
        [-dot(s, eye), -dot(u, eye), dot(f, eye), 1.0],
    ]
}

/// Right-handed, depth 0..1 (wgpu).
/// Orthographic projection that shows the same height as `perspective(fovy)` does at distance `dist`.
fn orthographic(fovy: f32, dist: f32, aspect: f32, near: f32, far: f32) -> M4 {
    let h = dist * (fovy / 2.0).tan();
    let w = h * aspect;
    [
        [1.0 / w, 0.0, 0.0, 0.0],
        [0.0, 1.0 / h, 0.0, 0.0],
        [0.0, 0.0, 1.0 / (near - far), 0.0],
        [0.0, 0.0, near / (near - far), 1.0],
    ]
}

fn perspective(fovy: f32, aspect: f32, near: f32, far: f32) -> M4 {
    let t = 1.0 / (fovy / 2.0).tan();
    [
        [t / aspect, 0.0, 0.0, 0.0],
        [0.0, t, 0.0, 0.0],
        [0.0, 0.0, far / (near - far), -1.0],
        [0.0, 0.0, near * far / (near - far), 0.0],
    ]
}

/// The views on the numpad (perspective keeps the current angles).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StandardView { Perspective, Front, Back, Right, Left, Top, Bottom }

pub struct Camera {
    target: [f32; 3],
    dist: f32,
    yaw: f32,
    pitch: f32,
    radius: f32,
    ortho: bool,
}

impl Camera {
    /// Distance from the eye to the orbit target (world units); sizes the viewport's edge fade.
    pub fn distance(&self) -> f32 {
        self.dist
    }
    pub fn frame(center: [f32; 3], radius: f32) -> Self {
        let radius = radius.max(1e-3); Camera { target: center, dist: radius * 2.6, yaw: 0.6, pitch: 0.5, radius, ortho: false }
    }

    fn eye(&self) -> [f32; 3] {
        // +Y up
        let (cp, sp) = (self.pitch.cos(), self.pitch.sin());
        [
            self.target[0] + self.dist * cp * self.yaw.sin(),
            self.target[1] + self.dist * sp,
            self.target[2] + self.dist * cp * self.yaw.cos(),
        ]
    }

    pub fn matrices(&self, aspect: f32) -> (M4, M4) {
        let view = look_at(self.eye(), self.target, [0.0, 1.0, 0.0]);
        let near = (self.dist - self.radius * 1.5).max(self.radius * 0.001).max(1e-4);
        let far = self.dist + self.radius * 3.0;
        let proj = if self.ortho { orthographic(0.8, self.dist, aspect, 1e-3_f32.min(near), far) } else { perspective(0.8, aspect, near, far) };
        (mul(&proj, &view), view)
    }
}


impl Camera {
    /// Snap to one of Blender's standard views (+Y is up; Front looks along -Z). Top and bottom stop just short of the pole.
    pub fn set_view(&mut self, view: crate::StandardView) {
        use crate::StandardView as V;
        const POLE: f32 = 1.5698;
        let (yaw, pitch, ortho) = match view {
            V::Perspective => (self.yaw, self.pitch, false),
            V::Front => (0., 0., true),
            V::Back => (std::f32::consts::PI, 0., true),
            V::Right => (std::f32::consts::FRAC_PI_2, 0., true),
            V::Left => (-std::f32::consts::FRAC_PI_2, 0., true),
            V::Top => (0., POLE, true),
            V::Bottom => (0., -POLE, true),
        };
        self.yaw = yaw; self.pitch = pitch; self.ortho = ortho;
    }
    pub fn is_ortho(&self) -> bool { self.ortho }
    /// The world-space ray through a point of the viewport: `nx`, `ny` in -1..1 (right, up), `aspect` = width / height.
    pub fn ray(&self, nx: f32, ny: f32, aspect: f32) -> ([f32; 3], [f32; 3]) {
        let eye = self.eye();
        let f = norm(sub(self.target, eye));
        let right = norm(cross(f, [0., 1., 0.]));
        let up = cross(right, f);
        let tan_half = (0.8_f32 / 2.0).tan();
        if self.ortho {
            let h = self.dist * tan_half;
            let o = [0, 1, 2].map(|k| eye[k] + right[k] * nx * aspect * h + up[k] * ny * h);
            (o, f)
        } else {
            let d = [0, 1, 2].map(|k| f[k] + right[k] * nx * aspect * tan_half + up[k] * ny * tan_half);
            (eye, norm(d))
        }
    }
    /// Orbit angles in radians (yaw about +Y, pitch up) for the view gizmo.
    pub fn angles(&self) -> (f32, f32) { (self.yaw, self.pitch) }
    pub fn orbit_degrees(&mut self, yaw: f32, pitch: f32) {
        self.ortho = false;
        self.yaw -= yaw.to_radians(); self.pitch = (self.pitch + pitch.to_radians()).clamp(-1.55, 1.55);
    }
    pub fn zoom_factor(&mut self, factor: f32) { if factor.is_finite() && factor > 0.0 { self.dist = (self.dist / factor).clamp(self.radius * 0.05, self.radius * 60.0); } }
    pub fn orbit(&mut self, dx: f32, dy: f32) { self.ortho = false; self.yaw -= dx * 0.008; self.pitch = (self.pitch + dy * 0.008).clamp(-1.55,1.55); }
    pub fn pan(&mut self, dx: f32, dy: f32) {
        let f = norm(sub(self.target,self.eye())); let s = norm(cross(f,[0.,1.,0.])); let u = cross(s,f);
        for i in 0..3 { self.target[i] += (-s[i]*dx+u[i]*dy)*self.dist*0.0011; }
    }
    pub fn zoom(&mut self, delta: f32) { self.dist = (self.dist * (-delta * 0.0025).exp()).clamp(self.radius*0.002,self.radius*10000.); }
}

#[cfg(test)]
mod tests;

/// Generated registry metadata; unknown parameters retain the generic widget.
pub fn param_meta(node_type: &str, name: &str) -> Option<&'static Value> {
    static META: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    META.get_or_init(|| serde_json::from_str(include_str!("param_meta.json")).expect("generated metadata"))
        .get(node_type)?.get(name)
}
pub fn metadata_kind(meta: &Value, default: &Value) -> ParamKind {
    match meta["kind"].as_str() {
        Some("int") => ParamKind::Integer,
        Some("float") => ParamKind::Float,
        Some("bool") => ParamKind::Bool,
        Some("str" | "enum") => ParamKind::String,
        Some("vec3") => ParamKind::Vec3,
        _ => param_kind(default),
    }
}
pub fn slider_range(meta: &Value) -> Option<std::ops::RangeInclusive<f64>> {
    Some(meta["min"].as_f64()?..=meta["max"].as_f64().or_else(|| meta["soft_max"].as_f64())?)
}
/// soft_max affects presentation only, never the accepted value.
pub fn clamp_parameter(meta: &Value, value: f64) -> f64 {
    value.max(meta["min"].as_f64().unwrap_or(f64::NEG_INFINITY))
        .min(meta["max"].as_f64().unwrap_or(f64::INFINITY))
}

#[derive(Clone, Debug)]
pub enum Edit {
    Param { node: String, name: String, value: Value },
    Bypass { node: String, flag: bool },
    Display(String),
    Replace(Value),
}
impl Document {
    /// Validate edits without creating or cooking a Session on the UI thread.
    pub fn edit(&mut self, edit: &Edit) -> Result<()> {
        match edit {
            Edit::Param { node, name, value } => {
                let typ = self.value["nodes"][node]["type"].as_str().ok_or("no such node")?;
                let spec = minihou_core::nodes::get(typ).ok_or("unsupported node")?;
                if !spec.params.iter().any(|p| p.name == name) { return Err("unknown parameter".into()); }
                self.set(&["nodes", node, "params", name], value.clone());
            }
            Edit::Bypass { node, flag } => {
                if self.value["nodes"].get(node).is_none() { return Err("no such node".into()); }
                self.set(&["nodes", node, "flags", "bypass"], json!(flag));
            }
            Edit::Display(node) => self.set_display(node)?,
            Edit::Replace(_) => return Err("replace uses document snapshots".into()),
        }
        Ok(())
    }
}

#[derive(Default)]
pub struct History {
    undo: Vec<(Document, Document)>,
    redo: Vec<(Document, Document)>,
    pending: Option<Document>,
}
impl History {
    pub fn edit(&mut self, doc: &mut Document, edit: &Edit) -> Result<()> {
        let before = doc.clone();
        doc.edit(edit)?;
        if before.text != doc.text {
            self.pending.get_or_insert(before);
            self.redo.clear();
        }
        Ok(())
    }
    /// A structural change (adding a node): `doc` becomes `new` as one undo step.
    pub fn replace(&mut self, doc: &mut Document, new: Document) {
        self.finish(doc);
        if doc.text != new.text {
            self.undo.push((doc.clone(), new.clone()));
            self.redo.clear();
            *doc = new;
        }
    }
    /// Called at the end of a drag, or immediately for discrete edits.
    pub fn finish(&mut self, doc: &Document) {
        if let Some(before) = self.pending.take() {
            if before.text != doc.text { self.undo.push((before, doc.clone())); }
        }
    }
    pub fn can_undo(&self) -> bool { !self.undo.is_empty() || self.pending.is_some() }
    pub fn can_redo(&self) -> bool { !self.redo.is_empty() }
    pub fn undo(&mut self, doc: &mut Document) -> Option<Edit> {
        self.finish(doc);
        let pair = self.undo.pop()?;
        *doc = pair.0.clone();
        self.redo.push(pair);
        Some(Edit::Replace(doc.value.clone()))
    }
    pub fn redo(&mut self, doc: &mut Document) -> Option<Edit> {
        self.finish(doc);
        let pair = self.redo.pop()?;
        *doc = pair.1.clone();
        self.undo.push(pair);
        Some(Edit::Replace(doc.value.clone()))
    }
}

use minihou_core::{Geometry, session::CookStats};
use std::sync::{Arc, Mutex, mpsc};
pub struct CookRequest { pub revision: u64, pub edit: Edit }
pub struct CookResponse<T> {
    pub revision: u64,
    pub geometry: Option<Arc<Geometry>>,
    pub prepared: Option<T>,
    pub stats: CookStats,
    pub error: Option<String>,
    pub milliseconds: f64,
}
/// Apply every queued edit in order, then cook just the final state.
/// This preserves independent edits while the last edit of any parameter wins.
pub fn cook_batch<T>(session: &mut Session, target: &mut String, requests: Vec<CookRequest>, prepare: impl FnOnce(&Geometry) -> T) -> CookResponse<T> {
    let revision = requests.last().map_or(0, |r| r.revision);
    let start = Instant::now();
    let mut error = None;
    for request in requests {
        let result: Result<()> = (|| {
            match request.edit {
                Edit::Param { node, name, value } => session.set_param(&node, &name, value)?,
                Edit::Bypass { node, flag } => session.set_bypass(&node, flag)?,
                Edit::Display(node) => *target = node,
                Edit::Replace(value) => {
                    let mut graph = Graph::from_value(&value)?;
                    let displayed = display_nodes(&value);
                    *target = if displayed.len() > 1 {
                        let id = (0..).map(|i| format!("__display_merge_{i}")).find(|id| !graph.nodes.iter().any(|(name, _)| name == id)).unwrap();
                        graph.nodes.push((id.clone(), minihou_core::graph::NodeDef { node_type: "merge".into(), inputs: vec![("geos".into(), displayed)], params: None, bypass: false }));
                        id
                    } else { graph.main_output()? };
                    *session = Session::new(graph);
                }
            }
            Ok(())
        })();
        if let Err(e) = result { error = Some(e.to_string()); }
    }
    let geometry = match session.cook(Some(target)) {
        Ok(g) => Some(g),
        Err(e) => { error = Some(e.to_string()); None }
    };
    let milliseconds = start.elapsed().as_secs_f64() * 1000.;
    let prepared = geometry.as_ref().map(|g| prepare(g));
    CookResponse { revision, geometry, prepared, stats: session.last_stats().clone(), error, milliseconds }
}
#[derive(Default)]
pub struct CookState {
    pub requested: u64,
    pub completed: u64,
    pub geometry: Option<Arc<Geometry>>,
    pub stats: CookStats,
    pub error: Option<String>,
    pub milliseconds: f64,
}
impl CookState {
    pub fn cooking(&self) -> bool { self.completed < self.requested }
    pub fn accept<T>(&mut self, response: CookResponse<T>) -> Option<T> {
        if response.revision != self.requested { return None; }
        self.completed = response.revision;
        self.error = response.error;
        self.stats = response.stats;
        self.milliseconds = response.milliseconds;
        if let Some(g) = response.geometry { self.geometry = Some(g); }
        response.prepared
    }
}
/// One result slot bounds memory even if the UI is occupied by a native dialog.
/// Dropping the sender shuts the worker down after its current cook; never joins on the UI.
pub struct CookWorker<T> {
    sender: mpsc::Sender<CookRequest>,
    latest: Arc<Mutex<Option<CookResponse<T>>>>,
}
impl<T: Send + 'static> CookWorker<T> {
    pub fn start(doc: &Document, prepare: impl Fn(&Geometry) -> T + Send + 'static) -> Result<Self> {
        let mut session = doc.session()?;
        let mut target = session.graph().main_output()?;
        let (sender, receiver) = mpsc::channel();
        let latest = Arc::new(Mutex::new(None));
        let output = latest.clone();
        std::thread::Builder::new().name("minihou-cook".into()).spawn(move || {
            while let Ok(first) = receiver.recv() {
                let mut batch = vec![first];
                batch.extend(receiver.try_iter());
                let response = cook_batch(&mut session, &mut target, batch, &prepare);
                *output.lock().unwrap() = Some(response);
            }
        })?;
        Ok(Self { sender, latest })
    }
    pub fn send(&self, request: CookRequest) -> Result<()> { self.sender.send(request).map_err(|_| "cook worker stopped".into()) }
    pub fn poll(&self) -> Option<CookResponse<T>> { self.latest.lock().unwrap().take() }
}

/// File identity, save point and history are replaced together on a successful Open.
pub struct EditorDocument {
    pub doc: Document,
    pub history: History,
    pub path: std::path::PathBuf,
    saved: String,
}
impl EditorDocument {
    pub fn new(doc: Document, path: std::path::PathBuf) -> Self {
        Self { saved:doc.text().into(), doc, path, history:History::default() }
    }
    pub fn dirty(&self) -> bool { self.doc.text() != self.saved }
    pub fn title(&self) -> String {
        format!("MiniHou - {}{}", self.path.file_name().unwrap_or_default().to_string_lossy(), if self.dirty() { " *" } else { "" })
    }
    pub fn save_to(&mut self, path: std::path::PathBuf) -> Result<()> {
        self.history.finish(&self.doc);
        self.doc.save(&path)?;
        self.path = path;
        self.saved = self.doc.text().into();
        Ok(())
    }
}
