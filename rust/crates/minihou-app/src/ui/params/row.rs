use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use minihou_app::{metadata_kind, param_kind, param_meta, slider_range, Edit, ParamKind};
use minihou_core::{nodes, params::ParamSpec};
use serde_json::{json, Value};

use super::super::{Actions, UiAction, View};
use super::header::{flag_icon, icon, invalid_reason, node_error, paint_text, title, value_of};
use super::output_groups::{is_output_group_field, output_group_label, output_group_toggle, target_field};
use super::param_values::{is_color_param, is_file_param, is_ramp_param};
use super::pick::{attribute_field, is_input_attribute_field, pick_domain_for, pick_field};
use super::value_links::{input_attribute_names, input_group_names, parameter_menu, vector_channel_menu};
use super::widgets::{color_widget, file_widget, ramp_widget, widget_styled};

const DISABLED: &str = "The Rust backend does not support it yet";

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
        let fields_right = reset_rect.left() - 8.0;
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

#[cfg(test)]
mod layout_tests {
    use super::*;

    #[test]
    fn edges_row_has_gaps_between_controls() {
        let row = Rect::from_min_size(Pos2::ZERO, Vec2::new(420., 28.));
        let layout = ParamRowLayout::compute(row);
        assert!(layout.label_rect.right() < layout.control_rect.left());
        assert!(layout.control_rect.right() + 7. <= layout.reset_rect.left());
        assert!(layout.reset_rect.right() + 7. <= layout.expose_rect.left());
    }
}

pub(crate) fn truncate_with_ellipsis(ui: &egui::Ui, text: &str, font: &FontId, max_w: f32) -> String {
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

#[allow(clippy::too_many_arguments)]
pub(crate) fn row(ui: &mut egui::Ui, v: &View<'_>, out: &mut Actions, id: &str, typ: &str, param: &ParamSpec, locked: bool, errors: &mut Vec<String>) {
    if typ == "wrangle" && param.name == "snippet" {
        wrangle_code_row(ui, v, out, id, param, locked, errors);
        return;
    }
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
                super::super::object_search::field(ui, &mut value, layout.control_rect.width(), v.theme)
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
            } else if kind == ParamKind::String && is_input_attribute_field(param.name, meta) && typ != "attribute_create" {
                attribute_field(ui, &mut value, layout.control_rect.width(), || input_attribute_names(v, id, param.name));
                false
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

fn wrangle_code_row(ui: &mut egui::Ui, v: &View<'_>, out: &mut Actions, id: &str, param: &ParamSpec, locked: bool, errors: &mut Vec<String>) {
    let default = (param.default)();
    let old = value_of(v, id, param.name, &default);
    let meta = param_meta("wrangle", param.name);
    let label = meta.and_then(|m| m["label"].as_str()).unwrap_or("code");
    let param_locked = v.doc["nodes"][id]["locked_params"].as_array().is_some_and(|a| a.iter().any(|x| x == param.name));
    let linked = v.doc["nodes"][id]["channels"].as_object().is_some_and(|c| c.contains_key(param.name));
    let enabled = param.supported && !locked && !param_locked && !linked;
    let modified = old != default;
    let width = ui.available_width().max(100.);
    let height_id = egui::Id::new(("wrangle_code_height", id));
    let mut height = ui.ctx().data_mut(|d| d.get_persisted::<f32>(height_id).unwrap_or(200.));
    ui.add_space(5.);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(label).strong().color(v.theme.text))
            .on_hover_text(meta.and_then(|m| m["doc"].as_str()).unwrap_or(""))
            .context_menu(|ui| parameter_menu(ui, v, out, id, param.name, &default));
        if modified && ui.add_enabled(enabled, egui::Button::new("Reset")).clicked() {
            out.push(UiAction::Edit { edit: Edit::Param { node: id.to_owned(), name: param.name.to_owned(), value: default.clone() }, continuous: false });
        }
        let exposed = minihou_app::is_exposed(v.doc, id, param.name);
        if ui.add_enabled(!locked, egui::Button::new(if exposed { "Unexpose" } else { "Expose" })).clicked() {
            out.push(UiAction::Edit { edit: minihou_app::toggle_exposed(v.doc, id, param.name), continuous: false });
        }
    });
    let mut code = old.as_str().unwrap_or_default().to_owned();
    let response = ui.add_enabled_ui(enabled, |ui| ui.add_sized([width, height],
        egui::TextEdit::multiline(&mut code).font(egui::TextStyle::Monospace).desired_rows(10).desired_width(width)))
        .inner;
    response.clone().context_menu(|ui| parameter_menu(ui, v, out, id, param.name, &default));
    if response.changed() {
        out.push(UiAction::Edit { edit: Edit::Param { node: id.to_owned(), name: param.name.to_owned(), value: json!(code) }, continuous: response.has_focus() });
    }
    let drag = ui.allocate_response(Vec2::new(width, 12.), Sense::drag());
    if drag.hovered() || drag.dragged() { ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical); }
    if drag.dragged() {
        height = (height + ui.ctx().input(|i| i.pointer.delta().y)).clamp(200., 1000.);
        ui.ctx().data_mut(|d| d.insert_persisted(height_id, height));
    }
    let center = drag.rect.center();
    for offset in [-3., 0., 3.] {
        ui.painter().line_segment([Pos2::new(center.x - 12., center.y + offset), Pos2::new(center.x + 12., center.y + offset)], Stroke::new(1., v.theme.dim));
    }
    if let Some(reason) = invalid_reason(meta, &old).or_else(|| node_error(v, id).map(str::to_owned)) {
        errors.push(format!("{label}: {reason}"));
    }
    ui.add_space(6.);
}
