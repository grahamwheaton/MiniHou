//! Per-node output target state and controls for the parameter panel.
use eframe::egui;

/// Select an output in the parameter panel without changing the graph document.
pub fn set_target(target: &mut Option<usize>, output: usize) {
    *target = Some(output);
}

pub fn target_button(ui: &mut egui::Ui, name: &str, selected: bool) -> egui::Response {
    let text = if selected { "●" } else { "○" };
    ui.add(egui::Button::new(egui::RichText::new(text).color(super::TARGET_TEAL))
        .selected(selected))
        .on_hover_text(format!("Set {name} as this node's output target"))
}

pub fn show(ui: &mut egui::Ui, node: &str, outputs: &[&str], accent: egui::Color32) {
    if outputs.is_empty() { return; }
    let id = egui::Id::new(("param_output_target", node));
    let mut target = ui.ctx().data_mut(|data| data.get_temp::<usize>(id));
    ui.add_space(5.);
    ui.label(egui::RichText::new("Outputs").strong().color(accent));
    for (index, name) in outputs.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.label(*name);
            if target_button(ui, name, target.unwrap_or(0) == index).clicked() {
                set_target(&mut target, index);
                ui.ctx().data_mut(|data| data.insert_temp(id, index));
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::set_target;

    #[test]
    fn selecting_an_output_replaces_the_current_target() {
        let mut target = Some(0);
        set_target(&mut target, 2);
        assert_eq!(target, Some(2));
    }
}
