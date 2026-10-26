use crate::{graph_embed, modifier_inputs, History, Result};
use minihou_core::{session::Session, Graph};
use serde_json::{json, Value};
use std::{cell::OnceCell, collections::HashMap, io::{self, Write}, ops::Range, path::Path,
    sync::{atomic::{AtomicU64, Ordering}, Arc}, time::Instant};

static SAVE_TEMP_ID: AtomicU64 = AtomicU64::new(0);


/// Keeps original spelling, whitespace and unknown fields. Edits replace only
/// the corresponding JSON token (or insert an omitted member).
#[derive(Clone)]
pub struct Document { pub(crate) value: Value, pub(crate) text: String, graph: OnceCell<Arc<Graph>>, revision: u64 }
impl Document {
    pub fn parse(text: String) -> Result<Self> {
        let text = text.strip_prefix('\u{feff}').unwrap_or(&text).to_owned();
        let value = serde_json::from_str(&text)?;
        let graph = Graph::from_value(&value)?;
        modifier_inputs::validate_document(&value)?;
        Ok(Self { value, text, graph: OnceCell::from(Arc::new(graph)), revision: 0 })
    }
    pub fn open(path: &Path) -> Result<Self> { Self::parse(std::fs::read_to_string(path)?) }
    pub fn value(&self) -> &Value { &self.value }
    pub fn graph(&self) -> &Graph { self.graph.get_or_init(|| Arc::new(Graph::from_value(&self.value).expect("validated document"))) }
    pub fn revision(&self) -> u64 { self.revision }
    pub fn text(&self) -> &str { &self.text }
    pub fn save(&self, path: &Path) -> Result<()> {
        save_with(path, self.text.as_bytes(), |file, bytes| file.write_all(bytes))
    }
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
        let next: Value = serde_json::from_str(&self.text).expect("validated JSON token replacement");
        if next != self.value {
            self.value = next;
            self.graph = OnceCell::new();
            self.revision = self.revision.wrapping_add(1);
        }
    }
    fn value_at(&self, keys: &[String]) -> &Value {
        keys.iter().fold(&self.value, |v,k| &v[k])
    }
}
/// Keep the destination untouched until every byte has reached a synced sibling file.
fn save_with(path: &Path, bytes: &[u8], write: impl FnOnce(&mut std::fs::File, &[u8]) -> io::Result<()>) -> Result<()> {
    let name = path.file_name().ok_or("save path has no file name")?.to_string_lossy();
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let (temp, mut file) = loop {
        let id = SAVE_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let temp = parent.join(format!(".{name}.{}.{}.tmp", std::process::id(), id));
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&temp) {
            Ok(file) => break (temp, file),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    };
    let result = write(&mut file, bytes)
        .and_then(|()| file.flush())
        .and_then(|()| file.sync_all());
    drop(file);
    let result = result.and_then(|()| std::fs::rename(&temp, path));
    if result.is_err() { let _ = std::fs::remove_file(&temp); }
    result.map_err(Into::into)
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

#[derive(Clone, Debug)]
pub enum Edit {
    Param { node: String, name: String, value: Value },
    Channel { node: String, path: String, expression: Option<String> },
    ChannelGroup { node: String, name: String, value: Option<Value>, channels: Vec<(String, Option<String>)> },
    LockParam { node: String, path: String, locked: bool },
    Bypass { node: String, flag: bool },
    Display(String),
    /// Cook the selected node for its viewport group overlay without changing the display.
    Preview(String),
    Replace(Value),
    Exposed(Vec<Value>),
    ModifierInputs { node: String, inputs: Vec<Value> },
    ModifierInputValue { node: String, id: String, value: Value },
    External { name: String, input: minihou_core::nodes::blender_input::ExternalInput },
    /// Timeline: the frame and fps cooks run at (`@Frame`/`@Time`, solvers). Not a document edit.
    Time { frame: f64, fps: f64 },
}
/// Emit a cook request only when the visible timeline actually advances.
pub fn timeline_time_edit(before: u32, after: u32, fps: f64) -> Option<Edit> {
    (before != after).then_some(Edit::Time { frame: after as f64, fps })
}
impl Document {
    /// Validate edits without creating or cooking a Session on the UI thread.
    pub fn edit(&mut self, edit: &Edit) -> Result<()> {
        match edit {
            Edit::Param { node, name, value } => {
                let typ = self.value["nodes"][node]["type"].as_str().ok_or("no such node")?.to_owned();
                let spec = minihou_core::nodes::get(&typ).ok_or("unsupported node")?;
                if !spec.params.iter().any(|p| p.name == name)
                    && !(typ == "controller" && minihou_core::controller::parameter(&self.value["nodes"][node], name).is_some())
                    && !(typ == "subnet" && minihou_core::nodes::subnet_public_params(&self.value["nodes"][node]).iter().any(|p| p["name"] == *name))
                { return Err("unknown parameter".into()); }
                self.set(&["nodes", node, "params", name], value.clone());
                if let Some(group) = self.value["nodes"][node]["linked_params_group"].as_str().map(str::to_owned) {
                    let peers: Vec<String> = self.value["nodes"].as_object().into_iter().flatten()
                        .filter(|(id, data)| *id != node && data["linked_params_group"] == group && data["type"] == typ)
                        .map(|(id, _)| id.clone()).collect();
                    for peer in peers { self.set(&["nodes", &peer, "params", name], value.clone()); }
                }
            }
            Edit::Channel { node, path, expression } => {
                let mut check=self.session()?;
                check.set_channel(node,path,expression.clone())?;
                let channels=check.graph().node_channels(node).ok_or("no such node")?;
                self.set(&["nodes",node,"channels"],json!(channels));
            }
            Edit::ChannelGroup { node, name, value, channels } => {
                let mut check=self.session()?;
                if let Some(value)=value {check.set_param(node,name,value.clone())?;}
                for (path,expression) in channels {check.set_channel(node,path,expression.clone())?;}
                if let Some(value)=value {self.set(&["nodes",node,"params",name],value.clone());}
                self.set(&["nodes",node,"channels"],json!(check.graph().node_channels(node).ok_or("no such node")?));
            }
            Edit::LockParam { node, path, locked } => {
                let mut check=self.session()?;
                check.set_locked_param(node,path,*locked)?;
                let locks=check.graph().node_locks(node).ok_or("no such node")?;
                self.set(&["nodes",node,"locked_params"],json!(locks));
            }
            Edit::Bypass { node, flag } => {
                if self.value["nodes"].get(node).is_none() { return Err("no such node".into()); }
                self.set(&["nodes", node, "flags", "bypass"], json!(flag));
            }
            Edit::Display(node) => self.set_display(node)?,
            Edit::Preview(_) => return Err("preview is not a document edit".into()),
            Edit::Exposed(entries) => self.set(&["exposed"], json!(entries)),
            Edit::ModifierInputs { node, inputs } => {
                if self.value["nodes"][node]["type"] != "modifier_input" { return Err("expected modifier_input node".into()); }
                modifier_inputs::validate(inputs)?;
                let mut check = self.value.clone();
                check["nodes"][node]["modifier_inputs"] = json!(inputs);
                modifier_inputs::validate_document(&check)?;
                let retained: std::collections::HashSet<_> = inputs.iter().filter_map(|p| p["id"].as_str()).collect();
                let removed: std::collections::HashSet<_> = self.value["nodes"][node]["modifier_inputs"].as_array()
                    .into_iter().flatten().filter_map(|p| p["id"].as_str())
                    .filter(|id| !retained.contains(id)).map(str::to_owned).collect();
                self.set(&["nodes", node, "modifier_inputs"], json!(inputs));
                if !removed.is_empty() {
                    let mut values = self.value["nodes"][node]["input_values"].as_object().cloned().unwrap_or_default();
                    values.retain(|id, _| retained.contains(id.as_str()));
                    self.set(&["nodes", node, "input_values"], Value::Object(values));
                }
            }
            Edit::ModifierInputValue { node, id, value } => {
                let mut check = self.value.clone();
                modifier_inputs::set_value(&mut check, node, id, value.clone())?;
                self.set(&["nodes", node, "input_values", id], value.clone());
            }
            Edit::External { .. } => return Err("live inputs are not document edits".into()),
            Edit::Time { .. } => return Err("the timeline is not a document edit".into()),
            Edit::Replace(_) => return Err("replace uses document snapshots".into()),
        }
        Ok(())
    }
}

#[cfg(test)]
mod default_graph_tests {
    use super::*;
    #[test]
    fn existing_python_compatible_graph_needs_no_channel_fields() {
        let doc=Document::parse(include_str!("default_graph.json").into()).unwrap();
        let graph=Graph::from_value(doc.value()).unwrap();
        assert_eq!(graph.to_value(),*doc.value());
        assert!(graph.nodes.iter().all(|(_,node)|node.channels.is_none()&&node.locked_params.is_none()));
    }

    #[test]
    fn input_and_output_indices_round_trip_through_save_and_load() {
        let mut doc = Document::parse(json!({"nodes": {
            "in": {"type": "input"},
            "out": {"type": "output", "inputs": {"geo": "in"}}
        }, "display": "out"}).to_string()).unwrap();
        for (node, index) in [("in", 3), ("out", 7)] {
            doc.edit(&Edit::Param { node: node.into(), name: "index".into(), value: json!(index) }).unwrap();
        }
        let path = std::env::temp_dir().join(format!("minihou-index-test-{}-{}.json",
            std::process::id(), SAVE_TEMP_ID.fetch_add(1, Ordering::Relaxed)));
        doc.save(&path).unwrap();
        let loaded = Document::open(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(loaded.value()["nodes"]["in"]["params"]["index"], 3);
        assert_eq!(loaded.value()["nodes"]["out"]["params"]["index"], 7);
        assert_eq!(loaded.graph().cook(None).unwrap().npoints, 0);
    }

    #[test]
    fn graph_snapshot_changes_only_with_document_value() {
        let mut doc = Document::parse(json!({"nodes":{"box":{"type":"box"}},"display":"box"}).to_string()).unwrap();
        let old_graph = doc.graph.get().unwrap().clone();
        assert!(std::ptr::eq(doc.graph(), doc.graph()));
        assert_eq!(doc.revision(), 0);
        doc.edit(&Edit::Bypass { node: "box".into(), flag: true }).unwrap();
        assert_eq!(doc.revision(), 1);
        assert!(doc.graph().nodes[0].1.bypass);
        assert!(!std::ptr::eq(old_graph.as_ref(), doc.graph()));
        let graph = doc.graph() as *const Graph;
        doc.edit(&Edit::Bypass { node: "box".into(), flag: true }).unwrap();
        assert_eq!(doc.revision(), 1);
        assert_eq!(graph, doc.graph() as *const Graph);
    }

    #[test]
    fn failed_save_preserves_original_and_removes_temp() {
        let dir = std::env::temp_dir().join(format!("minihou-save-test-{}-{}", std::process::id(), SAVE_TEMP_ID.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("project.minihou");
        let original = b"original valid project";
        std::fs::write(&path, original).unwrap();
        let result = save_with(&path, b"replacement", |file, bytes| {
            file.write_all(&bytes[..3])?;
            Err(io::Error::other("injected write failure"))
        });
        assert!(result.is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        let doc = Document::parse(json!({"nodes":{}}).to_string()).unwrap();
        doc.save(&path).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), doc.text().as_bytes());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

/// File identity, save point and history are replaced together on a successful Open.
pub struct EditorDocument {
    pub doc: Document,
    pub history: History,
    pub path: std::path::PathBuf,
    saved: String,
    /// Set when this document is bound to a Blender network (`<object> / <modifier>`).
    /// Its edits sync back to Blender; Save As only exports a copy.
    pub blender_label: Option<String>,
}
impl EditorDocument {
    pub fn new(doc: Document, path: std::path::PathBuf) -> Self {
        Self { saved:doc.text().into(), doc, path, history:History::default(), blender_label: None }
    }
    pub fn dirty(&self) -> bool { self.doc.text() != self.saved }
    pub fn title(&self) -> String {
        if let Some(label) = &self.blender_label {
            return format!("MiniHou - {}", graph_embed::binding_title(label));
        }
        format!("MiniHou - {}{}", self.path.file_name().unwrap_or_default().to_string_lossy(), if self.dirty() { " *" } else { "" })
    }
    /// Write a separate copy (e.g. a `.minihou` file for sharing) without changing this
    /// document's identity, save point or Blender binding.
    pub fn export_to(&mut self, path: &Path) -> Result<()> {
        self.history.finish(&self.doc);
        self.doc.save(path)
    }
    pub fn save_to(&mut self, path: std::path::PathBuf) -> Result<()> {
        self.history.finish(&self.doc);
        self.doc.save(&path)?;
        self.path = path;
        self.saved = self.doc.text().into();
        Ok(())
    }
}
