//! The Blender status chip and connection strip.
use super::{Actions, UiAction, View};
use eframe::egui::{self, Align2, FontId, Rect, Sense, Stroke, StrokeKind, Vec2};
#[path = "../blender_link.rs"]
pub(crate) mod blender_link;

pub fn indicator(ui: &mut egui::Ui, v: &View, out: &mut Actions) {
    let connected = v.blender.connected();
    let label = if connected { "Blender" } else { "Blender —" };
    let width = if connected { 77. } else { 88. };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 18.), Sense::click());
    let p = ui.painter();
    p.rect_stroke(rect, 9., Stroke::new(1.0_f32, v.theme.stroke), StrokeKind::Inside);
    p.circle_filled(rect.left_center() + Vec2::new(11., 0.), 3.5,
        if connected { v.theme.ok } else { v.theme.faint });
    p.text(rect.left_center() + Vec2::new(21., 0.), Align2::LEFT_CENTER, label,
        FontId::proportional(11.), v.theme.dim);
    let tooltip = if v.blender.clients.is_empty() {
        "No Blender connected. Open Blender or connect in Blender's MiniHou tab.".to_owned()
    } else {
        format!("Connected: {}", v.blender.clients.join(", "))
    };
    if response.on_hover_text(tooltip).clicked() { out.push(UiAction::BlenderBar(true)); }
}

fn button(ui: &mut egui::Ui, rect: Rect, text: &str, enabled: bool, v: &View, tooltip: &str) -> bool {
    let response = ui.interact(rect, ui.next_auto_id(), if enabled { Sense::click() } else { Sense::hover() });
    let p = ui.painter();
    p.rect_filled(rect, 4., if response.hovered() && enabled { v.theme.input } else { v.theme.panel });
    p.rect_stroke(rect, 4., Stroke::new(1.0_f32, v.theme.stroke), StrokeKind::Inside);
    p.text(rect.center(), Align2::CENTER_CENTER, text, FontId::proportional(12.),
        if enabled { v.theme.text } else { v.theme.faint });
    response.on_hover_text(tooltip).clicked() && enabled
}

fn selected_index(ui: &egui::Ui, len: usize) -> usize {
    let id = egui::Id::new("blender_install_pick");
    ui.ctx().data_mut(|d| d.get_persisted::<usize>(id).unwrap_or(0).min(len.saturating_sub(1)))
}

pub fn bar(ui: &mut egui::Ui, v: &View, out: &mut Actions, shown: &mut bool) {
    if !*shown { return; }
    let (strip, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 30.), Sense::hover());
    let p = ui.painter();
    p.rect_filled(strip, 0., v.theme.panel2);
    p.line_segment([strip.left_bottom() - Vec2::new(0., 0.5), strip.right_bottom() - Vec2::new(0., 0.5)],
        Stroke::new(1.0_f32, v.theme.stroke));

    let y = strip.center().y;
    let mut right = strip.right() - 10.;
    let close = Rect::from_center_size(egui::pos2(right - 10., y), Vec2::new(20., 22.));
    right = close.left() - 8.;
    let zip = Rect::from_min_size(egui::pos2(right - 103., y - 11.), Vec2::new(103., 22.));
    right = zip.left() - 8.;
    let open = Rect::from_min_size(egui::pos2(right - 103., y - 11.), Vec2::new(103., 22.));
    right = open.left() - 8.;
    let install = Rect::from_min_size(egui::pos2(right - 103., y - 11.), Vec2::new(103., 22.));
    right = install.left() - 8.;

    let selected = selected_index(ui, v.blender.installs.len());
    if v.blender.installs.len() > 1 {
        let pick_width = 172.;
        let pick = Rect::from_min_size(egui::pos2(right - pick_width, y - 11.), Vec2::new(pick_width, 22.));
        right = pick.left() - 8.;
        ui.scope_builder(egui::UiBuilder::new().max_rect(pick), |ui| {
            ui.set_min_size(pick.size());
            let mut choice = selected;
            egui::ComboBox::from_id_salt("blender_install_pick_combo")
                .width(pick_width - 28.)
                .selected_text(&v.blender.installs[choice].0)
                .show_ui(ui, |ui| {
                    for (index, (name, path)) in v.blender.installs.iter().enumerate() {
                        ui.selectable_value(&mut choice, index, name).on_hover_text(path);
                    }
                });
            if choice != selected {
                ui.ctx().data_mut(|d| d.insert_persisted(egui::Id::new("blender_install_pick"), choice));
            }
        });
    }

    let message = if v.blender.message.is_empty() { "Blender is not connected." } else { &v.blender.message };
    let text_rect = Rect::from_min_max(strip.min + Vec2::new(10., 0.), egui::pos2(right.max(strip.left() + 10.), strip.bottom()));
    ui.painter().with_clip_rect(text_rect).text(text_rect.left_center(), Align2::LEFT_CENTER,
        message, FontId::proportional(12.), v.theme.text);

    let can_launch = !v.blender.installs.is_empty();
    if button(ui, install, "Install add-on", can_launch, v,
        "Install and enable the add-on in the chosen Blender (starts it once in the background)") {
        out.push(UiAction::BlenderInstall(selected));
    }
    if button(ui, open, "Open Blender", can_launch, v,
        "Start Blender with the add-on already connected to this engine") {
        out.push(UiAction::BlenderOpen);
    }
    if button(ui, zip, "Download ZIP", true, v,
        "Download, then use Blender Preferences > Add-ons > Install from Disk") {
        out.push(UiAction::BlenderDownloadAddon);
    }
    if button(ui, close, "×", true, v,
        "Hide (click the Blender indicator in the top bar to bring it back)") {
        *shown = false;
        out.push(UiAction::BlenderBar(false));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::testing::Fixture;

    #[test]
    fn idle_widgets_do_not_emit_actions() {
        let fixture = Fixture::new();
        assert!(fixture.run(indicator).is_empty());
        assert!(fixture.run(|ui, v, out| bar(ui, v, out, &mut true)).is_empty());
    }

    #[test]
    fn selected_installation_stays_in_bounds() {
        let ctx = egui::Context::default();
        let id = egui::Id::new("blender_install_pick");
        ctx.data_mut(|d| d.insert_persisted(id, 10usize));
        let mut index = 0;
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| { index = selected_index(ui, 2); });
        });
        assert_eq!(index, 1);
    }
}
