//! A persistent, incremental evaluator: edit a parameter, cook again, and only the nodes whose recipe changed run.
//!
//! Each node's result is cached under a content key: a hash of its type, its parameters (canonical JSON, keys sorted), its
//! bypass flag and the keys of what it reads, in port order. There are no dirty flags to keep in step with edits: a changed
//! parameter changes that node's key and, through it, the keys of everything downstream, and nothing else. A cook computes the
//! keys first (cheap, no geometry), then pulls from the target: a cache hit stops there without touching its inputs.
//!
//! The result of `Session::cook` is the same as [`crate::graph::Graph::cook`] on the edited graph (both run nodes through
//! [`crate::graph::run_node`]); `tests/session.rs` and `tests/test_rust_session.py` check that after random edit sequences.
//!
//! The cache is bounded two ways, both applied at the end of a cook (never during one: a cook may need an input it cooked a
//! moment ago). Per node it keeps its newest `keep_versions` results (default 2: the current one and the previous one, so
//! toggling a value back is a cache hit while a slider drag does not pile up one version per tick). Overall an optional
//! byte budget evicts least-recently-used entries. Bytes are counted per geometry, so an array shared between two cached
//! geometries is counted twice: the budget errs on the side of evicting early.

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::any::Any;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Instant;

use serde_json::Value;

use crate::error::{Error, Result};
use crate::geometry::{AttrData, Geometry};
use crate::graph::{run_node_outputs, Graph};
use crate::params::Params;
use crate::nodes;

/// What the last cook did: which nodes ran, which were served from the cache, and how long each cook took (ms).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CookStats {
    pub cooked: Vec<String>,
    pub cached: Vec<String>,
    pub node_ms: Vec<(String, f64)>,
    /// One record per evaluated node (including cache hits), in evaluation order.
    pub nodes: Vec<NodeCookDiagnostic>,
    pub param_diags: Vec<crate::channels::ParamDiag>,
    /// Nodes waiting for external data (no Blender object picked, Blender not connected): (node, message).
    /// They cook empty geometry; this is a neutral state, not an error.
    pub waiting: Vec<(String, String)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NodeCookDiagnostic {
    pub node_id: String,
    pub cache_hit: bool,
    pub cook_ms: f64,
    pub point_count: usize,
    pub primitive_count: usize,
}

impl NodeCookDiagnostic {
    fn new(id: &str, cache_hit: bool, cook_ms: f64, geo: &Geometry) -> Self {
        Self { node_id: id.into(), cache_hit, cook_ms, point_count: geo.npoints, primitive_count: geo.nprims() }
    }
}

struct Entry {
    node: String,
    geo: Arc<Geometry>,
    outputs: Vec<Arc<Geometry>>,
    side: Option<Arc<dyn Any + Send + Sync>>,
    bytes: usize,
    used: u64,
}

struct FrameEntry {
    geo: Arc<Geometry>,
    bytes: usize,
    used: u64,
}

const DEFAULT_FRAME_BUDGET: usize = 2 * 1024 * 1024 * 1024;

pub struct Session {
    graph: Graph,
    external: nodes::blender_input::ExternalInputs,
    external_versions: HashMap<String,u64>,
    external_revision: u64,
    cache: HashMap<u64, Entry>,
    frames: HashMap<(String, u64, u64), FrameEntry>,
    frame_recipes: HashMap<String, u64>,
    frame_bytes: usize,
    frame_budget: usize,
    bytes: usize,
    budget: Option<usize>,
    keep_versions: usize,
    clock: u64,
    last: CookStats,
    /// (frame, fps) cooks run at.
    time: (f64, f64),
    /// Permission for `python` nodes; None (the default) denies them.
    python: Option<Arc<crate::python_bridge::PythonGate>>,
    /// Asked before each node that has to run: true abandons the cook (a newer request superseded it). Nodes already
    /// finished stay cached, so the next cook picks up where this one stopped.
    cancel: Option<CancelCheck>,
}

/// See [`Session::set_cancel`].
pub type CancelCheck = Arc<dyn Fn() -> bool + Send + Sync>;

/// The error text of a cook abandoned through [`Session::set_cancel`].
pub const SUPERSEDED: &str = "cook superseded by a newer edit";

impl Session {
    pub fn new(graph: Graph) -> Session {
        Session { graph, external: Default::default(), external_versions: Default::default(), external_revision:0, cache: HashMap::new(), frames: HashMap::new(), frame_recipes: HashMap::new(), frame_bytes: 0, frame_budget: DEFAULT_FRAME_BUDGET, bytes: 0, budget: None, keep_versions: 2, clock: 0, last: CookStats::default(), time: (1.0, crate::timectx::DEFAULT_FPS), python: None, cancel: None }
    }

    pub fn from_json(text: &str) -> Result<Session> {
        Ok(Session::new(Graph::from_json(text)?))
    }

    /// Keep live inputs across graph edits, undo and timeline recooks.
    /// The cache is content keyed, so it stays valid: a structural edit, undo or redo recooks only the nodes whose recipe
    /// changed. Entries of nodes that no longer exist are dropped.
    pub fn replace_graph(&mut self, graph: Graph) {
        self.graph = graph;
        let live: HashSet<_> = self.graph.nodes.iter().filter(|(_, d)| d.node_type == "cache").map(|(id, _)| id.as_str()).collect();
        self.frames.retain(|(id, _, _), e| { let keep = live.contains(id.as_str()); if !keep { self.frame_bytes -= e.bytes; } keep });
        self.frame_recipes.retain(|id, _| live.contains(id.as_str()));
        let (graph, bytes) = (&self.graph, &mut self.bytes);
        self.cache.retain(|_, e| {
            let keep = graph.node(&e.node).is_some();
            if !keep { *bytes -= e.bytes; }
            keep
        });
    }

    /// Install (or remove) the check a cook asks before running each node; see [`SUPERSEDED`].
    pub fn set_cancel(&mut self, cancel: Option<CancelCheck>) { self.cancel = cancel; }
    pub fn set_external(&mut self, name: String, input: nodes::blender_input::ExternalInput) {
        self.external_revision+=1;
        self.external_versions.insert(name.clone(),self.external_revision);
        self.external.insert(name,input);
    }
    pub fn graph(&self) -> &Graph {
        &self.graph
    }

    /// Current evaluated values and warnings, without cooking any geometry.
    pub fn resolved_channels(&self) -> crate::channels::ResolvedChannels {
        crate::timectx::at(Some(self.time.0),Some(self.time.1),||crate::channels::resolve_graph(&self.graph))
    }

    /// Set (or add) one parameter of a node. Unknown nodes and parameter names are errors here; a value the node refuses is
    /// an error at cook time, as in a graph file.
    pub fn set_param(&mut self, node: &str, name: &str, value: Value) -> Result<()> {
        let controller = self.graph.controller_param(node, name).is_some();
        let promoted = self.graph.promoted_param(node, name).is_some();
        let def = self.graph.nodes.iter_mut().find(|(n, _)| n == node).map(|(_, d)| d)
            .ok_or_else(|| Error::new(format!("no such node '{node}'")))?;
        let spec = nodes::get(&def.node_type).ok_or_else(|| {
            Error::new(format!("node '{node}': node type '{}' is not in the Rust backend yet", def.node_type))
        })?;
        if !spec.params.iter().any(|p| p.name == name) && !controller && !promoted {
            return Err(Error::new(format!("node '{node}' ({}): unknown parameter {name:?}", def.node_type)));
        }
        def.params.get_or_insert_with(Default::default).insert(name.to_string(), value);
        Ok(())
    }

    pub fn set_channel(&mut self, node: &str, path: &str, expression: Option<String>) -> Result<()> {
        let (base, component) = crate::channels::split_component(path);
        let controller = self.graph.controller_param(node, base).map(|p| p["default"].clone());
        let promoted = self.graph.promoted_param(node, base).map(|p| p["default"].clone());
        let def = self.graph.nodes.iter_mut().find(|(n, _)| n == node).map(|(_, d)| d)
            .ok_or_else(|| Error::new(format!("no such node '{node}'")))?;
        let spec = nodes::get(&def.node_type).ok_or_else(|| Error::new("unsupported node"))?;
        let target = spec.params.iter().find(|p| p.name == base).map(|p| (p.default)()).or(controller).or(promoted).ok_or_else(|| Error::new(format!("unknown parameter {path:?}")))?;
        if component.is_some() && !target.as_array().is_some_and(|a| a.len()==3) {
            return Err(Error::new(format!("parameter {path:?} has no vector component")));
        }
        let channels = def.channels.get_or_insert_with(Default::default);
        if let Some(expression)=expression {channels.insert(path.into(),Value::String(expression));}
        else {channels.remove(path);}
        Ok(())
    }

    pub fn set_locked_param(&mut self, node: &str, path: &str, locked: bool) -> Result<()> {
        let def = self.graph.nodes.iter_mut().find(|(n, _)| n == node).map(|(_, d)| d)
            .ok_or_else(|| Error::new(format!("no such node '{node}'")))?;
        let spec = nodes::get(&def.node_type).ok_or_else(|| Error::new("unsupported node"))?;
        let (base, component) = crate::channels::split_component(path);
        let target = spec.params.iter().find(|p| p.name == base).ok_or_else(|| Error::new(format!("unknown parameter {path:?}")))?;
        if component.is_some() && !(target.default)().as_array().is_some_and(|a| a.len()==3) {return Err(Error::new("parameter has no vector component"));}
        let list=def.locked_params.get_or_insert_with(Vec::new);
        list.retain(|p|p!=path);
        if locked {list.push(path.into());}
        Ok(())
    }

    pub fn set_bypass(&mut self, node: &str, flag: bool) -> Result<()> {
        let def = self.graph.nodes.iter_mut().find(|(n, _)| n == node).map(|(_, d)| d)
            .ok_or_else(|| Error::new(format!("no such node '{node}'")))?;
        def.bypass = flag;
        Ok(())
    }

    /// The frame and fps cooks run at (see [`crate::timectx`]; default frame 1 at 24 fps). Only time-dependent nodes (and
    /// what reads them) recook when it changes.
    pub fn set_time(&mut self, frame: f64, fps: f64) -> Result<()> {
        if !frame.is_finite() || !fps.is_finite() || fps <= 0.0 {
            return Err(Error::new("frame must be finite and fps positive"));
        }
        self.time = (frame, fps);
        Ok(())
    }

    pub fn time(&self) -> (f64, f64) {
        self.time
    }

    /// Let `python` nodes ask `gate` for permission (None: denied, the default). Only the app's own UI sets this.
    pub fn set_python_gate(&mut self, gate: Option<Arc<crate::python_bridge::PythonGate>>) {
        self.python = gate;
    }

    /// Cook `target` (default: the graph's main output), running only the nodes the cache does not already hold.
    pub fn cook(&mut self, target: Option<&str>) -> Result<Arc<Geometry>> {
        let (frame, fps) = self.time;
        let gate = self.python.clone();
        crate::python_bridge::with_gate(gate, || crate::timectx::at(Some(frame), Some(fps), || self.cook_now(target)))
    }

    fn cook_now(&mut self, target: Option<&str>) -> Result<Arc<Geometry>> {
        if self.graph.nodes.iter().any(|(_,n)|n.node_type=="subnet") {
            let source=self.graph.to_value();
            if source["nodes"].as_object().is_some_and(|nodes|nodes.values().any(|n|n["type"]=="subnet" && n["inner"].is_object())) {
            let mut expanded=source.clone();
            expanded["nodes"]=Value::Object(crate::graph::expand_subnets(source["nodes"].as_object().unwrap())?.unwrap());
            let cooked_graph=std::mem::replace(&mut self.graph,Graph::from_value(&expanded)?);
            let result=self.cook_now(target);
            self.graph=cooked_graph;
            return result;
            }
        }
        let target = match target {
            Some(t) => t.to_string(),
            None => self.graph.main_output()?,
        };
        let mut keys: HashMap<String, u64> = HashMap::new();
        self.last = CookStats::default();
        let resolved = crate::channels::resolve_graph(&self.graph);
        let plan = nodes::r#loop::Plan::analyze(&self.graph, &target)?;
        nodes::rbd_engine::check_static_inputs(&self.graph, &target)?;
        key_of(&self.graph, &resolved, &self.external_versions, &target, &mut keys, &mut HashSet::new())?;
        self.clock += 1;
        let mut stats = CookStats { param_diags: resolved.diagnostics.clone(), ..Default::default() };
        let mut run = Run { cancel: self.cancel.as_deref(), graph: &self.graph, resolved: &resolved, plan: &plan, budget: nodes::r#loop::budget(), external: &self.external, external_revision: self.external_revision, cache: &mut self.cache, frames: &mut self.frames, frame_recipes: &mut self.frame_recipes, frame_bytes: &mut self.frame_bytes, bytes: &mut self.bytes, clock: self.clock, frame: self.time.0.to_bits(), keys: &keys, stats: &mut stats };
        let result = nodes::usd_nodes::with_context(&self.graph, &self.external, || run.get(&target));
        stats.waiting = self.waiting(&resolved);
        self.last = stats;
        self.evict();
        self.evict_frames();
        result
    }

    /// Every external-data node currently waiting (whether or not the last cook ran it).
    fn waiting(&self, resolved: &crate::channels::ResolvedChannels) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for (id, def) in &self.graph.nodes {
            if def.bypass || !matches!(def.node_type.as_str(), "blender_object" | "blender_image" | "object_input") { continue; }
            let Some(spec) = nodes::get(&def.node_type) else { continue };
            let Ok(p) = Params::resolve(spec.params, resolved.params.get(id).or(def.params.as_ref())) else { continue };
            if let Some(msg) = nodes::blender_input::waiting_reason(id, &def.node_type, &p, &self.external) { out.push((id.clone(), msg)); }
        }
        out
    }

    pub fn last_stats(&self) -> &CookStats {
        &self.last
    }

    pub fn cache_bytes(&self) -> usize {
        self.bytes + self.frame_bytes
    }

    pub fn cache_len(&self) -> usize {
        self.cache.len()
    }

    /// Typed side result for a node's current content key. It is evicted with its geometry.
    pub fn side_payload<T: Any + Send + Sync>(&self, node: &str) -> Option<Arc<T>> {
        let key = crate::python_bridge::with_gate(self.python.clone(), || crate::timectx::at(Some(self.time.0), Some(self.time.1), || key_of(&self.graph, &crate::channels::resolve_graph(&self.graph), &self.external_versions, node, &mut HashMap::new(), &mut HashSet::new()))).ok()?;
        self.cache.get(&key)?.side.as_ref()?.clone().downcast::<T>().ok()
    }
    pub fn cached_geometry(&self, node: &str) -> Option<Arc<Geometry>> {
        let (key, keys) = crate::python_bridge::with_gate(self.python.clone(), || crate::timectx::at(Some(self.time.0), Some(self.time.1), || {
            let mut keys = HashMap::new();
            let key = key_of(&self.graph, &crate::channels::resolve_graph(&self.graph), &self.external_versions, node, &mut keys, &mut HashSet::new())?;
            Ok::<_, Error>((key, keys))
        })).ok()?;
        if let Some(def) = self.graph.node(node).filter(|def| def.node_type == "cache" && !def.bypass) {
            let input_hash = input_hash(def, &keys);
            return Some(self.frames.get(&(node.to_owned(), self.time.0.to_bits(), input_hash))?.geo.clone());
        }
        Some(self.cache.get(&key)?.geo.clone())
    }

    /// `None` or `Some(0)`: unlimited.
    pub fn set_budget_bytes(&mut self, budget: Option<usize>) {
        self.budget = budget.filter(|b| *b > 0);
        self.frame_budget = self.budget.unwrap_or(DEFAULT_FRAME_BUDGET);
        self.evict();
        self.evict_frames();
    }

    pub fn clear(&mut self) {
        self.cache.clear();
        self.bytes = 0;
        self.frames.clear();
        self.frame_recipes.clear();
        self.frame_bytes = 0;
    }

    /// Drop all stored frames of one Cache node.
    pub fn clear_cache_frames(&mut self, node: &str) {
        self.frames.retain(|(id, _, _), e| { let keep = id != node; if !keep { self.frame_bytes -= e.bytes; } keep });
        self.cache.clear(); self.bytes = 0;
    }

    /// Drop the current timeline frame of one Cache node.
    pub fn clear_current_frame_cache(&mut self, node: &str) {
        let frame = self.time.0.to_bits();
        self.frames.retain(|(id, f, _), e| { let keep = id != node || *f != frame; if !keep { self.frame_bytes -= e.bytes; } keep });
        self.cache.clear(); self.bytes = 0;
    }

    pub fn frame_cache_len(&self, node: &str) -> usize {
        self.frames.keys().filter(|(id, _, _)| id == node).count()
    }

    pub fn set_frame_budget_bytes(&mut self, budget: usize) {
        self.frame_budget = budget;
        self.evict_frames();
    }

    fn evict_frames(&mut self) {
        let mut order: Vec<_> = self.frames.iter().map(|(k, e)| (e.used, k.clone())).collect();
        order.sort_unstable();
        for (_, key) in order {
            if self.frame_bytes <= self.frame_budget { break; }
            if let Some(e) = self.frames.remove(&key) { self.frame_bytes -= e.bytes; }
        }
    }

    /// How many results to keep per node (at least 1; default 2).
    pub fn set_keep_versions(&mut self, n: usize) {
        self.keep_versions = n.max(1);
        self.evict();
    }

    fn evict(&mut self) {
        // per node: drop everything but its newest `keep_versions` results
        let mut by_node: HashMap<&str, Vec<(u64, u64)>> = HashMap::new();
        for (key, e) in &self.cache {
            by_node.entry(e.node.as_str()).or_default().push((e.used, *key));
        }
        let mut stale: Vec<u64> = Vec::new();
        for versions in by_node.values_mut() {
            if versions.len() > self.keep_versions {
                versions.sort_unstable_by(|a, b| b.cmp(a));
                stale.extend(versions[self.keep_versions..].iter().map(|(_, k)| *k));
            }
        }
        for key in stale {
            if let Some(e) = self.cache.remove(&key) {
                self.bytes -= e.bytes;
            }
        }
        let Some(budget) = self.budget else { return };
        if self.bytes <= budget {
            return;
        }
        let mut order: Vec<(u64, u64)> = self.cache.iter().map(|(k, e)| (e.used, *k)).collect();
        order.sort_unstable();
        for (_, key) in order {
            if self.bytes <= budget {
                break;
            }
            if let Some(e) = self.cache.remove(&key) {
                self.bytes -= e.bytes;
            }
        }
    }
}

/// The content key of `id`: validates what the cook would (unknown node or type, bad ports, cycles) with the same wording as
/// [`Graph::cook`].
fn key_of(graph: &Graph, resolved: &crate::channels::ResolvedChannels, external_versions: &HashMap<String,u64>, id: &str, keys: &mut HashMap<String, u64>, active: &mut HashSet<String>) -> Result<u64> {
    if let Some(k) = keys.get(id) {
        return Ok(*k);
    }
    let def = graph.node(id).ok_or_else(|| Error::new(format!("no such node '{id}'")))?;
    if !active.insert(id.to_string()) {
        return Err(Error::new(format!("the graph has a cycle through node '{id}'")));
    }
    let spec = nodes::get(&def.node_type).ok_or_else(|| {
        Error::new(format!("node '{id}': node type '{}' is not in the Rust backend yet", def.node_type))
    })?;
    let values = resolved.params.get(id).or(def.params.as_ref());
    for (port, _) in &def.inputs {
        if !spec.inputs.iter().any(|i| i.name == port) {
            return Err(Error::new(format!("node '{id}': '{}' has no input '{port}'", def.node_type)));
        }
    }
    let mut h = DefaultHasher::new();
    def.node_type.hash(&mut h);
    if def.node_type == "usd_export" {
        nodes::usd_nodes::exports_suppressed().hash(&mut h);
        let p = Params::resolve(spec.params, values)?;
        if let Ok(path) = nodes::usd_nodes::export_target(&p) {
            if let Ok(meta) = std::fs::metadata(path) { meta.len().hash(&mut h); meta.modified().ok().hash(&mut h); }
        }
    }
    if def.node_type == "usd_import" || def.node_type == "image_file" {
        let p = Params::resolve(spec.params, values)?;
        let path = crate::usd_io::resolve_path(&p.s("file")?);
        path.hash(&mut h);
        if let Ok(meta) = std::fs::metadata(path) { meta.len().hash(&mut h); meta.modified().ok().hash(&mut h); }
    }
    if def.node_type == "python" {
        let p = Params::resolve(spec.params, values)?;
        crate::python_bridge::key_material(&p.s("code")?).hash(&mut h);
    }
    if def.node_type == "copy_to_points" {
        nodes::copy_limit_bytes().hash(&mut h);
    }
    let binding=match def.node_type.as_str() {
        "blender_object" => Some(format!("object:{}",values.and_then(|p|p.get("object")).and_then(Value::as_str).unwrap_or(""))),
        "blender_image" => Some(format!("image:{}",values.and_then(|p|p.get("image")).and_then(Value::as_str).unwrap_or(""))),
        "object_input" => Some(nodes::blender_input::binding_key(id, &Params::resolve(spec.params,values)?)?),
        "modifier_input" => Some("modifier_input".into()), _ => None,
    };
    if let Some(binding)=binding { external_versions.get(&binding).hash(&mut h); }
    if def.node_type == "blender_object" { external_versions.get(&nodes::blender_input::picker_external(id)).hash(&mut h); }
    def.bypass.hash(&mut h);
    // time-dependent nodes (Python's key_extra = timectx.key()) recook when the frame changes
    if def.node_type == "cache" || nodes::is_time_dependent(&def.node_type, def.params.as_ref()) || resolved.time_dependent.contains(id) {
        crate::timectx::key().hash(&mut h);
    }
    match values {
        Some(p) => {
            1u8.hash(&mut h);
            let mut text = String::new();
            canonical(&Value::Object(p.clone()), &mut text);
            text.hash(&mut h);
        }
        None => 0u8.hash(&mut h),
    }
    for port in spec.inputs {
        let refs = def.inputs.iter().find(|(p, _)| p == port.name).map(|(_, r)| r.as_slice()).unwrap_or(&[]);
        if !port.multi && refs.len() > 1 {
            return Err(Error::new(format!("node '{id}': input '{}' takes one connection", port.name)));
        }
        port.name.hash(&mut h);
        refs.len().hash(&mut h);
        for (index, r) in refs.iter().enumerate() {
            key_of(graph, resolved, external_versions, r, keys, active)?.hash(&mut h);
            def.output_for(port.name, index, r).output.hash(&mut h);
        }
    }
    active.remove(id);
    let key = h.finish();
    keys.insert(id.to_string(), key);
    Ok(key)
}

/// Stable across timeline frames; changes when upstream graph structure or authored values change.
fn upstream_recipe(graph: &Graph, id: &str, memo: &mut HashMap<String, u64>) -> u64 {
    if let Some(hash) = memo.get(id) { return *hash; }
    let mut h = DefaultHasher::new();
    if let Some(def) = graph.node(id) {
        for (port, sources) in &def.inputs {
            port.hash(&mut h);
            for (index, source) in sources.iter().enumerate() {
                source.hash(&mut h);
                def.output_for(port, index, source).output.hash(&mut h);
                if source != id { upstream_recipe(graph, source, memo).hash(&mut h); }
            }
        }
        def.node_type.hash(&mut h);
        def.bypass.hash(&mut h);
        for values in [&def.params, &def.channels] {
            if let Some(values) = values { let mut canonical_values = String::new(); canonical(&Value::Object(values.clone()), &mut canonical_values); canonical_values.hash(&mut h); }
        }
    }
    let hash = h.finish();
    memo.insert(id.to_owned(), hash);
    hash
}

fn input_hash(def: &crate::graph::NodeDef, keys: &HashMap<String, u64>) -> u64 {
    let mut h = DefaultHasher::new();
    for (port, refs) in &def.inputs {
        port.hash(&mut h);
        for (index, source) in refs.iter().enumerate() {
            keys[source].hash(&mut h);
            def.output_for(port, index, source).output.hash(&mut h);
        }
    }
    h.finish()
}

/// JSON with object keys sorted, so equal parameters hash equal whatever order they were written in.
fn canonical(v: &Value, out: &mut String) {
    match v {
        Value::Object(m) => {
            let mut names: Vec<&String> = m.keys().collect();
            names.sort();
            out.push('{');
            for n in names {
                out.push_str(&Value::String(n.clone()).to_string());
                out.push(':');
                canonical(&m[n], out);
                out.push(',');
            }
            out.push('}');
        }
        Value::Array(a) => {
            out.push('[');
            for x in a {
                canonical(x, out);
                out.push(',');
            }
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}

#[cfg(test)]
mod channel_cache_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn driver_change_recooks_only_channel_dependents_without_cooking_driver_geometry() {
        let mut s=Session::new(Graph::from_value(&json!({"nodes":{
            "driver":{"type":"box","params":{"size":[2,2,2]}},
            "dependent":{"type":"box","params":{"size":[1,1,1]},"channels":{"size.x":"ch('../driver/size.x') * 2"}},
            "unrelated":{"type":"box","params":{"size":[1,1,1]}},
            "out":{"type":"merge","inputs":{"geos":["dependent","unrelated"]}}
        },"display":"out"})).unwrap());
        s.cook(None).unwrap();
        assert!(!s.last_stats().cooked.contains(&"driver".into()));
        s.set_param("driver","size",json!([2,3,2])).unwrap();
        s.cook(None).unwrap();
        assert!(s.last_stats().cooked.is_empty(),"{:?}",s.last_stats());
        s.set_param("driver","size",json!([3,2,2])).unwrap();
        s.cook(None).unwrap();
        assert!(s.last_stats().cooked.contains(&"dependent".into()));
        assert!(s.last_stats().cooked.contains(&"out".into()));
        assert!(!s.last_stats().cooked.contains(&"unrelated".into()));
        assert!(!s.last_stats().cooked.contains(&"driver".into()));
    }
    #[test]
    fn time_link_invalidates_only_dependent_nodes() {
        let mut s=Session::new(Graph::from_value(&json!({"nodes":{
            "dependent":{"type":"box","params":{"size":[1,1,1]},"channels":{"size.x":"@Frame"}},
            "unrelated":{"type":"box"}
        }})).unwrap());
        s.cook(Some("dependent")).unwrap();
        s.cook(Some("unrelated")).unwrap();
        s.set_time(2.,24.).unwrap();
        assert_eq!(s.resolved_channels().params["dependent"]["size"][0].as_f64(),Some(2.));
        s.cook(Some("dependent")).unwrap();
        assert_eq!(s.last_stats().cooked,vec!["dependent"]);
        s.cook(Some("unrelated")).unwrap();
        assert_eq!(s.last_stats().cached,vec!["unrelated"]);
    }
}

#[cfg(test)]
mod multi_output_cache_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn cache_shares_one_cook_across_output_ports() {
        let graph = Graph::from_value(&json!({"nodes": {
            "shape": {"type":"box"},
            "fill": {"type":"flood_fill", "inputs":{"geometry":"shape"},
                "params":{"seed_group":"0", "max_steps":0}},
            "pieces": {"type":"null", "inputs":{"geo":"fill"}},
            "frontier": {"type":"null", "inputs":{"geo":{"node":"fill","output":1}}}
        }})).unwrap();
        let mut session = Session::new(graph);
        let a = session.cook(Some("pieces")).unwrap();
        assert!(a.nprims() > 0);
        assert_eq!(session.last_stats().cooked.iter().filter(|id| *id == "fill").count(), 1);
        let b = session.cook(Some("frontier")).unwrap();
        assert!(b.nprims() > 0);
        assert!(session.last_stats().cached.contains(&"fill".into()));
        assert!(!session.last_stats().cooked.contains(&"fill".into()));
        assert!(session.cache_bytes() >= geometry_bytes(&a) + geometry_bytes(&b));
    }
}

#[cfg(test)]
mod sidecar_hook_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn another_node_attaches_and_caches_its_sidecar() {
        let graph = Graph::from_value(&json!({"nodes": {
            "simple": {"type": "test_sidecar"}
        }})).unwrap();
        let mut session = Session::new(graph);
        let geometry = session.cook(Some("simple")).unwrap();
        assert_eq!(geometry.npoints, 0);
        let payload = session.side_payload::<String>("simple").unwrap();
        assert_eq!(payload.as_str(), "attached");
        assert_eq!(session.cache_bytes(), geometry_bytes(&geometry) + 8);
        session.cook(Some("simple")).unwrap();
        assert_eq!(session.last_stats().cached, vec!["simple"]);
        assert!(Arc::ptr_eq(&payload, &session.side_payload::<String>("simple").unwrap()));
    }
}

struct Run<'a> {
    cancel: Option<&'a (dyn Fn() -> bool + Send + Sync)>,
    graph: &'a Graph,
    resolved: &'a crate::channels::ResolvedChannels,
    plan: &'a nodes::r#loop::Plan,
    budget: nodes::r#loop::Budget,
    external: &'a nodes::blender_input::ExternalInputs,
    external_revision: u64,
    cache: &'a mut HashMap<u64, Entry>,
    frames: &'a mut HashMap<(String, u64, u64), FrameEntry>,
    frame_recipes: &'a mut HashMap<String, u64>,
    frame_bytes: &'a mut usize,
    bytes: &'a mut usize,
    clock: u64,
    frame: u64,
    keys: &'a HashMap<String, u64>,
    stats: &'a mut CookStats,
}

impl Run<'_> {
    fn get(&mut self, id: &str) -> Result<Arc<Geometry>> {
        self.get_output(id, 0)
    }
    fn get_output(&mut self, id: &str, output: usize) -> Result<Arc<Geometry>> {
        let key = self.keys[id];
        let t0 = Instant::now();
        let def = self.graph.node(id).expect("validated by key_of");
        let cache_off = def.node_type == "cache" && !def.bypass &&
            self.resolved.params.get(id).or(def.params.as_ref())
                .and_then(|p| p.get("mode")).and_then(Value::as_str) == Some("off");
        if def.node_type == "cache" && !def.bypass {
            let spec = nodes::get("cache").expect("registered cache node");
            let p = Params::resolve(spec.params, self.resolved.params.get(id).or(def.params.as_ref())).map_err(|e| e.in_node(id, "cache"))?;
            p.e("mode", &["auto", "off"])?;
            p.b("cache_any_frame")?;
            let max_frames = p.i("max_frames")?;
            if max_frames < 1 { return Err(Error::new("max_frames must be positive").in_node(id, "cache")); }
            let mut recipe_hasher = DefaultHasher::new();
            self.external_revision.hash(&mut recipe_hasher);
            let mut memo = HashMap::new();
            for (port, sources) in &def.inputs {
                port.hash(&mut recipe_hasher);
                for (index, source) in sources.iter().enumerate() {
                    def.output_for(port, index, source).output.hash(&mut recipe_hasher);
                    upstream_recipe(self.graph, source, &mut memo).hash(&mut recipe_hasher);
                }
            }
            let recipe = recipe_hasher.finish();
            if p.b("clear_on_upstream_change")? && self.frame_recipes.get(id).is_some_and(|old| *old != recipe) {
                self.purge_frames(id, None);
            }
            self.frame_recipes.insert(id.to_owned(), recipe);
            if p.b("clear_cache")? { self.purge_frames(id, None); }
            if p.b("clear_current_frame_cache")? { self.purge_frames(id, Some(self.frame)); }
            if !cache_off {
                let frame_key = (id.to_owned(), self.frame, input_hash(def, self.keys));
                if let Some(e) = self.frames.get_mut(&frame_key) {
                    self.stats.cached.push(id.to_owned());
                    self.stats.nodes.push(NodeCookDiagnostic::new(id, true, t0.elapsed().as_secs_f64()*1000.0, &e.geo));
                    return if output == 0 { Ok(e.geo.clone()) } else { Err(Error::new(format!("node '{id}' has no output {output}"))) };
                }
            }
        }
        if let Some(e) = self.cache.get_mut(&key).filter(|_| def.node_type != "usd_export" && def.node_type != "cache") {
            e.used = self.clock;
            self.stats.cached.push(id.to_string());
            self.stats.nodes.push(NodeCookDiagnostic::new(id, true, t0.elapsed().as_secs_f64()*1000.0, &e.geo));
            return e.outputs.get(output).cloned().ok_or_else(|| Error::new(format!("node '{id}' has no output {output}")));
        }
        if self.cancel.is_some_and(|c| c()) {
            return Err(Error::new(SUPERSEDED));
        }
        let spec = nodes::get(&def.node_type).expect("validated by key_of");
        if def.node_type == "loop_end" {
            let t0 = Instant::now();
            let (plan, graph, budget, resolved) = (self.plan, self.graph, self.budget.clone(), self.resolved);
            let (geo,times) = plan.cook_end(graph, resolved, id, budget, &mut |src, output| self.get_output(src, output))?;
            let geo = Arc::new(geo);
            for (node,ms,points,primitives) in times {
                self.stats.cooked.push(node.clone()); self.stats.node_ms.push((node.clone(),ms));
                self.stats.nodes.push(NodeCookDiagnostic { node_id: node, cache_hit: false, cook_ms: ms, point_count: points, primitive_count: primitives });
            }
            let bytes = geometry_bytes(&geo);
            if let Some(old) = self.cache.insert(key, Entry { node: id.into(), geo: geo.clone(), outputs: vec![geo.clone()], side: None, bytes, used: self.clock }) { *self.bytes -= old.bytes; }
            *self.bytes += bytes;
            self.stats.cooked.push(id.into());
            let ms = t0.elapsed().as_secs_f64()*1000.0;
            self.stats.node_ms.push((id.into(), ms));
            self.stats.nodes.push(NodeCookDiagnostic::new(id, false, ms, &geo));
            return if output == 0 { Ok(geo) } else { Err(Error::new(format!("node '{id}' has no output {output}"))) };
        }
        let mut inputs: Vec<Vec<Arc<Geometry>>> = Vec::with_capacity(spec.inputs.len());
        for port in spec.inputs {
            let refs = def.inputs.iter().find(|(p, _)| p == port.name).map(|(_, r)| r.as_slice()).unwrap_or(&[]);
            let mut got = Vec::with_capacity(refs.len());
            for (index, r) in refs.iter().enumerate() {
                got.push(self.get_output(r, def.output_for(port.name, index, r).output)?);
            }
            inputs.push(got);
        }
        let t0 = Instant::now();
        let external = if def.bypass { None } else {
            let p=Params::resolve_node(&def.node_type,spec.params,if def.node_type=="subnet" {None} else {self.resolved.params.get(id).or(def.params.as_ref())}).map_err(|e|e.in_node(id,&def.node_type))?;
            if def.node_type == "object_input" { Some(nodes::blender_input::cook_binding(id,&p,self.external)) }
            else if def.node_type == "blender_object" { Some(nodes::blender_input::cook_object(id,&p,self.external)) }
            else { nodes::blender_input::cook_external(&def.node_type,&p,self.external) }
        };
        // A bypassed node hands its input on as is: the same shared geometry, no copy.
        let passed = if def.bypass { inputs.first().and_then(|i| i.first()).cloned() }
            else if def.node_type == "cache" {
                let p = Params::resolve(spec.params, self.resolved.params.get(id).or(def.params.as_ref())).map_err(|e| e.in_node(id, &def.node_type))?;
                p.e("mode", &["auto", "off"]).map_err(|e| e.in_node(id, &def.node_type))?;
                Some(inputs[0].first().ok_or_else(|| Error::new("geo is not connected").in_node(id, &def.node_type))?.clone())
            } else { None };
        let outputs: Vec<Arc<Geometry>> = if let Some(g) = passed {
            let mut outputs = vec![g];
            outputs.resize_with(spec.outputs.len(), || Arc::new(Geometry::default()));
            outputs
        } else { match external {
            // Waiting for external data: empty geometry, so downstream nodes still cook (see CookStats::waiting).
            Some(result)=>match result.and_then(|g|g.validate().map(|_|g)) {
                Err(e) if e.waiting_text().is_some() => vec![Arc::new(Geometry::default())],
                other => vec![Arc::new(other.map_err(|e|e.in_node(id,&def.node_type))?)],
            },
            None=>run_node_outputs(id, def, spec, &inputs, self.resolved.params.get(id))?.0.into_iter().map(Arc::new).collect(),
        } };
        let geo = outputs.first().cloned().ok_or_else(|| Error::new(format!("node '{id}' produced no outputs")))?;
        let sidecar = if def.bypass {
            None
        } else if let Some(cook_sidecar) = spec.cook_sidecar {
            let params = Params::resolve(spec.params, self.resolved.params.get(id).or(def.params.as_ref())).map_err(|e| e.in_node(id, &def.node_type))?;
            cook_sidecar(&inputs, &params).map_err(|e| e.in_node(id, &def.node_type))?
        } else {
            None
        };
        self.stats.cooked.push(id.to_string());
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        self.stats.node_ms.push((id.to_string(), ms));
        self.stats.nodes.push(NodeCookDiagnostic::new(id, false, ms, &geo));
        if def.node_type == "cache" && !def.bypass {
            if !cache_off {
                let p = Params::resolve(spec.params, self.resolved.params.get(id).or(def.params.as_ref()))?;
                let max_frames = if p.b("cache_any_frame")? { p.i("max_frames")? as usize } else { 1 };
                let frame_key = (id.to_owned(), self.frame, input_hash(def, self.keys));
                let bytes = geometry_bytes(&geo);
                if let Some(old) = self.frames.insert(frame_key, FrameEntry { geo: geo.clone(), bytes, used: self.clock }) { *self.frame_bytes -= old.bytes; }
                *self.frame_bytes += bytes;
                let mut order: Vec<_> = self.frames.iter().filter(|(k, _)| k.0 == id).map(|(k, e)| (e.used, k.clone())).collect();
                order.sort_unstable();
                for (_, key) in order.into_iter().take(self.frames.keys().filter(|k| k.0 == id).count().saturating_sub(max_frames)) {
                    if let Some(old) = self.frames.remove(&key) { *self.frame_bytes -= old.bytes; }
                }
            }
            return outputs.get(output).cloned().ok_or_else(|| Error::new(format!("node '{id}' has no output {output}")));
        }
        let bytes = outputs.iter().map(|g| geometry_bytes(g)).sum::<usize>() + sidecar.as_ref().map_or(0, |(_, bytes)| *bytes);
        let side = sidecar.map(|(payload, _)| payload);
        let selected = outputs.get(output).cloned().ok_or_else(|| Error::new(format!("node '{id}' has no output {output}")))?;
        if let Some(old) = self.cache.insert(key, Entry { node: id.to_string(), geo: geo.clone(), outputs, side, bytes, used: self.clock }) {
            *self.bytes -= old.bytes;
        }
        *self.bytes += bytes;
        Ok(selected)
    }

    fn purge_frames(&mut self, id: &str, frame: Option<u64>) {
        self.frames.retain(|(node, f, _), e| {
            let keep = node != id || frame.is_some_and(|target| target != *f);
            if !keep { *self.frame_bytes -= e.bytes; }
            keep
        });
    }
}

/// Bytes held by a geometry's arrays (topology and attribute tables; the detail map is small and not counted).
pub fn geometry_bytes(g: &Geometry) -> usize {
    let t = &g.topology;
    let mut b = t.vertex_point.len() * 4 + t.prim_offsets.len() * 4 + t.closed.as_ref().map_or(0, |c| c.len());
    for table in [&g.point, &g.vertex, &g.prim] {
        for a in table.values() {
            b += match &a.data {
                AttrData::F32(v) => v.len() * 4,
                AttrData::I32(v) => v.len() * 4,
            };
        }
    }
    b
}
