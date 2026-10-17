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
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link { pub source: String, pub source_param: String, pub target: String, pub target_param: String, pub broken: bool }

fn references(expression: &str) -> Vec<&str> {
    let mut found = Vec::new();
    for function in ["ch(\"", "chs(\"", "chv(\"", "ch('", "chs('", "chv('"] {
        let mut rest = expression;
        while let Some(start) = rest.find(function) {
            rest = &rest[start + function.len()..];
            let quote = if function.ends_with('"') { '"' } else { '\'' };
            if let Some(end) = rest.find(quote) { found.push(&rest[..end]); rest = &rest[end + 1..]; } else { break; }
        }
    }
    found
}
fn resolve_path(owner: &str, path: &str) -> Option<(String,String)> {
    let mut parts: Vec<&str> = if path.starts_with('/') { Vec::new() } else { owner.split('/').collect() };
    let segments: Vec<&str> = path.split('/').collect();
    for part in &segments[..segments.len().checked_sub(1)?] {
        match *part { "" | "." => {}, ".." => { parts.pop()?; }, other => parts.push(other) }
    }
    Some((parts.join("/"), segments.last()?.to_string()))
}
pub fn rewrite_expression(expression: &str, old_owner: &str, new_owner: &str, map_node: impl Fn(&str) -> String) -> String {
    let mut out=String::new(); let mut rest=expression;
    while !rest.is_empty() {
        let hit=["ch(\"","chs(\"","chv(\"","ch('","chs('","chv('"]
            .iter().filter_map(|prefix| rest.find(prefix).map(|at|(at,*prefix))).min_by_key(|(at,_)|*at);
        let Some((at,prefix))=hit else {out.push_str(rest);break};
        let start=at+prefix.len(); out.push_str(&rest[..start]);
        let quote=if prefix.ends_with('"') {'"'} else {'\''};
        let Some(end)=rest[start..].find(quote) else {out.push_str(&rest[start..]);break};
        let path=&rest[start..start+end];
        if let Some((source,param))=resolve_path(old_owner,path) {
            let source=map_node(&source);
            if path.starts_with('/') { out.push_str(&format!("/{source}/{param}")); }
            else { out.push_str(&relative_path(new_owner,&source,&param)); }
        } else {out.push_str(path);}
        rest=&rest[start+end..];
    }
    out
}
pub fn links(doc: &Value) -> Vec<Link> {
    let Some(nodes) = doc["nodes"].as_object() else { return Vec::new(); };
    let mut result = Vec::new();
    for (target, def) in nodes {
        if let Some(channels) = def["channels"].as_object() {
            for (target_param, expression) in channels {
                for path in expression.as_str().into_iter().flat_map(references) {
                    if let Some((source, source_param)) = resolve_path(target, path) {
                        let base = source_param.trim_end_matches(".x").trim_end_matches(".y").trim_end_matches(".z");
                        let broken = nodes.get(&source).and_then(|n| n["type"].as_str()).and_then(nodes::get)
                            .is_none_or(|spec| !spec.params.iter().any(|p|p.name==base));
                        result.push(Link { source, source_param, target:target.clone(), target_param:target_param.clone(), broken });
                    } else {
                        result.push(Link {source:String::new(),source_param:path.into(),target:target.clone(),target_param:target_param.clone(),broken:true});
                    }
                }
            }
        }
    }
    result
}

pub fn go_to_source(doc: &Value, target: &str, param: Option<&str>) -> Option<(String,String)> {
    links(doc).into_iter().find(|link| link.target == target && param.is_none_or(|p| link.target_param == p || link.target_param.starts_with(&format!("{p}."))) && !link.broken)
        .map(|link|(link.source,link.source_param))
}
pub fn go_to_target(doc: &Value, source: &str, param: Option<&str>) -> Option<(String,String)> {
    links(doc).into_iter().find(|link| link.source == source && param.is_none_or(|p| link.source_param == p || link.source_param.starts_with(&format!("{p}."))))
        .map(|link|(link.target,link.target_param))
}
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
    #[test] fn link_paths_and_navigation() {
        let doc=json!({"nodes":{"a":{"type":"box"},"b":{"type":"box","channels":{"size.x":"ch('../a/size.y') + 1"}}}});
        assert_eq!(go_to_source(&doc,"b",Some("size")),Some(("a".into(),"size.y".into())));
        assert_eq!(go_to_target(&doc,"a",None),Some(("b".into(),"size.x".into())));
        assert!(!links(&doc)[0].broken);
    }
    #[test] fn rewrite_only_channel_paths() {
        let expr="ch('../old/size.x') + ch(\"/other/size.y\")";
        assert_eq!(rewrite_expression(expr,"target","target",|node| if node=="old" {"new".into()} else {node.into()}),
            "ch('../new/size.x') + ch(\"/other/size.y\")");
    }
}
