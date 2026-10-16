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
            let params = nd.get("params").and_then(Value::as_object).cloned();
            nodes.push((id.clone(), NodeDef { node_type, inputs, params, bypass }));
        }
        let display = obj.get("display").and_then(|v| v.as_str().or_else(|| v.as_array()?.iter().find_map(Value::as_str))).map(str::to_string);
        Ok(Graph { nodes, display, source: v.clone() })
    }

    /// Current engine edits over the original document, preserving layout and metadata.
    pub fn to_value(&self) -> Value {
        let mut value = self.source.clone();
        let nodes = value["nodes"].as_object_mut().expect("validated graph");
        nodes.retain(|id, _| self.nodes.iter().any(|(n, _)| n == id));
        for (id, def) in &self.nodes {
            let nd = nodes.entry(id.clone()).or_insert_with(|| serde_json::json!({}));
            nd["type"] = Value::String(def.node_type.clone());
            if nd.get("params").and_then(Value::as_object) != def.params.as_ref() {
                if let Some(p) = &def.params { nd["params"] = Value::Object(p.clone()); }
                else { nd.as_object_mut().unwrap().remove("params"); }
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
        let target = match target {
            Some(t) => t.to_string(),
            None => self.main_output()?,
        };
        let plan = nodes::r#loop::Plan::analyze(self, &target)?;
        nodes::rbd_engine::check_static_inputs(self, &target)?;
        let mut ev = Cook { graph: self, plan: &plan, budget: nodes::r#loop::budget(), done: HashMap::new(), active: HashSet::new() };
        nodes::usd_nodes::with_context(self, &Default::default(), || ev.node(&target))
    }
}

/// Run one node on inputs that are already cooked: a bypassed node passes its first input through (nothing connected: empty
/// geometry), any other resolves its parameters, cooks and validates. Errors name the node. Shared by [`Graph::cook`] and
/// [`crate::session::Session`] so the two can not drift apart.
pub(crate) fn run_node(id: &str, def: &NodeDef, spec: &nodes::NodeSpec, inputs: &nodes::Inputs) -> Result<Geometry> {
    let result = if def.bypass {
        Ok(inputs.first().and_then(|i| i.first()).map(|g| (**g).clone()).unwrap_or_else(|| Geometry::points_only(Vec::new()).expect("empty geometry")))
    } else {
        Params::resolve(spec.params, def.params.as_ref())
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
            let (plan, graph, budget) = (self.plan, self.graph, self.budget.clone());
            let geo = Arc::new(plan.cook_end(graph, id, budget, &mut |src| self.node(src))?.0);
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
        let geo = Arc::new(run_node(id, def, spec, &inputs)?);
        self.active.remove(id);
        self.done.insert(id.to_string(), geo.clone());
        Ok(geo)
    }
}
