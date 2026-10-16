//! Viewport proxy control. The integrator places `menu` in the viewport header.
use super::{Actions, EditorChange, UiAction, View};
use eframe::egui::{self, RichText};
#[path = "../proxy.rs"]
pub mod proxy;

fn active_label(level: f32, auto: bool) -> String {
    if auto { return "Auto".into(); }
    proxy::LEVELS.iter().find(|(_, _, value, is_auto)| !is_auto && (level - value).abs() < 1e-6)
        .map_or_else(|| format!("{}%", (level * 100.0).round()), |(_, label, _, _)| (*label).into())
}

fn choose(level: f32, auto: bool, out: &mut Actions) {
    out.push(UiAction::Editor(EditorChange::ProxyLevel(level)));
    out.push(UiAction::Editor(EditorChange::ProxyAuto(auto)));
}

/// 26 px header dropdown; its choices mirror the old editor's proxy rows.
pub fn menu(ui: &mut egui::Ui, v: &View, out: &mut Actions, compact: bool) {
    let p = v.editor.proxy;
    let cluster_label = if compact { "LOD" } else { "Proxy: Clustered LOD" };
    let label = if compact { if p.auto { "Auto".into() } else { format!("{}%", (p.level * 100.).round()) } }
        else { format!("Proxy: {}  ▾", active_label(p.level, p.auto)) };
    let label = if p.clustered { cluster_label.to_owned() } else { label };
    ui.scope(|ui| {
        ui.style_mut().spacing.interact_size.y = 26.;
        ui.menu_button(RichText::new(label).size(12.).color(v.theme.text), |ui| {
            ui.set_min_width(180.);
            if ui.selectable_label(p.clustered, "Clustered LOD").clicked() {
                out.push(UiAction::Editor(EditorChange::ProxyClustered(true)));
                ui.close();
            }
            ui.separator();
            ui.label("Reduced triangles");
            for (_, label, level, auto) in proxy::LEVELS {
                let active = !p.clustered && if auto { p.auto } else { !p.auto && (p.level - level).abs() < 1e-6 };
                if ui.selectable_label(active, label).clicked() {
                    choose(level, auto, out);
                    out.push(UiAction::Editor(EditorChange::ProxyClustered(false)));
                    ui.close();
                }
            }
            ui.separator();
            ui.label(RichText::new("Blender triangle cap").size(11.).color(v.theme.dim));
            let mut cap = p.link_tris;
            if ui.add(egui::DragValue::new(&mut cap).range(0..=u32::MAX).speed(1_000.)).changed() {
                out.push(UiAction::Editor(EditorChange::ProxyLinkTris(cap)));
            }
            ui.label(RichText::new("0 = unlimited").size(11.).color(v.theme.dim));
            ui.separator();
            ui.label("Viewport clipping (world units)");
            let (mut near, mut far) = (p.clip_start, p.clip_end);
            let near_changed = ui.add(egui::DragValue::new(&mut near).range(1e-6..=1e8).speed(0.01).prefix("Clip Start  ")).changed();
            let far_changed = ui.add(egui::DragValue::new(&mut far).range(1e-6..=1e9).speed(1.0).prefix("Clip End  ")).changed();
            if near_changed || far_changed {
                out.push(UiAction::Editor(EditorChange::ClipPlanes(near, far)));
            }
        });
    });
}

/// Text for the viewport banner only when fewer triangles are being drawn.
pub fn banner(shown_tris: usize, total_tris: usize, level: f32, auto: bool) -> Option<String> {
    (shown_tris < total_tris).then(|| proxy::status_text(shown_tris, total_tris, level, auto))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::testing::Fixture;

    #[test]
    fn labels_include_auto_and_custom() {
        assert_eq!(active_label(0.1, true), "Auto");
        assert_eq!(active_label(0.25, false), "25%");
        assert_eq!(active_label(0.333, false), "33%");
    }

    #[test]
    fn a_choice_requests_both_settings() {
        let mut out = Actions::default();
        choose(0.1, true, &mut out);
        assert!(matches!(out.0.as_slice(), [UiAction::Editor(EditorChange::ProxyLevel(0.1)), UiAction::Editor(EditorChange::ProxyAuto(true))]));
    }

    #[test]
    fn banner_only_for_reduction() {
        assert_eq!(banner(12_000, 48_000, 0.25, false).as_deref(), Some("Reduced view: 12,000 of 48,000 triangles"));
        assert_eq!(banner(48_000, 48_000, 1.0, false), None);
    }

    #[test]
    fn menu_runs_without_action() {
        assert!(Fixture::new().run(|ui, v, out| menu(ui, v, out, false)).is_empty());
    }
}
