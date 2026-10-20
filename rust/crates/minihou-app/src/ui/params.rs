//! Parameters panel: Main.dc.html `.par` and Params.dc.html.
use super::{theme::Theme, Actions, UiAction, View};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use super::number_step;
use minihou_app::{clamp_parameter, metadata_kind, param_kind, param_meta, slider_range, Edit, ParamKind};
use minihou_core::{nodes, params::ParamSpec};
use serde_json::{json, Value};
use minihou_app::value_link_ui::{self, CopiedParam, PasteMode};

const DISABLED: &str = "The Rust backend does not support it yet";
pub mod sections;
pub use sections::*;
pub mod output_groups;
pub use output_groups::{is_output_group_field, output_group_label, output_group_toggle, target_field, TARGET_TEAL};
pub mod pick;
pub use pick::{domain_edit_mode, is_input_group_field, pick_domain, pick_domain_for, pick_field, PICK_PURPLE, SELECT_ORANGE};
pub mod widgets;
pub use widgets::{boxed, color_widget, drag_number, file_widget, number_field, ramp_widget, render_vector_fields, widget, widget_styled, widget_with_units};
fn clipboard_id() -> egui::Id { egui::Id::new("value_link_parameter_clipboard") }

fn input_group_names(v: &View<'_>, id: &str, field: &str, mode: super::SelectionMode) -> Vec<String> {
    let Some((_, node)) = v.graph.nodes.iter().find(|(name, _)| name == id) else { return Vec::new(); };
    let port_index = if field == "group2" { 1 } else { 0 };
    let Some(port) = nodes::get(&node.node_type).and_then(|spec| spec.inputs.get(port_index)) else { return Vec::new(); };
    let Some(source) = node.inputs.iter().find(|(name, _)| name == port.name).and_then(|(_, refs)| refs.first()) else { return Vec::new(); };
    v.graph.cook(Some(source)).map(|g| pick::input_groups(&g, mode)).unwrap_or_default()
}

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

pub(crate) fn paint_text(ui: &egui::Ui, at: Pos2, text: &str, size: f32, mono: bool, color: Color32, align: Align2) {
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
                let (clicked, hovered) = pick_field(ui, &mut value, layout.control_rect.width(), active, enabled,
                    || input_group_names(v, id, param.name, pick_mode.unwrap()));
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
