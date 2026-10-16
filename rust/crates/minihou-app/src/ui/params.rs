//! Parameters panel: Main.dc.html `.par` and Params.dc.html.
use super::{theme::Theme, Actions, UiAction, View};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use minihou_app::{clamp_parameter, metadata_kind, param_kind, param_meta, slider_range, Edit, ParamKind};
use minihou_core::{nodes, params::ParamSpec};
use serde_json::{json, Value};

const DISABLED: &str = "The Rust backend does not support it yet";

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

#[allow(clippy::too_many_arguments)]
fn row(ui: &mut egui::Ui, v: &View<'_>, out: &mut Actions, id: &str, typ: &str, param: &ParamSpec, locked: bool, errors: &mut Vec<String>) {
    let default = (param.default)();
    let old = value_of(v, id, param.name, &default);
    let mut value = old.clone();
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
    let kind = meta.map_or_else(|| param_kind(&default), |m| metadata_kind(m, &default));
    // Metadata-driven optional controls (for example Ray's distance and hit-output toggles).
    let condition_enabled = meta.and_then(|m| m["enabled_if"].as_object()).is_none_or(|conditions| {
        conditions.iter().all(|(name, expected)| {
            let fallback = nodes::get(typ).and_then(|s| s.params.iter().find(|p| p.name == name)).map(|p| (p.default)()).unwrap_or(Value::Null);
            value_of(v, id, name, &fallback) == *expected
        })
    });
    let reason = invalid_reason(meta, &old).or_else(|| node_error(v, id).and_then(|error| {
        error.to_lowercase().contains(&param.name.replace('_', " ")).then(|| error.to_owned())
    }));
    if let Some(reason) = &reason { errors.push(format!("{}: {reason}", title(param.name))); }
    let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 28.), Sense::hover());
    let layout = ParamRowLayout::compute(r);

    let enabled = param.supported && !locked && condition_enabled;
    if modified {
        ui.painter().circle_filled(
            Pos2::new(r.left() + 9., r.center().y),
            3.,
            if enabled { v.theme.accent } else { v.theme.accent.gamma_multiply(0.5) },
        );
    }

    let is_length = crate::units::is_length_param(typ, param.name);
    let units_opt = if is_length { Some(v.units) } else { None };
    let ink = if !enabled { v.theme.faint } else if modified { v.theme.text } else { v.theme.dim };
    let label_text = if is_length { format!("{} ({})", title(param.name), v.units.display.symbol()) } else { title(param.name) };

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
        paint_text(
            ui,
            Pos2::new(layout.label_rect.right(), r.center().y),
            &displayed_label,
            12.,
            false,
            ink,
            Align2::RIGHT_CENTER,
        );

        let label_sense = if is_vector && enabled { Sense::click_and_drag() } else { Sense::hover() };
        let label_response = ui.interact(layout.label_rect, ui.id().with((id, param.name, "label")), label_sense);
        let doc = meta.and_then(|m| m["doc"].as_str()).unwrap_or("");
        if !param.supported {
            label_response.clone().on_hover_text(DISABLED);
        } else if !doc.is_empty() {
            label_response.clone().on_hover_text(doc);
        } else if displayed_label != label_text {
            label_response.clone().on_hover_text(&label_text);
        }

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
            widget_styled(
                ui,
                kind,
                meta,
                &mut value,
                Some(v.theme),
                layout.control_rect.width(),
                units_opt,
                bool_label.as_deref(),
            )
        }).response;
        if !enabled {
            response.on_disabled_hover_text(if locked {
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

    if param.supported && !locked && value != old {
        let dragging = ui.ctx().input(|input| input.pointer.primary_down() || input.pointer.middle_down());
        out.push(UiAction::Edit { edit: Edit::Param { node: id.to_owned(), name: param.name.to_owned(), value }, continuous: dragging });
    }
    if modified && !locked && icon(ui, layout.reset_rect, ui.id().with((id, param.name, "reset")), true, false, v.theme)
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
    use minihou_app::modifier_inputs::{default_for, validate, KINDS};
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
        let id = (1..).map(|i| format!("input_{i}")).find(|id| !entries.iter().any(|e| e["id"] == *id)).unwrap();
        let options: Vec<_> = subtype.split(',').map(str::trim).filter(|s| !s.is_empty()).collect();
        let parsed = if default.trim().is_empty() {
            Ok(if kind == "menu" { options.first().map_or(Value::Null, |s| json!(s)) } else { default_for(&kind) })
        } else { serde_json::from_str::<Value>(&default) };
        match parsed {
            Ok(value) => {
                let mut entry = json!({"id":id,"name":draft.trim(),"kind":kind,"default":value});
                if !min.trim().is_empty() { entry["min"] = json!(min.parse::<f64>().ok()); }
                if !max.trim().is_empty() { entry["max"] = json!(max.parse::<f64>().ok()); }
                if kind == "menu" { entry["options"] = json!(options); }
                else if !subtype.trim().is_empty() { entry["subtype"] = json!(subtype.trim()); }
                let mut next = entries.clone(); next.push(entry);
                match validate(&next) {
                    Ok(()) => {
                        out.push(UiAction::Edit { edit: Edit::ModifierInputs { node: node.into(), inputs: next }, continuous: false });
                        draft.clear(); default.clear(); error.clear();
                        ui.ctx().data_mut(|d| { d.insert_persisted(key, draft); d.insert_persisted(default_key, default); });
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
    if nodes::get(typ).is_none() {
        unsupported(ui, v, id, typ);
        return;
    }
    let flags = v.flags_of(id);
    header(ui, v, out, id, typ, node.bypass, flags.locked, flags.template);
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
                for param in spec.params { ui.push_id(param.name, |ui| row(ui, v, out, id, typ, param, flags.locked, &mut errors)); }
            }
            if typ == "modifier_input" { modifier_inputs_panel(ui, v, out, id); }
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
/// A number you drag sideways to change and double-click to type, painted in `rect` (the design's mono 12 px value).
#[allow(clippy::too_many_arguments)] // painter-style helper, called from three places
fn drag_number(ui: &mut egui::Ui, rect: Rect, id: egui::Id, v: &mut f64, speed: f64, range: &std::ops::RangeInclusive<f64>, integer: bool, align: Align2, t: &Theme, units: Option<crate::units::UnitSettings>) -> bool {
    let edit_id = id.with("edit");
    let editing: Option<String> = ui.ctx().data(|d| d.get_temp(edit_id));
    let mut changed = false;
    let clamp = |x: f64| { let x = x.clamp(*range.start(), *range.end()); if integer { x.round() } else { x } };
    let anchor = match align { Align2::LEFT_CENTER => Pos2::new(rect.left() + 2., rect.center().y), Align2::RIGHT_CENTER => Pos2::new(rect.right() - 2., rect.center().y), _ => rect.center() };
    if let Some(mut text) = editing {
        let response = ui.put(rect, egui::TextEdit::singleline(&mut text).font(egui::FontId::monospace(12.)).frame(false).margin(Vec2::new(2., 0.)).id(id.with("text")));
        if !response.has_focus() && !response.lost_focus() { response.request_focus(); }
        if response.lost_focus() {
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
            let step = if let Some(u) = units {
                response.drag_delta().x as f64 * speed * u.display.metres() / u.unit_scale
            } else {
                response.drag_delta().x as f64 * speed
            };
            if step != 0. { let next = clamp(*v + step); if next != *v { *v = next; changed = true; } }
        }
        if response.double_clicked() {
            let text = if let Some(u) = units {
                u.format(*v)
            } else if integer {
                format!("{}", *v as i64)
            } else {
                format!("{v}")
            };
            ui.ctx().data_mut(|d| d.insert_temp(edit_id, text));
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

/// One editing widget for a parameter value; returns true when the value changed.
pub fn widget(ui: &mut egui::Ui, kind: ParamKind, meta: Option<&Value>, value: &mut Value) -> bool {
    let width = ui.available_width();
    widget_styled(ui, kind, meta, value, None, width, None, None)
}

pub fn widget_with_units(ui: &mut egui::Ui, kind: ParamKind, meta: Option<&Value>, value: &mut Value, units: Option<crate::units::UnitSettings>) -> bool {
    let width = ui.available_width();
    widget_styled(ui, kind, meta, value, None, width, units, None)
}

fn render_vector_fields(
    ui: &mut egui::Ui,
    value: &mut Value,
    n: usize,
    meta: Option<&Value>,
    theme: Option<&Theme>,
    width: f32,
    units: Option<crate::units::UnitSettings>,
) -> bool {
    let axis_labels = ["X", "Y", "Z", "W"];
    let mut v: Vec<f64> = (0..n).map(|i| value.get(i).and_then(Value::as_f64).unwrap_or(0.)).collect();
    let mut changed = false;
    let spacing = 4.0_f32;
    let total_spacing = spacing * (n as f32 - 1.0);
    let axis_width = ((width - total_spacing) / n as f32).max(22.0);
    let range = meta.and_then(|m| m["min"].as_f64()).unwrap_or(f64::NEG_INFINITY)..=meta.and_then(|m| m["max"].as_f64()).unwrap_or(f64::INFINITY);

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
                changed |= drag_number(
                    ui,
                    Rect::from_min_max(Pos2::new(inner.left() + 12., inner.top()), inner.max),
                    ui.id().with(("v", i, outer.min.x as i32, outer.min.y as i32)),
                    x,
                    0.01,
                    &range,
                    false,
                    Align2::LEFT_CENTER,
                    t,
                    units,
                );
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

fn widget_styled(ui: &mut egui::Ui, kind: ParamKind, meta: Option<&Value>, value: &mut Value, theme: Option<&Theme>, width: f32, units: Option<crate::units::UnitSettings>, bool_label: Option<&str>) -> bool {
    if let Some(meta) = meta {
        if meta["kind"] == "enum" {
            let before = value.clone();
            let combo_w = width.min(180.0).max(50.0);
            egui::ComboBox::from_id_salt("enum").width(combo_w).selected_text(value.as_str().unwrap_or(""))
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
            let s_range = slider_range(meta).or_else(|| {
                if meta.get("doc").and_then(Value::as_str) == Some("Multiplies scale") {
                    Some(0.0..=10.0)
                } else {
                    None
                }
            });
            let box_w: f32 = if units.is_some() { 84. } else { 68. };
            let drag_width = if integer {
                if theme.is_some() { width - 32. } else { width }
            } else if s_range.is_some() {
                box_w.min((width - 24.).max(36.))
            } else {
                width
            };
            let range = meta["min"].as_f64().or_else(|| if s_range.is_some() { Some(0.0) } else { None }).unwrap_or(f64::NEG_INFINITY)..=meta["max"].as_f64().unwrap_or(f64::INFINITY);
            if let (Some(t), false) = (theme, integer) {
                let (outer, inner) = boxed(ui, drag_width.max(24.), t);
                let drag_speed = s_range.as_ref().map_or(0.01, |r| ((r.end() - r.start()) / 400.).clamp(0.001, 1.));
                changed |= drag_number(ui, inner, ui.id().with(("num", meta["default"].to_string(), outer.min.x as i32, outer.min.y as i32)), &mut v, drag_speed, &range, false, Align2::RIGHT_CENTER, t, units);
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
                let (outer, inner) = boxed(ui, width.max(40.), t);
                let p = ui.painter();
                let c = outer.center().y;
                p.add(egui::Shape::line(vec![Pos2::new(outer.left() + 9., c - 3.), Pos2::new(outer.left() + 6., c), Pos2::new(outer.left() + 9., c + 3.)], Stroke::new(1.3_f32, t.dim)));
                p.add(egui::Shape::line(vec![Pos2::new(outer.right() - 9., c - 3.), Pos2::new(outer.right() - 6., c), Pos2::new(outer.right() - 9., c + 3.)], Stroke::new(1.3_f32, t.dim)));
                let range = meta.and_then(|m| m["min"].as_f64()).unwrap_or(f64::NEG_INFINITY)..=meta.and_then(|m| m["max"].as_f64()).unwrap_or(f64::INFINITY);
                let mut x = v as f64;
                let mut hit = ui.interact(Rect::from_min_size(outer.min, Vec2::new(16., 22.)), ui.id().with(("dec", outer.min.x as i32, outer.min.y as i32)), Sense::click()).clicked().then_some(x - 1.);
                if hit.is_none() { hit = ui.interact(Rect::from_min_size(Pos2::new(outer.right() - 16., outer.top()), Vec2::new(16., 22.)), ui.id().with(("inc", outer.min.x as i32, outer.min.y as i32)), Sense::click()).clicked().then_some(x + 1.); }
                let mut moved = false;
                if let Some(n) = hit { x = n.clamp(*range.start(), *range.end()); moved = true; }
                moved |= drag_number(ui, inner.shrink2(Vec2::new(16., 0.)), ui.id().with(("int", outer.min.x as i32, outer.min.y as i32)), &mut x, 0.1, &range, true, Align2::CENTER_CENTER, t, None);
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
                let (outer, inner) = boxed(ui, width.max(24.), t);
                drag_number(ui, inner, ui.id().with(("f_num", outer.min.x as i32, outer.min.y as i32)), &mut v, 0.01, &(f64::NEG_INFINITY..=f64::INFINITY), false, Align2::RIGHT_CENTER, t, units)
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
            render_vector_fields(ui, value, n, meta, theme, width, units)
        }
        ParamKind::Unsupported => {
            if let Some(arr) = value.as_array().filter(|a| (2..=4).contains(&a.len()) && a.iter().all(Value::is_number)) {
                render_vector_fields(ui, value, arr.len(), meta, theme, width, units)
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
