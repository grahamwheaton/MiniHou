//! Window-independent editor state and lossless JSON edits.
pub mod cluster_lod;
pub mod link_protocol;
pub mod http_api;
pub mod http_ops;
pub mod http_geometry;
#[cfg(test)] mod http_tests;
pub mod transform_math;
pub mod noodle_hit;
use minihou_core::{session::Session, lod::ClusterLod, Graph};
use serde_json::{json, Value};
use std::{collections::HashMap, ops::Range, path::Path, time::Instant};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub mod units;
pub use units::{LengthUnit, UnitSettings};

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
    clip_start: f32,
    clip_end: f32,
}

/// Keep persisted and interactive clipping values finite, positive and ordered.
pub fn sanitize_clip_planes(near: f32, far: f32) -> (f32, f32) {
    let near = if near.is_finite() { near.clamp(1e-6, 1e8) } else { 0.01 };
    let far = if far.is_finite() { far } else { 1000.0 };
    (near, far.clamp(near * 1.001, 1e9))
}

#[cfg(test)] mod fly_tests {
    use super::*;
    fn close(a: [f32; 3], b: [f32; 3]) { for i in 0..3 { assert!((a[i]-b[i]).abs()<1e-4, "{a:?} != {b:?}"); } }
    #[test] fn look_keeps_eye_and_orbit_distance() {
        let mut c = Camera::frame([1., 2., 3.], 2.);
        let eye = c.eye(); let distance = c.distance();
        let right = [c.yaw.cos(), c.yaw.sin(), 0.];
        c.fly([100., -80.], [0.; 3], 0.);
        assert!(dot(c.forward(), right) > 0., "moving the mouse right must look right");
        close(c.eye(), eye); assert_eq!(c.distance(), distance);
        close(sub(c.target, eye), c.forward().map(|x| x * distance));
    }
    #[test] fn fly_uses_view_forward_world_z_and_normalized_diagonals() {
        let mut c = Camera::frame([0.; 3], 2.);
        let eye = c.eye(); let forward = c.forward();
        c.fly([0.; 2], [1., 0., 0.], 2.);
        close(c.eye(), [0,1,2].map(|i| eye[i]+forward[i]*2.));
        let eye = c.eye(); c.fly([0.;2], [0.,0.,1.], 3.);
        close(c.eye(), [eye[0],eye[1],eye[2]+3.]);
        let eye = c.eye(); c.fly([0.;2], [1.,1.,1.], 4.);
        let delta = sub(c.eye(), eye); assert!((dot(delta,delta).sqrt()-4.).abs()<1e-4);
        c.fly([0., 1e6], [0.;3], 0.);
        assert!(c.pitch < std::f32::consts::FRAC_PI_2);
        assert!(c.forward().iter().all(|v| v.is_finite()));
    }
}

impl Camera {
    /// Distance from the eye to the orbit target (world units); sizes the viewport's edge fade.
    pub fn distance(&self) -> f32 {
        self.dist
    }
    pub fn clip_planes(&self) -> (f32, f32) {
        (self.clip_start, self.clip_end)
    }
    pub fn lod_camera(&self, size: [u32; 2]) -> minihou_core::lod::Camera {
        let (view_projection, _) = self.matrices(size[0].max(1) as f32 / size[1].max(1) as f32);
        let (near, _) = self.clip_planes();
        minihou_core::lod::Camera { view_projection, eye: self.eye(), viewport_height_px: size[1] as f32,
            near, orthographic: self.ortho }
    }
    pub fn frame(center: [f32; 3], radius: f32) -> Self {
        let radius = radius.max(1e-3); Camera { target: center, dist: radius * 2.6, yaw: 0.6, pitch: 0.5, radius, ortho: false, clip_start: 0.01, clip_end: 1000.0 }
    }
    /// Explicit eye-space clipping distances in world units, independent of framing and zoom.
    pub fn set_clip_planes(&mut self, near: f32, far: f32) {
        (self.clip_start, self.clip_end) = sanitize_clip_planes(near, far);
    }
    pub fn set_dist(&mut self, dist: f32) {
        self.dist = dist;
    }

    pub fn eye(&self) -> [f32; 3] {
        // +Z up (Blender); yaw 0 looks along +Y from -Y (Front view)
        let (cp, sp) = (self.pitch.cos(), self.pitch.sin());
        [
            self.target[0] + self.dist * cp * self.yaw.sin(),
            self.target[1] - self.dist * cp * self.yaw.cos(),
            self.target[2] + self.dist * sp,
        ]
    }

    pub fn forward(&self) -> [f32; 3] {
        norm(sub(self.target, self.eye()))
    }

    /// First-person look and motion, keeping the orbit distance and target attached to the eye.
    /// Motion is [forward, right, world Z], normalized to avoid faster diagonal flight.
    pub fn fly(&mut self, look: [f32; 2], motion: [f32; 3], distance: f32) {
        let eye = self.eye();
        self.ortho = false;
        self.yaw -= look[0] * 0.003;
        self.pitch = (self.pitch + look[1] * 0.003).clamp(-1.553343, 1.553343);
        let forward = [-self.pitch.cos() * self.yaw.sin(), self.pitch.cos() * self.yaw.cos(), -self.pitch.sin()];
        let right = [self.yaw.cos(), self.yaw.sin(), 0.];
        let delta = [0, 1, 2].map(|i| forward[i] * motion[0] + right[i] * motion[1] + if i == 2 { motion[2] } else { 0. });
        let length = dot(delta, delta).sqrt();
        self.target = [0, 1, 2].map(|i| eye[i] + if length > 0. { delta[i] / length * distance } else { 0. } + forward[i] * self.dist);
    }

    /// World up for the view: flips once the orbit has gone over the pole, so the camera rolls on through the zenith (turntable, as Blender).
    fn up(&self) -> [f32; 3] { [0.0, 0.0, if self.pitch.cos() < 0.0 { -1.0 } else { 1.0 }] }
    fn add_orbit(&mut self, dyaw: f32, dpitch: f32) {
        // upside down, horizontal drags turn the other way so the model still follows the mouse
        let sign = if self.pitch.cos() < 0.0 { -1.0 } else { 1.0 };
        self.yaw += sign * dyaw;
        let tau = std::f32::consts::TAU;
        self.pitch = (self.pitch + dpitch + std::f32::consts::PI).rem_euclid(tau) - std::f32::consts::PI;
    }
    pub fn matrices(&self, aspect: f32) -> (M4, M4) {
        let view = look_at(self.eye(), self.target, self.up());
        let (near, far) = self.clip_planes();
        let proj = if self.ortho { orthographic(0.8, self.dist, aspect, near, far) } else { perspective(0.8, aspect, near, far) };
        (mul(&proj, &view), view)
    }
}


impl Camera {
    /// Snap to one of Blender's standard views (+Z is up; Front looks along +Y). Top and bottom stop just short of the pole.
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
    /// Blender's numpad 5: switch projection, keeping the current angles.
    pub fn toggle_ortho(&mut self) { self.ortho = !self.ortho; }
    /// The world-space ray through a point of the viewport: `nx`, `ny` in -1..1 (right, up), `aspect` = width / height.
    pub fn ray(&self, nx: f32, ny: f32, aspect: f32) -> ([f32; 3], [f32; 3]) {
        let eye = self.eye();
        let f = norm(sub(self.target, eye));
        let right = norm(cross(f, self.up()));
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
    /// Orbit angles in radians (yaw about +Z, pitch up) for the view gizmo.
    pub fn angles(&self) -> (f32, f32) { (self.yaw, self.pitch) }
    pub fn orbit_degrees(&mut self, yaw: f32, pitch: f32) {
        self.ortho = false;
        self.add_orbit(-yaw.to_radians(), pitch.to_radians());
    }
    pub fn zoom_factor(&mut self, factor: f32) { if factor.is_finite() && factor > 0.0 { self.dist = (self.dist / factor).clamp(self.radius * 0.05, self.radius * 60.0); } }
    pub fn orbit(&mut self, dx: f32, dy: f32) { self.ortho = false; self.add_orbit(-dx * 0.008, dy * 0.008); }
    pub fn pan(&mut self, dx: f32, dy: f32) {
        let f = norm(sub(self.target,self.eye())); let s = norm(cross(f,self.up())); let u = cross(s,f);
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
    Exposed(Vec<Value>),
    External { name: String, input: minihou_core::nodes::blender_input::ExternalInput },
    /// Timeline: the frame and fps cooks run at (`@Frame`/`@Time`, solvers). Not a document edit.
    Time { frame: f64, fps: f64 },
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
            Edit::Exposed(entries) => self.set(&["exposed"], json!(entries)),
            Edit::External { .. } => return Err("live inputs are not document edits".into()),
            Edit::Time { .. } => return Err("the timeline is not a document edit".into()),
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
    /// Record a completed drag whose document was already updated in place.
    pub fn record(&mut self, before: Document, after: &Document) {
        self.finish(after);
        if before.text != after.text {
            self.undo.push((before, after.clone()));
            self.redo.clear();
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
    pub extras: std::collections::BTreeMap<String,Arc<Geometry>>,
    pub scene: Vec<(Value,link_protocol::Arrays)>,
    pub geometry: Option<Arc<Geometry>>,
    pub lod: Option<Arc<ClusterLod>>,
    pub prepared: Option<T>,
    /// Display output ownership in the merged geometry (point indices).
    pub display_ranges: Vec<(String, std::ops::Range<u32>)>,
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
                Edit::External { name, input } => session.set_external(name,input),
                Edit::Time { frame, fps } => session.set_time(frame, fps)?,
                Edit::Exposed(_) => {}, // Interface metadata does not change the core graph.
                Edit::Replace(value) => {
                    let mut graph = Graph::from_value(&value)?;
                    let displayed = display_nodes(&value);
                    *target = if displayed.len() > 1 {
                        let id = (0..).map(|i| format!("__display_merge_{i}")).find(|id| !graph.nodes.iter().any(|(name, _)| name == id)).unwrap();
                        graph.nodes.push((id.clone(), minihou_core::graph::NodeDef { node_type: "merge".into(), inputs: vec![("geos".into(), displayed)], params: None, bypass: false }));
                        id
                    } else { graph.main_output()? };
                    session.replace_graph(graph);
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
    let lod = if geometry.is_some() {
        // Breadth first finds the closest bake to the displayed node, including itself.
        let mut queue = std::collections::VecDeque::from([target.clone()]);
        let mut seen = std::collections::HashSet::new();
        let mut found = None;
        while let Some(id) = queue.pop_front() {
            if !seen.insert(id.clone()) { continue; }
            if let Some(def) = session.graph().nodes.iter().find(|(name, _)| name == &id).map(|(_, def)| def) {
                if def.node_type == "lod_bake" {
                    if !def.bypass {
                        if let (Some(source), Some(displayed)) = (session.cached_geometry(&id), geometry.as_ref()) {
                            let same_points = source.point.get("P").zip(displayed.point.get("P"))
                                .is_some_and(|(a,b)| Arc::ptr_eq(a,b));
                            if same_points && Arc::ptr_eq(&source.topology,&displayed.topology) {
                                found = session.side_payload::<ClusterLod>(&id);
                            }
                        }
                    }
                    break;
                }
                for (_, refs) in &def.inputs { queue.extend(refs.iter().cloned()); }
            }
        }
        found
    } else { None };
    let mut stats=session.last_stats().clone();
    let mut extras=std::collections::BTreeMap::new();
    let mut scene=Vec::new();
    let extra_targets=session.graph().extra_outputs();
    let sinks:Vec<_>=session.graph().nodes.iter().filter(|(_,d)|["scene_objects","image_out"].contains(&d.node_type.as_str()))
        .map(|(id,d)|(id.clone(),d.node_type.clone(),d.params.clone(),d.bypass)).collect();
    for (name,id) in extra_targets {
        match session.cook(Some(&id)) { Ok(g)=>{extras.insert(name,g);}, Err(e)=>error=Some(e.to_string()) }
        append_cook_stats(&mut stats,session.last_stats());
    }
    for (id,typ,params,bypass) in sinks {
        // A bypassed sink still passes geometry through but has no scene side effect.
        if bypass {continue;}
        let result:Result<(Value,link_protocol::Arrays)>=(|| {
            let g=session.cook(Some(&id))?;
            let p=minihou_core::params::Params::resolve(minihou_core::nodes::get(&typ).unwrap().params,params.as_ref())?;
            link_protocol::sink_manifest(&typ,&id,&p,&g,&revision.to_string())
        })();
        append_cook_stats(&mut stats,session.last_stats());
        match result {Ok(manifest)=>scene.push(manifest),Err(e)=>error=Some(e.to_string())}
    }
    let milliseconds = start.elapsed().as_secs_f64() * 1000.;
    // The synthetic display merge concatenates points in input order. Keep this
    // ownership alongside the cooked revision for viewport picking and outlines.
    let outputs = session.graph().nodes.iter().find(|(id, _)| id.as_str() == target.as_str())
        .filter(|(id, _)| id.starts_with("__display_merge_"))
        .map(|(_, def)| def.inputs.iter().flat_map(|(_, ids)| ids.iter().cloned()).collect::<Vec<_>>())
        .unwrap_or_else(|| vec![target.clone()]);
    let mut offset = 0;
    let display_ranges = outputs.into_iter().filter_map(|id| {
        let count = session.cached_geometry(&id)?.npoints as u32;
        let range = offset..offset + count;
        offset += count;
        Some((id, range))
    }).collect();
    let prepared = geometry.as_ref().map(|g| prepare(g));
    CookResponse { revision, geometry, lod, prepared, display_ranges, extras, scene, stats, error, milliseconds }
}
#[cfg(test)]
mod display_selection_tests {
    use super::*;

    #[test]
    fn cooked_display_ownership_survives_parameter_edits_and_display_changes() {
        let value = json!({"nodes": {"a":{"type":"box"}, "b":{"type":"box"}}, "display":["b","a"]});
        let doc = Document::parse(value.to_string()).unwrap();
        let mut session = doc.session().unwrap();
        let mut target = session.graph().main_output().unwrap();
        let result = cook_batch(&mut session, &mut target,
            vec![CookRequest { revision: 1, edit: Edit::Replace(value) }], |_| ());
        assert!(result.error.is_none(), "{:?}", result.error);
        let count = session.cached_geometry("b").unwrap().npoints as u32;
        assert!(count > 0);
        assert_eq!(result.display_ranges, vec![("b".into(), 0..count), ("a".into(), count..2*count)]);
        let result = cook_batch(&mut session, &mut target,
            vec![CookRequest { revision: 2, edit: Edit::Param { node: "a".into(), name: "size".into(), value: json!([2.,3.,4.]) } }], |_| ());
        assert!(result.error.is_none(), "{:?}", result.error);
        assert_eq!(result.display_ranges[1], ("a".into(), count..2*count));
        let result = cook_batch(&mut session, &mut target,
            vec![CookRequest { revision: 3, edit: Edit::Display("a".into()) }], |_| ());
        assert!(result.error.is_none(), "{:?}", result.error);
        assert_eq!(result.display_ranges, vec![("a".into(), 0..count)]);
    }
}

fn append_cook_stats(out:&mut CookStats, more:&CookStats) {
    for id in &more.cooked {if !out.cooked.contains(id){out.cooked.push(id.clone());}}
    for id in &more.cached {if !out.cached.contains(id)&&!out.cooked.contains(id){out.cached.push(id.clone());}}
    out.node_ms.extend(more.node_ms.iter().cloned());
}
#[derive(Default)]
pub struct CookState {
    pub requested: u64,
    pub completed: u64,
    pub geometry: Option<Arc<Geometry>>,
    pub lod: Option<Arc<ClusterLod>>,
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
        self.lod = response.lod;
        if let Some(g) = response.geometry { self.geometry = Some(g); }
        response.prepared
    }
}
/// One result slot bounds memory even if the UI is occupied by a native dialog.
/// Dropping the sender shuts the worker down after its current cook; never joins on the UI.
pub struct CookWorker<T> {
    sender: mpsc::Sender<CookRequest>,
    queries: mpsc::Sender<http_geometry::Query>,
    latest: Arc<Mutex<Option<CookResponse<T>>>>,
}
impl<T: Send + 'static> CookWorker<T> {
    pub fn start(doc: &Document, prepare: impl Fn(&Geometry) -> T + Send + 'static) -> Result<Self> {
        let mut session = doc.session()?;
        let mut target = session.graph().main_output()?;
        let (sender, receiver) = mpsc::channel();
        let (queries, query_rx) = mpsc::channel();
        let latest = Arc::new(Mutex::new(None));
        let output = latest.clone();
        std::thread::Builder::new().name("minihou-cook".into()).spawn(move || {
            loop {
                match receiver.recv_timeout(std::time::Duration::from_millis(10)) {
                    Ok(first) => {
                        let mut batch = vec![first];
                        batch.extend(receiver.try_iter());
                        let response = cook_batch(&mut session, &mut target, batch, &prepare);
                        *output.lock().unwrap() = Some(response);
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(mpsc::RecvTimeoutError::Timeout) => {},
                }
                for query in query_rx.try_iter() { http_geometry::run(&mut session, &target, query); }
            }
        })?;
        Ok(Self { sender, queries, latest })
    }
    pub fn send(&self, request: CookRequest) -> Result<()> { self.sender.send(request).map_err(|_| "cook worker stopped".into()) }
    pub fn poll(&self) -> Option<CookResponse<T>> { self.latest.lock().unwrap().take() }
    pub fn query(&self, query: http_geometry::Query) -> Result<()> { self.queries.send(query).map_err(|_| "cook worker stopped".into()) }
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

/// Nodes with saved exposed parameters; usable by the network UI without egui state.
pub fn exposed_nodes(doc: &Value) -> std::collections::HashSet<String> {
    doc["exposed"].as_array().into_iter().flatten()
        .filter_map(|e| e["node"].as_str().map(str::to_owned)).collect()
}

pub fn is_exposed(doc: &Value, node: &str, param: &str) -> bool {
    doc["exposed"].as_array().into_iter().flatten()
        .any(|e| e["node"] == node && e["param"] == param)
}

pub fn toggle_exposed(doc: &Value, node: &str, param: &str) -> Edit {
    let mut entries = doc["exposed"].as_array().cloned().unwrap_or_default();
    if is_exposed(doc, node, param) {
        entries.retain(|e| !(e["node"] == node && e["param"] == param));
    } else {
        let base = format!("{node}_{param}");
        let mut name = base.clone();
        let mut suffix = 2;
        while entries.iter().any(|e| e["name"] == name) {
            name = format!("{base}_{suffix}"); suffix += 1;
        }
        entries.push(json!({"name":name,"node":node,"param":param,"label":name}));
    }
    Edit::Exposed(entries)
}

/// Python-compatible Blender socket descriptions. Skip stale and non-socket parameters.
pub fn link_interface(doc: &Value) -> Value {
    let mut params = Vec::new();
    let mut names = std::collections::HashSet::new();
    for entry in doc["exposed"].as_array().into_iter().flatten() {
        let Some(node) = entry["node"].as_str() else { continue };
        let Some(param) = entry["param"].as_str() else { continue };
        let Some(name) = entry["name"].as_str() else { continue };
        let Some(typ) = doc["nodes"][node]["type"].as_str() else { continue };
        let Some(spec) = minihou_core::nodes::get(typ).and_then(|s| s.params.iter().find(|p| p.name == param)) else { continue };
        if !spec.supported { continue; }
        let default = (spec.default)();
        let meta = param_meta(typ, param);
        let kind = meta.map_or_else(|| param_kind(&default), |m| metadata_kind(m, &default));
        let kind = match kind { ParamKind::Bool => "bool", ParamKind::Integer => "int", ParamKind::Float => "float", ParamKind::Vec3 => "vec3", _ => continue };
        if !names.insert(name.to_owned()) { continue; }
        let mut p = meta.cloned().unwrap_or_else(|| json!({}));
        p["kind"] = json!(kind); p["default"] = default.clone();
        p["name"] = json!(name); p["node"] = json!(node); p["param"] = json!(param);
        p["label"] = json!(entry["label"].as_str().filter(|s| !s.is_empty()).unwrap_or(name));
        p["value"] = doc["nodes"][node]["params"].get(param).unwrap_or(&default).clone();
        params.push(p);
    }
    // This is an opaque change token in the addon, not a mandated hashing algorithm.
    let structure: Vec<_> = params.iter().cloned().map(|mut p| { p.as_object_mut().unwrap().remove("value"); p }).collect();
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    serde_json::to_string(&structure).unwrap().hash(&mut hash);
    json!({"type":"interface","structure":format!("{:016x}",hash.finish()),"params":params})
}

/// Match Python snap_float32: preserve arbitrary doubles, shorten exact float32 inputs.
pub fn snap_float32(value: &Value) -> Value {
    match value {
        Value::Number(n) if n.is_f64() => {
            let x = n.as_f64().unwrap(); let f = x as f32;
            if f.is_finite() && f as f64 == x {
                f.to_string().parse::<f64>().map_or_else(|_| value.clone(), |v| json!(v))
            } else { value.clone() }
        }
        Value::Array(xs) => Value::Array(xs.iter().map(snap_float32).collect()),
        _ => value.clone(),
    }
}

pub fn link_param_edit(doc: &Value, message: &Value) -> Result<Edit> {
    let (node, param) = if let Some(name) = message.get("name") {
        let e = doc["exposed"].as_array().into_iter().flatten()
            .find(|e| &e["name"] == name).ok_or("unknown exposed parameter")?;
        (e["node"].as_str().ok_or("invalid exposed node")?, e["param"].as_str().ok_or("invalid exposed parameter")?)
    } else {
        (message["node"].as_str().ok_or("missing node")?, message["param"].as_str().ok_or("missing param")?)
    };
    let value = snap_float32(message.get("value").ok_or("missing value")?);
    let typ = doc["nodes"][node]["type"].as_str().ok_or("no such node")?;
    let spec = minihou_core::nodes::get(typ).ok_or("unsupported node")?
        .params.iter().find(|p| p.name == param).ok_or("unknown parameter")?;
    if !spec.supported { return Err("parameter is not supported by the Rust backend".into()); }
    let default = (spec.default)();
    let meta = param_meta(typ, param);
    let kind = meta.map_or_else(|| param_kind(&default), |m| metadata_kind(m, &default));
    let valid = match kind {
        ParamKind::Float => value.is_number(),
        ParamKind::Integer => value.as_f64().is_some_and(|x| x.fract() == 0.0),
        ParamKind::Bool => value.is_boolean(),
        ParamKind::String => value.is_string(),
        ParamKind::Vec3 => value.as_array().is_some_and(|a| a.len() == 3 && a.iter().all(Value::is_number)),
        ParamKind::Unsupported => false,
    };
    if !valid { return Err(format!("invalid value for {node}.{param}").into()); }
    if let Some(meta) = meta {
        for v in value.as_array().map_or_else(|| vec![&value], |a| a.iter().collect()) {
            if let Some(x) = v.as_f64() {
                if x < meta["min"].as_f64().unwrap_or(f64::NEG_INFINITY) || x > meta["max"].as_f64().unwrap_or(f64::INFINITY) {
                    return Err(format!("value outside limits for {node}.{param}").into());
                }
            }
        }
        if meta["kind"] == "enum" && !meta["options"].as_array().is_some_and(|a| a.contains(&value)) {
            return Err("value is not an available enum option".into());
        }
    }
    Ok(Edit::Param { node:node.into(), name:param.into(), value })
}

#[cfg(test)]
mod link_tests {
    use super::*;
    #[test]
    fn exposure_is_saved_undoable_and_has_stable_structure_on_value_changes() {
        let mut doc = Document::parse(r#"{"nodes":{"b":{"type":"box"}},"display":"b","unknown":42}"#.into()).unwrap();
        let mut history = History::default();
        let edit = toggle_exposed(doc.value(), "b", "size");
        history.edit(&mut doc, &edit).unwrap();
        history.finish(&doc);
        assert!(exposed_nodes(doc.value()).contains("b"));
        let iface = link_interface(doc.value());
        assert_eq!(iface["params"][0]["kind"], "vec3");
        let saved = Document::parse(doc.text().into()).unwrap();
        assert!(is_exposed(saved.value(), "b", "size"));
        let edit = link_param_edit(doc.value(), &json!({"name":"b_size","value":[2,3,4]})).unwrap();
        history.edit(&mut doc, &edit).unwrap(); history.finish(&doc);
        assert_eq!(iface["structure"], link_interface(doc.value())["structure"]);
        assert_eq!(doc.value()["unknown"], 42);
        history.undo(&mut doc); history.undo(&mut doc);
        assert!(!is_exposed(doc.value(), "b", "size"));
        history.redo(&mut doc); assert!(is_exposed(doc.value(), "b", "size"));
    }
    #[test]
    fn rejects_invalid_remote_parameters_and_preserves_direct_addressing() {
        let doc = Document::parse(r#"{"nodes":{"b":{"type":"box"}},"display":"b"}"#.into()).unwrap();
        assert!(link_param_edit(doc.value(), &json!({"node":"b","param":"size","value":[1,2,3]})).is_ok());
        for message in [json!({"name":"missing","value":1}), json!({"node":"gone","param":"size","value":1}),
            json!({"node":"b","param":"missing","value":1}), json!({"node":"b","param":"size","value":"bad"}),
            json!({"node":"b","param":"size"})] {
            assert!(link_param_edit(doc.value(), &message).is_err());
        }
    }
    #[test]
    fn snaps_only_exact_float32_values_recursively() {
        assert_eq!(snap_float32(&json!([0.22_f32 as f64, 0.123456789, 3, true])), json!([0.22,0.123456789,3,true]));
    }
}
