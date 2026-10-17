//! Dynamic Geometry Nodes inputs owned by a modifier_input node.
use serde_json::{json, Value};
use std::collections::HashSet;
use crate::Result;

pub const KINDS: &[&str] = minihou_core::nodes::blender_input::MODIFIER_INPUT_KINDS;

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
        let subtypes: &[&str] = match kind {
            "vector" => &["translation", "xyz", "euler", "direction", "velocity", "acceleration"],
            "float" => &["angle", "distance", "factor", "percentage", "time"],
            "int" => &["factor", "percentage"],
            "string" => &["filepath"],
            _ => &[],
        };
        if let Some(subtype) = entry.get("subtype") {
            let subtype = subtype.as_str().ok_or("input subtype must be a string")?;
            if !subtype.is_empty() && !subtypes.contains(&subtype) {
                return Err(format!("{name}: unsupported {kind} subtype {subtype:?}").into());
            }
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
            let mut unique = HashSet::new();
            if !options.iter().all(|o| o.as_str().is_some_and(|s| !s.is_empty() && unique.insert(s)))
                || !options.contains(value) {
                return Err(format!("{name}: menu default must be one of its options").into());
            }
        }
        for bound in ["min", "max"] {
            if entry.get(bound).is_some() && !entry[bound].is_number() {
                return Err(format!("{name}: {bound} must be numeric").into());
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

/// Pseudo node of implicit object inputs: every `blender_object` node in a Blender-bound graph gets an
/// Object picker on the MiniHou group, keyed `input:@picker:<stable id>` like an object Add Input.
pub const PICKER_NODE: &str = "@picker";
/// Node field pinning the picker id across node renames (absent: the node id; pinned on first rename).
pub const PICKER_ID: &str = "picker_id";

/// (node id, stable picker id) for each `blender_object` node; duplicate pinned ids (copy/paste) fall back to the node id.
pub fn pickers(doc: &Value) -> Vec<(String, String)> {
    let mut used = HashSet::new();
    let mut out = Vec::new();
    let mut nodes: Vec<_> = doc["nodes"].as_object().into_iter().flatten()
        .filter(|(_, n)| n["type"] == "blender_object").collect();
    nodes.sort_by(|a, b| a.0.cmp(b.0));
    for (node, data) in nodes {
        let pinned = data[PICKER_ID].as_str().filter(|s| !s.is_empty() && s.len() <= 128);
        let uid = match pinned { Some(p) if !used.contains(p) => p.to_owned(), _ => node.clone() };
        let uid = if used.contains(&uid) { format!("{node}~") } else { uid };
        used.insert(uid.clone());
        out.push((node.clone(), uid));
    }
    out
}

/// World matrix from an `input_references` entry (row-major 16 numbers), if present.
pub fn reference_matrix(info: &Value) -> Option<[f64; 16]> {
    let values: Vec<f64> = info["matrix"].as_array()?.iter().filter_map(Value::as_f64).collect();
    (values.len() == 16).then(|| std::array::from_fn(|i| values[i]))
}

pub fn picker_key(uid: &str) -> String { format!("input:{PICKER_NODE}:{uid}") }

/// Implicit object entries for `blender_object` nodes: named after the node, value = its object param.
pub fn picker_entries(doc: &Value) -> Vec<Value> {
    pickers(doc).into_iter().map(|(node, uid)| json!({
        "id": uid, "name": node, "kind": "object", "default": "", "node": PICKER_NODE, "graph_node": node,
        "implicit": true, "doc": "MiniHou Blender Object node: pick the object it reads",
        "value": doc["nodes"][&node]["params"]["object"].as_str().unwrap_or(""),
    })).collect()
}

/// Everything the Blender group shows besides exposed parameters: dynamic inputs, then implicit pickers.
pub fn interface_entries(doc: &Value) -> Vec<Value> {
    let mut out = entries(doc);
    out.extend(picker_entries(doc));
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
        assert_eq!(crate::param_meta("modifier_input", "__dynamic_inputs__").unwrap()["types"], json!(KINDS));
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

    #[test]
    fn blender_object_nodes_expose_stable_object_pickers() {
        let doc = json!({"display":"b","exposed":[],"nodes":{
            "b":{"type":"blender_object","params":{"object":"Cube"}},
            "c":{"type":"blender_object","picker_id":"b"},
            "input":{"type":"modifier_input","modifier_inputs":[{"id":"input_1","name":"Amount","kind":"float","default":0.0}]}}});
        assert_eq!(pickers(&doc), vec![("b".into(), "b".into()), ("c".into(), "c".into())], "pasted duplicate id falls back");
        let params = link_interface(&doc)["params"].as_array().unwrap().clone();
        let picker = params.iter().find(|p| p["name"] == "b").unwrap();
        assert_eq!((picker["kind"].as_str(), picker["node"].as_str(), picker["id"].as_str(), picker["value"].as_str()),
                   (Some("object"), Some(PICKER_NODE), Some("b"), Some("Cube")));
        assert_eq!(picker_key("b"), "input:@picker:b");
        // A pick (or clear) from Blender sets the node's object name; it is not a modifier_input value.
        let edit = link_param_edit(&doc, &json!({"name":"b","value":"Suzanne"})).unwrap();
        assert!(matches!(edit, Edit::Param { ref node, ref name, ref value } if node == "b" && name == "object" && value == "Suzanne"));
        assert!(link_param_edit(&doc, &json!({"name":"b","value":3})).is_err());
        // Plain entries (parameter-link targets) never include pickers.
        assert_eq!(entries(&doc).len(), 1);
        // Add Input avoids picker names.
        let (_, entry) = crate::input_define::plan(&doc, &json!({"input":{"name":"b","kind":"float"}})).unwrap();
        assert_eq!(entry["name"], "b 2");
    }

    #[test]
    fn menu_and_subtype_definitions_reject_unusable_blender_values() {
        let menu = json!({"id":"choice","name":"Choice","kind":"menu",
                          "options":["One","Two"],"default":"One"});
        assert!(validate(&[menu.clone()]).is_ok());
        let mut bad = menu;
        bad["default"] = json!("");
        assert!(validate(&[bad.clone()]).is_err());
        bad["default"] = json!("One");
        bad["options"] = json!(["One", "One"]);
        assert!(validate(&[bad]).is_err());
        let mut scalar = json!({"id":"angle","name":"Angle","kind":"float",
                                "subtype":"angle","default":0.5,"min":0.0});
        assert!(validate(&[scalar.clone()]).is_ok());
        scalar["subtype"] = json!("not_a_socket");
        assert!(validate(&[scalar.clone()]).is_err());
        scalar["subtype"] = json!("angle");
        scalar["min"] = json!("bad");
        assert!(validate(&[scalar]).is_err());
    }
}
