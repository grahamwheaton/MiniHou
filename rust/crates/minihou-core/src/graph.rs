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
        let display = obj.get("display").and_then(Value::as_str).map(str::to_string);
        Ok(Graph { nodes, display })
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

    /// Cook `target` (default: the main output).
    pub fn cook(&self, target: Option<&str>) -> Result<Arc<Geometry>> {
        let target = match target {
            Some(t) => t.to_string(),
            None => self.main_output()?,
        };
        let mut ev = Cook { graph: self, done: HashMap::new(), active: HashSet::new() };
        ev.node(&target)
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
            .and_then(|p| (spec.cook)(inputs, &p))
            .and_then(|g| g.validate().map(|_| g))
    };
    result.map_err(|e| e.in_node(id, &def.node_type))
}

struct Cook<'a> {
    graph: &'a Graph,
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
