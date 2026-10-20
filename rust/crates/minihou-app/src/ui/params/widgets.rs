//! Parameter value editing widgets: numbers, sliders, ramps, colours, files, and enums.
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use minihou_app::{clamp_parameter, slider_range, ParamKind};
use serde_json::{json, Value};
use crate::ui::{number_step, theme::Theme};
use super::{
    is_color_param, is_file_param, is_ramp_param, parse_color_value, serialize_color_value,
    RampData, RampFormat, RampInterpolation, RampStop, RampValue,
};

/// A 22 px value box in the design's style (input fill, 1 px stroke, 4 px radius); returns the outer and inner rects.
pub fn boxed(ui: &mut egui::Ui, width: f32, t: &Theme) -> (Rect, Rect) {
    let (r, _) = ui.allocate_exact_size(Vec2::new(width, 22.), Sense::hover());
    let (bg, stroke) = if ui.is_enabled() {
        (t.input, t.stroke)
    } else {
        (t.input.gamma_multiply(0.5), t.stroke.gamma_multiply(0.5))
    };
    ui.painter().rect(r, 4., bg, Stroke::new(1.0_f32, stroke), StrokeKind::Inside);
    (r, r.shrink2(Vec2::new(2., 1.)))
}

pub const TAB_REQUEST: egui::Id = egui::Id::NULL;

/// Number fields drawn this pass in layout order (the previous pass's list is what Tab navigates).
pub fn register_field(ctx: &egui::Context, id: egui::Id, enabled: bool) {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| {
        let cur = d.get_temp_mut_or_default::<(u64, Vec<egui::Id>, Vec<egui::Id>)>(egui::Id::new("field_order"));
        if cur.0 != pass { cur.0 = pass; cur.2 = std::mem::take(&mut cur.1); }
        if enabled { cur.1.push(id); }
    });
}

pub fn last_field_order(ctx: &egui::Context) -> Vec<egui::Id> {
    // mid-pass the current list is partial, so navigate by the previous pass's complete one
    ctx.data(|d| d.get_temp::<(u64, Vec<egui::Id>, Vec<egui::Id>)>(egui::Id::new("field_order"))).map(|(_, _, prev)| prev).unwrap_or_default()
}

/// The field Tab (`dir` 1) or Shift+Tab (-1) moves to from `current`; none past either end.
pub fn tab_target(order: &[egui::Id], current: egui::Id, dir: i32) -> Option<egui::Id> {
    let i = order.iter().position(|x| *x == current)? as i32 + dir;
    usize::try_from(i).ok().and_then(|i| order.get(i)).copied()
}

/// A number you drag sideways to change and click to type, painted in `rect` (the design's mono 12 px value).
#[allow(clippy::too_many_arguments)] // painter-style helper, called from three places
pub fn drag_number(ui: &mut egui::Ui, rect: Rect, id: egui::Id, v: &mut f64, speed: f64, range: &std::ops::RangeInclusive<f64>, integer: bool, align: Align2, t: &Theme, units: Option<crate::units::UnitSettings>) -> bool {
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
pub fn number_field(ui: &mut egui::Ui, width: f32, id: egui::Id, v: &mut f64, speed: f64, range: &std::ops::RangeInclusive<f64>, integer: bool, soft: Option<(f64, f64)>, align: Align2, t: &Theme, units: Option<crate::units::UnitSettings>) -> (bool, Rect) {
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

pub fn step_by_wheel(ui: &egui::Ui, rect: Rect, id: egui::Id) -> i32 {
    let editing = ui.ctx().data(|d| d.get_temp::<String>(id.with("edit")).is_some());
    number_step::wheel(ui, rect, id, editing)
}

#[allow(clippy::too_many_arguments)]
pub fn apply_field_steps(ui: &egui::Ui, id: egui::Id, v: &mut f64, steps: i32, range: &std::ops::RangeInclusive<f64>, integer: bool, soft: Option<(f64, f64)>, units: Option<crate::units::UnitSettings>) -> bool {
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
