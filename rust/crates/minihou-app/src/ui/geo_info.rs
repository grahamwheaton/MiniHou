//! Geometry summary beneath Parameters.
#[path = "../geo_summary.rs"]
pub mod geo_summary;

use super::{theme::Theme, Actions, AttrInfo, GeoSummary, View};
use eframe::egui::{self, Align2, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};

const HEIGHT: f32 = 220.;
const HEADER: f32 = 26.;
const ROW: f32 = 18.;

fn line(ui: &mut egui::Ui, height: f32) -> Rect {
    ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover()).0
}

fn text(ui: &egui::Ui, x: f32, y: f32, value: &str, color: egui::Color32) {
    ui.painter().text(Pos2::new(x, y), Align2::LEFT_CENTER, value, FontId::monospace(11.), color);
}

fn fmt(value: f64) -> String { format!("{value:.3}") }

fn vec3(v: [f32; 3], units: crate::units::UnitSettings) -> String {
    format!("{}, {}, {}", crate::units::format(v[0] as f64, units), crate::units::format(v[1] as f64, units), crate::units::format(v[2] as f64, units))
}

fn stats(ui: &mut egui::Ui, s: &GeoSummary, theme: &Theme, units: crate::units::UnitSettings) {
    let r = line(ui, ROW + 4.);
    text(ui, r.left() + 10., r.center().y, "POINTS", theme.dim);
    text(ui, r.left() + 68., r.center().y, &s.points.to_string(), theme.text);
    text(ui, r.left() + 105., r.center().y, "VERTS", theme.dim);
    text(ui, r.left() + 155., r.center().y, &s.vertices.to_string(), theme.text);
    text(ui, r.left() + 197., r.center().y, "PRIMS", theme.dim);
    text(ui, r.left() + 245., r.center().y, &s.prims.to_string(), theme.text);
    if let Some((min, max)) = s.bounds {
        for (label, value) in [("Min", min), ("Max", max), ("Size", [max[0] - min[0], max[1] - min[1], max[2] - min[2]])] {
            let r = line(ui, ROW);
            text(ui, r.left() + 10., r.center().y, label, theme.dim);
            text(ui, r.left() + 48., r.center().y, &vec3(value, units), theme.text);
        }
    }
}

fn attribute_table(ui: &mut egui::Ui, label: &str, attrs: &[AttrInfo], ranges: bool, theme: &Theme) {
    let r = line(ui, 20.);
    ui.painter().rect_filled(r, 0., theme.panel2);
    ui.painter().line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(1.0_f32, theme.stroke));
    text(ui, r.left() + 10., r.center().y, &format!("{label} ({})", attrs.len()), theme.text);
    if attrs.is_empty() { return; }
    let r = line(ui, ROW);
    for (x, heading) in [(10., "NAME"), (112., "TYPE"), (196., "SIZE")] {
        text(ui, r.left() + x, r.center().y, heading, theme.faint);
    }
    if ranges {
        text(ui, r.left() + 240., r.center().y, "MIN", theme.faint);
        text(ui, r.left() + 320., r.center().y, "MAX", theme.faint);
    }
    for a in attrs {
        let r = line(ui, ROW);
        ui.painter().line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(1.0_f32, theme.stroke));
        text(ui, r.left() + 10., r.center().y, &a.name, theme.text);
        text(ui, r.left() + 112., r.center().y, &a.kind, theme.dim);
        text(ui, r.left() + 196., r.center().y, &a.components.to_string(), theme.dim);
        if ranges {
            text(ui, r.left() + 240., r.center().y, &a.min.map_or_else(|| "—".into(), fmt), theme.dim);
            text(ui, r.left() + 320., r.center().y, &a.max.map_or_else(|| "—".into(), fmt), theme.dim);
        }
    }
}

pub fn show(ui: &mut egui::Ui, v: &View<'_>, _out: &mut Actions) {
    let id = ui.id().with("geo_info");
    let mut open = ui.ctx().data_mut(|d| d.get_persisted::<bool>(id.with("open")).unwrap_or(true));
    let mut ranges = ui.ctx().data_mut(|d| d.get_persisted::<bool>(id.with("ranges")).unwrap_or(false));
    let width = ui.available_width();
    let (header, response) = ui.allocate_exact_size(Vec2::new(width, HEADER), Sense::click());
    ui.painter().rect(header, 0., v.theme.panel2, Stroke::new(1.0_f32, v.theme.stroke), StrokeKind::Inside);
    let c = Pos2::new(header.left() + 14., header.center().y);
    let path = if open { [c + Vec2::new(-4., -2.), c + Vec2::new(0., 2.), c + Vec2::new(4., -2.)] }
        else { [c + Vec2::new(-2., -4.), c + Vec2::new(2., 0.), c + Vec2::new(-2., 4.)] };
    ui.painter().line_segment([path[0], path[1]], Stroke::new(1.4_f32, v.theme.dim));
    ui.painter().line_segment([path[1], path[2]], Stroke::new(1.4_f32, v.theme.dim));
    let title = v.geo.map_or_else(|| "Geometry".to_owned(), |s| format!("Geometry {}", s.node));
    ui.painter().text(Pos2::new(header.left() + 26., header.center().y), Align2::LEFT_CENTER,
        title, FontId::proportional(12.), v.theme.text);
    if let Some(s) = v.geo {
        let displayed = s.node == v.display;
        let chip = Rect::from_min_size(Pos2::new(header.right() - 78., header.top() + 4.), Vec2::new(68., 18.));
        let (ink, fill) = if displayed { (v.theme.accent, v.theme.accent_bg) } else { (v.theme.dim, v.theme.panel) };
        ui.painter().rect(chip, 3., fill, Stroke::new(1.0_f32, ink), StrokeKind::Inside);
        ui.painter().text(chip.center(), Align2::CENTER_CENTER, if displayed { "displayed" } else { "other" },
            FontId::monospace(10.), ink);
    }
    if response.clicked() {
        open = !open;
        ui.ctx().data_mut(|d| d.insert_persisted(id.with("open"), open));
    }
    if !open { return; }
    ui.allocate_ui_with_layout(Vec2::new(width, HEIGHT - HEADER), egui::Layout::top_down(egui::Align::Min), |ui| {
        ui.set_clip_rect(ui.max_rect());
        let r = line(ui, 22.);
        let box_rect = Rect::from_min_size(Pos2::new(r.left() + 10., r.top() + 4.), Vec2::splat(14.));
        let check = ui.interact(box_rect, id.with("ranges_check"), Sense::click());
        ui.painter().rect(box_rect, 3., if ranges { v.theme.accent } else { v.theme.input },
            Stroke::new(1.0_f32, if ranges { v.theme.accent } else { v.theme.stroke2 }), StrokeKind::Inside);
        if ranges {
            let c = box_rect.center();
            let s = Stroke::new(1.4_f32, v.theme.panel);
            ui.painter().line_segment([c + Vec2::new(-4., 0.), c + Vec2::new(-1., 3.)], s);
            ui.painter().line_segment([c + Vec2::new(-1., 3.), c + Vec2::new(4., -3.)], s);
        }
        text(ui, r.left() + 30., r.center().y, "ranges", v.theme.dim);
        if check.clicked() {
            ranges = !ranges;
            ui.ctx().data_mut(|d| d.insert_persisted(id.with("ranges"), ranges));
        }
        let Some(s) = v.geo else {
            let r = line(ui, ROW);
            text(ui, r.left() + 10., r.center().y, "Nothing cooked yet", v.theme.dim);
            return;
        };
        egui::ScrollArea::both().id_salt("geo_info_scroll").max_height(HEIGHT - HEADER - 22.).show(ui, |ui| {
            ui.set_min_width(if ranges { 400. } else { width });
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            stats(ui, s, v.theme, v.units);
            attribute_table(ui, "Point", &s.point_attrs, ranges, v.theme);
            attribute_table(ui, "Vertex", &s.vertex_attrs, ranges, v.theme);
            attribute_table(ui, "Primitive", &s.prim_attrs, ranges, v.theme);
            attribute_table(ui, "Detail", &s.detail_attrs, ranges, v.theme);
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::testing::Fixture;

    #[test]
    fn empty_and_populated_frames_are_read_only() {
        let mut f = Fixture::new();
        assert!(f.run(show).is_empty());
        f.geo = Some(GeoSummary { node: f.display.clone(), points: 3, vertices: 3, prims: 1,
            bounds: Some(([0.; 3], [1.; 3])), point_attrs: vec![AttrInfo {
                name: "P".into(), kind: "float32".into(), components: 3, min: Some(0.), max: Some(1.)
            }], ..Default::default() });
        assert!(f.run(show).is_empty());
        f.theme = Theme::light();
        assert!(f.run(show).is_empty());
    }

    #[test]
    fn bounds_display_with_units() {
        let mut f = Fixture::new();
        f.geo = Some(GeoSummary {
            node: f.display.clone(), points: 3, vertices: 3, prims: 1,
            bounds: Some(([0.; 3], [1.; 3])), ..Default::default()
        });
        let mut out = Actions::default();
        let ctx = egui::Context::default();
        f.theme.apply(&ctx);
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280., 800.))), ..Default::default() };
        let full = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                show(ui, &f.view(), &mut out);
            });
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
        extract_texts(&full.shapes, &mut texts);
        assert!(texts.iter().any(|t| t.contains("1000 mm")), "expected 1000 mm in bounds: {texts:?}");
    }
}
