//! Graph JSON (the format `minihou.graph.Graph.from_dict` reads) and a pull evaluator: cook a node by cooking what it reads
//! first, each node once per cook. Only the node types in [`crate::nodes`] are known; anything else is a clear error.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde_json::{Map, Value};

use crate::error::{Error, Result};
use crate::geometry::Geometry;
use crate::nodes;
use crate::params::Params;

pub const GRAPH_VERSION: i64 = 1;

pub struct NodeDef {
    pub node_type: String,
    /// Port name -> the node ids connected to it (one, or several for a multi input).
    pub inputs: Vec<(String, Vec<String>)>,
    pub params: Option<Map<String, Value>>,
    pub channels: Option<Map<String, Value>>,
    pub locked_params: Option<Vec<String>>,
    pub bypass: bool,
}

pub struct Graph {
    /// In file order (the first `output` node is the main output, as in Python).
    pub nodes: Vec<(String, NodeDef)>,
    pub display: Option<String>,
    /// Original UI/extension fields survive graph embedding in interchange files.
    source: Value,
}

impl Graph {
    pub fn from_json(text: &str) -> Result<Graph> {
        let v: Value = serde_json::from_str(text).map_err(|e| Error::new(format!("invalid graph JSON: {e}")))?;
        Graph::from_value(&v)
    }

    pub fn from_value(v: &Value) -> Result<Graph> {
        let bad = || Error::new("graph JSON must be an object with a 'nodes' object");
        let obj = v.as_object().ok_or_else(bad)?;
        let nodes_v = obj.get("nodes").and_then(Value::as_object).ok_or_else(bad)?;
        if let Some(ver) = obj.get("version") {
            if ver.as_i64() != Some(GRAPH_VERSION) {
                return Err(Error::new(format!("unsupported graph version {ver} (this build reads {GRAPH_VERSION})")));
            }
        }
        let mut nodes = Vec::with_capacity(nodes_v.len());
        for (id, nd) in nodes_v {
            let nd = nd.as_object().ok_or_else(|| Error::new(format!("node '{id}': expected an object with a 'type'")))?;
            let node_type = nd.get("type").and_then(Value::as_str)
                .ok_or_else(|| Error::new(format!("node '{id}': expected an object with a 'type'")))?.to_string();
            if nd.contains_key("locked_geometry") {
                return Err(Error::new(format!("node '{id}': locked geometry is not supported by the Rust backend yet")));
            }
            let mut inputs = Vec::new();
            if let Some(ins) = nd.get("inputs").and_then(Value::as_object) {
                for (port, r) in ins {
                    if matches!(node_type.as_str(), "loop_begin" | "loop_end") && r.is_array() {
                        return Err(Error::new(format!("node '{id}': {port} input must be a single node")));
                    }
                    let refs = match r {
                        Value::String(s) => vec![s.clone()],
                        Value::Array(a) => a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect(),
                        Value::Null => Vec::new(),
                        _ => return Err(Error::new(format!("node '{id}': input '{port}' must name a node"))),
                    };
                    inputs.push((port.clone(), refs));
                }
            }
            let bypass = nd.get("flags").and_then(|f| f.get("bypass")).and_then(Value::as_bool).unwrap_or(false);
            let mut params = nd.get("params").and_then(Value::as_object).cloned();
            if let Some(params) = &mut params {
                for (param, value) in params {
                    if let Some(link) = value.as_object().and_then(|o| o.get("modifier_input")) {
                        let (source, input) = if let Some(s) = link.as_str() {
                            (None, s)
                        } else {
                            (link["node"].as_str(), link["id"].as_str().ok_or_else(|| Error::new(format!("node '{id}' parameter '{param}': modifier input link needs an id")))?)
                        };
                        let found = nodes_v.iter().filter(|(name, n)| n["type"] == "modifier_input" && source.is_none_or(|s| s == name.as_str()))
                            .flat_map(|(_, n)| n["modifier_inputs"].as_array().into_iter().flatten().map(move |entry| (n, entry)))
                            .find(|(_, entry)| entry["id"] == input || entry["name"] == input)
                            .ok_or_else(|| Error::new(format!("node '{id}' parameter '{param}': modifier input {input:?} is missing")))?;
                        let key = found.1["id"].as_str().unwrap_or(input);
                        *value = found.0["input_values"].get(key).unwrap_or(&found.1["default"]).clone();
                    }
                }
            }
            let channels = nd.get("channels").map(|v| v.as_object().cloned().ok_or_else(|| Error::new(format!("node '{id}': channels must be an object")))).transpose()?;
            if channels.as_ref().is_some_and(|m| m.values().any(|v| !v.is_string())) {return Err(Error::new(format!("node '{id}': channels must contain expression strings")));}
            let locked_params = nd.get("locked_params").map(|v| v.as_array().and_then(|a| a.iter().map(|x| x.as_str().map(str::to_owned)).collect()).ok_or_else(|| Error::new(format!("node '{id}': locked_params must be an array of strings")))).transpose()?;
            nodes.push((id.clone(), NodeDef { node_type, inputs, params, channels, locked_params, bypass }));
        }
        let display = obj.get("display").and_then(|v| v.as_str().or_else(|| v.as_array()?.iter().find_map(Value::as_str))).map(str::to_string);
        Ok(Graph { nodes, display, source: v.clone() })
    }

    /// Current engine edits over the original document, preserving layout and metadata.
    pub fn to_value(&self) -> Value {
        let mut value = self.source.clone();
        let baseline = Graph::from_value(&self.source).ok();
        let nodes = value["nodes"].as_object_mut().expect("validated graph");
        nodes.retain(|id, _| self.nodes.iter().any(|(n, _)| n == id));
        for (id, def) in &self.nodes {
            // The inner graph is stored on the subnet node in the original document.
            if id.contains('/') || def.node_type == "subnet" { continue; }
            let nd = nodes.entry(id.clone()).or_insert_with(|| serde_json::json!({}));
            nd["type"] = Value::String(def.node_type.clone());
            let mut params = def.params.clone();
            if let (Some(params), Some(original), Some(baseline)) = (&mut params,
                nd.get("params").and_then(Value::as_object), baseline.as_ref().and_then(|g| g.node(id)).and_then(|d| d.params.as_ref())) {
                for (name, link) in original {
                    if link.get("modifier_input").is_some() && params.get(name) == baseline.get(name) {
                        params.insert(name.clone(), link.clone());
                    }
                }
            }
            if nd.get("params").and_then(Value::as_object) != params.as_ref() {
                if let Some(p) = &params { nd["params"] = Value::Object(p.clone()); }
                else { nd.as_object_mut().unwrap().remove("params"); }
            }
            if nd.get("channels").and_then(Value::as_object) != def.channels.as_ref() {
                if let Some(c)=&def.channels {nd["channels"]=Value::Object(c.clone());}else{nd.as_object_mut().unwrap().remove("channels");}
            }
            if nd.get("locked_params").and_then(Value::as_array).map(|a|a.iter().filter_map(Value::as_str).collect::<Vec<_>>()) != def.locked_params.as_ref().map(|a|a.iter().map(String::as_str).collect::<Vec<_>>()) {
                if let Some(l)=&def.locked_params {nd["locked_params"]=serde_json::json!(l);}else{nd.as_object_mut().unwrap().remove("locked_params");}
            }
            if nd["bypass"].as_bool().unwrap_or(false) != def.bypass { nd["bypass"] = Value::Bool(def.bypass); }
            let old_inputs = nd.get("inputs").and_then(Value::as_object);
            let same = old_inputs.map_or(def.inputs.is_empty(), |m| m.len() == def.inputs.len() && def.inputs.iter().all(|(port, refs)| {
                let Some(old) = m.get(port) else { return false; };
                if let Some(s) = old.as_str() { refs.len() == 1 && refs[0] == s }
                else { old.as_array().is_some_and(|a| a.len() == refs.len() && a.iter().zip(refs).all(|(v,r)| v.as_str() == Some(r.as_str()))) }
            }));
            if !same { nd["inputs"] = Value::Object(def.inputs.iter().map(|(port,refs)|(port.clone(),if refs.len()==1 { Value::String(refs[0].clone()) } else { serde_json::json!(refs) })).collect()); }
        }
        if value["display"].as_str() != self.display.as_deref() {
            if let Some(display) = &self.display { value["display"] = Value::String(display.clone()); }
            else { value.as_object_mut().unwrap().remove("display"); }
        }
        value
    }

    pub(crate) fn node(&self, id: &str) -> Option<&NodeDef> {
        self.nodes.iter().find(|(n, _)| n == id).map(|(_, d)| d)
    }
    pub fn node_channels(&self,id:&str)->Option<&Map<String,Value>> {self.node(id)?.channels.as_ref()}
    pub fn node_locks(&self,id:&str)->Option<&Vec<String>> {self.node(id)?.locked_params.as_ref()}

    /// The display node, else the first `output` node without its own Blender object, else the first `output`.
    pub fn main_output(&self) -> Result<String> {
        if let Some(d) = &self.display {
            return Ok(d.clone());
        }
        let outs: Vec<&(String, NodeDef)> = self.nodes.iter().filter(|(_, d)| d.node_type == "output").collect();
        let own_object = |d: &NodeDef| d.params.as_ref().and_then(|p| p.get("object")).and_then(Value::as_str).is_some_and(|s| !s.is_empty());
        outs.iter().find(|(_, d)| !own_object(d)).or(outs.first())
            .map(|(id, _)| id.clone())
            .ok_or_else(|| Error::new("graph has no display node and no 'output' node"))
    }

    /// Python Graph.extra_outputs: first unique trimmed name, excluding the main output.
    pub fn extra_outputs(&self) -> Vec<(String,String)> {
        let main=self.main_output().ok();
        let mut seen=HashSet::new();
        self.nodes.iter().filter(|(id,d)|d.node_type=="output" && Some(id)!=main.as_ref())
            .filter_map(|(id,d)| {
                let name=d.params.as_ref()?.get("object")?.as_str()?.trim().chars().take(63).collect::<String>();
                if name.is_empty() || !seen.insert(name.clone()) {None} else {Some((name,id.clone()))}
            }).collect()
    }

    /// Cook `target` (default: the main output).
    pub fn cook(&self, target: Option<&str>) -> Result<Arc<Geometry>> {
        if !self.nodes.iter().any(|(_, def)| def.node_type == "subnet") {
            return self.cook_expanded(target);
        }
        let mut value = self.to_value();
        if let Some(expanded) = expand_subnets(value["nodes"].as_object().expect("validated graph"))? {
            value["nodes"] = Value::Object(expanded);
            return Graph::from_value(&value)?.cook_expanded(target);
        }
        self.cook_expanded(target)
    }

    fn cook_expanded(&self, target: Option<&str>) -> Result<Arc<Geometry>> {
        let target = match target {
            Some(t) => t.to_string(),
            None => self.main_output()?,
        };
        let plan = nodes::r#loop::Plan::analyze(self, &target)?;
        nodes::rbd_engine::check_static_inputs(self, &target)?;
        let mut ev = Cook { graph: self, plan: &plan, budget: nodes::r#loop::budget(), done: HashMap::new(), active: HashSet::new(), resolved: Arc::new(crate::channels::resolve_graph(self)) };
        nodes::usd_nodes::with_context(self, &Default::default(), || ev.node(&target))
    }
}

/// Run one node on inputs that are already cooked: a bypassed node passes its first input through (nothing connected: empty
/// geometry), any other resolves its parameters, cooks and validates. Errors name the node. Shared by [`Graph::cook`] and
/// [`crate::session::Session`] so the two can not drift apart.
pub(crate) fn run_node(id: &str, def: &NodeDef, spec: &nodes::NodeSpec, inputs: &nodes::Inputs, resolved: Option<&Map<String,Value>>) -> Result<Geometry> {
    let result = if def.bypass {
        Ok(inputs.first().and_then(|i| i.first()).map(|g| (**g).clone()).unwrap_or_else(|| Geometry::points_only(Vec::new()).expect("empty geometry")))
    } else {
        Params::resolve(spec.params, resolved.or(def.params.as_ref()))
            .and_then(|p| if def.node_type == "usd_export" { nodes::usd_nodes::cook_named(inputs, &p, id) } else { (spec.cook)(inputs, &p) })
            .and_then(|g| g.validate().map(|_| g))
    };
    result.map_err(|e| e.in_node(id, &def.node_type))
}

struct Cook<'a> {
    graph: &'a Graph,
    plan: &'a nodes::r#loop::Plan,
    budget: nodes::r#loop::Budget,
    done: HashMap<String, Arc<Geometry>>,
    active: HashSet<String>,
    resolved: Arc<crate::channels::ResolvedChannels>,
}

impl Cook<'_> {
    fn node(&mut self, id: &str) -> Result<Arc<Geometry>> {
        if let Some(g) = self.done.get(id) {
            return Ok(g.clone());
        }
        let def = self.graph.node(id).ok_or_else(|| Error::new(format!("no such node '{id}'")))?;
        if !self.active.insert(id.to_string()) {
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
        if def.node_type == "loop_end" {
            let (plan, graph, budget, resolved) = (self.plan, self.graph, self.budget.clone(), self.resolved.clone());
            let geo = Arc::new(plan.cook_end(graph, &resolved, id, budget, &mut |src| self.node(src))?.0);
            self.active.remove(id);
            self.done.insert(id.into(), geo.clone());
            return Ok(geo);
        }
        // cook what this node reads (their errors pass through unchanged: they name their own node)
        let mut inputs: Vec<Vec<Arc<Geometry>>> = Vec::with_capacity(spec.inputs.len());
        for port in spec.inputs {
            let refs = def.inputs.iter().find(|(p, _)| p == port.name).map(|(_, r)| r.as_slice()).unwrap_or(&[]);
            if !port.multi && refs.len() > 1 {
                return Err(Error::new(format!("node '{id}': input '{}' takes one connection", port.name)));
            }
            let mut got = Vec::with_capacity(refs.len());
            for r in refs {
                got.push(self.node(r)?);
            }
            inputs.push(got);
        }
        let geo = Arc::new(run_node(id, def, spec, &inputs, self.resolved.params.get(id))?);
        self.active.remove(id);
        self.done.insert(id.to_string(), geo.clone());
        Ok(geo)
    }
}

/// Expand `subnet` nodes (a node of type "subnet" carrying `inner: {nodes, output}`) into prefixed `<subnet>/<inner>` nodes.
/// Inner nodes read the subnet's external inputs as "$in0", "$in1" (the order of the subnet's `geo` wires). The subnet node
/// itself stays as a pass-through of the inner output. `None` when the graph has no subnet.
pub fn expand_subnets(nodes: &Map<String, Value>) -> Result<Option<Map<String, Value>>> {
    if !nodes.values().any(|n| n["type"] == "subnet") { return Ok(None); }
    let mut out = Map::new();
    flatten_subnets(nodes, "", &[], &mut out, 0)?;
    Ok(Some(out))
}

fn flatten_subnets(nodes: &Map<String, Value>, prefix: &str, ext: &[String], out: &mut Map<String, Value>, depth: usize) -> Result<()> {
    if depth > 16 { return Err(Error::new("subnets are nested too deeply")); }
    let map = |r: &str| -> Result<String> {
        match r.strip_prefix("$in") {
            Some(k) => k.parse::<usize>().ok().and_then(|k| ext.get(k)).cloned()
                .ok_or_else(|| Error::new(format!("subnet input '{r}' is not connected"))),
            None => Ok(format!("{prefix}{r}")),
        }
    };
    for (id, nd) in nodes {
        let full = format!("{prefix}{id}");
        let mut nd = nd.clone();
        if let Some(inputs) = nd.get_mut("inputs").and_then(Value::as_object_mut) {
            for r in inputs.values_mut() {
                match r {
                    Value::String(s) => *s = map(s)?,
                    Value::Array(a) => for x in a { if let Some(s) = x.as_str() { *x = Value::String(map(s)?); } },
                    _ => {}
                }
            }
        }
        if nd["type"] == "subnet" {
            let ext2: Vec<String> = match &nd["inputs"]["geo"] {
                Value::String(s) => vec![s.clone()],
                Value::Array(a) => a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect(),
                _ => Vec::new(),
            };
            let inner = nd["inner"]["nodes"].as_object().ok_or_else(|| Error::new(format!("node '{full}': subnet needs inner.nodes")))?;
            let output = nd["inner"]["output"].as_str().filter(|o| inner.contains_key(*o))
                .ok_or_else(|| Error::new(format!("node '{full}': subnet inner.output must name an inner node")))?;
            flatten_subnets(inner, &format!("{full}/"), &ext2, out, depth + 1)?;
            out.insert(full.clone(), serde_json::json!({"type": "subnet", "inputs": {"geo": format!("{full}/{output}")}}));
        } else {
            out.insert(full, nd);
        }
    }
    Ok(())
}

#[cfg(test)]
mod subnet_graph_tests {
    use super::*;
    use serde_json::json;

    fn grouped_box() -> Value {
        json!({"version": 1, "display": "out", "nodes": {
            "subnet1": {"type": "subnet", "inner": {"nodes": {
                "box1": {"type": "box"},
                "subdivide1": {"type": "subdivide", "inputs": {"geo": "box1"},
                    "params": {"depth": 1, "algorithm": "linear"}}
            }, "output": "subdivide1"}},
            "out": {"type": "output", "inputs": {"geo": "subnet1"}}
        }})
    }

    #[test]
    fn graph_from_value_keeps_one_subnet_node() {
        let graph = Graph::from_value(&grouped_box()).unwrap();
        assert_eq!(graph.nodes.len(), 2);
        assert_eq!(graph.node("subnet1").unwrap().node_type, "subnet");
        assert!(graph.node("subnet1/box1").is_none());
        assert!(graph.node("subnet1/subdivide1").is_none());
        assert_eq!(graph.to_value(), grouped_box());
    }

    #[test]
    fn graph_cook_subnet_matches_flattened_box_subdivide() {
        let grouped = grouped_box();
        let mut flattened = grouped.clone();
        flattened["nodes"] = Value::Object(expand_subnets(grouped["nodes"].as_object().unwrap()).unwrap().unwrap());
        let actual = Graph::from_value(&grouped).unwrap().cook(None).unwrap();
        let old = Graph::from_value(&flattened).unwrap().cook_expanded(None).unwrap();
        assert_eq!(actual.p(), old.p());
        assert_eq!(actual.topology.vertex_point, old.topology.vertex_point);
        assert_eq!(actual.topology.prim_offsets, old.topology.prim_offsets);
        assert_eq!(actual.nprims(), old.nprims());
    }
}
