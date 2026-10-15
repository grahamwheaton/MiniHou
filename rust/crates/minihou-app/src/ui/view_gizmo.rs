//! Camera-aligned navigation gizmo in the viewport's upper right corner.
use super::super::{Actions, UiAction, View, ViewPreset};
use eframe::egui::{self, Pos2, Rect, Sense, Stroke, Vec2};

#[derive(Clone, Copy, Debug)]
struct Ball { preset: ViewPreset, xy: Vec2, depth: f32, positive: bool, axis: usize }

fn axes(yaw: f32, pitch: f32) -> [Ball; 6] {
    let eye = [pitch.cos() * yaw.sin(), pitch.sin(), pitch.cos() * yaw.cos()];
    let right = [yaw.cos(), 0., -yaw.sin()];
    let up = [-pitch.sin() * yaw.sin(), pitch.cos(), -pitch.sin() * yaw.cos()];
    let specs = [
        (ViewPreset::Right, [1., 0., 0.], true, 0), (ViewPreset::Left, [-1., 0., 0.], false, 0),
        (ViewPreset::Top, [0., 1., 0.], true, 1), (ViewPreset::Bottom, [0., -1., 0.], false, 1),
        (ViewPreset::Front, [0., 0., 1.], true, 2), (ViewPreset::Back, [0., 0., -1.], false, 2),
    ];
    specs.map(|(preset, axis, positive, i)| Ball {
        preset,
        xy: Vec2::new(dot(axis, right) * 22., -dot(axis, up) * 22.),
        depth: dot(axis, eye), positive, axis: i,
    })
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 { a.iter().zip(b).map(|(a, b)| a * b).sum() }

fn nearest_view(yaw: f32, pitch: f32) -> ViewPreset {
    axes(yaw, pitch).into_iter().max_by(|a, b| a.depth.total_cmp(&b.depth)).unwrap().preset
}

pub fn show(ui: &mut egui::Ui, viewport: Rect, v: &View, out: &mut Actions) {
    let area = Rect::from_min_size(Pos2::new(viewport.right() - 88., viewport.top() + 48.), Vec2::splat(76.));
    let center = area.center();
    let mut balls = axes(v.camera.yaw, v.camera.pitch);
    balls.sort_by(|a, b| a.depth.total_cmp(&b.depth));
    let p = ui.painter();
    for b in &balls {
        let color = match b.axis { 0 => v.theme.x_axis, 1 => v.theme.y_axis, _ => v.theme.z_axis };
        if b.positive && b.xy.length() > 8. { p.line_segment([center, center + b.xy * 0.75], Stroke::new(2.4_f32, color)); }
    }
    p.circle_filled(center, 2.5, v.theme.dim);
    for b in &balls {
        let color = match b.axis { 0 => v.theme.x_axis, 1 => v.theme.y_axis, _ => v.theme.z_axis };
        let col = if b.positive { color } else { color.gamma_multiply(0.48) };
        let pos = center + b.xy;
        p.circle_filled(pos, if b.positive { 8. } else { 6. }, col);
        if b.positive { p.text(pos, egui::Align2::CENTER_CENTER, ["X", "Y", "Z"][b.axis], egui::FontId::monospace(10.), v.theme.panel); }
    }
    let response = ui.interact(area, egui::Id::new("vp_view_gizmo"), Sense::click_and_drag());
    let hit = |pos: Pos2| balls.iter().rev().find(|b| (center + b.xy).distance(pos) <= if b.positive { 9. } else { 7. }).map(|b| b.preset);
    if response.clicked() {
        if let Some(preset) = response.interact_pointer_pos().and_then(hit) { out.push(UiAction::ViewPreset(preset)); }
    }
    let drag_id = egui::Id::new("vp_gizmo_drag_background");
    if response.drag_started() {
        let bg = response.interact_pointer_pos().is_none_or(|pos| hit(pos).is_none());
        ui.ctx().data_mut(|d| d.insert_temp(drag_id, bg));
    }
    if response.dragged() && ui.ctx().data(|d| d.get_temp::<bool>(drag_id)).unwrap_or(false) {
        let delta = ui.ctx().input(|i| i.pointer.delta());
        if delta != Vec2::ZERO { out.push(UiAction::Orbit { yaw: delta.x * 0.4, pitch: delta.y * 0.4 }); }
    }
    if response.drag_stopped() { ui.ctx().data_mut(|d| d.remove::<bool>(drag_id)); }
    let label = Rect::from_min_size(Pos2::new(area.left(), area.bottom() + 2.), Vec2::new(76., 18.));
    let resp = ui.interact(label, egui::Id::new("vp_gizmo_projection"), Sense::click());
    if resp.hovered() { p.rect_filled(label, 3., v.theme.panel2); }
    p.text(label.center(), egui::Align2::CENTER_CENTER, if v.camera.ortho { "Ortho" } else { "Persp" }, egui::FontId::monospace(11.), v.theme.text);
    if resp.clicked() { out.push(UiAction::ViewPreset(if v.camera.ortho { ViewPreset::Perspective } else { nearest_view(v.camera.yaw, v.camera.pitch) })); }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_angles_put_x_right_and_z_toward_the_viewer() {
        let a = axes(0., 0.);
        assert!(a[0].xy.x > 0. && a[0].xy.y.abs() < 1e-5);
        assert!((a[4].depth - 1.).abs() < 1e-5);
        assert!(a[4].xy.length() < 1e-5);
    }

    #[test]
    fn default_camera_is_closest_to_front() {
        assert_eq!(nearest_view(0.6, 0.5), ViewPreset::Front);
        assert_eq!(nearest_view(std::f32::consts::FRAC_PI_2, 0.), ViewPreset::Right);
    }
}
