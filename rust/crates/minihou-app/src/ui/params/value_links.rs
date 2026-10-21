use eframe::egui;
use minihou_app::{
    value_link_ui::{self, CopiedParam, PasteMode},
    Edit,
};
use minihou_core::nodes;
use serde_json::Value;

use super::super::{Actions, SelectionMode, UiAction, View};
use super::pick;

pub(crate) fn clipboard_id() -> egui::Id {
    egui::Id::new("value_link_parameter_clipboard")
}

pub(crate) fn input_group_names(v: &View<'_>, id: &str, field: &str, mode: SelectionMode) -> Vec<String> {
    input_geometry(v, id, field).map(|g| pick::input_groups(&g, mode)).unwrap_or_default()
}

pub(crate) fn input_attribute_names(v: &View<'_>, id: &str, field: &str) -> Vec<String> {
    input_geometry(v, id, field).map(|g| pick::input_attributes(&g)).unwrap_or_default()
}

fn input_geometry(v: &View<'_>, id: &str, field: &str) -> Option<std::sync::Arc<minihou_core::Geometry>> {
    let (_, node) = v.graph.nodes.iter().find(|(name, _)| name == id)?;
    let port_index = if field == "group2" { 1 } else { 0 };
    let port = nodes::get(&node.node_type)?.inputs.get(port_index)?;
    let source = node.inputs.iter().find(|(name, _)| name == port.name).and_then(|(_, refs)| refs.first())?;
    v.graph.cook(Some(source)).ok()
}

pub(crate) fn open_expression(ctx: &egui::Context, doc: &Value, node: &str, path: &str) {
    let expr = doc["nodes"][node]["channels"][path].as_str().unwrap_or("").to_owned();
    ctx.data_mut(|d| {
        d.insert_temp(egui::Id::new("link_expression_open"), (node.to_owned(), path.to_owned()));
        d.insert_temp(egui::Id::new("link_expression_text"), expr);
    });
}

pub(crate) fn channel_reference_edit(node: &str, path: &str, source: &str, source_path: &str) -> Edit {
    let reference = value_link_ui::relative_path(node, source, source_path);
    Edit::Channel { node: node.into(), path: path.into(), expression: Some(format!("ch(\"{reference}\")")) }
}

pub(crate) fn vector_channel_menu(ui: &mut egui::Ui, v: &View<'_>, out: &mut Actions, node: &str, name: &str, axis: usize) {
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

pub(crate) fn parameter_menu(ui: &mut egui::Ui, v: &View<'_>, out: &mut Actions, node: &str, name: &str, default: &Value) {
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

pub(crate) fn expression_dialog(ctx: &egui::Context, v: &View<'_>, out: &mut Actions) {
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
