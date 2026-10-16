//! Dynamic Geometry Nodes inputs owned by a modifier_input node.
use serde_json::{json, Value};
use std::collections::HashSet;
use crate::Result;

pub const KINDS: &[&str] = &[
    "float", "int", "bool", "vector", "color", "rotation", "matrix", "string", "menu",
    "object", "collection", "material", "image", "texture", "geometry", "bundle", "closure",
];

pub fn default_for(kind: &str) -> Value {
    match kind {
        "float" => json!(0.0), "int" => json!(0), "bool" => json!(false),
        "vector" | "rotation" => json!([0.0, 0.0, 0.0]),
        "color" => json!([1.0, 1.0, 1.0, 1.0]),
        "matrix" => json!([1.,0.,0.,0.,0.,1.,0.,0.,0.,0.,1.,0.,0.,0.,0.,1.]),
        _ => json!(""),
    }
}

pub fn validate(inputs: &[Value]) -> Result<()> {
    if inputs.len() > 128 { return Err("modifier_input supports at most 128 inputs".into()); }
    let mut ids = HashSet::new();
    let mut names = HashSet::new();
    for entry in inputs {
        let id = entry["id"].as_str().filter(|s| !s.is_empty() && s.len() <= 128).ok_or("input needs a stable id")?;
        let name = entry["name"].as_str().filter(|s| !s.trim().is_empty() && s.len() <= 128).ok_or("input needs a name")?;
        if name == "Geometry" || name == "Source Object" || name.starts_with("Object: ") {
            return Err(format!("{name:?} is reserved by the Blender group").into());
        }
        let kind = entry["kind"].as_str().ok_or("input needs a type")?;
        if !KINDS.contains(&kind) { return Err(format!("unsupported Blender input type {kind:?}").into()); }
        if !ids.insert(id) || !names.insert(name) { return Err("duplicate input id or name".into()); }
        let value = entry.get("default").ok_or("input needs a default")?;
        if kind == "vector" && !entry["subtype"].as_str().unwrap_or("").is_empty()
            && !["translation", "xyz", "euler", "direction"].contains(&entry["subtype"].as_str().unwrap()) {
            return Err(format!("{name}: unsupported vector subtype").into());
        }
        let valid = match kind {
            "float" => value.is_number(), "int" => value.as_i64().is_some(), "bool" => value.is_boolean(),
            "vector" | "rotation" => number_array(value, 3), "color" => number_array(value, 4),
            "matrix" => number_array(value, 16),
            _ => value.is_string() || value.is_null(),
        };
        if !valid { return Err(format!("invalid default for {name} ({kind})").into()); }
        if kind == "menu" {
            let options = entry["options"].as_array().filter(|a| !a.is_empty()).ok_or("menu input needs options")?;
            if !options.iter().all(Value::is_string) || (value != "" && !options.contains(value)) {
                return Err(format!("{name}: menu default must be one of its options").into());
            }
        }
        if let (Some(min), Some(max)) = (entry["min"].as_f64(), entry["max"].as_f64()) {
            if min > max { return Err(format!("{name}: minimum exceeds maximum").into()); }
        }
        for component in value.as_array().map_or_else(|| vec![value], |a| a.iter().collect()) {
            if let Some(number) = component.as_f64() {
                if number < entry["min"].as_f64().unwrap_or(f64::NEG_INFINITY)
                    || number > entry["max"].as_f64().unwrap_or(f64::INFINITY) {
                    return Err(format!("{name}: value outside input limits").into());
                }
            }
        }
    }
    Ok(())
}

pub fn validate_document(doc: &Value) -> Result<()> {
    let mut names = HashSet::new();
    for exposed in doc["exposed"].as_array().into_iter().flatten() {
        if let Some(name) = exposed["name"].as_str() { names.insert(name.to_owned()); }
    }
    if let Some(nodes) = doc["nodes"].as_object() {
        for (node, data) in nodes {
            if data["type"] != "modifier_input" { continue; }
            let inputs = data["modifier_inputs"].as_array().map(Vec::as_slice).unwrap_or(&[]);
            validate(inputs).map_err(|e| format!("node {node:?}: {e}"))?;
            for entry in inputs {
                let name = entry["name"].as_str().unwrap();
                if !names.insert(name.to_owned()) {
                    return Err(format!("duplicate Blender input name {name:?}").into());
                }
            }
        }
    }
    Ok(())
}

fn number_array(value: &Value, count: usize) -> bool {
    value.as_array().is_some_and(|a| a.len() == count && a.iter().all(Value::is_number))
}

pub fn entries(doc: &Value) -> Vec<Value> {
    let mut out = Vec::new();
    if let Some(nodes) = doc["nodes"].as_object() {
        for (node, data) in nodes {
            if data["type"] != "modifier_input" { continue; }
            for input in data["modifier_inputs"].as_array().into_iter().flatten() {
                let mut item = input.clone();
                item["node"] = json!(node);
                let id = input["id"].as_str().unwrap_or("");
                item["value"] = data["input_values"].get(id).unwrap_or(&input["default"]).clone();
                out.push(item);
            }
        }
    }
    out
}

pub fn set_value(doc: &mut Value, node: &str, id: &str, value: Value) -> Result<()> {
    let defs = doc["nodes"][node]["modifier_inputs"].as_array().ok_or("unknown modifier input node")?;
    let def = defs.iter().find(|e| e["id"] == id).ok_or("unknown modifier input")?;
    let mut check = def.clone(); check["default"] = value.clone();
    validate(&[check])?;
    doc["nodes"][node]["input_values"][id] = value;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Document, Edit, link_interface, link_param_edit};

    #[test]
    fn stable_input_ids_survive_rename_and_value_edits() {
        let mut doc = Document::parse(json!({"display":"input","nodes":{"input":{"type":"modifier_input"}}}).to_string()).unwrap();
        let input = json!({"id":"a","name":"Amount","kind":"float","default":1.0,"min":0.0});
        doc.edit(&Edit::ModifierInputs { node:"input".into(), inputs:vec![input.clone()] }).unwrap();
        let edit = link_param_edit(doc.value(), &json!({"name":"Amount","value":2.5})).unwrap();
        doc.edit(&edit).unwrap();
        assert_eq!(link_interface(doc.value())["params"][0]["value"], json!(2.5));
        let mut renamed = input;
        renamed["name"] = json!("Size");
        doc.edit(&Edit::ModifierInputs { node:"input".into(), inputs:vec![renamed] }).unwrap();
        assert_eq!(link_interface(doc.value())["params"][0]["id"], "a");
        assert_eq!(link_interface(doc.value())["params"][0]["value"], json!(2.5));
        assert!(link_param_edit(doc.value(), &json!({"name":"Size","value":-1})).is_err());
    }

    #[test]
    fn parameter_links_resolve_at_graph_parse_and_missing_inputs_error() {
        let graph = json!({"display":"move","nodes":{
            "input":{"type":"modifier_input","modifier_inputs":[{"id":"offset","name":"Offset","kind":"vector","default":[0,0,0]}],
                     "input_values":{"offset":[3,4,5]}},
            "move":{"type":"transform","inputs":{"geo":"input"},"params":{"translate":{"modifier_input":{"node":"input","id":"offset"}}}}
        }});
        let parsed = minihou_core::Graph::from_value(&graph).unwrap();
        assert_eq!(parsed.nodes.iter().find(|(name,_)| name == "move").unwrap().1.params.as_ref().unwrap()["translate"], json!([3,4,5]));
        assert_eq!(parsed.to_value()["nodes"]["move"]["params"]["translate"], graph["nodes"]["move"]["params"]["translate"]);
        let mut broken = graph;
        broken["nodes"]["input"]["modifier_inputs"] = json!([]);
        assert!(minihou_core::Graph::from_value(&broken).err().unwrap().to_string().contains("is missing"));
    }

    #[test]
    fn removing_input_resets_its_parameter_links_and_discards_stale_value() {
        let mut doc = Document::parse(json!({"display":"move","nodes":{
            "input":{"type":"modifier_input","modifier_inputs":[{"id":"offset","name":"Offset","kind":"vector","default":[0,0,0]}],
                     "input_values":{"offset":[3,4,5]}},
            "move":{"type":"transform","inputs":{"geo":"input"},"params":{"translate":{"modifier_input":{"node":"input","id":"offset"}}}}
        }}).to_string()).unwrap();
        doc.edit(&Edit::ModifierInputs { node:"input".into(), inputs:vec![] }).unwrap();
        assert_eq!(doc.value()["nodes"]["move"]["params"]["translate"], json!([0.,0.,0.]));
        assert_eq!(doc.value()["nodes"]["input"]["input_values"], json!({}));
        assert!(minihou_core::Graph::from_value(doc.value()).is_ok());
    }
}
