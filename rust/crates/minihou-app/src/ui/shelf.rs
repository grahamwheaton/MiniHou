//! The two-row tool shelf.
#[path = "shelf_data.rs"]
mod shelf_data;
#[path = "shelf_icons.rs"] mod shelf_icons;

use super::{Actions, UiAction, View};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Vec2};
use shelf_data::{tabs, Tool};

fn drag_payload(tool: &Tool) -> super::ToolDrag { super::ToolDrag(tool.node_type.to_owned()) }
pub fn dropped_tool(tool: &super::ToolDrag, pos: Pos2) -> UiAction {
    UiAction::AddNetworkNode { node_type: tool.0.clone(), pos: [pos.x, pos.y], wire: None }
}

fn line(p: &egui::Painter, o: Pos2, points: &[(f32, f32)], color: Color32) {
    for pair in points.windows(2) {
        p.line_segment([o + Vec2::new(pair[0].0, pair[0].1), o + Vec2::new(pair[1].0, pair[1].1)], Stroke::new(1.5_f32, color));
    }
}
fn ellipse(p: &egui::Painter, c: Pos2, rx: f32, ry: f32, color: Color32) {
    p.add(Shape::ellipse_stroke(c, Vec2::new(rx, ry), Stroke::new(1.5_f32, color)));
}
fn dot(p: &egui::Painter, o: Pos2, x: f32, y: f32, r: f32, color: Color32) {
    p.circle_filled(o + Vec2::new(x, y), r, color);
}

fn icon(p: &egui::Painter, r: Rect, tool: &Tool, color: Color32, accent: Color32) {
    let o = r.center() - Vec2::splat(12.);
    let c = o + Vec2::splat(12.);
    match tool.icon {
        "box" => { line(p, o, &[(12., 3.), (20., 7.), (20., 17.), (12., 21.), (4., 17.), (4., 7.), (12., 3.)], color); line(p, o, &[(4., 7.), (12., 11.), (20., 7.)], color); line(p, o, &[(12., 11.), (12., 21.)], color); }
        "point" => { line(p, o, &[(12., 3.), (12., 9.)], color); line(p, o, &[(12., 15.), (12., 21.)], color); line(p, o, &[(3., 12.), (9., 12.)], color); line(p, o, &[(15., 12.), (21., 12.)], color); dot(p, o, 12., 12., 3., accent); }
        "sphere" => { p.circle_stroke(c, 8.5, Stroke::new(1.5_f32, color)); ellipse(p, c, 8.5, 3., color); }
        "tube" => { ellipse(p, o + Vec2::new(12., 6.), 6., 2.5, color); line(p, o, &[(6., 6.), (6., 18.)], color); line(p, o, &[(18., 6.), (18., 18.)], color); ellipse(p, o + Vec2::new(12., 18.), 6., 2.5, color); }
        "torus" => { ellipse(p, c, 9., 6., color); ellipse(p, c, 3.5, 1.6, color); }
        "grid" => { line(p, o, &[(3., 18.), (7., 7.), (21., 7.), (17., 18.), (3., 18.)], color); line(p, o, &[(5., 12.5), (19., 12.5)], color); line(p, o, &[(12., 7.), (8.5, 18.)], color); line(p, o, &[(16.5, 7.), (13., 18.)], color); }
        "null" => { line(p, o, &[(12., 4.), (12., 12.), (20., 12.)], color); line(p, o, &[(12., 12.), (6., 18.)], color); dot(p, o, 12., 12., 2., accent); }
        "line" => { line(p, o, &[(5.5, 18.5), (18.5, 5.5)], color); dot(p, o, 5.5, 18.5, 2., accent); dot(p, o, 18.5, 5.5, 2., accent); }
        "circle" => { p.circle_stroke(c, 8., Stroke::new(1.5_f32, color)); dot(p, o, 20., 12., 1.8, accent); }
        "curve" | "bezier" => { let pts = bezier((4.5, 18.), (8., 2.), (16., 22.), (19.5, 6.)); line(p, o, &pts, color); dot(p, o, 4.5, 18., 1.8, accent); dot(p, o, 19.5, 6., 1.8, accent); }
        "helix" => { let mut pts = Vec::new(); for i in 0..33 { let t = i as f32 / 32.; pts.push((12. + 6. * (t * 5. * std::f32::consts::PI).sin(), 20. - 15. * t)); } line(p, o, &pts, color); }
        "lsystem" => { for pts in [&[(12., 21.), (12., 6.)][..], &[(12., 16.), (7., 11.), (6., 6.)], &[(12., 13.), (17., 8.), (18., 4.)]] { line(p, o, pts, color); } }
        "lattice" => { line(p, o, &[(5., 5.), (19., 5.), (19., 19.), (5., 19.), (5., 5.)], color); line(p, o, &[(5., 12.), (19., 12.)], color); line(p, o, &[(12., 5.), (12., 19.)], color); for (x, y) in [(5., 5.), (19., 5.), (5., 19.), (19., 19.), (12., 12.)] { dot(p, o, x, y, 1.6, accent); } }
        "points_to_surface" => { p.circle_filled(o + Vec2::new(9.5, 13.), 6., accent); p.circle_filled(o + Vec2::new(16.5, 9.), 4.5, color.gamma_multiply(0.6)); }
        "file" => { line(p, o, &[(3., 19.), (3., 7.), (10., 7.), (12., 9.5), (21., 9.5), (21., 19.), (3., 19.)], color); }
        "host" => { p.rect_stroke(Rect::from_min_size(o + Vec2::new(11., 6.), Vec2::new(10., 12.)), 1., Stroke::new(1.5_f32, color), StrokeKind::Inside); line(p, o, &[(2.5, 12.), (15., 12.)], color); line(p, o, &[(10., 7.5), (14.5, 12.), (10., 16.5)], color); }
        _ if shelf_icons::draw(p, o, tool.icon, color, accent) => {}
        _ => { let chip = Rect::from_center_size(c, Vec2::splat(22.)); p.rect_stroke(chip, 5., Stroke::new(1.5_f32, color), StrokeKind::Inside); let initials = initials(tool.label); p.text(c, Align2::CENTER_CENTER, initials, FontId::proportional(10.), color); }
    }
}

fn bezier(a: (f32, f32), b: (f32, f32), c: (f32, f32), d: (f32, f32)) -> Vec<(f32, f32)> {
    (0..=16).map(|i| { let t = i as f32 / 16.; let s = 1. - t; (s*s*s*a.0 + 3.*s*s*t*b.0 + 3.*s*t*t*c.0 + t*t*t*d.0, s*s*s*a.1 + 3.*s*s*t*b.1 + 3.*s*t*t*c.1 + t*t*t*d.1) }).collect()
}
fn initials(label: &str) -> String {
    let words: Vec<_> = label.split([' ', '-', '/']).filter(|s| !s.is_empty()).collect();
    if words.len() > 1 { words.iter().take(2).filter_map(|w| w.chars().next()).collect::<String>().to_uppercase() }
    else { label.chars().take(2).collect::<String>().to_uppercase() }
}
fn tab_id(ui: &egui::Ui) -> egui::Id { ui.id().with("shelf.tab") }
fn folded_id(ui: &egui::Ui) -> egui::Id { ui.id().with("shelf.folded") }
fn visible_id(ui: &egui::Ui) -> egui::Id { ui.id().with("shelf.visible") }

pub fn show(ui: &mut egui::Ui, v: &View, out: &mut Actions) {
    let tab_id = tab_id(ui);
    let folded_id = folded_id(ui);
    let visible_id = visible_id(ui);
    let all_tabs = (1_u32 << tabs().len()) - 1;
    let mut visible = ui.ctx().data_mut(|d| d.get_persisted::<u32>(visible_id)).unwrap_or(all_tabs);
    let mut active = ui.ctx().data_mut(|d| d.get_persisted::<usize>(tab_id)).unwrap_or(0).min(tabs().len() - 1);
    if visible & (1 << active) == 0 { active = (0..tabs().len()).find(|i| visible & (1 << i) != 0).unwrap_or(0); }
    let mut folded = ui.ctx().data_mut(|d| d.get_persisted::<bool>(folded_id)).unwrap_or(false);
    let (tabs_rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 26.), Sense::hover());
    ui.painter().rect_filled(tabs_rect, 0., v.theme.panel);
    ui.painter().line_segment([tabs_rect.left_bottom() - Vec2::new(0., 0.5), tabs_rect.right_bottom() - Vec2::new(0., 0.5)], Stroke::new(1.0_f32, v.theme.stroke));
    let tabs_content = tabs_rect.shrink2(Vec2::new(10., 0.));
    ui.scope_builder(egui::UiBuilder::new().max_rect(tabs_content), |ui| {
        ui.set_clip_rect(tabs_content);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.;
            let tabs_width = (ui.available_width() - 54.).max(1.);
            ui.allocate_ui_with_layout(Vec2::new(tabs_width, 25.), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                egui::ScrollArea::horizontal().id_salt("shelf.tabs.scroll").show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.;
                        for (idx, tab) in tabs().iter().enumerate() {
                            if visible & (1 << idx) == 0 { continue; }
                            let w = tab.name.len() as f32 * 6.3 + 18.;
                            let (r, response) = ui.allocate_exact_size(Vec2::new(w, 24.), Sense::click());
                            ui.painter().text(r.center() + Vec2::new(0., 1.), Align2::CENTER_CENTER, tab.name, FontId::proportional(12.), if active == idx { v.theme.text } else { v.theme.dim });
                            if active == idx { ui.painter().rect_filled(Rect::from_min_max(Pos2::new(r.left(), r.bottom() - 2.), r.right_bottom()), 0., v.theme.accent); }
                            if response.clicked() { active = idx; folded = false; }
                        }
                    });
                });
            });
            ui.allocate_ui_with_layout(Vec2::new(26., 24.), egui::Layout::left_to_right(egui::Align::Center), |ui| { ui.menu_button("+", |ui| {
                for (idx, tab) in tabs().iter().enumerate() {
                    let mut shown = visible & (1 << idx) != 0;
                    if ui.checkbox(&mut shown, tab.name).changed() {
                        if shown { visible |= 1 << idx; }
                        else if visible.count_ones() > 1 { visible &= !(1 << idx); }
                        if visible & (1 << active) == 0 { active = (0..tabs().len()).find(|i| visible & (1 << i) != 0).unwrap_or(0); }
                    }
                }
            }); });
            let (fold, response) = ui.allocate_exact_size(Vec2::new(20., 24.), Sense::click());
            let y = if folded { 9. } else { 14. };
            let pts = if folded { [(4., y), (10., y + 6.), (16., y)] } else { [(4., y), (10., y - 6.), (16., y)] };
            line(ui.painter(), fold.min, &pts, v.theme.dim);
            if response.clicked() { folded = !folded; }
        });
    });
    ui.ctx().data_mut(|d| { d.insert_persisted(tab_id, active); d.insert_persisted(folded_id, folded); d.insert_persisted(visible_id, visible); });
    if folded { return; }
    let (row, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 52.), Sense::hover());
    ui.painter().rect_filled(row, 0., v.theme.panel2);
    ui.painter().line_segment([row.left_bottom() - Vec2::new(0., 0.5), row.right_bottom() - Vec2::new(0., 0.5)], Stroke::new(1.0_f32, v.theme.stroke));
    let tools_rect = Rect::from_min_max(row.min + Vec2::new(10., 0.), row.max - Vec2::new(112., 0.));
    ui.scope_builder(egui::UiBuilder::new().max_rect(tools_rect), |ui| {
        ui.set_clip_rect(tools_rect);
        egui::ScrollArea::horizontal().id_salt("shelf.tools.scroll").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.;
                for tool in tabs()[active].tools {
                    let (r, response) = ui.allocate_exact_size(Vec2::new(54., 50.), Sense::click_and_drag());
                    response.dnd_set_drag_payload(drag_payload(tool));
                    if response.hovered() { ui.painter().rect_filled(r, 5., v.theme.input); ui.painter().rect_stroke(r, 5., Stroke::new(1.0_f32, v.theme.stroke), StrokeKind::Inside); }
                    let icon_rect = Rect::from_center_size(r.center_top() + Vec2::new(0., 12.), Vec2::splat(22.));
                    icon(ui.painter(), icon_rect, tool, v.theme.text, v.theme.c_src);
                    let label = wrap_label(tool.label);
                    // one line at a time: egui left-aligns the lines of a multi-line string inside its block
                    for (i, text) in label.lines().enumerate() {
                        ui.painter().with_clip_rect(r).text(r.center_top() + Vec2::new(0., 27. + i as f32 * 11.), Align2::CENTER_TOP, text, FontId::proportional(9.5), v.theme.text);
                    }

                    if response.on_hover_text(format!("{} ({})", tool.label, tool.node_type)).clicked() {
                        if active == 0 && matches!(tool.node_type, "line" | "circle" | "curve") {
                            let mode = match tool.node_type { "line" => super::sketch::SketchMode::Line, "circle" => super::sketch::SketchMode::Circle, _ => super::sketch::SketchMode::Pen };
                            out.push(UiAction::StartSketch(mode));
                        } else if tool.node_type == "mirror" {
                            // The Mirror button starts the interactive tool (axis cubes + plane), like Alt+M.
                            out.push(UiAction::Command("mirror_tool".into()));
                        } else { out.push(UiAction::AddNode { node_type: tool.node_type.to_owned(), after: Some(v.selected.to_owned()) }); }
                    }
                }
            });
        });
    });


}

fn wrap_label(label: &str) -> String {
    if label.len() <= 9 { return label.to_owned(); }
    if let Some((a, b)) = label.split_once(' ') { format!("{a}\n{b}") } else { label.to_owned() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::testing::Fixture;
    #[test] fn shelf_lists_are_complete() { assert_eq!(tabs().len(), 20); assert_eq!(tabs()[0].name, "Create"); assert!(tabs()[0].tools.iter().any(|t| t.node_type == "box")); }
    #[test] fn package_is_in_shelf_without_changing_legacy_order() {
        for (legacy, merged) in shelf_data::TABS.iter().zip(tabs()) {
            assert_eq!(legacy.name, merged.name);
            assert_eq!(legacy.tools.iter().map(|t| (t.label, t.node_type, t.icon)).collect::<Vec<_>>(),
                merged.tools.iter().take(legacy.tools.len()).map(|t| (t.label, t.node_type, t.icon)).collect::<Vec<_>>());
        }
        let modify = tabs().iter().find(|t| t.name == "Modify").unwrap();
        assert_eq!(modify.tools[0].node_type, "lod_bake");
        // Packages in Modify come after the legacy tools in catalogue order (example_offset, pack, unpack).
        // This was once "example_offset last"; pack and unpack were added by the instancing work.
        let package_tools: Vec<_> = modify.tools.iter().skip(modify.tools.len() - 3).map(|t| t.node_type).collect();
        assert_eq!(package_tools, ["example_offset", "pack", "unpack"]);
        let measure = tabs().iter().find(|t| t.name == "Measure").unwrap();
        for id in ["measure_volume", "measure_bounds", "measure_area"] {
            assert_eq!(measure.tools.iter().filter(|t| t.node_type == id).count(), 1, "{id}");
        }
    }
    #[test] fn labels_and_initials() { assert_eq!(wrap_label("Blender Object"), "Blender\nObject"); assert_eq!(initials("Blender Object"), "BO"); }
    #[test] fn draws_a_frame() { assert!(Fixture::new().run(show).is_empty()); }
    #[test] fn drag_payload_requests_the_same_tool_on_drop() {
        let payload = drag_payload(&tabs()[0].tools[0]);
        assert!(matches!(dropped_tool(&payload, Pos2::new(320., -64.)), UiAction::AddNetworkNode { node_type, pos, .. }
            if node_type == "box" && pos == [320., -64.]));
    }
    #[test] fn web_icon_table_is_covered_and_unknown_tools_keep_initials() {
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                for tool in tabs().iter().flat_map(|tab| tab.tools) {
                    let built_in = matches!(tool.icon, "box" | "point" | "sphere" | "tube" | "torus" | "grid" | "null" |
                        "line" | "circle" | "curve" | "bezier" | "helix" | "lsystem" | "lattice" | "points_to_surface" | "file" | "host");
                    let fallback = matches!(tool.icon, "clean" | "clip" | "curve_simplify" | "falloff" | "occlusion" | "sdf_shape");
                    assert!(built_in || fallback || shelf_icons::draw(ui.painter(), Pos2::ZERO, tool.icon, Color32::WHITE, Color32::WHITE), "{}", tool.icon);
                }
            });
        });
    }
}
