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
                    let parse = |r: &Value| -> Result<String> {
                        if let Some(s) = r.as_str() { return Ok(s.to_owned()); }
                        let Some(o) = r.as_object() else { return Err(Error::new(format!("node '{id}': input '{port}' must name a node"))); };
                        let name = o.get("node").and_then(Value::as_str).ok_or_else(|| Error::new(format!("node '{id}': input '{port}' endpoint needs a node")))?;
                        let index = o.get("output").and_then(Value::as_u64).ok_or_else(|| Error::new(format!("node '{id}': input '{port}' endpoint needs an output index")))?;
                        if index > 0 && nodes_v.get(name).is_none_or(|n| n["type"] != "subnet") {
                            return Err(Error::new(format!("node '{id}': output {index} of '{name}' is unavailable")));
                        }
                        Ok(name.to_owned())
                    };
                    let refs = match r {
                        Value::Null => Vec::new(),
                        Value::Array(a) => a.iter().map(parse).collect::<Result<Vec<_>>>()?,
                        _ => vec![parse(r)?],
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
            if id.contains('/') { continue; }
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
            let same = baseline.as_ref().and_then(|g| g.node(id)).is_some_and(|original| original.inputs == def.inputs)
                && old_inputs.is_some() || old_inputs.is_none() && def.inputs.is_empty();
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

    /// Promoted names are declared on a subnet and stored as ordinary outer params.
    pub fn promoted_param(&self, id: &str, name: &str) -> Option<&Value> {
        if let Some(node) = self.source["nodes"].get(id) {
            return node["promoted_params"].as_array()?.iter().find(|p| p["name"] == name);
        }
        let mut node = &self.source["nodes"];
        let mut parts = id.split('/').peekable();
        while let Some(part) = parts.next() {
            node = &node[part];
            if parts.peek().is_some() { node = &node["inner"]["nodes"]; }
        }
        node["promoted_params"].as_array()?.iter().find(|p| p["name"] == name)
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
        Params::resolve(spec.params, if def.node_type == "subnet" {None} else {resolved.or(def.params.as_ref())})
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

/// Expand subnet boundaries into temporary, single-output nodes for the evaluator.
/// Persisted graphs retain their hierarchy and output-aware endpoints.
pub fn expand_subnets(nodes: &Map<String, Value>) -> Result<Option<Map<String, Value>>> {
    if !nodes.values().any(|n| n["type"] == "subnet") { return Ok(None); }
    let mut out = Map::new();
    flatten_subnets(nodes, "", &[], &mut out, 0)?;
    Ok(Some(out))
}

fn endpoint(v: &Value) -> Result<(String, usize)> {
    if let Some(s) = v.as_str() { return Ok((s.to_owned(), 0)); }
    let name = v.get("node").and_then(Value::as_str).ok_or_else(|| Error::new("endpoint needs a node"))?;
    let index = v.get("output").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok())
        .ok_or_else(|| Error::new("endpoint needs a nonnegative output index"))?;
    Ok((name.to_owned(), index))
}

fn sockets(node: &Value, direction: &str) -> Result<Vec<(usize, String)>> {
    let Some(list) = node.get(direction) else { return Ok(Vec::new()); };
    let list = list.as_array().ok_or_else(|| Error::new(format!("subnet {direction} must be an array")))?;
    let mut seen = HashSet::new();
    list.iter().map(|v| {
        let index = v.get("index").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| Error::new(format!("subnet {direction} socket needs a nonnegative index")))?;
        if !seen.insert(index) { return Err(Error::new(format!("duplicate subnet {direction} index {index}"))); }
        let name = v.get("name").and_then(Value::as_str).unwrap_or("").to_owned();
        Ok((index, name))
    }).collect()
}
fn subnet_alias(node: &Value, source: String) -> Value {
    let mut alias=serde_json::json!({"type":"subnet","inputs":{"geo":source}});
    for field in ["params","channels","promoted_params"] {
        if let Some(value)=node.get(field) {alias[field]=value.clone();}
    }
    alias
}

fn flatten_subnets(nodes: &Map<String, Value>, prefix: &str, ext: &[Option<String>], out: &mut Map<String, Value>, depth: usize) -> Result<()> {
    if depth > 16 { return Err(Error::new("subnets are nested too deeply")); }
    let map = |r: &Value| -> Result<String> {
        let (name, index) = endpoint(r)?;
        if let Some(k) = name.strip_prefix("$in") {
            if index != 0 { return Err(Error::new(format!("subnet input '{name}' has no output {index}"))); }
            let k = k.parse::<usize>().map_err(|_| Error::new(format!("invalid subnet input '{name}'")))?;
            return Ok(ext.get(k).and_then(Clone::clone).unwrap_or_else(|| format!("{prefix}$empty{k}")));
        }
        if index != 0 {
            let node = nodes.get(&name).ok_or_else(|| Error::new(format!("no such node '{name}'")))?;
            if node["type"] != "subnet" || !sockets(node, "outputs")?.iter().any(|(i, _)| *i == index) {
                return Err(Error::new(format!("subnet '{name}' has no output {index}")));
            }
        }
        Ok(if index == 0 { format!("{prefix}{name}") } else { format!("{prefix}{name}/$out{index}") })
    };
    for (id, nd) in nodes {
        let full = format!("{prefix}{id}");
        let mut nd = nd.clone();
        if let Some(inputs) = nd.get_mut("inputs").and_then(Value::as_object_mut) {
            for r in inputs.values_mut() {
                match r {
                    Value::String(_) | Value::Object(_) => *r = Value::String(map(r)?),
                    Value::Array(a) => for x in a { *x = Value::String(map(x)?); },
                    Value::Null => {},
                    _ => return Err(Error::new(format!("node '{full}': invalid input endpoint"))),
                }
            }
        }
        if nd["type"] == "subnet" {
            let input_desc = sockets(&nd, "input_sockets")?;
            let output_desc = sockets(&nd, "output_sockets")?;
            let mut ext2 = Vec::new();
            if input_desc.is_empty() {
                ext2 = match &nd["inputs"]["geo"] {
                    Value::String(s) => vec![Some(s.clone())],
                    Value::Array(a) => a.iter().map(|x| x.as_str().map(str::to_owned)).collect(),
                    _ => Vec::new(),
                };
            } else {
                ext2.resize(input_desc.iter().map(|(i,_)| *i).max().unwrap_or(0)+1, None);
                for (index, _) in &input_desc {
                    ext2[*index] = nd["inputs"].get(format!("in{index}")).and_then(Value::as_str).map(str::to_owned);
                }
            }
            let mut inner = nd["inner"]["nodes"].as_object().ok_or_else(|| Error::new(format!("node '{full}': subnet needs inner.nodes")))?.clone();
            if let Some(promoted) = nd["promoted_params"].as_array() {
                for entry in promoted {
                    let (Some(name),Some(target),Some(param)) = (entry["name"].as_str(),entry["node"].as_str(),entry["param"].as_str()) else { return Err(Error::new(format!("node '{full}': invalid promoted parameter"))); };
                    let target_node = inner.get_mut(target).ok_or_else(|| Error::new(format!("node '{full}': promoted target '{target}' is missing")))?;
                    target_node["channels"][param] = Value::String(format!("ch('../../{id}/{name}')"));
                }
            }
            flatten_subnets(&inner, &format!("{full}/"), &ext2, out, depth + 1)?;
            if output_desc.is_empty() {
                let output = nd["inner"]["output"].as_str().filter(|o| inner.contains_key(*o))
                    .ok_or_else(|| Error::new(format!("node '{full}': subnet inner.output must name an inner node")))?;
                out.insert(full.clone(), subnet_alias(&nd,format!("{full}/{output}")));
            } else {
                for (index, _) in output_desc {
                    let boundary = inner.iter().filter(|(_, node)| node["type"] == "subnet_output" && node["port_index"].as_u64() == Some(index as u64))
                        .map(|(id, _)| id).collect::<Vec<_>>();
                    if boundary.len() != 1 { return Err(Error::new(format!("subnet '{full}' output {index} needs one boundary node"))); }
                    let alias = if index == 0 { full.clone() } else { format!("{full}/$out{index}") };
                    let source=format!("{full}/{}", boundary[0]);
                    out.insert(alias, if index==0 {subnet_alias(&nd,source)} else {serde_json::json!({"type":"subnet","inputs":{"geo":source}})});
                }
            }
        } else if nd["type"] == "subnet_input" {
            let index = nd["port_index"].as_u64().and_then(|n| usize::try_from(n).ok()).ok_or_else(|| Error::new(format!("node '{full}': subnet_input needs port_index")))?;
            if let Some(Some(source)) = ext.get(index) {
                out.insert(full, serde_json::json!({"type": "subnet", "inputs": {"geo": source}}));
            } else {
                out.insert(full, serde_json::json!({"type": "subnet_input"}));
            }
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
    #[test]
    fn subnet_promoted_slider_value_recooks_inner_geometry() {
        let mut doc=json!({"display":"subnet1","nodes":{"subnet1":{"type":"subnet",
            "params":{"width":2.0},"promoted_params":[{"name":"width","node":"box1","param":"size.x","default":1.0}],
            "inner":{"nodes":{"box1":{"type":"box","params":{"size":[1.0,1.0,1.0]}}},"output":"box1"}}}});
        let first=Graph::from_value(&doc).unwrap().cook(None).unwrap();
        let mut session=crate::session::Session::new(Graph::from_value(&doc).unwrap());
        assert_eq!(session.cook(None).unwrap().p(),first.p());
        doc["nodes"]["subnet1"]["params"]["width"]=json!(4.0);
        let second=Graph::from_value(&doc).unwrap().cook(None).unwrap();
        session.replace_graph(Graph::from_value(&doc).unwrap());
        assert_eq!(session.cook(None).unwrap().p(),second.p());
        assert_ne!(first.p(),second.p());
        assert_eq!(second.p().chunks_exact(3).map(|p|p[0]).fold(f32::NEG_INFINITY,f32::max),2.0);
    }
    #[test]
    fn subnet_promoted_channel_resolves_into_inner_node() {
        let doc=json!({"display":"subnet1","nodes":{
            "driver":{"type":"box","params":{"size":[3.0,1.0,1.0]}},
            "follower":{"type":"box","params":{"size":[1.0,1.0,1.0]},"channels":{"size.x":"ch('../subnet1/width')"}},
            "subnet1":{"type":"subnet","params":{"width":1.0},"channels":{"width":"ch('../driver/size.x') * 2"},
                "promoted_params":[{"name":"width","node":"box1","param":"size.x","default":1.0}],
                "inner":{"nodes":{"box1":{"type":"box","params":{"size":[1.0,1.0,1.0]}}},"output":"box1"}}
        }});
        let mut expanded=doc.clone();
        expanded["nodes"]=Value::Object(expand_subnets(doc["nodes"].as_object().unwrap()).unwrap().unwrap());
        let resolved=crate::channels::resolve_graph(&Graph::from_value(&expanded).unwrap());
        assert!(resolved.diagnostics.is_empty(),"{:?}",resolved.diagnostics);
        assert_eq!(resolved.params["subnet1"]["width"],json!(6.0));
        assert_eq!(resolved.params["subnet1/box1"]["size"][0],json!(6.0));
        assert_eq!(resolved.params["follower"]["size"][0],json!(6.0));
    }
}
