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
    if typ=="polyextrude" {
        return Some(if matches!(name,"group"|"edges"|"points"|"divide_into"|"split_group") {"Selection"}
            else if name.starts_with("outgrp_") || matches!(name,"output_front"|"output_back"|"output_side"|"preserve_groups"|"copy_edge_groups"|"copy_point_groups"|"copy_primitive_groups") {"Output Geometry and Groups"}
            else if matches!(name,"front_magnitude"|"back_magnitude"|"front_stiffness"|"back_stiffness"|"interpolation"|"division_spacing"|"reverse_spine_direction"|"axial_rotation"|"front_blend"|"back_blend"|"thickness_scale"|"thickness_ramp"|"thickness_attribute"|"twist_range_half_width"|"local_twist_ramp"|"twist_attribute") {"Spine Control"}
            else if name.ends_with("_attribute") {"Local Control"}
            else {"Extrusion"});
    }
    if !super::deform_gizmo::is_deformer(typ) { return None; }
    Some(if matches!(name,"group"|"outside"|"max_points") {"Selection"}
        else if matches!(name,"origin"|"center"|"direction"|"up"|"length"|"radius"|"axis"|"frame"|"offset"|"stretch"|"symmetric"|"up_vector_angle") {"Capture"}
        else {"Deformation"})
}
fn param_section<'a>(typ: &str, name: &str, meta: Option<&'a Value>) -> Option<&'a str> {
    meta.and_then(|m| m["section"].as_str()).or_else(|| deformer_section(typ, name))
}
#[cfg(test)]
mod deformer_group_tests {
    use super::{deformer_section, param_section};
    use serde_json::json;
    #[test]
    fn capture_and_deformation_fields_are_grouped() {
        assert_eq!(deformer_section("bend","origin"),Some("Capture"));
        assert_eq!(deformer_section("linear_taper","taper_x"),Some("Deformation"));
        assert_eq!(deformer_section("point_deform","group"),Some("Selection"));
        assert_eq!(deformer_section("box","size"),None);
        assert_eq!(param_section("bend", "origin", Some(&json!({"section":"Manifest"}))), Some("Manifest"));
        assert_eq!(param_section("bend", "origin", None), Some("Capture"));
    }
}
#[cfg(test)]
mod polyextrude_panel_tests {
    use super::*;
    #[test]
    fn polyextrude_metadata_has_spine_local_and_houdini_output_group_names() {
        assert_eq!(deformer_section("polyextrude","thickness_ramp"),Some("Spine Control"));
        assert_eq!(deformer_section("polyextrude","distance_scale_attribute"),Some("Local Control"));
        assert_eq!(deformer_section("polyextrude","outgrp_front_name"),Some("Output Geometry and Groups"));
        assert_eq!(param_meta("polyextrude","offset_even").unwrap()["default"],json!(true));
        assert_eq!(param_meta("polyextrude","outgrp_front_name").unwrap()["default"],json!("extrudeFront"));
        assert_eq!(param_meta("polyextrude","outgrp_back_name").unwrap()["default"],json!("extrudeBack"));
        assert_eq!(param_meta("polyextrude","outgrp_side_name").unwrap()["default"],json!("extrudeSide"));
    }
}
pub mod output_groups;
pub use output_groups::{is_output_group_field, output_group_label, output_group_toggle, target_field, TARGET_TEAL};
pub mod pick;
pub use pick::{domain_edit_mode, is_input_group_field, pick_domain, pick_domain_for, pick_field, PICK_PURPLE, SELECT_ORANGE};
pub mod widgets;
pub use widgets::{boxed, color_widget, drag_number, file_widget, number_field, ramp_widget, render_vector_fields, widget, widget_styled, widget_with_units};
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

pub(crate) fn title(name: &str) -> String {
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

#[path = "param_values.rs"]
mod param_values;
#[allow(unused_imports)] // Keep the existing params module names available to callers.
pub use param_values::{parse_color_value, serialize_color_value, is_color_param, is_file_param, is_ramp_param, RampInterpolation, RampValue, RampStop, RampFormat, RampData};

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

    let output_group = is_output_group_field(param.name, meta);
    let row_height = if is_ramp { 76.0 } else { 28.0 };
    let (r, row_response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), row_height), if output_group { Sense::hover() } else { Sense::click() });
    let layout = ParamRowLayout::compute(r);
    if ui.ctx().data(|d| d.get_temp::<(String,String,f64)>(egui::Id::new("link_flash")))
        .is_some_and(|(node,name,time)| node == id && (name == param.name || name.starts_with(&format!("{}.",param.name))) && ui.ctx().input(|i|i.time) - time < 1.5) {
        ui.painter().rect_filled(r,3.,v.theme.accent.gamma_multiply(0.22));
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
    }

    let param_locked = v.doc["nodes"][id]["locked_params"].as_array().is_some_and(|a| a.iter().any(|x| x == param.name));
    let channels = v.doc["nodes"][id]["channels"].as_object();
    let linked = channels.is_some_and(|c| c.keys().any(|key| key == param.name || key.starts_with(&format!("{}.",param.name))));
    let group_toggle = output_group_toggle(typ, param.name, meta);
    let group_on = group_toggle.map_or_else(|| !old.as_str().unwrap_or("").is_empty(), |name| {
        let fallback = nodes::get(typ).and_then(|spec| spec.params.iter().find(|p| p.name == name))
            .map(|p| (p.default)()).unwrap_or(Value::Bool(false));
        value_of(v, id, name, &fallback).as_bool().unwrap_or(false)
    });
    let enabled = param.supported && !locked && !param_locked && !linked && condition_enabled;
    let checkbox_enabled = param.supported && !locked && !param_locked && !linked && group_toggle.is_none_or(|name| {
        !v.doc["nodes"][id]["locked_params"].as_array().is_some_and(|a| a.iter().any(|value| value == name))
            && !channels.is_some_and(|c| c.contains_key(name))
    });
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
    let label_text = if typ=="polyextrude" {
        match param.name {
            "outgrp_front"=>"Front Group".into(),"outgrp_back"=>"Back Group".into(),"outgrp_side"=>"Side Group".into(),
            "outgrp_front_name"=>"Front Group Name".into(),"outgrp_back_name"=>"Back Group Name".into(),"outgrp_side_name"=>"Side Group Name".into(),
            "outgrp_front_boundary"=>"Front Boundary Group".into(),"outgrp_back_boundary"=>"Back Boundary Group".into(),
            "outgrp_front_boundary_name"=>"Front Boundary Group Name".into(),"outgrp_back_boundary_name"=>"Back Boundary Group Name".into(),
            "generate_sides_uvs"=>"Generate Unwrapped Texture Coordinates for Sides".into(),
            "local_twist_ramp"=>"Local Twist Ramp".into(),"twist_range_half_width"=>"Range Half-Width".into(),
            "distance_scale_attribute"=>"Distance Scale".into(),"inset_scale_attribute"=>"Inset Scale".into(),
            "twist_scale_attribute"=>"Twist Scale".into(),"divisions_scale_attribute"=>"Divisions Scale".into(),
            "local_x_attribute"=>"X Direction".into(),"local_z_attribute"=>"Z Direction".into(),"local_center_attribute"=>"Center".into(),
            _=>title(param.name)
        }
    } else if output_group { output_group_label(param.name) }
        else if typ == "scene_objects" && param.name == "piece_attrib" { "Object ID Attribute".to_owned() }
        else if is_length { format!("{} ({})", title(param.name), v.units.display.symbol()) } else { title(param.name) };

    let is_bool = kind == ParamKind::Bool;
    let is_vector = kind == ParamKind::Vec3 || value.as_array().is_some_and(|a| (2..=4).contains(&a.len()) && a.iter().all(Value::is_number));

    let font = FontId::proportional(12.);
    let displayed_label = if !is_bool {
        truncate_with_ellipsis(ui, &label_text, &font, (layout.label_rect.width() - if output_group { 26. } else { 0. }).max(10.))
    } else {
        String::new()
    };

    if !is_bool {
        if output_group {
            let mut checked = group_on;
            let checkbox_rect = Rect::from_min_size(layout.label_rect.min + Vec2::new(2., 3.), Vec2::splat(22.));
            let response = ui.add_enabled_ui(checkbox_enabled, |ui| ui.put(checkbox_rect, egui::Checkbox::without_text(&mut checked))).inner;
            if response.changed() && checkbox_enabled {
                if let Some(toggle) = group_toggle {
                    out.push(UiAction::Edit { edit: Edit::Param { node: id.to_owned(), name: toggle.to_owned(), value: json!(checked) }, continuous: false });
                } else if !checked {
                    let cache = egui::Id::new(("output_group_name", id, param.name));
                    if let Some(name) = value.as_str().filter(|name| !name.is_empty()) {
                        ui.ctx().data_mut(|d| d.insert_persisted(cache, name.to_owned()));
                    }
                    value = json!("");
                } else {
                    let cache = egui::Id::new(("output_group_name", id, param.name));
                    let name = ui.ctx().data_mut(|d| d.get_persisted::<String>(cache))
                        .filter(|name| !name.is_empty())
                        .unwrap_or_else(|| default.as_str().filter(|name| !name.is_empty()).unwrap_or(param.name).to_owned());
                    value = json!(name);
                }
            }
        }
        // Houdini layout: labels are RIGHT-aligned and vertically centred against the row
        let label_pos_y = if is_ramp { r.top() + 14. } else { r.center().y };
        paint_text(
            ui,
            Pos2::new(layout.label_rect.right(), label_pos_y),
            &displayed_label,
            12.,
            false,
            if output_group && !group_on { v.theme.faint } else { ink },
            Align2::RIGHT_CENTER,
        );

        let label_sense = if is_vector && enabled { Sense::click_and_drag() } else { Sense::click() };
        let label_rect = if output_group {
            Rect::from_min_max(Pos2::new(layout.label_rect.left() + 26., layout.label_rect.top()), layout.label_rect.max)
        } else { layout.label_rect };
        let label_response = ui.interact(label_rect, ui.id().with((id, param.name, "label")), label_sense);
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
        let response = child.add_enabled_ui(enabled && (!output_group || group_on), |ui| {
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
    if typ == "controller" {
        controller_panel(ui, v, out, id);
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
                let outputs: Vec<_> = spec.params.iter().filter(|param| is_output_group_field(param.name, param_meta(typ, param.name))).collect();
                let output_toggles: Vec<_> = outputs.iter().filter_map(|param| output_group_toggle(typ, param.name, param_meta(typ, param.name))).collect();
                let mut section=None;
                for param in spec.params {
                    if typ == "bezier" && param.name == "controls" { continue; }
                    if typ == "knife" && param.name == "cuts" { continue; }
                    if typ == "attribute_create" && param.name != "group" { continue; }
                    if outputs.iter().any(|output| output.name == param.name) || output_toggles.contains(&param.name) { continue; }
                    let next=param_section(typ,param.name,param_meta(typ,param.name));
                    if next.is_some() && next!=section {
                        section=next;
                        ui.add_space(5.);
                        ui.label(egui::RichText::new(next.unwrap()).strong().color(v.theme.accent));
                    }
                    ui.push_id(param.name, |ui| row(ui, v, out, id, typ, param, flags.locked, &mut errors));
                }
                if !outputs.is_empty() {
                    ui.add_space(5.);
                    ui.label(egui::RichText::new("Output Groups").strong().color(v.theme.accent));
                    for param in outputs {
                        ui.push_id(param.name, |ui| row(ui, v, out, id, typ, param, flags.locked, &mut errors));
                    }
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

fn controller_panel(ui: &mut egui::Ui, v: &View<'_>, out: &mut Actions, id: &str) {
    let node = &v.doc["nodes"][id];
    let mut sections = node["controller_sections"].as_array().cloned().unwrap_or_default();
    let mut changed = false;
    ui.heading(format!("Controller: {id}"));
    ui.small("Use ch('../controller/parameter') to drive another node.");
    egui::ScrollArea::vertical().show(ui, |ui| {
        let section_drag_key=egui::Id::new(("controller_section_drag",id));
        let mut section_dragging=ui.ctx().data(|d|d.get_temp::<usize>(section_drag_key));
        let mut section_move=None;
        for si in 0..sections.len() {
            ui.push_id(si, |ui| {
                ui.separator();
                let mut title = sections[si]["name"].as_str().unwrap_or("").to_owned();
                let heading=ui.horizontal(|ui| {
                    let handle=ui.add(egui::Label::new("☰").sense(Sense::drag()));
                    if handle.drag_started() {section_dragging=Some(si);}
                    if ui.text_edit_singleline(&mut title).changed() && !title.trim().is_empty() {
                        sections[si]["name"] = json!(title.trim()); changed = true;
                    }
                    if ui.add_enabled(si > 0, egui::Button::new("↑")).clicked() { sections.swap(si, si-1); changed = true; }
                    if ui.add_enabled(si+1 < sections.len(), egui::Button::new("↓")).clicked() { sections.swap(si, si+1); changed = true; }
                    if ui.button("Remove section").clicked() { sections.remove(si); changed = true; }
                });
                if ui.ctx().input(|i|i.pointer.any_released()) && heading.response.hovered() {
                    if let Some(from)=section_dragging {if from!=si {section_move=Some((from,si));}}
                }
                if si >= sections.len() { return; }
                let mut entries = sections[si]["parameters"].as_array().cloned().unwrap_or_default();
                let drag_key=egui::Id::new(("controller_drag",id,si));
                let mut dragging=ui.ctx().data(|d|d.get_temp::<usize>(drag_key));
                let mut move_to=None;
                for pi in 0..entries.len() {
                    ui.push_id(pi, |ui| {
                        let name = entries[pi]["name"].as_str().unwrap_or("").to_owned();
                        let kind = entries[pi]["kind"].as_str().unwrap_or("float").to_owned();
                        let row=ui.horizontal(|ui| {
                            let handle=ui.add(egui::Label::new("☰").sense(Sense::drag()));
                            if handle.drag_started() {dragging=Some(pi);}
                            ui.label(&name);
                            ui.small(&kind);
                            let old = node["params"].get(&name).unwrap_or(&entries[pi]["default"]);
                            let mut value = old.clone();
                            let edited = match kind.as_str() {
                                "float" => { let mut n=value.as_f64().unwrap_or(0.); let yes=ui.add(egui::DragValue::new(&mut n).speed(0.01)).changed(); value=json!(n); yes },
                                "int" => { let mut n=value.as_i64().unwrap_or(0); let yes=ui.add(egui::DragValue::new(&mut n)).changed(); value=json!(n); yes },
                                "bool" => { let mut b=value.as_bool().unwrap_or(false); let yes=ui.checkbox(&mut b, "").changed(); value=json!(b); yes },
                                "vector" | "color" => { let mut any=false; let count=if kind=="vector" {3} else {4}; let mut xs=(0..count).map(|i|value[i].as_f64().unwrap_or(0.)).collect::<Vec<_>>(); for x in &mut xs {any|=ui.add(egui::DragValue::new(x).speed(0.01)).changed();} value=json!(xs); any },
                                "menu" => { let mut s=value.as_str().unwrap_or("").to_owned(); let before=s.clone(); egui::ComboBox::from_id_salt("choice").selected_text(&s).show_ui(ui, |ui| {for option in entries[pi]["options"].as_array().into_iter().flatten().filter_map(Value::as_str) {ui.selectable_value(&mut s,option.into(),option);}}); value=json!(s); s!=before },
                                _ => {let mut s=value.as_str().unwrap_or("").to_owned(); let yes=ui.text_edit_singleline(&mut s).changed(); value=json!(s); yes},
                            };
                            if edited { out.push(UiAction::Edit {edit: Edit::Param {node:id.into(),name:name.clone(),value},continuous:true}); }
                            let mut expose = entries[pi]["expose_to_blender"].as_bool().unwrap_or(false);
                            if matches!(kind.as_str(),"float"|"int"|"bool"|"vector"|"color") && ui.checkbox(&mut expose,"Expose to Blender").changed() {
                                entries[pi]["expose_to_blender"]=json!(expose); changed=true;
                            }
                            if ui.add_enabled(pi>0,egui::Button::new("↑")).clicked() {entries.swap(pi,pi-1);changed=true;}
                            if ui.add_enabled(pi+1<entries.len(),egui::Button::new("↓")).clicked() {entries.swap(pi,pi+1);changed=true;}
                            if ui.button("×").clicked() {entries.remove(pi);changed=true;}
                        });
                        if ui.ctx().input(|i|i.pointer.any_released()) && row.response.hovered() {
                            if let Some(from)=dragging {if from!=pi {move_to=Some((from,pi));}}
                        }
                        if pi < entries.len() && kind == "menu" {
                            let mut options=entries[pi]["options"].as_array().into_iter().flatten().filter_map(Value::as_str).collect::<Vec<_>>().join(", ");
                            ui.horizontal(|ui| {ui.label("Choices");
                                if ui.text_edit_singleline(&mut options).lost_focus() {
                                    let choices=options.split(',').map(str::trim).filter(|s|!s.is_empty()).collect::<Vec<_>>();
                                    if !choices.is_empty() {
                                        entries[pi]["options"]=json!(choices);
                                        if !choices.iter().any(|s|entries[pi]["default"]==*s) {entries[pi]["default"]=json!(choices[0]);}
                                        changed=true;
                                    }
                                }
                            });
                        }
                    });
                    if pi>=entries.len() {break;}
                }
                if let Some((from,to))=move_to {let entry=entries.remove(from);entries.insert(to,entry);changed=true;}
                if ui.ctx().input(|i|i.pointer.any_released()) {dragging=None;}
                ui.ctx().data_mut(|d|if let Some(from)=dragging {d.insert_temp(drag_key,from)} else {d.remove::<usize>(drag_key)});
                sections[si]["parameters"]=json!(entries);
                let key=egui::Id::new(("controller_draft",id,si));
                let mut draft=ui.ctx().data_mut(|d|d.get_persisted::<String>(key)).unwrap_or_default();
                let kind_key=key.with("kind");
                let mut kind=ui.ctx().data_mut(|d|d.get_persisted::<String>(kind_key)).unwrap_or_else(||"float".into());
                ui.horizontal(|ui| {
                    ui.text_edit_singleline(&mut draft);
                    egui::ComboBox::from_id_salt("new_kind").selected_text(&kind).show_ui(ui,|ui|for option in minihou_core::controller::KINDS {ui.selectable_value(&mut kind,(*option).into(),*option);});
                    if ui.button("Add parameter").clicked() {
                        let name=draft.trim();
                        if !name.is_empty() && !sections.iter().any(|s|s["parameters"].as_array().into_iter().flatten().any(|p|p["name"]==name)) {
                            let options=if kind=="menu" {json!(["Option"])} else {json!([])};
                            let default=if kind=="menu" {json!("Option")} else {minihou_core::controller::default(&kind)};
                            sections[si]["parameters"].as_array_mut().unwrap().push(json!({"name":name,"kind":kind,"default":default,"options":options,"expose_to_blender":false}));
                            changed=true; draft.clear();
                        }
                    }
                });
                ui.ctx().data_mut(|d|{d.insert_persisted(key,draft);d.insert_persisted(kind_key,kind);});
            });
        }
        if let Some((from,to))=section_move {let entry=sections.remove(from);sections.insert(to,entry);changed=true;}
        if ui.ctx().input(|i|i.pointer.any_released()) {section_dragging=None;}
        ui.ctx().data_mut(|d|if let Some(from)=section_dragging {d.insert_temp(section_drag_key,from)} else {d.remove::<usize>(section_drag_key)});
        if ui.button("Add section").clicked() {
            let mut i=sections.len()+1;
            while sections.iter().any(|s|s["name"]==format!("Section {i}")) {i+=1;}
            sections.push(json!({"name":format!("Section {i}"),"parameters":[]}));changed=true;
        }
    });
    if changed {
        let mut next=v.doc.clone();next["nodes"][id]["controller_sections"]=json!(sections);
        let kept:std::collections::HashSet<String>=minihou_core::controller::parameters(&next["nodes"][id]).iter().filter_map(|e|e["name"].as_str().map(str::to_owned)).collect();
        if let Some(values)=next["nodes"][id]["params"].as_object_mut() {values.retain(|name,_|kept.contains(name));}
        if let Some(channels)=next["nodes"][id]["channels"].as_object_mut() {channels.retain(|name,_|kept.contains(name.split('.').next().unwrap_or("")));}
        if minihou_core::controller::validate(&next["nodes"][id]).is_ok() {
            out.push(UiAction::Edit {edit:Edit::Replace(next),continuous:false});
        }
    }
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

    #[test]
    fn outgroup_checkbox_edits_outset_enable_parameter() {
        let mut f = Fixture::new();
        f.doc = minihou_app::Document::parse(json!({"display":"outset1","nodes":{
            "box1":{"type":"box"},"outset1":{"type":"outset","inputs":{"geo":"box1"}}
        }}).to_string()).unwrap();
        f.graph = minihou_core::Graph::from_value(f.doc.value()).unwrap();
        f.selected = "outset1".into(); f.display = "outset1".into();
        let ctx = egui::Context::default(); f.theme.apply(&ctx);
        let mut actions = Actions::default();
        let mut click = None;
        for phase in 0..3 {
            let events = match (phase, click) {
                (1, Some(pos)) => vec![egui::Event::PointerMoved(pos), egui::Event::PointerButton { pos, button:egui::PointerButton::Primary, pressed:true, modifiers:egui::Modifiers::NONE }],
                (2, Some(pos)) => vec![egui::Event::PointerButton { pos, button:egui::PointerButton::Primary, pressed:false, modifiers:egui::Modifiers::NONE }],
                _ => vec![],
            };
            let input = egui::RawInput { screen_rect:Some(Rect::from_min_size(Pos2::ZERO,Vec2::new(800.,600.))), events, ..Default::default() };
            let v = f.view();
            let _ = ctx.run(input, |ctx| { egui::CentralPanel::default().show(ctx, |ui| {
                let param = minihou_core::nodes::get("outset").unwrap().params.iter().find(|p| p.name == "output_group_name").unwrap();
                let row_rect = Rect::from_min_size(ui.cursor().min, Vec2::new(ui.available_width(), 28.));
                click = Some(ParamRowLayout::compute(row_rect).label_rect.min + Vec2::new(13., 14.));
                row(ui, &v, &mut actions, "outset1", "outset", param, false, &mut vec![]);
            }); });
        }
        assert!(actions.0.iter().any(|action| matches!(action, UiAction::Edit { edit: Edit::Param { node, name, value }, .. }
            if node == "outset1" && name == "output_group" && *value == json!(true))), "actions: {:?}; click: {:?}", actions.0, click);
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
}
