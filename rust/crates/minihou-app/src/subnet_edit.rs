//! Ctrl+G: collapse selected nodes into one `subnet` node that keeps them as its inner graph, or expand it again.
//! Inner external reads are "$in0", "$in1" ... (the order of the subnet's `geo` wires); see `minihou_core::graph::expand_subnets`.
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;

fn refs(node: &Value) -> Vec<String> {
    node.get("inputs").and_then(Value::as_object).into_iter().flat_map(|m| m.values()).flat_map(|v| match v {
        Value::String(s) => vec![s.clone()],
        Value::Array(a) => a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect(),
        _ => vec![],
    }).collect()
}

fn map_refs(node: &mut Value, f: &mut dyn FnMut(&str) -> String) {
    let Some(inputs) = node.get_mut("inputs").and_then(Value::as_object_mut) else { return };
    for v in inputs.values_mut() {
        match v {
            Value::String(s) => *s = f(s),
            Value::Array(a) => for x in a { if let Some(s) = x.as_str() { *x = json!(f(s)); } },
            _ => {}
        }
    }
}

fn fresh(nodes: &Map<String, Value>, ty: &str) -> String {
    (1..).map(|n| format!("{ty}{n}")).find(|id| !nodes.contains_key(id)).unwrap()
}

/// Ctrl+G on the selection: one selected subnet is expanded, otherwise the selection is collapsed. Returns the new graph and
/// the ids to select. `None` when nothing can be done (empty/unknown selection, or several selected nodes read by the rest).
pub fn toggle(value: &Value, ids: &[String]) -> Option<(Value, Vec<String>)> {
    let nodes = value.get("nodes")?.as_object()?;
    if ids.len() == 1 && nodes.get(&ids[0])?["type"] == "subnet" { return ungroup(value, &ids[0]); }
    group(value, ids).map(|(v, id)| (v, vec![id]))
}

pub fn group(value: &Value, ids: &[String]) -> Option<(Value, String)> {
    let nodes = value.get("nodes")?.as_object()?;
    let sel: BTreeSet<&str> = ids.iter().map(String::as_str).collect();
    if sel.is_empty() || sel.iter().any(|id| !nodes.contains_key(*id)) { return None; }
    let displayed = minihou_app::display_nodes(value);
    // The inner output: the one selected node read from outside, else the displayed or first selected sink.
    let mut outs: Vec<&str> = Vec::new();
    for (_, nd) in nodes.iter().filter(|(id, _)| !sel.contains(id.as_str())) {
        for r in refs(nd) {
            if let Some(s) = sel.get(r.as_str()) { if !outs.contains(s) { outs.push(s); } }
        }
    }
    let read_inside = |k: &str| nodes.iter().any(|(i, n)| sel.contains(i.as_str()) && refs(n).iter().any(|r| r == k));
    let output = match outs.len() {
        0 => displayed.iter().map(String::as_str).find(|d| sel.contains(d))
            .or_else(|| nodes.keys().map(String::as_str).find(|k| sel.contains(k) && !read_inside(k)))?,
        1 => outs[0],
        _ => return None,
    };
    let mut ext: Vec<String> = Vec::new();
    let mut inner = Map::new();
    for (id, nd) in nodes.iter().filter(|(id, _)| sel.contains(id.as_str())) {
        let mut nd = nd.clone();
        map_refs(&mut nd, &mut |r| if sel.contains(r) { r.to_owned() } else {
            let k = ext.iter().position(|e| e == r).unwrap_or_else(|| { ext.push(r.to_owned()); ext.len() - 1 });
            format!("$in{k}")
        });
        inner.insert(id.clone(), nd);
    }
    let mut next = value.clone();
    let all = next.get_mut("nodes")?.as_object_mut()?;
    let pos: Vec<[f64; 2]> = sel.iter().filter_map(|id| { let p = nodes[*id].get("pos")?.as_array()?; Some([p.first()?.as_f64()?, p.get(1)?.as_f64()?]) }).collect();
    let id = fresh(all, "subnet");
    for s in &sel { all.remove(*s); }
    for nd in all.values_mut() { map_refs(nd, &mut |r| if r == output { id.clone() } else { r.to_owned() }); }
    let mut node = json!({"type": "subnet", "inner": {"nodes": inner, "output": output}});
    if !ext.is_empty() { node["inputs"] = json!({"geo": if ext.len() == 1 { json!(ext[0]) } else { json!(ext) }}); }
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
    let output = sub["inner"]["output"].as_str().filter(|o| inner.contains_key(*o))?;
    let ext = refs(sub);
    let mut next = value.clone();
    let all = next.get_mut("nodes")?.as_object_mut()?;
    all.remove(id);
    // Restore the inner ids; rename on a clash.
    let mut rename = std::collections::HashMap::new();
    for key in inner.keys() {
        let new = if all.contains_key(key) { fresh(all, key.trim_end_matches(|c: char| c.is_ascii_digit())) } else { key.clone() };
        all.insert(new.clone(), json!({}));
        rename.insert(key.clone(), new);
    }
    for (key, nd) in inner {
        let mut nd = nd.clone();
        map_refs(&mut nd, &mut |r| match r.strip_prefix("$in").and_then(|k| k.parse::<usize>().ok()) {
            Some(k) => ext.get(k).cloned().unwrap_or_default(),
            None => rename.get(r).cloned().unwrap_or_else(|| r.to_owned()),
        });
        all.insert(rename[key].clone(), nd);
    }
    let out = rename[output].clone();
    for (k, nd) in all.iter_mut() {
        if !rename.values().any(|v| v == k) { map_refs(nd, &mut |r| if r == id { out.clone() } else { r.to_owned() }); }
    }
    let shown: Vec<String> = minihou_app::display_nodes(value).iter().map(|d| if d == id { out.clone() } else { d.clone() }).collect();
    if !shown.is_empty() { next["display"] = if shown.len() == 1 { json!(shown[0]) } else { json!(shown) }; }
    Some((next, rename.into_values().collect()))
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
    fn group_keeps_inner_graph_and_cooks_the_same() {
        let v = sample();
        let (g, id) = group(&v, &["xf".into(), "nrm".into()]).unwrap();
        assert_eq!(id, "subnet1");
        assert!(g["nodes"].get("xf").is_none());
        assert_eq!(g["nodes"]["out"]["inputs"]["geo"], "subnet1");
        assert_eq!(g["nodes"]["subnet1"]["inputs"]["geo"], "box1");
        assert_eq!(g["nodes"]["subnet1"]["inner"]["nodes"]["xf"]["inputs"]["geo"], "$in0");
        let doc = crate::graph_edit::validated(g.clone()).unwrap();
        assert_eq!(cook(&g), cook(&v));
        assert_eq!(cook(&serde_json::from_str::<Value>(doc.text()).unwrap()), cook(&v));
        assert_eq!(doc.session().unwrap().graph().to_value(), g);
    }
    #[test]
    fn ungroup_restores_the_original() {
        let v = sample();
        let (g, _) = group(&v, &["xf".into(), "nrm".into()]).unwrap();
        let (back, sel) = toggle(&g, &["subnet1".into()]).unwrap();
        assert_eq!(sel.len(), 2);
        assert_eq!(back["nodes"], v["nodes"]);
        assert_eq!(back["display"], v["display"]);
    }
    #[test]
    fn group_moves_display_and_rejects_ambiguous_outputs() {
        let v = sample();
        let (g, _) = group(&v, &["nrm".into(), "out".into()]).unwrap();
        assert_eq!(g["display"], "subnet1");
        let two = json!({"nodes": {"a": {"type": "box"}, "b": {"type": "transform", "inputs": {"geo": "a"}},
            "c": {"type": "output", "inputs": {"geo": ["a", "b"]}}}});
        assert!(group(&two, &["a".into(), "b".into()]).is_none());
        assert!(group(&v, &[]).is_none());
    }
    #[test]
    fn nested_subnets_cook() {
        let v = sample();
        let (g, _) = group(&v, &["xf".into(), "nrm".into()]).unwrap();
        let (g2, _) = group(&g, &["subnet1".into(), "box1".into()]).unwrap();
        assert_eq!(cook(&g2), cook(&v));
    }
}
