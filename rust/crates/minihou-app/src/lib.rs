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
pub struct Document { value: Value, text: String }
impl Document {
    pub fn parse(text: String) -> Result<Self> {
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
        if self.value["nodes"].get(node).is_none() { return Err(format!("no such node '{node}'").into()); }
        self.set(&["display"], json!(node)); Ok(())
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
fn perspective(fovy: f32, aspect: f32, near: f32, far: f32) -> M4 {
    let t = 1.0 / (fovy / 2.0).tan();
    [
        [t / aspect, 0.0, 0.0, 0.0],
        [0.0, t, 0.0, 0.0],
        [0.0, 0.0, far / (near - far), -1.0],
        [0.0, 0.0, near * far / (near - far), 0.0],
    ]
}

pub struct Camera {
    target: [f32; 3],
    dist: f32,
    yaw: f32,
    pitch: f32,
    radius: f32,
}

impl Camera {
    /// Distance from the eye to the orbit target (world units); sizes the viewport's edge fade.
    pub fn distance(&self) -> f32 {
        self.dist
    }
    pub fn frame(center: [f32; 3], radius: f32) -> Self {
        let radius = radius.max(1e-3); Camera { target: center, dist: radius * 2.6, yaw: 0.6, pitch: 0.5, radius }
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
        (mul(&perspective(0.8, aspect, near, far), &view), view)
    }
}


impl Camera {
    pub fn orbit(&mut self, dx: f32, dy: f32) { self.yaw -= dx * 0.008; self.pitch = (self.pitch + dy * 0.008).clamp(-1.55,1.55); }
    pub fn pan(&mut self, dx: f32, dy: f32) {
        let f = norm(sub(self.target,self.eye())); let s = norm(cross(f,[0.,1.,0.])); let u = cross(s,f);
        for i in 0..3 { self.target[i] += (-s[i]*dx+u[i]*dy)*self.dist*0.0011; }
    }
    pub fn zoom(&mut self, delta: f32) { self.dist = (self.dist * (-delta * 0.0025).exp()).clamp(self.radius*0.002,self.radius*10000.); }
}

#[cfg(test)]
mod tests;
