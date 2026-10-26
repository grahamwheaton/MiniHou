//! The Blender Object node's `object` field: a text box with a fuzzy dropdown of Blender object names
//! (object_names_v1). Requests go out only while the field has focus; the UI never waits for Blender.
//! Without a capable add-on connected it is a plain text box.
use eframe::egui::{self, Align2, FontId, Key, Order, Sense, Stroke, StrokeKind, Vec2};
use minihou_app::object_names::{self as names, Search};
use serde_json::{json, Value};

#[derive(Clone, Default)]
struct Field { search: Search, highlight: usize, open: bool, over_popup: bool, had_focus: bool,
    /// What the user is typing. It stays local until a name is picked: writing each partial name to
    /// the document would send it to Blender, which echoes back an empty object and wipes the field.
    draft: Option<String> }

static WAKE: std::sync::Once = std::sync::Once::new();

/// Returns true when the value changed (typed or picked).
pub fn field(ui: &mut egui::Ui, value: &mut Value, width: f32, theme: &super::theme::Theme) -> bool {
    let id = ui.id().with("blender_object_name");
    let available = names::available();
    let mut keys = (false, false, false);
    let state_id = id.with("state");
    let mut st: Field = ui.ctx().data_mut(|d| d.remove_temp(state_id)).unwrap_or_default();
    if !available { st.draft = None; }
    let mut text = st.draft.clone().unwrap_or_else(|| value.as_str().unwrap_or("").to_owned());
    if available && st.open && !st.search.results.is_empty() {
        // Arrow keys move the highlight instead of the caret while the dropdown is open.
        keys = ui.input_mut(|i| (i.consume_key(egui::Modifiers::NONE, Key::ArrowDown), i.consume_key(egui::Modifiers::NONE, Key::ArrowUp), i.key_pressed(Key::Enter)));
    }
    let response = ui.add_sized([width, 22.], egui::TextEdit::singleline(&mut text).id(id).desired_width(f32::INFINITY)
        .hint_text(if available { "type to search Blender objects" } else { "" }));
    let mut changed = response.changed();
    if !available { if changed { *value = json!(text); } return changed; }
    // Typing edits the draft only; the document changes when a name is chosen (below).
    if response.changed() { st.draft = Some(text.clone()); }
    changed = false;
    let now = ui.input(|i| i.time);
    // A Blender reply repaints the field at once instead of the field polling every frame.
    let ctx = ui.ctx().clone();
    WAKE.call_once(|| names::set_wake(move || ctx.request_repaint()));
    if response.gained_focus() { st.search.focus(&text, now); st.open = true; st.highlight = 0; }
    if response.changed() { st.search.edit(&text, now); st.highlight = 0; st.open = true; }
    for msg in names::take_incoming() { if let Some(r) = names::parse(&msg) { st.search.accept(r); } }
    // An unanswered query is abandoned after TIMEOUT, so the field cannot wait (and repaint) forever.
    st.search.expire(now);
    if response.has_focus() || st.over_popup {
        if let Some(q) = st.search.poll(now) { names::send(q); }
    }
    if st.search.debouncing() { ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(names::DEBOUNCE)); }
    if let Some(left) = st.search.expires_in(now) { ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(left)); }
    let len = st.search.results.len().min(names::LIMIT);
    if keys.0 { st.highlight = names::navigate(st.highlight, len, names::Key::Down).0; }
    if keys.1 { st.highlight = names::navigate(st.highlight, len, names::Key::Up).0; }
    let mut picked: Option<String> = None;
    if keys.2 && st.open {
        if let (_, Some(i)) = names::navigate(st.highlight, len, names::Key::Enter) { picked = Some(st.search.results[i].name.clone()); }
    }
    // Losing focus by typing or a stray repaint is not a choice: take it back unless the user
    // clicked elsewhere, picked a name, or pressed Enter/Escape.
    let deliberate = picked.is_some() || ui.input(|i| i.pointer.any_pressed() || i.key_pressed(Key::Enter) || i.key_pressed(Key::Escape));
    // Enter with nothing to pick commits the typed name as-is (e.g. an object Blender has not listed yet).
    if picked.is_none() && len == 0 && ui.input(|i| i.key_pressed(Key::Enter)) { if let Some(d) = &st.draft { picked = Some(d.clone()); } }
    if st.had_focus && response.lost_focus() && !deliberate { response.request_focus(); }
    st.had_focus = response.has_focus() || (st.had_focus && !deliberate && response.lost_focus());
    st.open = st.open && (response.has_focus() || st.over_popup) && picked.is_none();
    st.over_popup = false;
    if st.open {
        let at = response.rect.left_bottom() + Vec2::new(0., 2.);
        egui::Area::new(id.with("popup")).order(Order::Foreground).fixed_pos(at).show(ui.ctx(), |ui| {
            let row_h = 20.;
            let rows = len.min(12);
            let extra = if st.search.searching() || len == 0 { 1 } else { 0 } + usize::from(st.search.partial);
            let size = Vec2::new(response.rect.width().max(180.), (rows + extra) as f32 * row_h + 6.);
            let (rect, area) = ui.allocate_exact_size(size, Sense::hover());
            st.over_popup = area.hovered() || ui.ctx().pointer_hover_pos().is_some_and(|p| rect.contains(p));
            let p = ui.painter();
            p.rect(rect, 4., theme.panel2, Stroke::new(1.0_f32, theme.stroke2), StrokeKind::Inside);
            let mut y = rect.top() + 3.;
            for (i, m) in st.search.results.iter().take(rows).enumerate() {
                let r = egui::Rect::from_min_size(egui::pos2(rect.left() + 2., y), Vec2::new(rect.width() - 4., row_h));
                let resp = ui.interact(r, id.with(("row", i)), Sense::click());
                if i == st.highlight || resp.hovered() { p.rect_filled(r, 3., theme.accent_bg); }
                p.text(egui::pos2(r.left() + 8., r.center().y), Align2::LEFT_CENTER, &m.name, FontId::proportional(12.), theme.text);
                p.text(egui::pos2(r.right() - 8., r.center().y), Align2::RIGHT_CENTER, m.kind.to_lowercase(), FontId::proportional(10.), if m.mesh { theme.dim } else { theme.faint });
                if resp.clicked() { picked = Some(m.name.clone()); }
                y += row_h;
            }
            let note = if st.search.searching() { Some("searching...".to_owned()) }
                else if len == 0 { Some("no matching Blender objects".to_owned()) } else { None };
            if let Some(note) = note { p.text(egui::pos2(rect.left() + 10., y + row_h / 2.), Align2::LEFT_CENTER, note, FontId::proportional(11.), theme.dim); y += row_h; }
            if st.search.partial {
                p.text(egui::pos2(rect.left() + 10., y + row_h / 2.), Align2::LEFT_CENTER,
                       format!("partial results ({} objects)", st.search.count.unwrap_or(0)), FontId::proportional(10.), theme.faint);
            }
        });
    }
    // Leaving the field without choosing keeps a draft that names a listed object exactly, and drops anything else.
    if picked.is_none() && response.lost_focus() && deliberate {
        if let Some(d) = st.draft.take() {
            if let Some(hit) = st.search.results.iter().find(|m| m.name.eq_ignore_ascii_case(&d)) { picked = Some(hit.name.clone()); }
        }
    }
    if let Some(name) = picked {
        text = name;
        changed = true;
        st.draft = None;
        st.open = false;
        st.over_popup = false;
        ui.ctx().memory_mut(|m| m.surrender_focus(id));
    }
    if changed { *value = json!(text); }
    ui.ctx().data_mut(|d| d.insert_temp(state_id, st));
    changed
}
