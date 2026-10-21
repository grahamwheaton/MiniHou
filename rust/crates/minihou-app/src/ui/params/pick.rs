//! Pick domain mapping, element mode heuristics, and viewport selection pick fields.
use eframe::egui::{self, Color32, Stroke};
use minihou_app::param_meta;
use serde_json::{json, Value};
use crate::ui::SelectionMode;
use super::output_groups::{explicit_pick, is_output_group_field, is_string_meta};
use super::group_fuzzy;

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

pub fn is_input_attribute_field(name: &str, meta: Option<&Value>) -> bool {
    if meta.is_some_and(|m| !is_string_meta(Some(m))) { return false; }
    !matches!(name, "output_attrib" | "output_attribute")
        && (matches!(name, "attribute" | "attributes" | "attrib")
            || name.ends_with("_attribute") || name.ends_with("_attrib"))
}

pub fn input_attributes(g: &minihou_core::Geometry) -> Vec<String> {
    let mut names = std::collections::BTreeSet::new();
    for table in [&g.point, &g.vertex, &g.edge, &g.prim] {
        for name in table.keys() {
            if !name.starts_with("group_") && !name.starts_with("edge_group_") { names.insert(name.clone()); }
        }
    }
    names.extend(g.detail.keys().filter(|name| !name.starts_with("group_")).cloned());
    names.into_iter().collect()
}

pub fn attribute_field(ui: &mut egui::Ui, value: &mut Value, width: f32, attributes: impl FnOnce() -> Vec<String>) {
    let mut text = value.as_str().unwrap_or_default().to_owned();
    if ui.add_sized([(width - 38.).max(8.), 22.], egui::TextEdit::singleline(&mut text)).changed() { *value = json!(text); }
    group_fuzzy::dropdown(ui, value, "attributes", attributes);
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

/// Group attributes on the cooked input, filtered to the field's element domain.
pub fn input_groups(g: &minihou_core::Geometry, mode: SelectionMode) -> Vec<String> {
    let mut names = std::collections::BTreeSet::new();
    let tables: &[&minihou_core::geometry::AttrTable] = match mode {
        SelectionMode::Points => &[&g.point],
        SelectionMode::Edges => &[&g.edge, &g.vertex],
        SelectionMode::Faces => &[&g.prim],
        SelectionMode::Object => &[],
    };
    for table in tables {
        for key in table.keys() {
            if let Some(name) = key.strip_prefix("group_").or_else(||
                (mode == SelectionMode::Edges).then(|| key.strip_prefix("edge_group_")).flatten()) {
                if !name.is_empty() { names.insert(name.to_owned()); }
            }
        }
    }
    names.into_iter().collect()
}

fn filtered_group_names<'a>(names: &'a [String], search: &str) -> Vec<&'a str> {
    group_fuzzy::ranked_matches(names.iter().map(String::as_str), search)
}

// The dropdown, picker, and two gaps occupy roughly 76 points. Leave a little
// slack so the field does not paint over the row's reset button.
fn group_text_width(width: f32) -> f32 { (width - 80.).max(8.) }

pub fn pick_field(ui: &mut egui::Ui, value: &mut Value, width: f32, active: bool, enabled: bool,
    groups: impl FnOnce() -> Vec<String>) -> (bool, bool) {
    let mut text = value.as_str().unwrap_or_default().to_owned();
    let edit = ui.add_sized([group_text_width(width), 22.], egui::TextEdit::singleline(&mut text)
        .hint_text("Group name or wildcard"));
    if edit.changed() { *value = json!(text); }
    ui.add_enabled_ui(enabled, |ui| group_fuzzy::dropdown(ui, value, "groups", groups));
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
    fn path_loop_and_ring_group_fields_use_the_purple_picker() {
        assert_eq!(pick_domain_for("select_path", "waypoints", param_meta("select_path", "waypoints"), &json!({"domain":"points"})), Some(SelectionMode::Points));
        assert_eq!(pick_domain_for("select_path", "waypoints", param_meta("select_path", "waypoints"), &json!({"domain":"edges"})), Some(SelectionMode::Edges));
        assert_eq!(pick_domain_for("select_path", "waypoints", param_meta("select_path", "waypoints"), &json!({"domain":"primitives"})), Some(SelectionMode::Faces));
        for node in ["select_loop", "select_ring"] {
            assert_eq!(pick_domain_for(node, "seeds", param_meta(node, "seeds"), &json!({"domain":"primitives"})), Some(SelectionMode::Edges));
        }
    }

    #[test]
    fn arrow_ink_is_purple() {
        assert_eq!(PICK_PURPLE, Color32::from_rgb(184, 82, 245));
    }

    #[test]
    fn input_group_menu_uses_the_selected_domain() {
        let mut g = minihou_core::Geometry::points_only(vec![0., 0., 0.]).unwrap();
        use minihou_core::geometry::Attr;
        use std::sync::Arc;
        g.point.insert("group_points".into(), Arc::new(Attr::i32(1, vec![1])));
        g.edge.insert("edge_group_edges".into(), Arc::new(Attr::i32(1, vec![])));
        g.prim.insert("group_faces".into(), Arc::new(Attr::i32(1, vec![])));
        assert_eq!(input_groups(&g, SelectionMode::Points), ["points"]);
        assert_eq!(input_groups(&g, SelectionMode::Edges), ["edges"]);
        assert_eq!(input_groups(&g, SelectionMode::Faces), ["faces"]);
        let point_names = input_groups(&g, SelectionMode::Points);
        assert_eq!(filtered_group_names(&point_names, "PTS"), ["points"]);
        assert!(filtered_group_names(&point_names, "faces").is_empty());
    }

    #[test]
    fn dropdown_filters_and_ranks_detected_names() {
        let names = vec!["scatter".into(), "FaceCenter".into(), "face".into(), "other".into()];
        assert_eq!(filtered_group_names(&names, "fce"), ["face", "FaceCenter"]);
        assert!(filtered_group_names(&names, "missing").is_empty());
        assert_eq!(filtered_group_names(&names, "").len(), names.len());
    }

    #[test]
    fn attribute_fields_offer_detected_attributes() {
        use minihou_core::geometry::Attr;
        use std::sync::Arc;
        assert!(is_input_attribute_field("piece_attrib", Some(&json!({"kind":"str"}))));
        assert!(is_input_attribute_field("distance_scale_attribute", None));
        assert!(!is_input_attribute_field("output_attrib", None));
        let mut g = minihou_core::Geometry::points_only(vec![0., 0., 0.]).unwrap();
        g.point.insert("weight".into(), Arc::new(Attr::f32(1, vec![1.])));
        g.point.insert("group_selected".into(), Arc::new(Attr::i32(1, vec![1])));
        assert!(input_attributes(&g).contains(&"weight".to_owned()));
        assert!(!input_attributes(&g).contains(&"group_selected".to_owned()));
    }

    #[test]
    fn group_field_reserves_room_for_dropdown_and_picker() {
        assert_eq!(group_text_width(180.), 100.);
        assert_eq!(group_text_width(60.), 8.);
    }
}
