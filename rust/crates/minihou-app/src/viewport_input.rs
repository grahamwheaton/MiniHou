//! Viewport input ownership independent of document and renderer state.
use eframe::egui::{self, Key, Pos2, Rect};
use minihou_app::Camera;

#[derive(Clone, Default)]
struct Fly { press: Option<Pos2>, active: bool, speed: f32 }

fn activates(press: Pos2, pointer: Pos2, movement: bool) -> bool {
    movement || press.distance(pointer) > 3.
}
fn movement_key(key: Key) -> bool {
    matches!(key, Key::W | Key::A | Key::S | Key::D | Key::E | Key::Q | Key::Space)
}
const MODAL_CLICK: &str = "modal_confirm_click";

/// Claim the press while a modal owns it, even if the modal finishes in this frame.
pub fn claim_modal_click(ctx: &egui::Context, modal_was_active: bool) {
    if modal_was_active && ctx.input(|i| i.pointer.primary_pressed()) {
        ctx.data_mut(|d| d.insert_temp(egui::Id::new(MODAL_CLICK), true));
    }
}
/// A modal's confirming click belongs to the operation through release.
pub fn selection_click_available(ctx: &egui::Context) -> bool {
    !ctx.data(|d| d.get_temp::<bool>(egui::Id::new(MODAL_CLICK))).unwrap_or(false)
}
pub fn release_modal_click(ctx: &egui::Context) {
    if ctx.input(|i| i.pointer.primary_released()) {
        ctx.data_mut(|d| d.remove::<bool>(egui::Id::new(MODAL_CLICK)));
    }
}
pub fn fly(ctx: &egui::Context, rect: Rect, camera: &mut Camera, enabled: bool) -> bool {
    let id = egui::Id::new("viewport_fly");
    let mut state = ctx.data(|d| d.get_temp::<Fly>(id)).unwrap_or_default();
    let (down, pressed, pointer, modifiers, focused) = ctx.input(|i|
        (i.pointer.secondary_down(), i.pointer.secondary_pressed(), i.pointer.latest_pos(), i.modifiers, i.focused));
    let was_active = state.active;
    if !enabled || !focused || !down {
        state.press = None; state.active = false;
    } else {
        if pressed && !modifiers.alt && !modifiers.ctrl
            && pointer.is_some_and(|p| rect.contains(p) && crate::ui::viewport_ui::selection_canvas_contains(rect, p)) {
            state.press = pointer;
        }
        if let (Some(start), Some(pointer)) = (state.press, pointer) {
            let moving = !ctx.wants_keyboard_input() && ctx.input(|i|
                [Key::W, Key::A, Key::S, Key::D, Key::E, Key::Q, Key::Space].into_iter().any(|key| i.key_down(key)));
            state.active |= activates(start, pointer, moving);
        }
    }
    if state.active {
        if state.speed == 0. { state.speed = (camera.distance() * 0.5).max(0.1); }
        let (motion, dt, scroll) = ctx.input(|i| {
            let axis = |plus, minus| i.key_down(plus) as u8 as f32 - i.key_down(minus) as u8 as f32;
            ([axis(Key::W, Key::S), axis(Key::D, Key::A), axis(Key::E, Key::Q) + i.key_down(Key::Space) as u8 as f32 - i.modifiers.ctrl as u8 as f32],
                i.stable_dt.min(0.05), i.raw_scroll_delta.y)
        });
        state.speed = (state.speed * (scroll * 0.0015).exp()).clamp(0.001, 100000.);
        let mut delta = ctx.input(|i| i.pointer.delta());
        crate::wrap_drag(ctx, rect, &mut delta);
        camera.fly([delta.x, delta.y], motion, state.speed * dt * if modifiers.shift { 4. } else { 1. });
        ctx.input_mut(|i| {
            i.events.retain(|e| !matches!(e, egui::Event::Key { key, .. } if movement_key(*key)));
            i.smooth_scroll_delta = egui::Vec2::ZERO; i.raw_scroll_delta = egui::Vec2::ZERO;
        });
        ctx.request_repaint();
    }
    let consumed = state.active || was_active;
    if consumed { ctx.request_repaint(); }
    ctx.data_mut(|d| d.insert_temp(id, state));
    consumed
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn ordinary_click_remains_available_for_selection() {
        let ctx = egui::Context::default();
        let pos = egui::pos2(100., 100.);
        let _ = ctx.run(egui::RawInput { events: vec![egui::Event::PointerButton {
            pos, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE,
        }], ..Default::default() }, |ctx| {
            claim_modal_click(ctx, false);
            assert!(selection_click_available(ctx));
        });
    }
    #[test] fn extrude_confirm_click_keeps_selection_through_release() {
        use crate::ui::modal_model::{Kind, State};
        let ctx = egui::Context::default();
        let pos = egui::pos2(100., 100.);
        let mut modal = Some(State::new(Kind::Extrude, pos, 1.));
        let mut selected_face = 3;
        let button = |pressed| egui::Event::PointerButton {
            pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE,
        };
        let _ = ctx.run(egui::RawInput { events: vec![egui::Event::PointerMoved(pos), button(true)], ..Default::default() }, |ctx| {
            let modal_frame = modal.is_some();
            assert!(modal_frame);
            modal.take(); // update_model_modal confirms the extrude on this press.
            claim_modal_click(ctx, modal_frame);
            if selection_click_available(ctx) { selected_face = 8; }
            release_modal_click(ctx);
        });
        assert_eq!(selected_face, 3);
        let _ = ctx.run(egui::RawInput { events: vec![button(false)], ..Default::default() }, |ctx| {
            if selection_click_available(ctx) { selected_face = 8; }
            release_modal_click(ctx);
        });
        assert_eq!(selected_face, 3);
        let _ = ctx.run(egui::RawInput::default(), |ctx| assert!(selection_click_available(ctx)));
    }
    #[test] fn rmb_click_waits_for_drag_or_movement() {
        let start = egui::pos2(10., 10.);
        assert!(!activates(start, start, false));
        assert!(!activates(start, start + egui::vec2(2., 0.), false));
        assert!(activates(start, start + egui::vec2(4., 0.), false));
        assert!(activates(start, start, true));
    }
    #[test] fn held_rmb_owns_keys_wheel_and_release_but_plain_click_does_not() {
        let ctx = egui::Context::default();
        let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(800., 600.));
        let pos = egui::pos2(300., 250.);
        let mut camera = Camera::frame([0.; 3], 2.);
        let button = |pressed, modifiers| egui::Event::PointerButton { pos, button: egui::PointerButton::Secondary, pressed, modifiers };
        let run = |events, enabled, camera: &mut Camera| {
            let mut consumed = false;
            let _ = ctx.run(egui::RawInput { events, ..Default::default() }, |ctx| {
                consumed = fly(ctx, rect, camera, enabled);
            });
            consumed
        };
        assert!(!run(vec![egui::Event::PointerMoved(pos), button(true, egui::Modifiers::NONE)], true, &mut camera));
        assert!(!run(vec![button(false, egui::Modifiers::NONE)], true, &mut camera));
        let eye = camera.eye();
        assert!(run(vec![button(true, egui::Modifiers::NONE), egui::Event::Key { key: Key::W,
            physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }], true, &mut camera));
        assert_ne!(eye, camera.eye());
        assert!(!ctx.input(|i| i.key_pressed(Key::W)));
        let speed = ctx.data(|d| d.get_temp::<Fly>(egui::Id::new("viewport_fly")).unwrap().speed);
        let distance = camera.distance();
        assert!(run(vec![egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0., 100.), modifiers: egui::Modifiers::NONE }], true, &mut camera));
        assert!(ctx.data(|d| d.get_temp::<Fly>(egui::Id::new("viewport_fly")).unwrap().speed) > speed);
        assert_eq!(camera.distance(), distance);
        assert!(run(vec![button(false, egui::Modifiers::NONE)], true, &mut camera));
        assert!(!run(vec![], true, &mut camera));
        let eye = camera.eye();
        assert!(!run(vec![button(true, egui::Modifiers::NONE)], false, &mut camera));
        assert_eq!(eye, camera.eye());
    }
    #[test] fn ctrl_rmb_is_reserved_for_lasso() {
        let ctx = egui::Context::default();
        let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(800., 600.));
        let pos = egui::pos2(300., 250.);
        let mut camera = Camera::frame([0.; 3], 2.);
        let _ = ctx.run(egui::RawInput { modifiers: egui::Modifiers::CTRL, events: vec![
            egui::Event::PointerMoved(pos), egui::Event::PointerButton { pos, button: egui::PointerButton::Secondary,
                pressed: true, modifiers: egui::Modifiers::CTRL }, egui::Event::PointerMoved(pos + egui::vec2(40.,0.))], ..Default::default() }, |ctx| {
            assert!(!fly(ctx, rect, &mut camera, true));
        });
    }
    #[test] fn releasing_handle_ownership_allows_select_tool_region_drag() {
        use crate::ui::{self, Actions, Tool, UiAction};
        let ctx = egui::Context::default();
        let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(800.,600.));
        assert!(ui::transform_gizmo::active_kind(&ctx).is_none());
        assert_eq!(ui::modal_transform::BaseStage::enter(ui::modal_transform::base_snap_shortcut(
            Key::B,ui::transform_gizmo::active_kind(&ctx).is_some())),ui::modal_transform::BaseStage::Off);
        let mut fixture = ui::testing::Fixture::new();
        fixture.editor.tool = Tool::Move;
        fixture.camera.projection = [[1.,0.,0.,0.],[0.,1.,0.,0.],[0.,0.,1.,0.],[0.,0.,0.,1.]];
        let center = rect.center();
        let button = |pos, pressed| egui::Event::PointerButton { pos, pressed,
            button: egui::PointerButton::Primary, modifiers: egui::Modifiers::NONE };
        let _ = ctx.run(egui::RawInput { screen_rect: Some(rect), events: vec![egui::Event::PointerMoved(center),
            button(center,true)], ..Default::default() }, |ctx| {
            let mut view = fixture.view(); view.selection_center = Some([0.;3]);
            egui::CentralPanel::default().show(ctx, |ui| ui::transform_gizmo::show(ui,rect,&view,&mut Actions::default()));
        });
        assert!(ui::transform_gizmo::active(&ctx));
        assert_eq!(ui::transform_gizmo::active_kind(&ctx),Some(Tool::Move));
        assert_eq!(ui::modal_transform::BaseStage::enter(ui::modal_transform::base_snap_shortcut(
            Key::B,ui::transform_gizmo::active_kind(&ctx).is_some())),ui::modal_transform::BaseStage::Base);
        ui::transform_gizmo::finish_input(&ctx);
        assert!(!ui::transform_gizmo::active(&ctx));
        fixture.editor.tool = Tool::Select;
        let start = egui::pos2(100.,100.); let end = egui::pos2(220.,220.);
        let mut out = Actions::default();
        let _ = ctx.run(egui::RawInput { screen_rect: Some(rect), events: vec![button(center,false),
            button(start,true), egui::Event::PointerMoved(end), button(end,false)], ..Default::default() }, |ctx| {
            assert!(ui::selection_region::handle(ctx,rect,&fixture.view(),&mut out));
        });
        assert!(matches!(out.0.as_slice(), [UiAction::RegionSelect { .. }]));
    }
}
