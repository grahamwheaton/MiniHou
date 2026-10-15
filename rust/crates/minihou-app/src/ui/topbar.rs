//! Top bar (design: Main.dc.html `.top`): logo, Open / Save As, Undo / Redo, file name and dirty mark.
//! Baseline: the original functional toolbar; replace it with the design.
use super::{Actions, UiAction, View};
use crate::viewport::Mode;
use eframe::egui;

pub fn show(ui: &mut egui::Ui, v: &View, out: &mut Actions) {
    ui.horizontal(|ui| {
        ui.menu_button("File", |ui| {
            if ui.button("Open...").clicked() { out.push(UiAction::Open); ui.close(); }
            if ui.button("Save").clicked() { out.push(UiAction::Save); ui.close(); }
            if ui.button("Save As...").clicked() { out.push(UiAction::SaveAs); ui.close(); }
        });
        ui.menu_button("Edit", |ui| {
            if ui.add_enabled(v.can_undo, egui::Button::new("Undo (Ctrl+Z)")).clicked() { out.push(UiAction::Undo); ui.close(); }
            if ui.add_enabled(v.can_redo, egui::Button::new("Redo (Ctrl+Y)")).clicked() { out.push(UiAction::Redo); ui.close(); }
        });
        for (mode, label) in [(Mode::Solid, "Solid"), (Mode::Shaded, "Shaded"), (Mode::Wire, "Wire"), (Mode::ShadedWire, "Shaded+Wire")] {
            if ui.selectable_label(v.mode == mode, label).clicked() { out.push(UiAction::SetMode(mode)); }
        }
        if ui.button("Frame (F)").clicked() { out.push(UiAction::Frame); }
        if ui.button("Save (Ctrl+S)").clicked() { out.push(UiAction::Save); }
        ui.label(format!("{}{}", v.path.display(), if v.dirty { " *" } else { "" }));
    });
}
