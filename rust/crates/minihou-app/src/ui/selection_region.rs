//! Box selection gesture, armed with B while the viewport is hovered.
use super::{Actions, UiAction, View};
use eframe::egui::{self, pos2, Rect, Stroke, StrokeKind};

#[derive(Clone, Default)]
struct State { armed: bool, start: Option<egui::Pos2>, extend: bool, subtract: bool }
fn id() -> egui::Id { egui::Id::new("viewport_region_select") }
pub fn active(ctx: &egui::Context) -> bool { ctx.data(|d| d.get_temp::<State>(id()).is_some_and(|s| s.armed)) }
pub fn arm(ctx: &egui::Context) {
    ctx.data_mut(|d| d.insert_temp(id(), State { armed: true, ..Default::default() }));
    ctx.request_repaint();
}
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

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, Key, PointerButton, Pos2, RawInput};

    #[test]
    fn box_gesture_requests_region_selection() {
        let f = crate::ui::testing::Fixture::new();
        let ctx = egui::Context::default();
        let viewport = Rect::from_min_max(pos2(0., 0.), pos2(800., 600.));
        let frame = |events: Vec<Event>| {
            let input = RawInput { screen_rect: Some(viewport), events, ..Default::default() };
            let mut out = Actions::default();
            let _ = ctx.run(input, |ctx| { egui::CentralPanel::default().show(ctx, |ui| show(ui, viewport, &f.view(), &mut out)); });
            out.0
        };
        let key = Event::Key { key: Key::B, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
        assert!(frame(vec![key]).is_empty());
        let a = Pos2::new(100., 100.); let b = Pos2::new(210., 160.);
        assert!(frame(vec![Event::PointerMoved(a), Event::PointerButton { pos: a, button: PointerButton::Primary, pressed: true, modifiers: Default::default() }]).is_empty());
        let actions = frame(vec![Event::PointerMoved(b), Event::PointerButton { pos: b, button: PointerButton::Primary, pressed: false, modifiers: Default::default() }]);
        assert!(matches!(&actions[..], [UiAction::RegionSelect { rect, extend: false, subtract: false }] if *rect == Rect::from_two_pos(a, b)));
    }
}
