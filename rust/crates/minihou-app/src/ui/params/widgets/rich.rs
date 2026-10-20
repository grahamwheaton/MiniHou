use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use serde_json::{json, Value};
use crate::ui::theme::Theme;
use super::number::{boxed, drag_number};
use super::super::{
    parse_color_value, serialize_color_value, RampData, RampInterpolation, RampValue,
};

pub fn color_widget(
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

pub fn file_widget(
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

pub fn ramp_widget(
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
