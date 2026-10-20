use eframe::egui::{self, Align2, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use crate::ui::{number_step, theme::Theme};

/// A 22 px value box in the design's style (input fill, 1 px stroke, 4 px radius); returns the outer and inner rects.
pub fn boxed(ui: &mut egui::Ui, width: f32, t: &Theme) -> (Rect, Rect) {
    let (r, _) = ui.allocate_exact_size(Vec2::new(width, 22.), Sense::hover());
    let (bg, stroke) = if ui.is_enabled() {
        (t.input, t.stroke)
    } else {
        (t.input.gamma_multiply(0.5), t.stroke.gamma_multiply(0.5))
    };
    ui.painter().rect(r, 4., bg, Stroke::new(1.0_f32, stroke), StrokeKind::Inside);
    (r, r.shrink2(Vec2::new(2., 1.)))
}

pub const TAB_REQUEST: egui::Id = egui::Id::NULL;

/// Number fields drawn this pass in layout order (the previous pass's list is what Tab navigates).
pub fn register_field(ctx: &egui::Context, id: egui::Id, enabled: bool) {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| {
        let cur = d.get_temp_mut_or_default::<(u64, Vec<egui::Id>, Vec<egui::Id>)>(egui::Id::new("field_order"));
        if cur.0 != pass { cur.0 = pass; cur.2 = std::mem::take(&mut cur.1); }
        if enabled { cur.1.push(id); }
    });
}

pub fn last_field_order(ctx: &egui::Context) -> Vec<egui::Id> {
    // mid-pass the current list is partial, so navigate by the previous pass's complete one
    ctx.data(|d| d.get_temp::<(u64, Vec<egui::Id>, Vec<egui::Id>)>(egui::Id::new("field_order"))).map(|(_, _, prev)| prev).unwrap_or_default()
}

/// The field Tab (`dir` 1) or Shift+Tab (-1) moves to from `current`; none past either end.
pub fn tab_target(order: &[egui::Id], current: egui::Id, dir: i32) -> Option<egui::Id> {
    let i = order.iter().position(|x| *x == current)? as i32 + dir;
    usize::try_from(i).ok().and_then(|i| order.get(i)).copied()
}

/// A number you drag sideways to change and click to type, painted in `rect` (the design's mono 12 px value).
#[allow(clippy::too_many_arguments)] // painter-style helper, called from three places
pub fn drag_number(ui: &mut egui::Ui, rect: Rect, id: egui::Id, v: &mut f64, speed: f64, range: &std::ops::RangeInclusive<f64>, integer: bool, align: Align2, t: &Theme, units: Option<crate::units::UnitSettings>) -> bool {
    let edit_id = id.with("edit");
    let editing: Option<String> = ui.ctx().data(|d| d.get_temp(edit_id));
    let mut changed = false;
    let clamp = |x: f64| { let x = x.clamp(*range.start(), *range.end()); if integer { x.round() } else { x } };
    let anchor = match align { Align2::LEFT_CENTER => Pos2::new(rect.left() + 2., rect.center().y), Align2::RIGHT_CENTER => Pos2::new(rect.right() - 2., rect.center().y), _ => rect.center() };
    register_field(ui.ctx(), id, ui.is_enabled());
    let mut started = ui.ctx().data(|d| d.get_temp::<bool>(id.with("select_on_edit"))).unwrap_or(false);
    if started { ui.ctx().data_mut(|d| d.remove::<bool>(id.with("select_on_edit"))); }
    let editing = editing.or_else(|| {
        // Tab from the neighbouring field asked for this one to start editing
        if ui.ctx().data(|d| d.get_temp::<egui::Id>(TAB_REQUEST)) != Some(id) { return None; }
        ui.ctx().data_mut(|d| d.remove::<egui::Id>(TAB_REQUEST));
        started = true;
        Some(if let Some(u) = units { u.format(*v) } else if integer { format!("{}", *v as i64) } else { format!("{v}") })
    });
    if let Some(mut text) = editing {
        // Tab / Shift+Tab commit this field and start editing the next / previous one in layout order
        let tab = ui.input_mut(|i| if i.consume_key(egui::Modifiers::SHIFT, egui::Key::Tab) { Some(-1) }
            else if i.consume_key(egui::Modifiers::NONE, egui::Key::Tab) { Some(1) } else { None });
        let text_id = id.with("text");
        let horizontal_align = match align { Align2::RIGHT_CENTER => egui::Align::Max, Align2::LEFT_CENTER => egui::Align::Min, _ => egui::Align::Center };
        let response = ui.put(rect, egui::TextEdit::singleline(&mut text).font(egui::FontId::monospace(12.)).frame(false).margin(Vec2::new(2., 0.)).horizontal_align(horizontal_align).id(text_id));
        if !response.has_focus() && !response.lost_focus() { response.request_focus(); }
        if started {
            let mut state = egui::TextEdit::load_state(ui.ctx(), text_id).unwrap_or_default();
            state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(text.chars().count()))));
            state.store(ui.ctx(), text_id);
        }
        if let Some(dir) = tab {
            if let Some(next) = tab_target(&last_field_order(ui.ctx()), id, dir) { ui.ctx().data_mut(|d| d.insert_temp(TAB_REQUEST, next)); }
        }
        if response.lost_focus() || tab.is_some() {
            if let Some(u) = units {
                if let Some(x) = u.parse(&text) { *v = clamp(x); changed = true; }
            } else if let Ok(x) = text.trim().parse::<f64>() {
                *v = clamp(x); changed = true;
            }
            ui.ctx().data_mut(|d| d.remove::<String>(edit_id));
        } else { ui.ctx().data_mut(|d| d.insert_temp(edit_id, text)); }
    } else {
        let response = ui.interact(rect, id, Sense::click_and_drag());
        if response.hovered() || response.dragged() { ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal); }
        if response.dragged() {
            let speed = speed * number_step::drag_multiplier(ui.input(|i| i.modifiers), *v);
            let step = if let Some(u) = units {
                response.drag_delta().x as f64 * speed * u.display.metres() / u.unit_scale
            } else {
                response.drag_delta().x as f64 * speed
            };
            if integer {
                // accumulate sub-unit drags so slow integer drags still move
                let acc_id = id.with("drag_acc");
                let acc = ui.ctx().data(|d| d.get_temp::<f64>(acc_id)).unwrap_or(0.) + step;
                let whole = acc.trunc();
                ui.ctx().data_mut(|d| d.insert_temp(acc_id, acc - whole));
                if whole != 0. { let next = clamp(*v + whole); if next != *v { *v = next; changed = true; } }
            } else if step != 0. { let next = clamp(*v + step); if next != *v { *v = next; changed = true; } }
        } else if integer { ui.ctx().data_mut(|d| d.remove::<f64>(id.with("drag_acc"))); }
        if response.clicked() {
            let text = if let Some(u) = units {
                u.format(*v)
            } else if integer {
                format!("{}", *v as i64)
            } else {
                format!("{v}")
            };
            ui.ctx().data_mut(|d| d.insert_temp(edit_id, text));
            ui.ctx().data_mut(|d| d.insert_temp(id.with("select_on_edit"), true));
        }
        let text = if let Some(u) = units {
            u.format(*v)
        } else if integer {
            format!("{}", *v as i64)
        } else {
            format!("{:.2}", *v)
        };
        ui.painter().text(anchor, align, text, egui::FontId::monospace(12.), if ui.is_enabled() { t.text } else { t.faint });
    }
    changed
}

/// A boxed number field: drag/double-click text between a decrement button (left end) and an increment button
/// (right end), plus gated wheel stepping (see ui::number_step). Fields too narrow for the buttons keep drag + wheel.
#[allow(clippy::too_many_arguments)]
pub fn number_field(ui: &mut egui::Ui, width: f32, id: egui::Id, v: &mut f64, speed: f64, range: &std::ops::RangeInclusive<f64>, integer: bool, soft: Option<(f64, f64)>, align: Align2, t: &Theme, units: Option<crate::units::UnitSettings>) -> (bool, Rect) {
    let (outer, inner) = boxed(ui, width, t);
    let buttons = number_step::layout(outer);
    // the value reads centred between the - and + buttons
    let align = if buttons.is_some() { Align2::CENTER_CENTER } else { align };
    let text = buttons.map_or(inner, |(_, _, text)| Rect::from_min_max(Pos2::new(text.left() + 2., inner.top()), Pos2::new(text.right() - 2., inner.bottom())));
    let mut changed = drag_number(ui, text, id, v, speed, range, integer, align, t, units);
    let steps = buttons.map_or(0, |(dec, inc, _)| number_step::stepper(ui, dec, inc, id, t)) + step_by_wheel(ui, outer, id);
    changed |= apply_field_steps(ui, id, v, steps, range, integer, soft, units);
    (changed, outer)
}

pub fn step_by_wheel(ui: &egui::Ui, rect: Rect, id: egui::Id) -> i32 {
    let editing = ui.ctx().data(|d| d.get_temp::<String>(id.with("edit")).is_some());
    number_step::wheel(ui, rect, id, editing)
}

#[allow(clippy::too_many_arguments)]
pub fn apply_field_steps(ui: &egui::Ui, id: egui::Id, v: &mut f64, steps: i32, range: &std::ops::RangeInclusive<f64>, integer: bool, soft: Option<(f64, f64)>, units: Option<crate::units::UnitSettings>) -> bool {
    if steps == 0 { return false; }
    let mods = ui.input(|i| i.modifiers);
    let step = number_step::step_size(integer, soft, *v) * number_step::modifier_scale(mods, integer);
    let next = number_step::apply_step(*v, step, steps as f64, range, integer);
    if next == *v { return false; }
    *v = next;
    let edit_id = id.with("edit");
    if ui.ctx().data(|d| d.get_temp::<String>(edit_id).is_some()) {
        // keep the text being typed in step with the wheel
        let text = if let Some(u) = units { u.format(next) } else if integer { format!("{}", next as i64) } else { format!("{next}") };
        ui.ctx().data_mut(|d| d.insert_temp(edit_id, text));
    }
    true
}
