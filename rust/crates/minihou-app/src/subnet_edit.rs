//! Subnet creation, grouping, and ungrouping for the network editor.
//! Inner external reads are "$in0", "$in1" ... (the order of the subnet's `geo` wires); see `minihou_core::graph::expand_subnets`.
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;

fn refs(node: &Value) -> Vec<String> {
    node.get("inputs").and_then(Value::as_object).into_iter().flat_map(|m| m.values()).flat_map(|v| match v {
        Value::String(s) => vec![s.clone()],
        Value::Object(o) => o.get("node").and_then(Value::as_str).map(str::to_owned).into_iter().collect(),
        Value::Array(a) => a.iter().filter_map(|x| x.as_str().or_else(|| x["node"].as_str()).map(str::to_owned)).collect(),
        _ => vec![],
    }).collect()
}

fn map_refs(node: &mut Value, f: &mut dyn FnMut(&str) -> String) {
    let Some(inputs) = node.get_mut("inputs").and_then(Value::as_object_mut) else { return };
    for v in inputs.values_mut() {
        match v {
            Value::String(s) => *s = f(s),
            Value::Object(o) => if let Some(s) = o.get("node").and_then(Value::as_str) {
                o.insert("node".into(), json!(f(s)));
            },
            Value::Array(a) => for x in a {
                if let Some(s) = x.as_str() { *x = json!(f(s)); }
                else if let Some(o) = x.as_object_mut() {
                    if let Some(s) = o.get("node").and_then(Value::as_str) { o.insert("node".into(), json!(f(s))); }
                }
            },
            _ => {}
        }
    }
}

fn map_endpoints(node: &mut Value, f: &mut dyn FnMut(&Value) -> Value) {
    let Some(inputs) = node.get_mut("inputs").and_then(Value::as_object_mut) else { return };
    for value in inputs.values_mut() {
        if let Value::Array(refs) = value { for endpoint in refs { *endpoint = f(endpoint); } }
        else { *value = f(value); }
    }
}

fn restored_endpoint(endpoint: &Value, inner: &Map<String, Value>, ext: &[Option<Value>],
    rename: &std::collections::HashMap<String, String>) -> Value {
    let source = endpoint.as_str().or_else(|| endpoint["node"].as_str());
    let Some(source) = source else { return endpoint.clone() };
    if let Some(index) = inner.get(source).filter(|node| node["type"] == "subnet_input")
        .and_then(|node| node["port_index"].as_u64()).map(|i| i as usize)
        .or_else(|| source.strip_prefix("$in").and_then(|i| i.parse::<usize>().ok())) {
        return ext.get(index).and_then(Clone::clone).unwrap_or(Value::Null);
    }
    if let Some(new) = rename.get(source) {
        if endpoint.is_string() { return json!(new); }
        let mut changed = endpoint.clone();
        changed["node"] = json!(new);
        return changed;
    }
    endpoint.clone()
}

fn fresh(nodes: &Map<String, Value>, ty: &str) -> String {
    (1..).map(|n| format!("{ty}{n}")).find(|id| !nodes.contains_key(id)).unwrap()
}

fn depends_on(nodes: &Map<String, Value>, start: &str, sought: &str) -> bool {
    let mut pending = vec![start.to_owned()];
    let mut seen = BTreeSet::new();
    while let Some(id) = pending.pop() {
        if id == sought { return true; }
        if seen.insert(id.clone()) {
            if let Some(node) = nodes.get(&id) { pending.extend(refs(node)); }
        }
    }
    false
}

fn has_output(nodes: &Map<String, Value>, source: &str, output: usize) -> bool {
    let Some(node) = nodes.get(source) else { return false };
    let Some(ty) = node["type"].as_str() else { return false };
    minihou_core::nodes::get(ty).is_some_and(|spec| output < spec.outputs.len())
        || ty == "subnet" && node["output_sockets"].as_array().is_some_and(|sockets|
            sockets.iter().any(|socket| socket["index"].as_u64() == Some(output as u64)))
}

fn is_output_boundary(node: &Value) -> bool {
    node["type"] == "subnet_output" || node["type"] == "output" && node["port_index"].as_u64().is_some()
}

/// Give a newly added subnet a visible pass-through pair. Existing inner graphs are preserved.
pub fn initialize(value: &mut Value, id: &str) {
    let node = &mut value["nodes"][id];
    if node["type"] != "subnet" || node["inner"]["nodes"].is_object() { return; }
    let source = match &node["inputs"]["geo"] {
        Value::Array(refs) => refs.first().cloned().unwrap_or(Value::Null),
        source => source.clone(),
    };
    node["input_sockets"] = json!([{"index": 0, "name": "Input 1"}]);
    node["output_sockets"] = json!([{"index": 0, "name": "Output 1"}]);
    if !source.is_null() {
        node["inputs"] = json!({"in0": source});
    }
    node["inner"] = json!({"nodes": {
        "input1": {"type": "subnet_input", "port_index": 0, "pos": [0.0, 0.0]},
        "output1": {"type": "subnet_output", "port_index": 0, "inputs": {"geo": "input1"}, "pos": [0.0, 120.0]}
    }, "output": "output1"});
}

fn ensure_sockets(sub: &mut Value) -> Option<()> {
    if !sub["input_sockets"].is_array() {
        let sources = match &sub["inputs"]["geo"] {
            Value::String(_) | Value::Object(_) => vec![sub["inputs"]["geo"].clone()],
            Value::Array(refs) => refs.clone(),
            _ => Vec::new(),
        };
        let count = sources.len().max(1);
        let inner = sub["inner"]["nodes"].as_object_mut()?;
        let mut boundaries = Vec::new();
        for index in 0..count {
            let boundary = fresh(inner, "input");
            inner.insert(boundary.clone(), json!({"type": "subnet_input", "port_index": index,
                "pos": [-220.0, index as f64 * 96.0]}));
            boundaries.push(boundary);
        }
        for node in inner.values_mut() {
            map_refs(node, &mut |r| r.strip_prefix("$in").and_then(|n| n.parse::<usize>().ok())
                .and_then(|n| boundaries.get(n)).cloned().unwrap_or_else(|| r.to_owned()));
        }
        sub["input_sockets"] = Value::Array((0..count).map(|index|
            json!({"index": index, "name": format!("Input {}", index + 1)})).collect());
        sub["inputs"] = Value::Object(sources.into_iter().enumerate().map(|(index, source)|
            (format!("in{index}"), source)).collect());
    }
    if !sub["output_sockets"].is_array() {
        let output = sub["inner"]["output"].as_str()?.to_owned();
        let inner = sub["inner"]["nodes"].as_object_mut()?;
        if is_output_boundary(inner.get(&output)?) {
            inner.get_mut(&output)?["port_index"] = json!(0);
        } else {
            let boundary = fresh(inner, "output");
            inner.insert(boundary.clone(), json!({"type": "subnet_output", "port_index": 0,
                "inputs": {"geo": output}, "pos": [220.0, 0.0]}));
            sub["inner"]["output"] = json!(boundary);
        }
        sub["output_sockets"] = json!([{"index": 0, "name": "Output 1"}]);
    }
    Some(())
}

/// A noodle from a source node creates a connected input and its inner boundary.
pub fn add_input(value: &Value, id: &str, source: &str) -> Option<Value> {
    add_input_from(value, id, source, 0)
}

pub fn add_input_from(value: &Value, id: &str, source: &str, output: usize) -> Option<Value> {
    let mut next = value.clone();
    let nodes = next.get_mut("nodes")?.as_object_mut()?;
    if id == source || !has_output(nodes, source, output) || nodes[source]["type"] == "output"
        || depends_on(nodes, source, id) { return None; }
    let sub = nodes.get_mut(id).filter(|n| n["type"] == "subnet")?;
    ensure_sockets(sub)?;
    let index = sub["input_sockets"].as_array()?.iter().filter_map(|s| s["index"].as_u64()).max().map_or(0, |i| i + 1);
    let inner = sub["inner"]["nodes"].as_object_mut()?;
    let boundary = fresh(inner, "input");
    inner.insert(boundary, json!({"type": "subnet_input", "port_index": index,
        "pos": [-220.0, index as f64 * 96.0]}));
    sub["input_sockets"].as_array_mut()?.push(json!({"index": index, "name": format!("Input {}", index + 1)}));
    sub["inputs"][format!("in{index}")] = if output == 0 { json!(source) }
        else { json!({"node": source, "output": output}) };
    Some(next)
}

pub fn connect_input(value: &Value, id: &str, port: &str, source: &str) -> Option<Value> {
    connect_input_from(value, id, port, source, 0)
}

pub fn connect_input_from(value: &Value, id: &str, port: &str, source: &str, output: usize) -> Option<Value> {
    let index = port.strip_prefix("in")?.parse::<u64>().ok()?;
    let mut next = value.clone();
    let nodes = next.get_mut("nodes")?.as_object_mut()?;
    if id == source || !has_output(nodes, source, output) || nodes[source]["type"] == "output"
        || depends_on(nodes, source, id) { return None; }
    let sub = nodes.get_mut(id).filter(|n| n["type"] == "subnet")?;
    ensure_sockets(sub)?;
    if !sub["input_sockets"].as_array()?.iter().any(|socket| socket["index"] == index) { return None; }
    sub["inputs"][port] = if output == 0 { json!(source) }
        else { json!({"node": source, "output": output}) };
    Some(next)
}

/// A noodle from a downstream input creates an output boundary and wires that input to it.
pub fn add_output(value: &Value, id: &str, target: &str, port: &str) -> Option<Value> {
    let mut next = value.clone();
    let nodes = next.get_mut("nodes")?.as_object_mut()?;
    if id == target || !nodes.contains_key(target) || depends_on(nodes, id, target) { return None; }
    let sub = nodes.get_mut(id).filter(|n| n["type"] == "subnet")?;
    ensure_sockets(sub)?;
    let index = sub["output_sockets"].as_array()?.iter().filter_map(|s| s["index"].as_u64()).max().map_or(0, |i| i + 1);
    let first = sub["inner"]["output"].as_str()?.to_owned();
    let inner = sub["inner"]["nodes"].as_object_mut()?;
    let source = if is_output_boundary(&inner[&first]) {
        inner[&first]["inputs"]["geo"].clone()
    } else { json!(first) };
    let boundary = fresh(inner, "output");
    inner.insert(boundary, json!({"type": "subnet_output", "port_index": index,
        "inputs": {"geo": source}, "pos": [220.0, index as f64 * 96.0]}));
    sub["output_sockets"].as_array_mut()?.push(json!({"index": index, "name": format!("Output {}", index + 1)}));
    crate::graph_edit::connect_output(&next, id, index as usize, target, Some(port))
}

/// Turn the parameter exposure from an inner editor into an input on its
/// enclosing subnet. Keep the exposure in the inner graph so the editor can
/// show and undo the toggle when that subnet is entered again.
pub fn expose_parameters(value: &Value, id: &str, entries: Vec<Value>) -> Option<Value> {
    let mut next = value.clone();
    let sub = next["nodes"].get_mut(id).filter(|node| node["type"] == "subnet")?;
    let inner = sub["inner"]["nodes"].as_object()?;
    let mut additions = Vec::new();
    for entry in &entries {
        let target = entry["node"].as_str()?;
        let param = entry["param"].as_str()?;
        let node = inner.get(target)?;
        let spec = minihou_core::nodes::get(node["type"].as_str()?)?.params.iter()
            .find(|spec| spec.name == param && spec.supported)?;
        let default = node["params"].get(param).cloned().unwrap_or_else(|| (spec.default)());
        additions.push((target.to_owned(), param.to_owned(), default));
    }
    let old = sub["inner"]["exposed"].as_array().cloned().unwrap_or_default();
    let removed: Vec<(String, String)> = old.iter().filter(|entry| !entries.iter().any(|e| e["node"] == entry["node"] && e["param"] == entry["param"]))
        .filter_map(|entry| Some((entry["node"].as_str()?.to_owned(), entry["param"].as_str()?.to_owned()))).collect();
    let mut removed_names = Vec::new();
    if let Some(promoted) = sub["promoted_params"].as_array_mut() {
        promoted.retain(|entry| {
            let remove = removed.iter().any(|(node, param)| entry["node"] == *node && entry["param"] == *param);
            if remove { if let Some(name) = entry["name"].as_str() { removed_names.push(name.to_owned()); } }
            !remove
        });
    }
    for name in removed_names {
        sub["params"].as_object_mut().map(|params| params.remove(&name));
        sub["channels"].as_object_mut().map(|channels| channels.remove(&name));
    }
    for (entry, (target, param, default)) in entries.iter().zip(additions) {
        if old.iter().any(|e| e["node"] == *target && e["param"] == *param) { continue; }
        let base = entry["name"].as_str()?.to_owned();
        let mut name = base.clone();
        let mut suffix = 2;
        while minihou_core::nodes::subnet_public_params(sub).iter().any(|e| e["name"] == name) {
            name = format!("{base}_{suffix}"); suffix += 1;
        }
        if !sub["promoted_params"].is_array() { sub["promoted_params"] = json!([]); }
        sub["promoted_params"].as_array_mut()?.push(json!({"name":name,"node":target,"param":param,"default":default}));
        sub["params"][&name] = default;
    }
    sub["inner"]["exposed"] = Value::Array(entries);
    Some(next)
}

pub fn group(value: &Value, ids: &[String]) -> Option<(Value, String)> {
    let nodes = value.get("nodes")?.as_object()?;
    let sel: BTreeSet<&str> = ids.iter().map(String::as_str).collect();
    if sel.is_empty() || sel.iter().any(|id| !nodes.contains_key(*id)) { return None; }
    let displayed = minihou_app::display_nodes(value);
    // Preserve each distinct outward endpoint, including native secondary outputs.
    let mut outs: Vec<Value> = Vec::new();
    for (_, nd) in nodes.iter().filter(|(id, _)| !sel.contains(id.as_str())) {
        for endpoint in nd["inputs"].as_object().into_iter().flat_map(|inputs| inputs.values()).flat_map(|value|
            if let Value::Array(refs) = value { refs.iter().collect::<Vec<_>>() } else { vec![value] }) {
            let source=endpoint.as_str().or_else(|| endpoint["node"].as_str());
            if source.is_some_and(|source| sel.contains(source)) && !outs.contains(endpoint) { outs.push(endpoint.clone()); }
        }
    }
    let read_inside = |k: &str| nodes.iter().any(|(i, n)| sel.contains(i.as_str()) && refs(n).iter().any(|r| r == k));
    if outs.is_empty() {
        let output=displayed.iter().map(String::as_str).find(|d| sel.contains(d))
            .or_else(|| nodes.keys().map(String::as_str).find(|k| sel.contains(k) && !read_inside(k)))?;
        outs.push(json!(output));
    }
    let mut ext: Vec<Value> = Vec::new();
    let mut inner = Map::new();
    for (id, nd) in nodes.iter().filter(|(id, _)| sel.contains(id.as_str())) {
        let mut nd = nd.clone();
        map_endpoints(&mut nd, &mut |endpoint| {
            let source = endpoint.as_str().or_else(|| endpoint["node"].as_str());
            if source.is_none() { return endpoint.clone(); }
            if source.is_some_and(|source| sel.contains(source)) { return endpoint.clone(); }
            let k = ext.iter().position(|existing| existing == endpoint)
                .unwrap_or_else(|| { ext.push(endpoint.clone()); ext.len() - 1 });
            json!(format!("$in{k}"))
        });
        inner.insert(id.clone(), nd);
    }
    let mut boundary_ids = Vec::new();
    for index in 0..ext.len().max(1) {
        let boundary = fresh(&inner, "input");
        inner.insert(boundary.clone(), json!({"type": "subnet_input", "port_index": index,
            "pos": [-220.0, index as f64 * 96.0]}));
        boundary_ids.push(boundary);
    }
    for nd in inner.values_mut() {
        map_refs(nd, &mut |r| r.strip_prefix("$in").and_then(|n| n.parse::<usize>().ok())
            .and_then(|n| boundary_ids.get(n)).cloned().unwrap_or_else(|| r.to_owned()));
    }
    let mut output_boundaries=Vec::new();
    for (index, endpoint) in outs.iter().enumerate() {
        let boundary=fresh(&inner,"output");
        inner.insert(boundary.clone(),json!({"type":"subnet_output","port_index":index,
            "inputs":{"geo":endpoint},"pos":[220.0,index as f64 * 96.0]}));
        output_boundaries.push(boundary);
    }
    let mut next = value.clone();
    let all = next.get_mut("nodes")?.as_object_mut()?;
    let pos: Vec<[f64; 2]> = sel.iter().filter_map(|id| { let p = nodes[*id].get("pos")?.as_array()?; Some([p.first()?.as_f64()?, p.get(1)?.as_f64()?]) }).collect();
    let id = fresh(all, "subnet");
    for s in &sel { all.remove(*s); }
    for nd in all.values_mut() { map_endpoints(nd,&mut |endpoint| {
        outs.iter().position(|out| out==endpoint).map_or_else(||endpoint.clone(),|index|
            if index==0 {json!(id)} else {json!({"node":id,"output":index})})
    }); }
    let mut node = json!({"type": "subnet", "inner": {"nodes": inner, "output": output_boundaries[0]},
        "input_sockets": (0..boundary_ids.len()).map(|index| json!({"index": index, "name": format!("Input {}", index + 1)})).collect::<Vec<_>>(),
        "output_sockets": (0..output_boundaries.len()).map(|index| json!({"index":index,"name":format!("Output {}",index+1)})).collect::<Vec<_>>()});
    if !ext.is_empty() {
        node["inputs"] = Value::Object(ext.iter().enumerate().map(|(index, source)|
            (format!("in{index}"), source.clone())).collect());
    }
    if !pos.is_empty() {
        let n = pos.len() as f64;
        node["pos"] = json!([pos.iter().map(|p| p[0]).sum::<f64>() / n, pos.iter().map(|p| p[1]).sum::<f64>() / n]);
    }
    all.insert(id.clone(), node);
    let mut shown: Vec<String> = Vec::new();
    for d in displayed.iter().map(|d| if sel.contains(d.as_str()) { id.clone() } else { d.clone() }) { if !shown.contains(&d) { shown.push(d); } }
    if !shown.is_empty() { next["display"] = if shown.len() == 1 { json!(shown[0]) } else { json!(shown) }; }
    Some((next, id))
}

pub fn ungroup(value: &Value, id: &str) -> Option<(Value, Vec<String>)> {
    let nodes = value.get("nodes")?.as_object()?;
    let sub = nodes.get(id).filter(|n| n["type"] == "subnet")?;
    let inner = sub["inner"]["nodes"].as_object()?;
    let output_boundary = sub["inner"]["output"].as_str().filter(|o| inner.contains_key(*o))?;
    let output = if is_output_boundary(&inner[output_boundary]) {
        inner[output_boundary]["inputs"]["geo"].clone()
    } else { json!(output_boundary) };
    let ext = if let Some(sockets) = sub["input_sockets"].as_array() {
        let mut values = vec![None; sockets.iter().filter_map(|socket| socket["index"].as_u64()).max().unwrap_or(0) as usize + 1];
        for socket in sockets {
            let index = socket["index"].as_u64()? as usize;
            values[index] = sub["inputs"].get(format!("in{index}")).cloned();
        }
        values
    } else { refs(sub).into_iter().map(|source| Some(json!(source))).collect() };
    let mut next = value.clone();
    let all = next.get_mut("nodes")?.as_object_mut()?;
    all.remove(id);
    // Restore the inner ids; rename on a clash.
    let mut rename = std::collections::HashMap::new();
    for key in inner.keys().filter(|key| inner[*key]["type"] != "subnet_input" && !is_output_boundary(&inner[*key])) {
        let new = if all.contains_key(key) { fresh(all, key.trim_end_matches(|c: char| c.is_ascii_digit())) } else { key.clone() };
        all.insert(new.clone(), json!({}));
        rename.insert(key.clone(), new);
    }
    for (key, nd) in inner {
        let mut nd = nd.clone();
        if nd["type"] == "subnet_input" || is_output_boundary(&nd) { continue; }
        map_endpoints(&mut nd, &mut |endpoint| restored_endpoint(endpoint, inner, &ext, &rename));
        all.insert(rename[key].clone(), nd);
    }
    let out = restored_endpoint(&output, inner, &ext, &rename);
    if out.is_null() { return None; }
    let mut output_targets = std::collections::HashMap::from([(0_u64, out.clone())]);
    for socket in sub["output_sockets"].as_array().into_iter().flatten() {
        let Some(index) = socket["index"].as_u64() else { continue };
        let source = inner.values().find(|node| is_output_boundary(node) && node["port_index"] == index)
            .and_then(|node| node["inputs"].get("geo"));
        let resolved = source.map(|source| restored_endpoint(source, inner, &ext, &rename));
        if let Some(resolved) = resolved.filter(|value| !value.is_null()) { output_targets.insert(index, resolved); }
    }
    for (k, nd) in all.iter_mut() {
        if !rename.values().any(|v| v == k) {
            if let Some(inputs) = nd.get_mut("inputs").and_then(Value::as_object_mut) {
                for value in inputs.values_mut() {
                    let refs = match value { Value::Array(refs) => refs.as_mut_slice(), one => std::slice::from_mut(one) };
                    for endpoint in refs {
                        if endpoint.as_str() == Some(id) { *endpoint = out.clone(); }
                        else if endpoint["node"] == id {
                            let index = endpoint["output"].as_u64().unwrap_or(0);
                            if let Some(target) = output_targets.get(&index) { *endpoint = target.clone(); }
                        }
                    }
                }
            }
        }
    }
    let shown: Vec<String> = minihou_app::display_nodes(value).iter().map(|d| if d == id {
        out.as_str().or_else(|| out["node"].as_str()).unwrap_or_default().to_owned()
    } else { d.clone() }).collect();
    if !shown.is_empty() { next["display"] = if shown.len() == 1 { json!(shown[0]) } else { json!(shown) }; }
    let mut selected: Vec<_> = rename.into_values().collect();
    selected.sort();
    Some((next, selected))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample() -> Value {
        json!({"version": 1, "display": "out", "nodes": {
            "box1": {"type": "box", "pos": [0.0, 0.0]},
            "xf": {"type": "transform", "inputs": {"geo": "box1"}, "params": {"translate": [0.0, 2.0, 0.0]}, "pos": [0.0, 1.0]},
            "nrm": {"type": "normal", "inputs": {"geo": "xf"}, "pos": [0.0, 2.0]},
            "out": {"type": "output", "inputs": {"geo": "nrm"}}}})
    }
    fn cook(v: &Value) -> Vec<f32> { minihou_core::graph::Graph::from_value(v).unwrap().cook(None).unwrap().p().to_vec() }
    #[test]
    fn new_subnet_contains_connected_boundaries_and_passes_geometry() {
        let mut v = sample();
        v["nodes"]["sub"] = json!({"type": "subnet", "inputs": {"geo": "box1"}});
        initialize(&mut v, "sub");
        assert_eq!(v["nodes"]["sub"]["inner"]["nodes"]["output1"]["inputs"]["geo"], "input1");
        assert_eq!(v["nodes"]["sub"]["inputs"]["in0"], "box1");
        v["display"] = json!("sub");
        assert_eq!(cook(&v), cook(&json!({"nodes": {"box1": v["nodes"]["box1"]}, "display": "box1"})));
    }
    #[test]
    fn noodle_drop_adds_connected_input_and_output_boundaries() {
        let mut v = sample();
        v["nodes"]["sub"] = json!({"type": "subnet", "inputs": {"geo": "box1"}});
        initialize(&mut v, "sub");
        let with_input = add_input(&v, "sub", "xf").unwrap();
        assert_eq!(with_input["nodes"]["sub"]["inputs"]["in1"], "xf");
        assert_eq!(with_input["nodes"]["sub"]["inner"]["nodes"]["input2"]["port_index"], 1);
        let rewired = connect_input(&with_input, "sub", "in1", "box1").unwrap();
        assert_eq!(rewired["nodes"]["sub"]["inputs"]["in1"], "box1");
        let with_output = add_output(&with_input, "sub", "out", "geo").unwrap();
        assert_eq!(with_output["nodes"]["sub"]["inner"]["nodes"]["output2"]["port_index"], 1);
        assert_eq!(with_output["nodes"]["out"]["inputs"]["geo"], json!({"node": "sub", "output": 1}));
        assert_eq!(cook(&with_output), cook(&json!({"nodes": {"box1": v["nodes"]["box1"]}, "display": "box1"})));
        let (ungrouped, _) = ungroup(&with_output, "sub").unwrap();
        assert_eq!(ungrouped["nodes"]["out"]["inputs"]["geo"], "box1");
        assert_eq!(cook(&ungrouped), cook(&with_output));
    }
    #[test]
    fn noodle_drop_upgrades_a_legacy_subnet() {
        let v = json!({"nodes": {
            "box1": {"type": "box"},
            "sub": {"type": "subnet", "inputs": {"geo": "box1"}, "inner": {
                "nodes": {"move": {"type": "transform", "inputs": {"geo": "$in0"}}}, "output": "move"}},
            "out": {"type": "output", "inputs": {"geo": "sub"}}
        }, "display": "out"});
        let with_input = add_input(&v, "sub", "box1").unwrap();
        assert_eq!(with_input["nodes"]["sub"]["inner"]["nodes"]["move"]["inputs"]["geo"], "input1");
        assert_eq!(with_input["nodes"]["sub"]["inner"]["nodes"]["output1"]["inputs"]["geo"], "move");
        assert_eq!(cook(&with_input), cook(&v));
    }
    #[test]
    fn ungroup_preserves_a_higher_output_used_as_an_input() {
        let mut v = json!({"nodes": {
            "box1": {"type": "box"},
            "first": {"type": "subnet", "inputs": {"geo": "box1"}},
            "second": {"type": "subnet", "inputs": {"geo": "box1"}},
            "out": {"type": "output", "inputs": {"geo": "second"}}
        }, "display": "out"});
        initialize(&mut v, "first");
        initialize(&mut v, "second");
        let v = add_output(&v, "first", "out", "geo").unwrap();
        let mut v = add_input_from(&v, "second", "first", 1).unwrap();
        v["nodes"]["second"]["inner"]["nodes"]["output1"]["inputs"]["geo"] = json!("input2");
        v["nodes"]["out"]["inputs"]["geo"] = json!("second");
        let (back, _) = ungroup(&v, "second").unwrap();
        assert_eq!(back["nodes"]["out"]["inputs"]["geo"], json!({"node": "first", "output": 1}));
        assert_eq!(cook(&back), cook(&v));
    }
    #[test]
    fn group_preserves_an_external_higher_output_endpoint() {
        let mut v = json!({"nodes": {
            "box1": {"type": "box"}, "first": {"type": "subnet", "inputs": {"geo": "box1"}},
            "move": {"type": "transform", "inputs": {"geo": {"node": "first", "output": 1}}},
            "out": {"type": "output", "inputs": {"geo": "move"}}
        }, "display": "out"});
        initialize(&mut v, "first");
        v["nodes"]["first"]["output_sockets"].as_array_mut().unwrap().push(json!({"index": 1, "name": "Output 2"}));
        v["nodes"]["first"]["inner"]["nodes"]["output2"] = json!({"type": "subnet_output", "port_index": 1, "inputs": {"geo": "input1"}});
        let (grouped, id) = group(&v, &["move".into()]).unwrap();
        assert_eq!(grouped["nodes"][&id]["inputs"]["in0"], json!({"node": "first", "output": 1}));
        assert_eq!(cook(&grouped), cook(&v));
        let (back, _) = ungroup(&grouped, &id).unwrap();
        assert_eq!(back["nodes"]["move"]["inputs"]["geo"], json!({"node": "first", "output": 1}));
        assert_eq!(cook(&back), cook(&v));
    }
    #[test]
    fn group_keeps_inner_graph_and_cooks_the_same() {
        let v = sample();
        let (g, id) = group(&v, &["xf".into(), "nrm".into()]).unwrap();
        assert_eq!(id, "subnet1");
        assert!(g["nodes"].get("xf").is_none());
        assert_eq!(g["nodes"]["out"]["inputs"]["geo"], "subnet1");
        assert_eq!(g["nodes"]["subnet1"]["inputs"]["in0"], "box1");
        assert_eq!(g["nodes"]["subnet1"]["inner"]["nodes"]["xf"]["inputs"]["geo"], "input1");
        assert_eq!(g["nodes"]["subnet1"]["inner"]["nodes"]["output1"]["inputs"]["geo"], "nrm");
        let doc = crate::graph_edit::validated(g.clone()).unwrap();
        assert_eq!(cook(&g), cook(&v));
        assert_eq!(cook(&serde_json::from_str::<Value>(doc.text()).unwrap()), cook(&v));
        assert_eq!(doc.session().unwrap().graph().to_value(), g);
    }
    #[test]
    fn group_adds_a_boundary_for_each_external_input() {
        let v = json!({"nodes": {
            "a": {"type": "box"}, "b": {"type": "sphere"},
            "merge": {"type": "merge", "inputs": {"geos": ["a", "b"]}},
            "out": {"type": "output", "inputs": {"geo": "merge"}}
        }, "display": "out"});
        let (g, id) = group(&v, &["merge".into()]).unwrap();
        let sub = &g["nodes"][&id];
        assert_eq!(sub["input_sockets"].as_array().unwrap().len(), 2);
        assert_eq!(sub["inputs"]["in0"], "a");
        assert_eq!(sub["inputs"]["in1"], "b");
        assert_eq!(sub["inner"]["nodes"]["merge"]["inputs"]["geos"], json!(["input1", "input2"]));
        assert_eq!(cook(&g), cook(&v));
    }
    #[test]
    fn group_always_creates_an_input_and_output_even_without_external_wires() {
        let v = json!({"display":"box1","nodes":{"box1":{"type":"box","pos":[32.0,64.0]}}});
        let (grouped, id) = group(&v, &["box1".into()]).unwrap();
        let subnet = &grouped["nodes"][&id];
        assert_eq!(subnet["input_sockets"], json!([{"index":0,"name":"Input 1"}]));
        assert_eq!(subnet["output_sockets"], json!([{"index":0,"name":"Output 1"}]));
        assert_eq!(subnet["inner"]["nodes"]["input1"]["type"], "subnet_input");
        assert_eq!(subnet["inner"]["nodes"]["output1"]["inputs"]["geo"], "box1");
        assert_eq!(subnet["inner"]["output"], "output1");
        assert_eq!(grouped["display"], id);
        assert_eq!(cook(&grouped), cook(&v));
    }
    #[test]
    fn ungroup_restores_the_original() {
        let v = sample();
        let (g, _) = group(&v, &["xf".into(), "nrm".into()]).unwrap();
        let (back, sel) = ungroup(&g, "subnet1").unwrap();
        assert_eq!(sel.len(), 2);
        assert_eq!(back["nodes"], v["nodes"]);
        assert_eq!(back["display"], v["display"]);
    }
    #[test]
    fn ungroup_selected_subnet_restores_inner_nodes_and_external_wires() {
        let original = sample();
        let (grouped, subnet) = group(&original, &["xf".into(), "nrm".into()]).unwrap();
        let (ungrouped, selected) = ungroup(&grouped, &subnet).unwrap();
        assert_eq!(selected, vec!["nrm", "xf"]);
        assert!(ungrouped["nodes"].get(&subnet).is_none());
        assert_eq!(ungrouped["nodes"]["xf"]["inputs"]["geo"], "box1");
        assert_eq!(ungrouped["nodes"]["nrm"]["inputs"]["geo"], "xf");
        assert_eq!(ungrouped["nodes"]["out"]["inputs"]["geo"], "nrm");
        assert_eq!(cook(&ungrouped), cook(&original));
    }
    #[test]
    fn group_moves_display_and_keeps_multiple_outputs() {
        let v = sample();
        let (g, _) = group(&v, &["nrm".into(), "out".into()]).unwrap();
        assert_eq!(g["display"], "subnet1");
        let two = json!({"display":"c","nodes": {"a": {"type": "box"}, "b": {"type": "transform", "inputs": {"geo": "a"}},
            "c": {"type": "merge", "inputs": {"geos": ["a", "b"]}}}});
        let (grouped,id)=group(&two,&["a".into(),"b".into()]).unwrap();
        assert_eq!(grouped["nodes"][&id]["output_sockets"].as_array().unwrap().len(),2);
        assert_eq!(grouped["nodes"]["c"]["inputs"]["geos"],json!([id,{"node":id,"output":1}]));
        assert_eq!(cook(&grouped),cook(&two));
        let (back,_)=ungroup(&grouped,&id).unwrap();
        assert_eq!(back["nodes"]["c"]["inputs"]["geos"],two["nodes"]["c"]["inputs"]["geos"]);
        assert!(group(&v, &[]).is_none());
    }
    #[test]
    fn group_preserves_secondary_inner_stream_output() {
        let doc=json!({"display":"out","nodes":{
            "box":{"type":"box"},
            "fill":{"type":"flood_fill","inputs":{"geometry":"box"}},
            "out":{"type":"null","inputs":{"geo":{"node":"fill","output":1}}}
        }});
        let (grouped,id)=group(&doc,&["fill".into()]).unwrap();
        assert_eq!(grouped["nodes"][&id]["inner"]["nodes"]["output1"]["inputs"]["geo"],
            json!({"node":"fill","output":1}));
        assert_eq!(grouped["nodes"]["out"]["inputs"]["geo"],id);
        assert_eq!(cook(&grouped),cook(&doc));
        let (back,_)=ungroup(&grouped,&id).unwrap();
        assert_eq!(back["nodes"]["out"]["inputs"]["geo"],doc["nodes"]["out"]["inputs"]["geo"]);
    }
    #[test]
    fn nested_subnets_cook() {
        let v = sample();
        let (g, _) = group(&v, &["xf".into(), "nrm".into()]).unwrap();
        let (g2, _) = group(&g, &["subnet1".into(), "box1".into()]).unwrap();
        assert_eq!(cook(&g2), cook(&v));
    }
}
