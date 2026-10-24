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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoint {
    pub node: String,
    pub output: usize,
}

impl Endpoint {
    fn value(&self) -> Value {
        if self.output == 0 { Value::String(self.node.clone()) }
        else { serde_json::json!({"node": self.node, "output": self.output}) }
    }
}

pub struct NodeDef {
    pub node_type: String,
    /// Port name -> the node ids connected to it (one, or several for a multi input).
    pub inputs: Vec<(String, Vec<String>)>,
    /// Output sockets for the corresponding input connections.
    pub endpoints: Vec<(String, Vec<Endpoint>)>,
    pub params: Option<Map<String, Value>>,
    pub channels: Option<Map<String, Value>>,
    pub locked_params: Option<Vec<String>>,
    pub bypass: bool,
}

/// One framed loop in a graph scope. Nodes remain in that scope's ordinary nodes map.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ZoneDef {
    pub id: String,
    pub kind: String,
    pub start: String,
    pub end: String,
    pub members: Vec<String>,
    pub parent: Option<String>,
}

impl ZoneDef {
    fn value(&self) -> Value {
        let mut v = serde_json::json!({"kind":self.kind,"start":self.start,"end":self.end,"members":self.members});
        if let Some(parent) = &self.parent { v["parent"] = Value::String(parent.clone()); }
        v
    }
}

impl NodeDef {
    pub fn output_for(&self, port: &str, index: usize, source: &str) -> Endpoint {
        self.endpoints.iter().find(|(name, _)| name == port)
            .and_then(|(_, refs)| refs.get(index))
            .filter(|endpoint| endpoint.node == source)
            .cloned().unwrap_or_else(|| Endpoint { node: source.to_owned(), output: 0 })
    }
}

pub struct Graph {
    /// In file order (the first `output` node is the main output, as in Python).
    pub nodes: Vec<(String, NodeDef)>,
    pub zones: Vec<ZoneDef>,
    pub display: Option<String>,
    /// Original UI/extension fields survive graph embedding in interchange files.
    source: Value,
}

fn parse_zones(raw: Option<&Value>, nodes: &Map<String,Value>) -> Result<Vec<ZoneDef>> {
    let Some(raw) = raw else { return Ok(Vec::new()); };
    let records = raw.as_object().ok_or_else(||Error::new("zones must be an object"))?;
    let mut zones = Vec::with_capacity(records.len());
    let mut boundaries = HashSet::new();
    for (id, record) in records {
        if id.is_empty() || id.contains('/') { return Err(Error::new(format!("invalid zone id {id:?}"))); }
        let obj = record.as_object().ok_or_else(||Error::new(format!("zone '{id}' must be an object")))?;
        let field = |name: &str| -> Result<String> {obj.get(name).and_then(Value::as_str).map(str::to_owned)
            .ok_or_else(||Error::new(format!("zone '{id}' needs a {name} id")))};
        let kind = field("kind")?;
        if kind != "for_each" { return Err(Error::new(format!("zone '{id}' has unknown kind {kind:?}"))); }
        let start = field("start")?;
        let end = field("end")?;
        if start == end || !boundaries.insert(start.clone()) || !boundaries.insert(end.clone()) {
            return Err(Error::new(format!("zone '{id}' has reused boundaries")));
        }
        for (name, expected) in [(&start,"for_each_start"),(&end,"for_each_end")] {
            if nodes.get(name).and_then(|n|n["type"].as_str()) != Some(expected) {
                return Err(Error::new(format!("zone '{id}' boundary '{name}' must be a {expected} node")));
            }
        }
        let members = obj.get("members").and_then(Value::as_array)
            .ok_or_else(||Error::new(format!("zone '{id}' needs a members array")))?;
        let mut seen = HashSet::new();
        let members: Vec<String> = members.iter().map(|m| {
            let member = m.as_str().ok_or_else(||Error::new(format!("zone '{id}' has a non-string member")))?;
            if member == start || member == end || !nodes.contains_key(member) || !seen.insert(member) {
                return Err(Error::new(format!("zone '{id}' has an invalid or duplicate member '{member}'")));
            }
            Ok(member.to_owned())
        }).collect::<Result<_>>()?;
        let parent = match obj.get("parent") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => Some(s.clone()),
            _ => return Err(Error::new(format!("zone '{id}' parent must be a zone id"))),
        };
        zones.push(ZoneDef{id:id.clone(),kind,start,end,members,parent});
    }
    for zone in &zones {
        let mut seen = HashSet::new();
        let mut parent = zone.parent.as_deref();
        while let Some(id) = parent {
            if !seen.insert(id) { return Err(Error::new(format!("zone '{}' has a parent cycle",zone.id))); }
            let outer = zones.iter().find(|z|z.id==id).ok_or_else(||Error::new(format!("zone '{}' has missing parent '{id}'",zone.id)))?;
            if !outer.members.contains(&zone.start) || !outer.members.contains(&zone.end) {
                return Err(Error::new(format!("zone '{}' boundaries must be members of parent '{id}'",zone.id)));
            }
            parent = outer.parent.as_deref();
        }
    }
    Ok(zones)
}

#[cfg(test)]
mod zone_record_tests {
    use super::*;
    use serde_json::json;

    fn nested() -> Value {json!({"nodes":{
        "source":{"type":"point"},
        "outer_start":{"type":"for_each_start","inputs":{"geo":"source"}},
        "inner_start":{"type":"for_each_start","inputs":{"geo":"outer_start"}},
        "inner_body":{"type":"null","inputs":{"geo":"inner_start"}},
        "inner_end":{"type":"for_each_end","inputs":{"begin":"inner_start","generated":"inner_body"}},
        "outer_body":{"type":"null","inputs":{"geo":"inner_end"}},
        "outer_end":{"type":"for_each_end","inputs":{"begin":"outer_start","generated":"outer_body"}}
    },"zones":{
        "outer":{"kind":"for_each","start":"outer_start","end":"outer_end","members":["inner_start","inner_body","inner_end","outer_body"]},
        "inner":{"kind":"for_each","start":"inner_start","end":"inner_end","members":["inner_body"],"parent":"outer"}
    }})}

    #[test] fn nested_zones_round_trip_in_root_and_subnet_scopes() {
        let original=nested();
        let graph=Graph::from_value(&original).unwrap();
        assert_eq!(graph.zones.len(),2);
        assert_eq!(graph.to_value(),original);
        let scoped=json!({"nodes":{"sub":{"type":"subnet","inner":original}}});
        assert_eq!(Graph::from_value(&scoped).unwrap().to_value(),scoped);
    }
    #[test] fn rejects_invalid_boundaries_members_and_parent() {
        let mut bad=nested(); bad["zones"]["inner"]["start"]=json!("source");
        assert!(Graph::from_value(&bad).err().unwrap().to_string().contains("boundary"));
        let mut bad=nested(); bad["zones"]["inner"]["members"]=json!(["inner_body","inner_body"]);
        assert!(Graph::from_value(&bad).err().unwrap().to_string().contains("duplicate"));
        let mut bad=nested(); bad["zones"]["inner"]["parent"]=json!("missing");
        assert!(Graph::from_value(&bad).err().unwrap().to_string().contains("missing parent"));
        let mut bad=nested(); bad["zones"]["outer"]["members"]=json!(["inner_body","outer_body"]);
        assert!(Graph::from_value(&bad).err().unwrap().to_string().contains("parent"));
    }
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
            if node_type == "subnet" {
                if let Some(inner) = nd.get("inner") { Graph::from_value(inner).map_err(|e| Error::new(format!("subnet '{id}': {e}")))?; }
            }
            if node_type == "controller" { crate::controller::validate(&Value::Object(nd.clone())).map_err(|e| e.in_node(id, &node_type))?; }
            if nd.contains_key("locked_geometry") {
                return Err(Error::new(format!("node '{id}': locked geometry is not supported by the Rust backend yet")));
            }
            let mut inputs = Vec::new();
            let mut endpoints = Vec::new();
            if let Some(ins) = nd.get("inputs").and_then(Value::as_object) {
                for (port, r) in ins {
                    if matches!(node_type.as_str(), "loop_begin" | "loop_end" | "for_each_start" | "for_each_end") && r.is_array() {
                        return Err(Error::new(format!("node '{id}': {port} input must be a single node")));
                    }
                    let parse = |r: &Value| -> Result<Endpoint> {
                        if let Some(s) = r.as_str() { return Ok(Endpoint { node: s.to_owned(), output: 0 }); }
                        let Some(o) = r.as_object() else { return Err(Error::new(format!("node '{id}': input '{port}' must name a node"))); };
                        let name = o.get("node").and_then(Value::as_str).ok_or_else(|| Error::new(format!("node '{id}': input '{port}' endpoint needs a node")))?;
                        let index = o.get("output").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()).ok_or_else(|| Error::new(format!("node '{id}': input '{port}' endpoint needs an output index")))?;
                        if index > 0 {
                            let source = nodes_v.get(name);
                            let available = source.and_then(|n| n["type"].as_str()).and_then(nodes::get)
                                .is_some_and(|spec| index < spec.outputs.len());
                            let subnet = source.is_some_and(|n| n["type"] == "subnet")
                                && source.is_some_and(|n| sockets(n, "output_sockets").is_ok_and(|s| s.iter().any(|(i,_)| *i == index)));
                            if !available && !subnet { return Err(Error::new(format!("node '{id}': output {index} of '{name}' is unavailable"))); }
                        }
                        Ok(Endpoint { node: name.to_owned(), output: index })
                    };
                    let refs = match r {
                        Value::Null => Vec::new(),
                        Value::Array(a) => a.iter().map(parse).collect::<Result<Vec<_>>>()?,
                        _ => vec![parse(r)?],
                    };
                    inputs.push((port.clone(), refs.iter().map(|r| r.node.clone()).collect()));
                    endpoints.push((port.clone(), refs));
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
            nodes.push((id.clone(), NodeDef { node_type, inputs, endpoints, params, channels, locked_params, bypass }));
        }
        let zones = parse_zones(obj.get("zones"), nodes_v)?;
        let display = obj.get("display").and_then(|v| v.as_str().or_else(|| v.as_array()?.iter().find_map(Value::as_str))).map(str::to_string);
        Ok(Graph { nodes, zones, display, source: v.clone() })
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
            let same = baseline.as_ref().and_then(|g| g.node(id)).is_some_and(|original| original.inputs == def.inputs && original.endpoints == def.endpoints)
                && old_inputs.is_some() || old_inputs.is_none() && def.inputs.is_empty();
            if !same { nd["inputs"] = Value::Object(def.inputs.iter().map(|(port,refs)| {
                let values: Vec<Value> = refs.iter().enumerate().map(|(i, src)| def.output_for(port, i, src).value()).collect();
                (port.clone(), if values.len() == 1 { values[0].clone() } else { Value::Array(values) })
            }).collect()); }
        }
        if value["display"].as_str() != self.display.as_deref() {
            if let Some(display) = &self.display { value["display"] = Value::String(display.clone()); }
            else { value.as_object_mut().unwrap().remove("display"); }
        }
        if self.source.get("zones").is_some() || !self.zones.is_empty() {
            value["zones"] = Value::Object(self.zones.iter().map(|zone|(zone.id.clone(),zone.value())).collect());
        }
        value
    }

    pub(crate) fn node(&self, id: &str) -> Option<&NodeDef> {
        self.nodes.iter().find(|(n, _)| n == id).map(|(_, d)| d)
    }
    pub fn node_channels(&self,id:&str)->Option<&Map<String,Value>> {self.node(id)?.channels.as_ref()}
    pub fn node_locks(&self,id:&str)->Option<&Vec<String>> {self.node(id)?.locked_params.as_ref()}
    pub fn controller_param(&self, id: &str, name: &str) -> Option<&Value> {
        let node = self.source["nodes"].get(id)?;
        (node["type"] == "controller").then(|| crate::controller::parameter(node, name)).flatten()
    }

    /// Promoted names are declared on a subnet and stored as ordinary outer params.
    pub fn promoted_param(&self, id: &str, name: &str) -> Option<&Value> {
        if let Some(node) = self.source["nodes"].get(id) {
            return nodes::subnet_public_params(node).into_iter().find(|p| p["name"] == name);
        }
        let mut node = &self.source["nodes"];
        let mut parts = id.split('/').peekable();
        while let Some(part) = parts.next() {
            node = &node[part];
            if parts.peek().is_some() { node = &node["inner"]["nodes"]; }
        }
        nodes::subnet_public_params(node).into_iter().find(|p| p["name"] == name)
    }

    pub(crate) fn promoted_params(&self, id: &str) -> Vec<&Value> {
        let mut node = &self.source["nodes"];
        let mut parts = id.split('/').peekable();
        while let Some(part) = parts.next() {
            node = &node[part];
            if parts.peek().is_some() { node = &node["inner"]["nodes"]; }
        }
        nodes::subnet_public_params(node)
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
pub(crate) fn run_node_outputs(id: &str, def: &NodeDef, spec: &nodes::NodeSpec, inputs: &nodes::Inputs, resolved: Option<&Map<String,Value>>) -> Result<nodes::NodeOutputs> {
    let result = if def.bypass {
        let first = inputs.first().and_then(|i| i.first()).map(|g| (**g).clone()).unwrap_or_default();
        let mut outputs = vec![first];
        outputs.resize_with(spec.outputs.len(), Geometry::default);
        Ok(outputs.into())
    } else {
        Params::resolve_node(&def.node_type, spec.params, if def.node_type == "subnet" {None} else {resolved.or(def.params.as_ref())})
            .and_then(|p| if def.node_type == "usd_export" { nodes::usd_nodes::cook_named(inputs, &p, id).map(Into::into) } else { spec.cook_outputs(inputs, &p) })
            .and_then(|g: nodes::NodeOutputs| {
                if g.0.len() != spec.outputs.len() { return Err(Error::new(format!("expected {} outputs, got {}", spec.outputs.len(), g.0.len()))); }
                for output in &g.0 { output.validate()?; }
                Ok(g)
            })
    };
    result.map_err(|e| e.in_node(id, &def.node_type))
}

struct Cook<'a> {
    graph: &'a Graph,
    plan: &'a nodes::r#loop::Plan,
    budget: nodes::r#loop::Budget,
    done: HashMap<String, Vec<Arc<Geometry>>>,
    active: HashSet<String>,
    resolved: Arc<crate::channels::ResolvedChannels>,
}

impl Cook<'_> {
    fn node(&mut self, id: &str) -> Result<Arc<Geometry>> {
        self.node_output(id, 0)
    }
    fn node_output(&mut self, id: &str, output: usize) -> Result<Arc<Geometry>> {
        if let Some(g) = self.done.get(id) { return g.get(output).cloned().ok_or_else(|| Error::new(format!("node '{id}' has no output {output}"))); }
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
            let geo = Arc::new(plan.cook_end(graph, &resolved, id, budget, &mut |src, output| self.node_output(src, output))?.0);
            self.active.remove(id);
            self.done.insert(id.into(), vec![geo.clone()]);
            return if output == 0 { Ok(geo) } else { Err(Error::new(format!("node '{id}' has no output {output}"))) };
        }
        // cook what this node reads (their errors pass through unchanged: they name their own node)
        let mut inputs: Vec<Vec<Arc<Geometry>>> = Vec::with_capacity(spec.inputs.len());
        for port in spec.inputs {
            let refs = def.inputs.iter().find(|(p, _)| p == port.name).map(|(_, r)| r.as_slice()).unwrap_or(&[]);
            if !port.multi && refs.len() > 1 {
                return Err(Error::new(format!("node '{id}': input '{}' takes one connection", port.name)));
            }
            let mut got = Vec::with_capacity(refs.len());
            for (index, r) in refs.iter().enumerate() {
                let endpoint = def.output_for(port.name, index, r);
                got.push(self.node_output(r, endpoint.output)?);
            }
            inputs.push(got);
        }
        let outputs: Vec<_> = run_node_outputs(id, def, spec, &inputs, self.resolved.params.get(id))?.0.into_iter().map(Arc::new).collect();
        self.active.remove(id);
        let geo = outputs.get(output).cloned().ok_or_else(|| Error::new(format!("node '{id}' has no output {output}")))?;
        self.done.insert(id.to_string(), outputs);
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
fn subnet_alias(node: &Value, source: Value) -> Value {
    let mut alias=serde_json::json!({"type":"subnet","inputs":{"geo":source}});
    for field in ["params","channels","promoted_params"] {
        if let Some(value)=node.get(field) {alias[field]=value.clone();}
    }
    let public = nodes::subnet_public_params(node);
    if !public.is_empty() { alias["promoted_params"] = Value::Array(public.into_iter().cloned().collect()); }
    alias
}

fn flatten_subnets(nodes: &Map<String, Value>, prefix: &str, ext: &[Option<Value>], out: &mut Map<String, Value>, depth: usize) -> Result<()> {
    if depth > 16 { return Err(Error::new("subnets are nested too deeply")); }
    let map = |r: &Value| -> Result<Value> {
        let (name, index) = endpoint(r)?;
        if let Some(k) = name.strip_prefix("$in") {
            if index != 0 { return Err(Error::new(format!("subnet input '{name}' has no output {index}"))); }
            let k = k.parse::<usize>().map_err(|_| Error::new(format!("invalid subnet input '{name}'")))?;
            return Ok(ext.get(k).and_then(Clone::clone).unwrap_or_else(|| Value::String(format!("{prefix}$empty{k}"))));
        }
        if index != 0 {
            let node = nodes.get(&name).ok_or_else(|| Error::new(format!("no such node '{name}'")))?;
            if node["type"] == "subnet" {
                if !sockets(node, "output_sockets")?.iter().any(|(i, _)| *i == index) {
                    return Err(Error::new(format!("subnet '{name}' has no output {index}")));
                }
                return Ok(Value::String(format!("{prefix}{name}/$out{index}")));
            }
            if !node["type"].as_str().and_then(nodes::get).is_some_and(|spec| index < spec.outputs.len()) {
                return Err(Error::new(format!("node '{name}' has no output {index}")));
            }
            return Ok(serde_json::json!({"node": format!("{prefix}{name}"), "output": index}));
        }
        Ok(Value::String(format!("{prefix}{name}")))
    };
    for (id, nd) in nodes {
        let full = format!("{prefix}{id}");
        let mut nd = nd.clone();
        if let Some(inputs) = nd.get_mut("inputs").and_then(Value::as_object_mut) {
            for r in inputs.values_mut() {
                match r {
                    Value::String(_) | Value::Object(_) => *r = map(r)?,
                    Value::Array(a) => for x in a { *x = map(x)?; },
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
                    Value::String(_) | Value::Object(_) => vec![Some(nd["inputs"]["geo"].clone())],
                    Value::Array(a) => a.iter().map(|x| Some(x.clone())).collect(),
                    _ => Vec::new(),
                };
            } else {
                ext2.resize(input_desc.iter().map(|(i,_)| *i).max().unwrap_or(0)+1, None);
                for (index, _) in &input_desc {
                    ext2[*index] = nd["inputs"].get(format!("in{index}")).cloned();
                }
            }
            let mut inner = nd["inner"]["nodes"].as_object().ok_or_else(|| Error::new(format!("node '{full}': subnet needs inner.nodes")))?.clone();
            {
                let promoted = nodes::subnet_public_params(&nd).into_iter().cloned().collect::<Vec<_>>();
                for entry in &promoted {
                    let (Some(name),Some(target),Some(param)) = (entry["name"].as_str(),entry["node"].as_str(),entry["param"].as_str()) else { return Err(Error::new(format!("node '{full}': invalid promoted parameter"))); };
                    let target_node = inner.get_mut(target).ok_or_else(|| Error::new(format!("node '{full}': promoted target '{target}' is missing")))?;
                    if entry["default"].as_array().is_some_and(|values| values.len() == 3) {
                        for axis in ["x", "y", "z"] {
                            target_node["channels"][format!("{param}.{axis}")] =
                                Value::String(format!("ch('../../{id}/{name}.{axis}')"));
                        }
                    } else {
                        target_node["channels"][param] = Value::String(format!("ch('../../{id}/{name}')"));
                    }
                }
            }
            flatten_subnets(&inner, &format!("{full}/"), &ext2, out, depth + 1)?;
            if output_desc.is_empty() {
                let output = nd["inner"]["output"].as_str().filter(|o| inner.contains_key(*o))
                    .ok_or_else(|| Error::new(format!("node '{full}': subnet inner.output must name an inner node")))?;
                out.insert(full.clone(), subnet_alias(&nd,Value::String(format!("{full}/{output}"))));
            } else {
                for (index, _) in output_desc {
                    let boundary = inner.iter().filter(|(_, node)| matches!(node["type"].as_str(), Some("output" | "subnet_output")) && node["port_index"].as_u64() == Some(index as u64))
                        .map(|(id, _)| id).collect::<Vec<_>>();
                    if boundary.len() != 1 { return Err(Error::new(format!("subnet '{full}' output {index} needs one boundary node"))); }
                    let alias = if index == 0 { full.clone() } else { format!("{full}/$out{index}") };
                    let source=Value::String(format!("{full}/{}", boundary[0]));
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

    #[test]
    fn fresh_subnet_input_to_output_cooks_without_error() {
        let doc = json!({"display":"sub","nodes":{"sub":{"type":"subnet","inner":{
            "nodes":{"in":{"type":"input"},"out":{"type":"output","inputs":{"geo":"in"}}},
            "output":"out"}}}});
        let graph = Graph::from_value(&doc).unwrap();
        assert_eq!(graph.cook(None).unwrap().npoints, 0);
        assert_eq!(crate::session::Session::new(graph).cook(None).unwrap().npoints, 0);
    }

    #[test]
    fn input_custom_parameter_is_public_and_drives_inner_node() {
        let doc = json!({"display":"sub","nodes":{"sub":{"type":"subnet","inner":{
            "nodes":{
                "seed":{"type":"point"},
                "in":{"type":"input","inputs":{"preview":"seed"},"custom_params":[
                    {"name":"offset","default":1.0,"node":"move","param":"translate.x"}]},
                "move":{"type":"transform","inputs":{"geo":"in"}},
                "out":{"type":"output","inputs":{"geo":"move"},"custom_params":[
                    {"name":"rise","default":0.0,"node":"move","param":"translate.y"}]}
            },"output":"out"}}}});
        let graph = Graph::from_value(&doc).unwrap();
        assert_eq!(graph.promoted_param("sub", "offset").unwrap()["default"], json!(1.0));
        assert_eq!(graph.promoted_param("sub", "rise").unwrap()["default"], json!(0.0));
        let mut session = crate::session::Session::new(graph);
        session.set_param("sub", "offset", json!(3.0)).unwrap();
        session.set_param("sub", "rise", json!(4.0)).unwrap();
        assert_eq!(session.graph().to_value()["nodes"]["sub"]["params"], json!({"offset":3.0,"rise":4.0}));
        let result = session.cook(None).unwrap();
        assert_eq!(result.p()[0], 3.0);
        assert_eq!(result.p()[1], 4.0);
        assert_eq!(session.resolved_channels().params["sub"]["offset"], json!(3.0));
    }

    #[test]
    fn output_accepts_an_inner_stream_node() {
        let doc = json!({"display":"sub","nodes":{"sub":{"type":"subnet",
            "output_sockets":[{"index":0,"name":"geo"}],"inner":{
            "nodes":{"point":{"type":"point"},"move":{"type":"transform","inputs":{"geo":"point"},
                "params":{"translate":[2.0,0.0,0.0]}},"out":{"type":"subnet_output","port_index":0,
                "inputs":{"geo":"move"}}}}}}});
        let graph = Graph::from_value(&doc).unwrap();
        assert_eq!(graph.cook(None).unwrap().p()[0], 2.0);
        assert_eq!(crate::session::Session::new(graph).cook(None).unwrap().p()[0], 2.0);
    }

    #[test]
    fn indexed_legacy_output_boundary_cooks() {
        let doc=json!({"display":"sub","nodes":{"sub":{"type":"subnet",
            "output_sockets":[{"index":0,"name":"Geometry"}],"inner":{"nodes":{
                "point":{"type":"point"},"out":{"type":"output","port_index":0,"inputs":{"geo":"point"}}
            },"output":"out"}}}});
        assert_eq!(Graph::from_value(&doc).unwrap().cook(None).unwrap().npoints,1);
    }

    #[test]
    fn indexed_boundaries_keep_sparse_port_indices() {
        let doc=json!({"display":"result","nodes":{
            "b":{"type":"box"},
            "sub":{"type":"subnet","input_sockets":[{"index":2,"name":"Third"}],
                "output_sockets":[{"index":0,"name":"First"},{"index":3,"name":"Fourth"}],
                "inputs":{"in2":"b"},"inner":{"nodes":{
                    "incoming":{"type":"subnet_input","port_index":2},
                    "first":{"type":"subnet_output","port_index":0,"inputs":{"geo":"incoming"}},
                    "fourth":{"type":"subnet_output","port_index":3,"inputs":{"geo":"incoming"}}
                }}},
            "result":{"type":"null","inputs":{"geo":{"node":"sub","output":3}}}
        }});
        let graph=Graph::from_value(&doc).unwrap();
        assert_eq!(graph.to_value(),doc);
        let expanded=expand_subnets(doc["nodes"].as_object().unwrap()).unwrap().unwrap();
        assert_eq!(expanded["sub/incoming"]["inputs"]["geo"],"b");
        assert_eq!(expanded["sub/$out3"]["inputs"]["geo"],"sub/fourth");
        assert_eq!(expanded["result"]["inputs"]["geo"],"sub/$out3");
        assert_eq!(graph.cook(None).unwrap().nprims(),Graph::from_value(&json!({"nodes":{"b":{"type":"box"}},"display":"b"})).unwrap().cook(None).unwrap().nprims());
    }

    #[test]
    fn legacy_single_port_expansion_is_stable() {
        let doc=grouped_box();
        let expanded=expand_subnets(doc["nodes"].as_object().unwrap()).unwrap().unwrap();
        assert_eq!(expanded["subnet1"]["inputs"]["geo"],"subnet1/subdivide1");
        assert_eq!(expanded["subnet1/box1"]["type"],"box");
        assert_eq!(expanded["subnet1/subdivide1"]["inputs"]["geo"],"subnet1/box1");
        assert_eq!(expanded["out"]["inputs"]["geo"],"subnet1");
    }

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
    fn subnet_override_is_the_inner_nodes_effective_value() {
        let doc=json!({"display":"subnet1","nodes":{"subnet1":{"type":"subnet",
            "params":{"width":4.0},"promoted_params":[{"name":"width","node":"box1","param":"size.x","default":1.0}],
            "inner":{"nodes":{"box1":{"type":"box","params":{"size":[1.0,1.0,1.0]}}},"output":"box1"}}}});
        let scope=crate::channels::resolve_scope(&doc,&["subnet1".to_owned()]).unwrap();
        assert_eq!(scope.resolved.params["box1"]["size"],json!([4.0,1.0,1.0]));
        assert!(scope.overridden.contains(&("box1".to_owned(),"size".to_owned())));
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

#[cfg(test)]
mod multi_output_graph_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn graph_endpoints_round_trip_and_bounds() {
        let value = json!({"nodes": {
            "box": {"type":"box"},
            "fill": {"type":"flood_fill", "inputs":{"geometry":"box"}},
            "a": {"type":"null", "inputs":{"geo":"fill"}},
            "b": {"type":"null", "inputs":{"geo":{"node":"fill","output":1}}}
        }});
        let graph = Graph::from_value(&value).unwrap();
        assert_eq!(graph.node("a").unwrap().output_for("geo",0,"fill").output,0);
        assert_eq!(graph.node("b").unwrap().output_for("geo",0,"fill").output,1);
        assert_eq!(graph.to_value(), value);
        let mut bad = value.clone();
        bad["nodes"]["b"]["inputs"]["geo"]["output"] = json!(2);
        assert!(Graph::from_value(&bad).err().unwrap().to_string().contains("unavailable"));
    }

    #[test]
    fn graph_subnet_preserves_inner_multi_output() {
        let value = json!({"display":"out","nodes":{
            "sub":{"type":"subnet","output_sockets":[{"index":0},{"index":1}],
                "inner":{"nodes":{
                    "box":{"type":"box"},
                    "fill":{"type":"flood_fill","inputs":{"geometry":"box"},"params":{"seed_group":"0","max_steps":0}},
                    "piece":{"type":"subnet_output","port_index":0,"inputs":{"geo":"fill"}},
                    "front":{"type":"subnet_output","port_index":1,"inputs":{"geo":{"node":"fill","output":1}}}
                }}},
            "out":{"type":"null","inputs":{"geo":{"node":"sub","output":1}}}
        }});
        let graph=Graph::from_value(&value).unwrap();
        assert_eq!(graph.to_value(),value);
        assert!(graph.cook(None).unwrap().nprims()>0);
    }
}
