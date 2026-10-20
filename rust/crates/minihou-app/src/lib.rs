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
use minihou_core::{session::Session, lod::ClusterLod, Geometry, Graph};
use serde_json::{json, Value};
use std::{collections::HashMap, ops::Range, path::Path, sync::Arc, time::Instant};

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
    META.get_or_init(|| serde_json::from_str(include_str!("param_meta.json")).expect("generated metadata"))
        .get(node_type)?.get(name)
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
