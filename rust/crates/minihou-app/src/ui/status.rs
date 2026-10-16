//! The 24 px cook and file status strip.
use super::{Actions, View};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};

fn error_parts(error: &str) -> (Option<&str>, &str) {
    if let Some(rest) = error.strip_prefix("node '") {
        if let Some((node, message)) = rest.split_once("'): ") { return (Some(node), message); }
        if let Some((node, rest)) = rest.split_once("' (") {
            if let Some((_, message)) = rest.split_once("): ") { return (Some(node), message); }
        }
    }
    (None, error)
}

fn span(p: &egui::Painter, x: &mut f32, y: f32, text: &str, mono: bool, color: Color32) {
    let font = if mono { FontId::monospace(12.) } else { FontId::proportional(12.) };
    let galley = p.layout_no_wrap(text.to_owned(), font, color);
    p.galley(Pos2::new(*x, y - galley.size().y / 2.), galley.clone(), color);
    *x += galley.size().x;
}

fn blender_cap_text(cap: u32) -> Option<String> { (cap > 0).then(|| format!("Blender cap: {cap} tris")) }

pub fn show(ui: &mut egui::Ui, v: &View, _out: &mut Actions) {
    let (bar, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 24.), Sense::hover());
    let p = ui.painter();
    p.rect_filled(bar, 0., v.theme.panel2);
    p.line_segment([bar.left_top() + Vec2::new(0., 0.5), bar.right_top() + Vec2::new(0., 0.5)], Stroke::new(1.0_f32, v.theme.stroke));
    let y = bar.center().y;
    let (dot_color, state) = if v.cooking.cooking() { (v.theme.accent, 1) }
        else if v.cooking.error.is_some() { (v.theme.err, 2) }
        else { (v.theme.ok, 0) };
    p.circle_filled(Pos2::new(bar.left() + 13.5, y), 3.5, dot_color);
    let mut x = bar.left() + 28.;
    match state {
        1 => span(p, &mut x, y, "Cooking...", false, v.theme.text),
        2 => {
            let (node, message) = error_parts(v.cooking.error.as_deref().unwrap_or_default());
            span(p, &mut x, y, "Error", false, v.theme.err);
            if let Some(node) = node {
                span(p, &mut x, y, " in ", false, v.theme.err);
                span(p, &mut x, y, node, true, v.theme.err);
            }
            span(p, &mut x, y, ":  ", false, v.theme.err);
            span(p, &mut x, y, message, true, v.theme.text);
            x += 14.;
            span(p, &mut x, y, "Showing last good mesh.", false, v.theme.dim);
        }
        _ => {
            span(p, &mut x, y, "Cooked: ", false, v.theme.dim);
            span(p, &mut x, y, &v.cooking.stats.cooked.join(", "), true, v.theme.text);
            span(p, &mut x, y, " in ", false, v.theme.text);
            span(p, &mut x, y, &format!("{:.0} ms", v.cooking.milliseconds), true, v.theme.text);
            x += 14.;
            span(p, &mut x, y, "Cached: ", false, v.theme.dim);
            span(p, &mut x, y, &v.cooking.stats.cached.join(", "), true, v.theme.dim);
        }
    }
    if let Some(cap) = blender_cap_text(v.editor.proxy.link_tris) {
        x += 14.;
        span(p, &mut x, y, &cap, true, v.theme.dim);
    }
    let right = bar.right() - 10.;
    let file_width = if v.file_status.is_empty() { 0. } else { p.layout_no_wrap(v.file_status.to_owned(), FontId::proportional(12.), v.theme.dim).size().x + 14. };
    let label = build_label();
    let text_width = p.layout_no_wrap(label.clone(), FontId::proportional(11.), v.theme.dim).size().x;
    let chip_width = text_width + 32.;
    let chip = Rect::from_center_size(Pos2::new(right - file_width - chip_width / 2., y), Vec2::new(chip_width, 18.));
    p.rect_stroke(chip, 9., Stroke::new(1.0_f32, v.theme.stroke), StrokeKind::Inside);
    p.circle_filled(Pos2::new(chip.left() + 11., y), 3.5, v.theme.ok);
    p.text(Pos2::new(chip.left() + 21., y), Align2::LEFT_CENTER, label, FontId::proportional(11.), v.theme.dim);
    if !v.file_status.is_empty() { p.text(Pos2::new(right, y), Align2::RIGHT_CENTER, v.file_status, FontId::proportional(12.), v.theme.dim); }
}

/// "Rust core v0.1.0 · f80387d* · 2026-10-16 15:12 UTC": package version, git commit (* = uncommitted changes), build time.
pub fn build_label() -> String {
    format!("Rust core v{} \u{b7} {} \u{b7} {}", env!("CARGO_PKG_VERSION"), env!("MINIHOU_BUILD_COMMIT"), env!("MINIHOU_BUILD_TIME"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::testing::Fixture;
    #[test] fn parses_node_errors() {
        assert_eq!(error_parts("node 'noise1' (noise): bad value"), (Some("noise1"), "bad value"));
        assert_eq!(error_parts("unexpected failure"), (None, "unexpected failure"));
    }
    #[test] fn draws_a_frame() { assert!(Fixture::new().run(show).is_empty()); }
    #[test] fn blender_cap_is_reported_when_enabled() {
        assert_eq!(blender_cap_text(0), None);
        assert_eq!(blender_cap_text(12_000).as_deref(), Some("Blender cap: 12000 tris"));
    }
}
