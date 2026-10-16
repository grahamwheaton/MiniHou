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
use crate::graph::{run_node, Graph};
use crate::{lod::{ClusterLod, BuildError}, params::Params, preview};
use crate::nodes;

/// What the last cook did: which nodes ran, which were served from the cache, and how long each cook took (ms).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CookStats {
    pub cooked: Vec<String>,
    pub cached: Vec<String>,
    pub node_ms: Vec<(String, f64)>,
}

struct Entry {
    node: String,
    geo: Arc<Geometry>,
    side: Option<Arc<dyn Any + Send + Sync>>,
    bytes: usize,
    used: u64,
}

pub struct Session {
    graph: Graph,
    external: nodes::blender_input::ExternalInputs,
    external_versions: HashMap<String,u64>,
    external_revision: u64,
    cache: HashMap<u64, Entry>,
    bytes: usize,
    budget: Option<usize>,
    keep_versions: usize,
    clock: u64,
    last: CookStats,
    /// (frame, fps) cooks run at.
    time: (f64, f64),
}

impl Session {
    pub fn new(graph: Graph) -> Session {
        Session { graph, external: Default::default(), external_versions: Default::default(), external_revision:0, cache: HashMap::new(), bytes: 0, budget: None, keep_versions: 2, clock: 0, last: CookStats::default(), time: (1.0, crate::timectx::DEFAULT_FPS) }
    }

    pub fn from_json(text: &str) -> Result<Session> {
        Ok(Session::new(Graph::from_json(text)?))
    }

    /// Keep live inputs across graph edits, undo and timeline recooks.
    pub fn replace_graph(&mut self, graph: Graph) { self.graph=graph; self.clear(); }
    pub fn set_external(&mut self, name: String, input: nodes::blender_input::ExternalInput) {
        self.external_revision+=1;
        self.external_versions.insert(name.clone(),self.external_revision);
        self.external.insert(name,input);
    }
    pub fn graph(&self) -> &Graph {
        &self.graph
    }

    /// Set (or add) one parameter of a node. Unknown nodes and parameter names are errors here; a value the node refuses is
    /// an error at cook time, as in a graph file.
    pub fn set_param(&mut self, node: &str, name: &str, value: Value) -> Result<()> {
        let def = self.graph.nodes.iter_mut().find(|(n, _)| n == node).map(|(_, d)| d)
            .ok_or_else(|| Error::new(format!("no such node '{node}'")))?;
        let spec = nodes::get(&def.node_type).ok_or_else(|| {
            Error::new(format!("node '{node}': node type '{}' is not in the Rust backend yet", def.node_type))
        })?;
        if !spec.params.iter().any(|p| p.name == name) {
            return Err(Error::new(format!("node '{node}' ({}): unknown parameter {name:?}", def.node_type)));
        }
        def.params.get_or_insert_with(Default::default).insert(name.to_string(), value);
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

    /// Cook `target` (default: the graph's main output), running only the nodes the cache does not already hold.
    pub fn cook(&mut self, target: Option<&str>) -> Result<Arc<Geometry>> {
        let (frame, fps) = self.time;
        crate::timectx::at(Some(frame), Some(fps), || self.cook_now(target))
    }

    fn cook_now(&mut self, target: Option<&str>) -> Result<Arc<Geometry>> {
        let target = match target {
            Some(t) => t.to_string(),
            None => self.graph.main_output()?,
        };
        let mut keys: HashMap<String, u64> = HashMap::new();
        self.last = CookStats::default();
        let plan = nodes::r#loop::Plan::analyze(&self.graph, &target)?;
        nodes::rbd_engine::check_static_inputs(&self.graph, &target)?;
        key_of(&self.graph, &self.external_versions, &target, &mut keys, &mut HashSet::new())?;
        self.clock += 1;
        let mut stats = CookStats::default();
        let mut run = Run { graph: &self.graph, plan: &plan, budget: nodes::r#loop::budget(), external: &self.external, cache: &mut self.cache, bytes: &mut self.bytes, clock: self.clock, keys: &keys, stats: &mut stats };
        let result = nodes::usd_nodes::with_context(&self.graph, &self.external, || run.get(&target));
        self.last = stats;
        self.evict();
        result
    }

    pub fn last_stats(&self) -> &CookStats {
        &self.last
    }

    pub fn cache_bytes(&self) -> usize {
        self.bytes
    }

    pub fn cache_len(&self) -> usize {
        self.cache.len()
    }

    /// Typed side result for a node's current content key. It is evicted with its geometry.
    pub fn side_payload<T: Any + Send + Sync>(&self, node: &str) -> Option<Arc<T>> {
        let key = crate::timectx::at(Some(self.time.0), Some(self.time.1), || key_of(&self.graph, &self.external_versions, node, &mut HashMap::new(), &mut HashSet::new())).ok()?;
        self.cache.get(&key)?.side.as_ref()?.clone().downcast::<T>().ok()
    }
    pub fn cached_geometry(&self, node: &str) -> Option<Arc<Geometry>> {
        let key = crate::timectx::at(Some(self.time.0), Some(self.time.1), || key_of(&self.graph, &self.external_versions, node, &mut HashMap::new(), &mut HashSet::new())).ok()?;
        Some(self.cache.get(&key)?.geo.clone())
    }

    /// `None` or `Some(0)`: unlimited.
    pub fn set_budget_bytes(&mut self, budget: Option<usize>) {
        self.budget = budget.filter(|b| *b > 0);
        self.evict();
    }

    pub fn clear(&mut self) {
        self.cache.clear();
        self.bytes = 0;
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
fn key_of(graph: &Graph, external_versions: &HashMap<String,u64>, id: &str, keys: &mut HashMap<String, u64>, active: &mut HashSet<String>) -> Result<u64> {
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
    for (port, _) in &def.inputs {
        if !spec.inputs.iter().any(|i| i.name == port) {
            return Err(Error::new(format!("node '{id}': '{}' has no input '{port}'", def.node_type)));
        }
    }
    let mut h = DefaultHasher::new();
    def.node_type.hash(&mut h);
    if def.node_type == "usd_export" {
        nodes::usd_nodes::exports_suppressed().hash(&mut h);
        let p = Params::resolve(spec.params, def.params.as_ref())?;
        if let Ok(path) = nodes::usd_nodes::export_target(&p) {
            if let Ok(meta) = std::fs::metadata(path) { meta.len().hash(&mut h); meta.modified().ok().hash(&mut h); }
        }
    }
    if def.node_type == "usd_import" || def.node_type == "image_file" {
        let p = Params::resolve(spec.params, def.params.as_ref())?;
        let path = crate::usd_io::resolve_path(&p.s("file")?);
        path.hash(&mut h);
        if let Ok(meta) = std::fs::metadata(path) { meta.len().hash(&mut h); meta.modified().ok().hash(&mut h); }
    }
    if def.node_type == "copy_to_points" {
        nodes::copy_limit_bytes().hash(&mut h);
    }
    let binding=match def.node_type.as_str() {
        "blender_object" => Some(format!("object:{}",def.params.as_ref().and_then(|p|p.get("object")).and_then(Value::as_str).unwrap_or(""))),
        "blender_image" => Some(format!("image:{}",def.params.as_ref().and_then(|p|p.get("image")).and_then(Value::as_str).unwrap_or(""))),
        "object_input" => Some(nodes::blender_input::binding_key(id, &Params::resolve(spec.params,def.params.as_ref())?)?),
        "modifier_input" => Some("modifier_input".into()), _ => None,
    };
    if let Some(binding)=binding { external_versions.get(&binding).hash(&mut h); }
    def.bypass.hash(&mut h);
    // time-dependent nodes (Python's key_extra = timectx.key()) recook when the frame changes
    if nodes::is_time_dependent(&def.node_type, def.params.as_ref()) {
        crate::timectx::key().hash(&mut h);
    }
    match &def.params {
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
        for r in refs {
            key_of(graph, external_versions, r, keys, active)?.hash(&mut h);
        }
    }
    active.remove(id);
    let key = h.finish();
    keys.insert(id.to_string(), key);
    Ok(key)
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

struct Run<'a> {
    graph: &'a Graph,
    plan: &'a nodes::r#loop::Plan,
    budget: nodes::r#loop::Budget,
    external: &'a nodes::blender_input::ExternalInputs,
    cache: &'a mut HashMap<u64, Entry>,
    bytes: &'a mut usize,
    clock: u64,
    keys: &'a HashMap<String, u64>,
    stats: &'a mut CookStats,
}

impl Run<'_> {
    fn get(&mut self, id: &str) -> Result<Arc<Geometry>> {
        let key = self.keys[id];
        if let Some(e) = self.cache.get_mut(&key).filter(|_| self.graph.node(id).is_none_or(|n| n.node_type != "usd_export")) {
            e.used = self.clock;
            self.stats.cached.push(id.to_string());
            return Ok(e.geo.clone());
        }
        let def = self.graph.node(id).expect("validated by key_of");
        let spec = nodes::get(&def.node_type).expect("validated by key_of");
        if def.node_type == "loop_end" {
            let t0 = Instant::now();
            let (plan, graph, budget) = (self.plan, self.graph, self.budget.clone());
            let (geo,times) = plan.cook_end(graph, id, budget, &mut |src| self.get(src))?;
            let geo = Arc::new(geo);
            for (node,ms) in times { self.stats.cooked.push(node.clone()); self.stats.node_ms.push((node,ms)); }
            let bytes = geometry_bytes(&geo);
            if let Some(old) = self.cache.insert(key, Entry { node: id.into(), geo: geo.clone(), side: None, bytes, used: self.clock }) { *self.bytes -= old.bytes; }
            *self.bytes += bytes;
            self.stats.cooked.push(id.into());
            self.stats.node_ms.push((id.into(), t0.elapsed().as_secs_f64()*1000.0));
            return Ok(geo);
        }
        let mut inputs: Vec<Vec<Arc<Geometry>>> = Vec::with_capacity(spec.inputs.len());
        for port in spec.inputs {
            let refs = def.inputs.iter().find(|(p, _)| p == port.name).map(|(_, r)| r.as_slice()).unwrap_or(&[]);
            let mut got = Vec::with_capacity(refs.len());
            for r in refs {
                got.push(self.get(r)?);
            }
            inputs.push(got);
        }
        let t0 = Instant::now();
        let external = if def.bypass { None } else {
            let p=Params::resolve(spec.params,def.params.as_ref()).map_err(|e|e.in_node(id,&def.node_type))?;
            if def.node_type == "object_input" { Some(nodes::blender_input::cook_binding(id,&p,self.external)) }
            else { nodes::blender_input::cook_external(&def.node_type,&p,self.external) }
        };
        let geo = Arc::new(match external {
            Some(result)=>result.and_then(|g|g.validate().map(|_|g)).map_err(|e|e.in_node(id,&def.node_type))?,
            None=>run_node(id, def, spec, &inputs)?,
        });
        let mut side: Option<Arc<dyn Any + Send + Sync>> = None;
        let mut side_bytes = 0;
        if def.node_type == "lod_bake" && !def.bypass {
            let params = Params::resolve(spec.params, def.params.as_ref()).map_err(|e| e.in_node(id, &def.node_type))?;
            let cluster_size = params.i("cluster_size").map_err(|e| e.in_node(id, &def.node_type))?;
            let min_triangles = params.i("min_triangles").map_err(|e| e.in_node(id, &def.node_type))?;
            let error_px = params.f("error_px").map_err(|e| e.in_node(id, &def.node_type))?;
            if !(16..=128).contains(&cluster_size) || min_triangles < 0 || !(error_px as f32).is_finite() || error_px <= 0.0 {
                return Err(Error::new("cluster_size must be 16..128, min_triangles must be nonnegative, and error_px must be positive and finite").in_node(id, &def.node_type));
            }
            if params.b("enabled").map_err(|e| e.in_node(id, &def.node_type))? {
                let g = inputs[0].first().expect("validated by pass-through");
                let (tris, _) = preview::fan_triangles(&g.topology.vertex_point, &g.topology.prim_offsets, g.topology.closed.as_deref());
                if tris.len() / 3 >= min_triangles as usize && !tris.is_empty() {
                    let positions: Vec<[f32; 3]> = g.p().as_chunks::<3>().0.to_vec();
                    let triangles = tris.as_chunks::<3>().0.to_vec();
                    let mut lod = ClusterLod::build_with_cluster_size(positions, triangles, cluster_size as usize)
                        .map_err(|e| Error::new(match e { BuildError::InvalidIndices => "invalid triangle indices", BuildError::NonFinitePosition => "non-finite position", BuildError::EmptyMesh => "empty mesh" }).in_node(id, &def.node_type))?;
                    lod.error_px = error_px as f32;
                    side_bytes = lod.bytes();
                    side = Some(Arc::new(lod));
                }
            }
        }
        self.stats.cooked.push(id.to_string());
        self.stats.node_ms.push((id.to_string(), t0.elapsed().as_secs_f64() * 1000.0));
        let bytes = geometry_bytes(&geo) + side_bytes;
        if let Some(old) = self.cache.insert(key, Entry { node: id.to_string(), geo: geo.clone(), side, bytes, used: self.clock }) {
            *self.bytes -= old.bytes;
        }
        *self.bytes += bytes;
        Ok(geo)
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
