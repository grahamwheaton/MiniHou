//! Network panel (design: Main.dc.html `.net`, node cards in NodeStates.dc.html): the node graph as cards and wires.
//! Baseline: the original flat node list (display radio, selectable label, [bypass]).
use super::{Actions, UiAction, View};
use eframe::egui;
use minihou_app::Edit;

pub fn show(ui: &mut egui::Ui, v: &View, out: &mut Actions) {
    ui.heading("Nodes");
    egui::ScrollArea::vertical().show(ui, |ui| {
        for (id, node) in &v.graph.nodes {
            ui.horizontal(|ui| {
                if ui.radio(v.display == id, "").on_hover_text("Display this node").clicked() { out.push(UiAction::SetDisplay(id.clone())); }
                if ui.selectable_label(v.selected == id, format!("{id}  {}", node.node_type)).clicked() { out.push(UiAction::Select(id.clone())); }
                let mut bypass = node.bypass;
                if ui.checkbox(&mut bypass, "").on_hover_text("Mute (bypass)").changed() {
                    out.push(UiAction::Edit { edit: Edit::Bypass { node: id.clone(), flag: bypass }, continuous: false });
                }
            });
        }
    });
}
