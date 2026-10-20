//! Parameter value editing widgets: numbers, sliders, ramps, colours, files, and enums.
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use minihou_app::{clamp_parameter, slider_range, ParamKind};
use serde_json::{json, Value};
use crate::ui::{number_step, theme::Theme};
use super::{
    is_color_param, is_file_param, is_ramp_param, parse_color_value, serialize_color_value,
    RampData, RampFormat, RampInterpolation, RampStop, RampValue,
};

pub mod number;

pub use number::{
    apply_field_steps, boxed, drag_number, last_field_order, number_field, register_field,
    step_by_wheel, tab_target, TAB_REQUEST,
};

pub mod rich;

pub use rich::{color_widget, file_widget, ramp_widget};


/// One editing widget for a parameter value; returns true when the value changed.
pub fn widget(ui: &mut egui::Ui, kind: ParamKind, meta: Option<&Value>, value: &mut Value) -> bool {
    let width = ui.available_width();
    widget_styled(ui, kind, meta, value, None, width, None, None, None)
}

pub fn widget_with_units(ui: &mut egui::Ui, kind: ParamKind, meta: Option<&Value>, value: &mut Value, units: Option<crate::units::UnitSettings>) -> bool {
    let width = ui.available_width();
    widget_styled(ui, kind, meta, value, None, width, units, None, None)
}

pub fn render_vector_fields(
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

pub fn widget_styled(ui: &mut egui::Ui, kind: ParamKind, meta: Option<&Value>, value: &mut Value, theme: Option<&Theme>, width: f32, units: Option<crate::units::UnitSettings>, bool_label: Option<&str>, channel_menu: Option<&mut dyn FnMut(&mut egui::Ui, usize)>) -> bool {
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

    #[test]
    fn params_tab_moves_editing_to_the_next_and_previous_number_field() {
        let order = [egui::Id::new("a"), egui::Id::new("b"), egui::Id::new("c")];
        assert_eq!(tab_target(&order, order[0], 1), Some(order[1]));
        assert_eq!(tab_target(&order, order[1], -1), Some(order[0]));
        assert_eq!(tab_target(&order, order[2], 1), None);
        assert_eq!(tab_target(&order, order[0], -1), None);
        let ctx = egui::Context::default();
        let theme = Theme::dark();
        let (a, b) = (egui::Id::new("fa"), egui::Id::new("fb"));
        let (mut va, mut vb) = (1.0f64, 2.0f64);
        let frame = |ctx: &egui::Context, events: Vec<egui::Event>, va: &mut f64, vb: &mut f64| {
            let _ = ctx.run(egui::RawInput { events, ..Default::default() }, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let r = 1.0..=100.0;
                    let range = std::ops::RangeInclusive::new(*r.start(), *r.end());
                    let ra = ui.allocate_space(egui::vec2(80., 20.)).1;
                    drag_number(ui, ra, a, va, 0.1, &range, false, Align2::LEFT_CENTER, &theme, None);
                    let rb = ui.allocate_space(egui::vec2(80., 20.)).1;
                    drag_number(ui, rb, b, vb, 0.1, &range, false, Align2::LEFT_CENTER, &theme, None);
                });
            });
        };
        frame(&ctx, vec![], &mut va, &mut vb);
        frame(&ctx, vec![], &mut va, &mut vb);
        ctx.data_mut(|d| d.insert_temp(a.with("edit"), "7".to_string()));
        let tab = |shift: bool| vec![egui::Event::Key { key: egui::Key::Tab, physical_key: None, pressed: true, repeat: false, modifiers: if shift { egui::Modifiers::SHIFT } else { egui::Modifiers::NONE } }];
        frame(&ctx, tab(false), &mut va, &mut vb);
        frame(&ctx, vec![], &mut va, &mut vb);
        assert_eq!(va, 7.0); // committed on Tab
        assert!(ctx.data(|d| d.get_temp::<String>(a.with("edit"))).is_none());
        assert!(ctx.data(|d| d.get_temp::<String>(b.with("edit"))).is_some()); // next field is now editing
        frame(&ctx, tab(true), &mut va, &mut vb);
        frame(&ctx, vec![], &mut va, &mut vb);
        assert!(ctx.data(|d| d.get_temp::<String>(a.with("edit"))).is_some()); // Shift+Tab went back
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
