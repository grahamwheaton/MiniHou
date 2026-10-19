//! Parameters panel: Main.dc.html `.par` and Params.dc.html.
use super::{theme::Theme, Actions, UiAction, View};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use super::number_step;
use minihou_app::{clamp_parameter, metadata_kind, param_kind, param_meta, slider_range, Edit, ParamKind};
use minihou_core::{nodes, params::ParamSpec};
use serde_json::{json, Value};
use minihou_app::value_link_ui::{self, CopiedParam, PasteMode};

const DISABLED: &str = "The Rust backend does not support it yet";
fn deformer_section(typ:&str,name:&str)->Option<&'static str> {
    if !super::deform_gizmo::is_deformer(typ) { return None; }
    Some(if matches!(name,"group"|"outside"|"max_points") {"Selection"}
        else if matches!(name,"origin"|"center"|"direction"|"up"|"length"|"radius"|"axis"|"frame"|"offset"|"stretch"|"symmetric"|"up_vector_angle") {"Capture"}
        else {"Deformation"})
}
#[cfg(test)]
mod deformer_group_tests {
    use super::deformer_section;
    #[test]
    fn capture_and_deformation_fields_are_grouped() {
        assert_eq!(deformer_section("bend","origin"),Some("Capture"));
        assert_eq!(deformer_section("linear_taper","taper_x"),Some("Deformation"));
        assert_eq!(deformer_section("point_deform","group"),Some("Selection"));
        assert_eq!(deformer_section("box","size"),None);
    }
}
pub const PICK_PURPLE: Color32 = Color32::from_rgb(184, 82, 245);
/// Output group target button and its viewport highlight: teal, distinct from purple (input) and orange (object selection).
pub const TARGET_TEAL: Color32 = Color32::from_rgb(38, 205, 200);
/// Orange used for object selection and the barrier group.
pub const SELECT_ORANGE: Color32 = Color32::from_rgb(0xff, 0xa0, 0x28);

fn is_string_meta(meta: Option<&Value>) -> bool {
    meta.is_some_and(|m| matches!(m["kind"].as_str(), Some("str" | "string")))
}
fn explicit_pick(meta: Option<&Value>) -> bool {
    meta.is_some_and(|m| m.get("pick_domain").is_some() || m.get("pick_domain_param").is_some())
}
/// One rule for output group fields: string fields that name a group the node writes (outgrp_*_name, output_group, ...).
pub fn is_output_group_field(name: &str, meta: Option<&Value>) -> bool {
    if !is_string_meta(meta) || explicit_pick(meta) { return false; }
    let named = (name.starts_with("outgrp_") && name.ends_with("_name"))
        || matches!(name, "output_group" | "output_group_name" | "output_inner_group_name" | "result_group_name" | "patch_group" | "patchgroup"
            | "edge_loop_group" | "edgeloopgroup" | "fill_group" | "ray_hit_group" | "inside_group");
    named || meta.is_some_and(|m| m["doc"].as_str().is_some_and(|d| d.contains("receives")))
}
/// One rule for input group fields: string fields that select elements by group, pattern, or an element list.
pub fn is_input_group_field(name: &str, meta: Option<&Value>) -> bool {
    if explicit_pick(meta) { return true; }
    if (meta.is_some() && !is_string_meta(meta)) || is_output_group_field(name, meta) { return false; }
    let by_name = matches!(name, "group" | "group2" | "groups" | "group_name" | "pattern") || name.ends_with("_group") || name.ends_with("_groups");
    let selection_list = matches!(name, "points" | "edges" | "faces" | "exclude_edges")
        && meta.is_some_and(|m| m["doc"].as_str().is_some_and(|d| d.to_lowercase().contains("selection")));
    by_name || selection_list
}
fn name_domain(name: &str, meta: Option<&Value>) -> super::SelectionMode {
    let doc = meta.and_then(|m| m["doc"].as_str()).unwrap_or("").to_lowercase();
    if name.contains("edge") || doc.contains("edge") { super::SelectionMode::Edges }
    else if name.contains("face") || doc.contains("primitive") { super::SelectionMode::Faces }
    else { super::SelectionMode::Points }
}

/// The registry describes the element domain; dynamic nodes read their domain parameter.
pub fn pick_domain(meta: Option<&Value>, params: &Value) -> Option<super::SelectionMode> {
    let meta = meta?;
    let domain = meta["pick_domain_param"].as_str()
        .and_then(|name| params[name].as_str().or_else(|| meta["pick_domain_default"].as_str()))
        .or_else(|| meta["pick_domain"].as_str())?;
    match domain {
        "points" | "point" => Some(super::SelectionMode::Points),
        "edges" | "edge" => Some(super::SelectionMode::Edges),
        "primitives" | "primitive" | "faces" | "prim" => Some(super::SelectionMode::Faces),
        _ => None,
    }
}

/// The element mode a `domain`/`group_type` enum edit switches to, so a viewport selection can be promoted with it.
pub fn domain_edit_mode(name: &str, value: &Value) -> Option<super::SelectionMode> {
    if !matches!(name, "domain" | "group_type") { return None; }
    pick_domain(Some(&json!({"pick_domain": value.as_str()?})), &Value::Null)
}
/// Fallback for group fields without registry metadata: follow a sibling domain or group_type parameter.
pub fn pick_domain_for(typ: &str, name: &str, meta: Option<&Value>, params: &Value) -> Option<super::SelectionMode> {
    if typ == "attribute_create" && name == "group" {
        if let Some(class) = params["attributes"].as_array().and_then(|a| a.first()).and_then(|a| a["class"].as_str()) {
            return pick_domain(Some(&json!({"pick_domain":class})), params);
        }
    }
    if let Some(mode) = pick_domain(meta, params) { return Some(mode); }
    if !is_input_group_field(name, meta) { return None; }
    if !matches!(name, "group" | "group_name" | "pattern") { return Some(name_domain(name, meta)); }
    for sibling in ["domain", "group_type"] {
        let Some(m) = param_meta(typ, sibling) else { continue };
        let value = params[sibling].as_str().or_else(|| m["default"].as_str());
        if let Some(mode) = value.and_then(|d| pick_domain(Some(&json!({"pick_domain": d})), params)) { return Some(mode); }
    }
    Some(name_domain(name, meta))
}

#[cfg(test)]
mod pick_tests {
    use super::*;
    use super::super::SelectionMode;
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
        assert_eq!(pick_domain_for("group_create", "pattern", param_meta("group_create", "pattern"), &json!({"domain":"edges"})), Some(SelectionMode::Edges));
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
    fn output_rule_adds_target_to_output_group_names() {
        for (n, f) in [("flood_fill","output_group"),("polyfill","patch_group"),("polyfill","fill_group"),("ray","ray_hit_group"),("grow","result_group_name"),("voronoi_fracture","inside_group")] {
            assert!(is_output_group_field(f, param_meta(n, f)), "{n}.{f} needs the target");
        }
        assert!(is_output_group_field("outgrp_shared_name", Some(&json!({"kind":"str"}))));
        assert!(!is_output_group_field("group", param_meta("blast", "group")));
        assert!(!is_output_group_field("outgrp_cap", Some(&json!({"kind":"bool"}))));
    }
    #[test]
    fn target_colour_differs_from_purple_and_orange() {
        assert_ne!(TARGET_TEAL, PICK_PURPLE);
        assert_ne!(TARGET_TEAL, SELECT_ORANGE);
        assert!(TARGET_TEAL.g() > TARGET_TEAL.r() && TARGET_TEAL.b() > TARGET_TEAL.r());
    }
    #[test]
    fn arrow_ink_is_purple() {
        assert_eq!(PICK_PURPLE, Color32::from_rgb(184, 82, 245));
    }
}

fn pick_field(ui: &mut egui::Ui, value: &mut Value, width: f32, active: bool, enabled: bool) -> (bool, bool) {
    let mut text = value.as_str().unwrap_or_default().to_owned();
    let edit = ui.add_sized([(width - 28.).max(8.), 22.], egui::TextEdit::singleline(&mut text));
    if edit.changed() { *value = json!(text); }
    let arrow = ui.add_enabled(enabled, egui::Button::new(egui::RichText::new("↖").color(PICK_PURPLE))
        .fill(if active { PICK_PURPLE.gamma_multiply(0.27) } else { ui.visuals().widgets.inactive.bg_fill })
        .stroke(Stroke::new(1.0_f32, PICK_PURPLE))).on_hover_text("Pick elements in viewport (Enter to finish, Esc to cancel)");
    (arrow.clicked(), arrow.hovered())
}
fn target_field(ui: &mut egui::Ui, value: &mut Value, width: f32, enabled: bool) -> (bool, bool) {
    let mut text = value.as_str().unwrap_or_default().to_owned();
    let edit = ui.add_sized([(width - 28.).max(8.), 22.], egui::TextEdit::singleline(&mut text));
    if edit.changed() { *value = json!(text); }
    let button = ui.add_enabled(enabled, egui::Button::new(egui::RichText::new("◎").color(TARGET_TEAL))
        .fill(ui.visuals().widgets.inactive.bg_fill).stroke(Stroke::new(1.0_f32, TARGET_TEAL)))
        .on_hover_text("Show this output group in the viewport; click to select it");
    (button.clicked(), button.hovered())
}
fn clipboard_id() -> egui::Id { egui::Id::new("value_link_parameter_clipboard") }

fn open_expression(ctx: &egui::Context, doc: &Value, node: &str, path: &str) {
    let expr = doc["nodes"][node]["channels"][path].as_str().unwrap_or("").to_owned();
    ctx.data_mut(|d| { d.insert_temp(egui::Id::new("link_expression_open"), (node.to_owned(), path.to_owned())); d.insert_temp(egui::Id::new("link_expression_text"), expr); });
}

fn channel_reference_edit(node: &str, path: &str, source: &str, source_path: &str) -> Edit {
    let reference = value_link_ui::relative_path(node, source, source_path);
    Edit::Channel { node: node.into(), path: path.into(), expression: Some(format!("ch(\"{reference}\")")) }
}

fn vector_channel_menu(ui: &mut egui::Ui, v: &View<'_>, out: &mut Actions, node: &str, name: &str, axis: usize) {
    let suffix = ["x", "y", "z", "w"][axis];
    let path = format!("{name}.{suffix}");
    ui.menu_button("Link to Parameter...", |ui| {
        let search_id = egui::Id::new("link_picker_search");
        let mut search = ui.ctx().data(|d| d.get_temp::<String>(search_id)).unwrap_or_default();
        ui.add(egui::TextEdit::singleline(&mut search).hint_text("Search nodes and parameters"));
        ui.ctx().data_mut(|d| d.insert_temp(search_id, search.clone()));
        egui::ScrollArea::vertical().max_height(260.).show(ui, |ui| {
            if let Some(all) = v.doc["nodes"].as_object() {
                for (source, def) in all {
                    let Some(spec) = def["type"].as_str().and_then(nodes::get) else { continue; };
                    for param in spec.params {
                        let value = def["params"].get(param.name).cloned().unwrap_or_else(|| (param.default)());
                        let source_path = if value.as_array().is_some_and(|a| axis < a.len() && a[axis].is_number()) {
                            format!("{}.{suffix}", param.name)
                        } else if value.is_number() || value.is_boolean() { param.name.to_owned() } else { continue; };
                        if source == node && source_path == path { continue; }
                        let label = format!("{source} / {source_path}");
                        if !label.to_lowercase().contains(&search.to_lowercase()) { continue; }
                        if ui.button(label).clicked() {
                            out.push(UiAction::Edit { edit: channel_reference_edit(node, &path, source, &source_path), continuous: false });
                            ui.close();
                        }
                    }
                }
            }
        });
    });
    if let Some((source, param)) = value_link_ui::go_to_source(v.doc, node, Some(&path)) {
        if ui.button(format!("Go to Source: {source}.{param}")).clicked() { out.push(UiAction::NavigateLink { node: source, param }); ui.close(); }
    }
    if v.doc["nodes"][node]["channels"].get(&path).is_some() && ui.button("Delete Channel").clicked() {
        out.push(UiAction::Edit { edit: Edit::Channel { node: node.into(), path: path.clone(), expression: None }, continuous: false });
        ui.close();
    }
    if ui.button("Edit Expression...").clicked() {
        open_expression(ui.ctx(), v.doc, node, &path);
        ui.close();
    }
}

fn parameter_menu(ui: &mut egui::Ui, v: &View<'_>, out: &mut Actions, node: &str, name: &str, default: &Value) {
    let copied = ui.ctx().data(|d| d.get_temp::<CopiedParam>(clipboard_id()));
    if ui.button("Copy Parameter").clicked() {
        if let Some(copy) = value_link_ui::copied(v.doc, node, name) { ui.ctx().data_mut(|d| d.insert_temp(clipboard_id(), copy)); }
        ui.close();
    }
    for (label, mode) in [("Paste Relative Reference", PasteMode::Reference), ("Paste Value", PasteMode::Value), ("Paste Expression", PasteMode::Expression)] {
        let valid = copied.as_ref().is_some_and(|c| value_link_ui::compatible(&c.value, default));
        if ui.add_enabled(valid, egui::Button::new(label)).clicked() {
            if let Some(c) = &copied {
                match value_link_ui::paste(v.doc, node, name, c, mode) {
                    Ok(edit) => out.push(UiAction::Edit { edit, continuous: false }),
                    Err(message) => ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("link_message"), message)),
                }
            }
            ui.close();
        }
    }
    ui.menu_button(if v.doc["nodes"][node]["channels"].as_object().is_some_and(|c| c.contains_key(name)) { "Link to Parameter... / Fix Reference" } else { "Link to Parameter..." }, |ui| {
        let search_id = egui::Id::new("link_picker_search");
        let mut search = ui.ctx().data(|d| d.get_temp::<String>(search_id)).unwrap_or_default();
        ui.add(egui::TextEdit::singleline(&mut search).hint_text("Search nodes and parameters"));
        ui.ctx().data_mut(|d| d.insert_temp(search_id, search.clone()));
        egui::ScrollArea::vertical().max_height(260.).show(ui, |ui| {
            if let Some(all) = v.doc["nodes"].as_object() {
                for (source, def) in all {
                    let Some(typ) = def["type"].as_str() else { continue; };
                    let Some(spec) = nodes::get(typ) else { continue; };
                    for param in spec.params {
                        if source == node && param.name == name { continue; }
                        let label = format!("{source} / {}",param.name);
                        if !label.to_lowercase().contains(&search.to_lowercase()) { continue; }
                        let source_value = def["params"].get(param.name).cloned().unwrap_or_else(|| (param.default)());
                        let valid = value_link_ui::compatible(&source_value, default);
                        if ui.add_enabled(valid,egui::Button::new(&label)).on_disabled_hover_text("Incompatible parameter type").clicked() {
                            if let Some(copy) = value_link_ui::copied(v.doc,source,param.name) {
                                match value_link_ui::paste(v.doc,node,name,&copy,PasteMode::Reference) {
                                    Ok(edit) => out.push(UiAction::Edit { edit, continuous:false }),
                                    Err(message) => ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("link_message"),message)),
                                }
                            }
                            ui.close();
                        }
                    }
                }
            }
        });
    });
    let linked = v.doc["nodes"][node]["channels"].as_object().is_some_and(|c| c.keys().any(|k| k == name || k.starts_with(&format!("{name}."))));
    if let Some((source, param)) = value_link_ui::go_to_source(v.doc,node,Some(name)) {
        if ui.button(format!("Go to Source: {source}.{param}")).clicked() { out.push(UiAction::NavigateLink {node:source,param}); ui.close(); }
    }
    for link in value_link_ui::links(v.doc).into_iter().filter(|l| l.source == node && (l.source_param == name || l.source_param.starts_with(&format!("{name}.")))) {
        if ui.button(format!("Go to Target: {}.{}",link.target,link.target_param)).clicked() {
            out.push(UiAction::NavigateLink {node:link.target,param:link.target_param}); ui.close();
        }
    }
    if ui.add_enabled(linked, egui::Button::new("Delete Channel")).clicked() {
        if let Some(edit) = value_link_ui::clear(v.doc,node,name,false) { out.push(UiAction::Edit { edit, continuous: false }); }
        ui.close();
    }
    if ui.button("Revert to Default").clicked() {
        if linked { if let Some(edit) = value_link_ui::clear(v.doc,node,name,true) { out.push(UiAction::Edit { edit, continuous: false }); } }
        else { out.push(UiAction::Edit { edit: Edit::Param { node: node.into(), name: name.into(), value: default.clone() }, continuous: false }); }
        ui.close();
    }
    let locked = v.doc["nodes"][node]["locked_params"].as_array().is_some_and(|a| a.iter().any(|x| x == name));
    if ui.button(if locked { "Unlock Parameter" } else { "Lock Parameter" }).clicked() {
        out.push(UiAction::Edit { edit: Edit::LockParam { node:node.into(), path:name.into(), locked:!locked }, continuous:false }); ui.close();
    }
    let component_paths: Vec<_> = v.doc["nodes"][node]["channels"].as_object().into_iter().flatten()
        .filter(|(path, _)| path.starts_with(&format!("{name}."))).map(|(path, _)| path.clone()).collect();
    if v.doc["nodes"][node]["channels"].get(name).is_none() && !component_paths.is_empty() {
        ui.menu_button("Edit Expression...", |ui| {
            for path in &component_paths {
                if ui.button(path).clicked() { open_expression(ui.ctx(), v.doc, node, path); ui.close(); }
            }
        });
    } else if ui.button("Edit Expression...").clicked() {
        open_expression(ui.ctx(), v.doc, node, name);
        ui.close();
    }
    if let Some(message) = ui.ctx().data(|d| d.get_temp::<String>(egui::Id::new("link_message"))) { ui.colored_label(v.theme.err, message); }
}

fn expression_dialog(ctx: &egui::Context, v: &View<'_>, out: &mut Actions) {
    let id = egui::Id::new("link_expression_open");
    let Some((node, name)) = ctx.data(|d| d.get_temp::<(String,String)>(id)) else { return; };
    let mut open = true;
    egui::Window::new(format!("Expression: {node}.{name}")).open(&mut open).collapsible(false).show(ctx, |ui| {
        let text_id = egui::Id::new("link_expression_text");
        let mut expression = ctx.data(|d| d.get_temp::<String>(text_id)).unwrap_or_default();
        ui.text_edit_singleline(&mut expression);
        ctx.data_mut(|d| d.insert_temp(text_id, expression.clone()));
        if let Ok(graph) = minihou_core::Graph::from_value(v.doc) {
            for diag in minihou_core::channels::resolve_graph(&graph).diagnostics.iter().filter(|d| d.node == node && (d.param == name || d.param.starts_with(&format!("{name}.")))) {
                ui.colored_label(v.theme.err, &diag.message);
            }
        }
        if ui.button("Apply").clicked() {
            out.push(UiAction::Edit { edit: Edit::Channel { node:node.clone(), path:name.clone(), expression:(!expression.trim().is_empty()).then_some(expression) }, continuous:false });
            ctx.data_mut(|d| d.remove::<(String,String)>(id));
        }
    });
    if !open { ctx.data_mut(|d| d.remove::<(String,String)>(id)); }
}

fn title(name: &str) -> String {
    name.split('_').map(|part| {
        let mut chars = part.chars();
        chars.next().map_or_else(String::new, |c| c.to_uppercase().to_string() + chars.as_str())
    }).collect::<Vec<_>>().join(" ")
}

fn value_of(v: &View<'_>, node: &str, name: &str, default: &Value) -> Value {
    v.doc["nodes"][node]["params"].get(name).unwrap_or(default).clone()
}

fn modified_params(v: &View<'_>, node: &str, typ: &str) -> Vec<(String, Value)> {
    nodes::get(typ).map_or_else(Vec::new, |spec| spec.params.iter().filter_map(|param| {
        let default = (param.default)();
        (value_of(v, node, param.name, &default) != default).then(|| (param.name.to_owned(), default))
    }).collect())
}

fn invalid_reason(meta: Option<&Value>, value: &Value) -> Option<String> {
    let meta = meta?;
    if let Some(n) = value.as_f64() {
        if let Some(min) = meta["min"].as_f64() { if n < min { return Some(format!("Value must be at least {min}.")); } }
        if let Some(max) = meta["max"].as_f64() { if n > max { return Some(format!("Value must be at most {max}.")); } }
    }
    if meta["kind"] == "enum" && !meta["options"].as_array().is_some_and(|options| options.contains(value)) {
        return Some("Value is not one of the available options.".into());
    }
    None
}

fn node_error<'a>(v: &'a View<'_>, id: &str) -> Option<&'a str> {
    let error = v.cooking.error.as_deref()?;
    let name = error.strip_prefix("node '")?.split_once('\'')?.0;
    (name == id).then_some(error)
}

fn paint_text(ui: &egui::Ui, at: Pos2, text: &str, size: f32, mono: bool, color: Color32, align: Align2) {
    ui.painter().text(at, align, text, if mono { FontId::monospace(size) } else { FontId::proportional(size) }, color);
}

#[allow(clippy::too_many_arguments)]
fn button(ui: &mut egui::Ui, rect: Rect, id: egui::Id, label: &str, on: bool, color: Color32, theme: &Theme, enabled: bool) -> egui::Response {
    let response = ui.interact(rect, id, if enabled { Sense::click() } else { Sense::hover() });
    ui.painter().rect(rect, 4., if on { theme.accent_bg } else { theme.input },
        Stroke::new(1.0_f32, if on { color } else if response.hovered() { theme.stroke2 } else { theme.stroke }), StrokeKind::Inside);
    paint_text(ui, rect.center(), label, 11., false,
        if !enabled { theme.faint } else if on { color } else if label == "Reset all" { theme.text } else { theme.dim }, Align2::CENTER_CENTER);
    response
}

fn flag_icon(ui: &egui::Ui, at: Pos2, label: &str, color: Color32) {
    let p = ui.painter();
    let s = Stroke::new(1.3_f32, color);
    match label {
        "Mute" => {
            p.line_segment([at + Vec2::new(-4., 0.), at + Vec2::new(4., 0.)], s);
            p.line_segment([at + Vec2::new(1., -3.), at + Vec2::new(4., 0.)], s);
            p.line_segment([at + Vec2::new(1., 3.), at + Vec2::new(4., 0.)], s);
        }
        "Locked" => {
            p.rect_stroke(Rect::from_center_size(at + Vec2::new(0., 2.), Vec2::new(7., 5.)), 1., s, StrokeKind::Inside);
            p.circle_stroke(at + Vec2::new(0., -2.), 2.5, s);
        }
        "Template" => {
            for x in [-4., 1.] {
                p.line_segment([at + Vec2::new(x, -4.), at + Vec2::new(x + 2., -4.)], s);
                p.line_segment([at + Vec2::new(x, 4.), at + Vec2::new(x + 2., 4.)], s);
            }
            for y in [-3., 2.] {
                p.line_segment([at + Vec2::new(-4., y), at + Vec2::new(-4., y + 2.)], s);
                p.line_segment([at + Vec2::new(4., y), at + Vec2::new(4., y + 2.)], s);
            }
        }
        _ => {
            p.line_segment([at + Vec2::new(-5., 0.), at + Vec2::new(0., -3.)], s);
            p.line_segment([at + Vec2::new(0., -3.), at + Vec2::new(5., 0.)], s);
            p.line_segment([at + Vec2::new(-5., 0.), at + Vec2::new(0., 3.)], s);
            p.line_segment([at + Vec2::new(0., 3.), at + Vec2::new(5., 0.)], s);
            p.circle_filled(at, 1.3, color);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn header(ui: &mut egui::Ui, v: &View<'_>, out: &mut Actions, id: &str, typ: &str, bypass: bool, locked: bool, template: bool) {
    let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 112.), Sense::hover());
    let p = ui.painter();
    p.line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(1.0_f32, v.theme.stroke));
    let x = r.left() + 12.;
    let y = r.top() + 10.;
    ui.painter().rect_filled(Rect::from_min_size(Pos2::new(x, y + 8.), Vec2::splat(10.)), 2., v.theme.category_color(typ));
    let field = Rect::from_min_size(Pos2::new(x + 18., y), Vec2::new((r.width() - 94.).max(48.), 26.));
    ui.painter().rect(field, 4., v.theme.input, Stroke::new(1.0_f32, v.theme.stroke), StrokeKind::Inside);
    paint_text(ui, Pos2::new(field.left() + 6., field.center().y), id, 13., true, v.theme.text, Align2::LEFT_CENTER);
    let rust = v.runs_on_rust(typ);
    let tag_color = if rust { v.theme.ok } else { v.theme.warn };
    let tag = Rect::from_min_size(Pos2::new(field.right() + 8., y + 5.), Vec2::new(25., 16.));
    ui.painter().rect(tag, 3., if rust { v.theme.ok_bg } else { v.theme.warn_bg }, Stroke::new(1.0_f32, tag_color), StrokeKind::Inside);
    paint_text(ui, tag.center(), if rust { "Rs" } else { "Py" }, 10., true, tag_color, Align2::CENTER_CENTER);

    let mid = y + 46.;
    let name = title(typ);
    paint_text(ui, Pos2::new(x, mid), &name, 12., false, v.theme.dim, Align2::LEFT_CENTER);
    let nx = x + ui.painter().layout_no_wrap(name, FontId::proportional(12.), v.theme.dim).size().x;
    paint_text(ui, Pos2::new(nx + 8., mid), "·", 12., false, v.theme.dim, Align2::LEFT_CENTER);
    let time = if node_error(v, id).is_some() { "failed".to_owned() }
        else if let Some((_, ms)) = v.cooking.stats.node_ms.iter().find(|(node, _)| node == id) { format!("{ms:.1} ms") }
        else if v.cooking.stats.cached.iter().any(|node| node == id) { "cached".to_owned() }
        else { "—".to_owned() };
    paint_text(ui, Pos2::new(nx + 18., mid), &time, 12., true,
        if time == "failed" { v.theme.err } else { v.theme.dim }, Align2::LEFT_CENTER);
    let reset = Rect::from_min_size(Pos2::new(r.right() - 79., y + 34.), Vec2::new(67., 24.));
    if button(ui, reset, ui.id().with((id, "reset_all")), "Reset all", false, v.theme.text, v.theme, !locked)
        .on_disabled_hover_text("Locked").clicked() {
        for (name, value) in modified_params(v, id, typ) {
            out.push(UiAction::Edit { edit: Edit::Param { node: id.to_owned(), name, value }, continuous: false });
        }
    }
    let fy = y + 66.;
    let mut fx = x;
    for (label, on, color, width) in [
        ("Mute", bypass, v.theme.warn, 56.),
        ("Locked", locked, v.theme.dim, 65.),
        ("Template", template, v.theme.tpl, 78.),
        ("Visible", minihou_app::display_nodes(v.doc).iter().any(|node| node == id), v.theme.accent, 68.),
    ] {
        if label == "Template" {
            ui.painter().line_segment([Pos2::new(fx + 2., fy + 4.), Pos2::new(fx + 2., fy + 20.)], Stroke::new(1.0_f32, v.theme.stroke));
            fx += 10.;
        }
        let rect = Rect::from_min_size(Pos2::new(fx, fy), Vec2::new(width, 24.));
        let response = button(ui, rect, ui.id().with((id, label)), "", on, color, v.theme, true);
        let ink = if on { color } else { v.theme.dim };
        flag_icon(ui, Pos2::new(fx + 12., rect.center().y), label, ink);
        paint_text(ui, Pos2::new(fx + 21., rect.center().y), label, 11., false, ink, Align2::LEFT_CENTER);
        if response.clicked() {
            match label {
                "Mute" => out.push(UiAction::Edit { edit: Edit::Bypass { node: id.to_owned(), flag: !bypass }, continuous: false }),
                "Locked" => out.push(UiAction::ToggleLocked(id.to_owned())),
                "Template" => out.push(UiAction::ToggleTemplate(id.to_owned())),
                _ => out.push(if ui.input(|i| i.modifiers.shift) { UiAction::ToggleDisplay(id.to_owned()) } else { UiAction::SetDisplay(id.to_owned()) }),
            }
        }
        fx += width + 6.;
    }
}

fn icon(ui: &mut egui::Ui, rect: Rect, id: egui::Id, reset: bool, on: bool, theme: &Theme) -> egui::Response {
    let response = ui.interact(rect, id, Sense::click());
    if on { ui.painter().rect_filled(rect, 4., theme.accent_bg); }
    let color = if on { theme.accent } else { theme.dim.gamma_multiply(0.5) };
    let p = ui.painter();
    let c = rect.center();
    let stroke = Stroke::new(1.4_f32, color);
    if reset {
        p.circle_stroke(c, 4., stroke);
        p.rect_filled(Rect::from_min_size(c + Vec2::new(-5., -5.), Vec2::splat(4.)), 0., theme.panel);
        p.line_segment([c + Vec2::new(-5., -4.), c + Vec2::new(-5., -1.)], stroke);
        p.line_segment([c + Vec2::new(-5., -1.), c + Vec2::new(-2., -1.)], stroke);
    } else {
        for (a, b) in [((-4., -2.), (-4., 4.)), ((-4., 4.), (3., 4.)), ((-1., 1.), (5., -5.)),
            ((1., -5.), (5., -5.)), ((5., -5.), (5., -1.))] {
            p.line_segment([c + Vec2::new(a.0, a.1), c + Vec2::new(b.0, b.1)], stroke);
        }
    }
    response
}

/// Houdini parameter pane row layout helper.
/// Every row is [label | fields]:
/// - Fixed label column width (about 28-35% of the panel, minimum 110px, clamped).
/// - Labels right-aligned and vertically centred against the row.
/// - All field widgets left-aligned starting at the same x forming a clean column.
pub struct ParamRowLayout {
    pub row_rect: Rect,
    pub label_rect: Rect,
    pub control_rect: Rect,
    pub expose_rect: Rect,
    pub reset_rect: Rect,
}

impl ParamRowLayout {
    pub fn compute(row_rect: Rect) -> Self {
        let total_w = row_rect.width();
        let left_pad = 16.0_f32;
        let right_buttons_w = 54.0_f32;
        let gap = 8.0_f32;
        let min_field_w = 60.0_f32;

        let avail_content = (total_w - left_pad - right_buttons_w - gap).max(20.0);
        let max_label_w = (avail_content - min_field_w).max(30.0);
        // Fixed label column width: about 28-35% of the panel, minimum 110px, clamped
        let label_w = ((total_w * 0.30).clamp(110.0, 240.0)).min(max_label_w);

        let label_rect = Rect::from_min_size(
            Pos2::new(row_rect.left() + left_pad, row_rect.top()),
            Vec2::new(label_w, row_rect.height()),
        );

        let expose_rect = Rect::from_min_size(
            Pos2::new(row_rect.right() - 26., row_rect.top() + 4.),
            Vec2::splat(20.),
        );
        let reset_rect = Rect::from_min_size(
            Pos2::new(expose_rect.left() - 28., row_rect.top() + 4.),
            Vec2::splat(20.),
        );

        let fields_x = label_rect.right() + gap;
        let fields_right = reset_rect.left() - 6.0;
        let control_rect = Rect::from_min_max(
            Pos2::new(fields_x, row_rect.top() + 3.),
            Pos2::new(fields_right.max(fields_x + 10.0), row_rect.bottom() - 3.),
        );

        Self {
            row_rect,
            label_rect,
            control_rect,
            expose_rect,
            reset_rect,
        }
    }
}

fn truncate_with_ellipsis(ui: &egui::Ui, text: &str, font: &FontId, max_w: f32) -> String {
    if max_w <= 6.0 { return String::new(); }
    let g = ui.painter().layout_no_wrap(text.to_owned(), font.clone(), Color32::WHITE);
    if g.size().x <= max_w {
        return text.to_owned();
    }
    let ellipsis = "…";
    let chars: Vec<char> = text.chars().collect();
    let mut low = 0;
    let mut high = chars.len();
    let mut best = ellipsis.to_owned();
    while low <= high {
        let mid = (low + high) / 2;
        let candidate: String = chars[..mid].iter().collect::<String>() + ellipsis;
        let g = ui.painter().layout_no_wrap(candidate.clone(), font.clone(), Color32::WHITE);
        if g.size().x <= max_w {
            best = candidate;
            low = mid + 1;
        } else if mid == 0 {
            break;
        } else {
            high = mid - 1;
        }
    }
    best
}

/// Parse a color value into RGBA `[f32; 4]`, number of components (3 or 4), and whether it was a hex string.
pub fn parse_color_value(value: &Value) -> Option<([f32; 4], usize, bool)> {
    if let Some(arr) = value.as_array() {
        if arr.len() == 3 && arr.iter().all(Value::is_number) {
            let r = arr[0].as_f64()? as f32;
            let g = arr[1].as_f64()? as f32;
            let b = arr[2].as_f64()? as f32;
            return Some(([r, g, b, 1.0], 3, false));
        } else if arr.len() == 4 && arr.iter().all(Value::is_number) {
            let r = arr[0].as_f64()? as f32;
            let g = arr[1].as_f64()? as f32;
            let b = arr[2].as_f64()? as f32;
            let a = arr[3].as_f64()? as f32;
            return Some(([r, g, b, a], 4, false));
        }
    } else if let Some(s) = value.as_str() {
        let s = s.trim();
        if let Some(hex_part) = s.strip_prefix('#') {
            if hex_part.len() == 6 {
                if let (Ok(r), Ok(g), Ok(b)) = (
                    u8::from_str_radix(&hex_part[0..2], 16),
                    u8::from_str_radix(&hex_part[2..4], 16),
                    u8::from_str_radix(&hex_part[4..6], 16),
                ) {
                    return Some(([r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0], 3, true));
                }
            } else if hex_part.len() == 8 {
                if let (Ok(r), Ok(g), Ok(b), Ok(a)) = (
                    u8::from_str_radix(&hex_part[0..2], 16),
                    u8::from_str_radix(&hex_part[2..4], 16),
                    u8::from_str_radix(&hex_part[4..6], 16),
                    u8::from_str_radix(&hex_part[6..8], 16),
                ) {
                    return Some(([r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a as f32 / 255.0], 4, true));
                }
            } else if hex_part.len() == 3 {
                if let (Ok(r), Ok(g), Ok(b)) = (
                    u8::from_str_radix(&hex_part[0..1], 16),
                    u8::from_str_radix(&hex_part[1..2], 16),
                    u8::from_str_radix(&hex_part[2..3], 16),
                ) {
                    return Some(([
                        (r * 17) as f32 / 255.0,
                        (g * 17) as f32 / 255.0,
                        (b * 17) as f32 / 255.0,
                        1.0,
                    ], 3, true));
                }
            }
        }
    }
    None
}

/// Serialize RGBA back into the original format (array of 3/4 floats or hex string).
pub fn serialize_color_value(rgba: [f32; 4], comps: usize, is_hex: bool) -> Value {
    if is_hex {
        let r = (rgba[0] * 255.0).clamp(0.0, 255.0).round() as u8;
        let g = (rgba[1] * 255.0).clamp(0.0, 255.0).round() as u8;
        let b = (rgba[2] * 255.0).clamp(0.0, 255.0).round() as u8;
        if comps == 4 {
            let a = (rgba[3] * 255.0).clamp(0.0, 255.0).round() as u8;
            json!(format!("#{r:02x}{g:02x}{b:02x}{a:02x}"))
        } else {
            json!(format!("#{r:02x}{g:02x}{b:02x}"))
        }
    } else if comps == 4 {
        json!([rgba[0], rgba[1], rgba[2], rgba[3]])
    } else {
        json!([rgba[0], rgba[1], rgba[2]])
    }
}

/// Check if a parameter represents a colour.
pub fn is_color_param(param_name: &str, meta: Option<&Value>, value: &Value) -> bool {
    if meta.and_then(|m| m["kind"].as_str()) == Some("color") {
        return true;
    }
    let name_is_color = param_name == "color"
        || param_name == "color2"
        || param_name == "colour"
        || param_name.ends_with("_color")
        || param_name.ends_with("_colour")
        || param_name == "tint"
        || param_name == "diffuse"
        || param_name == "specular";
    if name_is_color && parse_color_value(value).is_some() {
        return true;
    }
    if param_name.is_empty() && value.as_str().is_some_and(|s| s.starts_with('#')) && parse_color_value(value).is_some() {
        return true;
    }
    false
}

/// Check if a parameter represents a file path.
pub fn is_file_param(param_name: &str, meta: Option<&Value>) -> bool {
    if let Some(m) = meta {
        if m["kind"].as_str() == Some("file") {
            return true;
        }
        if m["ui"].as_str().is_some_and(|ui| ui.starts_with("file:")) {
            return true;
        }
    }
    param_name == "file"
        || param_name == "path"
        || param_name == "rule_file"
        || param_name == "image_file"
        || param_name.ends_with("_file")
        || param_name.ends_with("_path")
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RampInterpolation {
    Linear,
    Constant,
    Smooth,
}

impl RampInterpolation {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Linear => "linear",
            Self::Constant => "constant",
            Self::Smooth => "smooth",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "constant" | "step" | "hold" => Self::Constant,
            "smooth" | "catmull-rom" | "catmullrom" | "monotone_cubic" | "ease" | "bezier" => Self::Smooth,
            _ => Self::Linear,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum RampValue {
    Float(f32),
    Color([f32; 3]),
    ColorAlpha([f32; 4]),
}

impl RampValue {
    pub fn as_float(&self) -> f32 {
        match self {
            Self::Float(f) => *f,
            Self::Color(c) => (c[0] + c[1] + c[2]) / 3.0,
            Self::ColorAlpha(c) => (c[0] + c[1] + c[2]) / 3.0,
        }
    }

    pub fn as_rgba(&self) -> [f32; 4] {
        match self {
            Self::Color(c) => [c[0], c[1], c[2], 1.0],
            Self::ColorAlpha(c) => *c,
            Self::Float(f) => [*f, *f, *f, 1.0],
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RampStop {
    pub pos: f32,
    pub value: RampValue,
    pub interp: Option<RampInterpolation>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RampFormat {
    StopsObject,       // {"stops": [...], "interpolation": "..."}
    KeysValuesObject,  // {"keys": [...], "values": [...], "basis": "..."}
    ArrayOfPairs,      // [[pos, val], ...]
    ArrayOfStops,      // [{"pos": ..., "value"/"color": ...}, ...]
}

#[derive(Clone, Debug, PartialEq)]
pub struct RampData {
    pub is_color: bool,
    pub stops: Vec<RampStop>,
    pub interpolation: RampInterpolation,
    pub format: RampFormat,
}

impl RampData {
    pub fn is_ramp_value(value: &Value) -> bool {
        if let Some(obj) = value.as_object() {
            if obj.contains_key("stops") || (obj.contains_key("keys") && obj.contains_key("values")) {
                return true;
            }
        } else if let Some(arr) = value.as_array() {
            if !arr.is_empty() {
                if let Some(first) = arr.first() {
                    if first.as_array().is_some_and(|pair| pair.len() == 2 && pair[0].is_number()) {
                        return true;
                    }
                    if first.as_object().is_some_and(|item| item.contains_key("pos") || item.contains_key("position")) {
                        return true;
                    }
                }
            }
        }
        false
    }

    pub fn from_value(value: &Value, is_color_hint: bool) -> Self {
        let mut stops = Vec::new();
        let mut interpolation = RampInterpolation::Linear;
        let mut format = RampFormat::StopsObject;
        let mut is_color = is_color_hint;

        if let Some(obj) = value.as_object() {
            if let Some(interp_str) = obj.get("interpolation").or_else(|| obj.get("basis")).and_then(Value::as_str) {
                interpolation = RampInterpolation::from_str(interp_str);
            }
            if let Some(stops_arr) = obj.get("stops").and_then(Value::as_array) {
                format = RampFormat::StopsObject;
                for item in stops_arr {
                    if let Some(stop) = Self::parse_stop_object(item, &mut is_color) {
                        stops.push(stop);
                    }
                }
            } else if let (Some(keys), Some(vals)) = (obj.get("keys").and_then(Value::as_array), obj.get("values").and_then(Value::as_array)) {
                format = RampFormat::KeysValuesObject;
                for (k, v) in keys.iter().zip(vals) {
                    if let Some(pos) = k.as_f64().map(|f| f as f32) {
                        let r_val = Self::parse_ramp_value(v, &mut is_color);
                        stops.push(RampStop { pos: pos.clamp(0.0, 1.0), value: r_val, interp: None });
                    }
                }
            }
        } else if let Some(arr) = value.as_array() {
            if let Some(first) = arr.first() {
                if first.as_array().is_some() {
                    format = RampFormat::ArrayOfPairs;
                    for item in arr {
                        if let Some(pair) = item.as_array() {
                            if pair.len() >= 2 {
                                if let Some(pos) = pair[0].as_f64().map(|f| f as f32) {
                                    let r_val = Self::parse_ramp_value(&pair[1], &mut is_color);
                                    stops.push(RampStop { pos: pos.clamp(0.0, 1.0), value: r_val, interp: None });
                                }
                            }
                        }
                    }
                } else if first.as_object().is_some() {
                    format = RampFormat::ArrayOfStops;
                    for item in arr {
                        if let Some(stop) = Self::parse_stop_object(item, &mut is_color) {
                            stops.push(stop);
                        }
                    }
                }
            }
        }

        let mut data = Self {
            is_color,
            stops,
            interpolation,
            format,
        };
        data.ensure_valid_defaults();
        data.sort_stops();
        data
    }

    fn parse_ramp_value(v: &Value, is_color: &mut bool) -> RampValue {
        if let Some(arr) = v.as_array() {
            if arr.len() == 4 && arr.iter().all(Value::is_number) {
                *is_color = true;
                return RampValue::ColorAlpha([
                    arr[0].as_f64().unwrap_or(0.) as f32,
                    arr[1].as_f64().unwrap_or(0.) as f32,
                    arr[2].as_f64().unwrap_or(0.) as f32,
                    arr[3].as_f64().unwrap_or(1.) as f32,
                ]);
            } else if arr.len() == 3 && arr.iter().all(Value::is_number) {
                *is_color = true;
                return RampValue::Color([
                    arr[0].as_f64().unwrap_or(0.) as f32,
                    arr[1].as_f64().unwrap_or(0.) as f32,
                    arr[2].as_f64().unwrap_or(0.) as f32,
                ]);
            }
        } else if let Some(n) = v.as_f64() {
            return RampValue::Float(n as f32);
        } else if let Some(s) = v.as_str() {
            if let Some((rgba, _, _)) = parse_color_value(&Value::String(s.to_owned())) {
                *is_color = true;
                return RampValue::Color([rgba[0], rgba[1], rgba[2]]);
            }
        }
        if *is_color {
            RampValue::Color([0.0, 0.0, 0.0])
        } else {
            RampValue::Float(0.0)
        }
    }

    fn parse_stop_object(item: &Value, is_color: &mut bool) -> Option<RampStop> {
        let obj = item.as_object()?;
        let pos = obj.get("pos")
            .or_else(|| obj.get("position"))
            .or_else(|| obj.get("t"))?
            .as_f64()? as f32;
        let interp = obj.get("interpolation")
            .or_else(|| obj.get("interp"))
            .and_then(Value::as_str)
            .map(RampInterpolation::from_str);
        let value = if let Some(c) = obj.get("color").or_else(|| obj.get("colour")) {
            Self::parse_ramp_value(c, is_color)
        } else if let Some(v) = obj.get("value").or_else(|| obj.get("val")) {
            Self::parse_ramp_value(v, is_color)
        } else {
            Self::parse_ramp_value(&Value::Null, is_color)
        };
        Some(RampStop { pos: pos.clamp(0.0, 1.0), value, interp })
    }

    fn ensure_valid_defaults(&mut self) {
        if self.stops.is_empty() {
            if self.is_color {
                self.stops = vec![
                    RampStop { pos: 0.0, value: RampValue::Color([0.0, 0.0, 0.0]), interp: None },
                    RampStop { pos: 1.0, value: RampValue::Color([1.0, 1.0, 1.0]), interp: None },
                ];
            } else {
                self.stops = vec![
                    RampStop { pos: 0.0, value: RampValue::Float(0.0), interp: None },
                    RampStop { pos: 1.0, value: RampValue::Float(1.0), interp: None },
                ];
            }
        }
    }

    pub fn to_value(&self) -> Value {
        match self.format {
            RampFormat::ArrayOfPairs => {
                let pairs: Vec<Value> = self.stops.iter().map(|s| {
                    let v = match &s.value {
                        RampValue::Float(f) => json!(*f),
                        RampValue::Color(c) => json!([c[0], c[1], c[2]]),
                        RampValue::ColorAlpha(c) => json!([c[0], c[1], c[2], c[3]]),
                    };
                    json!([s.pos, v])
                }).collect();
                json!(pairs)
            }
            RampFormat::ArrayOfStops => {
                let stops: Vec<Value> = self.stops.iter().map(|s| {
                    let mut map = serde_json::Map::new();
                    map.insert("pos".into(), json!(s.pos));
                    match &s.value {
                        RampValue::Float(f) => { map.insert("value".into(), json!(*f)); }
                        RampValue::Color(c) => { map.insert("color".into(), json!([c[0], c[1], c[2]])); }
                        RampValue::ColorAlpha(c) => { map.insert("color".into(), json!([c[0], c[1], c[2], c[3]])); }
                    }
                    if let Some(interp) = &s.interp {
                        map.insert("interpolation".into(), json!(interp.as_str()));
                    }
                    Value::Object(map)
                }).collect();
                json!(stops)
            }
            RampFormat::KeysValuesObject => {
                let keys: Vec<f32> = self.stops.iter().map(|s| s.pos).collect();
                let values: Vec<Value> = self.stops.iter().map(|s| {
                    match &s.value {
                        RampValue::Float(f) => json!(*f),
                        RampValue::Color(c) => json!([c[0], c[1], c[2]]),
                        RampValue::ColorAlpha(c) => json!([c[0], c[1], c[2], c[3]]),
                    }
                }).collect();
                json!({
                    "keys": keys,
                    "values": values,
                    "basis": self.interpolation.as_str()
                })
            }
            RampFormat::StopsObject => {
                let stops: Vec<Value> = self.stops.iter().map(|s| {
                    let mut map = serde_json::Map::new();
                    map.insert("pos".into(), json!(s.pos));
                    match &s.value {
                        RampValue::Float(f) => { map.insert("value".into(), json!(*f)); }
                        RampValue::Color(c) => { map.insert("color".into(), json!([c[0], c[1], c[2]])); }
                        RampValue::ColorAlpha(c) => { map.insert("color".into(), json!([c[0], c[1], c[2], c[3]])); }
                    }
                    if let Some(interp) = &s.interp {
                        map.insert("interpolation".into(), json!(interp.as_str()));
                    }
                    Value::Object(map)
                }).collect();
                json!({
                    "stops": stops,
                    "interpolation": self.interpolation.as_str()
                })
            }
        }
    }

    pub fn sort_stops(&mut self) {
        self.stops.sort_by(|a, b| a.pos.partial_cmp(&b.pos).unwrap_or(std::cmp::Ordering::Equal));
    }

    pub fn eval_float(&self, t: f32) -> f32 {
        if self.stops.is_empty() { return 0.0; }
        if self.stops.len() == 1 || t <= self.stops[0].pos {
            return self.stops[0].value.as_float();
        }
        let last = self.stops.last().unwrap();
        if t >= last.pos {
            return last.value.as_float();
        }
        for i in 0..self.stops.len() - 1 {
            let s0 = &self.stops[i];
            let s1 = &self.stops[i + 1];
            if t >= s0.pos && t <= s1.pos {
                let u = ((t - s0.pos) / (s1.pos - s0.pos).max(1e-6)).clamp(0.0, 1.0);
                let interp = s0.interp.unwrap_or(self.interpolation);
                let v0 = s0.value.as_float();
                let v1 = s1.value.as_float();
                return match interp {
                    RampInterpolation::Constant => v0,
                    RampInterpolation::Linear => (1.0 - u) * v0 + u * v1,
                    RampInterpolation::Smooth => {
                        let s = u * u * (3.0 - 2.0 * u);
                        (1.0 - s) * v0 + s * v1
                    }
                };
            }
        }
        0.0
    }

    pub fn eval_color(&self, t: f32) -> [f32; 4] {
        if self.stops.is_empty() { return [0.0, 0.0, 0.0, 1.0]; }
        if self.stops.len() == 1 || t <= self.stops[0].pos {
            return self.stops[0].value.as_rgba();
        }
        let last = self.stops.last().unwrap();
        if t >= last.pos {
            return last.value.as_rgba();
        }
        for i in 0..self.stops.len() - 1 {
            let s0 = &self.stops[i];
            let s1 = &self.stops[i + 1];
            if t >= s0.pos && t <= s1.pos {
                let u = ((t - s0.pos) / (s1.pos - s0.pos).max(1e-6)).clamp(0.0, 1.0);
                let interp = s0.interp.unwrap_or(self.interpolation);
                let c0 = s0.value.as_rgba();
                let c1 = s1.value.as_rgba();
                let factor = match interp {
                    RampInterpolation::Constant => 0.0,
                    RampInterpolation::Linear => u,
                    RampInterpolation::Smooth => u * u * (3.0 - 2.0 * u),
                };
                return [
                    (1.0 - factor) * c0[0] + factor * c1[0],
                    (1.0 - factor) * c0[1] + factor * c1[1],
                    (1.0 - factor) * c0[2] + factor * c1[2],
                    (1.0 - factor) * c0[3] + factor * c1[3],
                ];
            }
        }
        [0.0, 0.0, 0.0, 1.0]
    }

    pub fn add_stop(&mut self, pos: f32) -> usize {
        let pos = pos.clamp(0.0, 1.0);
        let val = if self.is_color {
            let c = self.eval_color(pos);
            RampValue::Color([c[0], c[1], c[2]])
        } else {
            RampValue::Float(self.eval_float(pos))
        };
        let new_stop = RampStop { pos, value: val, interp: None };
        self.stops.push(new_stop);
        self.sort_stops();
        self.stops.iter().position(|s| (s.pos - pos).abs() < 1e-4).unwrap_or(0)
    }

    pub fn delete_stop(&mut self, index: usize) -> bool {
        if self.stops.len() <= 1 || index >= self.stops.len() {
            return false;
        }
        self.stops.remove(index);
        true
    }

    pub fn move_stop(&mut self, index: usize, new_pos: f32) -> usize {
        if index >= self.stops.len() { return index; }
        let new_pos = new_pos.clamp(0.0, 1.0);
        self.stops[index].pos = new_pos;
        let target_stop = self.stops[index].clone();
        self.sort_stops();
        self.stops.iter().position(|s| s.pos == target_stop.pos && s.value == target_stop.value).unwrap_or(0)
    }
}

pub fn is_ramp_param(param_name: &str, meta: Option<&Value>, value: &Value) -> bool {
    if let Some(m) = meta {
        if let Some(k) = m["kind"].as_str() {
            if k == "float_ramp" || k == "color_ramp" || k == "ramp" {
                return true;
            }
        }
    }
    if param_name == "ramp"
        || param_name == "float_ramp"
        || param_name == "color_ramp"
        || param_name.ends_with("_ramp")
    {
        return true;
    }
    RampData::is_ramp_value(value)
}

#[allow(clippy::too_many_arguments)]
fn row(ui: &mut egui::Ui, v: &View<'_>, out: &mut Actions, id: &str, typ: &str, param: &ParamSpec, locked: bool, errors: &mut Vec<String>) {
    let default = (param.default)();
    let old = value_of(v, id, param.name, &default);
    let mut value = old.clone();
    let resolved_channels = ui.ctx().data(|d| d.get_temp::<std::sync::Arc<minihou_core::channels::ResolvedChannels>>(egui::Id::new("param_panel_resolved")))
        .or_else(|| minihou_core::Graph::from_value(v.doc).ok().map(|graph| std::sync::Arc::new(minihou_core::channels::resolve_graph(&graph))));
    if let Some(resolved_all) = &resolved_channels {
        if let Some(resolved) = resolved_all.params.get(id).and_then(|p| p.get(param.name)) {
            value = resolved.clone();
        }
    }
    let modified = old != default;
    let meta = param_meta(typ, param.name);
    let synthetic_meta;
    let meta = if param.name == "uniform_scale" {
        let mut m = meta.cloned().unwrap_or_else(|| json!({"kind": "float", "default": 1.0}));
        if m.get("min").is_none() { m["min"] = json!(0.0); }
        if m.get("soft_max").is_none() && m.get("max").is_none() { m["soft_max"] = json!(10.0); }
        synthetic_meta = m;
        Some(&synthetic_meta)
    } else {
        meta
    };
    let pick_mode = pick_domain_for(typ, param.name, meta, &v.doc["nodes"][id]["params"]);
    let kind = meta.map_or_else(|| param_kind(&default), |m| metadata_kind(m, &default));
    // Metadata-driven optional controls (for example Ray's distance and hit-output toggles).
    let condition_enabled = meta.and_then(|m| m["enabled_if"].as_object()).is_none_or(|conditions| {
        conditions.iter().all(|(name, expected)| {
            let fallback = nodes::get(typ).and_then(|s| s.params.iter().find(|p| p.name == name)).map(|p| (p.default)()).unwrap_or(Value::Null);
            value_of(v, id, name, &fallback) == *expected
        })
    });
    let channel_reason = resolved_channels.as_ref().and_then(|resolved| resolved.diagnostics.iter()
        .find(|d| d.node == id && (d.param == param.name || d.param.starts_with(&format!("{}.",param.name)))).map(|d| d.message.clone()));
    let reason = channel_reason.or_else(|| invalid_reason(meta, &old)).or_else(|| node_error(v, id).and_then(|error| {
        error.to_lowercase().contains(&param.name.replace('_', " ")).then(|| error.to_owned())
    }));
    if let Some(reason) = &reason { errors.push(format!("{}: {reason}", title(param.name))); }
    let is_ramp = is_ramp_param(param.name, meta, &value);
    let is_color = !is_ramp && is_color_param(param.name, meta, &value);
    let is_file = !is_ramp && !is_color && is_file_param(param.name, meta);

    let row_height = if is_ramp { 76.0 } else { 28.0 };
    let (r, row_response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), row_height), Sense::click());
    let layout = ParamRowLayout::compute(r);
    if ui.ctx().data(|d| d.get_temp::<(String,String,f64)>(egui::Id::new("link_flash")))
        .is_some_and(|(node,name,time)| node == id && (name == param.name || name.starts_with(&format!("{}.",param.name))) && ui.ctx().input(|i|i.time) - time < 1.5) {
        ui.painter().rect_filled(r,3.,v.theme.accent.gamma_multiply(0.22));
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
    }

    let param_locked = v.doc["nodes"][id]["locked_params"].as_array().is_some_and(|a| a.iter().any(|x| x == param.name));
    let channels = v.doc["nodes"][id]["channels"].as_object();
    let linked = channels.is_some_and(|c| c.keys().any(|key| key == param.name || key.starts_with(&format!("{}.",param.name))));
    let enabled = param.supported && !locked && !param_locked && !linked && condition_enabled;
    if linked {
        ui.painter().rect_filled(layout.control_rect, 4., Color32::from_rgba_unmultiplied(48, 164, 102, 42));
        ui.painter().rect_stroke(layout.control_rect, 4., Stroke::new(1., Color32::from_rgb(62, 190, 110)), StrokeKind::Inside);
        paint_text(ui, Pos2::new(layout.label_rect.left()+4., r.center().y), "↗", 12., false, Color32::from_rgb(62, 190, 110), Align2::LEFT_CENTER);
    }
    if param_locked { flag_icon(ui, Pos2::new(layout.label_rect.left()+4.,r.center().y), "Locked", v.theme.warn); }
    if modified {
        ui.painter().circle_filled(
            Pos2::new(r.left() + 9., if is_ramp { r.top() + 14. } else { r.center().y }),
            3.,
            if enabled { v.theme.accent } else { v.theme.accent.gamma_multiply(0.5) },
        );
    }

    let is_length = crate::units::is_length_param(typ, param.name);
    let units_opt = if is_length { Some(v.units) } else { None };
    let ink = if !enabled { v.theme.faint } else if modified { v.theme.text } else { v.theme.dim };
    let label_text = if typ == "scene_objects" && param.name == "piece_attrib" { "Object ID Attribute".to_owned() }
        else if is_length { format!("{} ({})", title(param.name), v.units.display.symbol()) } else { title(param.name) };

    let is_bool = kind == ParamKind::Bool;
    let is_vector = kind == ParamKind::Vec3 || value.as_array().is_some_and(|a| (2..=4).contains(&a.len()) && a.iter().all(Value::is_number));

    let font = FontId::proportional(12.);
    let displayed_label = if !is_bool {
        truncate_with_ellipsis(ui, &label_text, &font, layout.label_rect.width())
    } else {
        String::new()
    };

    if !is_bool {
        // Houdini layout: labels are RIGHT-aligned and vertically centred against the row
        let label_pos_y = if is_ramp { r.top() + 14. } else { r.center().y };
        paint_text(
            ui,
            Pos2::new(layout.label_rect.right(), label_pos_y),
            &displayed_label,
            12.,
            false,
            ink,
            Align2::RIGHT_CENTER,
        );

        let label_sense = if is_vector && enabled { Sense::click_and_drag() } else { Sense::click() };
        let label_response = ui.interact(layout.label_rect, ui.id().with((id, param.name, "label")), label_sense);
        let doc = meta.and_then(|m| m["doc"].as_str()).unwrap_or("");
        if !param.supported {
            label_response.clone().on_hover_text(DISABLED);
        } else if !doc.is_empty() {
            label_response.clone().on_hover_text(doc);
        } else if displayed_label != label_text {
            label_response.clone().on_hover_text(&label_text);
        }
        label_response.context_menu(|ui| parameter_menu(ui,v,out,id,param.name,&default));

        if is_vector && enabled {
            if label_response.hovered() || label_response.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            }
            if label_response.dragged_by(egui::PointerButton::Middle) {
                let mut delta = ui.ctx().input(|i| i.pointer.delta());
                let screen = ui.ctx().content_rect();
                if screen.width() > 10.0 && screen.height() > 10.0 {
                    crate::wrap_drag(ui.ctx(), screen, &mut delta);
                }
                let speed = meta.and_then(slider_range).map_or(0.01, |r| ((r.end() - r.start()) / 400.).clamp(0.001, 1.));
                let step = if let Some(u) = units_opt {
                    delta.x as f64 * speed * u.display.metres() / u.unit_scale
                } else {
                    delta.x as f64 * speed
                };
                if step != 0. {
                    let range = meta.and_then(|m| m["min"].as_f64()).unwrap_or(f64::NEG_INFINITY)..=meta.and_then(|m| m["max"].as_f64()).unwrap_or(f64::INFINITY);
                    if let Some(arr) = value.as_array() {
                        let mut v: Vec<f64> = arr.iter().map(|item| item.as_f64().unwrap_or(0.)).collect();
                        for x in &mut v {
                            *x = (*x + step).clamp(*range.start(), *range.end());
                        }
                        value = json!(v);
                    } else {
                        let mut v = [0, 1, 2].map(|i| value.get(i).and_then(Value::as_f64).unwrap_or(0.));
                        for x in &mut v {
                            *x = (*x + step).clamp(*range.start(), *range.end());
                        }
                        value = json!(v);
                    }
                }
            }
        }
    }

    if layout.control_rect.width() > 4. {
        if reason.is_some() {
            ui.painter().rect_stroke(layout.control_rect, 4., Stroke::new(1.0_f32, v.theme.err), StrokeKind::Outside);
        }
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(layout.control_rect)
                .id_salt((id, param.name, "widget"))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        child.set_width(layout.control_rect.width());
        child.spacing_mut().item_spacing.x = 4.;
        if !enabled {
            child.visuals_mut().override_text_color = Some(v.theme.faint);
            child.visuals_mut().widgets.noninteractive.fg_stroke.color = v.theme.faint;
            child.visuals_mut().widgets.inactive.fg_stroke.color = v.theme.faint;
        }
        let bool_label = if is_bool {
            let max_w = (layout.control_rect.width() - 24.0).max(10.0);
            Some(truncate_with_ellipsis(ui, &label_text, &font, max_w))
        } else {
            None
        };
        let response = child.add_enabled_ui(enabled, |ui| {
            if is_ramp {
                ramp_widget(ui, &mut value, meta, v.theme, layout.control_rect.width(), id, param.name)
            } else if is_color {
                color_widget(ui, &mut value, meta, Some(v.theme), layout.control_rect.width(), id, param.name)
            } else if is_file {
                file_widget(ui, &mut value, meta, typ, param.name, layout.control_rect.width())
            } else if typ == "blender_object" && param.name == "object" {
                super::object_search::field(ui, &mut value, layout.control_rect.width(), v.theme)
            } else if pick_mode.is_none() && kind == ParamKind::String && is_output_group_field(param.name, meta) {
                let (clicked, hovered) = target_field(ui, &mut value, layout.control_rect.width(), enabled);
                if hovered {
                    let pass = ui.ctx().cumulative_pass_nr();
                    let name = value.as_str().unwrap_or("").to_owned();
                    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("param_target_hover"), (id.to_owned(), name, pass)));
                }
                if clicked {
                    out.push(UiAction::SelectOutputGroup { node: id.to_owned(), name: value.as_str().unwrap_or("").to_owned() });
                }
                false
            } else if pick_mode.is_some() && kind == ParamKind::String {
                let active = ui.ctx().data(|d| d.get_temp::<(String,String)>(egui::Id::new("param_pick_active")))
                    .is_some_and(|(node,name)| node == id && name == param.name);
                let (clicked, hovered) = pick_field(ui, &mut value, layout.control_rect.width(), active, enabled);
                if hovered && !active {
                    // viewport previews the field's current group while the arrow is hovered (stamped so a stale hover expires)
                    let pass = ui.ctx().cumulative_pass_nr();
                    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("param_pick_hover"),
                        (id.to_owned(), param.name.to_owned(), pick_mode.unwrap(), meta.is_some_and(|m| m["pick_format"] == "pairs"), pass)));
                }
                if clicked {
                    out.push(UiAction::PickParameter { node:id.to_owned(), name:param.name.to_owned(), mode:pick_mode.unwrap(),
                        pairs:meta.is_some_and(|m| m["pick_format"] == "pairs") });
                }
                clicked
            } else {
                let mut channel_menu = |ui: &mut egui::Ui, axis| vector_channel_menu(ui, v, out, id, param.name, axis);
                widget_styled(
                    ui,
                    kind,
                    meta,
                    &mut value,
                    Some(v.theme),
                    layout.control_rect.width(),
                    units_opt,
                    bool_label.as_deref(),
                    Some(&mut channel_menu),
                )
            }
        }).response;
        response.clone().context_menu(|ui| parameter_menu(ui,v,out,id,param.name,&default));
        if linked {
            let expressions = channels.into_iter().flat_map(|c| c.iter()).filter(|(key,_)| *key == param.name || key.starts_with(&format!("{}.",param.name)))
                .map(|(key,value)| format!("{key} = {}", value.as_str().unwrap_or(""))).collect::<Vec<_>>().join("\n");
            response.clone().on_hover_text(format!("{expressions}\nValue: {value}"));
        } else if !enabled {
            response.on_disabled_hover_text(if locked || param_locked {
                "Locked"
            } else if !condition_enabled {
                "Enable the controlling parameter first"
            } else {
                DISABLED
            });
        } else if is_bool {
            let doc = meta.and_then(|m| m["doc"].as_str()).unwrap_or("");
            if !doc.is_empty() {
                response.on_hover_text(doc);
            } else if bool_label.as_deref().is_some_and(|l| l != label_text) {
                response.on_hover_text(&label_text);
            }
        }
    }
    row_response.context_menu(|ui| parameter_menu(ui,v,out,id,param.name,&default));

    if param.supported && !locked && value != old {
        let dragging = ui.ctx().input(|input| input.pointer.primary_down() || input.pointer.middle_down());
        out.push(UiAction::Edit { edit: Edit::Param { node: id.to_owned(), name: param.name.to_owned(), value }, continuous: dragging });
    }
    if modified && !locked && !param_locked && !linked && icon(ui, layout.reset_rect, ui.id().with((id, param.name, "reset")), true, false, v.theme)
        .on_hover_text(format!("Reset {} to {}", label_text, default)).clicked() {
        out.push(UiAction::Edit { edit: Edit::Param { node: id.to_owned(), name: param.name.to_owned(), value: default.clone() }, continuous: false });
    }
    let on = minihou_app::is_exposed(v.doc, id, param.name);
    if icon(ui, layout.expose_rect, ui.id().with((id, param.name, "expose")), false, on, v.theme)
        .on_hover_text(if on { "Exposed: click to remove" } else { "Expose parameter" }).clicked() {
        out.push(UiAction::Edit { edit: minihou_app::toggle_exposed(v.doc, id, param.name), continuous: false });
    }
    let matching: Vec<_> = minihou_app::modifier_inputs::entries(v.doc).into_iter().filter(|entry| {
        match kind {
            ParamKind::Float => entry["kind"] == "float",
            ParamKind::Integer => entry["kind"] == "int",
            ParamKind::Bool => entry["kind"] == "bool",
            ParamKind::Vec3 => entry["kind"] == "vector" || entry["kind"] == "rotation",
            ParamKind::String => ["string", "menu", "object", "collection", "material", "image", "texture"].iter().any(|k| entry["kind"] == *k),
            ParamKind::Unsupported => false,
        }
    }).collect();
    if !matching.is_empty() || old.get("modifier_input").is_some() {
        ui.horizontal(|ui| {
            if let Some(link) = old.get("modifier_input") { ui.label(format!("Blender input: {link}")); }
            ui.menu_button("Link to Blender input", |ui| {
                for entry in &matching {
                    let name = entry["name"].as_str().unwrap_or("");
                    if ui.button(name).clicked() {
                        out.push(UiAction::Edit { edit: Edit::Param { node: id.into(), name: param.name.into(),
                            value: json!({"modifier_input":{"node":entry["node"],"id":entry["id"]}}) }, continuous: false });
                        ui.close();
                    }
                }
                if old.get("modifier_input").is_some() && ui.button("Unlink").clicked() {
                    out.push(UiAction::Edit { edit: Edit::Param { node: id.into(), name: param.name.into(), value: default.clone() }, continuous: false });
                    ui.close();
                }
            });
        });
    }
}

fn unsupported(ui: &mut egui::Ui, v: &View<'_>, id: &str, typ: &str) {
    let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 48.), Sense::hover());
    ui.painter().rect_filled(Rect::from_min_size(r.min + Vec2::new(12., 18.), Vec2::splat(10.)), 2., v.theme.dim);
    paint_text(ui, r.min + Vec2::new(30., 23.), id, 13., true, v.theme.text, Align2::LEFT_CENTER);
    paint_text(ui, Pos2::new(r.right() - 12., r.top() + 23.), typ, 10., true, v.theme.dim, Align2::RIGHT_CENTER);
    let (warn, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 90.), Sense::hover());
    let box_rect = Rect::from_min_max(warn.min + Vec2::new(12., 0.), warn.max - Vec2::new(12., 0.));
    ui.painter().rect(box_rect, 6., v.theme.warn_bg, Stroke::new(1.0_f32, v.theme.warn), StrokeKind::Inside);
    paint_text(ui, box_rect.min + Vec2::new(10., 13.), "Unsupported node type", 12., false, v.theme.text, Align2::LEFT_TOP);
    paint_text(ui, box_rect.min + Vec2::new(10., 35.), "This build has no implementation.", 11., false, v.theme.dim, Align2::LEFT_TOP);
    paint_text(ui, box_rect.min + Vec2::new(10., 53.), "The node is saved unchanged.", 11., false, v.theme.dim, Align2::LEFT_TOP);
    ui.add_space(12.);
    ui.colored_label(v.theme.dim, "STORED PARAMETERS (READ ONLY)");
    if let Some(params) = v.doc["nodes"][id]["params"].as_object() {
        for (name, value) in params {
            let (row, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 22.), Sense::hover());
            let row = Rect::from_min_max(row.min + Vec2::new(12., 0.), row.max - Vec2::new(12., 0.));
            ui.painter().rect_filled(row, 0., v.theme.input);
            paint_text(ui, row.min + Vec2::new(8., 11.), &format!("{name}: {value}"), 11., true, v.theme.dim, Align2::LEFT_CENTER);
        }
    }
}

fn modifier_inputs_panel(ui: &mut egui::Ui, v: &View<'_>, out: &mut Actions, node: &str) {
    use minihou_app::modifier_inputs::{validate, KINDS};
    let entries = v.doc["nodes"][node]["modifier_inputs"].as_array().cloned().unwrap_or_default();
    ui.separator();
    ui.label("Blender inputs");
    for (index, entry) in entries.iter().enumerate() {
        let ident = entry["id"].as_str().unwrap_or("");
        let key = egui::Id::new(("modifier_input_name", node, ident));
        let mut name = ui.ctx().data_mut(|d| d.get_persisted::<String>(key)).unwrap_or_else(|| entry["name"].as_str().unwrap_or("").to_owned());
        ui.horizontal(|ui| {
            let response = ui.add(egui::TextEdit::singleline(&mut name).desired_width(110.));
            if response.changed() { ui.ctx().data_mut(|d| d.insert_persisted(key, name.clone())); }
            if response.lost_focus() && name != entry["name"] {
                let mut next = entries.clone(); next[index]["name"] = json!(name.trim());
                if validate(&next).is_ok() {
                    out.push(UiAction::Edit { edit: Edit::ModifierInputs { node: node.into(), inputs: next }, continuous: false });
                }
            }
            ui.label(entry["kind"].as_str().unwrap_or("?"));
            for (label, dest) in [("↑", index.checked_sub(1)), ("↓", (index + 1 < entries.len()).then_some(index + 1))] {
                if ui.add_enabled(dest.is_some(), egui::Button::new(label)).clicked() {
                    let mut next = entries.clone(); next.swap(index, dest.unwrap());
                    out.push(UiAction::Edit { edit: Edit::ModifierInputs { node: node.into(), inputs: next }, continuous: false });
                }
            }
            if ui.button("×").on_hover_text("Remove input").clicked() {
                let mut next = entries.clone(); next.remove(index);
                out.push(UiAction::Edit { edit: Edit::ModifierInputs { node: node.into(), inputs: next }, continuous: false });
            }
        });
        ui.small(format!("id: {ident} · default: {}", entry["default"]));
    }
    let key = egui::Id::new(("modifier_input_draft", node));
    let mut draft = ui.ctx().data_mut(|d| d.get_persisted::<String>(key)).unwrap_or_default();
    let kind_key = egui::Id::new(("modifier_input_kind", node));
    let mut kind = ui.ctx().data_mut(|d| d.get_persisted::<String>(kind_key)).unwrap_or_else(|| "float".into());
    let default_key = egui::Id::new(("modifier_input_default", node));
    let mut default = ui.ctx().data_mut(|d| d.get_persisted::<String>(default_key)).unwrap_or_default();
    let min_key = egui::Id::new(("modifier_input_min", node));
    let mut min = ui.ctx().data_mut(|d| d.get_persisted::<String>(min_key)).unwrap_or_default();
    let max_key = egui::Id::new(("modifier_input_max", node));
    let mut max = ui.ctx().data_mut(|d| d.get_persisted::<String>(max_key)).unwrap_or_default();
    let subtype_key = egui::Id::new(("modifier_input_subtype", node));
    let mut subtype = ui.ctx().data_mut(|d| d.get_persisted::<String>(subtype_key)).unwrap_or_default();
    let error_key = egui::Id::new(("modifier_input_error", node));
    let mut error = ui.ctx().data_mut(|d| d.get_persisted::<String>(error_key)).unwrap_or_default();
    ui.horizontal(|ui| { ui.label("Name"); ui.text_edit_singleline(&mut draft); });
    ui.horizontal(|ui| {
        ui.label("Type");
        egui::ComboBox::from_id_salt((node, "input_type")).selected_text(&kind).show_ui(ui, |ui| {
            for option in KINDS { ui.selectable_value(&mut kind, (*option).to_owned(), *option); }
        });
    });
    ui.horizontal(|ui| { ui.label("Default JSON"); ui.text_edit_singleline(&mut default); });
    if matches!(kind.as_str(), "float" | "int" | "vector" | "rotation") {
        ui.horizontal(|ui| { ui.label("Min"); ui.text_edit_singleline(&mut min); ui.label("Max"); ui.text_edit_singleline(&mut max); });
    }
    if matches!(kind.as_str(), "vector" | "float" | "int" | "string" | "menu") {
        ui.horizontal(|ui| { ui.label(if kind == "menu" { "Options (comma separated)" } else { "Subtype" }); ui.text_edit_singleline(&mut subtype); });
    }
    for (key, value) in [(key, &draft), (kind_key, &kind), (default_key, &default), (min_key, &min), (max_key, &max), (subtype_key, &subtype)] {
        ui.ctx().data_mut(|d| d.insert_persisted(key, value.clone()));
    }
    if ui.button("Add input").clicked() {
        // Same path as Blender's input_define_v1 (ids, collision suffixes, validation).
        let options: Vec<String> = subtype.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect();
        let parsed = if default.trim().is_empty() { Ok(None) } else { serde_json::from_str::<Value>(&default).map(Some) };
        match parsed {
            Ok(value) => {
                let draft = minihou_app::input_define::Draft { name: draft.trim().to_owned(), kind: kind.clone(), default: value,
                    min: min.trim().parse().ok(), max: max.trim().parse().ok(),
                    subtype: if kind == "menu" { String::new() } else { subtype.trim().to_owned() }, options, doc: String::new() };
                match minihou_app::input_define::add_entry(v.doc, node, &draft) {
                    Ok((next, _)) => {
                        out.push(UiAction::Edit { edit: Edit::ModifierInputs { node: node.into(), inputs: next }, continuous: false });
                        error.clear();
                        ui.ctx().data_mut(|d| { d.insert_persisted(key, String::new()); d.insert_persisted(default_key, String::new()); });
                    }
                    Err(e) => error = e.to_string(),
                }
            }
            Err(_) => error = "Default must be valid JSON".into(),
        }
        ui.ctx().data_mut(|d| d.insert_persisted(error_key, error.clone()));
    }
    if !error.is_empty() { ui.colored_label(v.theme.err, error); }
    ui.small("Link a parameter with this input's stable id in the node parameter menu.");
}

pub fn show(ui: &mut egui::Ui, v: &View<'_>, out: &mut Actions) {
    if let Ok(graph)=minihou_core::Graph::from_value(v.doc) {
        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("param_panel_resolved"),std::sync::Arc::new(minihou_core::channels::resolve_graph(&graph))));
    }
    expression_dialog(ui.ctx(), v, out);
    let (bar, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 32.), Sense::hover());
    ui.painter().rect_filled(bar, 0., v.theme.panel2);
    ui.painter().line_segment([bar.left_bottom(), bar.right_bottom()], Stroke::new(1.0_f32, v.theme.stroke));
    paint_text(ui, Pos2::new(bar.left() + 10., bar.center().y), "Parameters", 12., false, v.theme.text, Align2::LEFT_CENTER);
    let id = v.selected;
    let Some(node) = v.graph.nodes.iter().find(|(name, _)| name == id).map(|(_, node)| node) else {
        ui.add_space(20.);
        ui.colored_label(v.theme.dim, "Nothing selected. Click a node in the network to edit its parameters.");
        return;
    };
    let typ = &node.node_type;
    if typ == "subnet" {
        promoted_panel(ui, v, out, id);
        return;
    }
    if nodes::get(typ).is_none() {
        unsupported(ui, v, id, typ);
        return;
    }
    let flags = v.flags_of(id);
    header(ui, v, out, id, typ, node.bypass, flags.locked, flags.template);
    if typ == "polysplit" && v.doc["nodes"][id]["params"]["mode"] == "edge_loop" {
        loop_cut_redo(ui, v, out, id, flags.locked);
    }
    egui::ScrollArea::vertical().show(ui, |ui| {
        let (r, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 26.), Sense::click());
        ui.painter().rect_filled(r, 0., v.theme.panel2);
        ui.painter().line_segment([r.left_top(), r.right_top()], Stroke::new(1.0_f32, v.theme.stroke));
        ui.painter().line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(1.0_f32, v.theme.stroke));
        let key = egui::Id::new(("params_group", typ));
        let mut open = ui.ctx().data_mut(|d| d.get_persisted::<bool>(key).unwrap_or(true));
        if response.clicked() { open = !open; ui.ctx().data_mut(|d| d.insert_persisted(key, open)); }
        let (cx, cy) = (r.left() + 14., r.center().y);
        let chevron: [Pos2; 3] = if open { [Pos2::new(cx - 3., cy - 1.5), Pos2::new(cx, cy + 1.5), Pos2::new(cx + 3., cy - 1.5)] }
            else { [Pos2::new(cx - 1.5, cy - 3.), Pos2::new(cx + 1.5, cy), Pos2::new(cx - 1.5, cy + 3.)] };
        ui.painter().add(egui::Shape::line(chevron.to_vec(), Stroke::new(1.5_f32, v.theme.text)));
        paint_text(ui, Pos2::new(r.left() + 26., r.center().y), &title(typ), 12., false, v.theme.text, Align2::LEFT_CENTER);
        if open {
            ui.add_space(6.);
            let mut errors = Vec::new();
            if let Some(spec) = nodes::get(typ) {
                let mut section=None;
                for param in spec.params {
                    if typ == "bezier" && param.name == "controls" { continue; }
                    if typ == "knife" && param.name == "cuts" { continue; }
                    if typ == "attribute_create" && param.name != "group" { continue; }
                    let next=deformer_section(typ,param.name);
                    if next.is_some() && next!=section {
                        section=next;
                        ui.add_space(5.);
                        ui.label(egui::RichText::new(next.unwrap()).strong().color(v.theme.accent));
                    }
                    ui.push_id(param.name, |ui| row(ui, v, out, id, typ, param, flags.locked, &mut errors));
                }
                if typ == "attribute_create" { attribute_create_panel(ui,v,out,id,flags.locked); }
                if typ == "bezier" { bezier_controls_panel(ui,v,out,id,flags.locked); }
                if typ == "knife" { knife_chains_panel(ui,v,out,id,flags.locked); }
            }
            if typ == "modifier_input" { modifier_inputs_panel(ui, v, out, id); }
            if let Some(message) = super::waiting::waiting_of(&v.cooking.stats, id) { ui.add_space(4.); super::waiting::panel_note(ui, v.theme, message); }
            ui.add_space(6.);
            for error in errors {
                let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 34.), Sense::hover());
                let r = Rect::from_min_max(r.min + Vec2::new(12., 0.), r.max - Vec2::new(12., 0.));
                ui.painter().rect(r, 6., v.theme.err_bg, Stroke::new(1.0_f32, v.theme.err), StrokeKind::Inside);
                paint_text(ui, Pos2::new(r.left() + 10., r.center().y), &error, 11., true, v.theme.err, Align2::LEFT_CENTER);
            }
        }
    });
}

/// A subnet's public numeric controls live in its own `params`, with a mapping
/// to the target node inside `inner.nodes`.
fn promoted_panel(ui: &mut egui::Ui, v: &View<'_>, out: &mut Actions, id: &str) {
    ui.heading(format!("Subnet: {id}"));
    let subnet=&v.doc["nodes"][id];
    let mut candidates=Vec::<(String,String,Value)>::new();
    if let Some(inner)=subnet["inner"]["nodes"].as_object() {
        for (target,node) in inner {
            if let Some(spec)=node["type"].as_str().and_then(nodes::get) {
                for param in spec.params {
                    let value=node["params"].get(param.name).cloned().unwrap_or_else(||(param.default)());
                    if value.is_number() {candidates.push((target.clone(),param.name.into(),value));}
                    else if let Some(xs)=value.as_array().filter(|xs|xs.len()==3 && xs.iter().all(Value::is_number)) {
                        for (i,axis) in ["x","y","z"].iter().enumerate() {candidates.push((target.clone(),format!("{}.{}",param.name,axis),xs[i].clone()));}
                    }
                }
            }
        }
    }
    let choice_id=egui::Id::new(("promote_choice",id));
    let mut choice=ui.ctx().data(|d|d.get_temp::<usize>(choice_id)).unwrap_or(0).min(candidates.len().saturating_sub(1));
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt(choice_id).selected_text(candidates.get(choice).map(|(node,param,_)|format!("{node}.{param}")).unwrap_or_else(||"No numeric parameters".into()))
            .show_ui(ui,|ui|for (i,(node,param,_)) in candidates.iter().enumerate(){ui.selectable_value(&mut choice,i,format!("{node}.{param}"));});
        if ui.add_enabled(!candidates.is_empty(),egui::Button::new("Promote")).clicked() {
            let (target,param,value)=&candidates[choice];
            let mut next=v.doc.clone();
            let node=&mut next["nodes"][id];
            let mut name=param.replace('.',"_");
            if node["promoted_params"].as_array().is_some_and(|entries|entries.iter().any(|e|e["name"]==name)) {
                let base=name.clone(); let mut i=2;
                while node["promoted_params"].as_array().is_some_and(|entries|entries.iter().any(|e|e["name"]==name)) {name=format!("{base}_{i}");i+=1;}
            }
            if !node["promoted_params"].is_array(){node["promoted_params"]=json!([]);}
            node["promoted_params"].as_array_mut().unwrap().push(json!({"name":name.clone(),"node":target,"param":param,"default":value,"min":0.0,"max":10.0}));
            node["params"][&name]=value.clone();
            out.push(UiAction::Edit {edit:Edit::Replace(next),continuous:false});
        }
    });
    ui.ctx().data_mut(|d|d.insert_temp(choice_id,choice));
    let Some(entries)=subnet["promoted_params"].as_array() else {return};
    for entry in entries {
        let Some(name)=entry["name"].as_str() else {continue};
        let default=entry.get("default").cloned().unwrap_or(json!(0.0));
        let old=subnet["params"].get(name).unwrap_or(&default);
        let Some(mut number)=old.as_f64() else {continue};
        let min=entry["min"].as_f64().unwrap_or(0.0);
        let max=entry["max"].as_f64().unwrap_or(10.0).max(min+f64::EPSILON);
        ui.horizontal(|ui| {
            ui.label(entry["label"].as_str().unwrap_or(name));
            let slider=ui.add(egui::Slider::new(&mut number,min..=max).show_value(false));
            let field=ui.add(egui::DragValue::new(&mut number).speed((max-min)/200.0));
            if slider.changed() || field.changed() {
                out.push(promoted_value_edit(v.doc,id,name,number,slider.dragged() || field.dragged()));
            }
            let channel=&subnet["channels"][name];
            let key=egui::Id::new(("promoted_expression",id,name));
            if ui.button(if channel.is_string() {"ƒx ●"} else {"ƒx"}).on_hover_text("Channel expression").clicked() {
                ui.ctx().data_mut(|d|d.insert_temp(key,channel.as_str().unwrap_or("").to_owned()));
            }
            if let Some(mut expr)=ui.ctx().data(|d|d.get_temp::<String>(key)) {
                let mut open=true;
                let mut applied=false;
                egui::Window::new(format!("Expression: {id}.{name}")).id(key.with("window")).open(&mut open).show(ui.ctx(),|ui| {
                    ui.text_edit_singleline(&mut expr);
                    if ui.button("Apply").clicked() {
                        let mut next=v.doc.clone();
                        if expr.trim().is_empty() {next["nodes"][id]["channels"].as_object_mut().map(|c|c.remove(name));}
                        else {next["nodes"][id]["channels"][name]=json!(expr);}
                        out.push(UiAction::Edit {edit:Edit::Replace(next),continuous:false});
                        applied=true;
                    }
                });
                if open && !applied {ui.ctx().data_mut(|d|d.insert_temp(key,expr));}else{ui.ctx().data_mut(|d|d.remove::<String>(key));}
            }
        });
    }
}
fn promoted_value_edit(doc:&Value,id:&str,name:&str,value:f64,continuous:bool)->UiAction {
    let mut next=doc.clone(); next["nodes"][id]["params"][name]=json!(value);
    UiAction::Edit {edit:Edit::Replace(next),continuous}
}
#[cfg(test)]
mod subnet_parameter_tests {
    use super::*;
    #[test]
    fn promoted_slider_edit_writes_outer_param() {
        let doc=json!({"nodes":{"subnet1":{"type":"subnet","params":{"width":1.0},"inner":{"nodes":{"box1":{"type":"box"}},"output":"box1"}}}});
        let UiAction::Edit {edit:Edit::Replace(next),continuous:true}=promoted_value_edit(&doc,"subnet1","width",4.0,true) else {panic!("expected continuous document edit")};
        assert_eq!(next["nodes"]["subnet1"]["params"]["width"],json!(4.0));
        assert_eq!(doc["nodes"]["subnet1"]["params"]["width"],json!(1.0));
    }
}

fn redo_param(out: &mut Actions, id: &str, name: &str, value: Value, continuous: bool) {
    out.push(UiAction::Edit { edit: Edit::Param { node: id.into(), name: name.into(), value }, continuous });
}

fn attribute_create_panel(ui:&mut egui::Ui,v:&View<'_>,out:&mut Actions,id:&str,locked:bool){
    let params=&v.doc["nodes"][id]["params"];
    let mut attrs=params["attributes"].as_array().cloned().unwrap_or_else(||vec![json!({
        "name":params["name"].as_str().unwrap_or("attrib1"),
        "class":params["domain"].as_str().unwrap_or("points"),
        "type":params["type"].as_str().unwrap_or("float"),
        "size":match params["type"].as_str().unwrap_or("float") {"vector2"=>2,"vector"|"color"=>3,"vector4"=>4,_=>1},
        "default":[0.0,0.0,0.0,0.0],
        "value":params["value"].as_array().map_or(vec![json!(0.0),json!(0.0),json!(0.0),json!(1.0)],|a|{let mut a=a.clone();a.push(json!(1.0));a})
    })]);
    let before=attrs.clone();
    ui.add_enabled_ui(!locked,|ui|{
        ui.horizontal(|ui|{
            ui.label("Number of Attributes");
            let mut count=attrs.len() as u32;
            if ui.add(egui::DragValue::new(&mut count).range(0..=64)).changed(){
                while attrs.len() < count as usize { attrs.push(json!({"name":format!("attrib{}",attrs.len()+1),"class":"points","type":"float","size":1,"default":[0.,0.,0.,0.],"value":[0.,0.,0.,0.]})); }
                attrs.truncate(count as usize);
            }
            if ui.button("+").clicked(){attrs.push(json!({"name":format!("attrib{}",attrs.len()+1),"class":"points","type":"float","size":1,"default":[0.,0.,0.,0.],"value":[0.,0.,0.,0.]}));}
            if ui.button("Clear").clicked(){attrs.clear();}
        });
        let mut remove=None;
        for (i,a) in attrs.iter_mut().enumerate(){
            ui.push_id(i,|ui|{
                ui.separator();
                ui.horizontal(|ui|{ui.strong(format!("Attribute {}",i+1));if ui.button("Remove").clicked(){remove=Some(i);}});
                let mut name=a["name"].as_str().unwrap_or("").to_owned();
                ui.horizontal(|ui|{ui.label("Name");if ui.text_edit_singleline(&mut name).changed(){a["name"]=json!(name);}});
                for (key,label,options) in [("class","Class",&["points","vertices","primitives","detail"][..]),("type","Type",&["float","int","string","vector2","vector","color","vector4"][..])]{
                    let mut current=a[key].as_str().unwrap_or(options[0]).to_owned();
                    ui.horizontal(|ui|{ui.label(label);egui::ComboBox::from_id_salt(key).selected_text(&current).show_ui(ui,|ui|{for option in options{ui.selectable_value(&mut current,(*option).to_owned(),*option);}});});
                    if key=="type" && a[key]!=current {
                        a["size"]=json!(match current.as_str(){"vector2"=>2,"vector"|"color"=>3,"vector4"=>4,_=>1});
                        if current=="string" { a["value"]=json!(""); a["default"]=json!(""); }
                        else if a["value"].is_string() { a["value"]=json!([0.,0.,0.,0.]); a["default"]=json!([0.,0.,0.,0.]); }
                    }
                    a[key]=json!(current);
                }
                let mut size=a["size"].as_u64().unwrap_or(1) as u32;
                ui.horizontal(|ui|{ui.label("Size");ui.add(egui::DragValue::new(&mut size).range(1..=4));});a["size"]=json!(size);
                if a["type"]=="string"{
                    let mut value=a["value"].as_str().unwrap_or("").to_owned();
                    ui.horizontal(|ui|{ui.label("Value");if ui.text_edit_singleline(&mut value).changed(){a["value"]=json!(value);}});
                }else{
                    for key in ["default","value"]{
                        let mut values=a[key].as_array().cloned().unwrap_or_default();values.resize(4,json!(0.0));
                        ui.horizontal(|ui|{ui.label(if key=="default"{"Default"}else{"Value"});for component in values.iter_mut().take(size as usize){let mut x=component.as_f64().unwrap_or(0.);if ui.add(egui::DragValue::new(&mut x).speed(0.1)).changed(){*component=json!(x);}}});
                        a[key]=json!(values);
                    }
                }
            });
        }
        if let Some(i)=remove{attrs.remove(i);}
    });
    if attrs!=before{redo_param(out,id,"attributes",json!(attrs),false);}
}

/// The most common Loop Cut adjustments stay visible above the full node parameters.
fn loop_cut_redo(ui: &mut egui::Ui, v: &View<'_>, out: &mut Actions, id: &str, locked: bool) {
    let params = &v.doc["nodes"][id]["params"];
    let dragging = ui.ctx().input(|i| i.pointer.primary_down());
    ui.add_enabled_ui(!locked, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.strong("Loop Cut redo");
            let mut cuts = params["loops"].as_u64().unwrap_or(1) as i64;
            ui.label("Number of Cuts");
            if ui.add(egui::DragValue::new(&mut cuts).range(1..=64).speed(0.1)).changed() {
                redo_param(out, id, "loops", json!(cuts), dragging);
            }
            let mut smoothness = params["smoothness"].as_f64().unwrap_or(0.);
            ui.label("Smoothness");
            if ui.add(egui::DragValue::new(&mut smoothness).range(-1. ..=1.).speed(0.01)).changed() {
                redo_param(out, id, "smoothness", json!(smoothness), dragging);
            }
            let mut falloff = params["falloff"].as_str().unwrap_or("inverse_square").to_owned();
            ui.label("Falloff");
            egui::ComboBox::from_id_salt((id,"loop_cut_redo_falloff")).selected_text(&falloff)
                .show_ui(ui, |ui| { for option in ["inverse_square", "linear", "smooth"] {
                    ui.selectable_value(&mut falloff, option.into(), option);
                } });
            if params["falloff"].as_str().unwrap_or("inverse_square") != falloff {
                redo_param(out, id, "falloff", json!(falloff), false);
            }
            let mut factor = params["factor"].as_f64().unwrap_or(0.);
            ui.label("Factor");
            if ui.add(egui::DragValue::new(&mut factor).range(-1. ..=1.).speed(0.01)).changed() {
                redo_param(out, id, "factor", json!(factor), dragging);
            }
            for (label, name, default) in [("Even", "even", false), ("Flipped", "flipped", false), ("Clamp", "clamp", true)] {
                let mut value = params[name].as_bool().unwrap_or(default);
                if ui.checkbox(&mut value, label).changed() { redo_param(out, id, name, json!(value), false); }
            }
        });
    });
    ui.separator();
}

#[cfg(test)]
mod mesh_cut_redo_tests {
    use super::*;

    #[test]
    fn redo_values_write_to_polysplit_node_params() {
        let mut doc = minihou_app::Document::parse(json!({"display":"cut1","nodes":{
            "box1":{"type":"box"},
            "cut1":{"type":"polysplit","inputs":{"geo":"box1"},"params":{"mode":"edge_loop","split_edges":"p0-1"}}
        }}).to_string()).unwrap();
        let mut actions = Actions::default();
        for (name, value) in [
            ("loops",json!(4)), ("smoothness",json!(0.25)), ("falloff",json!("linear")),
            ("factor",json!(-0.4)), ("even",json!(true)), ("flipped",json!(true)), ("clamp",json!(false)),
        ] { redo_param(&mut actions,"cut1",name,value,false); }
        for action in actions.0 {
            let UiAction::Edit { edit, .. } = action else { panic!("expected parameter edit") };
            doc.edit(&edit).unwrap();
        }
        let params = &doc.value()["nodes"]["cut1"]["params"];
        assert_eq!(params["loops"],4);
        assert_eq!(params["smoothness"],0.25);
        assert_eq!(params["falloff"],"linear");
        assert_eq!(params["factor"],-0.4);
        assert_eq!(params["even"],true);
        assert_eq!(params["flipped"],true);
        assert_eq!(params["clamp"],false);
    }
}

fn knife_chains_panel(ui:&mut egui::Ui,v:&View<'_>,out:&mut Actions,id:&str,locked:bool) {
    let raw=v.doc["nodes"][id]["params"]["cuts"].as_str().unwrap_or("[]");
    let Ok(mut chains)=serde_json::from_str::<Vec<Value>>(raw) else { ui.colored_label(v.theme.err,"Invalid Knife chains"); return; };
    ui.add_space(5.); ui.horizontal(|ui| {ui.add_space(12.);ui.strong(format!("Cut chains ({})",chains.len()));});
    let mut clear=None;
    for (i,chain) in chains.iter().enumerate() {
        let count=chain.get("points").and_then(Value::as_array).map_or(2,Vec::len);
        ui.horizontal(|ui| {ui.add_space(12.);ui.label(format!("Chain {} · {} points",i+1,count));
            if ui.add_enabled(!locked,egui::Button::new("Clear")).clicked() {clear=Some(i);} });
    }
    if let Some(i)=clear {chains.remove(i);out.push(UiAction::Edit{edit:Edit::Param{node:id.into(),name:"cuts".into(),value:json!(Value::Array(chains).to_string())},continuous:false});}
}

fn bezier_controls_panel(ui:&mut egui::Ui,v:&View<'_>,out:&mut Actions,id:&str,locked:bool) {
    use super::curve_tool::{self,Part,Selected};
    let Some(cs)=curve_tool::visible_controls(ui.ctx(),v.doc,id) else {
        ui.colored_label(v.theme.err,"Invalid Bezier control data");return;
    };
    let selected=curve_tool::selected(ui.ctx(),id);
    let index=selected.index.min(cs.len()-1);
    ui.add_space(5.);
    ui.horizontal(|ui| {ui.add_space(12.);ui.strong(format!("Control point {} of {}",index+1,cs.len()));});
    let mut mode=cs[index].mode.clone();
    ui.add_enabled_ui(!locked,|ui| {
        ui.horizontal(|ui| {
            ui.add_space(12.);ui.label("Handle mode");
            egui::ComboBox::from_id_salt((id,"bezier_mode")).selected_text(&mode).show_ui(ui,|ui| {
                for choice in ["AUTO","VECTOR","ALIGNED","MIRRORED","FREE"] {ui.selectable_value(&mut mode,choice.into(),choice);}
            });
        });
        if mode!=cs[index].mode {
            let closed=v.doc["nodes"][id]["params"]["closed"].as_bool().unwrap_or(false);
            let mut next=curve_tool::resolved(&cs,closed);next[index].mode=mode;
            next=curve_tool::resolved(&next,closed);
            out.push(UiAction::Edit{edit:Edit::Param{node:id.into(),name:"controls".into(),value:curve_tool::write(&next)},continuous:false});
        }
        ui.horizontal(|ui| {
            ui.add_space(12.);
            let append=ui.button("Append point").clicked();
            let closed=v.doc["nodes"][id]["params"]["closed"].as_bool().unwrap_or(false);
            let split=ui.add_enabled(closed||index+1<cs.len(),egui::Button::new("Split segment")).clicked();
            let delete=ui.add_enabled(cs.len()>2,egui::Button::new("Delete point")).clicked();
            if append||split||delete {
                let next=if append {curve_tool::append(&cs,[cs.last().unwrap().p[0]+1.,cs.last().unwrap().p[1],cs.last().unwrap().p[2]])}
                    else if split {curve_tool::split(&cs,index,closed)} else {curve_tool::delete(&cs,index)};
                let selected_index=if append {next.len()-1} else if split {index+1} else {index.min(next.len()-1)};
                curve_tool::set_selected(ui.ctx(),id,Selected{index:selected_index,part:Part::Anchor});
                out.push(UiAction::Edit{edit:Edit::Param{node:id.into(),name:"controls".into(),value:curve_tool::write(&next)},continuous:false});
            }
        });
        let mut p=cs[index].p;
        ui.horizontal(|ui| {
            ui.add_space(12.);
            for (axis,label) in ["X","Y","Z"].into_iter().enumerate() {
                ui.label(label);
                let response=ui.add(egui::DragValue::new(&mut p[axis]).speed(0.1).max_decimals(5));
                if response.changed() && p[axis].is_finite() {
                    let next=curve_tool::move_part(&cs,index,Part::Anchor,p);
                    out.push(UiAction::Edit{edit:Edit::Param{node:id.into(),name:"controls".into(),value:curve_tool::write(&next)},continuous:response.dragged()});
                }
            }
        });
    });
}

/// A 22 px value box in the design's style (input fill, 1 px stroke, 4 px radius); returns the outer and inner rects.
fn boxed(ui: &mut egui::Ui, width: f32, t: &Theme) -> (Rect, Rect) {
    let (r, _) = ui.allocate_exact_size(Vec2::new(width, 22.), Sense::hover());
    let (bg, stroke) = if ui.is_enabled() {
        (t.input, t.stroke)
    } else {
        (t.input.gamma_multiply(0.5), t.stroke.gamma_multiply(0.5))
    };
    ui.painter().rect(r, 4., bg, Stroke::new(1.0_f32, stroke), StrokeKind::Inside);
    (r, r.shrink2(Vec2::new(2., 1.)))
}
const TAB_REQUEST: egui::Id = egui::Id::NULL;
/// Number fields drawn this pass in layout order (the previous pass's list is what Tab navigates).
fn register_field(ctx: &egui::Context, id: egui::Id, enabled: bool) {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| {
        let cur = d.get_temp_mut_or_default::<(u64, Vec<egui::Id>, Vec<egui::Id>)>(egui::Id::new("field_order"));
        if cur.0 != pass { cur.0 = pass; cur.2 = std::mem::take(&mut cur.1); }
        if enabled { cur.1.push(id); }
    });
}
fn last_field_order(ctx: &egui::Context) -> Vec<egui::Id> {
    // mid-pass the current list is partial, so navigate by the previous pass's complete one
    ctx.data(|d| d.get_temp::<(u64, Vec<egui::Id>, Vec<egui::Id>)>(egui::Id::new("field_order"))).map(|(_, _, prev)| prev).unwrap_or_default()
}
/// The field Tab (`dir` 1) or Shift+Tab (-1) moves to from `current`; none past either end.
fn tab_target(order: &[egui::Id], current: egui::Id, dir: i32) -> Option<egui::Id> {
    let i = order.iter().position(|x| *x == current)? as i32 + dir;
    usize::try_from(i).ok().and_then(|i| order.get(i)).copied()
}
/// A number you drag sideways to change and click to type, painted in `rect` (the design's mono 12 px value).
#[allow(clippy::too_many_arguments)] // painter-style helper, called from three places
fn drag_number(ui: &mut egui::Ui, rect: Rect, id: egui::Id, v: &mut f64, speed: f64, range: &std::ops::RangeInclusive<f64>, integer: bool, align: Align2, t: &Theme, units: Option<crate::units::UnitSettings>) -> bool {
    let edit_id = id.with("edit");
    let editing: Option<String> = ui.ctx().data(|d| d.get_temp(edit_id));
    let mut changed = false;
    let clamp = |x: f64| { let x = x.clamp(*range.start(), *range.end()); if integer { x.round() } else { x } };
    let anchor = match align { Align2::LEFT_CENTER => Pos2::new(rect.left() + 2., rect.center().y), Align2::RIGHT_CENTER => Pos2::new(rect.right() - 2., rect.center().y), _ => rect.center() };
    register_field(ui.ctx(), id, ui.is_enabled());
    let mut started = ui.ctx().data(|d| d.get_temp::<bool>(id.with("select_on_edit"))).unwrap_or(false);
    if started { ui.ctx().data_mut(|d| d.remove::<bool>(id.with("select_on_edit"))); }
    let editing = editing.or_else(|| {
        // Tab from the neighbouring field asked for this one to start editing
        if ui.ctx().data(|d| d.get_temp::<egui::Id>(TAB_REQUEST)) != Some(id) { return None; }
        ui.ctx().data_mut(|d| d.remove::<egui::Id>(TAB_REQUEST));
        started = true;
        Some(if let Some(u) = units { u.format(*v) } else if integer { format!("{}", *v as i64) } else { format!("{v}") })
    });
    if let Some(mut text) = editing {
        // Tab / Shift+Tab commit this field and start editing the next / previous one in layout order
        let tab = ui.input_mut(|i| if i.consume_key(egui::Modifiers::SHIFT, egui::Key::Tab) { Some(-1) }
            else if i.consume_key(egui::Modifiers::NONE, egui::Key::Tab) { Some(1) } else { None });
        let text_id = id.with("text");
        let horizontal_align = match align { Align2::RIGHT_CENTER => egui::Align::Max, Align2::LEFT_CENTER => egui::Align::Min, _ => egui::Align::Center };
        let response = ui.put(rect, egui::TextEdit::singleline(&mut text).font(egui::FontId::monospace(12.)).frame(false).margin(Vec2::new(2., 0.)).horizontal_align(horizontal_align).id(text_id));
        if !response.has_focus() && !response.lost_focus() { response.request_focus(); }
        if started {
            let mut state = egui::TextEdit::load_state(ui.ctx(), text_id).unwrap_or_default();
            state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(text.chars().count()))));
            state.store(ui.ctx(), text_id);
        }
        if let Some(dir) = tab {
            if let Some(next) = tab_target(&last_field_order(ui.ctx()), id, dir) { ui.ctx().data_mut(|d| d.insert_temp(TAB_REQUEST, next)); }
        }
        if response.lost_focus() || tab.is_some() {
            if let Some(u) = units {
                if let Some(x) = u.parse(&text) { *v = clamp(x); changed = true; }
            } else if let Ok(x) = text.trim().parse::<f64>() {
                *v = clamp(x); changed = true;
            }
            ui.ctx().data_mut(|d| d.remove::<String>(edit_id));
        } else { ui.ctx().data_mut(|d| d.insert_temp(edit_id, text)); }
    } else {
        let response = ui.interact(rect, id, Sense::click_and_drag());
        if response.hovered() || response.dragged() { ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal); }
        if response.dragged() {
            let speed = speed * number_step::drag_multiplier(ui.input(|i| i.modifiers), *v);
            let step = if let Some(u) = units {
                response.drag_delta().x as f64 * speed * u.display.metres() / u.unit_scale
            } else {
                response.drag_delta().x as f64 * speed
            };
            if integer {
                // accumulate sub-unit drags so slow integer drags still move
                let acc_id = id.with("drag_acc");
                let acc = ui.ctx().data(|d| d.get_temp::<f64>(acc_id)).unwrap_or(0.) + step;
                let whole = acc.trunc();
                ui.ctx().data_mut(|d| d.insert_temp(acc_id, acc - whole));
                if whole != 0. { let next = clamp(*v + whole); if next != *v { *v = next; changed = true; } }
            } else if step != 0. { let next = clamp(*v + step); if next != *v { *v = next; changed = true; } }
        } else if integer { ui.ctx().data_mut(|d| d.remove::<f64>(id.with("drag_acc"))); }
        if response.clicked() {
            let text = if let Some(u) = units {
                u.format(*v)
            } else if integer {
                format!("{}", *v as i64)
            } else {
                format!("{v}")
            };
            ui.ctx().data_mut(|d| d.insert_temp(edit_id, text));
            ui.ctx().data_mut(|d| d.insert_temp(id.with("select_on_edit"), true));
        }
        let text = if let Some(u) = units {
            u.format(*v)
        } else if integer {
            format!("{}", *v as i64)
        } else {
            format!("{:.2}", *v)
        };
        ui.painter().text(anchor, align, text, egui::FontId::monospace(12.), if ui.is_enabled() { t.text } else { t.faint });
    }
    changed
}

/// A boxed number field: drag/double-click text between a decrement button (left end) and an increment button
/// (right end), plus gated wheel stepping (see ui::number_step). Fields too narrow for the buttons keep drag + wheel.
#[allow(clippy::too_many_arguments)]
fn number_field(ui: &mut egui::Ui, width: f32, id: egui::Id, v: &mut f64, speed: f64, range: &std::ops::RangeInclusive<f64>, integer: bool, soft: Option<(f64, f64)>, align: Align2, t: &Theme, units: Option<crate::units::UnitSettings>) -> (bool, Rect) {
    let (outer, inner) = boxed(ui, width, t);
    let buttons = number_step::layout(outer);
    // the value reads centred between the - and + buttons
    let align = if buttons.is_some() { Align2::CENTER_CENTER } else { align };
    let text = buttons.map_or(inner, |(_, _, text)| Rect::from_min_max(Pos2::new(text.left() + 2., inner.top()), Pos2::new(text.right() - 2., inner.bottom())));
    let mut changed = drag_number(ui, text, id, v, speed, range, integer, align, t, units);
    let steps = buttons.map_or(0, |(dec, inc, _)| number_step::stepper(ui, dec, inc, id, t)) + step_by_wheel(ui, outer, id);
    changed |= apply_field_steps(ui, id, v, steps, range, integer, soft, units);
    (changed, outer)
}
fn step_by_wheel(ui: &egui::Ui, rect: Rect, id: egui::Id) -> i32 {
    let editing = ui.ctx().data(|d| d.get_temp::<String>(id.with("edit")).is_some());
    number_step::wheel(ui, rect, id, editing)
}
#[allow(clippy::too_many_arguments)]
fn apply_field_steps(ui: &egui::Ui, id: egui::Id, v: &mut f64, steps: i32, range: &std::ops::RangeInclusive<f64>, integer: bool, soft: Option<(f64, f64)>, units: Option<crate::units::UnitSettings>) -> bool {
    if steps == 0 { return false; }
    let mods = ui.input(|i| i.modifiers);
    let step = number_step::step_size(integer, soft, *v) * number_step::modifier_scale(mods, integer);
    let next = number_step::apply_step(*v, step, steps as f64, range, integer);
    if next == *v { return false; }
    *v = next;
    let edit_id = id.with("edit");
    if ui.ctx().data(|d| d.get_temp::<String>(edit_id).is_some()) {
        // keep the text being typed in step with the wheel
        let text = if let Some(u) = units { u.format(next) } else if integer { format!("{}", next as i64) } else { format!("{next}") };
        ui.ctx().data_mut(|d| d.insert_temp(edit_id, text));
    }
    true
}

fn color_widget(
    ui: &mut egui::Ui,
    value: &mut Value,
    _meta: Option<&Value>,
    theme: Option<&Theme>,
    width: f32,
    _id: &str,
    _param_name: &str,
) -> bool {
    let Some((mut rgba, comps, is_hex)) = parse_color_value(value) else {
        return false;
    };
    let mut changed = false;
    let swatch_w = 26.0_f32;
    let gap = 4.0_f32;
    let remaining_w = (width - swatch_w - gap).max(20.0);

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        // 1. Color swatch button with picker popup
        if comps == 4 {
            let mut arr = rgba;
            let resp = ui.color_edit_button_rgba_unmultiplied(&mut arr);
            if resp.changed() {
                rgba = arr;
                changed = true;
            }
        } else {
            let mut rgb = [rgba[0].clamp(0., 1.), rgba[1].clamp(0., 1.), rgba[2].clamp(0., 1.)];
            let resp = ui.color_edit_button_rgb(&mut rgb);
            if resp.changed() {
                rgba[0] = rgb[0];
                rgba[1] = rgb[1];
                rgba[2] = rgb[2];
                changed = true;
            }
        }

        // 2. Numeric / Hex fields beside swatch
        if is_hex {
            let mut hex_str = if let Some(s) = value.as_str() { s.to_owned() } else { serialize_color_value(rgba, comps, true).as_str().unwrap_or("").to_owned() };
            let resp = ui.add_sized([remaining_w, 22.], egui::TextEdit::singleline(&mut hex_str));
            if resp.changed() {
                if let Some((parsed, _, _)) = parse_color_value(&Value::String(hex_str.clone())) {
                    rgba = parsed;
                    changed = true;
                }
                *value = json!(hex_str);
                return;
            }
        } else {
            let n = comps;
            let axis_spacing = 3.0_f32;
            let comp_w = ((remaining_w - axis_spacing * (n as f32 - 1.0)) / n as f32).max(18.0);
            let labels = ["R", "G", "B", "A"];
            for i in 0..n {
                let comp_color = if let Some(t) = theme {
                    match i {
                        0 => t.x_axis,
                        1 => t.y_axis,
                        2 => t.z_axis,
                        _ => t.dim,
                    }
                } else {
                    Color32::GRAY
                };
                if let Some(t) = theme {
                    let (outer, inner) = boxed(ui, comp_w, t);
                    ui.painter().text(
                        Pos2::new(outer.left() + 4., outer.center().y),
                        Align2::LEFT_CENTER,
                        labels[i],
                        FontId::monospace(10.),
                        comp_color,
                    );
                    let mut val_f64 = rgba[i] as f64;
                    let speed = 0.01;
                    let range = 0.0..=1.0;
                    if drag_number(
                        ui,
                        Rect::from_min_max(Pos2::new(inner.left() + 10., inner.top()), inner.max),
                        ui.id().with(("col_c", i, outer.min.x as i32, outer.min.y as i32)),
                        &mut val_f64,
                        speed,
                        &range,
                        false,
                        Align2::LEFT_CENTER,
                        t,
                        None,
                    ) {
                        rgba[i] = val_f64 as f32;
                        changed = true;
                    }
                } else {
                    let mut val = rgba[i];
                    if ui.add_sized([comp_w, 22.], egui::DragValue::new(&mut val).speed(0.01).range(0.0..=1.0).prefix(format!("{}: ", labels[i]))).changed() {
                        rgba[i] = val;
                        changed = true;
                    }
                }
            }
        }
    });

    if changed {
        *value = serialize_color_value(rgba, comps, is_hex);
    }
    changed
}

fn file_widget(
    ui: &mut egui::Ui,
    value: &mut Value,
    meta: Option<&Value>,
    typ: &str,
    param_name: &str,
    width: f32,
) -> bool {
    let mut text = value.as_str().unwrap_or("").to_owned();
    let btn_w = 26.0_f32;
    let gap = 4.0_f32;
    let text_w = (width - btn_w - gap).max(30.0);
    let mut changed = false;

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        let text_resp = ui.add_sized([text_w, 22.], egui::TextEdit::singleline(&mut text).desired_width(f32::INFINITY));
        if text_resp.changed() {
            *value = json!(text);
            changed = true;
        }

        let browse_resp = ui.add_sized([btn_w, 22.], egui::Button::new("...")).on_hover_text("Browse files");
        if browse_resp.clicked() {
            let is_save = typ.contains("export")
                || typ.contains("out")
                || meta.and_then(|m| m["ui"].as_str()).is_some_and(|ui| ui.starts_with("file:save"));
            let mut dialog = rfd::FileDialog::new();
            let path_ref = std::path::Path::new(&text);
            if let Some(parent) = path_ref.parent().filter(|p| p.is_dir()) {
                dialog = dialog.set_directory(parent);
            }
            if let Some(file_name) = path_ref.file_name().and_then(|f| f.to_str()) {
                if !file_name.is_empty() {
                    dialog = dialog.set_file_name(file_name);
                }
            }
            if typ.contains("usd") {
                dialog = dialog.add_filter("USD files", &["usd", "usda", "usdc", "usdz"]);
            } else if typ.contains("image") || typ.contains("blender_image") {
                dialog = dialog.add_filter("Image files", &["png", "jpg", "jpeg", "tga", "bmp", "tif", "tiff"]);
            } else if typ.contains("gltf") {
                dialog = dialog.add_filter("glTF files", &["gltf", "glb"]);
            } else if param_name == "rule_file" {
                dialog = dialog.add_filter("Rules", &["txt"]);
            }
            dialog = dialog.add_filter("All files", &["*"]);
            let picked = if is_save { dialog.save_file() } else { dialog.pick_file() };
            if let Some(path) = picked {
                let p_str = path.to_string_lossy().to_string();
                *value = json!(p_str);
                changed = true;
            }
        }
    });

    changed
}

fn ramp_widget(
    ui: &mut egui::Ui,
    value: &mut Value,
    meta: Option<&Value>,
    theme: &Theme,
    width: f32,
    id: &str,
    param_name: &str,
) -> bool {
    let is_color_hint = meta.and_then(|m| m["kind"].as_str()) == Some("color_ramp")
        || param_name.contains("color");
    let mut ramp = RampData::from_value(value, is_color_hint);
    let mut changed = false;

    let total_w = width.max(100.0);
    let bar_h = 24.0_f32;
    let markers_h = 16.0_f32;
    let controls_h = 24.0_f32;

    let ramp_id = ui.id().with((id, param_name, "ramp"));
    let sel_id = ramp_id.with("selected_stop");
    let mut selected_stop: usize = ui.ctx().data(|d| d.get_temp(sel_id)).unwrap_or(0);
    if selected_stop >= ramp.stops.len() {
        selected_stop = ramp.stops.len().saturating_sub(1);
    }

    let (outer_rect, _) = ui.allocate_exact_size(Vec2::new(total_w, bar_h + markers_h + controls_h), Sense::hover());
    let bar_rect = Rect::from_min_size(outer_rect.min, Vec2::new(total_w, bar_h));
    let markers_rect = Rect::from_min_size(Pos2::new(outer_rect.left(), bar_rect.bottom()), Vec2::new(total_w, markers_h));
    let controls_rect = Rect::from_min_max(Pos2::new(outer_rect.left(), markers_rect.bottom() + 4.0), outer_rect.max);

    // 1. Paint preview bar
    if ramp.is_color {
        let n_steps = 50.min((total_w / 3.0) as usize).max(10);
        let step_w = total_w / n_steps as f32;
        for s in 0..n_steps {
            let t = (s as f32 + 0.5) / n_steps as f32;
            let c = ramp.eval_color(t);
            let col = Color32::from_rgba_premultiplied(
                (c[0].clamp(0., 1.) * 255.0) as u8,
                (c[1].clamp(0., 1.) * 255.0) as u8,
                (c[2].clamp(0., 1.) * 255.0) as u8,
                (c[3].clamp(0., 1.) * 255.0) as u8,
            );
            let sub_rect = Rect::from_min_size(
                Pos2::new(bar_rect.left() + s as f32 * step_w, bar_rect.top()),
                Vec2::new(step_w + 0.5, bar_h),
            );
            ui.painter().rect_filled(sub_rect, 0.0, col);
        }
    } else {
        ui.painter().rect_filled(bar_rect, 2.0, theme.input);
        let n_steps = 50.min((total_w / 3.0) as usize).max(10);
        let mut pts = Vec::with_capacity(n_steps + 1);
        for s in 0..=n_steps {
            let t = s as f32 / n_steps as f32;
            let val = ramp.eval_float(t).clamp(0.0, 1.0);
            let x = bar_rect.left() + t * bar_rect.width();
            let y = bar_rect.bottom() - 2.0 - val * (bar_h - 4.0);
            pts.push(Pos2::new(x, y));
        }
        ui.painter().add(egui::Shape::line(pts, Stroke::new(1.5_f32, theme.accent)));
    }
    ui.painter().rect_stroke(bar_rect, 3.0, Stroke::new(1.0_f32, theme.stroke), StrokeKind::Inside);

    // 2. Stop Markers & Interaction
    let interact_rect = bar_rect.union(markers_rect);
    let resp = ui.interact(interact_rect, ramp_id, Sense::click_and_drag());

    // Paint markers
    for (i, stop) in ramp.stops.iter().enumerate() {
        let sx = bar_rect.left() + stop.pos * bar_rect.width();
        let top_y = bar_rect.bottom() + 1.0;
        let bot_y = markers_rect.bottom() - 1.0;
        let is_sel = i == selected_stop;

        let fill = if ramp.is_color {
            let c = ramp.eval_color(stop.pos);
            Color32::from_rgb((c[0].clamp(0., 1.) * 255.) as u8, (c[1].clamp(0., 1.) * 255.) as u8, (c[2].clamp(0., 1.) * 255.) as u8)
        } else {
            let f = ramp.eval_float(stop.pos).clamp(0., 1.);
            Color32::from_gray((f * 255.) as u8)
        };
        let shape = vec![
            Pos2::new(sx, top_y),
            Pos2::new(sx + 4.0, top_y + 5.0),
            Pos2::new(sx + 4.0, bot_y),
            Pos2::new(sx - 4.0, bot_y),
            Pos2::new(sx - 4.0, top_y + 5.0),
        ];
        ui.painter().add(egui::Shape::convex_polygon(
            shape,
            fill,
            Stroke::new(if is_sel { 2.0_f32 } else { 1.0_f32 }, if is_sel { theme.accent } else { theme.stroke2 }),
        ));
    }

    // Handle clicks / drags
    if resp.clicked() || resp.drag_started() {
        if let Some(pos) = resp.interact_pointer_pos() {
            let mut clicked_marker = None;
            for (i, stop) in ramp.stops.iter().enumerate() {
                let sx = bar_rect.left() + stop.pos * bar_rect.width();
                if (pos.x - sx).abs() <= 8.0 && pos.y >= bar_rect.top() && pos.y <= markers_rect.bottom() {
                    clicked_marker = Some(i);
                    break;
                }
            }

            if ui.input(|i| i.pointer.button_clicked(egui::PointerButton::Secondary)) {
                // Right-click delete
                if let Some(idx) = clicked_marker {
                    if ramp.delete_stop(idx) {
                        selected_stop = selected_stop.saturating_sub(1);
                        changed = true;
                    }
                }
            } else if let Some(idx) = clicked_marker {
                selected_stop = idx;
            } else if pos.y <= bar_rect.bottom() + 4.0 {
                // Clicked on preview bar to add
                let new_pos = ((pos.x - bar_rect.left()) / bar_rect.width()).clamp(0.0, 1.0);
                selected_stop = ramp.add_stop(new_pos);
                changed = true;
            }
        }
    }

    if resp.dragged_by(egui::PointerButton::Primary) {
        if let Some(pos) = resp.interact_pointer_pos() {
            let new_pos = ((pos.x - bar_rect.left()) / bar_rect.width()).clamp(0.0, 1.0);
            selected_stop = ramp.move_stop(selected_stop, new_pos);
            changed = true;
        }
    }

    // Delete key to remove selected stop
    if ui.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace)) && ramp.stops.len() > 1 {
        if ramp.delete_stop(selected_stop) {
            selected_stop = selected_stop.saturating_sub(1);
            changed = true;
        }
    }

    // 3. Selected Stop Controls
    if selected_stop < ramp.stops.len() {
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(controls_rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        child.spacing_mut().item_spacing.x = 4.0;

        child.label(format!("#{}", selected_stop + 1));

        let mut stop_pos = ramp.stops[selected_stop].pos;
        if child.add(egui::DragValue::new(&mut stop_pos).speed(0.01).range(0.0..=1.0).prefix("Pos: ")).changed() {
            selected_stop = ramp.move_stop(selected_stop, stop_pos);
            changed = true;
        }

        if ramp.is_color {
            let mut rgb = match &ramp.stops[selected_stop].value {
                RampValue::Color(c) => *c,
                RampValue::ColorAlpha(c) => [c[0], c[1], c[2]],
                RampValue::Float(f) => [*f, *f, *f],
            };
            if child.color_edit_button_rgb(&mut rgb).changed() {
                ramp.stops[selected_stop].value = RampValue::Color(rgb);
                changed = true;
            }
        } else {
            let mut val = ramp.stops[selected_stop].value.as_float();
            if child.add(egui::DragValue::new(&mut val).speed(0.01).range(0.0..=1.0).prefix("Val: ")).changed() {
                ramp.stops[selected_stop].value = RampValue::Float(val);
                changed = true;
            }
        }

        // Interpolation ComboBox
        let current_interp = ramp.interpolation;
        egui::ComboBox::from_id_salt(ramp_id.with("interp_combo"))
            .selected_text(current_interp.as_str())
            .width(65.0)
            .show_ui(&mut child, |ui| {
                for opt in [RampInterpolation::Linear, RampInterpolation::Constant, RampInterpolation::Smooth] {
                    if ui.selectable_label(ramp.interpolation == opt, opt.as_str()).clicked() {
                        ramp.interpolation = opt;
                        changed = true;
                    }
                }
            });

        // Delete button
        if child.add_enabled(ramp.stops.len() > 1, egui::Button::new("×")).on_hover_text("Delete stop").clicked() {
            if ramp.delete_stop(selected_stop) {
                selected_stop = selected_stop.saturating_sub(1);
                changed = true;
            }
        }
    }

    ui.ctx().data_mut(|d| d.insert_temp(sel_id, selected_stop));

    if changed {
        *value = ramp.to_value();
    }
    changed
}

/// One editing widget for a parameter value; returns true when the value changed.
pub fn widget(ui: &mut egui::Ui, kind: ParamKind, meta: Option<&Value>, value: &mut Value) -> bool {
    let width = ui.available_width();
    widget_styled(ui, kind, meta, value, None, width, None, None, None)
}

pub fn widget_with_units(ui: &mut egui::Ui, kind: ParamKind, meta: Option<&Value>, value: &mut Value, units: Option<crate::units::UnitSettings>) -> bool {
    let width = ui.available_width();
    widget_styled(ui, kind, meta, value, None, width, units, None, None)
}

fn render_vector_fields(
    ui: &mut egui::Ui,
    value: &mut Value,
    n: usize,
    meta: Option<&Value>,
    theme: Option<&Theme>,
    width: f32,
    units: Option<crate::units::UnitSettings>,
    mut channel_menu: Option<&mut dyn FnMut(&mut egui::Ui, usize)>,
) -> bool {
    let axis_labels = ["X", "Y", "Z", "W"];
    let mut v: Vec<f64> = (0..n).map(|i| value.get(i).and_then(Value::as_f64).unwrap_or(0.)).collect();
    let mut changed = false;
    let spacing = 4.0_f32;
    let total_spacing = spacing * (n as f32 - 1.0);
    let axis_width = ((width - total_spacing) / n as f32).max(22.0);
    let range = meta.and_then(|m| m["min"].as_f64()).unwrap_or(f64::NEG_INFINITY)..=meta.and_then(|m| m["max"].as_f64()).unwrap_or(f64::INFINITY);
    let soft = meta.and_then(slider_range).map(|r| (*r.start(), *r.end()));

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = spacing;
        for (i, x) in v.iter_mut().enumerate() {
            if let Some(t) = theme {
                let axis_color = match i {
                    0 => t.x_axis,
                    1 => t.y_axis,
                    2 => t.z_axis,
                    _ => t.dim,
                };
                let (outer, inner) = boxed(ui, axis_width, t);
                let label = if i < axis_labels.len() { axis_labels[i] } else { "?" };
                ui.painter().text(
                    Pos2::new(outer.left() + 6., outer.center().y),
                    Align2::LEFT_CENTER,
                    label,
                    egui::FontId::monospace(11.),
                    axis_color,
                );
                if let Some(menu) = channel_menu.as_deref_mut() {
                    ui.interact(Rect::from_min_max(outer.min, Pos2::new(inner.left() + 12., outer.bottom())),
                        ui.id().with(("v_label", i, outer.min.x as i32, outer.min.y as i32)), Sense::click())
                        .context_menu(|ui| menu(ui, i));
                }
                let id = ui.id().with(("v", i, outer.min.x as i32, outer.min.y as i32));
                changed |= drag_number(
                    ui,
                    Rect::from_min_max(Pos2::new(inner.left() + 12., inner.top()), inner.max),
                    id,
                    x,
                    0.01,
                    &range,
                    false,
                    Align2::LEFT_CENTER,
                    t,
                    units,
                );
                if let Some(menu) = channel_menu.as_deref_mut() {
                    if let Some(response) = ui.ctx().read_response(id).or_else(|| ui.ctx().read_response(id.with("text"))) {
                        response.context_menu(|ui| menu(ui, i));
                    }
                }
                let steps = step_by_wheel(ui, outer, id);
                changed |= apply_field_steps(ui, id, x, steps, &range, false, soft, units);
            } else if let Some(u) = units {
                changed |= ui.add_sized([axis_width, 22.], egui::DragValue::new(x).speed(0.01).range(range.clone()).suffix(format!(" {}", u.display.symbol()))).changed();
            } else {
                changed |= ui.add_sized([axis_width, 22.], egui::DragValue::new(x).speed(0.01).range(range.clone())).changed();
            }
        }
    });
    if changed {
        *value = json!(v);
    }
    changed
}

fn widget_styled(ui: &mut egui::Ui, kind: ParamKind, meta: Option<&Value>, value: &mut Value, theme: Option<&Theme>, width: f32, units: Option<crate::units::UnitSettings>, bool_label: Option<&str>, channel_menu: Option<&mut dyn FnMut(&mut egui::Ui, usize)>) -> bool {
    let param_name = meta.and_then(|m| m["name"].as_str()).unwrap_or("");
    if is_ramp_param(param_name, meta, value) {
        let default_theme = Theme::dark();
        let t = theme.unwrap_or(&default_theme);
        return ramp_widget(ui, value, meta, t, width, "widget", "param");
    }
    if is_color_param(param_name, meta, value) {
        return color_widget(ui, value, meta, theme, width, "widget", "param");
    }
    if is_file_param(param_name, meta) {
        return file_widget(ui, value, meta, "", param_name, width);
    }
    if let Some(meta) = meta {
        if meta["kind"] == "enum" {
            let before = value.clone();
            let combo_w = width.min(180.0).max(50.0);
            // A domain dropdown shows "primitives" as "faces"; the stored value stays "primitives" (saved graphs use it).
            let is_domain = meta["options"].as_array().is_some_and(|o| o.iter().any(|v| v == "points") && o.iter().any(|v| v == "edges"));
            let label = |v: &'_ str| -> String { if is_domain && v == "primitives" { "faces".into() } else { v.to_owned() } };
            egui::ComboBox::from_id_salt("enum").width(combo_w).selected_text(label(value.as_str().unwrap_or("")))
                .show_ui(ui, |ui| {
                    if let Some(options) = meta["options"].as_array() {
                        for option in options { let text = label(option.as_str().unwrap_or("")); ui.selectable_value(value, option.clone(), text); }
                    }
                });
            return *value != before;
        }
        if matches!(kind, ParamKind::Integer | ParamKind::Float) {
            let mut v = value.as_f64().unwrap_or(0.);
            let integer = kind == ParamKind::Integer;
            let mut changed = false;
            let s_range = slider_range(meta).or_else(|| {
                if meta.get("doc").and_then(Value::as_str) == Some("Multiplies scale") {
                    Some(0.0..=10.0)
                } else {
                    None
                }
            });
            let box_w: f32 = 2. * number_step::STEPPER_W + if units.is_some() { 84. } else { 68. };
            let drag_width = if integer {
                if theme.is_some() { width - 32. } else { width }
            } else if s_range.is_some() {
                box_w.min((width - 24.).max(36.))
            } else {
                width
            };
            let range = meta["min"].as_f64().or_else(|| if s_range.is_some() { Some(0.0) } else { None }).unwrap_or(f64::NEG_INFINITY)..=meta["max"].as_f64().unwrap_or(f64::INFINITY);
            let soft = s_range.as_ref().map(|r| (*r.start(), *r.end()));
            if let Some(t) = theme {
                let drag_speed = if integer { 0.1 } else { s_range.as_ref().map_or(0.01, |r| ((r.end() - r.start()) / 400.).clamp(0.001, 1.)) };
                let at = ui.next_widget_position();
                let id = ui.id().with(("num", meta["default"].to_string(), at.x as i32, at.y as i32));
                changed |= number_field(ui, drag_width.max(36.), id, &mut v, drag_speed, &range, integer, soft, Align2::RIGHT_CENTER, t, units).0;
            } else if let Some(u) = units {
                changed |= ui.add_sized([drag_width.max(24.), 22.], egui::DragValue::new(&mut v).speed(0.01).range(range.clone()).suffix(format!(" {}", u.display.symbol()))).changed();
            } else {
                changed |= ui.add_sized([drag_width.max(24.), 22.], egui::DragValue::new(&mut v).speed(if integer { 1. } else { 0.01 }).range(range.clone())).changed();
            }
            if !integer {
                if let Some(range) = s_range.clone() {
                    let (min, max) = (*range.start(), *range.end());
                    let slider_w = (width - drag_width - ui.spacing().item_spacing.x).max(8.);
                    ui.spacing_mut().slider_width = slider_w;
                    let response = ui.scope(|ui| {
                        let w = &mut ui.visuals_mut().widgets;
                        for s in [&mut w.inactive, &mut w.hovered, &mut w.active, &mut w.noninteractive] {
                            s.bg_fill = Color32::TRANSPARENT;
                            s.weak_bg_fill = Color32::TRANSPARENT;
                            s.bg_stroke = Stroke::NONE;
                            s.fg_stroke = Stroke::NONE;
                        }
                        ui.visuals_mut().slider_trailing_fill = false;
                        ui.add(egui::Slider::new(&mut v, range).show_value(false).clamping(egui::SliderClamping::Edits))
                    }).inner;
                    changed |= response.changed();
                    if let Some(t) = theme {
                        let track = Rect::from_center_size(response.rect.center(), Vec2::new((response.rect.width() - 12.).max(4.), 6.));
                        let active = response.dragged();
                        let faded = !ui.is_enabled();
                        let color = |c: Color32| if faded { c.gamma_multiply(0.5) } else { c };
                        ui.painter().rect(track, 3., color(t.input), Stroke::new(1.0_f32,
                            color(if response.hovered() { t.stroke2 } else { t.stroke })), StrokeKind::Inside);
                        let fraction = if max > min { ((v - min) / (max - min)).clamp(0., 1.) as f32 } else { 0. };
                        let knob_x = egui::lerp(track.left()..=track.right(), fraction);
                        let fill = Rect::from_min_max(track.min, Pos2::new(knob_x, track.bottom()));
                        ui.painter().rect_filled(fill, 3., color(t.accent));
                        if active { ui.painter().circle_filled(Pos2::new(knob_x, track.center().y), 9., color(t.accent_bg)); }
                        ui.painter().circle(Pos2::new(knob_x, track.center().y), 6., color(t.text),
                            Stroke::new(1.0_f32, color(if response.hovered() || active { t.accent } else { t.stroke2 })));
                    }
                }
            }
            if changed {
                v = clamp_parameter(meta, v);
                *value = if integer { json!(v.round() as i64) } else { json!(v) };
            }
            return changed;
        }
    }
    match kind {
        ParamKind::Bool => {
            let mut v = value.as_bool().unwrap_or(false);
            if ui.checkbox(&mut v, bool_label.unwrap_or("")).changed() { *value = json!(v); true } else { false }
        }
        ParamKind::Integer => {
            let mut v = value.as_i64().or_else(|| value.as_f64().map(|f| f as i64)).unwrap_or(0);
            let changed = if let Some(t) = theme {
                let range = meta.and_then(|m| m["min"].as_f64()).unwrap_or(f64::NEG_INFINITY)..=meta.and_then(|m| m["max"].as_f64()).unwrap_or(f64::INFINITY);
                let mut x = v as f64;
                let at = ui.next_widget_position();
                let moved = number_field(ui, width.max(40.), ui.id().with(("int", at.x as i32, at.y as i32)), &mut x, 0.1, &range, true, None, Align2::RIGHT_CENTER, t, None).0;
                if moved { v = x as i64; }
                moved
            } else {
                ui.add_sized([width.max(24.), 22.], egui::DragValue::new(&mut v).speed(1)).changed()
            };
            if changed { *value = json!(v); }
            changed
        }
        ParamKind::Float => {
            let mut v = value.as_f64().unwrap_or(0.);
            let changed = if let Some(t) = theme {
                let at = ui.next_widget_position();
                number_field(ui, width.max(36.), ui.id().with(("f_num", at.x as i32, at.y as i32)), &mut v, 0.01, &(f64::NEG_INFINITY..=f64::INFINITY), false, None, Align2::RIGHT_CENTER, t, units).0
            } else if let Some(u) = units {
                ui.add_sized([width, 22.], egui::DragValue::new(&mut v).speed(0.01).suffix(format!(" {}", u.display.symbol()))).changed()
            } else {
                ui.add_sized([width, 22.], egui::DragValue::new(&mut v).speed(0.01)).changed()
            };
            if changed { *value = json!(v); true } else { false }
        }
        ParamKind::String => {
            let mut v = value.as_str().unwrap_or("").to_owned();
            if ui.add_sized([width, 22.], egui::TextEdit::singleline(&mut v).desired_width(f32::INFINITY)).changed() { *value = json!(v); true } else { false }
        }
        ParamKind::Vec3 => {
            let n = value.as_array().map_or(3, |a| a.len().clamp(2, 4));
            render_vector_fields(ui, value, n, meta, theme, width, units, channel_menu)
        }
        ParamKind::Unsupported => {
            if let Some(arr) = value.as_array().filter(|a| (2..=4).contains(&a.len()) && a.iter().all(Value::is_number)) {
                render_vector_fields(ui, value, arr.len(), meta, theme, width, units, channel_menu)
            } else {
                ui.label(value.to_string());
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::testing::Fixture;

    #[test] fn channel_link_changes_only_the_selected_axis() {
        let mut doc = minihou_app::Document::parse(json!({"nodes":{"a":{"type":"box"},"b":{"type":"box","channels":{"size.y":"ch('../a/size.y')"}}}}).to_string()).unwrap();
        doc.edit(&channel_reference_edit("b", "size.x", "a", "size.x")).unwrap();
        assert_eq!(doc.value()["nodes"]["b"]["channels"]["size.x"], "ch(\"../a/size.x\")");
        assert_eq!(doc.value()["nodes"]["b"]["channels"]["size.y"], "ch('../a/size.y')");
        assert!(doc.value()["nodes"]["b"]["channels"].get("size.z").is_none());
    }

    #[test] fn edit_expression_prefills_the_existing_channel_relationship() {
        let ctx = egui::Context::default();
        let doc = json!({"nodes":{"b":{"channels":{"size.x":"ch('../a/size.x')"}}}});
        open_expression(&ctx, &doc, "b", "size.x");
        assert_eq!(ctx.data(|d| d.get_temp::<String>(egui::Id::new("link_expression_text"))).as_deref(), Some("ch('../a/size.x')"));
        assert_eq!(ctx.data(|d| d.get_temp::<(String,String)>(egui::Id::new("link_expression_open"))), Some(("b".into(), "size.x".into())));
    }

    #[test] fn params_tab_moves_editing_to_the_next_and_previous_number_field() {
        let order=[egui::Id::new("a"),egui::Id::new("b"),egui::Id::new("c")];
        assert_eq!(tab_target(&order,order[0],1),Some(order[1]));
        assert_eq!(tab_target(&order,order[1],-1),Some(order[0]));
        assert_eq!(tab_target(&order,order[2],1),None);
        assert_eq!(tab_target(&order,order[0],-1),None);
        let ctx=egui::Context::default(); let theme=Theme::dark();
        let (a,b)=(egui::Id::new("fa"),egui::Id::new("fb"));
        let (mut va,mut vb)=(1.0f64,2.0f64);
        let frame=|ctx:&egui::Context,events:Vec<egui::Event>,va:&mut f64,vb:&mut f64| {
            let _=ctx.run(egui::RawInput{events,..Default::default()},|ctx| { egui::CentralPanel::default().show(ctx,|ui| {
                let r=1.0..=100.0; let range=std::ops::RangeInclusive::new(*r.start(),*r.end());
                let ra=ui.allocate_space(egui::vec2(80.,20.)).1; drag_number(ui,ra,a,va,0.1,&range,false,Align2::LEFT_CENTER,&theme,None);
                let rb=ui.allocate_space(egui::vec2(80.,20.)).1; drag_number(ui,rb,b,vb,0.1,&range,false,Align2::LEFT_CENTER,&theme,None);
            });});
        };
        frame(&ctx,vec![],&mut va,&mut vb); frame(&ctx,vec![],&mut va,&mut vb);
        ctx.data_mut(|d| d.insert_temp(a.with("edit"),"7".to_string()));
        let tab=|shift:bool| vec![egui::Event::Key{key:egui::Key::Tab,physical_key:None,pressed:true,repeat:false,modifiers:if shift{egui::Modifiers::SHIFT}else{egui::Modifiers::NONE}}];
        frame(&ctx,tab(false),&mut va,&mut vb); frame(&ctx,vec![],&mut va,&mut vb);
        assert_eq!(va,7.0); // committed on Tab
        assert!(ctx.data(|d| d.get_temp::<String>(a.with("edit"))).is_none());
        assert!(ctx.data(|d| d.get_temp::<String>(b.with("edit"))).is_some()); // next field is now editing
        frame(&ctx,tab(true),&mut va,&mut vb); frame(&ctx,vec![],&mut va,&mut vb);
        assert!(ctx.data(|d| d.get_temp::<String>(a.with("edit"))).is_some()); // Shift+Tab went back
    }
    #[test] fn linked_row_context_menu_opens_in_egui() {
        let mut f=Fixture::new();
        f.doc=minihou_app::Document::parse(json!({"display":"b","nodes":{"a":{"type":"box"},"b":{"type":"box","channels":{"size.x":"ch('../a/size.x')"}}}}).to_string()).unwrap();
        f.graph=minihou_core::Graph::from_value(f.doc.value()).unwrap(); f.selected="b".into(); f.display="b".into();
        let ctx=egui::Context::default(); f.theme.apply(&ctx);
        let mut label=Pos2::ZERO; let mut found=false; let mut tinted=false;
        for phase in 0..4 {
            let events=match phase {
                1=>vec![egui::Event::PointerMoved(label),egui::Event::PointerButton{pos:label,button:egui::PointerButton::Secondary,pressed:true,modifiers:egui::Modifiers::NONE}],
                2=>vec![egui::Event::PointerButton{pos:label,button:egui::PointerButton::Secondary,pressed:false,modifiers:egui::Modifiers::NONE}],
                _=>vec![],
            };
            let input=egui::RawInput{screen_rect:Some(Rect::from_min_size(Pos2::ZERO,Vec2::new(800.,600.))),events,..Default::default()};
            let v=f.view(); let mut out=Actions::default();
            let output=ctx.run(input,|ctx|{egui::CentralPanel::default().show(ctx,|ui|{
                let param=minihou_core::nodes::get("box").unwrap().params.iter().find(|p|p.name=="size").unwrap();
                row(ui,&v,&mut out,"b","box",param,false,&mut vec![]);
                if let Some(r)=ui.ctx().read_response(ui.id().with(("b","size","label"))) {label=r.rect.center();}
            });});
            found|=output.shapes.iter().any(|shape|matches!(&shape.shape,egui::Shape::Text(t) if t.galley.text().contains("Copy Parameter")));
            tinted|=output.shapes.iter().any(|shape|matches!(&shape.shape,egui::Shape::Rect(rect) if rect.fill == Color32::from_rgba_unmultiplied(48,164,102,42)));
            assert!(out.0.is_empty());
        }
        assert!(found,"right click should reveal parameter actions");
        assert!(tinted,"linked field should carry the green tint");
    }

    #[test] fn vector_channel_right_click_opens_channel_link_menu() {
        let mut f = Fixture::new();
        f.doc = minihou_app::Document::parse(json!({"display":"b","nodes":{"a":{"type":"box"},"b":{"type":"box"}}}).to_string()).unwrap();
        f.graph = minihou_core::Graph::from_value(f.doc.value()).unwrap(); f.selected = "b".into(); f.display = "b".into();
        let ctx = egui::Context::default(); f.theme.apply(&ctx);
        let mut at = Pos2::ZERO;
        let mut found = false;
        for phase in 0..4 {
            let events = match phase {
                1 => vec![egui::Event::PointerMoved(at), egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed: true, modifiers: egui::Modifiers::NONE }],
                2 => vec![egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed: false, modifiers: egui::Modifiers::NONE }],
                _ => vec![],
            };
            let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.,600.))), events, ..Default::default() };
            let v = f.view(); let mut out = Actions::default();
            let output = ctx.run(input, |ctx| { egui::CentralPanel::default().show(ctx, |ui| {
                let param = minihou_core::nodes::get("box").unwrap().params.iter().find(|p| p.name == "size").unwrap();
                row(ui, &v, &mut out, "b", "box", param, false, &mut vec![]);
            }); });
            if phase == 0 {
                at = output.shapes.iter().find_map(|shape| match &shape.shape {
                    egui::Shape::Text(t) if t.galley.text() == "X" => Some(t.pos + Vec2::new(4., 5.)),
                    _ => None,
                }).expect("X channel label");
            }
            found |= output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(t) if t.galley.text().contains("Edit Expression")));
            assert!(out.0.is_empty());
        }
        assert!(found, "right-clicking X should open its channel menu");
    }

    #[test]
    fn bezier_inspector_uses_control_model_with_default_param() {
        let mut f=Fixture::new();
        f.doc=minihou_app::Document::parse(json!({"display":"bezier1","nodes":{"bezier1":{"type":"bezier"}}}).to_string()).unwrap();
        f.graph=minihou_core::Graph::from_value(f.doc.value()).unwrap();
        f.selected="bezier1".into();f.display="bezier1".into();
        let actions=f.run(|ui,v,out| bezier_controls_panel(ui,v,out,"bezier1",false));
        assert!(actions.is_empty());
        let controls=super::super::curve_tool::node_controls(f.doc.value(),"bezier1");
        assert_eq!(super::super::curve_tool::parse(&controls).unwrap().len(),3);
    }

    #[test]
    fn clicking_expose_emits_saved_document_edit_for_the_link_interface() {
        let mut f=Fixture::new();
        f.doc=minihou_app::Document::parse(json!({"display":"box1","nodes":{"box1":{"type":"box"}}}).to_string()).unwrap();
        f.graph=minihou_core::Graph::from_value(f.doc.value()).unwrap();
        f.selected="box1".into();f.display="box1".into();
        let ctx=egui::Context::default();f.theme.apply(&ctx);
        let mut click=None;let mut actions=Actions::default();
        for phase in 0..3 {
            let events=match (phase,click) {
                (1,Some(pos))=>vec![egui::Event::PointerMoved(pos),egui::Event::PointerButton{pos,button:egui::PointerButton::Primary,pressed:true,modifiers:egui::Modifiers::NONE}],
                (2,Some(pos))=>vec![egui::Event::PointerButton{pos,button:egui::PointerButton::Primary,pressed:false,modifiers:egui::Modifiers::NONE}],
                _=>vec![],
            };
            let input=egui::RawInput{screen_rect:Some(Rect::from_min_size(Pos2::ZERO,Vec2::new(800.,600.))),events,..Default::default()};
            let v=f.view();
            let _=ctx.run(input,|ctx|{egui::CentralPanel::default().show(ctx,|ui|{
                let param=minihou_core::nodes::get("box").unwrap().params.iter().find(|p|p.name=="size").unwrap();
                row(ui,&v,&mut actions,"box1","box",param,false,&mut vec![]);
                click=ui.ctx().read_response(ui.id().with(("box1","size","expose"))).map(|r|r.rect.center());
            });});
        }
        let edit=actions.0.into_iter().find_map(|a|match a {UiAction::Edit{edit:edit @ Edit::Exposed(_),..}=>Some(edit),_=>None}).expect("Expose click must emit a document edit");
        let mut history=minihou_app::History::default();history.edit(&mut f.doc,&edit).unwrap();history.finish(&f.doc);
        assert_eq!(f.doc.value()["exposed"][0]["param"],"size");
        assert_eq!(minihou_app::link_interface(f.doc.value())["params"][0]["name"],"box1_size");
        history.undo(&mut f.doc);assert!(!minihou_app::is_exposed(f.doc.value(),"box1","size"));
    }

    #[test]
    fn clicking_group_arrow_emits_viewport_pick_action() {
        let mut f = Fixture::new();
        f.doc = minihou_app::Document::parse(json!({"display":"subdivide1","nodes":{
            "box1":{"type":"box"},"subdivide1":{"type":"subdivide","inputs":{"geo":"box1"}}
        }}).to_string()).unwrap();
        f.graph = minihou_core::Graph::from_value(f.doc.value()).unwrap();
        f.selected = "subdivide1".into(); f.display = "subdivide1".into();
        let ctx = egui::Context::default(); f.theme.apply(&ctx);
        let mut click = None;
        let mut actions = Actions::default();
        for phase in 0..3 {
            let events = match (phase, click) {
                (1,Some(pos)) => vec![egui::Event::PointerMoved(pos), egui::Event::PointerButton { pos, button:egui::PointerButton::Primary, pressed:true, modifiers:egui::Modifiers::NONE }],
                (2,Some(pos)) => vec![egui::Event::PointerButton { pos, button:egui::PointerButton::Primary, pressed:false, modifiers:egui::Modifiers::NONE }],
                _ => vec![],
            };
            let input = egui::RawInput { screen_rect:Some(Rect::from_min_size(Pos2::ZERO,Vec2::new(800.,600.))), events, ..Default::default() };
            let v = f.view();
            let output = ctx.run(input, |ctx| { egui::CentralPanel::default().show(ctx, |ui| {
                let param = minihou_core::nodes::get("subdivide").unwrap().params.iter().find(|p|p.name=="creases").unwrap();
                row(ui,&v,&mut actions,"subdivide1","subdivide",param,false,&mut vec![]);
            }); });
            if phase == 0 {
                for shape in output.shapes { if let egui::Shape::Text(t) = shape.shape {
                    if t.galley.text() == "↖" { click = Some(t.pos + t.galley.size() * 0.5); }
                }}
            }
        }
        assert!(click.is_some(), "purple arrow must be drawn");
        assert!(actions.0.iter().any(|a| matches!(a, UiAction::PickParameter {node,name,mode,..}
            if node=="subdivide1" && name=="creases" && *mode==super::super::SelectionMode::Edges)));
        // the arrow click must not touch the node selection (no Select / ToggleSelect / SelectSet / Display actions)
        assert!(!actions.0.iter().any(|a| matches!(a, UiAction::Select(_) | UiAction::ToggleSelect(_) | UiAction::SelectSet { .. })));
    }

    fn arrow_click_mode(typ: &str, field: &str) -> Option<super::super::SelectionMode> {
        let mut f = Fixture::new();
        f.doc = minihou_app::Document::parse(json!({"display":"n1","nodes":{
            "box1":{"type":"box"},"n1":{"type":typ,"inputs":{"geo":"box1"}}
        }}).to_string()).unwrap();
        f.graph = minihou_core::Graph::from_value(f.doc.value()).unwrap();
        f.selected = "n1".into(); f.display = "n1".into();
        let ctx = egui::Context::default(); f.theme.apply(&ctx);
        let mut click = None;
        let mut actions = Actions::default();
        for phase in 0..3 {
            let events = match (phase, click) {
                (1,Some(pos)) => vec![egui::Event::PointerMoved(pos), egui::Event::PointerButton { pos, button:egui::PointerButton::Primary, pressed:true, modifiers:egui::Modifiers::NONE }],
                (2,Some(pos)) => vec![egui::Event::PointerButton { pos, button:egui::PointerButton::Primary, pressed:false, modifiers:egui::Modifiers::NONE }],
                _ => vec![],
            };
            let input = egui::RawInput { screen_rect:Some(Rect::from_min_size(Pos2::ZERO,Vec2::new(800.,600.))), events, ..Default::default() };
            let v = f.view();
            let output = ctx.run(input, |ctx| { egui::CentralPanel::default().show(ctx, |ui| {
                let param = minihou_core::nodes::get(typ).unwrap().params.iter().find(|p|p.name==field).unwrap();
                row(ui,&v,&mut actions,"n1",typ,param,false,&mut vec![]);
            }); });
            if phase == 0 {
                for shape in output.shapes { if let egui::Shape::Text(t) = shape.shape {
                    if t.galley.text() == "↖" { click = Some(t.pos + t.galley.size() * 0.5); }
                }}
            }
        }
        actions.0.iter().find_map(|a| match a {
            UiAction::PickParameter {node,name,mode,..} if node=="n1" && name==field => Some(*mode),
            _ => None })
    }
    #[test]
    fn clicking_arrow_works_on_many_nodes() {
        use super::super::SelectionMode::*;
        assert_eq!(arrow_click_mode("blast", "group"), Some(Faces));
        assert_eq!(arrow_click_mode("group_create", "pattern"), Some(Points));
        assert_eq!(arrow_click_mode("edge_slide", "group"), Some(Edges));
        assert_eq!(arrow_click_mode("bend", "group"), Some(Points));
    }

    #[test]
    fn titles_and_invalid_ranges() {
        assert_eq!(title("attribute_create"), "Attribute Create");
        assert_eq!(invalid_reason(Some(&json!({"min": 1, "max": 4})), &json!(0)), Some("Value must be at least 1.".into()));
        assert_eq!(invalid_reason(Some(&json!({"min": 1, "max": 4})), &json!(3)), None);
    }

    #[test]
    fn reset_all_uses_only_modified_values() {
        let mut f = Fixture::new();
        f.selected = "noise1".into();
        let before = modified_params(&f.view(), "noise1", "noise");
        f.doc.edit(&Edit::Param { node: "noise1".into(), name: "gain".into(), value: json!(0.7) }).unwrap();
        let after = modified_params(&f.view(), "noise1", "noise");
        assert!(after.contains(&("gain".into(), json!(0.5))));
        assert_eq!(after.len(), before.len() + 1);
    }

    #[test]
    fn both_themes_render() {
        let mut f = Fixture::new();
        f.selected = "noise1".into();
        assert!(f.run(show).is_empty());
        f.theme = Theme::light();
        assert!(f.run(show).is_empty());
        f.flags.insert("noise1".into(), super::super::NodeFlags { locked: true, template: false });
        assert!(f.run(show).is_empty());
    }

    #[test]
    fn parameter_panel_shows_mm_for_length_params() {
        let mut f = Fixture::new();
        f.doc = minihou_app::Document::parse(json!({
            "version": 1,
            "display": "box1",
            "nodes": {
                "box1": {
                    "type": "box",
                    "params": {
                        "size": [1.0, 1.0, 1.0]
                    }
                }
            }
        }).to_string()).unwrap();
        f.graph = minihou_core::Graph::from_value(f.doc.value()).unwrap();
        f.selected = "box1".into();
        f.display = "box1".into();
        f.units = crate::units::UnitSettings { display: crate::units::LengthUnit::Mm, unit_scale: 1.0 };

        let ctx = egui::Context::default();
        f.theme.apply(&ctx);
        let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::pos2(0., 0.), egui::vec2(1280., 800.))), ..Default::default() };
        let mut out = Actions::default();
        let v = f.view();
        let full_output = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| show(ui, &v, &mut out));
        });

        fn extract_texts(shapes: &[egui::epaint::ClippedShape], out: &mut Vec<String>) {
            for s in shapes {
                match &s.shape {
                    egui::epaint::Shape::Text(t) => out.push(t.galley.text().to_owned()),
                    egui::epaint::Shape::Vec(subs) => {
                        let clipped: Vec<egui::epaint::ClippedShape> = subs.iter().map(|sub| egui::epaint::ClippedShape { clip_rect: s.clip_rect, shape: sub.clone() }).collect();
                        extract_texts(&clipped, out);
                    }
                    _ => {}
                }
            }
        }
        let mut texts = Vec::new();
        extract_texts(&full_output.shapes, &mut texts);
        assert!(texts.iter().any(|t| t == "1000 mm"), "expected '1000 mm' in painted texts, got: {texts:?}");
    }

    #[test]
    fn uniform_scale_slider_and_vector_scrub() {
        let mut f = Fixture::new();
        f.doc = minihou_app::Document::parse(json!({
            "version": 1,
            "display": "transform1",
            "nodes": {
                "transform1": {
                    "type": "transform",
                    "params": {
                        "translate": [1.0, 2.0, 3.0],
                        "scale": [1.0, 1.0, 1.0],
                        "uniform_scale": 1.0
                    }
                }
            }
        }).to_string()).unwrap();
        f.graph = minihou_core::Graph::from_value(f.doc.value()).unwrap();
        f.selected = "transform1".into();
        f.display = "transform1".into();
        assert!(f.run(show).is_empty());
    }

    #[test]
    fn houdini_row_layout_geometry() {
        // Standard width 400px: label width is ~30% (120px)
        let r400 = Rect::from_min_size(Pos2::ZERO, Vec2::new(400., 28.));
        let l400 = ParamRowLayout::compute(r400);
        assert!((l400.label_rect.width() - 120.0).abs() < 1e-3);
        assert_eq!(l400.label_rect.left(), 16.0);
        assert_eq!(l400.control_rect.left(), l400.label_rect.right() + 8.0);
        assert!(l400.control_rect.width() > 60.0);

        // Narrow width 350px: label width clamps to min 110px
        let r350 = Rect::from_min_size(Pos2::ZERO, Vec2::new(350., 28.));
        let l350 = ParamRowLayout::compute(r350);
        assert_eq!(l350.label_rect.width(), 110.0);
        assert_eq!(l350.control_rect.left(), l350.label_rect.right() + 8.0);

        // Wide width 1200px: label width clamps to max 240px
        let r1200 = Rect::from_min_size(Pos2::ZERO, Vec2::new(1200., 28.));
        let l1200 = ParamRowLayout::compute(r1200);
        assert_eq!(l1200.label_rect.width(), 240.0);
        assert_eq!(l1200.control_rect.left(), l1200.label_rect.right() + 8.0);

        // Very narrow width 180px: fields shrink gracefully to minimum
        let r180 = Rect::from_min_size(Pos2::ZERO, Vec2::new(180., 28.));
        let l180 = ParamRowLayout::compute(r180);
        assert!(l180.label_rect.width() >= 30.0);
        assert!(l180.control_rect.width() >= 10.0);
        assert_eq!(l180.control_rect.left(), l180.label_rect.right() + 8.0);
    }

    #[test]
    fn label_truncation_helper() {
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let font = FontId::proportional(12.);
                let full = "Consolidate Corner Points";
                let untruncated = truncate_with_ellipsis(ui, full, &font, 300.0);
                assert_eq!(untruncated, full);

                let truncated = truncate_with_ellipsis(ui, full, &font, 50.0);
                assert!(truncated.ends_with('…'));
                assert!(truncated.len() < full.len());
            });
        });
    }

    #[test]
    fn boolean_row_omits_label_column_and_renders_toggle_at_field_column() {
        let mut f = Fixture::new();
        f.doc = minihou_app::Document::parse(json!({
            "version": 1,
            "display": "box1",
            "nodes": {
                "box1": {
                    "type": "box",
                    "params": {
                        "consolidate": true
                    }
                }
            }
        }).to_string()).unwrap();
        f.graph = minihou_core::Graph::from_value(f.doc.value()).unwrap();
        f.selected = "box1".into();
        f.display = "box1".into();

        let ctx = egui::Context::default();
        f.theme.apply(&ctx);
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(500., 400.))), ..Default::default() };
        let mut out = Actions::default();
        let v = f.view();
        let full_output = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| show(ui, &v, &mut out));
        });

        let mut label_texts = Vec::new();
        fn collect_texts(shapes: &[egui::epaint::ClippedShape], out: &mut Vec<(String, Pos2)>) {
            for s in shapes {
                match &s.shape {
                    egui::epaint::Shape::Text(t) => out.push((t.galley.text().to_owned(), t.pos)),
                    egui::epaint::Shape::Vec(subs) => {
                        let clipped: Vec<egui::epaint::ClippedShape> = subs.iter().map(|sub| egui::epaint::ClippedShape { clip_rect: s.clip_rect, shape: sub.clone() }).collect();
                        collect_texts(&clipped, out);
                    }
                    _ => {}
                }
            }
        }
        collect_texts(&full_output.shapes, &mut label_texts);

        let row_layout = ParamRowLayout::compute(Rect::from_min_size(Pos2::ZERO, Vec2::new(500., 28.)));
        let consolidate_text = label_texts.iter().find(|(t, _)| t.contains("Consolidate"));
        assert!(consolidate_text.is_some(), "Consolidate toggle text should be present");
        let (_, pos) = consolidate_text.unwrap();
        assert!(pos.x >= row_layout.label_rect.right(), "Checkbox text pos ({}) must sit at/after field column start ({})", pos.x, row_layout.label_rect.right());
    }

    /// f32 round-trips: compare numbers with a tolerance, everything else exactly.
fn json_close(a: &serde_json::Value, b: &serde_json::Value) -> bool {
    use serde_json::Value::*;
    match (a, b) {
        (Number(x), Number(y)) => (x.as_f64().unwrap_or(f64::NAN) - y.as_f64().unwrap_or(f64::NAN)).abs() < 1e-5,
        (Array(x), Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(p, q)| json_close(p, q)),
        (Object(x), Object(y)) => x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| json_close(v, w))),
        _ => a == b,
    }
}
    #[test]
    fn color_deserialization_and_serialization() {
        // 3-component RGB array
        let rgb_val = json!([0.2, 0.4, 0.6]);
        let (rgba, comps, is_hex) = parse_color_value(&rgb_val).expect("parse rgb");
        assert_eq!(comps, 3);
        assert!(!is_hex);
        assert!((rgba[0] - 0.2).abs() < 1e-4);
        assert!((rgba[1] - 0.4).abs() < 1e-4);
        assert!((rgba[2] - 0.6).abs() < 1e-4);
        assert_eq!(rgba[3], 1.0);
        let ser_rgb = serialize_color_value(rgba, comps, is_hex);
        assert!(json_close(&ser_rgb, &rgb_val), "{ser_rgb:?} vs {rgb_val:?}");

        // 4-component RGBA array
        let rgba_val = json!([0.1, 0.2, 0.3, 0.8]);
        let (rgba, comps, is_hex) = parse_color_value(&rgba_val).expect("parse rgba");
        assert_eq!(comps, 4);
        assert!(!is_hex);
        assert!((rgba[0] - 0.1).abs() < 1e-4);
        assert!((rgba[3] - 0.8).abs() < 1e-4);
        let ser_rgba = serialize_color_value(rgba, comps, is_hex);
        assert!(json_close(&ser_rgba, &rgba_val), "{ser_rgba:?} vs {rgba_val:?}");

        // 6-digit hex string
        let hex_val = json!("#336699");
        let (rgba, comps, is_hex) = parse_color_value(&hex_val).expect("parse hex");
        assert_eq!(comps, 3);
        assert!(is_hex);
        assert_eq!((rgba[0] * 255.0).round() as u8, 0x33);
        assert_eq!((rgba[1] * 255.0).round() as u8, 0x66);
        assert_eq!((rgba[2] * 255.0).round() as u8, 0x99);
        let ser_hex = serialize_color_value(rgba, comps, is_hex);
        assert_eq!(ser_hex, hex_val);

        // 8-digit hex string
        let hex_alpha_val = json!("#336699cc");
        let (rgba, comps, is_hex) = parse_color_value(&hex_alpha_val).expect("parse hex alpha");
        assert_eq!(comps, 4);
        assert!(is_hex);
        assert_eq!((rgba[3] * 255.0).round() as u8, 0xcc);
        let ser_hex_alpha = serialize_color_value(rgba, comps, is_hex);
        assert_eq!(ser_hex_alpha, hex_alpha_val);

        // 3-digit shorthand hex string
        let hex3_val = json!("#369");
        let (rgba, comps, is_hex) = parse_color_value(&hex3_val).expect("parse hex3");
        assert_eq!(comps, 3);
        assert!(is_hex);
        assert_eq!((rgba[0] * 255.0).round() as u8, 0x33);
        assert_eq!((rgba[1] * 255.0).round() as u8, 0x66);
        assert_eq!((rgba[2] * 255.0).round() as u8, 0x99);
        let ser_hex3 = serialize_color_value(rgba, comps, is_hex);
        assert_eq!(ser_hex3, json!("#336699"));

        // Invalid values
        assert!(parse_color_value(&json!("not-a-color")).is_none());
        assert!(parse_color_value(&json!([1.0, 2.0])).is_none());
        assert!(parse_color_value(&json!(123)).is_none());
    }

    #[test]
    fn ramp_deserialization_and_serialization() {
        // Format 1: StopsObject {"stops": [...], "interpolation": "..."}
        let stops_obj = json!({
            "stops": [
                {"pos": 0.0, "value": 0.2},
                {"pos": 1.0, "value": 0.8}
            ],
            "interpolation": "smooth"
        });
        let ramp1 = RampData::from_value(&stops_obj, false);
        assert_eq!(ramp1.format, RampFormat::StopsObject);
        assert_eq!(ramp1.stops.len(), 2);
        assert_eq!(ramp1.interpolation, RampInterpolation::Smooth);
        assert_eq!(ramp1.stops[0].value, RampValue::Float(0.2));
        assert_eq!(ramp1.stops[1].value, RampValue::Float(0.8));
        let ser1 = ramp1.to_value();
        assert_eq!(ser1["stops"].as_array().unwrap().len(), 2);
        assert_eq!(ser1["interpolation"], "smooth");

        // Format 2: KeysValuesObject {"keys": [...], "values": [...], "basis": "..."}
        let keys_obj = json!({
            "keys": [0.0, 0.5, 1.0],
            "values": [0.0, 0.4, 1.0],
            "basis": "linear"
        });
        let ramp2 = RampData::from_value(&keys_obj, false);
        assert_eq!(ramp2.format, RampFormat::KeysValuesObject);
        assert_eq!(ramp2.stops.len(), 3);
        assert_eq!(ramp2.interpolation, RampInterpolation::Linear);
        let ser2 = ramp2.to_value();
        assert_eq!(ser2["keys"].as_array().unwrap().len(), 3);
        assert_eq!(ser2["values"].as_array().unwrap().len(), 3);
        assert_eq!(ser2["basis"], "linear");

        // Format 3: ArrayOfPairs [[pos, val], ...]
        let pairs = json!([
            [0.0, 0.1],
            [1.0, 0.9]
        ]);
        let ramp3 = RampData::from_value(&pairs, false);
        assert_eq!(ramp3.format, RampFormat::ArrayOfPairs);
        assert_eq!(ramp3.stops.len(), 2);
        let ser3 = ramp3.to_value();
        assert!(json_close(&ser3, &pairs), "{ser3:?} vs {pairs:?}");

        // Format 4: ArrayOfStops [{"pos": 0.0, "color": [...]}, ...]
        let stops_arr = json!([
            {"pos": 0.0, "color": [1.0, 0.0, 0.0]},
            {"pos": 1.0, "color": [0.0, 0.0, 1.0]}
        ]);
        let ramp4 = RampData::from_value(&stops_arr, true);
        assert_eq!(ramp4.format, RampFormat::ArrayOfStops);
        assert!(ramp4.is_color);
        assert_eq!(ramp4.stops.len(), 2);
        let ser4 = ramp4.to_value();
        assert_eq!(ser4[0]["pos"], 0.0);
        assert_eq!(ser4[0]["color"], json!([1.0, 0.0, 0.0]));
        assert_eq!(ser4[1]["pos"], 1.0);
        assert_eq!(ser4[1]["color"], json!([0.0, 0.0, 1.0]));
    }

    #[test]
    fn ramp_stop_add_delete_sort_and_eval() {
        let mut ramp = RampData {
            is_color: false,
            stops: vec![
                RampStop { pos: 0.0, value: RampValue::Float(0.0), interp: None },
                RampStop { pos: 1.0, value: RampValue::Float(1.0), interp: None },
            ],
            interpolation: RampInterpolation::Linear,
            format: RampFormat::ArrayOfPairs,
        };

        // Linear evaluation
        assert!((ramp.eval_float(0.0) - 0.0).abs() < 1e-4);
        assert!((ramp.eval_float(0.5) - 0.5).abs() < 1e-4);
        assert!((ramp.eval_float(1.0) - 1.0).abs() < 1e-4);

        // Constant evaluation
        ramp.interpolation = RampInterpolation::Constant;
        assert!((ramp.eval_float(0.49) - 0.0).abs() < 1e-4);
        assert!((ramp.eval_float(0.99) - 0.0).abs() < 1e-4);

        // Smooth evaluation
        ramp.interpolation = RampInterpolation::Smooth;
        assert!((ramp.eval_float(0.5) - 0.5).abs() < 1e-4);

        // Add stop at 0.5
        let idx = ramp.add_stop(0.5);
        assert_eq!(ramp.stops.len(), 3);
        assert_eq!(idx, 1);
        assert_eq!(ramp.stops[1].pos, 0.5);

        // Move stop 1 to 0.2 (verify sorted order)
        let new_idx = ramp.move_stop(1, 0.2);
        assert_eq!(new_idx, 1);
        assert_eq!(ramp.stops[1].pos, 0.2);

        // Move stop 1 to 0.8 (verify re-sort)
        let moved_idx = ramp.move_stop(1, 0.8);
        assert_eq!(moved_idx, 1);
        assert_eq!(ramp.stops[1].pos, 0.8);

        // Delete stop
        assert!(ramp.delete_stop(1));
        assert_eq!(ramp.stops.len(), 2);

        // Cannot delete below 2 stops
        assert!(ramp.delete_stop(0));
        assert_eq!(ramp.stops.len(), 1);
        assert!(!ramp.delete_stop(0)); // Cannot delete the last stop
        assert_eq!(ramp.stops.len(), 1);

        // Color eval test
        let color_ramp = RampData {
            is_color: true,
            stops: vec![
                RampStop { pos: 0.0, value: RampValue::Color([0.0, 0.2, 0.4]), interp: None },
                RampStop { pos: 1.0, value: RampValue::Color([1.0, 0.8, 0.6]), interp: None },
            ],
            interpolation: RampInterpolation::Linear,
            format: RampFormat::ArrayOfPairs,
        };
        let mid = color_ramp.eval_color(0.5);
        assert!((mid[0] - 0.5).abs() < 1e-3);
        assert!((mid[1] - 0.5).abs() < 1e-3);
        assert!((mid[2] - 0.5).abs() < 1e-3);
        assert_eq!(mid[3], 1.0);
    }

    #[test]
    fn param_type_detection() {
        // Color
        assert!(is_color_param("color", None, &json!([1.0, 1.0, 1.0])));
        assert!(is_color_param("tint", None, &json!("#ff0000")));
        assert!(is_color_param("diffuse", None, &json!([0.5, 0.5, 0.5])));
        assert!(is_color_param("custom", Some(&json!({"kind": "color"})), &json!(null)));
        assert!(!is_color_param("translate", None, &json!([1.0, 2.0, 3.0])));

        // File
        assert!(is_file_param("file", None));
        assert!(is_file_param("path", None));
        assert!(is_file_param("rule_file", None));
        assert!(is_file_param("custom", Some(&json!({"kind": "file"}))));
        assert!(is_file_param("custom", Some(&json!({"ui": "file:open"}))));
        assert!(!is_file_param("scale", None));

        // Ramp
        assert!(is_ramp_param("float_ramp", None, &json!(null)));
        assert!(is_ramp_param("color_ramp", None, &json!(null)));
        assert!(is_ramp_param("custom", Some(&json!({"kind": "float_ramp"})), &json!(null)));
        assert!(is_ramp_param("custom", Some(&json!({"kind": "color_ramp"})), &json!(null)));
        assert!(is_ramp_param("my_ramp", None, &json!(null)));
        assert!(is_ramp_param("custom", None, &json!({"stops": []})));
        assert!(!is_ramp_param("scale", None, &json!(1.0)));
    }

    #[test]
    fn widget_styled_renders_color_file_and_ramp() {
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                // Color widget
                let mut col_val = json!([0.2, 0.4, 0.6]);
                let changed_col = widget(ui, ParamKind::Unsupported, Some(&json!({"kind": "color"})), &mut col_val);
                assert!(!changed_col);

                // File widget
                let mut file_val = json!("models/teapot.obj");
                let changed_file = widget(ui, ParamKind::String, Some(&json!({"kind": "file"})), &mut file_val);
                assert!(!changed_file);

                // Ramp widget
                let mut ramp_val = json!({"stops": [{"pos": 0.0, "value": 0.0}, {"pos": 1.0, "value": 1.0}]});
                let changed_ramp = widget(ui, ParamKind::Unsupported, Some(&json!({"kind": "float_ramp"})), &mut ramp_val);
                assert!(!changed_ramp);
            });
        });
    }

    /// Drives number_field through real egui frames: arrow press, hold-repeat with clamping,
    /// Alt+wheel stepping and plain wheel left to the panel's scrolling.
    #[test]
    fn number_field_arrows_and_gated_wheel() {
        let ctx = egui::Context::default();
        let theme = Theme::dark();
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(400., 200.));
        let field = std::cell::Cell::new(Rect::NOTHING);
        let frame = |ctx: &egui::Context, v: &mut f64, range: std::ops::RangeInclusive<f64>, integer: bool, time: f64, modifiers: egui::Modifiers, events: Vec<egui::Event>| -> (bool, egui::Vec2) {
            let mut changed = false;
            let input = egui::RawInput { screen_rect: Some(screen), time: Some(time), modifiers, events, ..Default::default() };
            let mut leftover = egui::Vec2::ZERO;
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let (c, r) = number_field(ui, 120., egui::Id::new("nf"), v, 0.1, &range, integer, Some((0., 10.)), Align2::RIGHT_CENTER, &theme, None);
                    changed = c; field.set(r);
                });
                leftover = ctx.input(|i| i.smooth_scroll_delta + i.raw_scroll_delta);
            });
            (changed, leftover)
        };
        let none = egui::Modifiers::NONE;
        let mut v = 2.;
        frame(&ctx, &mut v, 0.0..=3.0, true, 0., none, vec![]);
        let fr = field.get();
        // increment button at the right end, decrement at the left end, each the full field height
        let up = Pos2::new(fr.right() - 10., fr.bottom() - 4.);
        let down = Pos2::new(fr.left() + 10., fr.top() + 4.);
        let press = |pos: Pos2, pressed: bool| vec![egui::Event::PointerMoved(pos), egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE }];
        // press the up arrow: one step on press, then held repeats stop at max 3
        assert!(frame(&ctx, &mut v, 0.0..=3.0, true, 0.1, none, press(up, true)).0);
        assert_eq!(v, 3.);
        frame(&ctx, &mut v, 0.0..=3.0, true, 2.0, none, vec![]);
        assert_eq!(v, 3.);
        frame(&ctx, &mut v, 0.0..=3.0, true, 2.1, none, press(up, false));
        // a held down arrow repeats after the delay: press (1) + due at 0.5 s held
        let mut w = 10.;
        frame(&ctx, &mut w, f64::NEG_INFINITY..=f64::INFINITY, true, 3.0, none, press(down, true));
        assert_eq!(w, 9.);
        frame(&ctx, &mut w, f64::NEG_INFINITY..=f64::INFINITY, true, 3.5, none, vec![]);
        assert_eq!(w, 10. - number_step::repeat_steps_due(0.5) as f64);
        frame(&ctx, &mut w, f64::NEG_INFINITY..=f64::INFINITY, true, 3.6, none, press(down, false));
        // plain wheel over the field scrolls the panel, Alt+wheel steps (float: soft range 0..10 -> 0.1)
        let mut f = 1.;
        let wheel = |m: egui::Modifiers| vec![egui::Event::PointerMoved(fr.center()), egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Line, delta: Vec2::new(0., 1.), modifiers: m }];
        let (changed, leftover) = frame(&ctx, &mut f, 0.0..=10.0, false, 4.0, none, wheel(none));
        assert!(!changed && f == 1. && leftover != Vec2::ZERO, "{leftover:?}");
        let alt = egui::Modifiers { alt: true, ..Default::default() };
        let (changed, leftover) = frame(&ctx, &mut f, 0.0..=10.0, false, 4.1, alt, wheel(alt));
        assert!(changed && (f - 1.1).abs() < 1e-9 && leftover == Vec2::ZERO, "{f} {leftover:?}");
        let fine = egui::Modifiers { alt: true, shift: true, ..Default::default() };
        frame(&ctx, &mut f, 0.0..=10.0, false, 4.2, fine, wheel(fine));
        assert!((f - 1.11).abs() < 1e-9, "{f}");
    }
}
