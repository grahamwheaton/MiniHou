//! The Blender status chip and connection strip.
use super::{Actions, UiAction, View};
use eframe::egui::{self, Align2, FontId, Sense, Stroke, StrokeKind, Vec2};
#[path = "../blender_link.rs"]
pub(crate) mod blender_link;

pub(crate) fn connection_message(listening: bool, clients: &[String], previous: &str, disconnected: bool) -> String {
    if !listening {
        "Link listener stopped. Click Restart link to listen on 127.0.0.1:8765 again.".into()
    } else if clients.is_empty() {
        if disconnected || previous.starts_with("Blender disconnected") {
            "Blender disconnected; listening on 127.0.0.1:8765 for reconnection.".into()
        } else { "Listening on 127.0.0.1:8765; waiting for Blender.".into() }
    } else { format!("Blender connected; listening on 127.0.0.1:8765; {} client(s).", clients.len()) }
}

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
        format!("{}\nOpen Blender or connect in Blender's MiniHou tab.", v.blender.link_message)
    } else {
        format!("Connected: {}", v.blender.clients.join(", "))
    };
    if response.on_hover_text(tooltip).clicked() { out.push(UiAction::BlenderBar(true)); }
}

pub(crate) fn selected_index(ctx: &egui::Context, len: usize) -> usize {
    let id = egui::Id::new("blender_install_pick");
    ctx.data_mut(|d| d.get_persisted::<usize>(id).unwrap_or(0).min(len.saturating_sub(1)))
}

pub fn bar(ui: &mut egui::Ui, v: &View, out: &mut Actions, shown: &mut bool) {
    if !*shown { return; }
    ui.horizontal_wrapped(|ui| {
        let mut selected = selected_index(ui.ctx(), v.blender.installs.len());
        ui.add_enabled_ui(!v.blender.busy, |ui| {
            if !v.blender.installs.is_empty() {
                egui::ComboBox::from_id_salt("blender_install_pick_combo")
                    .selected_text(&v.blender.installs[selected].0)
                    .show_ui(ui, |ui| {
                        for (index, (name, path)) in v.blender.installs.iter().enumerate() {
                            ui.selectable_value(&mut selected, index, name).on_hover_text(path);
                        }
                    });
                ui.ctx().data_mut(|d| d.insert_persisted(egui::Id::new("blender_install_pick"), selected));
            }
            let available = !v.blender.installs.is_empty();
            if ui.add_enabled(available, egui::Button::new("Install add-on"))
                .on_hover_text(format!("Install and enable add-on V{} in the selected Blender, in the background", blender_link::addon_version())).clicked() {
                out.push(UiAction::BlenderInstall(selected));
            }
            ui.label(egui::RichText::new(format!("Latest add-on: V{}", blender_link::addon_version())).color(v.theme.dim))
                .on_hover_text("Blender's modifier panel header shows the installed version as \"MiniHou V<version>\"");
            if ui.add_enabled(available, egui::Button::new("Open Blender"))
                .on_hover_text("Launch the selected Blender and connect to this engine").clicked() {
                out.push(UiAction::BlenderOpen);
            }
            if ui.button("Save ZIP").on_hover_text("Save the add-on for manual installation").clicked() {
                out.push(UiAction::BlenderDownloadAddon);
            }
            if ui.button("Detect installs").clicked() { out.push(UiAction::BlenderDetect); }
        });
        if ui.button("Restart link").clicked() { out.push(UiAction::BlenderRestartLink); }
        if ui.button("Close").clicked() { *shown = false; out.push(UiAction::BlenderBar(false)); }
        if v.blender.busy { ui.spinner(); }
    });
    ui.label(&v.blender.link_message);
    if !v.blender.clients.is_empty() {
        ui.label(format!("Connected clients ({}): {}", v.blender.clients.len(), v.blender.clients.join(", ")));
    }
    if !v.blender.message.is_empty() { ui.label(&v.blender.message); }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::testing::Fixture;

    #[test]
    fn link_status_tracks_connect_disconnect_and_reconnect() {
        let waiting = connection_message(true, &[], "", false);
        assert!(waiting.starts_with("Listening"));
        let clients = vec!["blender 5.2 (protocol 1)".into()];
        let connected = connection_message(true, &clients, &waiting, false);
        assert!(connected.starts_with("Blender connected"));
        let disconnected = connection_message(true, &[], &connected, true);
        assert!(disconnected.starts_with("Blender disconnected"));
        assert_eq!(connection_message(true, &[], &disconnected, false), disconnected);
        assert_eq!(connection_message(true, &clients, &disconnected, false), connected);
        assert!(connection_message(false, &[], &disconnected, false).starts_with("Link listener stopped"));
    }

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
            egui::CentralPanel::default().show(ctx, |ui| { index = selected_index(ui.ctx(), 2); });
        });
        assert_eq!(index, 1);
    }
}
