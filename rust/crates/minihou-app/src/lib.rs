//! Window-independent editor state and lossless JSON edits.
extern crate self as minihou_app;
pub mod cluster_lod;
pub mod link_protocol;
pub mod graph_embed;
pub mod http_api;
pub mod http_ops;
pub mod http_geometry;
#[cfg(test)] mod http_tests;
pub mod transform_math;
pub mod noodle_hit;
pub mod graph_edit;
pub mod modifier_inputs;
pub mod object_names;
pub mod input_define;
pub mod geo_info;
pub mod materials;
pub mod image_assets;
pub mod params;
pub mod value_link_ui;
#[cfg(test)]
use minihou_core::Geometry;
use serde_json::Value;
#[cfg(test)]
use std::sync::Arc;


pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub mod units;
pub mod inference;
pub use units::{LengthUnit, UnitSettings};
pub mod history;
pub use history::{History, HistoryOutcome, SelectionSnapshot, SelectionState};
pub mod cook_worker;
pub use cook_worker::{cook_batch, CookRequest, CookResponse, CookState, CookWorker};
pub mod camera;
pub use camera::{sanitize_clip_planes, Camera, StandardView};
pub mod remote_link;
pub use remote_link::{controller_entries, exposed_nodes, is_exposed, link_interface, link_param_edit, snap_float32, toggle_exposed};
pub mod document;
pub use document::{display_nodes, selftest, timeline_time_edit, Document, Edit, EditorDocument};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParamKind { Bool, Integer, Float, String, Vec3, Unsupported }
pub fn param_kind(default: &Value) -> ParamKind {
    match default {
        Value::Bool(_) => ParamKind::Bool,
        Value::Number(n) if n.is_i64() || n.is_u64() => ParamKind::Integer,
        Value::Number(_) => ParamKind::Float,
        Value::String(_) => ParamKind::String,
        Value::Array(a) if a.len() == 3 && a.iter().all(Value::is_number) => ParamKind::Vec3,
        _ => ParamKind::Unsupported,
    }
}


#[cfg(test)]
mod tests;

/// Generated registry metadata; unknown parameters retain the generic widget.
pub fn param_meta(node_type: &str, name: &str) -> Option<&'static Value> {
    static META: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    param_meta_from(META.get_or_init(|| serde_json::from_str(include_str!("param_meta.json")).expect("generated metadata")), node_type, name)
}
fn param_meta_from<'a>(metadata: &'a Value, node_type: &str, name: &str) -> Option<&'a Value> {
    if let Some(meta) = metadata.get(node_type).and_then(|node| node.get(name)) {
        return Some(meta);
    }
    static DECLARED: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    DECLARED.get_or_init(|| {
        let mut nodes = serde_json::Map::new();
        for spec in minihou_core::nodes::registry() {
            let mut params = serde_json::Map::new();
            for param in spec.params {
                let default = (param.default)();
                let kind = match param_kind(&default) {
                    ParamKind::Bool => "bool",
                    ParamKind::Integer => "int",
                    ParamKind::Float => "float",
                    ParamKind::String => "str",
                    ParamKind::Vec3 => "vec3",
                    ParamKind::Unsupported => "unsupported",
                };
                params.insert(param.name.to_owned(), serde_json::json!({
                    "default": default, "kind": kind, "doc": ""
                }));
            }
            nodes.insert(spec.name.to_owned(), Value::Object(params));
        }
        Value::Object(nodes)
    }).get(node_type)?.get(name)
}
#[cfg(test)]
#[test]
fn missing_metadata_uses_declared_parameter() {
    let declared = minihou_core::nodes::get("grid").unwrap().params.iter()
        .find(|param| param.name == "columns").unwrap();
    let empty = serde_json::json!({});
    let fallback = param_meta_from(&empty, "grid", "columns").unwrap();
    assert_eq!(fallback["default"], (declared.default)());
    assert_eq!(fallback["kind"], "int");
    assert!(param_meta_from(&empty, "grid", "unknown").is_none());
}
#[cfg(test)]
#[test]
fn foreach_method_and_domain_metadata() {
    assert_eq!(param_meta("for_each", "method").unwrap()["options"],
        serde_json::json!(["connected", "attribute", "domain"]));
    assert_eq!(param_meta("for_each", "domain").unwrap()["default"], "points");
    assert_eq!(param_meta("for_each", "domain").unwrap()["options"],
        serde_json::json!(["points", "edges", "faces", "instances"]));
}
pub fn metadata_kind(meta: &Value, default: &Value) -> ParamKind {
    match meta["kind"].as_str() {
        Some("int") => ParamKind::Integer,
        Some("float") => ParamKind::Float,
        Some("bool") => ParamKind::Bool,
        Some("str" | "enum") => ParamKind::String,
        Some("vec3") => ParamKind::Vec3,
        _ => param_kind(default),
    }
}
pub fn slider_range(meta: &Value) -> Option<std::ops::RangeInclusive<f64>> {
    Some(meta["min"].as_f64()?..=meta["max"].as_f64().or_else(|| meta["soft_max"].as_f64())?)
}
/// soft_max affects presentation only, never the accepted value.
pub fn clamp_parameter(meta: &Value, value: f64) -> f64 {
    value.max(meta["min"].as_f64().unwrap_or(f64::NEG_INFINITY))
        .min(meta["max"].as_f64().unwrap_or(f64::INFINITY))
}
