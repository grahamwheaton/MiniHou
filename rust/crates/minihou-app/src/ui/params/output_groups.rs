//! Output group target buttons, fields, toggles, and metadata heuristics.
use eframe::egui::{self, Color32, Stroke};
use minihou_core::nodes;
use serde_json::{json, Value};

/// Output group target button and its viewport highlight: teal, distinct from purple (input) and orange (object selection).
pub const TARGET_TEAL: Color32 = Color32::from_rgb(38, 205, 200);

pub(crate) fn is_string_meta(meta: Option<&Value>) -> bool {
    meta.is_some_and(|m| matches!(m["kind"].as_str(), Some("str" | "string")))
}

pub(crate) fn explicit_pick(meta: Option<&Value>) -> bool {
    meta.is_some_and(|m| m.get("pick_domain").is_some() || m.get("pick_domain_param").is_some())
}

/// One rule for output group fields: string fields that name a group the node writes (outgrp_*_name, output_group, ...).
pub fn is_output_group_field(name: &str, meta: Option<&Value>) -> bool {
    if !is_string_meta(meta) || explicit_pick(meta) { return false; }
    let named = (name.starts_with("outgrp_") && name.ends_with("_name"))
        || matches!(name, "output_group" | "output_group_name" | "output_inner_group_name" | "result_group_name" | "patch_group" | "patchgroup"
            | "edge_loop_group" | "edgeloopgroup" | "fill_group" | "ray_hit_group" | "inside_group");
    named || meta.is_some_and(|m| m["doc"].as_str().is_some_and(|d| d.to_lowercase().contains("group that receives")))
}

/// Find the existing Boolean that controls a named output group, if the node has one.
/// String-only outputs use an empty name as their off state.
pub fn output_group_toggle(typ: &str, name: &str, meta: Option<&Value>) -> Option<&'static str> {
    let spec = nodes::get(typ)?;
    let by_name = name.strip_suffix("_name").filter(|candidate| candidate.starts_with("outgrp_") || candidate.starts_with("output_"));
    let by_meta = meta.and_then(|m| m["enabled_if"].as_object()).and_then(|conditions| {
        conditions.iter().find(|(key, value)| value.as_bool() == Some(true)
            && (key.contains("group") || key.as_str() == "create_point_group"))
            .map(|(key, _)| key.as_str())
    });
    by_name.into_iter().chain(by_meta).find_map(|candidate| {
        spec.params.iter().find(|param| param.name == candidate && (param.default)().is_boolean()).map(|param| param.name)
    })
}

pub fn output_group_label(name: &str) -> String {
    let name = name.strip_prefix("outgrp_").unwrap_or(name);
    let name = name.strip_suffix("_name").unwrap_or(name);
    if let Some(stem) = name.strip_suffix("polys") {
        return format!("{} Polygons", output_group_label(stem));
    }
    for (suffix, word) in [("points", "Points"), ("edges", "Edges"), ("caps", "Caps"), ("cap", "Cap")] {
        if let Some(stem) = name.strip_suffix(suffix).filter(|stem| !stem.is_empty()) {
            return format!("{} {word}", output_group_label(stem));
        }
    }
    if let Some(stem) = name.strip_suffix("fillet") {
        return format!("{} Fillet", output_group_label(stem));
    }
    super::title(name)
}

pub fn target_field(ui: &mut egui::Ui, value: &mut Value, width: f32, enabled: bool) -> (bool, bool) {
    let mut text = value.as_str().unwrap_or_default().to_owned();
    let edit = ui.add_sized([(width - 28.).max(8.), 22.], egui::TextEdit::singleline(&mut text));
    if edit.changed() { *value = json!(text); }
    let button = ui.add_enabled(enabled, egui::Button::new(egui::RichText::new("◎").color(TARGET_TEAL))
        .fill(ui.visuals().widgets.inactive.bg_fill).stroke(Stroke::new(1.0_f32, TARGET_TEAL)))
        .on_hover_text("Show this output group in the viewport; click to select it");
    (button.clicked(), button.hovered())
}

#[cfg(test)]
mod tests {
    use super::*;
    use minihou_app::param_meta;

    #[test]
    fn output_rule_adds_target_to_output_group_names() {
        for (n, f) in [("flood_fill","output_group"),("polyfill","patch_group"),("polyfill","fill_group"),("ray","ray_hit_group"),("grow","result_group_name"),("voronoi_fracture","inside_group"),("select_loop","group_name")] {
            assert!(is_output_group_field(f, param_meta(n, f)), "{n}.{f} needs the target");
        }
        assert!(is_output_group_field("outgrp_shared_name", Some(&json!({"kind":"str"}))));
        assert!(!is_output_group_field("group", param_meta("blast", "group")));
        assert!(!is_output_group_field("outgrp_cap", Some(&json!({"kind":"bool"}))));
        assert!(!is_output_group_field("leaf_param_a", param_meta("lsystem", "leaf_param_a")));
    }

    #[test]
    fn output_group_rows_use_existing_enable_parameters() {
        for (node, field, toggle) in [
            ("polybevel", "outgrp_pointfilletpolys_name", "outgrp_pointfilletpolys"),
            ("outset", "output_group_name", "output_group"),
            ("outset", "output_inner_group_name", "output_inner_group"),
            ("ray", "ray_hit_group", "create_point_group"),
        ] {
            assert_eq!(output_group_toggle(node, field, param_meta(node, field)), Some(toggle));
        }
        assert_eq!(output_group_toggle("polybevel", "output_group", param_meta("polybevel", "output_group")), None);
        assert_eq!(output_group_label("outgrp_pointfilletpolys_name"), "Point Fillet Polygons");
    }

    #[test]
    fn target_colour_differs_from_purple_and_orange() {
        assert_ne!(TARGET_TEAL, crate::ui::params::PICK_PURPLE);
        assert_ne!(TARGET_TEAL, crate::ui::params::SELECT_ORANGE);
        assert!(TARGET_TEAL.g() > TARGET_TEAL.r() && TARGET_TEAL.b() > TARGET_TEAL.r());
    }
}
