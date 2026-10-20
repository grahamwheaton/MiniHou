use super::*;
use super::keys::{upstream_recipe, input_hash};
use crate::graph::run_node_outputs;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::Instant;

pub(super) struct Run<'a> {
    pub(super) cancel: Option<&'a (dyn Fn() -> bool + Send + Sync)>,
    pub(super) graph: &'a Graph,
    pub(super) resolved: &'a crate::channels::ResolvedChannels,
    pub(super) plan: &'a nodes::r#loop::Plan,
    pub(super) budget: nodes::r#loop::Budget,
    pub(super) external: &'a nodes::blender_input::ExternalInputs,
    pub(super) external_revision: u64,
    pub(super) cache: &'a mut HashMap<u64, Entry>,
    pub(super) frames: &'a mut HashMap<(String, u64, u64), FrameEntry>,
    pub(super) frame_recipes: &'a mut HashMap<String, u64>,
    pub(super) frame_bytes: &'a mut usize,
    pub(super) bytes: &'a mut usize,
    pub(super) clock: u64,
    pub(super) frame: u64,
    pub(super) keys: &'a HashMap<String, u64>,
    pub(super) stats: &'a mut CookStats,
}

impl Run<'_> {
    pub(super) fn get(&mut self, id: &str) -> Result<Arc<Geometry>> {
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

