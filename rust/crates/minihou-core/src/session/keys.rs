use super::*;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// The content key of `id`: validates what the cook would (unknown node or type, bad ports, cycles) with the same wording as
/// [`Graph::cook`].
pub(super) fn key_of(graph: &Graph, resolved: &crate::channels::ResolvedChannels, external_versions: &HashMap<String,u64>, id: &str, keys: &mut HashMap<String, u64>, active: &mut HashSet<String>) -> Result<u64> {
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
    let values = resolved.params.get(id).or(def.params.as_ref());
    for (port, _) in &def.inputs {
        if !spec.inputs.iter().any(|i| i.name == port) {
            return Err(Error::new(format!("node '{id}': '{}' has no input '{port}'", def.node_type)));
        }
    }
    let mut h = DefaultHasher::new();
    def.node_type.hash(&mut h);
    if def.node_type == "usd_export" {
        nodes::usd_nodes::exports_suppressed().hash(&mut h);
        let p = Params::resolve(spec.params, values)?;
        if let Ok(path) = nodes::usd_nodes::export_target(&p) {
            if let Ok(meta) = std::fs::metadata(path) { meta.len().hash(&mut h); meta.modified().ok().hash(&mut h); }
        }
    }
    if def.node_type == "usd_import" || def.node_type == "image_file" {
        let p = Params::resolve(spec.params, values)?;
        let path = crate::usd_io::resolve_path(&p.s("file")?);
        path.hash(&mut h);
        if let Ok(meta) = std::fs::metadata(path) { meta.len().hash(&mut h); meta.modified().ok().hash(&mut h); }
    }
    if def.node_type == "python" {
        let p = Params::resolve(spec.params, values)?;
        crate::python_bridge::key_material(&p.s("code")?).hash(&mut h);
    }
    if def.node_type == "copy_to_points" {
        nodes::copy_limit_bytes().hash(&mut h);
    }
    let binding=match def.node_type.as_str() {
        "blender_object" => Some(format!("object:{}",values.and_then(|p|p.get("object")).and_then(Value::as_str).unwrap_or(""))),
        "blender_image" => Some(format!("image:{}",values.and_then(|p|p.get("image")).and_then(Value::as_str).unwrap_or(""))),
        "object_input" => Some(nodes::blender_input::binding_key(id, &Params::resolve(spec.params,values)?)?),
        "modifier_input" => Some("modifier_input".into()), _ => None,
    };
    if let Some(binding)=binding { external_versions.get(&binding).hash(&mut h); }
    if def.node_type == "blender_object" { external_versions.get(&nodes::blender_input::picker_external(id)).hash(&mut h); }
    def.bypass.hash(&mut h);
    // time-dependent nodes (Python's key_extra = timectx.key()) recook when the frame changes
    if def.node_type == "cache" || nodes::is_time_dependent(&def.node_type, def.params.as_ref()) || resolved.time_dependent.contains(id) {
        crate::timectx::key().hash(&mut h);
    }
    match values {
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
        for (index, r) in refs.iter().enumerate() {
            key_of(graph, resolved, external_versions, r, keys, active)?.hash(&mut h);
            def.output_for(port.name, index, r).output.hash(&mut h);
        }
    }
    active.remove(id);
    let key = h.finish();
    keys.insert(id.to_string(), key);
    Ok(key)
}

/// Stable across timeline frames; changes when upstream graph structure or authored values change.
pub(super) fn upstream_recipe(graph: &Graph, id: &str, memo: &mut HashMap<String, u64>) -> u64 {
    if let Some(hash) = memo.get(id) { return *hash; }
    let mut h = DefaultHasher::new();
    if let Some(def) = graph.node(id) {
        for (port, sources) in &def.inputs {
            port.hash(&mut h);
            for (index, source) in sources.iter().enumerate() {
                source.hash(&mut h);
                def.output_for(port, index, source).output.hash(&mut h);
                if source != id { upstream_recipe(graph, source, memo).hash(&mut h); }
            }
        }
        def.node_type.hash(&mut h);
        def.bypass.hash(&mut h);
        for values in [&def.params, &def.channels] {
            if let Some(values) = values { let mut canonical_values = String::new(); canonical(&Value::Object(values.clone()), &mut canonical_values); canonical_values.hash(&mut h); }
        }
    }
    let hash = h.finish();
    memo.insert(id.to_owned(), hash);
    hash
}

pub(super) fn input_hash(def: &crate::graph::NodeDef, keys: &HashMap<String, u64>) -> u64 {
    let mut h = DefaultHasher::new();
    for (port, refs) in &def.inputs {
        port.hash(&mut h);
        for (index, source) in refs.iter().enumerate() {
            keys[source].hash(&mut h);
            def.output_for(port, index, source).output.hash(&mut h);
        }
    }
    h.finish()
}

/// JSON with object keys sorted, so equal parameters hash equal whatever order they were written in.
pub(super) fn canonical(v: &Value, out: &mut String) {
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

