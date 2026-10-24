//! Paired DAG repeat regions, mirroring nodes/loop.py and loops.py.
use std::{cell::Cell, collections::{HashMap, HashSet}, rc::Rc, sync::Arc, time::Instant};
use crate::{Error, Geometry, Graph, Result, graph::run_node_outputs, params::Params};
use super::{CachedOutput, InputSpec, Inputs, NodeSpec};

pub const LOOP_BEGIN: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None, name: "loop_begin", inputs: super::GEO, params: &[], cook: begin };
pub const LOOP_END: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None, name: "loop_end", inputs: &[
    InputSpec { name: "geo", multi: false, optional: false },
    InputSpec { name: "begin", multi: false, optional: false },
], params: crate::params!("iterations": 3), cook: end };
fn begin(_: &Inputs, _: &Params) -> Result<Geometry> { Err(Error::new("Loop Begin must be evaluated through its paired Loop End")) }
fn end(_: &Inputs, _: &Params) -> Result<Geometry> { Err(Error::new("Loop End requires graph evaluation (it is not a standalone body node)")) }

pub(crate) type Budget = Rc<Cell<usize>>;
pub(crate) fn budget() -> Budget { Rc::new(Cell::new(crate::limits::Limits::default().loop_iterations)) }

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
                "loop_begin" | "for_each_start" => {
                    one(graph, id, "geo")?;
                    if d.node_type == "for_each_start" && !graph.zones.iter().any(|z|z.start==*id) {
                        return Err(Error::new(format!("For Each Start '{id}' has no zone record")));
                    }
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
                "for_each_end" => {
                    let zone = graph.zones.iter().find(|z|z.end==*id)
                        .ok_or_else(||Error::new(format!("For Each End '{id}' has no zone record")))?;
                    let b = one(graph,id,"begin")?;
                    if b != zone.start {return Err(Error::new(format!("For Each End '{id}' must pair with '{}'",zone.start)));}
                    let body=one(graph,id,"generated")?;
                    let body_scope=&plan.scopes[body];
                    if body_scope.last().map(String::as_str)!=Some(b) || body_scope!=&plan.scopes[b] {
                        return Err(Error::new(format!("For Each End '{id}' has a crossed or disconnected body")));
                    }
                    scope=body_scope[..body_scope.len()-1].to_vec();
                    pairs.insert(b.into(),id.clone());
                }
                "output" if !scope.is_empty() => return Err(Error::new(format!("output '{id}' escapes a loop body; connect the paired Loop End instead"))),
                "rbd_solver" if !scope.is_empty() => return Err(Error::new("RBD Solver cannot run inside a repeat body")),
                _ => {}
            }
            plan.scopes.insert(id.clone(), scope);
        }
        for id in &order {
            if matches!(graph.node(id).unwrap().node_type.as_str(),"loop_begin" | "for_each_start") && !pairs.contains_key(id) {
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

struct Frame { scope: Vec<String>, done: HashMap<String, Vec<CachedOutput>> }
struct Region<'a> {
    graph: &'a Graph, resolved: &'a crate::channels::ResolvedChannels, plan: &'a Plan, budget: Budget,
    fetch: &'a mut dyn FnMut(&str, usize) -> Result<Arc<Geometry>>, frames: Vec<Frame>, times:Vec<(String,f64,usize,usize)>,
}
impl Region<'_> {
    fn node(&mut self, id: &str) -> Result<Arc<Geometry>> {
        self.node_output(id, 0)
    }
    fn node_output(&mut self, id: &str, output: usize) -> Result<Arc<Geometry>> {
        self.node_value(id,output)?.geometry()
    }
    fn node_value(&mut self, id: &str, output: usize) -> Result<CachedOutput> {
        let scope = &self.plan.scopes[id];
        let Some(slot) = self.frames.iter().rposition(|f| &f.scope == scope) else {
            return (self.fetch)(id, output).map(CachedOutput::Geometry);
        };
        if let Some(g) = self.frames[slot].done.get(id) { return g.get(output).cloned().ok_or_else(||Error::new(format!("node '{id}' has no output {output}"))); }
        let def = self.graph.node(id).unwrap();
        let spec = super::get(&def.node_type).ok_or_else(|| Error::new(format!("node '{id}': node type '{}' is not in the Rust backend yet", def.node_type)))?;
        let start=Instant::now();
        let outputs = if matches!(def.node_type.as_str(),"loop_end" | "for_each_end") {
            vec![CachedOutput::Geometry(self.end(id)?)]
        } else if def.node_type == "for_each_index" {
            let geo=one(self.graph,id,"geo")?;
            let input=one(self.graph,id,"index")?;
            let index_output=one_output(self.graph,id,"index")?;
            let source=self.node_output(geo,one_output(self.graph,id,"geo")?)?;
            let index=self.node_value(input,index_output)?.integer()?;
            let spec=super::get("for_each_index").unwrap();
            let p=Params::resolve(spec.params,self.resolved.params.get(id).or(def.params.as_ref()))?;
            vec![CachedOutput::Geometry(Arc::new(super::foreach::stamp_index(&source,index,&p.s("name")?)?))]
        } else {
            for (port, _) in &def.inputs { if !spec.inputs.iter().any(|p| p.name == port) { return Err(Error::new(format!("node '{id}': '{}' has no input '{port}'", def.node_type))); } }
            let mut inputs = Vec::new();
            for port in spec.inputs {
                let refs = def.inputs.iter().find(|(p, _)| p == port.name).map(|(_, r)| r.as_slice()).unwrap_or(&[]);
                if !port.multi && refs.len() > 1 { return Err(Error::new(format!("node '{id}': input '{}' takes one connection", port.name))); }
                let mut got = Vec::new(); for (index,src) in refs.iter().enumerate() { got.push(self.node_output(src,def.output_for(port.name,index,src).output)?); } inputs.push(got);
            }
            run_node_outputs(id, def, spec, &inputs, self.resolved.params.get(id))?.into_cached()
        };
        let out = outputs.get(output).cloned().ok_or_else(||Error::new(format!("node '{id}' has no output {output}")))?;
        let (points,primitives)=match &outputs[0] {CachedOutput::Geometry(g)=>(g.npoints,g.nprims()),CachedOutput::Integer(_)=>(0,0)};
        self.times.push((id.into(),start.elapsed().as_secs_f64()*1000.,points,primitives));
        self.frames[slot].done.insert(id.into(), outputs); Ok(out)
    }
    fn end(&mut self, id: &str) -> Result<Arc<Geometry>> {
        let def = self.graph.node(id).unwrap();
        if def.node_type == "for_each_end" {return self.end_for_each(id);}
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
            self.frames.push(Frame { scope: self.plan.scopes[&b].clone(), done: HashMap::from([(b.clone(), vec![CachedOutput::Geometry(Arc::new(incoming))])]) });
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

    fn end_for_each(&mut self, id: &str) -> Result<Arc<Geometry>> {
        let begin=one(self.graph,id,"begin")?.to_owned();
        let source=one(self.graph,&begin,"geo")?.to_owned();
        let source_output=one_output(self.graph,&begin,"geo")?;
        let body=one(self.graph,id,"generated")?.to_owned();
        let body_output=one_output(self.graph,id,"generated")?;
        let initial=self.node_output(&source,source_output)?;
        let start=self.graph.node(&begin).unwrap();
        let spec=super::get("for_each_start").unwrap();
        let p=Params::resolve(spec.params,self.resolved.params.get(&begin).or(start.params.as_ref()))
            .map_err(|e|e.in_node(&begin,"for_each_start"))?;
        let limit=p.i("max_pieces")?;
        if !(1..=10000).contains(&limit) {return Err(Error::new("max_pieces must be 1..10000").in_node(&begin,"for_each_start"));}
        let parts=super::foreach::partitions(&initial,&p.e("method", &["connected","attribute","domain"])?,
            &p.e("domain", &["points","edges","faces","instances"])?,&p.s("attribute")?,
            &p.s("selection")?,limit as usize).map_err(|e|e.in_node(&begin,"for_each_start"))?;
        let count=parts.len();
        let mut results=Vec::with_capacity(count);
        for (iteration,part) in parts.into_iter().enumerate() {
            let left=self.budget.get();
            if left==0 {return Err(Error::new("loop budget exceeded (10000 total iterations per cook)").in_node(id,"for_each_end"));}
            self.budget.set(left-1);
            let mut element=part.geometry;
            element.detail.insert("iteration".into(),serde_json::json!(iteration));
            element.detail.insert("numiterations".into(),serde_json::json!(count));
            element.detail.insert("value".into(),part.value);
            element.detail.insert("source_index".into(),serde_json::json!(part.source_index));
            let index=i64::try_from(part.source_index).map_err(|_|Error::new("source index exceeds i64"))?;
            self.frames.push(Frame {scope:self.plan.scopes[&begin].clone(),
                done:HashMap::from([(begin.clone(),vec![CachedOutput::Geometry(Arc::new(element)),CachedOutput::Integer(index)])])});
            let cooked=self.node_output(&body,body_output);
            self.frames.pop();
            results.push((*cooked?).clone());
        }
        let mut out=if results.is_empty() {Geometry::points_only(vec![])?}
            else {super::packed::merge_all(results)?};
        for key in ["iteration","numiterations","value","source_index"] {
            match initial.detail.get(key) {
                Some(value)=>{out.detail.insert(key.into(),value.clone());},
                None=>{out.detail.remove(key);},
            }
        }
        out.validate()?;
        Ok(Arc::new(out))
    }
}

#[cfg(test)] mod tests {
    use super::*;
    use serde_json::json;
    fn foreach_graph() -> Graph {Graph::from_value(&json!({
        "display":"end","nodes":{
            "grid":{"type":"grid","params":{"columns":2,"rows":2}},
            "start":{"type":"for_each_start","inputs":{"geo":"grid"},"params":{"method":"domain","domain":"points"}},
            "stamp":{"type":"for_each_index","inputs":{"geo":"start","index":{"node":"start","output":1}}},
            "end":{"type":"for_each_end","inputs":{"generated":"stamp","begin":"start"}}
        },"zones":{"zone1":{"kind":"for_each","start":"start","end":"end","members":["stamp"]}}
    })).unwrap()}
    #[test] fn foreach_typed_index_graph_session_cache_and_selection() {
        let graph=foreach_graph();
        let fresh=graph.cook(None).unwrap();
        assert_eq!(fresh.npoints,4);
        assert_eq!(fresh.point["source_index"].as_i32().unwrap(),[0,1,2,3]);
        let mut session=crate::session::Session::new(graph);
        assert_eq!(session.cook(None).unwrap().point["source_index"].as_i32().unwrap(),[0,1,2,3]);
        session.cook(None).unwrap();
        assert!(session.last_stats().cooked.is_empty());
        session.set_param("start","selection",json!("1 3")).unwrap();
        assert_eq!(session.cook(None).unwrap().point["source_index"].as_i32().unwrap(),[1,3]);
        session.set_param("start","selection",json!("99")).unwrap();
        assert_eq!(session.cook(None).unwrap().npoints,0);
        session.set_param("start","selection",json!("")).unwrap();
        session.set_param("grid","size_x",json!(20.0)).unwrap();
        assert_eq!(session.cook(None).unwrap().npoints,4);
        assert!(session.last_stats().cooked.contains(&"end".into()));
    }
    #[test] fn foreach_nested_budget_and_socket_types() {
        let mut graph=foreach_graph().to_value();
        graph["nodes"].as_object_mut().unwrap().remove("stamp");
        graph["zones"]["zone1"]["members"]=json!(["inner_start","inner_stamp","inner_end"]);
        graph["nodes"]["inner_start"]=json!({"type":"for_each_start","inputs":{"geo":"start"},"params":{"method":"domain","domain":"points"}});
        graph["nodes"]["inner_stamp"]=json!({"type":"for_each_index","inputs":{"geo":"inner_start","index":{"node":"inner_start","output":1}}});
        graph["nodes"]["inner_end"]=json!({"type":"for_each_end","inputs":{"begin":"inner_start","generated":"inner_stamp"}});
        graph["nodes"]["end"]["inputs"]["generated"]=json!("inner_end");
        graph["zones"]["zone2"]=json!({"kind":"for_each","start":"inner_start","end":"inner_end","members":["inner_stamp"],"parent":"zone1"});
        let graph=Graph::from_value(&graph).unwrap();
        assert_eq!(graph.cook(None).unwrap().npoints,4);
        assert_eq!(crate::session::Session::new(graph).cook(None).unwrap().npoints,4);
        let graph=foreach_graph();
        let plan=Plan::analyze(&graph,"end").unwrap();
        let limit=budget();limit.set(1);
        assert!(plan.cook_end(&graph,&crate::channels::resolve_graph(&graph),"end",limit,&mut |src,_|graph.cook(Some(src))).unwrap_err().to_string().contains("budget exceeded"));
        let mut invalid=graph.to_value();
        invalid["nodes"]["stamp"]["inputs"]["geo"]=json!({"node":"start","output":1});
        assert!(Graph::from_value(&invalid).err().unwrap().to_string().contains("socket type"));
    }
    #[test] fn default_iteration_budget_matches_previous_value() {
        assert_eq!(budget().get(), 10_000);
    }
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
