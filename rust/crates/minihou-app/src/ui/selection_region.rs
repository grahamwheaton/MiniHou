//! Box selection gesture, armed with B while the viewport is hovered.
use super::{Actions, UiAction, View};
use eframe::egui::{self, pos2, Rect, Stroke, StrokeKind};

#[derive(Clone, Default)]
struct State { armed: bool, start: Option<egui::Pos2>, extend: bool, subtract: bool }
fn id() -> egui::Id { egui::Id::new("viewport_region_select") }
pub fn active(ctx: &egui::Context) -> bool { ctx.data(|d| d.get_temp::<State>(id()).is_some_and(|s| s.armed)) }
pub fn show(ui: &mut egui::Ui, viewport: Rect, v: &View, out: &mut Actions) {
    let ctx = ui.ctx().clone(); let mut s: State = ctx.data_mut(|d| d.get_temp(id())).unwrap_or_default();
    if !s.armed && v.editor.tool == super::Tool::Select && ui.input(|i| i.key_pressed(egui::Key::B) && !i.modifiers.any()) { s.armed = true; }
    if !s.armed { return; }
    for event in ui.input(|i| i.events.clone()) { match event {
        egui::Event::Key { key: egui::Key::Escape, pressed: true, .. } => s = State::default(),
        egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers } if viewport.contains(pos) => {
            s.start = Some(pos); s.extend = modifiers.shift; s.subtract = modifiers.ctrl;
        }
        egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, .. } => {
            if let Some(start) = s.start { let rect = Rect::from_two_pos(start, pos); if rect.width() > 3. && rect.height() > 3. { out.push(UiAction::RegionSelect { rect, extend: s.extend, subtract: s.subtract }); } }
            s = State::default();
        }
        _ => {}
    } }
    if let (Some(start), Some(now)) = (s.start, ctx.input(|i| i.pointer.hover_pos())) {
        let r = Rect::from_two_pos(start, now).intersect(viewport);
        let p = ui.painter_at(viewport); p.rect(r, 0., v.theme.accent_bg, Stroke::new(1_f32, v.theme.accent), StrokeKind::Inside);
    } else if s.armed {
        let r = Rect::from_min_size(pos2(viewport.left() + 56., viewport.top() + 48.), egui::vec2(112., 24.));
        let p = ui.painter_at(viewport); p.rect_filled(r, 2., v.theme.panel);
        p.text(r.center(), egui::Align2::CENTER_CENTER, "Box select · Esc", egui::FontId::monospace(11.), v.theme.text);
    }
    ctx.data_mut(|d| d.insert_temp(id(), s));
}
