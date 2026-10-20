//! Pick domain mapping, element mode heuristics, and viewport selection pick fields.
use eframe::egui::{self, Color32, Stroke};
use minihou_app::param_meta;
use serde_json::{json, Value};
use crate::ui::SelectionMode;
use super::output_groups::{explicit_pick, is_output_group_field, is_string_meta};

pub const PICK_PURPLE: Color32 = Color32::from_rgb(184, 82, 245);
/// Orange used for object selection and the barrier group.
pub const SELECT_ORANGE: Color32 = Color32::from_rgb(0xff, 0xa0, 0x28);

/// One rule for input group fields: string fields that select elements by group, pattern, or an element list.
pub fn is_input_group_field(name: &str, meta: Option<&Value>) -> bool {
    if explicit_pick(meta) { return true; }
    if (meta.is_some() && !is_string_meta(meta)) || is_output_group_field(name, meta) { return false; }
    let by_name = matches!(name, "group" | "group2" | "groups" | "group_name" | "pattern") || name.ends_with("_group") || name.ends_with("_groups");
    let selection_list = matches!(name, "points" | "edges" | "faces" | "exclude_edges")
        && meta.is_some_and(|m| m["doc"].as_str().is_some_and(|d| d.to_lowercase().contains("selection")));
    by_name || selection_list
}

fn name_domain(name: &str, meta: Option<&Value>) -> SelectionMode {
    let doc = meta.and_then(|m| m["doc"].as_str()).unwrap_or("").to_lowercase();
    if name.contains("edge") || doc.contains("edge") { SelectionMode::Edges }
    else if name.contains("face") || doc.contains("primitive") { SelectionMode::Faces }
    else { SelectionMode::Points }
}

/// The registry describes the element domain; dynamic nodes read their domain parameter.
pub fn pick_domain(meta: Option<&Value>, params: &Value) -> Option<SelectionMode> {
    let meta = meta?;
    let domain = meta["pick_domain_param"].as_str()
        .and_then(|name| params[name].as_str().or_else(|| meta["pick_domain_default"].as_str()))
        .or_else(|| meta["pick_domain"].as_str())?;
    match domain {
        "points" | "point" => Some(SelectionMode::Points),
        "edges" | "edge" => Some(SelectionMode::Edges),
        "primitives" | "primitive" | "faces" | "prim" => Some(SelectionMode::Faces),
        _ => None,
    }
}

/// The element mode a `domain`/`group_type` enum edit switches to, so a viewport selection can be promoted with it.
pub fn domain_edit_mode(name: &str, value: &Value) -> Option<SelectionMode> {
    if !matches!(name, "domain" | "group_type") { return None; }
    pick_domain(Some(&json!({"pick_domain": value.as_str()?})), &Value::Null)
}

/// Fallback for group fields without registry metadata: follow a sibling domain or group_type parameter.
pub fn pick_domain_for(typ: &str, name: &str, meta: Option<&Value>, params: &Value) -> Option<SelectionMode> {
    let declared = pick_domain(meta, params);
    if typ == "attribute_create" && name == "group" {
        if let Some(class) = params["attributes"].as_array().and_then(|a| a.first()).and_then(|a| a["class"].as_str()) {
            return pick_domain(Some(&json!({"pick_domain":class})), params);
        }
    }
    if let Some(mode) = declared { return Some(mode); }
    if !is_input_group_field(name, meta) { return None; }
    if !matches!(name, "group" | "group_name" | "pattern") { return Some(name_domain(name, meta)); }
    for sibling in ["domain", "group_type"] {
        let Some(m) = param_meta(typ, sibling) else { continue };
        let value = params[sibling].as_str().or_else(|| m["default"].as_str());
        if let Some(mode) = value.and_then(|d| pick_domain(Some(&json!({"pick_domain": d})), params)) { return Some(mode); }
    }
    Some(name_domain(name, meta))
}

pub fn pick_field(ui: &mut egui::Ui, value: &mut Value, width: f32, active: bool, enabled: bool) -> (bool, bool) {
    let mut text = value.as_str().unwrap_or_default().to_owned();
    let edit = ui.add_sized([(width - 28.).max(8.), 22.], egui::TextEdit::singleline(&mut text));
    if edit.changed() { *value = json!(text); }
    let arrow = ui.add_enabled(enabled, egui::Button::new(egui::RichText::new("↖").color(PICK_PURPLE))
        .fill(if active { PICK_PURPLE.gamma_multiply(0.27) } else { ui.visuals().widgets.inactive.bg_fill })
        .stroke(Stroke::new(1.0_f32, PICK_PURPLE))).on_hover_text("Pick elements in viewport (Enter to finish, Esc to cancel)");
    (arrow.clicked(), arrow.hovered())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn params_domain_edit_maps_to_selection_mode() {
        assert_eq!(domain_edit_mode("domain", &json!("faces")), Some(SelectionMode::Faces));
        assert_eq!(domain_edit_mode("group_type", &json!("edges")), Some(SelectionMode::Edges));
        assert_eq!(domain_edit_mode("domain", &json!("points")), Some(SelectionMode::Points));
        assert_eq!(domain_edit_mode("steps", &json!("faces")), None);
        assert_eq!(domain_edit_mode("domain", &json!(3)), None);
    }

    #[test]
    fn flood_fill_group_fields_have_pick_arrow_following_domain() {
        for f in ["seed_group","avoid_group","allowed_group","barrier_group","target_group"] {
            assert_eq!(pick_domain(param_meta("flood_fill", f), &json!({})), Some(SelectionMode::Points), "{f}");
            assert_eq!(pick_domain(param_meta("flood_fill", f), &json!({"domain":"edges"})), Some(SelectionMode::Edges), "{f}");
            assert_eq!(pick_domain(param_meta("flood_fill", f), &json!({"domain":"faces"})), Some(SelectionMode::Faces), "{f}");
        }
    }

    #[test]
    fn group_create_edges_domain_picks_edges() {
        let meta = param_meta("group_create", "pattern");
        assert_eq!(meta.unwrap()["pick_domain_param"], "domain");
        assert_eq!(pick_domain_for("group_create", "pattern", meta, &json!({"domain":"edges"})), Some(SelectionMode::Edges));
        assert_eq!(param_meta("blast", "group").unwrap()["pick_domain_default"], "primitives");
        assert_eq!(pick_domain_for("blast", "group", param_meta("blast", "group"), &json!({"domain":"points"})), Some(SelectionMode::Points));
    }

    #[test]
    fn declared_picker_domain_precedes_name_and_sibling_heuristics() {
        let declared = json!({"kind":"str", "pick_domain":"edges"});
        assert!(is_input_group_field("selection", Some(&declared)));
        assert_eq!(pick_domain_for("blast", "group", Some(&declared), &json!({"domain":"points"})), Some(SelectionMode::Edges));
        let undeclared = json!({"kind":"str"});
        assert_eq!(pick_domain_for("blast", "group", Some(&undeclared), &json!({"domain":"points"})), Some(SelectionMode::Points));
        assert_eq!(pick_domain_for("other", "edge_group", Some(&undeclared), &json!({})), Some(SelectionMode::Edges));
    }

    #[test]
    fn registry_maps_group_and_crease_fields() {
        assert_eq!(pick_domain(param_meta("subdivide", "creases"), &json!({})), Some(SelectionMode::Edges));
        assert_eq!(pick_domain(param_meta("polyextrude", "group"), &json!({})), Some(SelectionMode::Faces));
        assert_eq!(pick_domain(param_meta("transform", "group"), &json!({})), Some(SelectionMode::Points));
        assert_eq!(pick_domain(param_meta("polybevel", "group"), &json!({"group_type":"edges"})), Some(SelectionMode::Edges));
        assert_eq!(pick_domain(param_meta("polybevel", "group"), &json!({"group_type":"primitives"})), Some(SelectionMode::Faces));
        assert_eq!(pick_domain(param_meta("attribute_create", "group"), &json!({})), Some(SelectionMode::Points));
        assert_eq!(pick_domain(param_meta("attribute_create", "group"), &json!({"domain":"points"})), Some(SelectionMode::Points));
        assert_eq!(pick_domain(param_meta("attribute_create", "group"), &json!({"domain":"primitives"})), Some(SelectionMode::Faces));
    }

    #[test]
    fn group_fields_of_many_nodes_get_picker() {
        assert_eq!(pick_domain(param_meta("blast", "group"), &json!({})), Some(SelectionMode::Faces));
        assert_eq!(pick_domain(param_meta("blast", "group"), &json!({"domain":"points"})), Some(SelectionMode::Points));
        assert_eq!(pick_domain(param_meta("blast", "group"), &json!({"domain":"edges"})), Some(SelectionMode::Edges));
        assert_eq!(pick_domain(param_meta("group_create", "pattern"), &json!({"domain":"primitives"})), Some(SelectionMode::Faces));
        assert_eq!(pick_domain(param_meta("edge_slide", "group"), &json!({})), Some(SelectionMode::Edges));
        assert_eq!(pick_domain(param_meta("bend", "group"), &json!({})), Some(SelectionMode::Points));
        assert_eq!(pick_domain_for("blast", "group", None, &json!({"domain":"edges"})), Some(SelectionMode::Edges));
    }

    #[test]
    fn attribute_create_group_field_gets_picker() {
        assert_eq!(pick_domain(param_meta("attribute_create", "group"), &json!({})), Some(SelectionMode::Points));
        assert_eq!(pick_domain(param_meta("attribute_create", "group"), &json!({"domain":"points"})), Some(SelectionMode::Points));
        assert_eq!(pick_domain(param_meta("attribute_create", "group"), &json!({"domain":"primitives"})), Some(SelectionMode::Faces));
        assert_eq!(pick_domain_for("attribute_create", "group", param_meta("attribute_create", "group"),
            &json!({"attributes":[{"class":"primitives"}]})), Some(SelectionMode::Faces));
    }

    #[test]
    fn input_rule_adds_arrow_to_group_fields_only() {
        for (n, f) in [("blast","group"),("transform","group"),("group_create","pattern"),("ray","collision_group"),("polyfill","boundary_group"),
                       ("measure_distance","group2"),("assemble","groups"),("polyextrude","edges"),("polyextrude","points")] {
            let m = param_meta(n, f);
            assert!(pick_domain_for(n, f, m, &json!({})).is_some(), "{n}.{f} needs the arrow");
        }
        for (n, f) in [("blast","negate"),("curve","points"),("polyfill","patch_group"),("flood_fill","output_group"),("assemble","output_attrib"),("polybevel","group_type")] {
            assert!(pick_domain_for(n, f, param_meta(n, f), &json!({})).is_none(), "{n}.{f} must not get an arrow");
        }
    }

    #[test]
    fn arrow_ink_is_purple() {
        assert_eq!(PICK_PURPLE, Color32::from_rgb(184, 82, 245));
    }
}
