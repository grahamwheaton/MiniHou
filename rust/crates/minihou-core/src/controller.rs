//! Saved, ordered Controller sections and their typed parameters.
use serde_json::{Map, Value};
use std::collections::HashSet;
use crate::{Error, Result};

pub const KINDS: &[&str] = &["float", "int", "bool", "vector", "color", "string", "menu"];

pub fn parameters(node: &Value) -> Vec<&Value> {
    node["controller_sections"].as_array().into_iter().flatten()
        .flat_map(|section| section["parameters"].as_array().into_iter().flatten()).collect()
}

pub fn parameter<'a>(node: &'a Value, name: &str) -> Option<&'a Value> {
    parameters(node).into_iter().find(|p| p["name"] == name)
}

pub fn default(kind: &str) -> Value {
    match kind {
        "float" => serde_json::json!(0.0), "int" => serde_json::json!(0),
        "bool" => serde_json::json!(false), "vector" => serde_json::json!([0.0,0.0,0.0]),
        "color" => serde_json::json!([1.0,1.0,1.0,1.0]), _ => serde_json::json!(""),
    }
}

pub fn valid_value(kind: &str, value: &Value, options: &Value) -> bool {
    match kind {
        "float" => value.is_number(), "int" => value.as_i64().is_some(), "bool" => value.is_boolean(),
        "vector" | "color" => value.as_array().is_some_and(|a| a.len() == if kind == "vector" {3} else {4} && a.iter().all(Value::is_number)),
        "string" => value.is_string(),
        "menu" => value.as_str().is_some_and(|s| options.as_array().is_some_and(|a| !a.is_empty() && a.iter().any(|x| x.as_str() == Some(s)))),
        _ => false,
    }
}

pub fn validate(node: &Value) -> Result<()> {
    let empty = Vec::new();
    let sections = if node.get("controller_sections").is_none() { &empty }
        else { node["controller_sections"].as_array().ok_or_else(|| Error::new("controller_sections must be an array"))? };
    let mut names = HashSet::new();
    let mut sections_seen = HashSet::new();
    for section in sections {
        let title = section["name"].as_str().filter(|s| !s.trim().is_empty()).ok_or_else(|| Error::new("controller section needs a name"))?;
        if !sections_seen.insert(title) { return Err(Error::new("duplicate controller section")); }
        let entries = section["parameters"].as_array().ok_or_else(|| Error::new("controller section needs parameters"))?;
        for entry in entries {
            let name = entry["name"].as_str().filter(|s| !s.is_empty() && s.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
                .ok_or_else(|| Error::new("controller parameter needs an identifier"))?;
            if !names.insert(name) { return Err(Error::new("duplicate controller parameter")); }
            let kind = entry["kind"].as_str().ok_or_else(|| Error::new("controller parameter needs a type"))?;
            if !KINDS.contains(&kind) || !valid_value(kind, &entry["default"], &entry["options"]) { return Err(Error::new(format!("invalid controller parameter {name}"))); }
            if let Some(flag) = entry.get("expose_to_blender") { if !flag.is_boolean() { return Err(Error::new("expose_to_blender must be a boolean")); } }
            if entry["expose_to_blender"] == true && !["float", "int", "bool", "vector", "color"].contains(&kind) {
                return Err(Error::new(format!("controller parameter {name} has no Blender socket")));
            }
        }
    }
    for (name, value) in node["params"].as_object().into_iter().flatten() {
        let entry = parameter(node, name).ok_or_else(|| Error::new(format!("unknown controller parameter {name}")))?;
        if !valid_value(entry["kind"].as_str().unwrap(), value, &entry["options"]) { return Err(Error::new(format!("invalid controller value {name}"))); }
    }
    Ok(())
}

pub fn values(node: &Value, given: Option<&Map<String, Value>>) -> Map<String, Value> {
    let mut values = Map::new();
    for entry in parameters(node) {
        let name = entry["name"].as_str().unwrap();
        values.insert(name.into(), given.and_then(|m| m.get(name)).cloned().unwrap_or_else(|| entry["default"].clone()));
    }
    values
}
