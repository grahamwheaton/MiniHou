use crate::{display_nodes, http_geometry, link_protocol, modifier_inputs, Document, Edit, Result};
use minihou_core::{lod::ClusterLod, session::{CookStats, Session}, Geometry, Graph};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Instant;

pub struct CookRequest { pub revision: u64, pub edit: Edit }
pub struct CookResponse<T> {
    pub revision: u64,
    pub extras: std::collections::BTreeMap<String,Arc<Geometry>>,
    pub scene: Vec<(Value,link_protocol::Arrays)>,
    pub geometry: Option<Arc<Geometry>>,
    pub preview: Option<(String, Arc<Geometry>, Option<Arc<Geometry>>)>,
    pub lod: Option<Arc<ClusterLod>>,
    pub prepared: Option<T>,
    /// Display output ranges followed by subnet-expanded ranges (point indices).
    pub display_ranges: Vec<(String, std::ops::Range<u32>)>,
    pub output_range_count: usize,
    pub stats: CookStats,
    pub error: Option<String>,
    pub milliseconds: f64,
}
/// Apply every queued edit in order, then cook just the final state.
/// This preserves independent edits while the last edit of any parameter wins.
pub fn cook_batch<T>(session: &mut Session, target: &mut String, requests: Vec<CookRequest>, prepare: impl FnOnce(&Geometry) -> T) -> CookResponse<T> {
    cook_batch_preview(session, target, requests, None, prepare)
}
fn cook_batch_preview<T>(session: &mut Session, target: &mut String, requests: Vec<CookRequest>, preview_node: Option<&str>, prepare: impl FnOnce(&Geometry) -> T) -> CookResponse<T> {
    let revision = requests.last().map_or(0, |r| r.revision);
    let start = Instant::now();
    let mut error = None;
    for request in requests {
        let result: Result<()> = (|| {
            match request.edit {
                Edit::Param { node, name, value } => session.set_param(&node, &name, value)?,
                Edit::Channel { node, path, expression } => session.set_channel(&node,&path,expression)?,
                Edit::ChannelGroup { node, name, value, channels } => {
                    if let Some(value)=value {session.set_param(&node,&name,value)?;}
                    for (path,expression) in channels {session.set_channel(&node,&path,expression)?;}
                }
                Edit::LockParam { node, path, locked } => session.set_locked_param(&node,&path,locked)?,
                Edit::Bypass { node, flag } => session.set_bypass(&node, flag)?,
                Edit::Display(node) => *target = node,
                Edit::Preview(_) => {},
                Edit::External { name, input } => session.set_external(name,input),
                Edit::Time { frame, fps } => session.set_time(frame, fps)?,
                Edit::Exposed(_) => {}, // Interface metadata does not change the core graph.
                Edit::ModifierInputs { node, inputs } => {
                    let mut value = session.graph().to_value();
                    value["nodes"][&node]["modifier_inputs"] = json!(inputs);
                    session.replace_graph(Graph::from_value(&value)?);
                }
                Edit::ModifierInputValue { node, id, value } => {
                    let mut graph = session.graph().to_value();
                    modifier_inputs::set_value(&mut graph, &node, &id, value)?;
                    session.replace_graph(Graph::from_value(&graph)?);
                }
                Edit::Replace(value) => {
                    let mut graph = Graph::from_value(&value)?;
                    let displayed = display_nodes(&value);
                    *target = if displayed.len() > 1 {
                        let id = (0..).map(|i| format!("__display_merge_{i}")).find(|id| !graph.nodes.iter().any(|(name, _)| name == id)).unwrap();
                        graph.nodes.push((id.clone(), minihou_core::graph::NodeDef { node_type: "merge".into(), inputs: vec![("geos".into(), displayed)], endpoints: Vec::new(), params: None, channels: None, locked_params: None, bypass: false }));
                        id
                    } else if graph.nodes.is_empty() { String::new() }
                    else { graph.main_output()? };
                    session.replace_graph(graph);
                }
            }
            Ok(())
        })();
        if let Err(e) = result { error = Some(e.to_string()); }
    }
    let geometry = if session.graph().nodes.is_empty() {
        // Removing the final node clears the viewport; there is no node to cook.
        Some(Arc::new(Geometry::points_only(Vec::new()).expect("empty geometry")))
    } else {
        match session.cook(Some(target)) {
            Ok(g) => Some(g),
            Err(e) => { error = Some(e.to_string()); None }
        }
    };
    let preview = preview_node.and_then(|id| {
        if !session.graph().nodes.iter().any(|(name, _)| name == id) { return None; }
        let source = session.graph().nodes.iter().find(|(name, _)| name == id)
            .and_then(|(_, def)| def.inputs.iter().find(|(port, _)| port == "geo")
                .or_else(|| def.inputs.first()))
            .and_then(|(_, ids)| ids.first()).cloned();
        match session.cook(Some(id)) {
            Ok(g) => {
                let input = source.and_then(|source| session.cook(Some(&source)).ok());
                Some((id.to_owned(), g, input))
            },
            Err(_) => None,
        }
    });
    let lod = if geometry.is_some() {
        // Breadth first finds the closest bake to the displayed node, including itself.
        let mut queue = std::collections::VecDeque::from([target.clone()]);
        let mut seen = std::collections::HashSet::new();
        let mut found = None;
        while let Some(id) = queue.pop_front() {
            if !seen.insert(id.clone()) { continue; }
            if let Some(def) = session.graph().nodes.iter().find(|(name, _)| name == &id).map(|(_, def)| def) {
                if def.node_type == "lod_bake" {
                    if !def.bypass {
                        if let (Some(source), Some(displayed)) = (session.cached_geometry(&id), geometry.as_ref()) {
                            let same_points = source.point.get("P").zip(displayed.point.get("P"))
                                .is_some_and(|(a,b)| Arc::ptr_eq(a,b));
                            if same_points && Arc::ptr_eq(&source.topology,&displayed.topology) {
                                found = session.side_payload::<ClusterLod>(&id);
                            }
                        }
                    }
                    break;
                }
                for (_, refs) in &def.inputs { queue.extend(refs.iter().cloned()); }
            }
        }
        found
    } else { None };
    let mut stats=session.last_stats().clone();
    let mut extras=std::collections::BTreeMap::new();
    let mut scene=Vec::new();
    let extra_targets=session.graph().extra_outputs();
    let sinks:Vec<_>=session.graph().nodes.iter().filter(|(_,d)|["scene_objects","image_out"].contains(&d.node_type.as_str()))
        .map(|(id,d)|(id.clone(),d.node_type.clone(),d.params.clone(),d.bypass)).collect();
    for (name,id) in extra_targets {
        match session.cook(Some(&id)) { Ok(g)=>{extras.insert(name,g);}, Err(e)=>error=Some(e.to_string()) }
        append_cook_stats(&mut stats,session.last_stats());
    }
    for (id,typ,params,bypass) in sinks {
        // A bypassed sink still passes geometry through but has no scene side effect.
        if bypass {continue;}
        let result:Result<(Value,link_protocol::Arrays)>=(|| {
            let g=session.cook(Some(&id))?;
            let p=minihou_core::params::Params::resolve(minihou_core::nodes::get(&typ).unwrap().params,params.as_ref())?;
            link_protocol::sink_manifest(&typ,&id,&p,&g,&revision.to_string())
        })();
        append_cook_stats(&mut stats,session.last_stats());
        match result {Ok(manifest)=>scene.push(manifest),Err(e)=>error=Some(e.to_string())}
    }
    let milliseconds = start.elapsed().as_secs_f64() * 1000.;
    // The synthetic display merge concatenates points in input order. Keep this
    // ownership alongside the cooked revision for viewport picking and outlines.
    let outputs = session.graph().nodes.iter().find(|(id, _)| id.as_str() == target.as_str())
        .filter(|(id, _)| id.starts_with("__display_merge_"))
        .map(|(_, def)| def.inputs.iter().flat_map(|(_, ids)| ids.iter().cloned()).collect::<Vec<_>>())
        .unwrap_or_else(|| vec![target.clone()]);
    let expanded = session.graph().to_value()["nodes"].as_object()
        .and_then(|nodes| minihou_core::graph::expand_subnets(nodes).ok().flatten());
    let expanded_counts = expanded.as_ref().and_then(|_| session.expanded_cached_point_counts());
    let mut offset = 0;
    let mut display_ranges: Vec<_> = outputs.into_iter().filter_map(|id| {
        let count = session.cached_geometry(&id).map(|geometry| geometry.npoints as u32)
            .or_else(|| expanded_counts.as_ref()?.get(&id).copied())?;
        let range = offset..offset + count;
        offset += count;
        Some((id, range))
    }).collect();
    let output_range_count = display_ranges.len();
    if let (Some(expanded), Some(counts)) = (expanded, expanded_counts) {
        let roots = display_ranges.clone();
        for (id, range) in roots { trace_expanded_ranges(&expanded, &counts, &id, range, &mut display_ranges, 0); }
    }
    let prepared = geometry.as_ref().map(|g| prepare(g));
    CookResponse { revision, geometry, preview, lod, prepared, display_ranges, output_range_count, extras, scene, stats, error, milliseconds }
}

fn trace_expanded_ranges(nodes: &serde_json::Map<String, Value>, counts: &std::collections::HashMap<String, u32>,
    id: &str, range: std::ops::Range<u32>, out: &mut Vec<(String, std::ops::Range<u32>)>, depth: usize) {
    if depth > 64 || range.is_empty() { return; }
    let Some(node) = nodes.get(id) else { return; };
    if id.contains('/') && !id.rsplit('/').next().unwrap_or_default().starts_with('$') {
        out.push((id.to_owned(), range.clone()));
    }
    fn source(value: &Value) -> Option<&str> {
        if value["output"].as_u64().unwrap_or(0) != 0 { return None; }
        value.as_str().or_else(|| value["node"].as_str())
    }
    if node["type"] == "merge" {
        let Some(inputs) = node["inputs"]["geos"].as_array() else { return; };
        let parts: Vec<_> = inputs.iter().filter_map(|input| {
            let id = source(input)?;
            Some((id, *counts.get(id)?))
        }).collect();
        if parts.len() != inputs.len() || parts.iter().map(|(_, count)| count).sum::<u32>() != range.len() as u32 { return; }
        let mut start = range.start;
        for (child, count) in parts {
            trace_expanded_ranges(nodes, counts, child, start..start + count, out, depth + 1);
            start += count;
        }
    } else if matches!(node["type"].as_str(), Some("output" | "subnet_output" | "subnet" | "transform" | "normal" | "null")) {
        if let Some(child) = source(&node["inputs"]["geo"]) {
            if counts.get(child).copied() == Some(range.len() as u32) {
                trace_expanded_ranges(nodes, counts, child, range, out, depth + 1);
            }
        }
    }
}

fn append_cook_stats(out:&mut CookStats, more:&CookStats) {
    for id in &more.cooked {if !out.cooked.contains(id){out.cooked.push(id.clone());}}
    for id in &more.cached {if !out.cached.contains(id)&&!out.cooked.contains(id){out.cached.push(id.clone());}}
    out.node_ms.extend(more.node_ms.iter().cloned());
    for diag in &more.param_diags {if !out.param_diags.contains(diag){out.param_diags.push(diag.clone());}}
    for w in &more.waiting {if !out.waiting.contains(w){out.waiting.push(w.clone());}}
}

#[derive(Default)]
pub struct CookState {
    pub requested: u64,
    pub completed: u64,
    pub geometry: Option<Arc<Geometry>>,
    pub preview: Option<(String, Arc<Geometry>, Option<Arc<Geometry>>)>,
    pub lod: Option<Arc<ClusterLod>>,
    pub stats: CookStats,
    pub error: Option<String>,
    pub milliseconds: f64,
}
impl CookState {
    pub fn cooking(&self) -> bool { self.completed < self.requested }
    pub fn accept<T>(&mut self, response: CookResponse<T>) -> Option<T> {
        if response.revision != self.requested { return None; }
        self.completed = response.revision;
        self.error = response.error;
        self.stats = response.stats;
        self.milliseconds = response.milliseconds;
        self.lod = response.lod;
        self.preview = response.preview;
        if let Some(g) = response.geometry { self.geometry = Some(g); }
        response.prepared
    }
}
/// One result slot bounds memory even if the UI is occupied by a native dialog.
/// Dropping the sender shuts the worker down after its current cook; never joins on the UI.
pub struct CookWorker<T> {
    sender: mpsc::Sender<CookRequest>,
    queries: mpsc::Sender<http_geometry::Query>,
    latest: Arc<Mutex<Option<CookResponse<T>>>>,
    /// Newest revision sent; the worker abandons a cook once a newer one is queued.
    sent: Arc<std::sync::atomic::AtomicU64>,
}
/// A short quiet period folds a drag's frame events into one cook. Playback's
/// 24 fps ticks are farther apart, so each displayed frame still gets a cook.
fn collect_cook_batch(receiver: &mpsc::Receiver<CookRequest>, first: CookRequest) -> Vec<CookRequest> {
    let mut batch = vec![first];
    batch.extend(receiver.try_iter());
    while matches!(batch.last().map(|r| &r.edit), Some(Edit::Time { .. })) {
        match receiver.recv_timeout(std::time::Duration::from_millis(16)) {
            Ok(next) => { batch.push(next); batch.extend(receiver.try_iter()); }
            Err(_) => break,
        }
    }
    batch
}
impl<T: Send + 'static> CookWorker<T> {
    pub fn start(doc: &Document, prepare: impl Fn(&Geometry) -> T + Send + 'static) -> Result<Self> {
        Self::start_with_python(doc, None, prepare)
    }
    /// `python`: the desktop UI's permission gate. None (as for every remote or headless cook) denies python nodes.
    pub fn start_with_python(doc: &Document, python: Option<Arc<minihou_core::python_bridge::PythonGate>>, prepare: impl Fn(&Geometry) -> T + Send + 'static) -> Result<Self> {
        let mut session = doc.session()?;
        session.set_python_gate(python);
        let mut target = session.graph().main_output()?;
        let (sender, receiver) = mpsc::channel();
        let (queries, query_rx) = mpsc::channel();
        let latest = Arc::new(Mutex::new(None));
        let output = latest.clone();
        let sent = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let sent_worker = sent.clone();
        std::thread::Builder::new().name("minihou-cook".into()).spawn(move || {
            let mut preview_node: Option<String> = None;
            loop {
                match receiver.recv_timeout(std::time::Duration::from_millis(10)) {
                    Ok(first) => {
                        let batch = collect_cook_batch(&receiver, first);
                        for request in &batch { if let Edit::Preview(node) = &request.edit { preview_node = Some(node.clone()); } }
                        // A newer request abandons this cook between nodes; finished nodes stay cached for it.
                        let revision = batch.last().map_or(0, |r| r.revision);
                        let newest = sent_worker.clone();
                        session.set_cancel(Some(Arc::new(move || newest.load(std::sync::atomic::Ordering::Relaxed) > revision)));
                        let response = cook_batch_preview(&mut session, &mut target, batch, preview_node.as_deref(), &prepare);
                        session.set_cancel(None);
                        *output.lock().unwrap() = Some(response);
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(mpsc::RecvTimeoutError::Timeout) => {},
                }
                for query in query_rx.try_iter() { http_geometry::run(&mut session, &target, query); }
            }
        })?;
        Ok(Self { sender, queries, latest, sent })
    }
    pub fn send(&self, request: CookRequest) -> Result<()> {
        self.sent.fetch_max(request.revision, std::sync::atomic::Ordering::Relaxed);
        self.sender.send(request).map_err(|_| "cook worker stopped".into()) }
    pub fn poll(&self) -> Option<CookResponse<T>> { self.latest.lock().unwrap().take() }
    pub fn query(&self, query: http_geometry::Query) -> Result<()> { self.queries.send(query).map_err(|_| "cook worker stopped".into()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subnet_merge_exposes_inner_display_ranges_for_picking() {
        let value = json!({"display":"sub","nodes":{"sub":{"type":"subnet",
            "input_sockets":[{"index":0,"name":"Input 1"}],
            "output_sockets":[{"index":0,"name":"Output 1"}],
            "inner":{"output":"output1","nodes":{
                "a":{"type":"box"},"b":{"type":"box"},
                "merge1":{"type":"merge","inputs":{"geos":["a","b"]}},
                "output1":{"type":"subnet_output","port_index":0,"inputs":{"geo":"merge1"}}
            }}}}});
        let mut session = Document::parse(value.to_string()).unwrap().session().unwrap();
        let mut target = "sub".to_owned();
        let response = cook_batch(&mut session, &mut target,
            vec![CookRequest { revision: 1, edit: Edit::Replace(value) }], |_| ());
        assert!(response.error.is_none(), "{:?}", response.error);
        assert_eq!(response.output_range_count, 1);
        let a = response.display_ranges.iter().find(|(id, _)| id == "sub/a").unwrap().1.clone();
        let b = response.display_ranges.iter().find(|(id, _)| id == "sub/b").unwrap().1.clone();
        assert_eq!(a.start, 0);
        assert_eq!(a.end, b.start);
        assert_eq!(b.end, response.display_ranges[0].1.end);
    }

    #[test]
    fn removing_last_node_cooks_empty_viewport_without_error() {
        let doc = Document::parse(json!({"nodes":{"box1":{"type":"box"}},"display":"box1"}).to_string()).unwrap();
        let mut session = doc.session().unwrap();
        let mut target = "box1".to_owned();
        let response = cook_batch(&mut session, &mut target,
            vec![CookRequest { revision: 1, edit: Edit::Replace(json!({"nodes":{},"display":null})) }],
            |g| g.npoints);
        assert_eq!(target, "");
        assert_eq!(response.prepared, Some(0));
        assert_eq!(response.geometry.unwrap().npoints, 0);
        assert!(response.display_ranges.is_empty());
        assert!(response.error.is_none(), "{:?}", response.error);
    }

    /// A drag of one parameter re-cooks only that node and what is downstream of it, once per step; the upstream node stays cached.
    #[test]
    fn dragging_one_parameter_cooks_the_edited_node_and_downstream_once_per_step() {
        let value = json!({"nodes": {
            "a": {"type": "box"},
            "b": {"type": "transform", "inputs": {"geo": "a"}, "params": {"translate": [0., 0., 0.]}},
            "c": {"type": "transform", "inputs": {"geo": "b"}, "params": {"translate": [0., 1., 0.]}},
            "other": {"type": "box"}}, "display": "c"});
        let doc = Document::parse(value.to_string()).unwrap();
        let mut session = doc.session().unwrap();
        let mut target = session.graph().main_output().unwrap();
        let first = cook_batch(&mut session, &mut target, vec![CookRequest { revision: 1, edit: Edit::Replace(value.clone()) }], |_| ());
        assert!(first.error.is_none(), "{:?}", first.error);
        for step in 1..=5u64 {
            let edit = Edit::Param { node: "b".into(), name: "translate".into(), value: json!([step as f64, 0., 0.]) };
            let r = cook_batch(&mut session, &mut target, vec![CookRequest { revision: step + 1, edit }], |_| ());
            assert!(r.error.is_none(), "{:?}", r.error);
            let mut cooked = r.stats.cooked.clone(); cooked.sort();
            assert_eq!(cooked, vec!["b".to_string(), "c".to_string()], "step {step}");
        }
        // Several queued steps fold into a single cook of the same two nodes.
        let batch = (6..=10u64).map(|step| CookRequest { revision: step, edit: Edit::Param { node: "b".into(), name: "translate".into(), value: json!([step as f64, 0., 0.]) } }).collect();
        let r = cook_batch(&mut session, &mut target, batch, |_| ());
        let mut cooked = r.stats.cooked.clone(); cooked.sort();
        assert_eq!(cooked, vec!["b".to_string(), "c".to_string()]);
    }

    #[test]
    fn selected_group_preview_keeps_its_own_geometry_across_display_changes() {
        let value=json!({"nodes":{
            "box":{"type":"box"},
            "group":{"type":"group_create","inputs":{"geo":"box"},"params":{"domain":"edges","name":"rim","pattern":"0"}},
            "other":{"type":"box"}},"display":"other"});
        let mut session=Document::parse(value.to_string()).unwrap().session().unwrap();
        let mut target="other".to_owned();
        let r=cook_batch_preview(&mut session,&mut target,vec![CookRequest{revision:1,edit:Edit::Replace(value)}],Some("group"),|_|());
        let (id,output,input)=r.preview.unwrap();
        assert_eq!(id,"group");
        assert!(output.edge.contains_key("group_rim") || output.vertex.contains_key("edge_group_rim") || output.edge.contains_key("edge_group_rim"));
        assert_eq!(input.unwrap().npoints,output.npoints);
        assert_eq!(r.display_ranges[0].0,"other");
    }

    #[test]
    fn cooked_display_ownership_survives_parameter_edits_and_display_changes() {
        let value = json!({"nodes": {"a":{"type":"box"}, "b":{"type":"box"}}, "display":["b","a"]});
        let doc = Document::parse(value.to_string()).unwrap();
        let mut session = doc.session().unwrap();
        let mut target = session.graph().main_output().unwrap();
        let result = cook_batch(&mut session, &mut target,
            vec![CookRequest { revision: 1, edit: Edit::Replace(value) }], |_| ());
        assert!(result.error.is_none(), "{:?}", result.error);
        let count = session.cached_geometry("b").unwrap().npoints as u32;
        assert!(count > 0);
        assert_eq!(result.display_ranges, vec![("b".into(), 0..count), ("a".into(), count..2*count)]);
        let result = cook_batch(&mut session, &mut target,
            vec![CookRequest { revision: 2, edit: Edit::Param { node: "a".into(), name: "size".into(), value: json!([2.,3.,4.]) } }], |_| ());
        assert!(result.error.is_none(), "{:?}", result.error);
        assert_eq!(result.display_ranges[1], ("a".into(), count..2*count));
        let result = cook_batch(&mut session, &mut target,
            vec![CookRequest { revision: 3, edit: Edit::Display("a".into()) }], |_| ());
        assert!(result.error.is_none(), "{:?}", result.error);
        assert_eq!(result.display_ranges, vec![("a".into(), 0..count)]);
    }

    #[test]
    fn channel_group_keeps_unrelated_cook_cache() {
        let doc=Document::parse(json!({"display":"b","nodes":{"a":{"type":"box"},"b":{"type":"box"}}}).to_string()).unwrap();
        let mut session=doc.session().unwrap();
        session.cook(Some("a")).unwrap();
        let mut target="b".to_string();
        let edit=Edit::ChannelGroup{node:"b".into(),name:"size".into(),value:None,channels:vec![("size.x".into(),Some("ch('../a/size.x')".into()))]};
        let response=cook_batch(&mut session,&mut target,vec![CookRequest{revision:1,edit}],|_|());
        assert!(response.error.is_none(),"{:?}",response.error);
        session.cook(Some("a")).unwrap();
        assert!(session.last_stats().cached.contains(&"a".to_string()));
    }

    #[test]
    fn cook_ui_only_and_undo_edits_cook_nothing() {
        let value=json!({"display":"t","nodes":{"g":{"type":"grid"},"t":{"type":"transform","inputs":{"geo":"g"}}}});
        let doc=Document::parse(value.to_string()).unwrap();
        let mut session=doc.session().unwrap();
        let mut target="t".to_string();
        let first=cook_batch(&mut session,&mut target,vec![CookRequest{revision:1,edit:Edit::Time{frame:1.,fps:24.}}],|_|());
        assert_eq!(first.stats.cooked.len(),2);
        for (revision,edit) in [(2,Edit::Exposed(vec![])),(3,Edit::Replace(value.clone())),(4,Edit::Display("t".into()))] {
            let r=cook_batch(&mut session,&mut target,vec![CookRequest{revision,edit}],|_|());
            assert!(r.error.is_none()&&r.stats.cooked.is_empty(),"{:?}",r.stats);
        }
    }

    #[test]
    fn cook_response_exposes_broken_channel_diagnostic() {
        let doc=Document::parse(json!({"nodes":{"out":{"type":"box","params":{"size":[1,2,3]},"channels":{"size.y":"ch('../gone/size.y')"}}},"display":"out"}).to_string()).unwrap();
        let mut session=doc.session().unwrap();
        let mut target="out".to_string();
        let response=cook_batch(&mut session,&mut target,vec![CookRequest{revision:1,edit:Edit::Time{frame:1.,fps:24.}}],|_|());
        assert!(response.error.is_none(),"{:?}",response.error);
        assert!(response.stats.param_diags.iter().any(|d|d.node=="out"&&d.param=="size.y"&&d.kind==minihou_core::channels::ParamDiagKind::BadReference));
    }
}
