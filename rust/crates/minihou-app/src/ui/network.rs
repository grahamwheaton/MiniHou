//! Network canvas, following Main.dc.html and NodeStates.dc.html.
#[path = "network_layout.rs"]
mod network_layout;
#[path = "network_physics.rs"]
mod network_physics;
#[path = "network_wires.rs"]
mod network_wires;
#[path = "network_menu.rs"]
mod network_menu;

mod context;
use context::*;
mod canvas;
use canvas::*;
mod menus;
use menus::*;
pub use menus::{open_add_menu, open_pie_menu};
mod nodes;
use nodes::*;
mod wires;
use wires::*;
mod position;
use position::*;
pub use position::{insert_added, layout_changed, position_add, restore_positions};

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
    subnet_path: Vec<String>, inner_selected: String, inner_selected_set: BTreeSet<String>,
    tidy_order: HashMap<String, Pos2>,
    selected_wires: BTreeSet<WireRef>, graph_snapshot: Option<serde_json::Value>,
    cut: Option<Cut>, box_op: SelectionOp,
    wiggle: Wiggle,
    wire: Option<AddConnection>, wire_output_index: usize, pickup: Option<crate::graph_edit::WirePickup>,
    drag_before: Option<serde_json::Value>, drag_physics: bool,
    drag: Option<(String, Pos2)>, box_start: Option<Pos2>, alt_pan: bool, menu: Option<network_menu::MenuState>,
    canvas: Option<Rect>, add_pos: Option<Pos2> }
impl Default for State {
    fn default() -> Self { Self { show_links: true, focus: None, zoom: 1.0, pan: Vec2::ZERO, moved: HashMap::new(), snap: true, physics: true, springs: None,
        subnet_path: Vec::new(), inner_selected: String::new(), inner_selected_set: BTreeSet::new(),
        selected_wires: BTreeSet::new(), graph_snapshot: None, cut: None, box_op: SelectionOp::Replace,
        tidied: false, tidy_order: HashMap::new(), wiggle: Wiggle::default(), wire: None, wire_output_index: 0, pickup: None, drag_before: None, drag_physics: false, drag: None, box_start: None, alt_pan: false, menu: None,
        canvas: None, add_pos: None } }
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
    state.subnet_path = valid_path(v.doc, &state.subnet_path);
    if !state.subnet_path.is_empty() && ui.ctx().input(|i| i.key_pressed(egui::Key::Escape)) {
        state.subnet_path.pop(); reset_scope(&mut state);
        ui.ctx().input_mut(|i| { i.consume_key(egui::Modifiers::NONE,egui::Key::Escape); });
    }
    let scoped_doc = scope_doc(v.doc, &state.subnet_path);
    let scoped_graph = scoped_doc.as_ref().and_then(|doc| minihou_core::Graph::from_value(doc).ok());
    let inner_selected = state.inner_selected.clone();
    let inner_selected_set = state.inner_selected_set.clone();
    let mut scoped = v.clone();
    if let (Some(doc),Some(graph)) = (&scoped_doc,&scoped_graph) {
        scoped.doc = doc; scoped.graph = graph;
        scoped.selected = &inner_selected; scoped.selected_set = &inner_selected_set;
    }
    sync_wire_selection(&mut state, scoped.doc);
    let (header, _) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::hover());
    header_ui(ui, header, &scoped, out, &mut state);
    let (body, _) = ui.allocate_exact_size(ui.available_size(), Sense::hover());
    let (crumb, canvas) = split_breadcrumb(body);
    breadcrumb_ui(ui, crumb, canvas, &scoped, out, &mut state);
    let first = out.0.len();
    canvas_ui(ui, canvas, &scoped, out, &mut state, id);
    if !state.subnet_path.is_empty() {
        let mut local = out.0.split_off(first);
        for action in local.drain(..) {
            match action {
                UiAction::Select(name) => { state.inner_selected=name.clone(); state.inner_selected_set=BTreeSet::from([name]); },
                UiAction::ToggleSelect(name) => { if !state.inner_selected_set.insert(name.clone()) {state.inner_selected_set.remove(&name);} state.inner_selected=name; },
                UiAction::SelectSet { ids, extend } => {if !extend {state.inner_selected_set.clear();} state.inner_selected_set.extend(ids); state.inner_selected=state.inner_selected_set.iter().next_back().cloned().unwrap_or_default();},
                other => if let Some(edit)=scoped_edit(v.doc,&state.subnet_path,other) {out.push(edit);},
            }
        }
    }
    ui.ctx().data_mut(|d| d.insert_persisted(id, state));
    ui.spacing_mut().item_spacing = old_spacing;
}
fn valid_path(doc: &serde_json::Value, path: &[String]) -> Vec<String> {
    let mut node = doc;
    let mut valid = Vec::new();
    for id in path {
        let next = &node["nodes"][id];
        if next["type"] != "subnet" || !next["inner"]["nodes"].is_object() { break; }
        valid.push(id.clone()); node=&next["inner"];
    }
    valid
}
fn scope_doc(doc: &serde_json::Value, path: &[String]) -> Option<serde_json::Value> {
    if path.is_empty() {return None;}
    let mut scope=doc;
    for id in path {scope=&scope["nodes"][id]["inner"];}
    let mut value=scope.clone();
    if value["display"].is_null() {value["display"]=scope["output"].clone();}
    Some(value)
}
pub fn current_scope(ctx: &egui::Context, file: &Path, doc: &serde_json::Value) -> Option<(Vec<String>,serde_json::Value,String,BTreeSet<String>)> {
    let state=ctx.data_mut(|d|d.get_persisted::<State>(state_id(file)).unwrap_or_default());
    let path=valid_path(doc,&state.subnet_path);
    let scoped=scope_doc(doc,&path)?;
    Some((path,scoped,state.inner_selected,state.inner_selected_set))
}
pub fn select_inner(ctx:&egui::Context,file:&Path,ids:Vec<String>) {
    ctx.data_mut(|d| {
        let key=state_id(file);
        let mut state=d.get_persisted::<State>(key).unwrap_or_default();
        state.inner_selected=ids.first().cloned().unwrap_or_default();
        state.inner_selected_set=ids.into_iter().collect();
        d.insert_persisted(key,state);
    });
}
pub fn embed_scope(doc: &serde_json::Value, path: &[String], inner: serde_json::Value) -> Option<serde_json::Value> {
    let mut next=doc.clone();
    let mut scope=&mut next;
    for id in path {scope=&mut scope["nodes"][id]["inner"];}
    if !scope.is_object() {return None;}
    scope["nodes"]=inner["nodes"].clone();
    if let Some(output)=inner.get("display").or_else(||inner.get("output")) {scope["output"]=output.clone();}
    Some(next)
}
pub fn scoped_edit(doc: &serde_json::Value, path: &[String], action: UiAction) -> Option<UiAction> {
    let mut inner=scope_doc(doc,path)?;
    match action {
        UiAction::Edit {edit,continuous:_} => match edit {
            Edit::Param {node,name,value} => inner["nodes"][&node]["params"][&name]=value,
            Edit::Channel {node,path,expression} => {
                if let Some(expression)=expression {inner["nodes"][&node]["channels"][&path]=serde_json::json!(expression);}
                else {inner["nodes"][&node]["channels"].as_object_mut()?.remove(&path);}
            }
            Edit::Bypass {node,flag} => inner["nodes"][&node]["flags"]["bypass"]=serde_json::json!(flag),
            Edit::Replace(next) => inner=next,
            _ => return None,
        },
        UiAction::NetworkGesture {next,..} => inner=next,
        UiAction::DeleteNode(node) => inner=crate::graph_edit::delete(&inner,&node)?,
        UiAction::AddNetworkNode {node_type,pos,wire} => {
            let (mut next,id)=crate::graph_edit::add(&inner,&node_type,None)?;
            if node_type == "subnet" { crate::subnet_edit::initialize(&mut next,&id); }
            next["nodes"][&id]["pos"]=serde_json::json!(pos);
            if let Some(wire)=wire {next=crate::graph_edit::insert_existing(&next,&id,&wire)?;}
            inner=next;
        }
        UiAction::AddConnectedNode {node_type,connection,pos} => {
            let (mut next,id)=crate::graph_edit::add_connected(&inner,&node_type,&connection,pos)?;
            if node_type == "subnet" { crate::subnet_edit::initialize(&mut next,&id); }
            inner=next;
        },
        UiAction::AddNode {node_type,after} => {
            let (mut next,id)=crate::graph_edit::add(&inner,&node_type,after.as_deref())?;
            if node_type == "subnet" { crate::subnet_edit::initialize(&mut next,&id); }
            inner=next;
        },
        UiAction::ConnectNodes {from,to,port} => inner=crate::graph_edit::connect(&inner,&from,&to,port.as_deref())?,
        UiAction::DisconnectAll(node) => inner=crate::graph_edit::disconnect_all(&inner,&node)?,
        UiAction::DisconnectInput {node,port} => inner=crate::graph_edit::disconnect(&inner,&node,&port)?,
        UiAction::DisconnectWires(wires) => inner=crate::graph_edit::disconnect_wires(&inner,&wires)?,
        UiAction::RenameNode {old,new} => inner=crate::graph_edit::rename(&inner,&old,&new)?,
        UiAction::SetDisplay(node) => inner["output"]=serde_json::json!(node),
        _ => return None,
    }
    Some(UiAction::Edit {edit:Edit::Replace(embed_scope(doc,path,inner)?),continuous:false})
}
fn reset_scope(state: &mut State) {
    state.inner_selected.clear(); state.inner_selected_set.clear(); state.selected_wires.clear();
    state.moved.clear(); state.graph_snapshot=None; state.zoom=-1.0; state.pan=Vec2::ZERO;
}
#[cfg(test)]
mod subnet_scope_tests;
#[cfg(test)]
mod tests;


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
