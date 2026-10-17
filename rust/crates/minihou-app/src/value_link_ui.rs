//! UI-facing channel operations. Multi-component changes use one document snapshot.
use serde_json::{json, Value};
use minihou_core::{channels, graph::Graph, nodes};
use crate::{Edit, ParamKind, param_kind};

#[derive(Clone, Debug)]
pub struct CopiedParam { pub node: String, pub name: String, pub value: Value, pub expressions: Vec<(String, String)> }

pub fn copied(doc: &Value, node: &str, name: &str) -> Option<CopiedParam> {
    let graph = Graph::from_value(doc).ok()?;
    let resolved = channels::resolve_graph(&graph);
    let value = resolved.params.get(node)?.get(name)?.clone();
    let expressions = doc["nodes"][node]["channels"].as_object().into_iter().flatten()
        .filter(|(key, _)| *key == name || key.starts_with(&format!("{name}.")))
        .filter_map(|(key, value)| Some((key.strip_prefix(name)?.to_owned(), value.as_str()?.to_owned()))).collect();
    Some(CopiedParam { node: node.into(), name: name.into(), value, expressions })
}

pub fn compatible(source: &Value, target: &Value) -> bool {
    use ParamKind::*;
    matches!((param_kind(source), param_kind(target)),
        (Float | Integer | Bool, Float | Integer | Bool) | (Vec3, Vec3) | (String, String))
}

pub fn relative_path(from: &str, to: &str, param: &str) -> String {
    let a: Vec<_> = from.split('/').collect();
    let b: Vec<_> = to.split('/').collect();
    let shared = a.iter().zip(&b).take_while(|(x,y)| x == y).count();
    let mut parts = vec![".."; a.len() - shared];
    parts.extend_from_slice(&b[shared..]);
    if parts.is_empty() { format!("./{param}") } else { format!("{}/{param}", parts.join("/")) }
}

fn target_value(doc: &Value, node: &str, name: &str) -> Option<Value> {
    let typ = doc["nodes"][node]["type"].as_str()?;
    let spec = nodes::get(typ)?.params.iter().find(|p| p.name == name)?;
    Some(doc["nodes"][node]["params"].get(name).cloned().unwrap_or_else(|| (spec.default)()))
}

fn literal_for(source: &Value, target: &Value) -> Option<Value> {
    if !compatible(source, target) { return None; }
    match param_kind(target) {
        ParamKind::Integer => Some(json!(source.as_f64()?.round() as i64)),
        ParamKind::Bool => Some(json!(source.as_bool().unwrap_or_else(|| source.as_f64().unwrap_or(0.) != 0.))),
        ParamKind::Float => Some(json!(source.as_f64().unwrap_or_else(|| if source.as_bool() == Some(true) { 1. } else { 0. }))),
        _ => Some(source.clone()),
    }
}

pub enum PasteMode { Value, Expression, Reference }
pub fn paste(doc: &Value, node: &str, name: &str, copied: &CopiedParam, mode: PasteMode) -> Result<Edit, String> {
    let target = target_value(doc, node, name).ok_or("Unknown target parameter")?;
    if !compatible(&copied.value, &target) { return Err("Parameter types are incompatible".into()); }
    if node == copied.node && name == copied.name && matches!(mode, PasteMode::Reference) { return Err("A parameter cannot reference itself".into()); }
    let mut next = doc.clone();
    let channels = next["nodes"][node].as_object_mut().ok_or("Unknown target node")?
        .entry("channels").or_insert_with(|| json!({})).as_object_mut().ok_or("Invalid channels")?;
    channels.retain(|key, _| key != name && !key.starts_with(&format!("{name}.")));
    match mode {
        PasteMode::Value => { next["nodes"][node]["params"][name] = literal_for(&copied.value, &target).ok_or("Incompatible value")?; }
        PasteMode::Expression => {
            if copied.expressions.is_empty() { return Err("Copied parameter has no expression".into()); }
            for (suffix, expr) in &copied.expressions { next["nodes"][node]["channels"][format!("{name}{suffix}")] = json!(expr); }
        }
        PasteMode::Reference => {
            let path = relative_path(node, &copied.node, &copied.name);
            if matches!(param_kind(&target), ParamKind::Vec3) {
                for component in ["x", "y", "z"] {
                    next["nodes"][node]["channels"][format!("{name}.{component}")] = json!(format!("ch(\"{path}.{component}\")"));
                }
            } else {
                let fun = if matches!(param_kind(&target), ParamKind::String) { "chs" } else { "ch" };
                next["nodes"][node]["channels"][name] = json!(format!("{fun}(\"{path}\")"));
            }
        }
    }
    Ok(Edit::Replace(next))
}

pub fn clear(doc: &Value, node: &str, name: &str, default: bool) -> Option<Edit> {
    let mut next = doc.clone();
    let value = if default { target_value(doc,node,name).and_then(|_| {
        let typ=doc["nodes"][node]["type"].as_str()?;
        Some((nodes::get(typ)?.params.iter().find(|p|p.name==name)?.default)())
    })? } else {
        let graph=Graph::from_value(doc).ok()?;
        channels::resolve_graph(&graph).params.get(node)?.get(name)?.clone()
    };
    next["nodes"][node]["params"][name] = value;
    next["nodes"][node]["channels"].as_object_mut()?.retain(|key,_| key != name && !key.starts_with(&format!("{name}.")));
    Some(Edit::Replace(next))
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn paths_and_types() {
        assert_eq!(relative_path("a", "b", "size.x"), "../b/size.x");
        assert_eq!(relative_path("sub/a", "sub/b", "p"), "../b/p");
        assert_eq!(relative_path("sub/a", "b", "p"), "../../b/p");
        assert!(compatible(&json!(1), &json!(1.0)));
        assert!(!compatible(&json!("a"), &json!(1.0)));
    }
    #[test] fn vector_reference_is_one_edit() {
        let doc=json!({"nodes":{"a":{"type":"box","params":{"size":[2,3,4]}},"b":{"type":"box"}}});
        let copy=copied(&doc,"a","size").unwrap();
        let Edit::Replace(next)=paste(&doc,"b","size",&copy,PasteMode::Reference).unwrap() else {panic!()};
        assert_eq!(next["nodes"]["b"]["channels"]["size.y"],json!("ch(\"../a/size.y\")"));
    }
}
