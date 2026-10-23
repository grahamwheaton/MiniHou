//! The 40 px workspace toolbar.
use super::{Actions, UiAction, View};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};

fn path(p: &egui::Painter, origin: Pos2, parts: &[&[(f32, f32)]], color: Color32, width: f32) {
    for part in parts {
        for pair in part.windows(2) {
            p.line_segment([origin + Vec2::new(pair[0].0, pair[0].1), origin + Vec2::new(pair[1].0, pair[1].1)], Stroke::new(width, color));
        }
    }
}

fn icon(p: &egui::Painter, r: Rect, kind: &str, color: Color32) {
    let o = r.center() - Vec2::splat(7.);
    match kind {
        "open" => path(p, o, &[&[(1.5, 3.5), (5.5, 3.5), (6.7, 5.), (12.5, 5.), (12.5, 11.5), (1.5, 11.5), (1.5, 3.5)]], color, 1.4),
        "save" => path(p, o, &[&[(2., 12.), (2., 2.), (10., 2.), (12., 4.), (12., 12.), (2., 12.)], &[(4.5, 2.), (4.5, 5.5), (8.5, 5.5), (8.5, 2.)], &[(4.5, 12.), (4.5, 8.5), (9.5, 8.5), (9.5, 12.)]], color, 1.4),
        "undo" => path(p, o, &[&[(4.5, 2.5), (2., 5.), (4.5, 7.5)], &[(2., 5.), (8., 5.), (9.4, 5.3), (10.6, 6.3), (11.3, 7.7), (11., 9.4), (9.7, 11.2), (8., 12.), (5., 12.)]], color, 1.4),
        "redo" => path(p, o, &[&[(9.5, 2.5), (12., 5.), (9.5, 7.5)], &[(12., 5.), (6., 5.), (4.6, 5.3), (3.4, 6.3), (2.7, 7.7), (3., 9.4), (4.3, 11.2), (6., 12.), (9., 12.)]], color, 1.4),
        _ => {}
    }
}

fn button(ui: &mut egui::Ui, label: &str, kind: &str, enabled: bool, outlined: bool, v: &View, tooltip: Option<&str>) -> bool {
    let width = match label { "Save As" => 86., "Open" => 67., "Repeat" => 75., _ => 65. };
    let (rect, mut response) = ui.allocate_exact_size(Vec2::new(width, 28.), if enabled { Sense::click() } else { Sense::hover() });
    if let Some(tip) = tooltip { response = response.on_hover_text(tip); }
    let p = ui.painter();
    if outlined || response.hovered() && enabled {
        p.rect_filled(rect, 5., if response.hovered() { v.theme.input } else { v.theme.panel2 });
        p.rect_stroke(rect, 5., Stroke::new(1.0_f32, v.theme.stroke), StrokeKind::Inside);
    }
    let color = if enabled { v.theme.text } else { v.theme.faint };
    icon(p, Rect::from_min_size(rect.min + Vec2::new(10., 7.), Vec2::splat(14.)), kind, color);
    p.text(rect.min + Vec2::new(30., 14.), Align2::LEFT_CENTER, label, FontId::proportional(12.), color);
    response.clicked()
}

/// File menu entries in display order (excluding Export USD).
pub fn file_menu_entries() -> Vec<(&'static str, UiAction)> {
    vec![("Open", UiAction::Open), ("Save", UiAction::Save), ("Save As", UiAction::SaveAs)]
}

/// Preferences menu entries in display order (a separator goes before "Preferences…").
pub fn preferences_menu_entries() -> Vec<(&'static str, UiAction)> {
    vec![("Save UI Layout", UiAction::SaveUiLayout), ("Load UI Layout", UiAction::LoadUiLayout),
        ("Reset UI to Default", UiAction::ResetUiLayout), ("Preferences…", UiAction::OpenPreferences)]
}

pub fn show(ui: &mut egui::Ui, v: &View, out: &mut Actions) {
    let (bar, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 40.), Sense::hover());
    ui.painter().rect_filled(bar, 0., v.theme.panel);
    ui.painter().line_segment([bar.left_bottom() - Vec2::new(0., 0.5), bar.right_bottom() - Vec2::new(0., 0.5)], Stroke::new(1.0_f32, v.theme.stroke));
    let content = Rect::from_min_max(bar.min + Vec2::new(10., 0.), bar.max - Vec2::new(10., 1.));
    ui.scope_builder(egui::UiBuilder::new().max_rect(content), |ui| {
        ui.set_clip_rect(content);
        ui.spacing_mut().item_spacing.x = 8.;
        ui.horizontal_centered(|ui| {
            let (logo, _) = ui.allocate_exact_size(Vec2::splat(20.), Sense::hover());
            ui.painter().rect_filled(logo.shrink(1.), 4., v.theme.accent);
            path(ui.painter(), logo.min, &[&[(5., 14.), (5., 6.), (10., 11.), (15., 6.), (15., 14.)]], Color32::WHITE, 2.);
            let (name, _) = ui.allocate_exact_size(Vec2::new(55., 20.), Sense::hover());
            ui.painter().text(name.left_center(), Align2::LEFT_CENTER, "MiniHou", FontId::proportional(13.), v.theme.text);
            separator(ui, v);
            ui.menu_button("File", |ui| {
                for (label, action) in file_menu_entries() {
                    if ui.button(label).clicked() { out.push(action); ui.close(); }
                }
                ui.separator();
                if ui.button("Export USD…").clicked() { out.push(UiAction::Command("export_usd".into())); ui.close(); }
            });
            ui.menu_button("Edit", |ui| {
                if ui.button("Clear all").clicked() { out.push(UiAction::ClearMeasurements); ui.close(); }
            });
            ui.menu_button("Preferences", |ui| {
                for (i, (label, action)) in preferences_menu_entries().into_iter().enumerate() {
                    if i == 3 { ui.separator(); }
                    if ui.button(label).clicked() { out.push(action); ui.close(); }
                }
            });
            separator(ui, v);
            if button(ui, "Undo", "undo", v.can_undo, false, v, None) { out.push(UiAction::Undo); }
            if button(ui, "Redo", "redo", v.can_redo, false, v, None) { out.push(UiAction::Redo); }
            let repeat_tip = v.repeat_label.unwrap_or("Nothing to repeat (Shift+R)");
            if button(ui, "Repeat", "redo", v.repeat_label.is_some(), false, v, Some(repeat_tip)) { out.push(UiAction::RepeatLast); }
            let filename = v.path.file_name().map(|s| s.to_string_lossy()).unwrap_or_default();
            let trailing = 12. * filename.len() as f32 * 0.61 + if v.dirty { 18. } else { 0. };
            ui.add_space((ui.available_width() - trailing).max(0.));
            ui.label(egui::RichText::new(filename.as_ref()).font(FontId::monospace(12.)).color(v.theme.text));
            if v.dirty { ui.label(egui::RichText::new("*").font(FontId::monospace(12.)).color(v.theme.warn)).on_hover_text("Unsaved changes"); }
        });
    });
}

fn separator(ui: &mut egui::Ui, v: &View) {
    let (r, _) = ui.allocate_exact_size(Vec2::new(1., 20.), Sense::hover());
    ui.painter().rect_filled(r, 0., v.theme.stroke);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::testing::Fixture;

    #[test]
    fn file_menu_entries_are_open_save_save_as() {
        let e = file_menu_entries();
        let labels: Vec<_> = e.iter().map(|(l, _)| *l).collect();
        assert_eq!(labels, ["Open", "Save", "Save As"]);
        assert!(matches!(e[0].1, UiAction::Open) && matches!(e[1].1, UiAction::Save) && matches!(e[2].1, UiAction::SaveAs));
    }

    #[test]
    fn preferences_menu_entries_in_order() {
        let e = preferences_menu_entries();
        let labels: Vec<_> = e.iter().map(|(l, _)| *l).collect();
        assert_eq!(labels, ["Save UI Layout", "Load UI Layout", "Reset UI to Default", "Preferences…"]);
        assert!(matches!(e[0].1, UiAction::SaveUiLayout) && matches!(e[1].1, UiAction::LoadUiLayout)
            && matches!(e[2].1, UiAction::ResetUiLayout) && matches!(e[3].1, UiAction::OpenPreferences));
    }

    #[test]
    fn edit_menu_contains_clear_all_action() {
        let f = Fixture::new();
        let ctx = egui::Context::default();
        let viewport = Rect::from_min_size(Pos2::ZERO, Vec2::new(1280., 800.));
        let v = f.view();
        let frame = |events: Vec<egui::Event>| {
            let mut out = Actions::default();
            let _ = ctx.run(egui::RawInput { screen_rect: Some(viewport), events, ..Default::default() }, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    show(ui, &v, &mut out);
                });
            });
            out
        };
        let click = |p: Pos2| {
            frame(vec![egui::Event::PointerMoved(p), egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() }]);
            frame(vec![egui::Event::PointerMoved(p), egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() }])
        };
        frame(vec![]);
        click(Pos2::new(170., 27.));
        // Next frame to let popup layout
        frame(vec![egui::Event::PointerMoved(Pos2::new(185., 53.))]);
        let out = click(Pos2::new(185., 53.));
        assert!(out.0.iter().any(|a| matches!(a, UiAction::ClearMeasurements)));
    }
}

