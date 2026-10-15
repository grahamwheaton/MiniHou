//! Status bar (design: Main.dc.html `.status`): cook state dot, cooked / cached nodes, errors, the "Rust core" chip.
//! Baseline: the original stats lines.
use super::{Actions, View};
use eframe::egui;

pub fn show(ui: &mut egui::Ui, v: &View, _out: &mut Actions) {
    let stats = &v.cooking.stats;
    if v.cooking.cooking() { ui.label("cooking..."); }
    ui.label(format!("cooked: {} in {:.1} ms; cached: {}", stats.cooked.join(", "), v.cooking.milliseconds, stats.cached.join(", ")));
    if let Some(error) = &v.cooking.error { ui.colored_label(egui::Color32::RED, error); }
    if !v.file_status.is_empty() { ui.label(v.file_status); }
}
