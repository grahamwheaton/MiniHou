use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use minihou_app::Edit;
use minihou_core::nodes;
use serde_json::Value;

use super::super::{theme::Theme, Actions, UiAction, View};

pub(crate) fn title(name: &str) -> String {
    name.split('_').map(|part| {
        let mut chars = part.chars();
        chars.next().map_or_else(String::new, |c| c.to_uppercase().to_string() + chars.as_str())
    }).collect::<Vec<_>>().join(" ")
}

pub(crate) fn value_of(v: &View<'_>, node: &str, name: &str, default: &Value) -> Value {
    v.doc["nodes"][node]["params"].get(name).unwrap_or(default).clone()
}

pub(crate) fn modified_params(v: &View<'_>, node: &str, typ: &str) -> Vec<(String, Value)> {
    nodes::get(typ).map_or_else(Vec::new, |spec| spec.params.iter().filter_map(|param| {
        let default = (param.default)();
        (value_of(v, node, param.name, &default) != default).then(|| (param.name.to_owned(), default))
    }).collect())
}

pub(crate) fn invalid_reason(meta: Option<&Value>, value: &Value) -> Option<String> {
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

pub(crate) fn node_error<'a>(v: &'a View<'_>, id: &str) -> Option<&'a str> {
    let error = v.cooking.error.as_deref()?;
    let name = error.strip_prefix("node '")?.split_once('\'')?.0;
    (name == id).then_some(error)
}

pub(crate) fn paint_text(ui: &egui::Ui, at: Pos2, text: &str, size: f32, mono: bool, color: Color32, align: Align2) {
    ui.painter().text(at, align, text, if mono { FontId::monospace(size) } else { FontId::proportional(size) }, color);
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn button(ui: &mut egui::Ui, rect: Rect, id: egui::Id, label: &str, on: bool, color: Color32, theme: &Theme, enabled: bool) -> egui::Response {
    let response = ui.interact(rect, id, if enabled { Sense::click() } else { Sense::hover() });
    ui.painter().rect(rect, 4., if on { theme.accent_bg } else { theme.input },
        Stroke::new(1.0_f32, if on { color } else if response.hovered() { theme.stroke2 } else { theme.stroke }), StrokeKind::Inside);
    paint_text(ui, rect.center(), label, 11., false,
        if !enabled { theme.faint } else if on { color } else if label == "Reset all" { theme.text } else { theme.dim }, Align2::CENTER_CENTER);
    response
}

pub(crate) fn flag_icon(ui: &egui::Ui, at: Pos2, label: &str, color: Color32) {
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
pub(crate) fn header(ui: &mut egui::Ui, v: &View<'_>, out: &mut Actions, id: &str, typ: &str, bypass: bool, locked: bool, template: bool) {
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

pub(crate) fn icon(ui: &mut egui::Ui, rect: Rect, id: egui::Id, reset: bool, on: bool, theme: &Theme) -> egui::Response {
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
