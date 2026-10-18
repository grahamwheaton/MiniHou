//! Stepping for the parameter panel's number fields: the -/+ buttons at the field's two ends and Alt+wheel.
//!
//! Step size: integers step by 1; floats by a power of ten derived from the soft slider range
//! (range / 100, so 0..10 steps 0.1) or, without a range, from 10% of the value's magnitude
//! (5.0 steps 0.1... 50 steps 1; 0 steps 0.1). Shift is fine (x0.1, integers stay 1), Ctrl coarse (x10).
//! Results are clamped to the parameter's hard min/max.
//!
//! Wheel gating: the wheel only changes a value while the pointer is over the field AND Alt is
//! held or the field is being typed into, so ordinary wheel scrolling always scrolls the panel.
use eframe::egui::{self, Pos2, Rect, Sense, Stroke, Vec2};

use super::theme::Theme;

/// Width of each stepper button: decrement at the left end, increment at the right end of a number field.
pub const STEPPER_W: f32 = 20.;
/// Narrowest text area kept between the two buttons; narrower fields get no buttons (wheel and drag only).
pub const MIN_TEXT_W: f32 = 28.;
/// Held arrow: first repeat after this delay, then one step per interval (seconds).
pub const REPEAT_DELAY: f64 = 0.4;
pub const REPEAT_INTERVAL: f64 = 0.06;
/// Point-unit (touchpad) wheel distance that counts as one notch.
const POINTS_PER_NOTCH: f32 = 40.;

pub const WHEEL_HINT: &str = "The - / + buttons step the value (hold to repeat). Alt+wheel over the field, or the wheel while typing, steps it too. Shift: fine, Ctrl: coarse.";

fn pow10_floor(x: f64) -> f64 { 10f64.powf(x.log10().floor()) }

/// The base step (before modifiers) for a field.
pub fn step_size(integer: bool, soft_range: Option<(f64, f64)>, value: f64) -> f64 {
    if integer { return 1.; }
    if let Some((lo, hi)) = soft_range.filter(|(lo, hi)| hi > lo && (hi - lo).is_finite()) {
        return pow10_floor((hi - lo) / 100.).max(1e-6);
    }
    let magnitude = value.abs() * 0.1;
    if magnitude < 1e-6 || !magnitude.is_finite() { 0.1 } else { pow10_floor(magnitude).max(1e-6) }
}

/// Shift = fine x0.1, Ctrl = coarse x10. Integers never go below a step of 1.
pub fn modifier_scale(mods: egui::Modifiers, integer: bool) -> f64 {
    let mut scale = 1.;
    if mods.shift && !integer { scale *= 0.1; }
    if mods.ctrl || mods.command { scale *= 10.; }
    scale
}

/// Multiplier for a sideways drag: grows with the value's magnitude (10 per decade from 10 up, so 500 drags 100x faster than 5),
/// Shift is fine (x0.1), Ctrl or Alt coarse (x10).
pub fn drag_multiplier(mods: egui::Modifiers, value: f64) -> f64 {
    let magnitude = if value.is_finite() && value.abs() >= 10. { pow10_floor(value.abs()) } else { 1. };
    let mut scale = magnitude;
    if mods.shift { scale *= 0.1; }
    if mods.ctrl || mods.command || mods.alt { scale *= 10.; }
    scale
}

/// `value + steps * step`, clamped to `range`, rounded to integers or away from float noise.
pub fn apply_step(value: f64, step: f64, steps: f64, range: &std::ops::RangeInclusive<f64>, integer: bool) -> f64 {
    let next = value + step * steps;
    let next = if integer { next.round() } else { (next * 1e9).round() / 1e9 };
    next.clamp(*range.start(), *range.end())
}

/// How many steps a held arrow should have produced `held` seconds after the press (the press itself is one).
pub fn repeat_steps_due(held: f64) -> u32 {
    if held < REPEAT_DELAY { 1 } else { 2 + ((held - REPEAT_DELAY) / REPEAT_INTERVAL).floor() as u32 }
}

/// Whether the wheel may change a field's value (see the module docs).
pub fn wheel_steps_value(hovered: bool, alt: bool, editing: bool) -> bool { hovered && (alt || editing) }

/// The decrement (left end) and increment (right end) button rects and the text rect between them, or `None` when
/// the field is too narrow for buttons plus `MIN_TEXT_W` of text.
pub fn layout(outer: Rect) -> Option<(Rect, Rect, Rect)> {
    if outer.width() < 2. * STEPPER_W + MIN_TEXT_W { return None; }
    let (top, bottom) = (outer.top() + 1., outer.bottom() - 1.);
    let dec = Rect::from_min_max(Pos2::new(outer.left() + 1., top), Pos2::new(outer.left() + STEPPER_W, bottom));
    let inc = Rect::from_min_max(Pos2::new(outer.right() - STEPPER_W, top), Pos2::new(outer.right() - 1., bottom));
    Some((dec, inc, Rect::from_min_max(Pos2::new(dec.right(), outer.top()), Pos2::new(inc.left(), outer.bottom()))))
}

/// Draw both stepper buttons and return the number of steps (+/-) to apply this frame (press = 1, held = repeats).
pub fn stepper(ui: &mut egui::Ui, dec: Rect, inc: Rect, id: egui::Id, t: &Theme) -> i32 {
    let mut steps = 0;
    for (dir, r) in [(-1, dec), (1, inc)] {
        let bid = id.with(("step", dir));
        let response = ui.interact(r, bid, Sense::click_and_drag()).on_hover_text(WHEEL_HINT);
        let enabled = ui.is_enabled();
        let active = response.hovered() || response.is_pointer_button_down_on();
        let color = if !enabled { t.faint } else if active { t.text } else { t.dim };
        if active && enabled { ui.painter().rect_filled(r.shrink(1.), 2., t.stroke.gamma_multiply(if response.is_pointer_button_down_on() { 1. } else { 0.6 })); }
        let c = r.center();
        let (w, h) = (3.0 * dir as f32, 4.0);
        ui.painter().add(egui::Shape::convex_polygon(
            vec![Pos2::new(c.x - w, c.y - h), Pos2::new(c.x + w, c.y), Pos2::new(c.x - w, c.y + h)],
            color, Stroke::NONE));
        let state = bid.with("held");
        if response.is_pointer_button_down_on() && enabled {
            let now = ui.input(|i| i.time);
            let (start, fired) = ui.ctx().data(|d| d.get_temp::<(f64, u32)>(state)).unwrap_or((now, 0));
            let due = repeat_steps_due(now - start);
            if due > fired { steps += dir * (due - fired) as i32; }
            ui.ctx().data_mut(|d| d.insert_temp(state, (start, due.max(fired))));
            ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(REPEAT_INTERVAL / 2.));
        } else {
            ui.ctx().data_mut(|d| d.remove::<(f64, u32)>(state));
        }
    }
    steps
}

/// Wheel notches over `rect` this frame when gated in (and consumed so the panel does not also scroll).
pub fn wheel(ui: &egui::Ui, rect: Rect, id: egui::Id, editing: bool) -> i32 {
    if !ui.is_enabled() { return 0; }
    let hovered = ui.rect_contains_pointer(rect);
    let alt = ui.input(|i| i.modifiers.alt);
    if !wheel_steps_value(hovered, alt, editing) { return 0; }
    let acc_id = id.with("wheel_acc");
    let mut acc = ui.ctx().data(|d| d.get_temp::<f32>(acc_id)).unwrap_or(0.);
    let mut notches = 0;
    let mut any = false;
    ui.input(|i| for e in &i.events {
        if let egui::Event::MouseWheel { unit, delta, .. } = e {
            any = true;
            // Shift may arrive as horizontal scrolling on some platforms
            let d = if delta.y != 0. { delta.y } else { delta.x };
            match unit {
                egui::MouseWheelUnit::Point => {
                    acc += d;
                    while acc >= POINTS_PER_NOTCH { notches += 1; acc -= POINTS_PER_NOTCH; }
                    while acc <= -POINTS_PER_NOTCH { notches -= 1; acc += POINTS_PER_NOTCH; }
                }
                _ => notches += if d > 0. { d.round().max(1.) as i32 } else if d < 0. { d.round().min(-1.) as i32 } else { 0 },
            }
        }
    });
    if any {
        ui.ctx().data_mut(|d| d.insert_temp(acc_id, acc));
        ui.ctx().input_mut(|i| { i.smooth_scroll_delta = Vec2::ZERO; i.raw_scroll_delta = Vec2::ZERO; });
    }
    notches
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_sizes_follow_kind_range_and_magnitude() {
        assert_eq!(step_size(true, Some((0., 1000.)), 3.), 1.);
        assert!((step_size(false, Some((0., 10.)), 3.) - 0.1).abs() < 1e-12);
        assert!((step_size(false, Some((0., 1.)), 0.3) - 0.01).abs() < 1e-12);
        assert!((step_size(false, None, 0.) - 0.1).abs() < 1e-12);
        assert!((step_size(false, None, 5.) - 0.1).abs() < 1e-12);
        assert!((step_size(false, None, 50.) - 1.).abs() < 1e-12);
        assert!((step_size(false, None, -250.) - 10.).abs() < 1e-12);
    }
    #[test]
    fn modifiers_scale_and_steps_clamp() {
        let shift = egui::Modifiers { shift: true, ..Default::default() };
        assert_eq!(modifier_scale(egui::Modifiers::NONE, false), 1.);
        assert!((modifier_scale(shift, false) - 0.1).abs() < 1e-12);
        assert_eq!(modifier_scale(shift, true), 1.);
        assert_eq!(modifier_scale(egui::Modifiers::CTRL, true), 10.);
        let r = 0.0..=3.0;
        assert_eq!(apply_step(2., 1., 1., &r, true), 3.);
        assert_eq!(apply_step(3., 1., 1., &r, true), 3.);
        assert_eq!(apply_step(0., 1., -1., &r, true), 0.);
        assert_eq!(apply_step(1., 10., 1., &r, true), 3.);
        assert_eq!(apply_step(0.2, 0.1, 1., &(f64::NEG_INFINITY..=f64::INFINITY), false), 0.3);
    }
    #[test]
    fn held_arrow_repeats_after_a_delay() {
        assert_eq!(repeat_steps_due(0.), 1);
        assert_eq!(repeat_steps_due(REPEAT_DELAY - 0.01), 1);
        assert_eq!(repeat_steps_due(REPEAT_DELAY), 2);
        assert_eq!(repeat_steps_due(REPEAT_DELAY + REPEAT_INTERVAL * 3.5), 5);
    }
    #[test]
    fn buttons_sit_at_both_ends_and_need_room() {
        let outer = Rect::from_min_size(Pos2::new(10., 0.), Vec2::new(120., 22.));
        let (dec, inc, text) = layout(outer).unwrap();
        assert_eq!(dec.left(), 11.); assert_eq!(inc.right(), 129.);
        assert!(dec.width() >= STEPPER_W - 1. && inc.width() >= STEPPER_W - 1. && dec.height() >= 20.);
        assert!(text.left() >= dec.right() && text.right() <= inc.left() && text.width() >= MIN_TEXT_W);
        assert!(layout(Rect::from_min_size(Pos2::ZERO, Vec2::new(60., 22.))).is_none());
    }
    #[test]
    fn wheel_never_hijacks_plain_panel_scrolling() {
        assert!(!wheel_steps_value(true, false, false));
        assert!(wheel_steps_value(true, true, false));
        assert!(wheel_steps_value(true, false, true));
        assert!(!wheel_steps_value(false, true, true));
    }
    #[test]
    fn params_drag_multiplier_scales_with_magnitude_and_modifiers() {
        let none = egui::Modifiers::NONE;
        assert_eq!(drag_multiplier(none, 5.), 1.);
        assert_eq!(drag_multiplier(none, 0.), 1.);
        assert_eq!(drag_multiplier(none, -500.), 100.);
        assert!(drag_multiplier(egui::Modifiers::SHIFT, 500.) < drag_multiplier(none, 500.));
        assert_eq!(drag_multiplier(egui::Modifiers::CTRL, 5.), 10.);
        assert_eq!(drag_multiplier(egui::Modifiers::ALT, 5.), 10.);
        assert!((drag_multiplier(egui::Modifiers::SHIFT, 5.) - 0.1).abs() < 1e-12);
    }
}
