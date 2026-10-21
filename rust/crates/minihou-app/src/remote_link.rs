use crate::{metadata_kind, modifier_inputs, param_kind, param_meta, Edit, ParamKind, Result};
use serde_json::{json, Value};

/// Nodes with saved exposed parameters; usable by the network UI without egui state.
pub fn exposed_nodes(doc: &Value) -> std::collections::HashSet<String> {
    doc["exposed"].as_array().into_iter().flatten()
        .filter_map(|e| e["node"].as_str().map(str::to_owned)).collect()
}

pub fn is_exposed(doc: &Value, node: &str, param: &str) -> bool {
    doc["exposed"].as_array().into_iter().flatten()
        .any(|e| e["node"] == node && e["param"] == param)
}

pub fn toggle_exposed(doc: &Value, node: &str, param: &str) -> Edit {
    let mut entries = doc["exposed"].as_array().cloned().unwrap_or_default();
    if is_exposed(doc, node, param) {
        entries.retain(|e| !(e["node"] == node && e["param"] == param));
    } else {
        let base = format!("{node}_{param}");
        let mut name = base.clone();
        let mut suffix = 2;
        while entries.iter().any(|e| e["name"] == name) {
            name = format!("{base}_{suffix}"); suffix += 1;
        }
        entries.push(json!({"name":name,"node":node,"param":param,"label":name}));
    }
    Edit::Exposed(entries)
}

/// Python-compatible Blender socket descriptions. Skip stale and non-socket parameters.
pub fn link_interface(doc: &Value) -> Value {
    let mut params = Vec::new();
    let mut names = std::collections::HashSet::new();
    for entry in doc["exposed"].as_array().into_iter().flatten() {
        let Some(node) = entry["node"].as_str() else { continue };
        let Some(param) = entry["param"].as_str() else { continue };
        let Some(name) = entry["name"].as_str() else { continue };
        let Some(typ) = doc["nodes"][node]["type"].as_str() else { continue };
        let spec = minihou_core::nodes::get(typ).and_then(|s| s.params.iter().find(|p| p.name == param && p.supported));
        let promoted = (typ == "subnet").then(|| minihou_core::nodes::subnet_public_params(&doc["nodes"][node])
            .into_iter().find(|p| p["name"] == param)).flatten();
        let Some(default) = spec.map(|spec| (spec.default)()).or_else(|| promoted.map(|p| p["default"].clone())) else { continue };
        let meta = param_meta(typ, param);
        let kind = meta.map_or_else(|| param_kind(&default), |m| metadata_kind(m, &default));
        let kind = match kind { ParamKind::Bool => "bool", ParamKind::Integer => "int", ParamKind::Float => "float", ParamKind::Vec3 => "vec3", _ => continue };
        if !names.insert(name.to_owned()) { continue; }
        let mut p = meta.cloned().unwrap_or_else(|| json!({}));
        p["kind"] = json!(kind); p["default"] = default.clone();
        p["name"] = json!(name); p["node"] = json!(node); p["param"] = json!(param);
        p["label"] = json!(entry["label"].as_str().filter(|s| !s.is_empty()).unwrap_or(name));
        p["value"] = doc["nodes"][node]["params"].get(param).unwrap_or(&default).clone();
        params.push(p);
    }
    for p in controller_entries(doc) {
        let Some(name) = p["name"].as_str() else { continue };
        if names.insert(name.to_owned()) { params.push(p); }
    }
    for mut p in modifier_inputs::interface_entries(doc) {
        let Some(name) = p["name"].as_str().map(str::to_owned) else { continue };
        if !names.insert(name) { continue; }
        p["label"] = p["name"].clone();
        params.push(p);
    }
    // This is an opaque change token in the addon, not a mandated hashing algorithm.
    let structure: Vec<_> = params.iter().cloned().map(|mut p| { p.as_object_mut().unwrap().remove("value"); p }).collect();
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    serde_json::to_string(&structure).unwrap().hash(&mut hash);
    json!({"type":"interface","structure":format!("{:016x}",hash.finish()),"params":params})
}

/// Controller controls use their saved section order; each exposed control becomes one Blender input.
pub fn controller_entries(doc: &Value) -> Vec<Value> {
    let mut out = Vec::new();
    for (node_id, node) in doc["nodes"].as_object().into_iter().flatten() {
        if node["type"] != "controller" { continue; }
        for section in node["controller_sections"].as_array().into_iter().flatten() {
            let section_name = section["name"].as_str().unwrap_or("");
            for entry in section["parameters"].as_array().into_iter().flatten() {
                if entry["expose_to_blender"] != true { continue; }
                let name = entry["name"].as_str().unwrap_or("");
                let kind = match entry["kind"].as_str().unwrap_or("") { "vector" => "vec3", "color" => "color", k => k };
                let default = &entry["default"];
                out.push(json!({"name":format!("{node_id}_{name}"),"label":name,"node":node_id,"param":name,
                    "kind":kind,"default":default,"value":node["params"].get(name).unwrap_or(default),"section":section_name}));
            }
        }
    }
    out
}

#[cfg(test)]
mod controller_tests {
    use super::*;
    use crate::Document;

    #[test]
    fn controller_exposes_only_flagged_controls_in_saved_sections() {
        let original=json!({"version":1,"display":"driver","nodes":{"driver":{"type":"controller",
            "controller_sections":[
                {"name":"Shape","parameters":[{"name":"size","kind":"float","default":1.0,"expose_to_blender":true},{"name":"note","kind":"string","default":"hello"}]},
                {"name":"Flags","parameters":[{"name":"enabled","kind":"bool","default":true,"expose_to_blender":true}]}],
            "params":{"size":2.0}}}});
        let doc=Document::parse(original.to_string()).unwrap();
        let restored=Document::parse(doc.text().to_owned()).unwrap();
        assert_eq!(restored.value()["nodes"]["driver"]["controller_sections"],original["nodes"]["driver"]["controller_sections"]);
        let iface=link_interface(restored.value());
        assert_eq!(iface["params"].as_array().unwrap().len(),2);
        assert_eq!(iface["params"][0]["section"],"Shape");
        assert_eq!(iface["params"][1]["section"],"Flags");
        assert_eq!(iface["params"][0]["value"],json!(2.0));
        let edit=link_param_edit(restored.value(),&json!({"name":"driver_size","value":3.0})).unwrap();
        assert!(matches!(edit,Edit::Param{node,name,value} if node=="driver" && name=="size" && value==json!(3.0)));
    }
}

/// Match Python snap_float32: preserve arbitrary doubles, shorten exact float32 inputs.
pub fn snap_float32(value: &Value) -> Value {
    match value {
        Value::Number(n) if n.is_f64() => {
            let x = n.as_f64().unwrap(); let f = x as f32;
            if f.is_finite() && f as f64 == x {
                f.to_string().parse::<f64>().map_or_else(|_| value.clone(), |v| json!(v))
            } else { value.clone() }
        }
        Value::Array(xs) => Value::Array(xs.iter().map(snap_float32).collect()),
        _ => value.clone(),
    }
}

pub fn link_param_edit(doc: &Value, message: &Value) -> Result<Edit> {
    if let Some(name) = message["name"].as_str() {
        if let Some(entry) = controller_entries(doc).into_iter().find(|e| e["name"] == name) {
            let node = entry["node"].as_str().unwrap().to_owned();
            let param = entry["param"].as_str().unwrap().to_owned();
            let value = snap_float32(message.get("value").ok_or("missing value")?);
            let definition = minihou_core::controller::parameter(&doc["nodes"][&node], &param).ok_or("unknown controller parameter")?;
            if !minihou_core::controller::valid_value(definition["kind"].as_str().unwrap(), &value, &definition["options"]) { return Err("invalid controller value".into()); }
            return Ok(Edit::Param { node, name:param, value });
        }
        if let Some(entry) = modifier_inputs::interface_entries(doc).into_iter().find(|e| e["name"] == name) {
            if entry["implicit"] == true {
                // A picked Blender object names the node's object param (standalone use); the live feed wins.
                let value = message.get("value").ok_or("missing value")?;
                let value = value.as_str().ok_or("object picker value must be a name")?;
                return Ok(Edit::Param { node: entry["graph_node"].as_str().unwrap().to_owned(), name: "object".into(), value: json!(value) });
            }
            let node = entry["node"].as_str().unwrap().to_owned();
            let id = entry["id"].as_str().unwrap().to_owned();
            let value = message.get("value").ok_or("missing value")?.clone();
            let mut check = doc.clone();
            modifier_inputs::set_value(&mut check, &node, &id, value.clone())?;
            return Ok(Edit::ModifierInputValue { node, id, value });
        }
    }
    let (node, param) = if let Some(name) = message.get("name") {
        let e = doc["exposed"].as_array().into_iter().flatten()
            .find(|e| &e["name"] == name).ok_or("unknown exposed parameter")?;
        (e["node"].as_str().ok_or("invalid exposed node")?, e["param"].as_str().ok_or("invalid exposed parameter")?)
    } else {
        (message["node"].as_str().ok_or("missing node")?, message["param"].as_str().ok_or("missing param")?)
    };
    let value = snap_float32(message.get("value").ok_or("missing value")?);
    let typ = doc["nodes"][node]["type"].as_str().ok_or("no such node")?;
    let spec = minihou_core::nodes::get(typ).ok_or("unsupported node")?
        .params.iter().find(|p| p.name == param && p.supported);
    let promoted = (typ == "subnet").then(|| minihou_core::nodes::subnet_public_params(&doc["nodes"][node])
        .into_iter().find(|p| p["name"] == param)).flatten();
    let default = spec.map(|spec| (spec.default)()).or_else(|| promoted.map(|p| p["default"].clone()))
        .ok_or("unknown or unsupported parameter")?;
    let meta = param_meta(typ, param);
    let kind = meta.map_or_else(|| param_kind(&default), |m| metadata_kind(m, &default));
    let valid = match kind {
        ParamKind::Float => value.is_number(),
        ParamKind::Integer => value.as_f64().is_some_and(|x| x.fract() == 0.0),
        ParamKind::Bool => value.is_boolean(),
        ParamKind::String => value.is_string(),
        ParamKind::Vec3 => value.as_array().is_some_and(|a| a.len() == 3 && a.iter().all(Value::is_number)),
        ParamKind::Unsupported => false,
    };
    if !valid { return Err(format!("invalid value for {node}.{param}").into()); }
    if let Some(meta) = meta {
        for v in value.as_array().map_or_else(|| vec![&value], |a| a.iter().collect()) {
            if let Some(x) = v.as_f64() {
                if x < meta["min"].as_f64().unwrap_or(f64::NEG_INFINITY) || x > meta["max"].as_f64().unwrap_or(f64::INFINITY) {
                    return Err(format!("value outside limits for {node}.{param}").into());
                }
            }
        }
        if meta["kind"] == "enum" && !meta["options"].as_array().is_some_and(|a| a.contains(&value)) {
            return Err("value is not an available enum option".into());
        }
    }
    Ok(Edit::Param { node:node.into(), name:param.into(), value })
}

#[cfg(test)]
mod link_tests {
    use super::*;
    use crate::{Document, History};

    #[test]
    fn root_subnet_input_is_blender_visible_but_inner_exposure_is_not() {
        let graph=json!({"display":"sub","exposed":[{"name":"Offset","node":"sub","param":"move_translate"}],
            "nodes":{"sub":{"type":"subnet","promoted_params":[{"name":"move_translate","node":"move","param":"translate","default":[0,0,0]}],
                "inner":{"exposed":[{"name":"move_translate","node":"move","param":"translate"}],
                    "nodes":{"point":{"type":"point"},"move":{"type":"transform","inputs":{"geo":"point"}}},"output":"move"}}}});
        let params=link_interface(&graph)["params"].as_array().unwrap().clone();
        assert_eq!(params.len(),1);
        assert_eq!((params[0]["name"].as_str(),params[0]["kind"].as_str()),(Some("Offset"),Some("vec3")));
        let edit=link_param_edit(&graph,&json!({"name":"Offset","value":[2,3,4]})).unwrap();
        let mut doc=Document::parse(graph.to_string()).unwrap();
        doc.edit(&edit).unwrap();
        assert_eq!(doc.value()["nodes"]["sub"]["params"]["move_translate"],json!([2,3,4]));
        assert_eq!(doc.session().unwrap().cook(None).unwrap().p(),&[2.0,3.0,4.0]);
    }

    #[test]
    fn network_linked_parameter_edits_follow_both_directions_and_undo_together() {
        let original = json!({"display":"a","nodes":{
            "a":{"type":"box","linked_params_group":"a"},
            "b":{"type":"box","linked_params_group":"a"}
        }});
        let mut doc = Document::parse(original.to_string()).unwrap();
        let mut history = History::default();
        history.edit(&mut doc, &Edit::Param {node:"b".into(), name:"size".into(), value:json!([2,3,4])}).unwrap();
        history.finish(&doc);
        assert_eq!(doc.value()["nodes"]["a"]["params"]["size"], json!([2,3,4]));
        assert_eq!(doc.value()["nodes"]["b"]["params"]["size"], json!([2,3,4]));
        history.undo(&mut doc);
        assert!(doc.value()["nodes"]["a"]["params"].get("size").is_none());
        history.redo(&mut doc);
        assert_eq!(doc.value()["nodes"]["a"]["params"]["size"], json!([2,3,4]));
    }

    #[test]
    fn exposure_is_saved_undoable_and_has_stable_structure_on_value_changes() {
        let mut doc = Document::parse(r#"{"nodes":{"b":{"type":"box"}},"display":"b","unknown":42}"#.into()).unwrap();
        let mut history = History::default();
        let edit = toggle_exposed(doc.value(), "b", "size");
        history.edit(&mut doc, &edit).unwrap();
        history.finish(&doc);
        assert!(exposed_nodes(doc.value()).contains("b"));
        let iface = link_interface(doc.value());
        assert_eq!(iface["params"][0]["kind"], "vec3");
        let saved = Document::parse(doc.text().into()).unwrap();
        assert!(is_exposed(saved.value(), "b", "size"));
        let edit = link_param_edit(doc.value(), &json!({"name":"b_size","value":[2,3,4]})).unwrap();
        history.edit(&mut doc, &edit).unwrap(); history.finish(&doc);
        assert_eq!(iface["structure"], link_interface(doc.value())["structure"]);
        assert_eq!(doc.value()["unknown"], 42);
        history.undo(&mut doc); history.undo(&mut doc);
        assert!(!is_exposed(doc.value(), "b", "size"));
        history.redo(&mut doc); assert!(is_exposed(doc.value(), "b", "size"));
    }

    #[test]
    fn rejects_invalid_remote_parameters_and_preserves_direct_addressing() {
        let doc = Document::parse(r#"{"nodes":{"b":{"type":"box"}},"display":"b"}"#.into()).unwrap();
        assert!(link_param_edit(doc.value(), &json!({"node":"b","param":"size","value":[1,2,3]})).is_ok());
        for message in [json!({"name":"missing","value":1}), json!({"node":"gone","param":"size","value":1}),
            json!({"node":"b","param":"missing","value":1}), json!({"node":"b","param":"size","value":"bad"}),
            json!({"node":"b","param":"size"})] {
            assert!(link_param_edit(doc.value(), &message).is_err());
        }
    }

    #[test]
    fn snaps_only_exact_float32_values_recursively() {
        assert_eq!(snap_float32(&json!([0.22_f32 as f64, 0.123456789, 3, true])), json!([0.22,0.123456789,3,true]));
    }
}
