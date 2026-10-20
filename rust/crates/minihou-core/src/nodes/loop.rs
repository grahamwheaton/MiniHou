//! Paired DAG repeat regions, mirroring nodes/loop.py and loops.py.
use std::{cell::Cell, collections::{HashMap, HashSet}, rc::Rc, sync::Arc, time::Instant};
use crate::{Error, Geometry, Graph, Result, graph::run_node_outputs, params::Params};
use super::{InputSpec, Inputs, NodeSpec};

pub const LOOP_BEGIN: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None, name: "loop_begin", inputs: super::GEO, params: &[], cook: begin };
pub const LOOP_END: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None, name: "loop_end", inputs: &[
    InputSpec { name: "geo", multi: false, optional: false },
    InputSpec { name: "begin", multi: false, optional: false },
], params: crate::params!("iterations": 3), cook: end };
fn begin(_: &Inputs, _: &Params) -> Result<Geometry> { Err(Error::new("Loop Begin must be evaluated through its paired Loop End")) }
fn end(_: &Inputs, _: &Params) -> Result<Geometry> { Err(Error::new("Loop End requires graph evaluation (it is not a standalone body node)")) }

pub(crate) type Budget = Rc<Cell<usize>>;
pub(crate) fn budget() -> Budget { Rc::new(Cell::new(10000)) }

#[derive(Default)]
pub(crate) struct Plan { scopes: HashMap<String, Vec<String>> }

fn one<'a>(graph: &'a Graph, id: &str, port: &str) -> Result<&'a str> {
    let d = graph.node(id).ok_or_else(|| Error::new(format!("no such node '{id}'")))?;
    let refs = d.inputs.iter().find(|(p, _)| p == port).map(|(_, r)| r.as_slice()).unwrap_or(&[]);
    if refs.len() != 1 { return Err(Error::new(format!("node '{id}': {port} input must be a single node"))); }
    Ok(&refs[0])
}
fn one_output(graph: &Graph, id: &str, port: &str) -> Result<usize> {
    let source = one(graph, id, port)?;
    Ok(graph.node(id).unwrap().output_for(port, 0, source).output)
}

impl Plan {
    /// Only reachable nodes are inspected: unfinished unrelated branches are harmless.
    pub(crate) fn analyze(graph: &Graph, target: &str) -> Result<Self> {
        fn visit(graph: &Graph, id: &str, done: &mut HashSet<String>, active: &mut HashSet<String>, order: &mut Vec<String>) -> Result<()> {
            if done.contains(id) { return Ok(()); }
            if !active.insert(id.into()) { return Err(Error::new(format!("the graph has a cycle through node '{id}'"))); }
            let d = graph.node(id).ok_or_else(|| Error::new(format!("no such node '{id}'")))?;
            let spec = super::get(&d.node_type).ok_or_else(||Error::new(format!("node '{id}': node type '{}' is not in the Rust backend yet",d.node_type)))?;
            for (port,refs) in &d.inputs {
                let input=spec.inputs.iter().find(|p|p.name==port).ok_or_else(||Error::new(format!("node '{id}': '{}' has no input '{port}'",d.node_type)))?;
                if !input.multi&&refs.len()>1{return Err(Error::new(format!("node '{id}': input '{port}' takes one connection")));}
            }
            for (_, refs) in &d.inputs { for src in refs { visit(graph, src, done, active, order)?; } }
            active.remove(id); done.insert(id.into()); order.push(id.into()); Ok(())
        }
        let mut order = Vec::new();
        visit(graph, target, &mut HashSet::new(), &mut HashSet::new(), &mut order)?;
        let mut plan = Self::default(); let mut pairs = HashMap::new();
        for id in &order {
            let d = graph.node(id).unwrap();
            let incoming: Vec<Vec<String>> = d.inputs.iter().flat_map(|(_, r)| r).map(|src| plan.scopes[src].clone()).collect();
            let mut scope = incoming.iter().max_by_key(|s| s.len()).cloned().unwrap_or_default();
            if incoming.iter().any(|s| !scope.starts_with(s)) { return Err(Error::new(format!("node '{id}': crossed loop blocks; close each block before combining results"))); }
            match d.node_type.as_str() {
                "loop_begin" => {
                    one(graph, id, "geo")?;
                    scope.push(id.clone());
                    if scope.len() > 16 { return Err(Error::new("loop nesting exceeds 16 blocks")); }
                }
                "loop_end" => {
                    let b = one(graph, id, "begin")?;
                    if graph.node(b).is_none_or(|n| n.node_type != "loop_begin") { return Err(Error::new(format!("Loop End '{id}': begin input must connect directly to a Loop Begin"))); }
                    if pairs.insert(b.to_owned(), id.clone()).is_some() { return Err(Error::new(format!("Loop Begin '{b}' is paired with multiple Ends"))); }
                    let body = one(graph, id, "geo")?;
                    let body_scope = &plan.scopes[body];
                    if body_scope.last().map(String::as_str) != Some(b) || body_scope != &plan.scopes[b] {
                        return Err(Error::new(format!("Loop End '{id}': crossed or disconnected body; close innermost Begin '{b}' first")));
                    }
                    scope = body_scope[..body_scope.len()-1].to_vec();
                }
                "output" if !scope.is_empty() => return Err(Error::new(format!("output '{id}' escapes a loop body; connect the paired Loop End instead"))),
                "rbd_solver" if !scope.is_empty() => return Err(Error::new("RBD Solver cannot run inside a repeat body")),
                _ => {}
            }
            plan.scopes.insert(id.clone(), scope);
        }
        for id in &order {
            if graph.node(id).unwrap().node_type == "loop_begin" && !pairs.contains_key(id) {
                return Err(Error::new(format!("Loop Begin '{id}' is unpaired; connect a Loop End and display the End, not the body")));
            }
        }
        if !plan.scopes[target].is_empty() { return Err(Error::new("display is inside a loop body; display the paired Loop End to evaluate the repeat")); }
        Ok(plan)
    }

    pub(crate) fn cook_end(&self, graph: &Graph, resolved: &crate::channels::ResolvedChannels, id: &str, budget: Budget,
        fetch: &mut dyn FnMut(&str, usize) -> Result<Arc<Geometry>>) -> Result<(Geometry,Vec<(String,f64,usize,usize)>)> {
        let mut cook = Region { graph, resolved, plan: self, budget, fetch, frames: Vec::new(), times:Vec::new() };
        let geometry=(*cook.end(id)?).clone();Ok((geometry,cook.times))
    }
}

struct Frame { scope: Vec<String>, done: HashMap<String, Vec<Arc<Geometry>>> }
struct Region<'a> {
    graph: &'a Graph, resolved: &'a crate::channels::ResolvedChannels, plan: &'a Plan, budget: Budget,
    fetch: &'a mut dyn FnMut(&str, usize) -> Result<Arc<Geometry>>, frames: Vec<Frame>, times:Vec<(String,f64,usize,usize)>,
}
impl Region<'_> {
    fn node(&mut self, id: &str) -> Result<Arc<Geometry>> {
        self.node_output(id, 0)
    }
    fn node_output(&mut self, id: &str, output: usize) -> Result<Arc<Geometry>> {
        let scope = &self.plan.scopes[id];
        let Some(slot) = self.frames.iter().rposition(|f| &f.scope == scope) else { return (self.fetch)(id, output); };
        if let Some(g) = self.frames[slot].done.get(id) { return g.get(output).cloned().ok_or_else(||Error::new(format!("node '{id}' has no output {output}"))); }
        let def = self.graph.node(id).unwrap();
        let spec = super::get(&def.node_type).ok_or_else(|| Error::new(format!("node '{id}': node type '{}' is not in the Rust backend yet", def.node_type)))?;
        let start=Instant::now();
        let outputs = if def.node_type == "loop_end" { vec![self.end(id)?] } else {
            for (port, _) in &def.inputs { if !spec.inputs.iter().any(|p| p.name == port) { return Err(Error::new(format!("node '{id}': '{}' has no input '{port}'", def.node_type))); } }
            let mut inputs = Vec::new();
            for port in spec.inputs {
                let refs = def.inputs.iter().find(|(p, _)| p == port.name).map(|(_, r)| r.as_slice()).unwrap_or(&[]);
                if !port.multi && refs.len() > 1 { return Err(Error::new(format!("node '{id}': input '{}' takes one connection", port.name))); }
                let mut got = Vec::new(); for (index,src) in refs.iter().enumerate() { got.push(self.node_output(src,def.output_for(port.name,index,src).output)?); } inputs.push(got);
            }
            run_node_outputs(id, def, spec, &inputs, self.resolved.params.get(id))?.0.into_iter().map(Arc::new).collect()
        };
        let out = outputs.get(output).cloned().ok_or_else(||Error::new(format!("node '{id}' has no output {output}")))?;
        self.times.push((id.into(),start.elapsed().as_secs_f64()*1000.,outputs[0].npoints,outputs[0].nprims()));
        self.frames[slot].done.insert(id.into(), outputs); Ok(out)
    }
    fn end(&mut self, id: &str) -> Result<Arc<Geometry>> {
        let def = self.graph.node(id).unwrap();
        let b = one(self.graph, id, "begin")?.to_owned();
        let source = one(self.graph, &b, "geo")?.to_owned();
        let source_output = one_output(self.graph, &b, "geo")?;
        let body = one(self.graph, id, "geo")?.to_owned();
        let body_output = one_output(self.graph, id, "geo")?;
        let initial = self.node_output(&source, source_output)?;
        let p = Params::resolve(LOOP_END.params, self.resolved.params.get(id).or(def.params.as_ref())).map_err(|e| e.in_node(id, "loop_end"))?;
        // Int bounds are enforced here because Params deliberately has no UI range coercion.
        let n = p.f("iterations")?;
        if !n.is_finite() || n.fract() != 0.0 || !(0.0..=1000.0).contains(&n)
            || self.resolved.params.get(id).or(def.params.as_ref()).and_then(|p| p.get("iterations")).is_some_and(|v| v.is_boolean()) {
            return Err(Error::new("iterations must be an integer from 0 to 1000").in_node(id, "loop_end"));
        }
        let count = n as usize;
        let mut state = initial.clone();
        for i in 0..count {
            let left = self.budget.get();
            if left == 0 { return Err(Error::new("repeat budget exceeded (10000 total iterations per cook)").in_node(id, "loop_end")); }
            self.budget.set(left - 1);
            let mut incoming = (*state).clone();
            incoming.detail.insert("iteration".into(), i.into()); incoming.detail.insert("numiterations".into(), count.into());
            self.frames.push(Frame { scope: self.plan.scopes[&b].clone(), done: HashMap::from([(b.clone(), vec![Arc::new(incoming)])]) });
            let result = self.node_output(&body, body_output); self.frames.pop(); state = result?;
        }
        if count > 0 {
            let mut out = (*state).clone();
            for key in ["iteration", "numiterations"] { match initial.detail.get(key) {
                Some(v) => { out.detail.insert(key.into(), v.clone()); }, None => { out.detail.remove(key); }
            } }
            out.validate()?; state = Arc::new(out);
        }
        Ok(state)
    }
}

#[cfg(test)] mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn loop_body_reads_second_output() {
        let graph=Graph::from_value(&json!({"display":"end","nodes":{
            "box":{"type":"box"},
            "begin":{"type":"loop_begin","inputs":{"geo":"box"}},
            "fill":{"type":"flood_fill","inputs":{"geometry":"begin"},"params":{"seed_group":"0","max_steps":0}},
            "end":{"type":"loop_end","inputs":{"begin":"begin","geo":{"node":"fill","output":1}},"params":{"iterations":1}}
        }})).unwrap();
        assert!(graph.cook(None).unwrap().nprims()>0);
        assert!(crate::session::Session::new(graph).cook(None).unwrap().nprims()>0);
    }
    fn graph(n: usize) -> Graph { Graph::from_value(&json!({"display":"end", "nodes":{
        "p":{"type":"point"}, "begin":{"type":"loop_begin","inputs":{"geo":"p"}},
        "step":{"type":"transform","inputs":{"geo":"begin"},"params":{"translate":[0.5,0,0]}},
        "end":{"type":"loop_end","inputs":{"geo":"step","begin":"begin"},"params":{"iterations":n}}
    }})).unwrap() }
    #[test] fn channel_link_inside_loop_uses_parameter_dag() {
        let g=Graph::from_value(&json!({"display":"end","nodes":{
            "driver":{"type":"box","params":{"size":[2,1,1]}},
            "p":{"type":"point"},
            "begin":{"type":"loop_begin","inputs":{"geo":"p"}},
            "step":{"type":"transform","inputs":{"geo":"begin"},"params":{"translate":[0,0,0]},"channels":{"translate.x":"ch('../driver/size.x')"}},
            "end":{"type":"loop_end","inputs":{"geo":"step","begin":"begin"},"params":{"iterations":2}}
        }})).unwrap();
        assert_eq!(g.cook(None).unwrap().p(),&[4.,0.,0.]);
        let mut s=crate::session::Session::new(g);
        assert_eq!(s.cook(None).unwrap().p(),&[4.,0.,0.]);
        assert!(!s.last_stats().cooked.contains(&"driver".into()));
    }
    #[test] fn feedback_graph_and_session() {
        for n in [0,1,7] { let g=graph(n); let out=g.cook(None).unwrap(); assert_eq!(out.p(), &[n as f32*0.5,0.,0.]); assert!(!out.detail.contains_key("iteration"));
            let mut s=crate::session::Session::new(g); assert_eq!(s.cook(None).unwrap().p(), out.p());
            s.cook(None).unwrap(); assert!(s.last_stats().cooked.is_empty());
            s.set_param("step","translate",json!([2,0,0])).unwrap(); assert_eq!(s.cook(None).unwrap().p()[0],n as f32*2.);
        }
    }
    #[test] fn preview_and_pairing_errors() {
        assert!(graph(3).cook(Some("step")).is_err());
        let mut g=graph(0); g.nodes.iter_mut().find(|(id,_)|id=="end").unwrap().1.inputs.iter_mut().find(|(p,_)|p=="begin").unwrap().1=vec!["p".into()];
        assert!(g.cook(None).unwrap_err().to_string().contains("directly"));
    }
    #[test] fn restores_initial_metadata_and_checks_budget() {
        let mut g=graph(2);g.nodes.iter_mut().find(|(id,_)|id=="p").unwrap().1.node_type="object_input".into();
        let mut initial=Geometry::points_only(vec![0.,0.,0.]).unwrap();initial.detail.insert("iteration".into(),90.into());initial.detail.insert("numiterations".into(),100.into());
        let mut session=crate::session::Session::new(g);
        session.set_external("object_input:p".into(),super::super::blender_input::ExternalInput::Object{geo:Arc::new(initial.clone()),matrix:None,info:serde_json::Value::Null});
        let out=session.cook(None).unwrap();assert_eq!(out.detail,initial.detail);assert_eq!(out.p()[0],1.);
        assert_eq!(session.last_stats().cooked.iter().filter(|id|id.as_str()=="step").count(),2);
        let g=graph(2);let plan=Plan::analyze(&g,"end").unwrap();let b=budget();b.set(1);
        assert!(plan.cook_end(&g,&crate::channels::resolve_graph(&g),"end",b,&mut |src, _|g.cook(Some(src))).unwrap_err().to_string().contains("budget exceeded"));
        assert!(g.cook(None).is_ok());
    }
    #[test] fn nested_feedback() {
        let mut g=graph(3);
        let extra=Graph::from_value(&json!({"nodes":{
            "ib":{"type":"loop_begin","inputs":{"geo":"step"}},
            "is":{"type":"transform","inputs":{"geo":"ib"},"params":{"translate":[0,1,0]}},
            "ie":{"type":"loop_end","inputs":{"geo":"is","begin":"ib"},"params":{"iterations":2}}
        }})).unwrap(); g.nodes.extend(extra.nodes);
        g.nodes.iter_mut().find(|(id,_)|id=="end").unwrap().1.inputs.iter_mut().find(|(p,_)|p=="geo").unwrap().1=vec!["ie".into()];
        assert_eq!(g.cook(None).unwrap().p(), &[1.5,6.,0.]);
    }
}
