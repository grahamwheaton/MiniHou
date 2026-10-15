//! Parameters panel (design: Main.dc.html `.par` and Params.dc.html): node header with flags, groups, parameter rows
//! (label, slider / drag value / combo / vec3, reset, expose). Baseline: the original functional panel.
use super::{Actions, UiAction, View};
use eframe::egui;
use minihou_app::{clamp_parameter, metadata_kind, param_kind, param_meta, slider_range, Edit, ParamKind};
use minihou_core::nodes;
use serde_json::{json, Value};

pub fn show(ui: &mut egui::Ui, v: &View, out: &mut Actions) {
    ui.heading(v.selected);
    let selected = v.selected;
    let node = v.graph.nodes.iter().find(|(id, _)| id == selected).map(|(_, n)| (n.node_type.clone(), n.bypass));
    let Some((typ, mut bypass)) = node else { return };
    ui.label(&typ);
    if ui.checkbox(&mut bypass, "Mute (bypass)").changed() {
        out.push(UiAction::Edit { edit: Edit::Bypass { node: selected.to_owned(), flag: bypass }, continuous: false });
    }
    let dragging = ui.ctx().input(|i| i.pointer.primary_down());
    egui::ScrollArea::vertical().show(ui, |ui| {
        let Some(spec) = nodes::get(&typ) else {
            ui.colored_label(egui::Color32::RED, "Node type is not supported by the Rust backend yet");
            return;
        };
        for param in spec.params {
            ui.push_id(param.name, |ui| {
                let meta = param_meta(&typ, param.name);
                let doc = meta.and_then(|m| m["doc"].as_str()).unwrap_or("");
                ui.label(param.name).on_hover_text(doc);
                let default = (param.default)();
                let mut value = v.doc["nodes"][selected]["params"].get(param.name).unwrap_or(&default).clone();
                let response = ui.add_enabled_ui(param.supported, |ui| {
                    widget(ui, meta.map_or_else(|| param_kind(&default), |m| metadata_kind(m, &default)), meta, &mut value)
                }).response;
                let response = response.on_hover_text(doc);
                if !param.supported { response.on_hover_text("The Rust backend does not support it yet").on_disabled_hover_text("The Rust backend does not support it yet"); }
                // Response.changed is not propagated by a scope; inspect the value instead.
                let old = v.doc["nodes"][selected]["params"].get(param.name).unwrap_or(&default);
                if param.supported && &value != old {
                    out.push(UiAction::Edit { edit: Edit::Param { node: selected.to_owned(), name: param.name.into(), value }, continuous: dragging });
                }
            });
        }
    });
}

/// One editing widget for a parameter value; returns true when the value changed.
pub fn widget(ui: &mut egui::Ui, kind: ParamKind, meta: Option<&Value>, value: &mut Value) -> bool {
    if let Some(meta) = meta {
        if meta["kind"] == "enum" {
            let before = value.clone();
            egui::ComboBox::from_id_salt("enum").selected_text(value.as_str().unwrap_or(""))
                .show_ui(ui, |ui| {
                    if let Some(options) = meta["options"].as_array() {
                        for option in options { ui.selectable_value(value, option.clone(), option.as_str().unwrap_or("")); }
                    }
                });
            return *value != before;
        }
        if matches!(kind, ParamKind::Integer | ParamKind::Float) {
            let mut v = value.as_f64().unwrap_or(0.);
            let integer = kind == ParamKind::Integer;
            let mut changed = false;
            if let Some(range) = slider_range(meta) {
                let mut slider = egui::Slider::new(&mut v, range).show_value(false).clamping(egui::SliderClamping::Edits);
                if integer { slider = slider.integer(); }
                changed |= ui.add(slider).changed();
            }
            changed |= ui.add(egui::DragValue::new(&mut v).speed(if integer { 1. } else { 0.01 })
                .range(meta["min"].as_f64().unwrap_or(f64::NEG_INFINITY)..=meta["max"].as_f64().unwrap_or(f64::INFINITY))).changed();
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
            if ui.checkbox(&mut v, "").changed() { *value = json!(v); true } else { false }
        }
        ParamKind::Integer => {
            let mut v = value.as_i64().or_else(|| value.as_f64().map(|f| f as i64)).unwrap_or(0);
            if ui.add(egui::DragValue::new(&mut v).speed(1)).changed() { *value = json!(v); true } else { false }
        }
        ParamKind::Float => {
            let mut v = value.as_f64().unwrap_or(0.);
            if ui.add(egui::DragValue::new(&mut v).speed(0.01)).changed() { *value = json!(v); true } else { false }
        }
        ParamKind::String => {
            let mut v = value.as_str().unwrap_or("").to_owned();
            if ui.text_edit_singleline(&mut v).changed() { *value = json!(v); true } else { false }
        }
        ParamKind::Vec3 => {
            let mut v = [0, 1, 2].map(|i| value.get(i).and_then(Value::as_f64).unwrap_or(0.));
            let mut changed = false;
            ui.horizontal(|ui| {
                for x in &mut v {
                    changed |= ui.add(egui::DragValue::new(x).speed(0.01).range(
                        meta.and_then(|m| m["min"].as_f64()).unwrap_or(f64::NEG_INFINITY)..=meta.and_then(|m| m["max"].as_f64()).unwrap_or(f64::INFINITY))).changed();
                }
            });
            if changed { *value = json!(v); }
            changed
        }
        ParamKind::Unsupported => { ui.label(value.to_string()); false }
    }
}
