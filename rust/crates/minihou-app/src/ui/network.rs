//! Network canvas, following Main.dc.html and NodeStates.dc.html.
#[path = "network_layout.rs"]
mod network_layout;
#[path = "network_physics.rs"]
mod network_physics;
#[path = "network_wires.rs"]
mod network_wires;
#[path = "network_menu.rs"]
mod network_menu;

use super::{Actions, UiAction, View};
use eframe::egui::{self, pos2, vec2, Align2, Color32, FontId, Id, PointerButton, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use minihou_app::Edit;
use crate::graph_edit::{AddConnection, WireRef};
use network_wires::{SelectionOp, Cut};
use std::collections::BTreeSet;
use minihou_core::graph::NodeDef;
use std::{collections::HashMap, path::Path};

const CARD: Vec2 = vec2(220.0, 56.0);

/// Pass the persisted preference to the canvas without storing it in document layout state.
pub fn set_physics_snappiness(ctx: &egui::Context, value: f32) {
    ctx.data_mut(|d| d.insert_temp(Id::new("node_physics_snappiness"), value));
}

#[derive(Clone, Default)]
struct Wiggle { extreme: f32, direction: f32, reversals: u8, started: f64, initialized: bool, fired: bool }
impl Wiggle {
    fn update(&mut self, x: f32, time: f64) -> bool {
        if self.fired { return false; }
        if !self.initialized || time - self.started > 0.8 {
            *self = Self { extreme: x, started: time, initialized: true, ..Default::default() };
            return false;
        }
        let delta = x - self.extreme;
        if self.direction == 0.0 {
            if delta.abs() >= 24.0 { self.direction = delta.signum(); self.extreme = x; }
            return false;
        }
        if delta * self.direction >= 0.0 { self.extreme = x; }
        else if delta.abs() >= 24.0 {
            self.direction = -self.direction;
            self.extreme = x;
            self.reversals += 1;
            if self.reversals >= 3 { self.fired = true; return true; }
        }
        false
    }
}

#[derive(Clone)]
struct State { show_links: bool, focus: Option<String>, zoom: f32, pan: Vec2, moved: HashMap<String, Pos2>, snap: bool, physics: bool, springs: Option<network_physics::Springs>, tidied: bool,
    tidy_order: HashMap<String, Pos2>,
    selected_wires: BTreeSet<WireRef>, graph_snapshot: Option<serde_json::Value>,
    cut: Option<Cut>, box_op: SelectionOp,
    wiggle: Wiggle,
    wire: Option<AddConnection>, pickup: Option<crate::graph_edit::WirePickup>,
    drag_before: Option<serde_json::Value>, drag_physics: bool,
    drag: Option<(String, Pos2)>, box_start: Option<Pos2>, alt_pan: bool, menu: Option<network_menu::MenuState> }
impl Default for State {
    fn default() -> Self { Self { show_links: true, focus: None, zoom: 1.0, pan: Vec2::ZERO, moved: HashMap::new(), snap: true, physics: true, springs: None,
        selected_wires: BTreeSet::new(), graph_snapshot: None, cut: None, box_op: SelectionOp::Replace,
        tidied: false, tidy_order: HashMap::new(), wiggle: Wiggle::default(), wire: None, pickup: None, drag_before: None, drag_physics: false, drag: None, box_start: None, alt_pan: false, menu: None } }
}

// Wire selection belongs only to the canvas. A changed graph invalidates it,
// including undo/redo, reload, external edits and parameter-driven graph changes.
fn sync_wire_selection(state: &mut State, doc: &serde_json::Value) {
    if state.graph_snapshot.as_ref() != Some(doc) {
        state.selected_wires.clear();
        state.cut = None;
        state.graph_snapshot = Some(doc.clone());
    }
}

pub fn take_selected_wires(ctx: &egui::Context, path: &Path, doc: &serde_json::Value) -> BTreeSet<WireRef> {
    ctx.data_mut(|d| {
        let id = state_id(path);
        let mut state = d.get_persisted::<State>(id).unwrap_or_default();
        sync_wire_selection(&mut state, doc);
        let wires = std::mem::take(&mut state.selected_wires);
        d.insert_persisted(id, state);
        wires
    })
}

fn state_id(path: &Path) -> Id { Id::new(("network", path.to_string_lossy().as_ref())) }

/// Tidy unlocked cards while preserving the current positions of locked nodes. The action dispatcher should call this when it receives
/// `UiAction::TidyNetwork` from the keymap, then request a repaint.
pub fn tidy(ctx: &egui::Context, path: &Path, doc: &serde_json::Value, flags: &HashMap<String,super::NodeFlags>) {
    let id = state_id(path);
    ctx.data_mut(|d| {
        let mut state = d.get_persisted::<State>(id).unwrap_or_default();
        if let Ok(graph)=minihou_core::Graph::from_value(doc) { tidy_preserving_locks(&mut state,&graph,doc,flags); }
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
pub fn focus(ctx: &egui::Context, path: &Path, node: &str) {
    let id = state_id(path);
    ctx.data_mut(|d| { let mut state=d.get_persisted::<State>(id).unwrap_or_default(); state.focus=Some(node.into()); d.insert_persisted(id,state); });
    ctx.request_repaint();
}
pub fn toggle_links(ctx: &egui::Context, path: &Path) {
    let id=state_id(path);
    ctx.data_mut(|d| { let mut state=d.get_persisted::<State>(id).unwrap_or_default(); state.show_links=!state.show_links; d.insert_persisted(id,state); });
    ctx.request_repaint();
}
pub fn set_links(ctx: &egui::Context, path: &Path, show: bool) {
    let id=state_id(path);
    ctx.data_mut(|d| { let mut state=d.get_persisted::<State>(id).unwrap_or_default(); state.show_links=show; d.insert_persisted(id,state); });
}
/// Honor an explicit noodle placement even when Tidy ignores authored positions.
pub fn place_connected(ctx: &egui::Context, path: &Path, node: &str, pos: [f32; 2]) {
    ctx.data_mut(|d| {
        let id = state_id(path);
        let mut state = d.get_persisted::<State>(id).unwrap_or_default();
        state.moved.insert(node.to_owned(), pos2(pos[0], pos[1]));
        d.insert_persisted(id, state);
    });
}

pub fn open_add_menu(ctx: &egui::Context, path: &Path) {
    let id = state_id(path);
    let at = ctx.input(|i| i.pointer.hover_pos()).unwrap_or(pos2(160., 190.));
    ctx.data_mut(|d| {
        let mut state = d.get_persisted::<State>(id).unwrap_or_default();
        state.menu = if state.menu.is_some() { None } else { Some(network_menu::MenuState::new(at, None, true)) };
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
fn tidy_preserving_locks(state:&mut State, graph:&minihou_core::Graph, doc:&serde_json::Value, flags:&HashMap<String,super::NodeFlags>) {
    let positions=positions_for(state,graph,doc);
    let pinned: HashMap<_,_>=positions.iter().filter(|(id,_)| flags.get(id).is_some_and(|f|f.locked)).cloned().collect();
    tidy_state(state);
    state.tidy_order=positions.into_iter().collect();
    state.moved=pinned;
}

fn tidy_state(state: &mut State) {
    if !state.moved.is_empty() { state.tidy_order = std::mem::take(&mut state.moved); }
    state.drag = None;
    state.springs = None;
    state.tidied = true;
    state.zoom = -1.0;
}

pub fn show(ui: &mut egui::Ui, v: &View, out: &mut Actions) {
    rename_dialog(ui.ctx(),out);
    let old_spacing = ui.spacing().item_spacing;
    ui.spacing_mut().item_spacing.y = 0.0;
    let id = state_id(v.path);
    let mut state = ui.ctx().data_mut(|d| d.get_persisted::<State>(id).unwrap_or_default());
    sync_wire_selection(&mut state, v.doc);
    let (header, _) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::hover());
    header_ui(ui, header, v, out, &mut state);
    let (body, _) = ui.allocate_exact_size(ui.available_size(), Sense::hover());
    let (crumb, canvas) = split_breadcrumb(body);
    breadcrumb_ui(ui, crumb, canvas, v, out, &mut state);
    canvas_ui(ui, canvas, v, out, &mut state, id);
    ui.ctx().data_mut(|d| d.insert_persisted(id, state));
    ui.spacing_mut().item_spacing = old_spacing;
}
fn rename_dialog(ctx: &egui::Context, out: &mut Actions) {
    let id=egui::Id::new("rename_link_node");
    let Some((old,mut new))=ctx.data(|d|d.get_temp::<(String,String)>(id)) else {return};
    let mut open=true;
    egui::Window::new("Rename node").open(&mut open).show(ctx,|ui| {
        ui.text_edit_singleline(&mut new);
        if ui.button("Rename").clicked() {out.push(UiAction::RenameNode {old:old.clone(),new:new.clone()});ctx.data_mut(|d|d.remove::<(String,String)>(id));}
    });
    if open && ctx.data(|d|d.get_temp::<(String,String)>(id)).is_some() {ctx.data_mut(|d|d.insert_temp(id,(old,new)));}
    if !open {ctx.data_mut(|d|d.remove::<(String,String)>(id));}
}

fn header_ui(ui: &mut egui::Ui, r: Rect, v: &View, out: &mut Actions, state: &mut State) {
    let p = ui.painter();
    p.rect_filled(r, 0.0, v.theme.panel2);
    p.line_segment([pos2(r.left(), r.bottom() - 0.5), pos2(r.right(), r.bottom() - 0.5)], Stroke::new(1.0_f32, v.theme.stroke));
    p.text(pos2(r.left()+10.,r.center().y),Align2::LEFT_CENTER,"Network",FontId::proportional(12.),v.theme.text);
    let links_rect=Rect::from_min_size(pos2(r.right()-226.,r.top()+5.),vec2(28.,22.));
    let snap_rect=Rect::from_min_size(pos2(r.right()-194.,r.top()+5.),vec2(28.,22.));
    if header_toggle(ui,links_rect,"links",state.show_links,"Show parameter references (Shift+D)",v) {
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

fn header_toggle(ui:&egui::Ui,r:Rect,id:&str,on:bool,tooltip:&str,v:&View)->bool {
    let response=ui.interact(r,ui.id().with(id),Sense::click()).on_hover_text(tooltip);
    let fill=if on {v.theme.accent.gamma_multiply(0.18)}else if response.hovered(){v.theme.card}else{v.theme.panel2};
    ui.painter().rect_filled(r,3.,fill);
    ui.painter().rect_stroke(r,3.,Stroke::new(1.0_f32,if on {v.theme.accent}else{v.theme.stroke}),StrokeKind::Inside);
    response.clicked()
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
fn frame_node(state: &mut State, canvas: Rect, node: &str, v: &View) {
    let positions = node_positions(state, v);
    if let Some((_, at)) = positions.iter().find(|(id, _)| id == node) { frame_all(state, canvas, Rect::from_min_size(*at, CARD)); }
}


fn canvas_ui(ui: &egui::Ui, r: Rect, v: &View, out: &mut Actions, state: &mut State, id: Id) {
    let ctx = ui.ctx();
    if v.cooking.cooking() { ctx.request_repaint_after(std::time::Duration::from_millis(16)); }
    let pointer = ctx.input(|i| i.pointer.hover_pos());
    let over = pointer.is_some_and(|at| r.contains(at));
    let mut positions = node_positions(state, v);
    if let Some(focus) = state.focus.take() {
        if let Some((_,at)) = positions.iter().find(|(node,_)| *node == focus) {
            state.pan = r.center().to_vec2() - r.min.to_vec2() - (*at + CARD * 0.5).to_vec2() * state.zoom;
        }
    }
    // Cache automatic positions too: adding a root must not shuffle the existing layer.
    state.moved.retain(|node, _| v.graph.nodes.iter().any(|(id, _)| id == node));
    for (node, at) in &positions { state.moved.insert(node.clone(), *at); }

    if !state.physics { state.springs = None; }
    if let Some(springs) = &mut state.springs {
        let snappiness = ctx.data(|d| d.get_temp::<f32>(Id::new("node_physics_snappiness"))).unwrap_or(0.7);
        if springs.step_with_snappiness(&mut positions, ctx.input(|i| i.stable_dt), snappiness) {
            ctx.request_repaint();
        } else if state.drag.is_none() { state.springs = None; }
        for (node, at) in &positions { state.moved.insert(node.clone(), *at); }
    }
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
    let mut tool_dropped = false;
    if over && ctx.input(|i| i.pointer.any_released()) {
        if let Some(tool) = egui::DragAndDrop::take_payload::<super::ToolDrag>(ctx) {
            let at = shelf_drop_position(pointer.unwrap(), r, state.pan, state.zoom, state.snap, ctx.input(|i| i.modifiers.ctrl));
            let mut action = super::shelf::dropped_tool(&tool, at);
            if let Some((added, node)) = crate::graph_edit::add(v.doc, &tool.0, None) {
                if let UiAction::AddNetworkNode { wire, .. } = &mut action {
                    *wire = insertion_candidate_at_point(&added, &node, pointer.unwrap(), &wire_curves(r, state, v, &positions));
                }
            }
            out.push(action);
            state.menu = None;
            tool_dropped = true;
        }
    }
    let secondary_pressed = over && ctx.input(|i| i.pointer.button_pressed(PointerButton::Secondary));
    let shift = ctx.input(|i| i.modifiers.shift);
    if secondary_pressed && shift && state.menu.is_none() && state.wire.is_none() {
        state.cut = pointer.map(Cut::new);
        state.box_start = None;
    }
    if pointer.is_none() || ctx.input(|i| i.key_pressed(egui::Key::Escape) || !i.focused) {
        state.cut = None;
        state.box_start = None;
    }
    // Only Shift-RMB is deferred until release so an ordinary RMB still opens
    // its context menu immediately. A sub-threshold Shift-RMB is a normal click.
    let mut shifted_click = None;
    let drawn_wires = wire_curves(r, state, v, &positions);
    if let Some(cut) = &mut state.cut {
        if let Some(at) = pointer {
            if shift || cut.active { cut.update(at, &drawn_wires); }
        }
        if ctx.input(|i| i.pointer.button_released(PointerButton::Secondary)) {
            let cut = state.cut.take().unwrap();
            if cut.active {
                if !cut.hits.is_empty() { out.push(UiAction::DisconnectWires(cut.hits)); }
            } else { shifted_click = cut.points.first().copied(); }
        } else if !ctx.input(|i| i.pointer.secondary_down()) {
            state.cut = None;
        }
    }
    let right_click = ((secondary_pressed && !shift) || shifted_click.is_some()) && state.wire.is_none();
    let tab = over && state.menu.is_none() && ctx.input(|i| i.key_pressed(egui::Key::Tab));
    if right_click || tab {
        let at = shifted_click.or(pointer).unwrap_or(r.center());
        let target = if right_click { positions.iter().rev().find(|(_, pos)| Rect::from_min_size(screen(r, state, *pos), CARD * state.zoom).contains(at))
            .map(|(id, _)| id.clone()) } else { None };
        state.menu = Some(network_menu::MenuState::new(at, target, tab));
        if tab { ctx.input_mut(|i| i.consume_key(egui::Modifiers::default(), egui::Key::Tab)); }
    }
    let p = ui.painter().with_clip_rect(r);
    p.rect_filled(r, 0.0, v.theme.base);
    grid(&p, r, state, v.theme.grid);
    let ports = canvas_ports(v, &positions);
    let hit_port = pointer.filter(|at| r.contains(*at)).and_then(|at| ports.iter().rev()
        .find(|(_, world)| screen(r, state, *world).distance(at) <= (9.0 * state.zoom).max(6.0)));
    if state.menu.is_none() && ctx.input(|i| i.pointer.button_pressed(PointerButton::Primary)) {
        if let Some((port, _)) = hit_port {
            state.pickup = match port {
                AddConnection::Input { node, port } => crate::graph_edit::WirePickup::begin(v.doc, node, port),
                _ => None,
            };
            state.wire = Some(state.pickup.as_ref().map_or_else(|| port.clone(), |p| AddConnection::Output(p.original.from.clone())));
            state.box_start = None;
            state.drag = None;
            state.springs = None;
        }
    }
    let blocked = tool_dropped || state.menu.is_some() || state.wire.is_some() || state.cut.is_some();
    if ctx.input(|i| i.key_pressed(egui::Key::Escape) || !i.focused || i.pointer.button_pressed(PointerButton::Secondary)) {
        state.wire = None;
        state.pickup = None;
    }
    if state.wire.is_some() && ctx.input(|i| i.pointer.button_released(PointerButton::Primary)) {
        let source = state.wire.take().unwrap();
        if let Some(pickup) = state.pickup.take() {
            let target = hit_port.map(|(target, _)| target.clone()).or_else(|| {
                let at = pointer.filter(|at| r.contains(*at))?;
                let (node, _) = positions.iter().rev().find(|(_, pos)| Rect::from_min_size(screen(r, state, *pos), CARD * state.zoom).contains(at))?;
                let (_, def) = v.graph.nodes.iter().find(|(id, _)| id == node)?;
                let port = minihou_core::nodes::get(&def.node_type)?.inputs.first()?;
                Some(AddConnection::Input { node: node.clone(), port: port.name.into() })
            });
            let next = match target {
                Some(AddConnection::Input { node, port }) => pickup.release(v.doc, Some((&node, &port))),
                None if over => pickup.release(v.doc, None),
                _ => None,
            };
            if let Some(next) = next { out.push(UiAction::NetworkGesture { expected: v.doc.clone(), before: v.doc.clone(), next }); }
        } else if let Some((target, _)) = hit_port {
            if let Some(action) = wire_connection(&source, target) { out.push(action); }
        } else if let Some(at) = pointer.filter(|at| r.contains(*at)) {
            let on_card = positions.iter().any(|(_, pos)| Rect::from_min_size(screen(r, state, *pos), CARD * state.zoom).contains(at));
            let on_map = Rect::from_min_size(pos2(r.right() - 106.0, r.bottom() - 130.0), vec2(96.0, 120.0)).contains(at);
            if !on_card && !on_map {
                let world = ((at - r.min - state.pan) / state.zoom).to_pos2();
                let world = snap(world - CARD * 0.5, 32.0, state.snap, ctx.input(|i| i.modifiers.ctrl));
                state.menu = Some(network_menu::MenuState::connected(at, source, world));
            }
        }
    }
    // Pointer disappearance/cancellation must not leave a stale noodle.
    if !ctx.input(|i| i.pointer.primary_down()) || pointer.is_none() { state.wire = None; state.pickup = None; }
    let sense = if blocked { Sense::hover() } else { Sense::click_and_drag() };
    let background = ui.interact(r, id.with("box_selection"), sense);
    let background_hit_card = pointer.is_some_and(|at| positions.iter().any(|(_, pos)| Rect::from_min_size(screen(r, state, *pos), CARD * state.zoom).contains(at)));
    if background.double_clicked() && !background_hit_card {
        state.menu = Some(network_menu::MenuState::new(pointer.unwrap_or(r.center()), None, true));
    }
    if background.drag_started_by(PointerButton::Primary) && !state.alt_pan && !background_hit_card {
        state.box_start = ctx.input(|i| i.pointer.press_origin());
        state.box_op = ctx.input(|i| SelectionOp::modifiers(i.modifiers.shift, i.modifiers.ctrl));
    }
    if background.clicked() && !background_hit_card && !blocked && hit_port.is_none() {
        let hit = pointer.and_then(|at| drawn_wires.iter().rev().find(|(_, points)|
            points.windows(2).any(|w| network_wires::distance(at,w[0],w[1]) <= 6.0)));
        let op = ctx.input(|i| SelectionOp::modifiers(i.modifiers.shift, i.modifiers.ctrl));
        network_wires::select(&mut state.selected_wires, hit.map(|(wire,_)| wire.clone()), op);
        if matches!(op, SelectionOp::Replace) { out.push(UiAction::SelectSet { ids: vec![], extend: false }); }
    }
    if positions.is_empty() {
        p.text(pos2(r.center().x, r.center().y - 12.0), Align2::CENTER_CENTER, "The network is empty", FontId::proportional(15.0), v.theme.text);
        let hint = p.layout("Open a graph file, press Tab, or pick a tool from the shelf above.".into(), FontId::proportional(12.0), v.theme.dim, (r.width() - 80.0).max(80.0));
        p.galley(pos2(r.center().x - hint.size().x * 0.5, r.center().y + 5.0), hint, v.theme.dim);
    } else {
        if state.show_links { dependency_links(&p,r,state,v,&positions); }
        wires(&p, r, state, v, &positions);
        let mut guide = None;
        let ref_links = minihou_app::value_link_ui::links(v.doc);
        for (node_id, at) in &positions {
            let Some((_, node)) = v.graph.nodes.iter().find(|(n, _)| n == node_id) else { continue };
            let card = Rect::from_min_size(screen(r, state, *at), CARD * state.zoom);
            if !card.intersects(r) { continue; }
            let face = Rect::from_min_max(pos2(card.left() + 20.0 * state.zoom, card.top()), pos2(card.right() - 20.0 * state.zoom, card.bottom()));
            let response = ui.interact(face, id.with(("node", node_id)), sense)
                .on_hover_text(format!("{} · {} · {} params", node_id, node.node_type, node.params.as_ref().map_or(0, |p| p.len())));
            response.clone().context_menu(|ui| {
                if ui.button("Rename node...").clicked() {
                    ui.ctx().data_mut(|d|d.insert_temp(egui::Id::new("rename_link_node"),(node_id.clone(),node_id.clone())));
                    ui.close();
                }
                let links = minihou_app::value_link_ui::links(v.doc);
                for link in links.iter().filter(|l| &l.target == node_id && !l.broken) {
                    if ui.button(format!("Go to source: {}.{}",link.source,link.source_param)).clicked() {
                        out.push(UiAction::NavigateLink {node:link.source.clone(),param:link.source_param.clone()}); ui.close();
                    }
                }
                for link in links.iter().filter(|l| &l.source == node_id) {
                    if ui.button(format!("Go to target: {}.{}",link.target,link.target_param)).clicked() {
                        out.push(UiAction::NavigateLink {node:link.target.clone(),param:link.target_param.clone()}); ui.close();
                    }
                }
            });
            if response.double_clicked() { out.push(UiAction::SetDisplay(node_id.clone())); }
            else if response.clicked() {
                state.selected_wires.clear();
                out.push(if ctx.input(|i| i.modifiers.shift || i.modifiers.ctrl) { UiAction::ToggleSelect(node_id.clone()) } else { UiAction::Select(node_id.clone()) });
            }
            if response.drag_started_by(PointerButton::Primary) {
                state.selected_wires.clear();
                if !v.selected_set.contains(node_id) { out.push(UiAction::Select(node_id.clone())); }
                state.drag = Some((node_id.clone(), *at)); state.springs = None; state.box_start = None;
                let mut before = v.doc.clone();
                for (id, at) in &positions { before["nodes"][id]["pos"] = serde_json::json!([at.x, at.y]); }
                state.drag_before = Some(before);
                state.drag_physics = false;
                state.wiggle = Wiggle::default();
                if let Some(origin) = ctx.input(|i| i.pointer.press_origin()) {
                    state.wiggle.update(origin.x, ctx.input(|i| i.time));
                }
            }
            if response.dragged_by(PointerButton::Primary) && !ctx.input(|i| i.key_down(egui::Key::Space)) {
                if let Some(pointer) = response.interact_pointer_pos() {
                    if state.wiggle.update(pointer.x, ctx.input(|i| i.time)) {
                        out.push(UiAction::DetachNode(node_id.clone()));
                        state.springs = None;
                    }
                }
                if !state.wiggle.fired && state.physics && ctx.input(|i| i.modifiers.shift) {
                    state.drag_physics = true;
                    if state.springs.is_none() {
                        let mut pinned = if v.selected_set.contains(node_id) { v.selected_set.clone() } else { Default::default() };
                        pinned.insert(node_id.clone());
                        state.springs = Some(network_physics::Springs::new(v.graph, &positions, pinned));
                    }
                    // The solver runs at the start of the next canvas frame. Schedule it
                    // even if this was the final pointer event before release.
                    ctx.request_repaint();
                } else { state.springs = None; }
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

            for flag in 0..4 {
                if ui.interact(flag_rect(card, flag), id.with(("flag", node_id, flag)), if blocked { Sense::hover() } else { Sense::click() })
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
            let count = ref_links.iter().filter(|l| &l.source == node_id || &l.target == node_id).count();
            if count > 0 {
                let broken = ref_links.iter().any(|l| l.broken && &l.target == node_id);
                p.text(card.right_top() + vec2(-25., 4.) * state.zoom, Align2::RIGHT_TOP,
                    if broken { format!("⚠ {count}") } else { format!("↗ {count}") }, FontId::proportional(10. * state.zoom),
                    if broken { Color32::from_rgb(235,80,80) } else { Color32::from_rgb(62,190,110) });
            }
        }
        if let Some(lines) = guide {
            if let Some(x) = lines.x { let x = screen(r, state, pos2(x, 0.0)).x; p.line_segment([pos2(x, r.top()), pos2(x, r.bottom())], Stroke::new(1.0_f32, v.theme.accent)); }
            if let Some(y) = lines.y { let y = screen(r, state, pos2(0.0, y)).y; p.line_segment([pos2(r.left(), y), pos2(r.right(), y)], Stroke::new(1.0_f32, v.theme.accent)); }
        }
        if !blocked { minimap(ui, r, state, v, &positions, id); }
    }
    let cancelled = pointer.is_none() || ctx.input(|i| i.key_pressed(egui::Key::Escape) || !i.focused || i.pointer.button_pressed(PointerButton::Secondary));
    if cancelled && state.drag.is_some() {
        if let Some(before) = state.drag_before.take() {
            for (id, _) in &positions { if let Some(at) = json_pos(&before, id) { state.moved.insert(id.clone(), at); } }
        }
        state.drag = None;
        state.springs = None;
    }
    let current_positions: Vec<_> = positions.iter().map(|(id, at)| (id.clone(), state.moved.get(id).copied().unwrap_or(*at))).collect();
    let current_wires = wire_curves(r, state, v, &current_positions);
    let candidate = state.drag.as_ref().filter(|(node, _)| !state.drag_physics && !state.wiggle.fired &&
        !ctx.input(|i| i.modifiers.shift || i.modifiers.alt) && (!v.selected_set.contains(node) || v.selected_set.len() <= 1))
        .and_then(|(node, _)| {
            let at = current_positions.iter().find(|(id, _)| id == node)?.1;
            pointer.and_then(|at| insertion_candidate_at_point(v.doc, node, at, &current_wires))
                .or_else(|| insertion_candidate(v.doc, node, Rect::from_min_size(screen(r, state, at), CARD * state.zoom), &current_wires))
        });
    if let Some(wire) = &candidate {
        if let Some((_, points)) = current_wires.iter().find(|(w, _)| w == wire) {
            p.add(egui::Shape::line(points.clone(), Stroke::new(3.5_f32, v.theme.accent)));
        }
    }
    if state.drag.is_some() && ctx.input(|i| i.pointer.button_released(PointerButton::Primary)) {
        if let (Some(wire), Some(before), Some((node, _))) = (candidate, state.drag_before.take(), state.drag.as_ref()) {
            let mut moved = v.doc.clone();
            for (id, at) in &current_positions { moved["nodes"][id]["pos"] = serde_json::json!([at.x, at.y]); }
            if let Some(next) = crate::graph_edit::insert_existing(&moved, node, &wire) {
                out.push(UiAction::NetworkGesture { expected: v.doc.clone(), before, next });
            }
        }
        state.drag = None;
        state.drag_before = None;
    }
    // Shelf and Tab-menu drag payloads share the same preview and drop edit.
    if let (Some(tool), Some(at)) = (egui::DragAndDrop::payload::<super::ToolDrag>(ctx), pointer) {
        if over {
            if let Some((value, node)) = crate::graph_edit::add(v.doc, &tool.0, None) {
                if let Some(wire) = insertion_candidate_at_point(&value, &node, at, &drawn_wires) {
                    if let Some((_, points)) = drawn_wires.iter().find(|(w, _)| w == &wire) {
                        p.add(egui::Shape::line(points.clone(), Stroke::new(3.5_f32, v.theme.accent)));
                    }
                }
            }
        }
        let rect = if over {
            let world = shelf_drop_position(at, r, state.pan, state.zoom, state.snap, ctx.input(|i| i.modifiers.ctrl));
            Rect::from_min_size(screen(r, state, world), CARD * state.zoom)
        } else { Rect::from_center_size(at, CARD) };
        let ghost = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, id.with("tool_ghost")));
        ghost_card(&ghost, rect, &tool.0, v, if over { state.zoom } else { 1.0 });
    }
    if let (Some(start), Some(end)) = (state.box_start, pointer) {
        if background.dragged_by(PointerButton::Primary) {
            let box_rect = Rect::from_two_pos(start, end);
            p.rect_stroke(box_rect, 0.0, Stroke::new(1.0_f32, v.theme.accent), StrokeKind::Inside);
        }
        if background.drag_stopped_by(PointerButton::Primary) {
            let box_rect = Rect::from_two_pos(start, end);
            let hits = positions.iter().filter(|(_, at)| Rect::from_min_size(screen(r, state, *at), CARD * state.zoom).intersects(box_rect)).map(|(id, _)| id.clone());
            let mut nodes = v.selected_set.clone();
            network_wires::select(&mut nodes, hits, state.box_op);
            out.push(UiAction::SelectSet { ids: nodes.into_iter().collect(), extend: false });
            let hits = drawn_wires.iter().filter(|(_, points)| network_wires::in_box(points, box_rect)).map(|(wire,_)| wire.clone());
            network_wires::select(&mut state.selected_wires, hits, state.box_op);
            state.box_start = None;
        }
    }
    if let Some(cut) = state.cut.as_ref().filter(|cut| cut.active) {
        // Thin white underlay and red freehand trace, with red cut candidates.
        p.add(egui::Shape::line(cut.points.clone(), Stroke::new(2.0_f32, Color32::WHITE)));
        p.add(egui::Shape::line(cut.points.clone(), Stroke::new(1.0_f32, Color32::from_rgb(255,100,100))));
    }
    let live = state.wire.as_ref().map(|port| (port, pointer))
        .or_else(|| { let menu = state.menu.as_ref()?; menu.connect_to.as_ref().map(|(port, _)| (port, Some(menu.at()))) });
    if let Some((port, Some(end))) = live {
        if let Some((_, start)) = ports.iter().find(|(candidate, _)| candidate == port) {
            let start = screen(r, state, *start);
            let (a, b) = if matches!(port, AddConnection::Output(_)) { (start, end) } else { (end, start) };
            noodle(&p, a, b, v.theme.accent);
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

fn node_positions(state: &State, v: &View) -> Vec<(String, Pos2)> {
    positions_for(state, v.graph, v.doc)
}

fn positions_for(state: &State, graph: &minihou_core::Graph, doc: &serde_json::Value) -> Vec<(String, Pos2)> {
    let authored = |id: &str| json_pos(doc, id);
    network_layout::layout_ordered(graph, |id| state.moved.get(id).copied()
        .or_else(|| (!state.tidied).then(|| authored(id)).flatten()),
        |id| state.tidy_order.get(id).copied().or_else(|| authored(id)))
}

fn insertion_candidate(doc: &serde_json::Value, node: &str, rect: Rect, wires: &[(WireRef, Vec<Pos2>)]) -> Option<WireRef> {
    wires.iter().filter_map(|(wire, points)| {
        let distance = network_wires::node_hit(points, rect)?;
        crate::graph_edit::insert_existing(doc, node, wire)?;
        Some((wire, distance))
    }).min_by(|(a, da), (b, db)| da.total_cmp(db).then_with(|| a.cmp(b))).map(|(wire, _)| wire.clone())
}

fn insertion_candidate_at_point(doc: &serde_json::Value, node: &str, at: Pos2, wires: &[(WireRef, Vec<Pos2>)]) -> Option<WireRef> {
    crate::noodle_hit::closest_at_point(wires, at, 8.0, |wire| crate::graph_edit::insert_existing(doc, node, wire).is_some())
}

pub fn insert_added(_ctx: &egui::Context, _path: &Path, old: &serde_json::Value, next: &mut serde_json::Value, node: &str, _pos: [f32; 2], drop_wire: Option<&WireRef>) -> bool {
    let candidate = drop_wire.cloned();
    if let Some(wire) = candidate {
        if let Some(mut inserted) = crate::graph_edit::insert_existing(next, node, &wire) {
            // A stream insertion keeps the existing display/output flags.
            if let Some(display) = old.get("display") { inserted["display"] = display.clone(); }
            else { inserted.as_object_mut().unwrap().remove("display"); }
            *next = inserted;
            return true;
        }
    }
    false
}

fn shelf_drop_position(at: Pos2, canvas: Rect, pan: Vec2, zoom: f32, snap_enabled: bool, invert: bool) -> Pos2 {
    let center = ((at - canvas.min - pan) / zoom).to_pos2();
    snap(center - CARD * 0.5, 32.0, snap_enabled, invert)
}

fn ghost_card(p: &egui::Painter, rect: Rect, node_type: &str, v: &View, zoom: f32) {
    let theme = v.theme;
    p.rect_filled(rect, 6.0 * zoom, theme.card.gamma_multiply(0.7));
    p.rect_stroke(rect, 6.0 * zoom, Stroke::new(1.5, theme.accent.gamma_multiply(0.8)), StrokeKind::Inside);
    let dot = pos2(rect.left() + 35.0 * zoom, rect.center().y);
    p.circle_filled(dot, 5.0 * zoom, theme.category_color(node_type).gamma_multiply(0.8));
    p.text(dot + vec2(13.0 * zoom, 0.0), Align2::LEFT_CENTER, node_type, FontId::proportional(13.0 * zoom), theme.text.gamma_multiply(0.8));
}

/// Persist insertion layout in the add's document edit, using the actual canvas
/// positions (which may have been dragged or tidied since the file was saved).
pub fn position_add(ctx: &egui::Context, path: &Path, old: &serde_json::Value,
    next: &mut serde_json::Value, after: Option<&str>) -> Option<minihou_app::Document> {
    let (Ok(graph), Ok(next_graph)) = (minihou_core::Graph::from_value(old), minihou_core::Graph::from_value(next)) else { return None };
    let state = ctx.data_mut(|d| d.get_persisted::<State>(state_id(path)).unwrap_or_default());
    let mut positions = positions_for(&state, &graph, old);
    // History must remember the pre-add canvas too, including unsaved drags.
    let mut before = old.clone();
    for (id, at) in &positions { before["nodes"][id]["pos"] = serde_json::json!([at.x, at.y]); }
    if let Some(anchor) = after.filter(|id| old["nodes"].get(*id).is_some()) {
        let before = old["nodes"][anchor]["type"] == "output";
        let inserted = next_graph.nodes.iter().find(|(id, node)| old["nodes"].get(id).is_none() &&
            if before { next_graph.nodes.iter().any(|(target, def)| target == anchor &&
                def.inputs.iter().any(|(_, refs)| refs.contains(id))) }
            else { node.inputs.iter().any(|(_, refs)| refs.iter().any(|source| source == anchor)) });
        if let Some((id, _)) = inserted {
            positions = network_layout::insert_position(&graph, &positions, anchor, id, before);
        }
    }
    let lookup: HashMap<_, _> = positions.into_iter().collect();
    for (id, at) in network_layout::layout(&next_graph, |id| lookup.get(id).copied()) {
        next["nodes"][&id]["pos"] = serde_json::json!([at.x, at.y]);
    }
    crate::graph_edit::validated(before).ok()
}

pub fn layout_changed(old: &serde_json::Value, next: &serde_json::Value) -> bool {
    let positions = |doc: &serde_json::Value| -> HashMap<String, Pos2> {
        doc["nodes"].as_object().into_iter().flat_map(|nodes| nodes.keys())
            .filter_map(|id| json_pos(doc, id).map(|at| (id.clone(), at))).collect()
    };
    positions(old) != positions(next)
}

/// Discard stale canvas overrides after a document layout change or history edit.
pub fn restore_positions(ctx: &egui::Context, path: &Path, doc: &serde_json::Value) {
    ctx.data_mut(|d| {
        let id = state_id(path);
        let mut state = d.get_persisted::<State>(id).unwrap_or_default();
        state.moved.clear();
        state.springs = None;
        state.tidied = false;
        if let Ok(graph) = minihou_core::Graph::from_value(doc) {
            state.moved = positions_for(&state, &graph, doc).into_iter().collect();
        }
        d.insert_persisted(id, state);
    });
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

fn json_pos(doc: &serde_json::Value, id: &str) -> Option<Pos2> {
    let a = doc.get("nodes")?.get(id)?.get("pos")?.as_array()?;
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

/// Slots include unused declared inputs; multi-input wires share their named port.
fn input_slots(node: &NodeDef) -> Vec<String> {
    minihou_core::nodes::get(&node.node_type).map(|spec| spec.inputs.iter().flat_map(|port| {
        let count = node.inputs.iter().find(|(name, _)| name == port.name).map_or(0, |(_, refs)| refs.len()).max(1);
        std::iter::repeat_n(port.name.to_owned(), count)
    }).collect()).unwrap_or_default()
}

fn input_offset(index: usize, count: usize) -> Vec2 {
    vec2(CARD.x * 0.5 + (index as f32 - (count.saturating_sub(1)) as f32 * 0.5) * 16.0, 0.0)
}

fn canvas_ports(v: &View, positions: &[(String, Pos2)]) -> Vec<(AddConnection, Pos2)> {
    let mut ports = Vec::new();
    for (id, at) in positions {
        let Some((_, node)) = v.graph.nodes.iter().find(|(n, _)| n == id) else { continue };
        let inputs = input_slots(node);
        for (index, port) in inputs.iter().enumerate() {
            ports.push((AddConnection::Input { node: id.clone(), port: port.clone() }, *at + input_offset(index, inputs.len())));
        }
        if node.node_type != "output" { ports.push((AddConnection::Output(id.clone()), *at + vec2(CARD.x * 0.5, CARD.y))); }
    }
    ports
}

fn wire_connection(a: &AddConnection, b: &AddConnection) -> Option<UiAction> {
    let (from, to, port) = match (a, b) {
        (AddConnection::Output(from), AddConnection::Input { node, port }) |
        (AddConnection::Input { node, port }, AddConnection::Output(from)) => (from, node, port),
        _ => return None,
    };
    (from != to).then(|| UiAction::ConnectNodes { from: from.clone(), to: to.clone(), port: Some(port.clone()) })
}

fn noodle(p: &egui::Painter, a: Pos2, b: Pos2, color: Color32) {
    p.add(egui::Shape::line(network_wires::curve(a,b), Stroke::new(1.5_f32, color)));
}

fn link_curve(a: Pos2, b: Pos2) -> Vec<Pos2> {
    let bend = ((b.x - a.x).abs() * 0.45).max(40.);
    let c = a + vec2(bend, 0.);
    let d = b - vec2(bend, 0.);
    (0..=32).map(|i| {
        let t = i as f32 / 32.; let u = 1. - t;
        pos2(u*u*u*a.x + 3.*u*u*t*c.x + 3.*u*t*t*d.x + t*t*t*b.x,
             u*u*u*a.y + 3.*u*u*t*c.y + 3.*u*t*t*d.y + t*t*t*b.y)
    }).collect()
}
fn link_hit(points: &[Pos2], at: Pos2) -> bool {
    points.windows(2).any(|w| network_wires::distance(at,w[0],w[1]) <= 6.)
}
fn dependency_links(p: &egui::Painter, canvas: Rect, state: &State, v: &View, positions: &[(String, Pos2)]) {
    let lookup: HashMap<&str,Pos2> = positions.iter().map(|(id,at)|(id.as_str(),*at)).collect();
    let pointer = p.ctx().input(|i| i.pointer.hover_pos());
    for link in minihou_app::value_link_ui::links(v.doc) {
        let Some(&target) = lookup.get(link.target.as_str()) else { continue; };
        let b = screen(canvas,state,target + vec2(0.,CARD.y*0.5));
        let a = lookup.get(link.source.as_str()).map(|at|screen(canvas,state,*at + vec2(CARD.x,CARD.y*0.5)))
            .unwrap_or(b - vec2(80. * state.zoom, 35. * state.zoom));
        let points = link_curve(a,b);
        let hot = v.selected == link.source || v.selected == link.target || pointer.is_some_and(|at| link_hit(&points,at));
        let color = if link.broken { Color32::from_rgb(239,68,68) } else { Color32::from_rgb(56,189,248) }.gamma_multiply(if hot { 0.95 } else { 0.25 });
        for (i,segment) in points.windows(2).enumerate() {
            if i % 4 < 2 { p.line_segment([segment[0],segment[1]],Stroke::new(if hot {2.} else {1.2},color)); }
        }
        if hot {
            let mid=points[16];
            p.text(mid,Align2::CENTER_BOTTOM,format!("{} → {}",link.source_param,link.target_param),FontId::proportional(10.),color);
        }
    }
}

#[cfg(test)] mod link_geometry_tests {
    use super::*;
    #[test] fn curved_link_hit_is_separate_from_card_and_noodle_geometry() {
        let points=link_curve(pos2(10.,20.),pos2(210.,70.));
        assert_eq!(points.first().copied(),Some(pos2(10.,20.)));
        assert_eq!(points.last().copied(),Some(pos2(210.,70.)));
        assert!(link_hit(&points,points[16]));
        assert!(!link_hit(&points,pos2(100.,300.)));
    }
}

fn wire_curves(canvas: Rect, state: &State, v: &View, positions: &[(String, Pos2)]) -> Vec<(WireRef, Vec<Pos2>)> {
    curves_for(canvas, state, v.graph, positions)
}
fn curves_for(canvas: Rect, state: &State, graph: &minihou_core::Graph, positions: &[(String, Pos2)]) -> Vec<(WireRef, Vec<Pos2>)> {
    let mut wires = Vec::new();
    let mut seen = BTreeSet::new();
    let lookup: HashMap<&str, Pos2> = positions.iter().map(|(id, at)| (id.as_str(), *at)).collect();
    for (target, def) in &graph.nodes {
        let Some(&end) = lookup.get(target.as_str()) else { continue };
        let slots = input_slots(def);
        let mut used = HashMap::<&str, usize>::new();
        for (port, source) in def.inputs.iter().flat_map(|(port, refs)| refs.iter().map(move |source| (port, source))) {
            let offset = used.entry(port).or_default();
            let Some(index) = slots.iter().position(|name| name == port).map(|i| i + *offset) else { continue };
            *offset += 1;
            let Some(&start) = lookup.get(source.as_str()) else { continue };
            let a = screen(canvas, state, start + vec2(CARD.x * 0.5, CARD.y));
            let b = screen(canvas, state, end + input_offset(index, slots.len()));
            let wire = WireRef { from: source.clone(), to: target.clone(), port: port.clone() };
            if seen.insert(wire.clone()) { wires.push((wire, network_wires::curve(a,b))); }
        }
    }
    wires
}

fn wires(p: &egui::Painter, canvas: Rect, state: &State, v: &View, positions: &[(String, Pos2)]) {
    for (wire, points) in wire_curves(canvas, state, v, positions) {
        if state.pickup.as_ref().is_some_and(|p| p.original == wire) { continue; }
        let cutting = state.cut.as_ref().is_some_and(|cut| cut.hits.contains(&wire));
        let selected = state.selected_wires.contains(&wire);
        let color = if cutting { Color32::from_rgb(255,100,100) }
            else if selected || v.selected == wire.from || v.selected == wire.to { v.theme.accent }
            else { v.theme.wire };
        p.add(egui::Shape::line(points, Stroke::new(if selected || cutting { 2.5_f32 } else { 1.5_f32 }, color)));
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

fn exposed_to_blender(doc: &serde_json::Value, id: &str) -> bool {
    minihou_app::exposed_nodes(doc).contains(id)
}

fn blender_badge(p: &egui::Painter, center: Pos2, v: &View, zoom: f32) {
    let orange = Color32::from_rgb(234, 118, 0);
    let eye = center + vec2(1.5, 1.0) * zoom;
    let stroke = Stroke::new(2.0 * zoom, orange);
    p.circle_stroke(eye, 4.2 * zoom, stroke);
    for (a, b) in [((-7.0, -2.0), (0.0, -2.0)), ((-4.5, -6.0), (4.0, -2.0)), ((-7.0, 3.5), (-1.0, -1.5))] {
        p.line_segment([center + vec2(a.0, a.1) * zoom, center + vec2(b.0, b.1) * zoom], stroke);
    }
    p.circle_filled(eye, 2.5 * zoom, v.theme.card);
    p.circle_filled(eye, 1.8 * zoom, Color32::from_rgb(38, 139, 210));
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
    let exposed = exposed_to_blender(v.doc, id);
    if zoom >= 0.7 {
        p.rect_filled(Rect::from_min_size(pos2(face.left(), face.top() + 2.0 * zoom), vec2(10.0 * zoom, 10.0 * zoom)), 2.0 * zoom, cat);
        let cooked = v.cooking.stats.cooked.iter().chain(v.cooking.stats.cached.iter()).any(|n| n == id);
        let backend_width = if cooked && !node.bypass && !unsupported { 24.0 * zoom } else { 0.0 };
        let tag_width = backend_width + if exposed { 18.0 * zoom } else { 0.0 };
        if exposed { blender_badge(p, pos2(face.right() - backend_width - 9.0 * zoom, face.top() + 7.5 * zoom), v, zoom); }
        if backend_width > 0.0 {
            let tag = Rect::from_min_size(pos2(face.right() - backend_width, face.top()), vec2(backend_width, 15.0 * zoom));
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
        p.text(pos2(face.left() + 16.0 * zoom, r.center().y), Align2::LEFT_CENTER, ellipsis(id, face.width() - (16.0 + if exposed { 18.0 } else { 0.0 }) * zoom, 12.0), FontId::monospace(12.0), text);
        if exposed { blender_badge(p, pos2(face.right() - 7.0 * zoom, r.center().y), v, zoom); }
    } else { p.rect_filled(r.shrink(2.0), 3.0, cat); }
    let inputs = input_slots(node);
    for index in 0..inputs.len() {
        p.circle(r.min + input_offset(index, inputs.len()) * zoom, 5.0 * zoom, t.base, Stroke::new(1.5_f32, t.stroke2));
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

    #[test] fn shelf_drop_centers_card_and_only_snaps_when_enabled() {
        let canvas = Rect::from_min_size(pos2(100., 80.), vec2(1000., 700.));
        let pan = vec2(-40., 25.);
        let zoom = 0.5;
        let pointer = canvas.min + pan + vec2(645., -61.) * zoom;
        assert_eq!(shelf_drop_position(pointer, canvas, pan, zoom, true, false), pos2(544., -96.));
        assert_eq!(shelf_drop_position(pointer, canvas, pan, zoom, false, false), pos2(535., -89.));
        assert_eq!(shelf_drop_position(pointer, canvas, pan, zoom, true, true), pos2(535., -89.));
    }

    #[test] fn shelf_payload_release_on_canvas_creates_a_standalone_node_at_pointer() {
        let f = Fixture::new();
        let view = f.view();
        let ctx = egui::Context::default();
        let canvas = Rect::from_min_size(pos2(100., 80.), vec2(1000., 700.));
        let mut state = State { physics: false, zoom: 0.5, pan: vec2(-40., 25.), ..Default::default() };
        let at = canvas.min + state.pan + vec2(645., 323.) * state.zoom;
        let mut out = Actions::default();
        for pressed in [true, false] {
            let input = egui::RawInput { events: vec![egui::Event::PointerMoved(at),
                egui::Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() }],
                ..Default::default() };
            let _ = ctx.run(input, |ctx| {
                if pressed { egui::DragAndDrop::set_payload(ctx, super::super::ToolDrag("transform".into())); }
                egui::CentralPanel::default().show(ctx, |ui| {
                    canvas_ui(ui, canvas, &view, &mut out, &mut state, Id::new("shelf_drop_test"));
                });
            });
        }
        assert!(matches!(out.0.as_slice(), [UiAction::AddNetworkNode { node_type, pos, .. }]
            if node_type == "transform" && *pos == [544., 288.]));
        assert!(!egui::DragAndDrop::has_payload_of_type::<super::super::ToolDrag>(&ctx));
    }

    #[test]
    fn stream_add_uses_cached_layout_and_undo_redo_restores_positions() {
        let ctx = egui::Context::default();
        let path = Path::new("insertion-test.json");
        let old = serde_json::json!({"version":1,"display":"out","nodes":{
            "a":{"type":"normal"},"out":{"type":"output","inputs":{"geo":"a"}}
        }});
        for anchor in ["out", "a"] {
            let mut state = State::default();
            state.moved = HashMap::from([("a".into(),pos2(320.,96.)),("out".into(),pos2(320.,256.))]);
            ctx.data_mut(|d| d.insert_persisted(state_id(path), state));
            let (mut next, id) = crate::graph_edit::add(&old, "normal", Some(anchor)).unwrap();
            let mut doc = position_add(&ctx, path, &old, &mut next, Some(anchor)).unwrap();
            assert_eq!(next["nodes"][&id]["pos"], serde_json::json!([320.,256.]));
            assert_eq!(next["nodes"]["out"]["pos"], serde_json::json!([320.,416.]));
            assert_eq!(next["nodes"]["out"]["inputs"]["geo"], id.as_str());
            let mut history = minihou_app::History::default();
            history.replace(&mut doc, crate::graph_edit::validated(next).unwrap());
            restore_positions(&ctx, path, doc.value());
            let cached = || ctx.data_mut(|d| d.get_persisted::<State>(state_id(path)).unwrap());
            assert_eq!(cached().moved["out"], pos2(320.,416.));
            assert!(history.undo(&mut doc).is_some());
            restore_positions(&ctx, path, doc.value());
            assert_eq!(cached().moved["out"], pos2(320.,256.));
            assert_eq!(cached().moved["a"], pos2(320.,96.));
            assert!(!cached().moved.contains_key(&id));
            assert!(!history.can_undo());
            assert!(history.redo(&mut doc).is_some());
            restore_positions(&ctx, path, doc.value());
            assert_eq!(cached().moved[&id], pos2(320.,256.));
            assert_eq!(cached().moved["out"], pos2(320.,416.));
        }
    }

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
    #[test] fn wire_direction_requires_opposite_ports_on_distinct_nodes() {
        let output = AddConnection::Output("a".into());
        let input = AddConnection::Input { node: "b".into(), port: "geo".into() };
        for (a, b) in [(&output, &input), (&input, &output)] {
            assert!(matches!(wire_connection(a, b), Some(UiAction::ConnectNodes { from, to, port: Some(port) }) if from == "a" && to == "b" && port == "geo"));
        }
        assert!(wire_connection(&output, &output).is_none());
        assert!(wire_connection(&input, &input).is_none());
        assert!(wire_connection(&AddConnection::Output("b".into()), &input).is_none());
    }

    #[test] fn noodle_placement_survives_tidied_canvas_without_moving_peers() {
        let ctx = egui::Context::default();
        let path = Path::new("tidied_noodle.json");
        let mut state = State { tidied: true, ..Default::default() };
        state.moved.insert("a".into(), pos2(96., 32.));
        ctx.data_mut(|d| d.insert_persisted(state_id(path), state));
        place_connected(&ctx, path, "new", [640., 320.]);
        let state = ctx.data_mut(|d| d.get_persisted::<State>(state_id(path)).unwrap());
        let doc = serde_json::json!({"nodes":{"a":{"type":"box"},"new":{"type":"transform","inputs":{"geo":"a"},"pos":[640,320]}}});
        let graph = minihou_core::Graph::from_value(&doc).unwrap();
        let positions: HashMap<_, _> = positions_for(&state, &graph, &doc).into_iter().collect();
        assert_eq!(positions["a"], pos2(96., 32.));
        assert_eq!(positions["new"], pos2(640., 320.));
    }

    #[test] fn headless_noodle_release_add_connect_and_cancel() {
        let f = Fixture::new();
        let doc = serde_json::json!({"nodes": {"a":{"type":"box"},"b":{"type":"transform"}}});
        let graph = minihou_core::Graph::from_value(&doc).unwrap();
        let mut view = f.view(); view.doc = &doc; view.graph = &graph;
        let canvas = Rect::from_min_size(Pos2::ZERO, vec2(1000., 700.));
        let button = |at, pressed| egui::Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
        let key = |key| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
        for reverse in [false, true] {
            for finish in [0, 1, 2, 3] { // choose, Escape, click away, direct connection
                let ctx = egui::Context::default();
                let mut state = State { physics: false, zoom: 0.75, pan: vec2(20., 30.), ..Default::default() };
                state.moved = [("a".into(), pos2(96., 96.)), ("b".into(), pos2(96., 256.))].into();
                let source = if reverse { AddConnection::Input { node: "b".into(), port: "geo".into() } } else { AddConnection::Output("a".into()) };
                let ports = canvas_ports(&view, &node_positions(&state, &view));
                let start = screen(canvas, &state, ports.iter().find(|(p, _)| p == &source).unwrap().1);
                let end = if finish == 3 {
                    let target = if reverse { AddConnection::Output("a".into()) } else { AddConnection::Input { node: "b".into(), port: "geo".into() } };
                    screen(canvas, &state, ports.iter().find(|(p, _)| p == &target).unwrap().1)
                } else { pos2(550., 350.) };
                let mut out = Actions::default();
                let frame = |events: Vec<egui::Event>, state: &mut State, out: &mut Actions| {
                    let input = egui::RawInput { events, screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280., 800.))), ..Default::default() };
                    let _ = ctx.run(input, |ctx| { egui::CentralPanel::default().show(ctx, |ui| {
                        canvas_ui(ui, canvas, &view, out, state, Id::new("noodle_test"));
                    }); });
                };
                frame(vec![egui::Event::PointerMoved(start)], &mut state, &mut out);
                frame(vec![button(start, true)], &mut state, &mut out);
                assert_eq!(state.wire.as_ref(), Some(&source));
                frame(vec![egui::Event::PointerMoved(end)], &mut state, &mut out);
                frame(vec![button(end, false)], &mut state, &mut out);
                assert!(state.wire.is_none());
                if finish == 3 {
                    assert!(state.menu.is_none());
                    assert!(matches!(out.0.as_slice(), [UiAction::ConnectNodes { from, to, port: Some(port) }] if from == "a" && to == "b" && port == "geo"));
                    continue;
                }
                assert!(out.0.is_empty());
                let (connection, at) = state.menu.as_ref().unwrap().connect_to.as_ref().unwrap();
                assert_eq!(connection, &source);
                assert_eq!(*at, pos2(608., 384.));
                match finish {
                    0 => {
                        frame(vec![egui::Event::Text("transform".into())], &mut state, &mut out);
                        frame(vec![key(egui::Key::Enter)], &mut state, &mut out);
                        assert!(matches!(out.0.as_slice(), [UiAction::AddConnectedNode { node_type, connection, pos }] if node_type == "transform" && connection == &source && *pos == [608., 384.]));
                    }
                    1 => frame(vec![key(egui::Key::Escape)], &mut state, &mut out),
                    _ => frame(vec![egui::Event::PointerMoved(pos2(20., 650.)), button(pos2(20., 650.), true)], &mut state, &mut out),
                }
                assert!(state.menu.is_none());
                assert!(state.wire.is_none());
                if finish != 0 { assert!(out.0.is_empty()); }
            }
        }
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
    fn tidy_keeps_locked_authored_manual_and_automatic_positions() {
        let ctx=egui::Context::default(); let path=Path::new("locked_tidy.json");
        let doc=serde_json::json!({"nodes":{
            "authored":{"type":"box","pos":[13.,17.]},
            "manual":{"type":"box"},"automatic":{"type":"box"},
            "free":{"type":"box","pos":[111.,333.]}
        }});
        let graph=minihou_core::Graph::from_value(&doc).unwrap();
        let mut state=State::default();
        state.moved.insert("manual".into(),pos2(-123.,345.));
        let before:HashMap<_,_>=positions_for(&state,&graph,&doc).into_iter().collect();
        let mut flags=HashMap::new();
        for id in ["authored","manual","automatic"] {flags.insert(id.into(),super::super::NodeFlags {locked:true,..Default::default()});}
        ctx.data_mut(|d| d.insert_persisted(state_id(path),state));
        for _ in 0..2 {
            tidy(&ctx,path,&doc,&flags);
            let state=ctx.data_mut(|d|d.get_persisted::<State>(state_id(path)).unwrap());
            let positions:HashMap<_,_>=positions_for(&state,&graph,&doc).into_iter().collect();
            for id in ["authored","manual","automatic"] {assert_eq!(positions[id],before[id],"locked {id} moved");}
            assert_ne!(positions["free"],before["free"]);
            let free=Rect::from_min_size(positions["free"],CARD);
            for id in ["authored","manual","automatic"] {assert!(!free.intersects(Rect::from_min_size(positions[id],CARD)));}
        }
        flags.clear(); tidy(&ctx,path,&doc,&flags);
        let state=ctx.data_mut(|d|d.get_persisted::<State>(state_id(path)).unwrap());
        let positions:HashMap<_,_>=positions_for(&state,&graph,&doc).into_iter().collect();
        assert_ne!(positions["authored"],before["authored"],"unlocking should allow tidy to move the node");
    }
    #[test]
    fn header_has_icon_toggles_without_list_or_zoom_text() {
        let f=Fixture::new(); let ctx=egui::Context::default();
        let mut state=State::default(); let mut actions=Actions::default();
        let r=Rect::from_min_size(Pos2::ZERO,vec2(800.,32.));
        let frame=|events, state:&mut State, actions:&mut Actions| ctx.run(egui::RawInput {events,screen_rect:Some(Rect::from_min_size(Pos2::ZERO,vec2(800.,400.))),..Default::default()},|ctx| {
            egui::CentralPanel::default().show(ctx,|ui|header_ui(ui,r,&f.view(),actions,state));
        });
        let output=frame(vec![],&mut state,&mut actions);
        for shape in output.shapes {
            if let egui::Shape::Text(text)=shape.shape {
                assert!(!["List","Snap","Links"].contains(&text.galley.text()));
                assert!(!text.galley.text().ends_with('%'));
            }
        }
        for (pos,links) in [(pos2(588.,16.),true),(pos2(620.,16.),false)] {
            for pressed in [true,false] {
                frame(vec![egui::Event::PointerMoved(pos),egui::Event::PointerButton {pos,button:PointerButton::Primary,pressed,modifiers:Default::default()}],&mut state,&mut actions);
            }
            if links {assert!(!state.show_links);assert!(actions.0.iter().any(|a|matches!(a,UiAction::LinkVisibility(false))));}
            else {assert!(!state.snap);}
        }
    }
    #[test]
    fn tidy_clears_manual_positions() {
        let mut state = State::default();
        state.moved.insert("box1".into(), pos2(123.0, 45.0));
        tidy_state(&mut state);
        assert!(state.moved.is_empty() && state.tidied && state.zoom < 0.0);
    }
    #[test]
    fn tidy_preserves_current_source_order_across_repeated_requests() {
        let f = Fixture::new();
        let doc = serde_json::json!({"nodes": {
            "grid":{"type":"grid","pos":[-500,20]},
            "noise":{"type":"noise","inputs":{"geo":"grid"}},
            "torus":{"type":"torus","pos":[500,20]},
            "transform":{"type":"transform","inputs":{"geo":"torus"}}
        }});
        let graph = minihou_core::Graph::from_value(&doc).unwrap();
        let mut view = f.view(); view.doc = &doc; view.graph = &graph;
        let mut state = State::default();
        state.moved.insert("grid".into(), pos2(600.,250.));
        state.moved.insert("torus".into(), pos2(-200.,450.));
        tidy_state(&mut state);
        // Header and dispatcher can both request Tidy before the next canvas frame.
        tidy_state(&mut state);
        let positions: HashMap<_, _> = node_positions(&state, &view).into_iter().collect();
        assert!(positions["torus"].x < positions["grid"].x);
        assert_eq!(positions["grid"].x, positions["noise"].x);
        assert_eq!(positions["torus"].x, positions["transform"].x);
        assert_eq!(positions["grid"].y, positions["torus"].y);
    }
    #[test]
    fn tidy_snaps_even_with_snap_disabled_and_overrides_authored_positions() {
        let f = Fixture::new();
        let mut doc = f.doc.value().clone();
        doc["nodes"][&f.graph.nodes[0].0]["pos"] = serde_json::json!([13.,17.]);
        let mut view = f.view();
        view.doc = &doc;
        let mut state = State::default();
        state.snap = false;
        assert_eq!(node_positions(&state, &view)[0].1, pos2(13.,17.));
        state.moved.insert(f.graph.nodes[0].0.clone(), pos2(45.,77.));
        tidy_state(&mut state);
        for (_, at) in node_positions(&state, &view) {
            assert_eq!(at, snap(at, 32.0, true, false));
        }
    }
    #[test]
    fn viewport_primitive_add_reserves_space_without_shifting_existing_cards() {
        let f = Fixture::new();
        let mut state = State::default();
        state.moved = node_positions(&state, &f.view()).into_iter().collect();
        let before = state.moved.clone();
        let (doc, _) = crate::graph_edit::add_at(f.doc.value(), "box", None, [0.; 3]).unwrap();
        let graph = minihou_core::Graph::from_value(&doc).unwrap();
        let mut view = f.view();
        view.doc = &doc;
        view.graph = &graph;
        let positions = node_positions(&state, &view);
        for (id, at) in &before { assert_eq!(positions.iter().find(|(node, _)| node == id).unwrap().1, *at); }
        for (id, at) in &positions {
            if before.contains_key(id) { continue; }
            for (peer, peer_at) in &positions {
                if peer != id { assert!(!Rect::from_min_size(*at, CARD).intersects(Rect::from_min_size(*peer_at, CARD))); }
            }
        }
    }
    #[test]
    fn wiggling_detaches_once_with_or_without_shift_and_leaves_card_at_drop() {
        for shift in [false, true] {
            let mut f = Fixture::new();
            f.selected = f.graph.nodes[0].0.clone();
            f.selected_set = std::collections::BTreeSet::from([f.selected.clone()]);
            let ctx = egui::Context::default();
            let mut state = State::default();
            state.snap = false;
            for (i, (id, _)) in f.graph.nodes.iter().enumerate() {
                state.moved.insert(id.clone(), pos2(96., 96. + i as f32 * 96.));
            }
            let initial = state.moved[&f.selected];
            let modifiers = egui::Modifiers { shift, ..Default::default() };
            let mut out = Actions::default();
            // Warm up, press, cross the drag threshold, then reverse rapidly three times.
            for (frame, x) in [206., 206., 256., 206., 256., 226., 226.].into_iter().enumerate() {
                let at = pos2(x, 124.);
                let mut events = vec![egui::Event::PointerMoved(at)];
                if frame == 1 || frame == 6 { events.push(egui::Event::PointerButton {
                    pos: at, button: PointerButton::Primary, pressed: frame == 1, modifiers,
                }); }
                let input = egui::RawInput { events, modifiers, time: Some(frame as f64 * 0.05),
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.,800.))), ..Default::default() };
                let _ = ctx.run(input, |ctx| { egui::CentralPanel::default().show(ctx, |ui| {
                    canvas_ui(ui, ui.max_rect(), &f.view(), &mut out, &mut state, Id::new("wiggle_regression"));
                }); });
            }
            assert_ne!(state.moved[&f.selected], initial, "gesture must actually drag a card");
            assert_eq!(out.0.iter().filter(|a| matches!(a, UiAction::DetachNode(id) if id == &f.selected)).count(), 1);
            assert!(state.drag.is_none());
            assert!(state.springs.is_none());
            assert!((state.moved[&f.selected].x - initial.x - 20.).abs() < 0.01);
        }
    }
    #[test]
    fn wiggle_ignores_jitter_slow_reversals_and_ordinary_dragging() {
        for samples in [vec![(0.,0.),(100.,0.1),(90.,0.2),(110.,0.3),(100.,0.4)],
            vec![(0.,0.),(50.,0.3),(0.,0.6),(50.,0.9),(0.,1.2)]] {
            let mut wiggle = Wiggle::default();
            for (x, time) in samples { assert!(!wiggle.update(x, time)); }
        }
        for sign in [-1., 1.] {
            let mut wiggle = Wiggle::default();
            for (frame, x) in [0.,50.,0.,50.,0.,50.].into_iter().enumerate() {
                assert_eq!(wiggle.update(x * sign, frame as f64 * 0.05), frame == 4);
            }
        }
    }
    #[test]
    fn physics_toggle_survives_persisted_state_and_tidy() {
        let ctx = egui::Context::default();
        let path = Path::new("physics_toggle.json");
        let mut state = State::default();
        assert!(state.physics, "connected drag physics is on by default");
        state.physics = false;
        ctx.data_mut(|d| d.insert_persisted(state_id(path), state));
        let f=Fixture::new();
        tidy(&ctx, path, f.doc.value(), &HashMap::new());
        assert!(!ctx.data_mut(|d| d.get_persisted::<State>(state_id(path)).unwrap().physics));
    }
    #[test]
    fn canvas_shift_drag_drives_springs_and_continues_after_release() {
        for (physics, shift) in [(true, true), (false, true), (true, false)] {
            let f = Fixture::new();
            let doc = serde_json::json!({"nodes": {
                "a":{"type":"grid"}, "b":{"type":"normal","inputs":{"geo":"a"}},
                "c":{"type":"output","inputs":{"geo":"b"}}, "alone":{"type":"torus"}
            }});
            let graph = minihou_core::Graph::from_value(&doc).unwrap();
            let selected = std::collections::BTreeSet::from(["a".to_owned()]);
            let mut view = f.view();
            view.doc = &doc; view.graph = &graph; view.selected = "a"; view.selected_set = &selected;
            let ctx = egui::Context::default();
            let mut state = State::default();
            state.snap = false; state.physics = physics;
            state.moved = [("a", pos2(96.,96.)), ("b", pos2(96.,224.)),
                ("c", pos2(96.,352.)), ("alone", pos2(800.,96.))]
                .into_iter().map(|(id, at)| (id.to_owned(), at)).collect();
            let modifiers = egui::Modifiers { shift, ..Default::default() };
            let mut out = Actions::default();
            let mut release_x = 0.0;
            for frame in 0..126 {
                let x = [206.,206.,270.,334.,334.,334.].get(frame).copied().unwrap_or(334.);
                let at = pos2(x,124.);
                let mut events = vec![egui::Event::PointerMoved(at)];
                if frame == 1 || frame == 5 { events.push(egui::Event::PointerButton {
                    pos: at, button: PointerButton::Primary, pressed: frame == 1, modifiers,
                }); }
                let input = egui::RawInput { events, modifiers, time: Some(frame as f64 / 60.),
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.,800.))), ..Default::default() };
                let _ = ctx.run(input, |ctx| { egui::CentralPanel::default().show(ctx, |ui| {
                    canvas_ui(ui, ui.max_rect(), &view, &mut out, &mut state, Id::new("spring_drag"));
                }); });
                if frame == 5 {
                    release_x = state.moved["b"].x;
                    assert!(state.drag.is_none());
                    assert_eq!(state.springs.is_some(), physics && shift);
                }
            }
            assert!(state.moved["a"].x > 96., "test must drag the anchor");
            if physics && shift {
                assert!(state.moved["b"].x > release_x + 1., "neighbours must keep following after release");
                assert!(state.moved["c"].x > 96.);
            } else {
                assert_eq!(state.moved["b"], pos2(96.,224.));
                assert_eq!(state.moved["c"], pos2(96.,352.));
            }
            assert_eq!(state.moved["alone"], pos2(800.,96.));
            assert!(!out.0.iter().any(|a| matches!(a, UiAction::Edit { .. } | UiAction::DisconnectAll(_) | UiAction::DetachNode(_)
                | UiAction::DisconnectInput { .. } | UiAction::ConnectNodes { .. })));
        }
    }
    #[test]
    fn exposure_badge_uses_document_interface() {
        let doc = serde_json::json!({"exposed":[{"node":"box1","param":"size","name":"Size"}]});
        assert!(exposed_to_blender(&doc, "box1"));
        assert!(!exposed_to_blender(&doc, "box2"));
        assert!(!exposed_to_blender(&serde_json::json!({}), "box1"));
    }
    #[test]
    fn open_add_menu_no_deadlock() {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let ctx = egui::Context::default();
            open_add_menu(&ctx, Path::new("scene.json"));
            tx.send(()).unwrap();
        });
        assert!(rx.recv_timeout(std::time::Duration::from_millis(500)).is_ok(), "open_add_menu deadlocked on nested ctx lock");
    }
    #[test]
    fn menu_can_open_and_close_fifty_times() {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let ctx = egui::Context::default();
            let path = Path::new("fifty_cycles.json");
            let id = state_id(path);
            for i in 0..50 {
                open_add_menu(&ctx, path);
                let is_open = ctx.data_mut(|d| d.get_persisted::<State>(id).and_then(|s| s.menu).is_some());
                assert!(is_open, "Failed to open menu on iteration {i}");
                open_add_menu(&ctx, path);
                let is_open = ctx.data_mut(|d| d.get_persisted::<State>(id).and_then(|s| s.menu).is_some());
                assert!(!is_open, "Failed to close menu on iteration {i}");
            }
            tx.send(()).unwrap();
        });
        assert!(rx.recv_timeout(std::time::Duration::from_secs(5)).is_ok(), "Menu 50 open/close cycles timed out!");
    }
    #[test]
    fn viewport_tab_in_edit_mode_opens_the_add_menu() {
        use crate::ui::keymap::{Command, KeyContext};
        // Tab is the node menu in edit mode too; the object / edit toggle is Ctrl+Tab (Command::ToggleEdit)
        let kctx_edit = KeyContext { viewport_hovered: true, network_hovered: false, in_edit_mode: true, selected: Some("box1") };
        assert!(matches!(Command::AddMenu.to_action(&kctx_edit), Some(UiAction::Command(cmd)) if cmd == "add_menu"));
        assert!(matches!(Command::ToggleEdit.to_action(&kctx_edit), Some(UiAction::Command(cmd)) if cmd == "toggle_edit"));

        let kctx_obj = KeyContext { viewport_hovered: true, network_hovered: false, in_edit_mode: false, selected: Some("box1") };
        assert!(matches!(Command::AddMenu.to_action(&kctx_obj), Some(UiAction::Command(cmd)) if cmd == "add_menu"));

        let kctx_net = KeyContext { viewport_hovered: false, network_hovered: true, in_edit_mode: true, selected: Some("box1") };
        assert!(matches!(Command::AddMenu.to_action(&kctx_net), Some(UiAction::Command(cmd)) if cmd == "add_menu"));
    }
    #[test]
    fn headless_tab_opens_menu_filters_adds_node_and_closes() {
        use crate::ui::keymap::{KeyContext, Keymap};
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let f = Fixture::new();
            let ctx = egui::Context::default();
            f.theme.apply(&ctx);
            let path = std::path::Path::new("untitled.json");
            let keymap = Keymap::blender();

            // Frame 1: Tab over network area opens menu without deadlocking
            let mut out = Actions::default();
            let p_network = pos2(200.0, 300.0);
            let input1 = egui::RawInput {
                events: vec![
                    egui::Event::PointerMoved(p_network),
                    egui::Event::Key { key: egui::Key::Tab, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() },
                ],
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1280., 800.))),
                ..Default::default()
            };
            let v = f.view();
            let _ = ctx.run(input1, |ctx| {
                let kctx = KeyContext { viewport_hovered: false, network_hovered: true, in_edit_mode: false, selected: Some(&f.selected) };
                for a in keymap.poll(ctx, &kctx) { out.push(a); }
                egui::SidePanel::left("network").default_width(400.).show(ctx, |ui| {
                    show(ui, &v, &mut out);
                });
            });
            for a in &out.0 {
                if let UiAction::Command(cmd) = a {
                    if cmd == "add_menu" { open_add_menu(&ctx, path); }
                }
            }
            let id = state_id(path);
            let menu_is_open = ctx.data_mut(|d| d.get_persisted::<State>(id).and_then(|s| s.menu).is_some());
            assert!(menu_is_open, "Tab should open menu");

            // Frame 2: Show menu so TextEdit requests focus
            let input2 = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1280., 800.))), ..Default::default() };
            let _ = ctx.run(input2, |ctx| {
                egui::SidePanel::left("network").default_width(400.).show(ctx, |ui| {
                    show(ui, &v, &mut Actions::default());
                });
            });

            // Frame 3: Type "transform" into the focused TextEdit to filter
            let input3 = egui::RawInput {
                events: vec![
                    egui::Event::Text("transform".into()),
                ],
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1280., 800.))),
                ..Default::default()
            };
            let _ = ctx.run(input3, |ctx| {
                egui::SidePanel::left("network").default_width(400.).show(ctx, |ui| {
                    show(ui, &v, &mut Actions::default());
                });
            });
            let query = ctx.data_mut(|d| d.get_persisted::<State>(id).and_then(|s| s.menu).map(|m| m.query().to_owned())).unwrap_or_default();
            assert_eq!(query, "transform");

            // Frame 4: Press Enter to add the filtered node
            let mut out4 = Actions::default();
            let input4 = egui::RawInput {
                events: vec![
                    egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() },
                ],
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1280., 800.))),
                ..Default::default()
            };
            let _ = ctx.run(input4, |ctx| {
                egui::SidePanel::left("network").default_width(400.).show(ctx, |ui| {
                    show(ui, &v, &mut out4);
                });
            });
            assert!(out4.0.iter().any(|a| matches!(a, UiAction::AddNode { node_type, .. } if node_type == "transform")));
            let menu_is_open = ctx.data_mut(|d| d.get_persisted::<State>(id).and_then(|s| s.menu).is_some());
            assert!(!menu_is_open, "Enter should close menu");

            // Frame 4: Open menu again, then close with Tab
            open_add_menu(&ctx, path);
            assert!(ctx.data_mut(|d| d.get_persisted::<State>(id).and_then(|s| s.menu).is_some()));
            let input4a = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1280., 800.))), ..Default::default() };
            let _ = ctx.run(input4a, |ctx| {
                egui::SidePanel::left("network").default_width(400.).show(ctx, |ui| {
                    show(ui, &v, &mut Actions::default());
                });
            });
            let mut out4b = Actions::default();
            let input4b = egui::RawInput {
                events: vec![
                    egui::Event::Key { key: egui::Key::Tab, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() },
                ],
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1280., 800.))),
                ..Default::default()
            };
            let _ = ctx.run(input4b, |ctx| {
                let kctx = KeyContext { viewport_hovered: false, network_hovered: true, in_edit_mode: false, selected: Some(&f.selected) };
                for a in keymap.poll(ctx, &kctx) { out4b.push(a); }
                egui::SidePanel::left("network").default_width(400.).show(ctx, |ui| {
                    show(ui, &v, &mut out4b);
                });
            });
            for a in &out4b.0 {
                if let UiAction::Command(cmd) = a {
                    if cmd == "add_menu" { open_add_menu(&ctx, path); }
                }
            }
            assert!(ctx.data_mut(|d| d.get_persisted::<State>(id).and_then(|s| s.menu).is_none()), "Tab should close open menu");

            // Frame 5: Open menu again, then close with Escape
            open_add_menu(&ctx, path);
            assert!(ctx.data_mut(|d| d.get_persisted::<State>(id).and_then(|s| s.menu).is_some()));
            let mut out5 = Actions::default();
            let input5 = egui::RawInput {
                events: vec![
                    egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() },
                ],
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1280., 800.))),
                ..Default::default()
            };
            let _ = ctx.run(input5, |ctx| {
                let kctx = KeyContext { viewport_hovered: false, network_hovered: true, in_edit_mode: false, selected: Some(&f.selected) };
                for a in keymap.poll(ctx, &kctx) { out5.push(a); }
                egui::SidePanel::left("network").default_width(400.).show(ctx, |ui| {
                    show(ui, &v, &mut out5);
                });
            });
            assert!(ctx.data_mut(|d| d.get_persisted::<State>(id).and_then(|s| s.menu).is_none()), "Escape should close menu");

            tx.send(()).unwrap();
        });
        assert!(rx.recv_timeout(std::time::Duration::from_secs(5)).is_ok(), "Test timed out!");
    }
}


#[cfg(test)]
mod noodle_selection_tests {
    use super::*;
    use crate::ui::testing::Fixture;
    fn fixture() -> (Fixture, State, WireRef) {
        let mut f=Fixture::new();
        f.doc=crate::graph_edit::validated(serde_json::json!({"nodes":{
            "a":{"type":"box", "pos":[100,100]},
            "out":{"type":"output", "inputs":{"geo":"a"}, "pos":[100,400]}
        },"display":"out"})).unwrap();
        f.graph=minihou_core::Graph::from_value(f.doc.value()).unwrap();
        f.selected="out".into(); f.selected_set=BTreeSet::from(["out".into()]);
        let state=State { physics:false, ..Default::default() };
        (f,state,WireRef { from:"a".into(),to:"out".into(),port:"geo".into() })
    }
    fn frame(ctx: &egui::Context, f: &Fixture, state: &mut State, at: Pos2,
        button: Option<(PointerButton,bool)>, modifiers: egui::Modifiers) -> Vec<UiAction> {
        let r=Rect::from_min_size(Pos2::ZERO,vec2(1000.,800.));
        let mut events=vec![egui::Event::PointerMoved(at)];
        if let Some((button,pressed))=button {
            events.push(egui::Event::PointerButton { pos:at,button,pressed,modifiers });
        }
        let input=egui::RawInput { screen_rect:Some(r),events,modifiers,..Default::default() };
        let mut out=Actions::default();
        let _=ctx.run(input,|ctx| { egui::CentralPanel::default().show(ctx,|ui| {
            canvas_ui(ui,r,&f.view(),&mut out,state,Id::new("noodle_canvas_test"));
        }); });
        out.0
    }
    #[test]
    fn noodle_pickup_canvas_disconnect_reconnect_and_rmb_cancel() {
        for mode in 0..4 {
            let (mut f, mut state, wire) = fixture();
            let mut doc = f.doc.value().clone();
            doc["nodes"]["t"] = serde_json::json!({"type":"transform","pos":[400,400]});
            f.doc = crate::graph_edit::validated(doc).unwrap();
            f.graph = minihou_core::Graph::from_value(f.doc.value()).unwrap();
            let ctx = egui::Context::default(); let input = pos2(210.,400.);
            frame(&ctx,&f,&mut state,input,None,egui::Modifiers::NONE);
            frame(&ctx,&f,&mut state,input,Some((PointerButton::Primary,true)),egui::Modifiers::NONE);
            assert_eq!(state.pickup.as_ref().unwrap().original,wire);
            assert_eq!(state.wire,Some(AddConnection::Output("a".into())));
            let end = if mode == 1 { pos2(510.,400.) } else { pos2(700.,300.) };
            frame(&ctx,&f,&mut state,end,None,egui::Modifiers::NONE);
            if mode == 2 {
                let actions = frame(&ctx,&f,&mut state,end,Some((PointerButton::Secondary,true)),egui::Modifiers::NONE);
                assert!(actions.is_empty()); assert!(state.menu.is_none()); assert!(state.pickup.is_none());
            }
            if mode == 3 {
                let input = egui::RawInput { events: vec![egui::Event::Key { key: egui::Key::Escape,
                    physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }], ..Default::default() };
                let mut out = Actions::default();
                let _ = ctx.run(input, |ctx| { egui::CentralPanel::default().show(ctx, |ui| {
                    canvas_ui(ui,Rect::from_min_size(Pos2::ZERO,vec2(1000.,800.)),&f.view(),&mut out,&mut state,Id::new("noodle_canvas_test"));
                }); });
                assert!(out.0.is_empty()); assert!(state.pickup.is_none());
            }
            let actions = frame(&ctx,&f,&mut state,end,Some((PointerButton::Primary,false)),egui::Modifiers::NONE);
            if mode >= 2 { assert!(actions.is_empty()); }
            else {
                assert!(matches!(actions.as_slice(),[UiAction::NetworkGesture { before, next, .. }]
                    if before == f.doc.value() && next["nodes"]["out"]["inputs"].get("geo").is_none()
                    && (mode == 0 || next["nodes"]["t"]["inputs"]["geo"] == "a")));
                assert!(state.menu.is_none());
            }
        }
    }
    #[test]
    fn noodle_drop_canvas_move_insert_and_shift_suppression() {
        for physics in [false,true] {
            let (mut f, mut state, _) = fixture(); state.snap = false;
            let mut doc = f.doc.value().clone();
            doc["nodes"]["t"] = serde_json::json!({"type":"transform","pos":[400,240]});
            f.doc = crate::graph_edit::validated(doc).unwrap();
            f.graph = minihou_core::Graph::from_value(f.doc.value()).unwrap();
            f.selected = "t".into(); f.selected_set = BTreeSet::from(["t".into()]);
            let ctx = egui::Context::default(); let start = pos2(510.,268.); let end = pos2(210.,268.);
            frame(&ctx,&f,&mut state,start,None,egui::Modifiers::NONE);
            frame(&ctx,&f,&mut state,start,Some((PointerButton::Primary,true)),egui::Modifiers::NONE);
            frame(&ctx,&f,&mut state,end,None,egui::Modifiers::NONE);
            assert!(state.drag.is_some());
            state.drag_physics = physics;
            let actions = frame(&ctx,&f,&mut state,end,Some((PointerButton::Primary,false)),egui::Modifiers::NONE);
            let edits: Vec<_> = actions.iter().filter(|a| matches!(a,UiAction::NetworkGesture { .. })).collect();
            if physics { assert!(edits.is_empty()); }
            else { assert!(matches!(edits.as_slice(),[UiAction::NetworkGesture { before,next,.. }]
                if before["nodes"]["t"]["pos"] == serde_json::json!([400.,240.])
                && next["nodes"]["out"]["inputs"]["geo"] == "t" && next["nodes"]["t"]["inputs"]["geo"] == "a")); }
        }
    }
    #[test]
    fn noodle_shelf_and_tab_payload_drop_dispatch_position() {
        let (f,mut state,_) = fixture(); let ctx = egui::Context::default();
        let at = pos2(100.,240.);
        frame(&ctx,&f,&mut state,at,None,egui::Modifiers::NONE);
        frame(&ctx,&f,&mut state,at,Some((PointerButton::Primary,true)),egui::Modifiers::NONE);
        state.menu = Some(network_menu::MenuState::new(pos2(700.,500.),None,true));
        egui::DragAndDrop::set_payload(&ctx,super::super::ToolDrag("transform".into()));
        frame(&ctx,&f,&mut state,at,None,egui::Modifiers::NONE);
        let actions = frame(&ctx,&f,&mut state,at,Some((PointerButton::Primary,false)),egui::Modifiers::NONE);
        assert!(matches!(actions.as_slice(),[UiAction::AddNetworkNode { node_type,pos, .. }]
            if node_type == "transform" && *pos == [0.,224.]), "actions: {actions:?}");
        assert!(state.menu.is_none());
    }
    #[test]
    fn noodle_shelf_drop_splices_even_when_collision_moves_card_past_wire() {
        let (f, mut state, expected_wire) = fixture();
        let ctx = egui::Context::default();
        let at = pos2(210., 360.);
        frame(&ctx, &f, &mut state, at, None, egui::Modifiers::NONE);
        frame(&ctx, &f, &mut state, at, Some((PointerButton::Primary, true)), egui::Modifiers::NONE);
        egui::DragAndDrop::set_payload(&ctx, super::super::ToolDrag("transform".into()));
        let actions = frame(&ctx, &f, &mut state, at, Some((PointerButton::Primary, false)), egui::Modifiers::NONE);
        let [UiAction::AddNetworkNode { pos, wire, .. }] = actions.as_slice() else { panic!("actions: {actions:?}"); };
        assert_eq!(wire.as_ref(), Some(&expected_wire));
        let (mut next, id) = crate::graph_edit::add(f.doc.value(), "transform", None).unwrap();
        next["nodes"][&id]["pos"] = serde_json::json!(pos);
        assert!(insert_added(&ctx, Path::new("drop.mhou"), f.doc.value(), &mut next, &id, *pos, wire.as_ref()));
        assert_eq!(next["nodes"]["out"]["inputs"]["geo"], id);
        assert_eq!(next["nodes"][&id]["inputs"]["geo"], "a");
    }
    #[test]
    fn noodle_insertion_picks_closest_centre_and_shelf_add_splices() {
        let (f,_,wire) = fixture();
        let (mut value,id) = crate::graph_edit::add(f.doc.value(),"transform",None).unwrap();
        let near = wire.clone(); let mut far = wire.clone(); far.to = "other".into();
        value["nodes"]["other"] = serde_json::json!({"type":"transform","inputs":{"geo":"a"}});
        let rect = Rect::from_min_max(pos2(100.,240.),pos2(320.,296.));
        let wires = vec![(far,vec![pos2(110.,200.),pos2(110.,350.)]),(near.clone(),vec![pos2(210.,200.),pos2(210.,350.)])];
        assert_eq!(insertion_candidate(&value,&id,rect,&wires),Some(near));
        let ctx = egui::Context::default();
        assert!(!insert_added(&ctx,Path::new("drop.mhou"),f.doc.value(),&mut value,&id,[100.,240.],None));
        assert_eq!(value["nodes"]["out"]["inputs"]["geo"],"a");
        assert_eq!(value["display"],id.as_str());
    }
    #[test]
    fn noodle_shift_rmb_cuts_on_release_and_plain_rmb_opens_menu() {
        let (f,mut state,wire)=fixture(); let ctx=egui::Context::default();
        let start=pos2(180.,278.); let end=pos2(240.,278.);
        frame(&ctx,&f,&mut state,start,None,egui::Modifiers::SHIFT);
        assert!(frame(&ctx,&f,&mut state,start,Some((PointerButton::Secondary,true)),egui::Modifiers::SHIFT).is_empty());
        assert!(state.menu.is_none());
        assert!(frame(&ctx,&f,&mut state,end,None,egui::Modifiers::SHIFT).is_empty());
        assert!(state.cut.as_ref().unwrap().hits.contains(&wire));
        let actions=frame(&ctx,&f,&mut state,end,Some((PointerButton::Secondary,false)),egui::Modifiers::SHIFT);
        assert!(matches!(actions.as_slice(),[UiAction::DisconnectWires(wires)] if wires==&BTreeSet::from([wire])));
        assert!(state.menu.is_none());
        frame(&ctx,&f,&mut state,start,Some((PointerButton::Secondary,true)),egui::Modifiers::NONE);
        assert!(state.menu.is_some());
    }
    #[test]
    fn noodle_shift_rmb_subthreshold_is_a_context_click() {
        let (f,mut state,_)=fixture(); let ctx=egui::Context::default(); let at=pos2(180.,278.);
        frame(&ctx,&f,&mut state,at,None,egui::Modifiers::SHIFT);
        frame(&ctx,&f,&mut state,at,Some((PointerButton::Secondary,true)),egui::Modifiers::SHIFT);
        let actions=frame(&ctx,&f,&mut state,at+vec2(2.,0.),Some((PointerButton::Secondary,false)),egui::Modifiers::SHIFT);
        assert!(actions.is_empty()); assert!(state.menu.is_some()); assert!(state.cut.is_none());
    }
    #[test]
    fn noodle_marquee_and_click_selection_modifiers() {
        for modifiers in [egui::Modifiers::NONE,egui::Modifiers::SHIFT,egui::Modifiers::CTRL] {
            let (f,mut state,wire)=fixture(); let ctx=egui::Context::default();
            let other=WireRef { from:"other".into(),..wire.clone() };
            state.selected_wires=BTreeSet::from([wire.clone(),other.clone()]);
            let start=pos2(180.,260.); let end=pos2(240.,300.);
            frame(&ctx,&f,&mut state,start,None,modifiers);
            frame(&ctx,&f,&mut state,start,Some((PointerButton::Primary,true)),modifiers);
            frame(&ctx,&f,&mut state,end,None,modifiers);
            let actions=frame(&ctx,&f,&mut state,end,Some((PointerButton::Primary,false)),modifiers);
            let expected=if modifiers.ctrl { BTreeSet::from([other]) }
                else if modifiers.shift { BTreeSet::from([wire.clone(),other]) }
                else { BTreeSet::from([wire.clone()]) };
            assert_eq!(state.selected_wires,expected);
            assert!(actions.iter().any(|action| matches!(action,UiAction::SelectSet { ids,extend:false }
                if if modifiers.shift || modifiers.ctrl { ids==&vec!["out".to_owned()] } else { ids.is_empty() })));
        }
        let (f,mut state,wire)=fixture(); let ctx=egui::Context::default(); let at=pos2(210.,278.);
        frame(&ctx,&f,&mut state,at,None,egui::Modifiers::NONE);
        frame(&ctx,&f,&mut state,at,Some((PointerButton::Primary,true)),egui::Modifiers::NONE);
        let actions=frame(&ctx,&f,&mut state,at,Some((PointerButton::Primary,false)),egui::Modifiers::NONE);
        assert_eq!(state.selected_wires,BTreeSet::from([wire]));
        assert!(matches!(actions.as_slice(),[UiAction::SelectSet { ids,extend:false }] if ids.is_empty()));
    }

    #[test]
    fn noodle_cut_cancels_on_escape_focus_loss_and_pointer_loss() {
        for cancellation in 0..3 {
            let (f,mut state,_)=fixture(); let ctx=egui::Context::default();
            let start=pos2(180.,278.); let end=pos2(240.,278.);
            frame(&ctx,&f,&mut state,start,None,egui::Modifiers::SHIFT);
            frame(&ctx,&f,&mut state,start,Some((PointerButton::Secondary,true)),egui::Modifiers::SHIFT);
            frame(&ctx,&f,&mut state,end,None,egui::Modifiers::SHIFT);
            assert!(state.cut.as_ref().unwrap().active);
            let event=if cancellation==0 { egui::Event::Key { key:egui::Key::Escape,
                physical_key:None,pressed:true,repeat:false,modifiers:egui::Modifiers::SHIFT } }
                else { egui::Event::PointerGone };
            let input=egui::RawInput { focused:cancellation!=1,events:vec![event],..Default::default() };
            let mut out=Actions::default();
            let _=ctx.run(input,|ctx| { egui::CentralPanel::default().show(ctx,|ui| {
                canvas_ui(ui,Rect::from_min_size(Pos2::ZERO,vec2(1000.,800.)),&f.view(),&mut out,&mut state,Id::new("noodle_canvas_test"));
            }); });
            assert!(state.cut.is_none()); assert!(out.0.is_empty());
            let actions=frame(&ctx,&f,&mut state,end,Some((PointerButton::Secondary,false)),egui::Modifiers::SHIFT);
            assert!(!actions.iter().any(|a| matches!(a,UiAction::DisconnectWires(_))));
        }
    }
    #[test]
    fn noodle_ctrl_marquee_subtracts_nodes_and_wires_together() {
        let (f,mut state,wire)=fixture(); let ctx=egui::Context::default();
        state.selected_wires.insert(wire);
        let start=pos2(80.,260.); let end=pos2(340.,500.); let mods=egui::Modifiers::CTRL;
        frame(&ctx,&f,&mut state,start,None,mods);
        frame(&ctx,&f,&mut state,start,Some((PointerButton::Primary,true)),mods);
        frame(&ctx,&f,&mut state,end,None,mods);
        let actions=frame(&ctx,&f,&mut state,end,Some((PointerButton::Primary,false)),mods);
        assert!(state.selected_wires.is_empty());
        assert!(actions.iter().any(|a| matches!(a,UiAction::SelectSet { ids,extend:false } if ids.is_empty())));
    }
    #[test]
    fn noodle_selection_is_ui_only_and_clears_on_graph_change() {
        let ctx=egui::Context::default();
        let path=Path::new("wire-selection-test.mhou");
        let doc=serde_json::json!({"nodes":{"a":{"type":"box"}}});
        let mut state=State::default();
        sync_wire_selection(&mut state,&doc);
        let wire=WireRef { from:"a".into(),to:"out".into(),port:"geo".into() };
        state.selected_wires.insert(wire.clone());
        state.cut=Some(Cut::new(Pos2::ZERO));
        sync_wire_selection(&mut state,&doc);
        assert!(state.selected_wires.contains(&wire));
        ctx.data_mut(|d| d.insert_persisted(state_id(path),state.clone()));
        assert_eq!(take_selected_wires(&ctx,path,&doc),BTreeSet::from([wire.clone()]));
        assert!(take_selected_wires(&ctx,path,&doc).is_empty());
        let changed=serde_json::json!({"nodes":{"b":{"type":"box"}}});
        sync_wire_selection(&mut state,&changed);
        assert!(state.selected_wires.is_empty());
        assert!(state.cut.is_none());
        ctx.data_mut(|d| d.insert_persisted(state_id(path),state));
        assert!(take_selected_wires(&ctx,path,&doc).is_empty());
    }
}
