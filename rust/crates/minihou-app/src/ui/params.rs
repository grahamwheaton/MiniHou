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

/// Parse a color value into RGBA `[f32; 4]`, number of components (3 or 4), and whether it was a hex string.
pub fn parse_color_value(value: &Value) -> Option<([f32; 4], usize, bool)> {
    if let Some(arr) = value.as_array() {
        if arr.len() == 3 && arr.iter().all(Value::is_number) {
            let r = arr[0].as_f64()? as f32;
            let g = arr[1].as_f64()? as f32;
            let b = arr[2].as_f64()? as f32;
            return Some(([r, g, b, 1.0], 3, false));
        } else if arr.len() == 4 && arr.iter().all(Value::is_number) {
            let r = arr[0].as_f64()? as f32;
            let g = arr[1].as_f64()? as f32;
            let b = arr[2].as_f64()? as f32;
            let a = arr[3].as_f64()? as f32;
            return Some(([r, g, b, a], 4, false));
        }
    } else if let Some(s) = value.as_str() {
        let s = s.trim();
        if let Some(hex_part) = s.strip_prefix('#') {
            if hex_part.len() == 6 {
                if let (Ok(r), Ok(g), Ok(b)) = (
                    u8::from_str_radix(&hex_part[0..2], 16),
                    u8::from_str_radix(&hex_part[2..4], 16),
                    u8::from_str_radix(&hex_part[4..6], 16),
                ) {
                    return Some(([r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0], 3, true));
                }
            } else if hex_part.len() == 8 {
                if let (Ok(r), Ok(g), Ok(b), Ok(a)) = (
                    u8::from_str_radix(&hex_part[0..2], 16),
                    u8::from_str_radix(&hex_part[2..4], 16),
                    u8::from_str_radix(&hex_part[4..6], 16),
                    u8::from_str_radix(&hex_part[6..8], 16),
                ) {
                    return Some(([r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a as f32 / 255.0], 4, true));
                }
            } else if hex_part.len() == 3 {
                if let (Ok(r), Ok(g), Ok(b)) = (
                    u8::from_str_radix(&hex_part[0..1], 16),
                    u8::from_str_radix(&hex_part[1..2], 16),
                    u8::from_str_radix(&hex_part[2..3], 16),
                ) {
                    return Some(([
                        (r * 17) as f32 / 255.0,
                        (g * 17) as f32 / 255.0,
                        (b * 17) as f32 / 255.0,
                        1.0,
                    ], 3, true));
                }
            }
        }
    }
    None
}

/// Serialize RGBA back into the original format (array of 3/4 floats or hex string).
pub fn serialize_color_value(rgba: [f32; 4], comps: usize, is_hex: bool) -> Value {
    if is_hex {
        let r = (rgba[0] * 255.0).clamp(0.0, 255.0).round() as u8;
        let g = (rgba[1] * 255.0).clamp(0.0, 255.0).round() as u8;
        let b = (rgba[2] * 255.0).clamp(0.0, 255.0).round() as u8;
        if comps == 4 {
            let a = (rgba[3] * 255.0).clamp(0.0, 255.0).round() as u8;
            json!(format!("#{r:02x}{g:02x}{b:02x}{a:02x}"))
        } else {
            json!(format!("#{r:02x}{g:02x}{b:02x}"))
        }
    } else if comps == 4 {
        json!([rgba[0], rgba[1], rgba[2], rgba[3]])
    } else {
        json!([rgba[0], rgba[1], rgba[2]])
    }
}

/// Check if a parameter represents a colour.
pub fn is_color_param(param_name: &str, meta: Option<&Value>, value: &Value) -> bool {
    if meta.and_then(|m| m["kind"].as_str()) == Some("color") {
        return true;
    }
    let name_is_color = param_name == "color"
        || param_name == "color2"
        || param_name == "colour"
        || param_name.ends_with("_color")
        || param_name.ends_with("_colour")
        || param_name == "tint"
        || param_name == "diffuse"
        || param_name == "specular";
    if name_is_color && parse_color_value(value).is_some() {
        return true;
    }
    if param_name.is_empty() && value.as_str().is_some_and(|s| s.starts_with('#')) && parse_color_value(value).is_some() {
        return true;
    }
    false
}

/// Check if a parameter represents a file path.
pub fn is_file_param(param_name: &str, meta: Option<&Value>) -> bool {
    if let Some(m) = meta {
        if m["kind"].as_str() == Some("file") {
            return true;
        }
        if m["ui"].as_str().is_some_and(|ui| ui.starts_with("file:")) {
            return true;
        }
    }
    param_name == "file"
        || param_name == "path"
        || param_name == "rule_file"
        || param_name == "image_file"
        || param_name.ends_with("_file")
        || param_name.ends_with("_path")
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RampInterpolation {
    Linear,
    Constant,
    Smooth,
}

impl RampInterpolation {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Linear => "linear",
            Self::Constant => "constant",
            Self::Smooth => "smooth",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "constant" | "step" | "hold" => Self::Constant,
            "smooth" | "catmull-rom" | "catmullrom" | "monotone_cubic" | "ease" | "bezier" => Self::Smooth,
            _ => Self::Linear,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum RampValue {
    Float(f32),
    Color([f32; 3]),
    ColorAlpha([f32; 4]),
}

impl RampValue {
    pub fn as_float(&self) -> f32 {
        match self {
            Self::Float(f) => *f,
            Self::Color(c) => (c[0] + c[1] + c[2]) / 3.0,
            Self::ColorAlpha(c) => (c[0] + c[1] + c[2]) / 3.0,
        }
    }

    pub fn as_rgba(&self) -> [f32; 4] {
        match self {
            Self::Color(c) => [c[0], c[1], c[2], 1.0],
            Self::ColorAlpha(c) => *c,
            Self::Float(f) => [*f, *f, *f, 1.0],
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RampStop {
    pub pos: f32,
    pub value: RampValue,
    pub interp: Option<RampInterpolation>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RampFormat {
    StopsObject,       // {"stops": [...], "interpolation": "..."}
    KeysValuesObject,  // {"keys": [...], "values": [...], "basis": "..."}
    ArrayOfPairs,      // [[pos, val], ...]
    ArrayOfStops,      // [{"pos": ..., "value"/"color": ...}, ...]
}

#[derive(Clone, Debug, PartialEq)]
pub struct RampData {
    pub is_color: bool,
    pub stops: Vec<RampStop>,
    pub interpolation: RampInterpolation,
    pub format: RampFormat,
}

impl RampData {
    pub fn is_ramp_value(value: &Value) -> bool {
        if let Some(obj) = value.as_object() {
            if obj.contains_key("stops") || (obj.contains_key("keys") && obj.contains_key("values")) {
                return true;
            }
        } else if let Some(arr) = value.as_array() {
            if !arr.is_empty() {
                if let Some(first) = arr.first() {
                    if first.as_array().is_some_and(|pair| pair.len() == 2 && pair[0].is_number()) {
                        return true;
                    }
                    if first.as_object().is_some_and(|item| item.contains_key("pos") || item.contains_key("position")) {
                        return true;
                    }
                }
            }
        }
        false
    }

    pub fn from_value(value: &Value, is_color_hint: bool) -> Self {
        let mut stops = Vec::new();
        let mut interpolation = RampInterpolation::Linear;
        let mut format = RampFormat::StopsObject;
        let mut is_color = is_color_hint;

        if let Some(obj) = value.as_object() {
            if let Some(interp_str) = obj.get("interpolation").or_else(|| obj.get("basis")).and_then(Value::as_str) {
                interpolation = RampInterpolation::from_str(interp_str);
            }
            if let Some(stops_arr) = obj.get("stops").and_then(Value::as_array) {
                format = RampFormat::StopsObject;
                for item in stops_arr {
                    if let Some(stop) = Self::parse_stop_object(item, &mut is_color) {
                        stops.push(stop);
                    }
                }
            } else if let (Some(keys), Some(vals)) = (obj.get("keys").and_then(Value::as_array), obj.get("values").and_then(Value::as_array)) {
                format = RampFormat::KeysValuesObject;
                for (k, v) in keys.iter().zip(vals) {
                    if let Some(pos) = k.as_f64().map(|f| f as f32) {
                        let r_val = Self::parse_ramp_value(v, &mut is_color);
                        stops.push(RampStop { pos: pos.clamp(0.0, 1.0), value: r_val, interp: None });
                    }
                }
            }
        } else if let Some(arr) = value.as_array() {
            if let Some(first) = arr.first() {
                if first.as_array().is_some() {
                    format = RampFormat::ArrayOfPairs;
                    for item in arr {
                        if let Some(pair) = item.as_array() {
                            if pair.len() >= 2 {
                                if let Some(pos) = pair[0].as_f64().map(|f| f as f32) {
                                    let r_val = Self::parse_ramp_value(&pair[1], &mut is_color);
                                    stops.push(RampStop { pos: pos.clamp(0.0, 1.0), value: r_val, interp: None });
                                }
                            }
                        }
                    }
                } else if first.as_object().is_some() {
                    format = RampFormat::ArrayOfStops;
                    for item in arr {
                        if let Some(stop) = Self::parse_stop_object(item, &mut is_color) {
                            stops.push(stop);
                        }
                    }
                }
            }
        }

        let mut data = Self {
            is_color,
            stops,
            interpolation,
            format,
        };
        data.ensure_valid_defaults();
        data.sort_stops();
        data
    }

    fn parse_ramp_value(v: &Value, is_color: &mut bool) -> RampValue {
        if let Some(arr) = v.as_array() {
            if arr.len() == 4 && arr.iter().all(Value::is_number) {
                *is_color = true;
                return RampValue::ColorAlpha([
                    arr[0].as_f64().unwrap_or(0.) as f32,
                    arr[1].as_f64().unwrap_or(0.) as f32,
                    arr[2].as_f64().unwrap_or(0.) as f32,
                    arr[3].as_f64().unwrap_or(1.) as f32,
                ]);
            } else if arr.len() == 3 && arr.iter().all(Value::is_number) {
                *is_color = true;
                return RampValue::Color([
                    arr[0].as_f64().unwrap_or(0.) as f32,
                    arr[1].as_f64().unwrap_or(0.) as f32,
                    arr[2].as_f64().unwrap_or(0.) as f32,
                ]);
            }
        } else if let Some(n) = v.as_f64() {
            return RampValue::Float(n as f32);
        } else if let Some(s) = v.as_str() {
            if let Some((rgba, _, _)) = parse_color_value(&Value::String(s.to_owned())) {
                *is_color = true;
                return RampValue::Color([rgba[0], rgba[1], rgba[2]]);
            }
        }
        if *is_color {
            RampValue::Color([0.0, 0.0, 0.0])
        } else {
            RampValue::Float(0.0)
        }
    }

    fn parse_stop_object(item: &Value, is_color: &mut bool) -> Option<RampStop> {
        let obj = item.as_object()?;
        let pos = obj.get("pos")
            .or_else(|| obj.get("position"))
            .or_else(|| obj.get("t"))?
            .as_f64()? as f32;
        let interp = obj.get("interpolation")
            .or_else(|| obj.get("interp"))
            .and_then(Value::as_str)
            .map(RampInterpolation::from_str);
        let value = if let Some(c) = obj.get("color").or_else(|| obj.get("colour")) {
            Self::parse_ramp_value(c, is_color)
        } else if let Some(v) = obj.get("value").or_else(|| obj.get("val")) {
            Self::parse_ramp_value(v, is_color)
        } else {
            Self::parse_ramp_value(&Value::Null, is_color)
        };
        Some(RampStop { pos: pos.clamp(0.0, 1.0), value, interp })
    }

    fn ensure_valid_defaults(&mut self) {
        if self.stops.is_empty() {
            if self.is_color {
                self.stops = vec![
                    RampStop { pos: 0.0, value: RampValue::Color([0.0, 0.0, 0.0]), interp: None },
                    RampStop { pos: 1.0, value: RampValue::Color([1.0, 1.0, 1.0]), interp: None },
                ];
            } else {
                self.stops = vec![
                    RampStop { pos: 0.0, value: RampValue::Float(0.0), interp: None },
                    RampStop { pos: 1.0, value: RampValue::Float(1.0), interp: None },
                ];
            }
        }
    }

    pub fn to_value(&self) -> Value {
        match self.format {
            RampFormat::ArrayOfPairs => {
                let pairs: Vec<Value> = self.stops.iter().map(|s| {
                    let v = match &s.value {
                        RampValue::Float(f) => json!(*f),
                        RampValue::Color(c) => json!([c[0], c[1], c[2]]),
                        RampValue::ColorAlpha(c) => json!([c[0], c[1], c[2], c[3]]),
                    };
                    json!([s.pos, v])
                }).collect();
                json!(pairs)
            }
            RampFormat::ArrayOfStops => {
                let stops: Vec<Value> = self.stops.iter().map(|s| {
                    let mut map = serde_json::Map::new();
                    map.insert("pos".into(), json!(s.pos));
                    match &s.value {
                        RampValue::Float(f) => { map.insert("value".into(), json!(*f)); }
                        RampValue::Color(c) => { map.insert("color".into(), json!([c[0], c[1], c[2]])); }
                        RampValue::ColorAlpha(c) => { map.insert("color".into(), json!([c[0], c[1], c[2], c[3]])); }
                    }
                    if let Some(interp) = &s.interp {
                        map.insert("interpolation".into(), json!(interp.as_str()));
                    }
                    Value::Object(map)
                }).collect();
                json!(stops)
            }
            RampFormat::KeysValuesObject => {
                let keys: Vec<f32> = self.stops.iter().map(|s| s.pos).collect();
                let values: Vec<Value> = self.stops.iter().map(|s| {
                    match &s.value {
                        RampValue::Float(f) => json!(*f),
                        RampValue::Color(c) => json!([c[0], c[1], c[2]]),
                        RampValue::ColorAlpha(c) => json!([c[0], c[1], c[2], c[3]]),
                    }
                }).collect();
                json!({
                    "keys": keys,
                    "values": values,
                    "basis": self.interpolation.as_str()
                })
            }
            RampFormat::StopsObject => {
                let stops: Vec<Value> = self.stops.iter().map(|s| {
                    let mut map = serde_json::Map::new();
                    map.insert("pos".into(), json!(s.pos));
                    match &s.value {
                        RampValue::Float(f) => { map.insert("value".into(), json!(*f)); }
                        RampValue::Color(c) => { map.insert("color".into(), json!([c[0], c[1], c[2]])); }
                        RampValue::ColorAlpha(c) => { map.insert("color".into(), json!([c[0], c[1], c[2], c[3]])); }
                    }
                    if let Some(interp) = &s.interp {
                        map.insert("interpolation".into(), json!(interp.as_str()));
                    }
                    Value::Object(map)
                }).collect();
                json!({
                    "stops": stops,
                    "interpolation": self.interpolation.as_str()
                })
            }
        }
    }

    pub fn sort_stops(&mut self) {
        self.stops.sort_by(|a, b| a.pos.partial_cmp(&b.pos).unwrap_or(std::cmp::Ordering::Equal));
    }

    pub fn eval_float(&self, t: f32) -> f32 {
        if self.stops.is_empty() { return 0.0; }
        if self.stops.len() == 1 || t <= self.stops[0].pos {
            return self.stops[0].value.as_float();
        }
        let last = self.stops.last().unwrap();
        if t >= last.pos {
            return last.value.as_float();
        }
        for i in 0..self.stops.len() - 1 {
            let s0 = &self.stops[i];
            let s1 = &self.stops[i + 1];
            if t >= s0.pos && t <= s1.pos {
                let u = ((t - s0.pos) / (s1.pos - s0.pos).max(1e-6)).clamp(0.0, 1.0);
                let interp = s0.interp.unwrap_or(self.interpolation);
                let v0 = s0.value.as_float();
                let v1 = s1.value.as_float();
                return match interp {
                    RampInterpolation::Constant => v0,
                    RampInterpolation::Linear => (1.0 - u) * v0 + u * v1,
                    RampInterpolation::Smooth => {
                        let s = u * u * (3.0 - 2.0 * u);
                        (1.0 - s) * v0 + s * v1
                    }
                };
            }
        }
        0.0
    }

    pub fn eval_color(&self, t: f32) -> [f32; 4] {
        if self.stops.is_empty() { return [0.0, 0.0, 0.0, 1.0]; }
        if self.stops.len() == 1 || t <= self.stops[0].pos {
            return self.stops[0].value.as_rgba();
        }
        let last = self.stops.last().unwrap();
        if t >= last.pos {
            return last.value.as_rgba();
        }
        for i in 0..self.stops.len() - 1 {
            let s0 = &self.stops[i];
            let s1 = &self.stops[i + 1];
            if t >= s0.pos && t <= s1.pos {
                let u = ((t - s0.pos) / (s1.pos - s0.pos).max(1e-6)).clamp(0.0, 1.0);
                let interp = s0.interp.unwrap_or(self.interpolation);
                let c0 = s0.value.as_rgba();
                let c1 = s1.value.as_rgba();
                let factor = match interp {
                    RampInterpolation::Constant => 0.0,
                    RampInterpolation::Linear => u,
                    RampInterpolation::Smooth => u * u * (3.0 - 2.0 * u),
                };
                return [
                    (1.0 - factor) * c0[0] + factor * c1[0],
                    (1.0 - factor) * c0[1] + factor * c1[1],
                    (1.0 - factor) * c0[2] + factor * c1[2],
                    (1.0 - factor) * c0[3] + factor * c1[3],
                ];
            }
        }
        [0.0, 0.0, 0.0, 1.0]
    }

    pub fn add_stop(&mut self, pos: f32) -> usize {
        let pos = pos.clamp(0.0, 1.0);
        let val = if self.is_color {
            let c = self.eval_color(pos);
            RampValue::Color([c[0], c[1], c[2]])
        } else {
            RampValue::Float(self.eval_float(pos))
        };
        let new_stop = RampStop { pos, value: val, interp: None };
        self.stops.push(new_stop);
        self.sort_stops();
        self.stops.iter().position(|s| (s.pos - pos).abs() < 1e-4).unwrap_or(0)
    }

    pub fn delete_stop(&mut self, index: usize) -> bool {
        if self.stops.len() <= 1 || index >= self.stops.len() {
            return false;
        }
        self.stops.remove(index);
        true
    }

    pub fn move_stop(&mut self, index: usize, new_pos: f32) -> usize {
        if index >= self.stops.len() { return index; }
        let new_pos = new_pos.clamp(0.0, 1.0);
        self.stops[index].pos = new_pos;
        let target_stop = self.stops[index].clone();
        self.sort_stops();
        self.stops.iter().position(|s| s.pos == target_stop.pos && s.value == target_stop.value).unwrap_or(0)
    }
}

pub fn is_ramp_param(param_name: &str, meta: Option<&Value>, value: &Value) -> bool {
    if let Some(m) = meta {
        if let Some(k) = m["kind"].as_str() {
            if k == "float_ramp" || k == "color_ramp" || k == "ramp" {
                return true;
            }
        }
    }
    if param_name == "ramp"
        || param_name == "float_ramp"
        || param_name == "color_ramp"
        || param_name.ends_with("_ramp")
    {
        return true;
    }
    RampData::is_ramp_value(value)
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
    let is_ramp = is_ramp_param(param.name, meta, &value);
    let is_color = !is_ramp && is_color_param(param.name, meta, &value);
    let is_file = !is_ramp && !is_color && is_file_param(param.name, meta);

    let row_height = if is_ramp { 76.0 } else { 28.0 };
    let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), row_height), Sense::hover());
    let layout = ParamRowLayout::compute(r);

    let enabled = param.supported && !locked && condition_enabled;
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
        let label_pos_y = if is_ramp { r.top() + 14. } else { r.center().y };
        paint_text(
            ui,
            Pos2::new(layout.label_rect.right(), label_pos_y),
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
            if is_ramp {
                ramp_widget(ui, &mut value, meta, v.theme, layout.control_rect.width(), id, param.name)
            } else if is_color {
                color_widget(ui, &mut value, meta, Some(v.theme), layout.control_rect.width(), id, param.name)
            } else if is_file {
                file_widget(ui, &mut value, meta, typ, param.name, layout.control_rect.width())
            } else {
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
            }
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

fn color_widget(
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

fn file_widget(
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

fn ramp_widget(
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
        assert_eq!(ser_rgb, rgb_val);

        // 4-component RGBA array
        let rgba_val = json!([0.1, 0.2, 0.3, 0.8]);
        let (rgba, comps, is_hex) = parse_color_value(&rgba_val).expect("parse rgba");
        assert_eq!(comps, 4);
        assert!(!is_hex);
        assert!((rgba[0] - 0.1).abs() < 1e-4);
        assert!((rgba[3] - 0.8).abs() < 1e-4);
        let ser_rgba = serialize_color_value(rgba, comps, is_hex);
        assert_eq!(ser_rgba, rgba_val);

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
        assert_eq!(ser3, pairs);

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
}
