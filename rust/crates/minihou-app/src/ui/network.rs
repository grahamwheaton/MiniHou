//! Network canvas and flat list, following Main.dc.html and NodeStates.dc.html.
#[path = "network_layout.rs"]
mod network_layout;
#[path = "network_menu.rs"]
mod network_menu;

use super::{Actions, UiAction, View};
use eframe::egui::{self, pos2, vec2, Align2, Color32, FontId, Id, PointerButton, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use minihou_app::Edit;
use minihou_core::graph::NodeDef;
use std::{collections::HashMap, path::Path};

const CARD: Vec2 = vec2(220.0, 56.0);

#[derive(Clone)]
struct State { list: bool, zoom: f32, pan: Vec2, moved: HashMap<String, Pos2>, snap: bool, tidied: bool,
    drag: Option<(String, Pos2)>, wiggle: Option<WiggleDetector>, box_start: Option<Pos2>, alt_pan: bool, toast: Option<(String, f64)>, menu: Option<network_menu::MenuState> }
impl Default for State {
    fn default() -> Self { Self { list: false, zoom: 1.0, pan: Vec2::ZERO, moved: HashMap::new(), snap: true,
        tidied: false, drag: None, wiggle: None, box_start: None, alt_pan: false, toast: None, menu: None } }
}

/// The web editor samples pointer X in 18 px chunks and detaches after three
/// reversals within 650 ms of the preceding reversal.
#[derive(Clone, Default)]
pub struct WiggleDetector { x: f32, anchor: f32, direction: i8, turns: u8, last_turn: f64 }
impl WiggleDetector {
    pub fn push(&mut self, time: f64, dx: f32) -> bool {
        self.x += dx;
        let delta = self.x - self.anchor;
        if delta.abs() < 18.0 { return false; }
        let direction = if delta > 0.0 { 1 } else { -1 };
        if self.direction != 0 && direction != self.direction {
            self.turns = if time - self.last_turn > 0.650 { 1 } else { self.turns + 1 };
            self.last_turn = time;
        }
        self.direction = direction;
        self.anchor = self.x;
        self.turns >= 3
    }
}

fn state_id(path: &Path) -> Id { Id::new(("network", path.to_string_lossy().as_ref())) }

/// Clear manual positions for a document. The action dispatcher should call this when it receives
/// `UiAction::TidyNetwork` from the keymap, then request a repaint.
pub fn tidy(ctx: &egui::Context, path: &Path) {
    let id = state_id(path);
    ctx.data_mut(|d| {
        let mut state = d.get_persisted::<State>(id).unwrap_or_default();
        tidy_state(&mut state);
        d.insert_persisted(id, state);
    });
    ctx.request_repaint();
}
pub fn frame(ctx: &egui::Context, path: &Path) {
    let id = state_id(path);
    ctx.data_mut(|d| {
        let mut state = d.get_persisted::<State>(id).unwrap_or_default();
        state.zoom = -1.0;
        d.insert_persisted(id, state);
    });
    ctx.request_repaint();
}
pub fn open_add_menu(ctx: &egui::Context, path: &Path) {
    let id = state_id(path);
    ctx.data_mut(|d| {
        let mut state = d.get_persisted::<State>(id).unwrap_or_default();
        state.list = false;
        let at = ctx.input(|i| i.pointer.hover_pos()).unwrap_or(pos2(160., 190.));
        state.menu = Some(network_menu::MenuState::new(at, None, true));
        d.insert_persisted(id, state);
    });
    ctx.request_repaint();
}
pub fn open_pie_menu(ctx: &egui::Context, path: &Path) {
    let id = state_id(path);
    ctx.data_mut(|d| {
        let mut state = d.get_persisted::<State>(id).unwrap_or_default();
        let at = ctx.input(|i| i.pointer.hover_pos()).unwrap_or(pos2(160., 190.));
        state.menu = Some(network_menu::MenuState::pie(at));
        d.insert_persisted(id, state);
    });
    ctx.request_repaint();
}
fn tidy_state(state: &mut State) {
    state.moved.clear();
    state.drag = None;
    state.tidied = true;
    state.zoom = -1.0;
}

pub fn show(ui: &mut egui::Ui, v: &View, out: &mut Actions) {
    let old_spacing = ui.spacing().item_spacing;
    ui.spacing_mut().item_spacing.y = 0.0;
    let id = state_id(v.path);
    let mut state = ui.ctx().data_mut(|d| d.get_persisted::<State>(id).unwrap_or_default());
    let (header, _) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::hover());
    header_ui(ui, header, v, out, &mut state);
    let (body, _) = ui.allocate_exact_size(ui.available_size(), Sense::hover());
    if state.list { list_ui(ui, body, v, out); } else {
        let (crumb, canvas) = split_breadcrumb(body);
        breadcrumb_ui(ui, crumb, canvas, v, out, &mut state);
        canvas_ui(ui, canvas, v, out, &mut state, id);
    }
    ui.ctx().data_mut(|d| d.insert_persisted(id, state));
    ui.spacing_mut().item_spacing = old_spacing;
}

fn header_ui(ui: &egui::Ui, r: Rect, v: &View, out: &mut Actions, state: &mut State) {
    let p = ui.painter();
    p.rect_filled(r, 0.0, v.theme.panel2);
    p.line_segment([pos2(r.left(), r.bottom() - 0.5), pos2(r.right(), r.bottom() - 0.5)], Stroke::new(1.0_f32, v.theme.stroke));
    for (i, (label, list)) in [("Network", false), ("List", true)].iter().enumerate() {
        let x = r.left() + if i == 0 { 10.0 } else { 91.0 };
        let tab = Rect::from_min_size(pos2(x, r.top()), vec2(if i == 0 { 68.0 } else { 28.0 }, 32.0));
        if ui.interact(tab, ui.id().with(("tab", i)), Sense::click()).clicked() { state.list = *list; }
        p.text(pos2(x, r.center().y), Align2::LEFT_CENTER, label, FontId::proportional(12.0), if state.list == *list { v.theme.text } else { v.theme.dim });
        if state.list == *list { p.line_segment([pos2(tab.left(), tab.bottom() - 1.0), pos2(tab.right(), tab.bottom() - 1.0)], Stroke::new(2.0_f32, v.theme.accent)); }
    }
    let snap_rect = Rect::from_min_size(pos2(r.right() - 142.0, r.top() + 5.0), vec2(52.0, 22.0));
    if ui.interact(snap_rect, ui.id().with("snap"), Sense::click()).on_hover_text("Snap cards (Ctrl to invert)").clicked() { state.snap = !state.snap; }
    let sc = if state.snap { v.theme.accent } else { v.theme.dim };
    magnet_icon(p, pos2(snap_rect.left() + 9.0, snap_rect.center().y), sc);
    p.text(pos2(snap_rect.left() + 19.0, snap_rect.center().y), Align2::LEFT_CENTER, "Snap", FontId::proportional(11.0), sc);
    let tidy_rect = Rect::from_min_size(pos2(r.right() - 88.0, r.top() + 5.0), vec2(44.0, 22.0));
    if ui.interact(tidy_rect, ui.id().with("tidy"), Sense::click()).on_hover_text("Tidy network").clicked() {
        tidy_state(state);
        out.push(UiAction::TidyNetwork);
    }
    tidy_icon(p, pos2(tidy_rect.left() + 9.0, tidy_rect.center().y), v.theme.text);
    p.text(pos2(tidy_rect.left() + 18.0, tidy_rect.center().y), Align2::LEFT_CENTER, "Tidy", FontId::proportional(11.0), v.theme.text);
    let fit = Rect::from_min_size(pos2(r.right() - 34.0, r.top() + 5.0), vec2(24.0, 22.0));
    if ui.interact(fit, ui.id().with("fit"), Sense::click()).on_hover_text("Fit network to view").clicked() { state.zoom = -1.0; }
    fit_icon(p, fit.center(), v.theme.text);
    p.text(pos2(snap_rect.left() - 5.0, r.center().y), Align2::RIGHT_CENTER, format!("{}%", (state.zoom.max(0.25) * 100.0).round() as i32), FontId::monospace(11.0), v.theme.dim);
}

fn magnet_icon(p: &egui::Painter, c: Pos2, color: Color32) {
    let s = Stroke::new(1.4_f32, color);
    p.line_segment([c + vec2(-4.0, -4.0), c + vec2(-4.0, 1.0)], s);
    p.line_segment([c + vec2(4.0, -4.0), c + vec2(4.0, 1.0)], s);
    p.add(egui::Shape::line(vec![c + vec2(-4.0, 1.0), c + vec2(-3.0, 4.0), c + vec2(0.0, 5.0), c + vec2(3.0, 4.0), c + vec2(4.0, 1.0)], s));
}
fn tidy_icon(p: &egui::Painter, c: Pos2, color: Color32) {
    let s = Stroke::new(1.0_f32, color);
    for d in [-3.0, 0.0, 3.0] {
        p.line_segment([c + vec2(-4.0, d), c + vec2(4.0, d)], s);
        p.line_segment([c + vec2(d, -4.0), c + vec2(d, 4.0)], s);
    }
}

fn split_breadcrumb(body: Rect) -> (Rect, Rect) {
    (Rect::from_min_size(body.min, vec2(body.width(), 22.0)), Rect::from_min_max(body.min + vec2(0.0, 22.0), body.max))
}
fn crumbs(path: &Path, nested: &[String], selected: &str, has_selected: bool) -> Vec<String> {
    let mut parts = vec![path.file_stem().and_then(|s| s.to_str()).unwrap_or("Untitled").to_owned()];
    parts.extend_from_slice(nested);
    if has_selected && !selected.is_empty() { parts.push(selected.to_owned()); }
    parts
}
fn breadcrumb_ui(ui: &egui::Ui, r: Rect, canvas: Rect, v: &View, out: &mut Actions, state: &mut State) {
    let p = ui.painter();
    p.rect_filled(r, 0.0, v.theme.panel);
    p.line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(1.0_f32, v.theme.stroke));
    let names = crumbs(v.path, &[], v.selected, v.graph.nodes.iter().any(|(id, _)| id == v.selected));
    let mut x = r.left() + 10.0;
    p.text(pos2(x, r.center().y), Align2::LEFT_CENTER, "/", FontId::monospace(11.0), v.theme.dim);
    x += 14.0;
    for (index, name) in names.iter().enumerate() {
        let width = (name.chars().count() as f32 * 6.8 + 8.0).min((r.right() - x).max(0.0));
        if width <= 0.0 { break; }
        let hit = Rect::from_min_size(pos2(x, r.top()), vec2(width, r.height()));
        if ui.interact(hit, ui.id().with(("crumb", index)), Sense::click()).clicked() {
            if index == 0 { state.zoom = -1.0; }
            else if index + 1 == names.len() && v.graph.nodes.iter().any(|(id, _)| id == name) {
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
fn frame_node(state: &mut State, canvas: Rect, node: &str, v: &View) {
    let positions = network_layout::layout(v.graph, |id| state.moved.get(id).copied().or_else(|| (!state.tidied).then(|| json_pos(v, id)).flatten()));
    if let Some((_, at)) = positions.iter().find(|(id, _)| id == node) { frame_all(state, canvas, Rect::from_min_size(*at, CARD)); }
}

fn list_ui(ui: &mut egui::Ui, r: Rect, v: &View, out: &mut Actions) {
    ui.painter().rect_filled(r, 0.0, v.theme.panel);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r));
    egui::ScrollArea::vertical().show(&mut child, |ui| {
        for (id, node) in &v.graph.nodes {
            ui.horizontal(|ui| {
                if ui.radio(minihou_app::display_nodes(v.doc).iter().any(|node| node == id), "").on_hover_text("Display this node (Shift adds or removes)").clicked() {
                    out.push(if ui.input(|i| i.modifiers.shift) { UiAction::ToggleDisplay(id.clone()) } else { UiAction::SetDisplay(id.clone()) });
                }
                if ui.selectable_label(v.selected_set.contains(id), format!("{id}  {}", node.node_type)).clicked() { out.push(if ui.input(|i| i.modifiers.shift || i.modifiers.ctrl) { UiAction::ToggleSelect(id.clone()) } else { UiAction::Select(id.clone()) }); }
                let mut bypass = node.bypass;
                if ui.checkbox(&mut bypass, "").on_hover_text("Mute (bypass)").changed() {
                    out.push(UiAction::Edit { edit: Edit::Bypass { node: id.clone(), flag: bypass }, continuous: false });
                }
            });
        }
    });
}

fn canvas_ui(ui: &egui::Ui, r: Rect, v: &View, out: &mut Actions, state: &mut State, id: Id) {
    let ctx = ui.ctx();
    if v.cooking.cooking() { ctx.request_repaint_after(std::time::Duration::from_millis(16)); }
    let pointer = ctx.input(|i| i.pointer.hover_pos());
    let over = pointer.is_some_and(|at| r.contains(at));
    let positions = network_layout::layout(v.graph, |node| state.moved.get(node).copied().or_else(|| (!state.tidied).then(|| json_pos(v, node)).flatten()));
    let bounds = content_bounds(&positions);
    if state.zoom < 0.0 || (over && ctx.input(|i| i.key_pressed(egui::Key::Home))) { frame_all(state, r, bounds); }
    if !ctx.input(|i| i.pointer.primary_down()) { state.alt_pan = false; }
    if over && ctx.input(|i| i.pointer.button_pressed(PointerButton::Primary) && i.modifiers.alt) {
        let on_card = pointer.is_some_and(|at| positions.iter().any(|(_, pos)| Rect::from_min_size(screen(r, state, *pos), CARD * state.zoom).contains(at)));
        if !on_card { state.alt_pan = true; }
    }
    if over {
        let (zoom_delta, scroll, delta, dragging, ctrl) = ctx.input(|i| (i.zoom_delta(), i.smooth_scroll_delta, i.pointer.delta(),
            i.pointer.middle_down() || state.alt_pan || (i.key_down(egui::Key::Space) && i.pointer.primary_down()), i.modifiers.ctrl));
        let trackpad_pan = !ctrl && scroll.x != 0.0 && scroll.y.abs() < 50.0;
        let factor = if zoom_delta != 1.0 { zoom_delta } else if !trackpad_pan && scroll.y != 0.0 { (scroll.y * if ctrl { 0.01 } else { 0.0015 }).exp() } else { 1.0 };
        if factor != 1.0 {
            let at = pointer.unwrap_or(r.center());
            let old = state.zoom;
            state.zoom = (old * factor).clamp(0.25, 2.0);
            state.pan = (at - r.min) - (at - r.min - state.pan) * (state.zoom / old);
        } else if trackpad_pan { state.pan += scroll; }
        if dragging { state.pan += delta; }
    }
    let right_click = over && ctx.input(|i| i.pointer.button_pressed(PointerButton::Secondary));
    let tab = over && state.menu.is_none() && ctx.input(|i| i.key_pressed(egui::Key::Tab));
    if right_click || tab {
        let at = pointer.unwrap_or(r.center());
        let target = if right_click { positions.iter().rev().find(|(_, pos)| Rect::from_min_size(screen(r, state, *pos), CARD * state.zoom).contains(at))
            .map(|(id, _)| id.clone()) } else { None };
        state.menu = Some(network_menu::MenuState::new(at, target, tab));
    }
    let p = ui.painter().with_clip_rect(r);
    p.rect_filled(r, 0.0, v.theme.base);
    grid(&p, r, state, v.theme.grid);
    let background = ui.interact(r, id.with("box_selection"), Sense::click_and_drag());
    let background_hit_card = pointer.is_some_and(|at| positions.iter().any(|(_, pos)| Rect::from_min_size(screen(r, state, *pos), CARD * state.zoom).contains(at)));
    if background.double_clicked() && !background_hit_card {
        state.menu = Some(network_menu::MenuState::new(pointer.unwrap_or(r.center()), None, true));
    }
    if background.drag_started_by(PointerButton::Primary) && !state.alt_pan { state.box_start = background.interact_pointer_pos(); }
    if positions.is_empty() {
        p.text(pos2(r.center().x, r.center().y - 12.0), Align2::CENTER_CENTER, "The network is empty", FontId::proportional(15.0), v.theme.text);
        let hint = p.layout("Open a graph file, press Tab, or pick a tool from the shelf above.".into(), FontId::proportional(12.0), v.theme.dim, (r.width() - 80.0).max(80.0));
        p.galley(pos2(r.center().x - hint.size().x * 0.5, r.center().y + 5.0), hint, v.theme.dim);
    } else {
        wires(&p, r, state, v, &positions);
        let mut guide = None;
        for (node_id, at) in &positions {
            let Some((_, node)) = v.graph.nodes.iter().find(|(n, _)| n == node_id) else { continue };
            let card = Rect::from_min_size(screen(r, state, *at), CARD * state.zoom);
            if !card.intersects(r) { continue; }
            let face = Rect::from_min_max(pos2(card.left() + 20.0 * state.zoom, card.top()), pos2(card.right() - 20.0 * state.zoom, card.bottom()));
            let response = ui.interact(face, id.with(("node", node_id)), Sense::click_and_drag())
                .on_hover_text(format!("{} · {} · {} params", node_id, node.node_type, node.params.as_ref().map_or(0, |p| p.len())));
            if response.double_clicked() { out.push(UiAction::SetDisplay(node_id.clone())); }
            else if response.clicked() { out.push(if ctx.input(|i| i.modifiers.shift || i.modifiers.ctrl) { UiAction::ToggleSelect(node_id.clone()) } else { UiAction::Select(node_id.clone()) }); }
            if response.drag_started_by(PointerButton::Primary) {
                if !v.selected_set.contains(node_id) { out.push(UiAction::Select(node_id.clone())); }
                state.drag = Some((node_id.clone(), *at)); state.wiggle = Some(WiggleDetector::default()); state.box_start = None;
            }
            if response.dragged_by(PointerButton::Primary) && !ctx.input(|i| i.key_down(egui::Key::Space)) {
                let (time, dx) = ctx.input(|i| (i.time, i.pointer.delta().x));
                if state.wiggle.as_mut().is_some_and(|wiggle| wiggle.push(time, dx)) {
                    out.push(UiAction::DisconnectAll(node_id.clone()));
                    state.toast = Some((node_id.clone(), time + 1.2));
                    state.wiggle = None;
                }
                let delta = ctx.input(|i| i.pointer.delta()) / state.zoom;
                let raw = if let Some((active, raw)) = &mut state.drag {
                    if active == node_id { *raw += delta; *raw } else { *at + delta }
                } else { *at + delta };
                let invert = ctx.input(|i| i.modifiers.ctrl);
                let enabled = state.snap ^ invert;
                let base = snap(raw, 32.0, state.snap, invert);
                let peers: Vec<_> = positions.iter().filter(|(id, _)| id != node_id).map(|(_, at)| *at).collect();
                let (next, lines) = if enabled { align_to_cards(base, &peers, CARD, 6.0 / state.zoom) } else { (base, Guides::default()) };
                guide = Some(lines);
                state.moved.insert(node_id.clone(), next);
                if v.selected_set.contains(node_id) {
                    let offset = next - *at;
                    for (peer, peer_at) in &positions {
                        if peer != node_id && v.selected_set.contains(peer) { state.moved.insert(peer.clone(), *peer_at + offset); }
                    }
                }
            }
            if response.drag_stopped_by(PointerButton::Primary) { state.drag = None; state.wiggle = None; }
            for flag in 0..4 {
                if ui.interact(flag_rect(card, flag), id.with(("flag", node_id, flag)), Sense::click())
                    .on_hover_text(["Mute", "Locked", "Template", "Visible"][flag]).clicked() {
                    if v.selected_set.contains(node_id) && v.selected_set.len() > 1 {
                        match flag {
                            0 => out.push(UiAction::Command("mute_flag".into())),
                            1 => for peer in v.selected_set { out.push(UiAction::ToggleLocked(peer.clone())); },
                            2 => for peer in v.selected_set { out.push(UiAction::ToggleTemplate(peer.clone())); },
                            _ => out.push(UiAction::Command("display_flag".into())),
                        }
                    } else {
                        out.push(if flag == 3 && ctx.input(|i| i.modifiers.shift) { UiAction::ToggleDisplay(node_id.clone()) } else { flag_action(flag, node_id, node.bypass) });
                    }
                }
            }
            card_ui(&p, card, node_id, node, v, state.zoom);
            if state.toast.as_ref().is_some_and(|(shown, until)| shown == node_id && ctx.input(|i| i.time) < *until) {
                p.text(card.center_top() + vec2(0.0, -9.0), Align2::CENTER_BOTTOM, "disconnected", FontId::proportional(12.0), v.theme.accent);
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
            }
        }
        if let Some(lines) = guide {
            if let Some(x) = lines.x { let x = screen(r, state, pos2(x, 0.0)).x; p.line_segment([pos2(x, r.top()), pos2(x, r.bottom())], Stroke::new(1.0_f32, v.theme.accent)); }
            if let Some(y) = lines.y { let y = screen(r, state, pos2(0.0, y)).y; p.line_segment([pos2(r.left(), y), pos2(r.right(), y)], Stroke::new(1.0_f32, v.theme.accent)); }
        }
        minimap(ui, r, state, v, &positions, id);
    }
    if let (Some(start), Some(end)) = (state.box_start, pointer) {
        if background.dragged_by(PointerButton::Primary) {
            let box_rect = Rect::from_two_pos(start, end);
            p.rect_stroke(box_rect, 0.0, Stroke::new(1.0_f32, v.theme.accent), StrokeKind::Inside);
        }
        if background.drag_stopped_by(PointerButton::Primary) {
            let box_rect = Rect::from_two_pos(start, end);
            let ids = positions.iter().filter(|(_, at)| Rect::from_min_size(screen(r, state, *at), CARD * state.zoom).intersects(box_rect)).map(|(id, _)| id.clone()).collect();
            out.push(UiAction::SelectSet { ids, extend: ctx.input(|i| i.modifiers.shift) });
            state.box_start = None;
        }
    }
    if let Some(menu) = &mut state.menu {
        if let Some(choice) = network_menu::show(ctx, id.with("menu"), menu, v, out) {
            match choice {
                network_menu::Choice::Close => state.menu = None,
                network_menu::Choice::FrameAll => { frame_all(state, r, content_bounds(&positions)); state.menu = None; },
                network_menu::Choice::Tidy => { tidy_state(state); out.push(UiAction::TidyNetwork); state.menu = None; },
            }
        }
    }
    p.text(pos2(r.left() + 10.0, r.bottom() - 8.0), Align2::LEFT_BOTTOM, "Tab add node · Home frame all · Del delete", FontId::proportional(11.0), v.theme.faint);
}

fn snap(pos: Pos2, grid: f32, enabled: bool, invert: bool) -> Pos2 {
    if !(enabled ^ invert) || grid <= 0.0 { return pos; }
    pos2((pos.x / grid).round() * grid, (pos.y / grid).round() * grid)
}

#[derive(Clone, Copy, Default, Debug, PartialEq)]
struct Guides { x: Option<f32>, y: Option<f32> }
fn align_to_cards(pos: Pos2, peers: &[Pos2], size: Vec2, tolerance: f32) -> (Pos2, Guides) {
    let mut best_x: Option<(f32, f32, f32)> = None;
    let mut best_y: Option<(f32, f32)> = None;
    for peer in peers {
        for peer_offset in [0.0, size.x * 0.5, size.x] {
            for own_offset in [0.0, size.x * 0.5, size.x] {
                let guide = peer.x + peer_offset;
                let distance = (pos.x + own_offset - guide).abs();
                if distance <= tolerance && best_x.is_none_or(|(d, _, _)| distance < d) {
                    best_x = Some((distance, guide - own_offset, guide));
                }
            }
        }
        let distance = (pos.y - peer.y).abs();
        if distance <= tolerance && best_y.is_none_or(|(d, _)| distance < d) { best_y = Some((distance, peer.y)); }
    }
    let next = pos2(best_x.map_or(pos.x, |(_, x, _)| x), best_y.map_or(pos.y, |(_, y)| y));
    (next, Guides { x: best_x.map(|(_, _, guide)| guide), y: best_y.map(|(_, y)| y) })
}

fn json_pos(v: &View, id: &str) -> Option<Pos2> {
    let a = v.doc.get("nodes")?.get(id)?.get("pos")?.as_array()?;
    let x = a.first()?.as_f64()? as f32;
    let y = a.get(1)?.as_f64()? as f32;
    (x.is_finite() && y.is_finite()).then_some(pos2(x, y))
}
fn content_bounds(positions: &[(String, Pos2)]) -> Rect {
    positions.iter().fold(Rect::NOTHING, |r, (_, at)| r.union(Rect::from_min_size(*at, CARD)))
}
fn frame_all(state: &mut State, canvas: Rect, bounds: Rect) {
    if bounds == Rect::NOTHING { state.zoom = 1.0; state.pan = Vec2::ZERO; return; }
    let size = bounds.size() + vec2(48.0, 48.0);
    state.zoom = (canvas.width() / size.x).min(canvas.height() / size.y).clamp(0.25, 2.0);
    state.pan = canvas.size() * 0.5 - bounds.center().to_vec2() * state.zoom;
}
fn screen(canvas: Rect, state: &State, at: Pos2) -> Pos2 { canvas.min + state.pan + at.to_vec2() * state.zoom }

fn grid(p: &egui::Painter, r: Rect, state: &State, color: Color32) {
    let step = 32.0 * state.zoom;
    let origin = r.min + state.pan;
    let mut x = r.left() + (origin.x - r.left()).rem_euclid(step);
    while x < r.right() { p.line_segment([pos2(x.round(), r.top()), pos2(x.round(), r.bottom())], Stroke::new(1.0_f32, color)); x += step; }
    let mut y = r.top() + (origin.y - r.top()).rem_euclid(step);
    while y < r.bottom() { p.line_segment([pos2(r.left(), y.round()), pos2(r.right(), y.round())], Stroke::new(1.0_f32, color)); y += step; }
}

fn wires(p: &egui::Painter, canvas: Rect, state: &State, v: &View, positions: &[(String, Pos2)]) {
    let lookup: HashMap<&str, Pos2> = positions.iter().map(|(id, at)| (id.as_str(), *at)).collect();
    for (target, def) in &v.graph.nodes {
        let Some(&end) = lookup.get(target.as_str()) else { continue };
        let refs: Vec<&String> = def.inputs.iter().flat_map(|(_, ids)| ids).collect();
        for (index, source) in refs.iter().enumerate() {
            let Some(&start) = lookup.get(source.as_str()) else { continue };
            let a = screen(canvas, state, start + vec2(CARD.x * 0.5, CARD.y));
            let spread = (index as f32 - (refs.len() - 1) as f32 * 0.5) * 16.0;
            let b = screen(canvas, state, end + vec2(CARD.x * 0.5 + spread, 0.0));
            let dy = (b.y - a.y) * 0.5;
            p.add(egui::epaint::CubicBezierShape::from_points_stroke([a, a + vec2(0.0, dy), b - vec2(0.0, dy), b], false,
                Color32::TRANSPARENT, Stroke::new(1.5_f32, if v.selected == source.as_str() || v.selected == target { v.theme.accent } else { v.theme.wire })));
        }
    }
}

fn flag_rect(card: Rect, flag: usize) -> Rect {
    let w = card.width() * 20.0 / CARD.x;
    let x = if flag < 2 { card.left() } else { card.right() - w };
    let y = if flag.is_multiple_of(2) { card.top() } else { card.center().y };
    Rect::from_min_size(pos2(x, y), vec2(w, card.height() * 0.5))
}

fn flag_action(flag: usize, id: &str, bypass: bool) -> UiAction {
    match flag {
        0 => UiAction::Edit { edit: Edit::Bypass { node: id.into(), flag: !bypass }, continuous: false },
        1 => UiAction::ToggleLocked(id.into()),
        2 => UiAction::ToggleTemplate(id.into()),
        _ => UiAction::SetDisplay(id.into()),
    }
}

fn card_ui(p: &egui::Painter, r: Rect, id: &str, node: &NodeDef, v: &View, zoom: f32) {
    let t = v.theme;
    let flags = v.flags_of(id);
    let unsupported = !v.runs_on_rust(&node.node_type);
    let error = v.cooking.error.as_deref().filter(|e| e.contains(&format!("node '{id}'")));
    let visible = minihou_app::display_nodes(v.doc).iter().any(|node| node == id);
    let fill = if node.bypass { t.panel } else if error.is_some() { t.err_bg } else if visible { t.accent_bg } else { t.card };
    let border = if node.bypass { t.stroke } else if error.is_some() { t.err } else if unsupported { t.warn } else if flags.template { t.tpl } else if visible || v.cooking.cooking() { t.accent } else { t.stroke2 };
    p.rect_filled(r, 6.0 * zoom, fill);
    if node.bypass || unsupported || flags.template { dashed_rect(p, r, border); }
    else { p.rect_stroke(r, 6.0 * zoom, Stroke::new(1.0_f32, border), StrokeKind::Inside); }
    if v.selected_set.contains(id) { p.rect_stroke(r.expand(2.0), 8.0 * zoom, Stroke::new(2.0_f32, t.sel), StrokeKind::Outside); }
    let left = r.left() + 20.0 * zoom;
    let right = r.right() - 20.0 * zoom;
    for x in [left, right] { p.line_segment([pos2(x, r.top()), pos2(x, r.bottom())], Stroke::new(1.0_f32, t.stroke)); }
    for flag in 0..4 {
        let fr = flag_rect(r, flag);
        if !flag.is_multiple_of(2) { p.line_segment([pos2(fr.left(), fr.top()), pos2(fr.right(), fr.top())], Stroke::new(1.0_f32, t.stroke)); }
        let on = match flag { 0 => node.bypass, 1 => flags.locked, 2 => flags.template, _ => visible };
        if on { p.rect_filled(fr.shrink(1.0), 2.0 * zoom, [t.warn, t.dim, t.tpl, t.accent][flag]); }
        let ink = if on { if flag == 1 && !t.dark { t.card } else { t.sel } }
            else { t.dim.gamma_multiply(if v.selected == id { 0.85 } else { 0.5 }) };
        flag_icon(p, fr.center(), flag, ink, zoom);
    }
    let face = Rect::from_min_max(pos2(left + 10.0 * zoom, r.top() + 7.0 * zoom), pos2(right - 10.0 * zoom, r.bottom() - 7.0 * zoom));
    let text = if node.bypass { t.faint } else { t.text };
    let dim = if node.bypass { t.faint } else { t.dim };
    let cat = if node.bypass || unsupported { t.faint } else { t.category_color(&node.node_type) };
    if zoom >= 0.7 {
        p.rect_filled(Rect::from_min_size(pos2(face.left(), face.top() + 2.0 * zoom), vec2(10.0 * zoom, 10.0 * zoom)), 2.0 * zoom, cat);
        let cooked = v.cooking.stats.cooked.iter().chain(v.cooking.stats.cached.iter()).any(|n| n == id);
        let tag_width = if cooked && !node.bypass && !unsupported { 24.0 * zoom } else { 0.0 };
        if tag_width > 0.0 {
            let tag = Rect::from_min_size(pos2(face.right() - tag_width, face.top()), vec2(tag_width, 15.0 * zoom));
            p.rect_filled(tag, 3.0 * zoom, t.ok_bg);
            p.rect_stroke(tag, 3.0 * zoom, Stroke::new(1.0_f32, t.ok), StrokeKind::Inside);
            p.text(tag.center(), Align2::CENTER_CENTER, if node.node_type == "lod_bake" { "LOD" } else { "Rs" }, FontId::monospace(10.0 * zoom), t.ok);
        }
        p.text(pos2(face.left() + 17.0 * zoom, face.top()), Align2::LEFT_TOP, ellipsis(id, face.width() - 17.0 * zoom - tag_width, 12.5 * zoom), FontId::monospace(12.5 * zoom), text);
        let (status, status_color) = node_status(v, id, node.bypass, unsupported, error.is_some());
        p.text(pos2(face.left(), face.bottom()), Align2::LEFT_BOTTOM, ellipsis(if node.node_type == "lod_bake" { "Nanite proxy" } else { &node.node_type }, 92.0 * zoom, 11.0 * zoom), FontId::proportional(11.0 * zoom), dim);
        p.text(pos2(face.right(), face.bottom()), Align2::RIGHT_BOTTOM, status, FontId::monospace(11.0 * zoom), status_color);
        if v.cooking.cooking() && !node.bypass {
            let time = p.ctx().input(|i| i.time as f32);
            let center = pos2(face.right() - 58.0 * zoom, face.bottom() - 5.0 * zoom);
            let arc: Vec<_> = (0..=12).map(|i| {
                let angle = time * std::f32::consts::TAU + i as f32 * std::f32::consts::PI * 1.5 / 12.0;
                center + vec2(angle.cos(), angle.sin()) * 4.5 * zoom
            }).collect();
            p.add(egui::Shape::line(arc, Stroke::new(1.8_f32 * zoom, t.accent)));
        }
    } else if zoom >= 0.4 {
        p.rect_filled(Rect::from_min_size(pos2(face.left(), r.center().y - 5.0 * zoom), vec2(10.0 * zoom, 10.0 * zoom)), 2.0, cat);
        p.text(pos2(face.left() + 16.0 * zoom, r.center().y), Align2::LEFT_CENTER, ellipsis(id, face.width() - 16.0 * zoom, 12.0), FontId::monospace(12.0), text);
    } else { p.rect_filled(r.shrink(2.0), 3.0, cat); }
    let inputs: usize = node.inputs.iter().map(|(_, refs)| refs.len()).sum();
    for index in 0..inputs.max(1) {
        let offset = (index as f32 - (inputs.max(1) - 1) as f32 * 0.5) * 16.0 * zoom;
        p.circle(pos2(r.center().x + offset, r.top()), 5.0 * zoom, t.base, Stroke::new(1.5_f32, t.stroke2));
    }
    if node.node_type != "output" { p.circle(pos2(r.center().x, r.bottom()), 5.0 * zoom, t.base, Stroke::new(1.5_f32, t.stroke2)); }
    if v.cooking.cooking() && !node.bypass { p.rect_filled(Rect::from_min_size(pos2(r.left() + 2.0, r.bottom() - 3.0), vec2(r.width() * 0.38, 2.0)), 0.0, t.accent); }
    if let Some(message) = error {
        let galley = p.layout(message.to_owned(), FontId::monospace(11.0 * zoom), t.text, r.width() - 16.0);
        let msg = Rect::from_min_size(pos2(r.left(), r.bottom() + 6.0 * zoom), vec2(r.width(), galley.size().y + 6.0));
        p.rect_filled(msg, 4.0, t.err_bg);
        p.rect_stroke(msg, 4.0, Stroke::new(1.0_f32, t.err), StrokeKind::Inside);
        p.galley(msg.left_top() + vec2(8.0, 3.0), galley, t.text);
    }
}

fn node_status(v: &View, id: &str, bypass: bool, unsupported: bool, error: bool) -> (String, Color32) {
    let t = v.theme;
    if bypass { return ("muted".into(), t.faint); }
    if unsupported { return ("unsupported".into(), t.warn); }
    if error { return ("failed".into(), t.err); }
    if v.cooking.cooking() { return ("cooking".into(), t.accent); }
    if v.cooking.stats.cached.iter().any(|n| n == id) { return ("cached".into(), t.dim); }
    if let Some((_, ms)) = v.cooking.stats.node_ms.iter().find(|(n, _)| n == id) {
        return (format!("{ms:.1} ms"), if *ms > 100.0 { t.warn } else { t.dim });
    }
    ("not cooked".into(), t.dim)
}
fn ellipsis(text: &str, width: f32, font_size: f32) -> String {
    let count = (width / (font_size * 0.61)).floor().max(1.0) as usize;
    if text.chars().count() <= count { text.into() } else { format!("{}…", text.chars().take(count.saturating_sub(1)).collect::<String>()) }
}
fn dashed_rect(p: &egui::Painter, r: Rect, color: Color32) {
    for (a, b) in [(r.left_top(), r.right_top()), (r.right_top(), r.right_bottom()), (r.right_bottom(), r.left_bottom()), (r.left_bottom(), r.left_top())] {
        let steps = (a.distance(b) / 6.0).ceil().max(1.0) as usize;
        for i in (0..steps).step_by(2) { p.line_segment([a.lerp(b, i as f32 / steps as f32), a.lerp(b, (i + 1).min(steps) as f32 / steps as f32)], Stroke::new(1.0_f32, color)); }
    }
}
fn fit_icon(p: &egui::Painter, center: Pos2, color: Color32) {
    let s = Stroke::new(1.4_f32, color);
    for x in [-1.0, 1.0] { for y in [-1.0, 1.0] {
        let corner = center + vec2(6.0 * x, 6.0 * y);
        p.line_segment([corner, corner - vec2(3.0 * x, 0.0)], s);
        p.line_segment([corner, corner - vec2(0.0, 3.0 * y)], s);
    } }
}
fn flag_icon(p: &egui::Painter, center: Pos2, flag: usize, color: Color32, zoom: f32) {
    let s = Stroke::new(1.3_f32, color);
    let at = |x: f32, y: f32| center + vec2(x * zoom, y * zoom);
    match flag {
        0 => { p.line_segment([at(-4.0, 0.0), at(3.0, 0.0)], s); p.line_segment([at(0.0, -3.0), at(3.0, 0.0)], s); p.line_segment([at(0.0, 3.0), at(3.0, 0.0)], s); }
        1 => { p.rect_stroke(Rect::from_min_max(at(-3.0, 0.0), at(3.0, 4.0)), 1.0, s, StrokeKind::Inside); p.line_segment([at(-2.0, 0.0), at(-2.0, -2.0)], s); p.line_segment([at(2.0, 0.0), at(2.0, -2.0)], s); p.line_segment([at(-2.0, -2.0), at(2.0, -2.0)], s); }
        2 => dashed_rect(p, Rect::from_min_max(at(-4.0, -4.0), at(4.0, 4.0)), color),
        _ => { p.add(egui::Shape::convex_polygon(vec![at(-4.0, 0.0), at(-2.0, -2.5), at(0.0, -3.0), at(2.0, -2.5), at(4.0, 0.0), at(2.0, 2.5), at(0.0, 3.0), at(-2.0, 2.5)], Color32::TRANSPARENT, s)); p.circle_filled(center, 1.2 * zoom, color); }
    }
}

fn minimap(ui: &egui::Ui, canvas: Rect, state: &mut State, v: &View, positions: &[(String, Pos2)], id: Id) {
    let p = ui.painter().with_clip_rect(canvas);
    let bounds = content_bounds(positions);
    let map = Rect::from_min_size(pos2(canvas.right() - 106.0, canvas.bottom() - 130.0), vec2(96.0, 120.0));
    p.rect_filled(map, 4.0, v.theme.panel);
    p.rect_stroke(map, 4.0, Stroke::new(1.0_f32, v.theme.stroke), StrokeKind::Inside);
    let extent = bounds.expand(20.0);
    let scale = (92.0 / extent.width()).min(116.0 / extent.height());
    let origin = map.min + vec2(2.0, 2.0);
    let project = |point: Pos2| origin + (point - extent.min) * scale;
    for (node_id, at) in positions {
        if let Some((_, def)) = v.graph.nodes.iter().find(|(id, _)| id == node_id) {
            p.rect_filled(Rect::from_min_size(project(*at), CARD * scale), 2.0, v.theme.category_color(&def.node_type));
        }
    }
    let viewport = Rect::from_min_size(pos2(-state.pan.x / state.zoom, -state.pan.y / state.zoom), canvas.size() / state.zoom);
    let visible = Rect::from_min_max(project(viewport.min), project(viewport.max)).intersect(map.shrink(1.0));
    if visible.width() > 0.0 && visible.height() > 0.0 {
        p.rect_stroke(visible, 2.0, Stroke::new(1.0_f32, v.theme.accent), StrokeKind::Inside);
    }
    let response = ui.interact(map, id.with("minimap"), Sense::click_and_drag());
    if response.clicked() || response.dragged() {
        if let Some(at) = response.interact_pointer_pos() {
            let world = extent.min + (at - origin) / scale;
            state.pan = canvas.size() * 0.5 - world.to_vec2() * state.zoom;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::testing::Fixture;

    #[test]
    fn panel_runs_and_status_precedence() {
        let f = Fixture::new();
        assert!(f.run(show).is_empty());
        let mut light = Fixture::new();
        light.theme = crate::ui::theme::Theme::light();
        assert!(light.run(show).is_empty());
        let v = f.view();
        assert_eq!(node_status(&v, "grid1", true, false, false).0, "muted");
        assert_eq!(node_status(&v, "grid1", false, true, false).0, "unsupported");
        assert_eq!(node_status(&v, "grid1", false, false, true).0, "failed");
    }
    #[test]
    fn frame_centers_content() {
        let mut state = State::default();
        let canvas = Rect::from_min_size(Pos2::ZERO, vec2(400.0, 600.0));
        frame_all(&mut state, canvas, Rect::from_min_size(pos2(90.0, 34.0), vec2(220.0, 456.0)));
        assert!((0.25..=2.0).contains(&state.zoom));
        assert!((screen(canvas, &state, pos2(200.0, 262.0)).x - canvas.center().x).abs() < 0.01);
    }
    #[test]
    fn flag_cells_request_the_expected_actions() {
        assert!(matches!(flag_action(0, "a", false), UiAction::Edit { edit: Edit::Bypass { node, flag: true }, continuous: false } if node == "a"));
        assert!(matches!(flag_action(1, "a", false), UiAction::ToggleLocked(node) if node == "a"));
        assert!(matches!(flag_action(2, "a", false), UiAction::ToggleTemplate(node) if node == "a"));
        assert!(matches!(flag_action(3, "a", false), UiAction::SetDisplay(node) if node == "a"));
    }
    #[test]
    fn grid_snap_and_ctrl_inversion() {
        let at = pos2(47.0, -17.0);
        assert_eq!(snap(at, 32.0, true, false), pos2(32.0, -32.0));
        assert_eq!(snap(at, 32.0, true, true), at);
        assert_eq!(snap(at, 32.0, false, false), at);
        assert_eq!(snap(at, 32.0, false, true), pos2(32.0, -32.0));
    }
    #[test]
    fn card_guides_snap_left_center_right_and_top() {
        let size = vec2(220.0, 56.0);
        let peers = [pos2(100.0, 80.0)];
        assert_eq!(align_to_cards(pos2(105.0, 75.0), &peers, size, 6.0).0, pos2(100.0, 80.0));
        assert_eq!(align_to_cards(pos2(96.0, 90.0), &peers, size, 6.0).1.x, Some(100.0));
        assert_eq!(align_to_cards(pos2(319.0, 90.0), &peers, size, 6.0), (pos2(320.0, 90.0), Guides { x: Some(320.0), y: None }));
        assert_eq!(align_to_cards(pos2(-9.0, 90.0), &peers, size, 6.0).0.x, -10.0);
        assert_eq!(align_to_cards(pos2(107.0, 80.0), &peers, size, 6.0).1.x, None);
    }
    #[test]
    fn breadcrumbs_are_ready_for_nested_paths() {
        let nested = vec!["subnet".to_owned(), "inner".to_owned()];
        assert_eq!(crumbs(Path::new("C:/work/scene.json"), &nested, "box1", true), vec!["scene", "subnet", "inner", "box1"]);
        assert_eq!(crumbs(Path::new("scene.json"), &[], "gone", false), vec!["scene"]);
    }
    #[test]
    fn tidy_clears_manual_positions() {
        let mut state = State::default();
        state.moved.insert("box1".into(), pos2(123.0, 45.0));
        tidy_state(&mut state);
        assert!(state.moved.is_empty() && state.tidied && state.zoom < 0.0);
    }
    #[test]
    fn wiggle_needs_three_fast_reversals() {
        let mut w = WiggleDetector::default();
        assert!(!w.push(0.0, 17.0));
        assert!(!w.push(0.1, 19.0));
        assert!(!w.push(0.2, -36.0));
        assert!(!w.push(0.3, 36.0));
        assert!(w.push(0.4, -36.0));
    }
    #[test]
    fn wiggle_ignores_small_and_slow_changes() {
        let mut w = WiggleDetector::default();
        for i in 0..12 { assert!(!w.push(i as f64 * 0.1, if i % 2 == 0 { 10.0 } else { -10.0 })); }
        assert!(!w.push(2.0, 25.0));
        assert!(!w.push(2.8, -25.0));
        assert!(!w.push(3.6, 25.0));
        assert!(!w.push(4.4, -25.0));
    }
}
