//! Window-independent editor state and lossless JSON edits.
extern crate self as minihou_app;
pub mod cluster_lod;
pub mod link_protocol;
pub mod graph_embed;
pub mod http_api;
pub mod http_ops;
pub mod http_geometry;
#[cfg(test)] mod http_tests;
pub mod transform_math;
pub mod noodle_hit;
pub mod graph_edit;
pub mod modifier_inputs;
pub mod object_names;
pub mod input_define;
pub mod geo_info;
pub mod materials;
pub mod image_assets;
pub mod params;
pub mod value_link_ui;
use minihou_core::{session::Session, lod::ClusterLod, Geometry, Graph};
use serde_json::{json, Value};
use std::{collections::HashMap, ops::Range, path::Path, sync::Arc, time::Instant};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub mod units;
pub mod inference;
pub use units::{LengthUnit, UnitSettings};
pub mod history;
pub use history::{History, HistoryOutcome, SelectionSnapshot, SelectionState};
pub mod cook_worker;
pub use cook_worker::{cook_batch, CookRequest, CookResponse, CookState, CookWorker};
pub mod camera;
pub use camera::{sanitize_clip_planes, Camera, StandardView};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

#[cfg(test)]
mod fence_asset_tests {
    use super::*;

    #[test]
    fn fence_asset_cooks_headlessly_with_expected_counts_and_bounds() {
        let doc = Document::parse(include_str!("test_data/fence.minihou").to_owned()).unwrap();
        let geo = doc.session().unwrap().cook(None).unwrap();
        assert_eq!(geo.detail["fence_bays"], 4);
        assert_eq!(geo.detail["fence_posts"], 5);
        assert_eq!(geo.detail["fence_slats"], 36);
        assert_eq!(geo.npoints / 8, 5 + 36 + 4 * 3 + 4 * 2);
        assert!(geo.p().iter().all(|p| p.is_finite()));
        let max_x = geo.p().chunks_exact(3).map(|p| p[0]).fold(f32::NEG_INFINITY, f32::max);
        let max_y = geo.p().chunks_exact(3).map(|p| p[1]).fold(f32::NEG_INFINITY, f32::max);
        assert!(max_x > 5.9 && max_x < 6.2);
        assert!(max_y > 0.35 && max_y < 0.55);
    }
}

/// Keeps original spelling, whitespace and unknown fields. Edits replace only
/// the corresponding JSON token (or insert an omitted member).
#[derive(Clone)]
pub struct Document { pub(crate) value: Value, pub(crate) text: String }
impl Document {
    pub fn parse(text: String) -> Result<Self> {
        let text = text.strip_prefix('\u{feff}').unwrap_or(&text).to_owned();
        let value = serde_json::from_str(&text)?;
        Graph::from_value(&value)?;
        modifier_inputs::validate_document(&value)?;
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
                if !spec.params.iter().any(|p| p.name == name) && !(typ == "controller" && minihou_core::controller::parameter(&self.value["nodes"][node], name).is_some()) { return Err("unknown parameter".into()); }
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
                let mut resets = Vec::new();
                if let Some(nodes) = self.value["nodes"].as_object() {
                    for (nid, data) in nodes {
                        if let Some(params) = data["params"].as_object() {
                            for (name, value) in params {
                                let link = &value["modifier_input"];
                                let referenced = link["node"] == *node && link["id"].as_str().is_some_and(|id| removed.contains(id))
                                    || link.as_str().is_some_and(|id| removed.contains(id));
                                if referenced {
                                    let default = data["type"].as_str().and_then(minihou_core::nodes::get)
                                        .and_then(|s| s.params.iter().find(|p| p.name == name)).map(|p| (p.default)()).unwrap_or(Value::Null);
                                    resets.push((nid.clone(), name.clone(), default));
                                }
                            }
                        }
                    }
                }
                self.set(&["nodes", node, "modifier_inputs"], json!(inputs));
                if !removed.is_empty() {
                    let mut values = self.value["nodes"][node]["input_values"].as_object().cloned().unwrap_or_default();
                    values.retain(|id, _| retained.contains(id.as_str()));
                    self.set(&["nodes", node, "input_values"], Value::Object(values));
                    for (nid, name, value) in resets { self.set(&["nodes", &nid, "params", &name], value); }
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
    for p in controller_entries(doc) {
        let Some(name) = p["name"].as_str() else { continue };
        if names.insert(name.to_owned()) { params.push(p); }
    }
    for mut p in modifier_inputs::interface_entries(doc) {
        let Some(name) = p["name"].as_str().map(str::to_owned) else { continue };
        if !names.insert(name) { continue; }
        p["label"] = p["name"].clone();
        params.push(p);
    }
    // This is an opaque change token in the addon, not a mandated hashing algorithm.
    let structure: Vec<_> = params.iter().cloned().map(|mut p| { p.as_object_mut().unwrap().remove("value"); p }).collect();
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    serde_json::to_string(&structure).unwrap().hash(&mut hash);
    json!({"type":"interface","structure":format!("{:016x}",hash.finish()),"params":params})
}

/// Controller controls use their saved section order; each exposed control becomes one Blender input.
pub fn controller_entries(doc: &Value) -> Vec<Value> {
    let mut out = Vec::new();
    for (node_id, node) in doc["nodes"].as_object().into_iter().flatten() {
        if node["type"] != "controller" { continue; }
        for section in node["controller_sections"].as_array().into_iter().flatten() {
            let section_name = section["name"].as_str().unwrap_or("");
            for entry in section["parameters"].as_array().into_iter().flatten() {
                if entry["expose_to_blender"] != true { continue; }
                let name = entry["name"].as_str().unwrap_or("");
                let kind = match entry["kind"].as_str().unwrap_or("") { "vector" => "vec3", "color" => "color", k => k };
                let default = &entry["default"];
                out.push(json!({"name":format!("{node_id}_{name}"),"label":name,"node":node_id,"param":name,
                    "kind":kind,"default":default,"value":node["params"].get(name).unwrap_or(default),"section":section_name}));
            }
        }
    }
    out
}

#[cfg(test)]
mod controller_tests {
    use super::*;
    #[test]
    fn controller_exposes_only_flagged_controls_in_saved_sections() {
        let original=json!({"version":1,"display":"driver","nodes":{"driver":{"type":"controller",
            "controller_sections":[
                {"name":"Shape","parameters":[{"name":"size","kind":"float","default":1.0,"expose_to_blender":true},{"name":"note","kind":"string","default":"hello"}]},
                {"name":"Flags","parameters":[{"name":"enabled","kind":"bool","default":true,"expose_to_blender":true}]}],
            "params":{"size":2.0}}}});
        let doc=Document::parse(original.to_string()).unwrap();
        let restored=Document::parse(doc.text().to_owned()).unwrap();
        assert_eq!(restored.value()["nodes"]["driver"]["controller_sections"],original["nodes"]["driver"]["controller_sections"]);
        let iface=link_interface(restored.value());
        assert_eq!(iface["params"].as_array().unwrap().len(),2);
        assert_eq!(iface["params"][0]["section"],"Shape");
        assert_eq!(iface["params"][1]["section"],"Flags");
        assert_eq!(iface["params"][0]["value"],json!(2.0));
        let edit=link_param_edit(restored.value(),&json!({"name":"driver_size","value":3.0})).unwrap();
        assert!(matches!(edit,Edit::Param{node,name,value} if node=="driver" && name=="size" && value==json!(3.0)));
    }
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
    if let Some(name) = message["name"].as_str() {
        if let Some(entry) = controller_entries(doc).into_iter().find(|e| e["name"] == name) {
            let node = entry["node"].as_str().unwrap().to_owned();
            let param = entry["param"].as_str().unwrap().to_owned();
            let value = snap_float32(message.get("value").ok_or("missing value")?);
            let definition = minihou_core::controller::parameter(&doc["nodes"][&node], &param).ok_or("unknown controller parameter")?;
            if !minihou_core::controller::valid_value(definition["kind"].as_str().unwrap(), &value, &definition["options"]) { return Err("invalid controller value".into()); }
            return Ok(Edit::Param { node, name:param, value });
        }
        if let Some(entry) = modifier_inputs::interface_entries(doc).into_iter().find(|e| e["name"] == name) {
            if entry["implicit"] == true {
                // A picked Blender object names the node's object param (standalone use); the live feed wins.
                let value = message.get("value").ok_or("missing value")?;
                let value = value.as_str().ok_or("object picker value must be a name")?;
                return Ok(Edit::Param { node: entry["graph_node"].as_str().unwrap().to_owned(), name: "object".into(), value: json!(value) });
            }
            let node = entry["node"].as_str().unwrap().to_owned();
            let id = entry["id"].as_str().unwrap().to_owned();
            let value = message.get("value").ok_or("missing value")?.clone();
            let mut check = doc.clone();
            modifier_inputs::set_value(&mut check, &node, &id, value.clone())?;
            return Ok(Edit::ModifierInputValue { node, id, value });
        }
    }
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
    fn network_linked_parameter_edits_follow_both_directions_and_undo_together() {
        let original = json!({"display":"a","nodes":{
            "a":{"type":"box","linked_params_group":"a"},
            "b":{"type":"box","linked_params_group":"a"}
        }});
        let mut doc = Document::parse(original.to_string()).unwrap();
        let mut history = History::default();
        history.edit(&mut doc, &Edit::Param {node:"b".into(), name:"size".into(), value:json!([2,3,4])}).unwrap();
        history.finish(&doc);
        assert_eq!(doc.value()["nodes"]["a"]["params"]["size"], json!([2,3,4]));
        assert_eq!(doc.value()["nodes"]["b"]["params"]["size"], json!([2,3,4]));
        history.undo(&mut doc);
        assert!(doc.value()["nodes"]["a"]["params"].get("size").is_none());
        history.redo(&mut doc);
        assert_eq!(doc.value()["nodes"]["a"]["params"]["size"], json!([2,3,4]));
    }
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
