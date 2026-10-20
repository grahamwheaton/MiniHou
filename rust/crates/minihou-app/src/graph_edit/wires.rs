use super::*;

pub fn connect(value: &Value, from: &str, to: &str, port: Option<&str>) -> Option<Value> {
    connect_output(value, from, 0, to, port)
}

fn endpoint_node(value: &Value) -> Option<&str> {
    value.as_str().or_else(|| value.get("node")?.as_str())
}

fn endpoint_output(value: &Value) -> Option<usize> {
    if value.is_string() { Some(0) }
    else { value.get("output")?.as_u64().and_then(|n| usize::try_from(n).ok()) }
}

pub fn connect_output(value: &Value, from: &str, output: usize, to: &str, port: Option<&str>) -> Option<Value> {
    if from == to { return None; }
    let all = nodes(value)?;
    let source = all.get(from)?;
    let source_type = source.get("type")?.as_str()?;
    let native = output < nodes::get(source_type)?.outputs.len();
    let subnet = source_type == "subnet" && source.get("output_sockets").and_then(Value::as_array)
        .is_some_and(|sockets| sockets.iter().any(|socket| socket.get("index").and_then(Value::as_u64) == Some(output as u64)));
    if !native && !subnet { return None; }
    let ty = all.get(to)?.get("type")?.as_str()?;
    let spec = nodes::get(ty)?;
    let input = match port { Some(name) => spec.inputs.iter().find(|p| p.name == name)?, None => spec.inputs.first()? };
    let mut value = value.clone();
    let target = nodes_mut(&mut value)?.get_mut(to)?;
    if target.get("inputs").is_none() { target["inputs"] = json!({}); }
    let inputs = target.get_mut("inputs")?.as_object_mut()?;
    let endpoint = if output == 0 { json!(from) } else { json!({"node":from,"output":output}) };
    if input.multi {
        let list = inputs.entry(input.name).or_insert_with(|| json!([]));
        if let Some(a) = list.as_array_mut() { if !a.contains(&endpoint) { a.push(endpoint); } }
        else { *list = json!([endpoint]); }
    } else { inputs.insert(input.name.to_owned(), endpoint); }
    Some(value)
}

/// Identity of one noodle, including its source within a multi-input port.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct WireRef { pub from: String, pub to: String, pub port: String }

/// Splice one exact edge, preserving multi-input order and terminal Output nodes.
/// Native ports all carry Geometry; the registry declares required/optional ports.
pub fn insert_existing(value: &Value, id: &str, wire: &WireRef) -> Option<Value> {
    let all = nodes(value)?;
    let node = all.get(id)?;
    if id == wire.from || id == wire.to || node["type"] == "output" || all.get(&wire.from)?["type"] == "output" { return None; }
    if node.get("inputs").and_then(Value::as_object).is_some_and(|inputs|
        inputs.values().any(|v| endpoint_node(v).is_some() || v.as_array().is_some_and(|a| !a.is_empty()))) { return None; }
    nodes::get(all.get(&wire.from)?["type"].as_str()?)?;
    let spec = nodes::get(node["type"].as_str()?)?;
    let input = spec.inputs.iter().find(|p| !p.optional).or_else(|| spec.inputs.first())?;
    if spec.inputs.iter().any(|p| p.name != input.name && !p.optional) { return None; }
    let dest = nodes::get(all.get(&wire.to)?["type"].as_str()?)?;
    dest.inputs.iter().find(|p| p.name == wire.port)?;
    // Downstream wiring must not introduce a cycle through an existing consumer.
    let mut pending = vec![id];
    let mut seen = HashSet::new();
    while let Some(source) = pending.pop() {
        if source == wire.from { return None; }
        if !seen.insert(source) { continue; }
        for (target, def) in all {
            if def.get("inputs").and_then(Value::as_object).is_some_and(|i| i.values().any(|v|
                endpoint_node(v) == Some(source) || v.as_array().is_some_and(|a| a.iter().any(|r| endpoint_node(r) == Some(source))))) {
                pending.push(target.as_str());
            }
        }
    }
    let original = all.get(&wire.to)?.get("inputs")?.get(&wire.port)?;
    let original = if let Some(a) = original.as_array() { a.iter().find(|r| endpoint_node(r)==Some(&wire.from))? } else { original };
    let mut next = connect_output(value, &wire.from, endpoint_output(original)?, id, Some(input.name))?;
    let edge = next.get_mut("nodes")?.get_mut(&wire.to)?.get_mut("inputs")?.get_mut(&wire.port)?;
    if let Some(refs) = edge.as_array_mut() {
        if refs.iter().any(|r| endpoint_node(r) == Some(id)) { return None; }
        let index = refs.iter().position(|r| endpoint_node(r) == Some(&wire.from))?;
        refs[index] = json!(id);
        refs.retain(|r| endpoint_node(r) != Some(&wire.from));
    } else if endpoint_node(edge) == Some(&wire.from) { *edge = json!(id); }
    else { return None; }
    Some(next)
}

/// A pickup remains a UI preview until release, so cancellation needs no graph edit.
#[derive(Clone, Debug)]
pub struct WirePickup { pub original: WireRef }
impl WirePickup {
    pub fn begin(value: &Value, node: &str, port: &str) -> Option<Self> {
        let refs = value.get("nodes")?.get(node)?.get("inputs")?.get(port)?;
        let from = endpoint_node(refs).or_else(|| refs.as_array()?.last().and_then(endpoint_node))?;
        Some(Self { original: WireRef { from: from.into(), to: node.into(), port: port.into() } })
    }
    pub fn release(&self, value: &Value, target: Option<(&str, &str)>) -> Option<Value> {
        if target == Some((self.original.to.as_str(), self.original.port.as_str())) { return None; }
        let original = value.get("nodes")?.get(&self.original.to)?.get("inputs")?.get(&self.original.port)?;
        let original = if let Some(a)=original.as_array() { a.iter().find(|r| endpoint_node(r)==Some(&self.original.from))? } else { original };
        let output = endpoint_output(original)?;
        let next = disconnect_wires(value, &std::collections::BTreeSet::from([self.original.clone()]))?;
        if let Some((node, port)) = target {
            if value["nodes"][&self.original.from]["type"] == "output" { return None; }
            let next = connect_output(&next, &self.original.from, output, node, Some(port))?;
            // Reject cycles, including those outside the displayed branch.
            let mut pending = vec![self.original.from.as_str()];
            let mut seen = HashSet::new();
            while let Some(id) = pending.pop() {
                if id == node { return None; }
                if !seen.insert(id) { continue; }
                if let Some(inputs) = next["nodes"][id]["inputs"].as_object() {
                    for refs in inputs.values() {
                        if let Some(source) = endpoint_node(refs) { pending.push(source); }
                        else if let Some(refs) = refs.as_array() { pending.extend(refs.iter().filter_map(endpoint_node)); }
                    }
                }
            }
            Some(next)
        } else { Some(next) }
    }
}

/// Disconnect exact edges in one snapshot. Never dissolve or remove nodes.
pub fn disconnect_wires(value: &Value, wires: &std::collections::BTreeSet<WireRef>) -> Option<Value> {
    let mut next = value.clone();
    let mut changed = false;
    for wire in wires {
        let Some(inputs) = next.get_mut("nodes").and_then(|n| n.get_mut(&wire.to))
            .and_then(|n| n.get_mut("inputs")).and_then(Value::as_object_mut) else { continue };
        let Some(input) = inputs.get_mut(&wire.port) else { continue };
        if let Some(refs) = input.as_array_mut() {
            let before = refs.len();
            refs.retain(|r| endpoint_node(r) != Some(wire.from.as_str()));
            changed |= before != refs.len();
            if refs.is_empty() { inputs.remove(&wire.port); }
        } else if endpoint_node(input) == Some(wire.from.as_str()) {
            inputs.remove(&wire.port);
            changed = true;
        }
    }
    changed.then_some(next)
}

pub fn disconnect(value: &Value, node: &str, port: &str) -> Option<Value> {
    let mut value = value.clone();
    nodes_mut(&mut value)?.get_mut(node)?.get_mut("inputs")?.as_object_mut()?.remove(port)?;
    Some(value)
}

