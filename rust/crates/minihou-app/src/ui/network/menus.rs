use super::*;

pub fn open_add_menu(ctx: &egui::Context, path: &Path) {
    let id = state_id(path);
    let at = ctx.input(|i| i.pointer.hover_pos()).unwrap_or(pos2(160., 190.));
    ctx.data_mut(|d| {
        let mut state = d.get_persisted::<State>(id).unwrap_or_default();
        let r = state.canvas.unwrap_or_else(|| Rect::from_min_size(Pos2::ZERO, vec2(1000., 800.)));
        let zoom = if state.zoom <= 0.0 { 1.0 } else { state.zoom };
        let graph_pos = ((at - r.min - state.pan) / zoom).to_pos2();
        if state.menu.is_none() {
            state.add_pos = Some(graph_pos);
            state.menu = Some(network_menu::MenuState::tab(at).with_canvas_pos(graph_pos));
        }
        d.insert_persisted(id, state);
    });
    ctx.input_mut(|i| i.consume_key(egui::Modifiers::default(), egui::Key::Tab));
    ctx.request_repaint();
}
pub fn open_pie_menu(ctx: &egui::Context, path: &Path) {
    let id = state_id(path);
    let at = ctx.input(|i| i.pointer.hover_pos()).unwrap_or(pos2(160., 190.));
    ctx.data_mut(|d| {
        let mut state = d.get_persisted::<State>(id).unwrap_or_default();
        state.menu = Some(network_menu::MenuState::pie(at));
        d.insert_persisted(id, state);
    });
    ctx.request_repaint();
}
pub fn open_rename_dialog(ctx: &egui::Context, node_id: &str) {
    let id = egui::Id::new("rename_link_node");
    ctx.data_mut(|d| {
        d.insert_temp(id, (node_id.to_owned(), node_id.to_owned()));
        d.insert_temp(id.with("select_name"), true);
    });
    ctx.request_repaint();
}

pub(super) fn rename_dialog(ctx: &egui::Context, out: &mut Actions) {
    let id=egui::Id::new("rename_link_node");
    let Some((old,mut new))=ctx.data(|d|d.get_temp::<(String,String)>(id)) else {return};
    let (confirm, cancel) = ctx.input_mut(|i| (
        i.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
        i.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
    ));
    if cancel {
        ctx.data_mut(|d| d.remove::<(String,String)>(id));
        return;
    }
    let mut open=true;
    egui::Window::new("Rename node").open(&mut open).show(ctx,|ui| {
        let text_id = id.with("name");
        let response = ui.add(egui::TextEdit::singleline(&mut new).id(text_id));
        let select_name = ctx.data_mut(|d| {
            let select = d.get_temp::<bool>(id.with("select_name")).unwrap_or(false);
            d.remove::<bool>(id.with("select_name"));
            select
        });
        if select_name {
            response.request_focus();
            let mut state = egui::TextEdit::load_state(ctx, text_id).unwrap_or_default();
            state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::new(0), egui::text::CCursor::new(new.chars().count()))));
            state.store(ctx, text_id);
        }
        let clicked = ui.button("Rename").clicked();
        if confirm || clicked {out.push(UiAction::RenameNode {old:old.clone(),new:new.clone()});ctx.data_mut(|d|d.remove::<(String,String)>(id));}
    });
    if open && ctx.data(|d|d.get_temp::<(String,String)>(id)).is_some() {ctx.data_mut(|d|d.insert_temp(id,(old,new)));}
    if !open {ctx.data_mut(|d|d.remove::<(String,String)>(id));}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opening_rename_focuses_and_selects_the_current_name() {
        let ctx = egui::Context::default();
        let id = egui::Id::new("rename_link_node");
        let text_id = id.with("name");
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            open_rename_dialog(ctx, "box1");
            rename_dialog(ctx, &mut Actions::default());
            assert!(ctx.memory(|m| m.has_focus(text_id)));
            let state = egui::TextEdit::load_state(ctx, text_id).unwrap();
            assert_eq!(state.cursor.char_range().unwrap().as_sorted_char_range(), 0..4);
        });
    }

    #[test]
    fn enter_commits_and_escape_cancels_rename() {
        for (key, commits) in [(egui::Key::Enter, true), (egui::Key::Escape, false)] {
            let ctx = egui::Context::default();
            let id = egui::Id::new("rename_link_node");
            let _ = ctx.run(egui::RawInput::default(), |ctx| open_rename_dialog(ctx, "box1"));
            let mut out = Actions::default();
            let _ = ctx.run(egui::RawInput { events: vec![egui::Event::Key {
                key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE,
            }], ..Default::default() }, |ctx| rename_dialog(ctx, &mut out));
            assert_eq!(matches!(out.0.as_slice(), [UiAction::RenameNode {old, new}]
                if old == "box1" && new == "box1"), commits);
            assert!(ctx.data(|d| d.get_temp::<(String, String)>(id)).is_none());
        }
    }
}

pub(super) fn header_ui(ui: &mut egui::Ui, r: Rect, v: &View, out: &mut Actions, state: &mut State) {
    let p = ui.painter();
    p.rect_filled(r, 0.0, v.theme.panel2);
    p.line_segment([pos2(r.left(), r.bottom() - 0.5), pos2(r.right(), r.bottom() - 0.5)], Stroke::new(1.0_f32, v.theme.stroke));
    p.text(pos2(r.left()+10.,r.center().y),Align2::LEFT_CENTER,"Network",FontId::proportional(12.),v.theme.text);
    let links_rect=Rect::from_min_size(pos2(r.right()-226.,r.top()+5.),vec2(28.,22.));
    let snap_rect=Rect::from_min_size(pos2(r.right()-194.,r.top()+5.),vec2(28.,22.));
    if header_toggle(ui,links_rect,"links",state.show_links,"Show parameter references (Ctrl+Shift+D)",v) {
        state.show_links=!state.show_links; out.push(UiAction::LinkVisibility(state.show_links));
    }
    let ink=if state.show_links {v.theme.accent}else{v.theme.dim};
    let c=links_rect.center();
    p.circle_stroke(c+vec2(-3.,-2.),4.,Stroke::new(1.4_f32,ink));
    p.circle_stroke(c+vec2(3.,2.),4.,Stroke::new(1.4_f32,ink));
    p.line_segment([c+vec2(-2.,-1.),c+vec2(2.,1.)],Stroke::new(1.4_f32,ink));
    if header_toggle(ui,snap_rect,"snap",state.snap,"Snap cards (Ctrl to invert)",v) {state.snap=!state.snap;}
    magnet_icon(p,snap_rect.center(),if state.snap {v.theme.accent}else{v.theme.dim});
    let physics_rect = Rect::from_min_size(pos2(r.right() - 160.0, r.top() + 5.0), vec2(70.0, 22.0));
    let tidy_rect = Rect::from_min_size(pos2(r.right() - 88.0, r.top() + 5.0), vec2(44.0, 22.0));
    if ui.interact(tidy_rect, ui.id().with("tidy"), Sense::click()).on_hover_text("Tidy network").clicked() {
        out.push(UiAction::TidyNetwork);
    }
    tidy_icon(p, pos2(tidy_rect.left() + 9.0, tidy_rect.center().y), v.theme.text);
    p.text(pos2(tidy_rect.left() + 18.0, tidy_rect.center().y), Align2::LEFT_CENTER, "Tidy", FontId::proportional(11.0), v.theme.text);
    let fit = Rect::from_min_size(pos2(r.right() - 34.0, r.top() + 5.0), vec2(24.0, 22.0));
    if ui.interact(fit, ui.id().with("fit"), Sense::click()).on_hover_text("Fit network to view").clicked() { state.zoom = -1.0; }
    fit_icon(p, fit.center(), v.theme.text);
    let mut controls = ui.new_child(egui::UiBuilder::new().max_rect(physics_rect));
    if controls.checkbox(&mut state.physics, "Physics").on_hover_text("Hold Shift while dragging to pull connected nodes with springs").changed() {
        state.springs = None;
    }
}

pub(super) fn header_toggle(ui:&egui::Ui,r:Rect,id:&str,on:bool,tooltip:&str,v:&View)->bool {
    let response=ui.interact(r,ui.id().with(id),Sense::click()).on_hover_text(tooltip);
    let fill=if on {v.theme.accent.gamma_multiply(0.18)}else if response.hovered(){v.theme.card}else{v.theme.panel2};
    ui.painter().rect_filled(r,3.,fill);
    ui.painter().rect_stroke(r,3.,Stroke::new(1.0_f32,if on {v.theme.accent}else{v.theme.stroke}),StrokeKind::Inside);
    response.clicked()
}

pub(super) fn magnet_icon(p: &egui::Painter, c: Pos2, color: Color32) {
    let s = Stroke::new(1.4_f32, color);
    p.line_segment([c + vec2(-4.0, -4.0), c + vec2(-4.0, 1.0)], s);
    p.line_segment([c + vec2(4.0, -4.0), c + vec2(4.0, 1.0)], s);
    p.add(egui::Shape::line(vec![c + vec2(-4.0, 1.0), c + vec2(-3.0, 4.0), c + vec2(0.0, 5.0), c + vec2(3.0, 4.0), c + vec2(4.0, 1.0)], s));
}
pub(super) fn tidy_icon(p: &egui::Painter, c: Pos2, color: Color32) {
    let s = Stroke::new(1.0_f32, color);
    for d in [-3.0, 0.0, 3.0] {
        p.line_segment([c + vec2(-4.0, d), c + vec2(4.0, d)], s);
        p.line_segment([c + vec2(d, -4.0), c + vec2(d, 4.0)], s);
    }
}

pub(super) fn split_breadcrumb(body: Rect) -> (Rect, Rect) {
    (Rect::from_min_size(body.min, vec2(body.width(), 22.0)), Rect::from_min_max(body.min + vec2(0.0, 22.0), body.max))
}
pub(super) fn crumbs(path: &Path, nested: &[String], selected: &str, has_selected: bool) -> Vec<String> {
    let mut parts = vec![path.file_stem().and_then(|s| s.to_str()).unwrap_or("Untitled").to_owned()];
    parts.extend_from_slice(nested);
    if has_selected && !selected.is_empty() { parts.push(selected.to_owned()); }
    parts
}
pub(super) fn breadcrumb_ui(ui: &egui::Ui, r: Rect, canvas: Rect, v: &View, out: &mut Actions, state: &mut State) {
    let p = ui.painter();
    p.rect_filled(r, 0.0, v.theme.panel);
    p.line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(1.0_f32, v.theme.stroke));
    let names = crumbs(v.path, &state.subnet_path, v.selected, v.graph.nodes.iter().any(|(id, _)| id == v.selected));
    let mut x = r.left() + 10.0;
    p.text(pos2(x, r.center().y), Align2::LEFT_CENTER, "/", FontId::monospace(11.0), v.theme.dim);
    x += 14.0;
    for (index, name) in names.iter().enumerate() {
        let width = (name.chars().count() as f32 * 6.8 + 8.0).min((r.right() - x).max(0.0));
        if width <= 0.0 { break; }
        let hit = Rect::from_min_size(pos2(x, r.top()), vec2(width, r.height()));
        if ui.interact(hit, ui.id().with(("crumb", index)), Sense::click()).clicked() {
            if index <= state.subnet_path.len() { state.subnet_path.truncate(index); reset_scope(state); }
            else if index + 1 == names.len() && v.graph.nodes.iter().any(|(id, _)| id == name) {
                state.selected_wires.clear();
                out.push(UiAction::Select(name.clone()));
                frame_node(state, canvas, name, v);
            }
        }
        p.text(pos2(x, r.center().y), Align2::LEFT_CENTER, name, FontId::monospace(11.0), v.theme.dim);
        x += width;
        if index + 1 < names.len() {
            p.text(pos2(x, r.center().y), Align2::LEFT_CENTER, ">", FontId::monospace(11.0), v.theme.dim);
            x += 14.0;
        }
    }
}
pub(super) fn frame_node(state: &mut State, canvas: Rect, node: &str, v: &View) {
    let positions = node_positions(state, v);
    if let Some((_, at)) = positions.iter().find(|(id, _)| id == node) { frame_all(state, canvas, Rect::from_min_size(*at, CARD)); }
}


