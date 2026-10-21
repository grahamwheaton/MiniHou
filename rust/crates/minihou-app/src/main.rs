mod picking;
mod subnet_edit;
mod element_select;
mod element_pick_glue;
use element_pick_glue::{point_pickable, active_range, navigation_point, loop_cut_has_marks, loop_cut_selection, selection_exists_on_scene, selection_in_range, pick_displayed, pick_displayed_culled, keep_extrude_selection, pick_displayed_wire, wire_selection};
mod node_selection;
mod node_add;
mod selection_ui;
use selection_ui::{selected_point_ids, select_active_nodes, select_nodes};
mod action_dispatch;
mod app_frame;
mod history_selection;
use history_selection::{restore, selection_state};
mod coplanar_tool;
mod group_preview;
mod param_pick;
use param_pick::{field_selection, infer_mode, ParamPick, PickedField, param_pick_end, input_source, source_ready, pick_source_ready};
mod selection_metrics;
#[cfg(test)] mod face_mode_profile;
use minihou_app::graph_edit;
mod noodle_hit;
mod http_ui;
mod ui;
mod viewport;
mod viewport_input;
mod transform_math;
mod repeat;
mod usd_export;
mod window_setup;
mod transform_ops;
mod mirror_ops;
mod model_modal_ops;
mod modal_ops;
mod document_ops;
mod app_session;
mod blender_ops;
mod app_helpers;
use app_helpers::*;
pub use repeat::LastOperator;
#[cfg(test)] mod transform_tests;
#[cfg(test)] mod live_path_tests;
use eframe::{egui, egui_wgpu};
pub use minihou_app::units;
use minihou_app::{Camera, Document, Result, Edit, EditorDocument, CookWorker, CookRequest, CookState, StandardView, SelectionState, SelectionSnapshot, HistoryOutcome};
use minihou_core::Graph;
use serde_json::{json, Value};
use std::{collections::HashMap, path::PathBuf, time::{Duration, Instant}};
use ui::keymap::{KeyContext, Keymap, Preferences, ThemePref};
use ui::{theme::Theme, Actions, BlenderStatus, CameraInfo, EditorState, GeoSummary, NodeFlags, TransformOp, UiAction, View};
use ui::modal_transform::{Modal, Kind as ModalKind, Constraint};
use ui::modal_model::{State as ModelState, Kind as ModelKind, Outcome as ModelOutcome};
use viewport::{Gpu, Mode, Overlay, Scene, Target};
use minihou_core::python_bridge::PythonGate;

struct App {
    editor: EditorDocument, worker: CookWorker<Scene>, cooking: CookState,
    selected: String, selected_set: std::collections::BTreeSet<String>, display: String,
    subnet_scope: Option<(Vec<String>, String)>, subnet_pending_selection: Option<Vec<String>>,
    state: egui_wgpu::RenderState, gpu: Option<Gpu>, target: Option<Target>,
    materials:minihou_app::materials::Registry,
    camera: Camera, orbit_drag: Option<([f32;3], f32, f32, f32)>, bounds: ([f32;3], f32), mode: Mode,
    file_status: String,
    http: http_ui::HttpUi,
    http_time: (f64, f64),
    theme: Theme, flags: HashMap<String, NodeFlags>, themed: bool,
    editor_state: EditorState,
    geo: Option<GeoSummary>, blender: BlenderStatus,
    link: Option<ui::blender_ui::blender_link::LinkServer>,
    blender_job: Option<std::thread::JoinHandle<std::io::Result<(String, bool)>>>,
    prefs: Preferences, keymap: Keymap, prefs_open: bool, viewport_hover: bool, network_hover: bool,
    /// The last cooked scene at full detail, and the proxy level it is currently uploaded at.
    display_ranges: Vec<(String, std::ops::Range<u32>)>,
    /// The selected preview follows these ranges; output-node resolution uses only this prefix.
    output_range_count: usize,
    scene: Option<Scene>, selected_preview: bool, shown_level: f32, geo_rev: u64,
    lod_uploaded: bool, last_lod_view: Option<([[f32;4];4], [u32;2], bool)>, last_lod_select: Instant,
    blender_paths: Vec<PathBuf>, blender_bar: bool,
    selection: picking::Selection, selection_dirty: bool,
    param_pick: Option<ParamPick>, barrier_key: Option<(String,ui::SelectionMode,String,u64)>, target_key: Option<(String,String,u64)>, picked_field: Option<PickedField>,
    selection_metrics: selection_metrics::Cache,
    coplanar: Option<coplanar_tool::State>, group_preview:group_preview::State,
    /// Polygon/edge adjacency of `scene` for loops, rings, paths and mode conversion (built on first use).
    elem_topo: std::cell::OnceCell<element_select::Topology>,
    /// The element selection hidden while in Object mode; converted back when an element mode is entered.
    stashed_selection: picking::Selection,
    clipboard: Option<Value>,
    last_viewport: egui::Rect,
    transform_before: Option<Document>, transform_target: Option<String>,
    transform_delta: Option<LastOperator>,
    transform_pivot: Option<[f32; 3]>,
    transform_orientation: Option<ui::Orientation>,
    modal: Option<Modal>,
    pending_duplicate_move: Option<String>,
    duplicate_move_peers: Vec<String>,
    /// graph_embed_v1: a pushed Blender graph waiting for the user's answer (modal).
    embed_conflict: Option<Value>,
    model_modal: Option<(ModelState, Document, String, String)>,
    mirror: Option<ui::mirror_tool::Session>,
    pushpull_pick: bool,
    pub last_operator: Option<LastOperator>,
    last_edit_selection: ui::SelectionMode, params_open: bool, object_hidden: bool,
    held_keys: Vec<egui::Event>,
    /// Element selection (and the Object-mode stash) per cooked topology, see element_select::SelectionMemory.
    selection_memory: element_select::SelectionMemory<(picking::Selection, picking::Selection)>,
    pending_loop_selection: Option<String>,
    pending_restore_selection: Option<(picking::Selection, picking::Selection, Option<picking::Selection>)>,
    model_before_selection: Option<(picking::Selection, picking::Selection)>,
    sel_baseline: (picking::Selection, picking::Selection),
    extrude_keep: Option<picking::Selection>,
}
/// Endless drag (as Blender): when the pointer reaches the viewport edge it is warped to the opposite side, and the jump
/// that warp causes is dropped from `delta`, so the orbit/pan/zoom keeps going.
pub(crate) fn wrap_drag(ctx: &egui::Context, rect: egui::Rect, delta: &mut egui::Vec2) {
    if delta.x.abs() > rect.width() * 0.5 || delta.y.abs() > rect.height() * 0.5 { *delta = egui::Vec2::ZERO; }
    let Some(pos) = ctx.input(|i| i.pointer.latest_pos()) else { return; };
    let inner = rect.shrink(2.0);
    if inner.contains(pos) { return; }
    let m = 6.0;
    let wrap = |v: f32, lo: f32, hi: f32| if v < lo + 2.0 { hi - m } else if v > hi - 2.0 { lo + m } else { v };
    let to = egui::pos2(wrap(pos.x, rect.left(), rect.right()), wrap(pos.y, rect.top(), rect.bottom()));
    ctx.send_viewport_cmd(egui::ViewportCommand::CursorPosition(to));
}

fn standard_view(v: ui::ViewPreset) -> StandardView {
    use ui::ViewPreset as P;
    match v { P::Perspective => StandardView::Perspective, P::Front => StandardView::Front, P::Back => StandardView::Back,
        P::Right => StandardView::Right, P::Left => StandardView::Left, P::Top => StandardView::Top, P::Bottom => StandardView::Bottom }
}
impl App {
    /// The fraction of triangles to show: the menu's level, or one picked from the triangle count when Auto is on.
    fn proxy_level(&self, triangles: usize) -> f32 {
        let p = self.editor_state.proxy;
        if p.auto { ui::proxy_ui::proxy::auto_level(triangles, self.prefs.viewport.auto_budget as usize) } else { p.level }
    }
    /// A shelf tool: a new node of `node_type` wired to `after` (its first input) and made the displayed node; one undo step.
    /// Begin a plane sketch; the committed outline is a `curve` node added through `graph_edit::add_sketch`.
    fn start_sketch(&mut self, ctx: &egui::Context, mode: ui::sketch::SketchMode) {
        if minihou_core::nodes::get("curve").is_some() {
            self.finish_for_selection(ctx);
            self.editor_state.apply(ui::EditorChange::Tool(mode.tool()));
            ui::sketch::start(ctx, mode);
        } else { self.file_status = "Sketches need the Rust core curve node".into(); }
    }
    fn set_display_mode(&mut self,mode:Mode) {
        self.mode=mode;
        self.prefs.viewport.default_shading=mode.label().into();
        if let Err(e)=self.prefs.save() { self.file_status=format!("Could not save preferences: {e}"); }
    }
}
fn main() {
    if let Err(e)=window_setup::run() {
        eprintln!("{e}");
        // A double-clicked build has no terminal: say why it did not start.
        // (MINIHOU_NO_DIALOG: the tests start the app with bad input and must not wait for someone to click OK.)
        if std::env::var_os("MINIHOU_NO_DIALOG").is_none() {
            rfd::MessageDialog::new().set_level(rfd::MessageLevel::Error).set_title("MiniHou").set_description(e.to_string()).show();
        }
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tools_tests {
    use super::*;

    #[test]
    fn alt_and_ctrl_selection_gestures_never_create_nodes() {
        // Alt (loop), Ctrl+Alt (ring), Ctrl (path) clicks and Ctrl+Numpad +/- (select more / less) change the viewport
        // selection only. The grow / select_* nodes exist only when chosen from the Tab menu (docs/RUST_SELECT_NODES.md).
        let doc = json!({"version":1,"display":"output1","nodes":{
            "grid1":{"type":"grid"},"output1":{"type":"output","inputs":{"geo":"grid1"}}}});
        let before = doc.clone();
        // a 3 x 3 grid of quads split by diagonals, as the preview layout gives it
        let w = 4u32;
        let p: Vec<f32> = (0..4).flat_map(|j| (0..4).flat_map(move |i| [i as f32, j as f32, 0.])).collect();
        let (mut tris, mut edges) = (Vec::new(), Vec::new());
        for j in 0..3 { for i in 0..3 {
            let (a, b, c, d) = (j * w + i, j * w + i + 1, (j + 1) * w + i + 1, (j + 1) * w + i);
            tris.extend([a, b, c, a, c, d]); edges.extend([a, b, b, c, c, d, d, a]);
        } }
        let topo = element_select::Topology::new(&p, &tris, &edges);
        let tri = 2 * (3 + 1); // quad (1, 1)
        for (shift, ctrl, alt) in [(false, false, true), (false, true, true), (true, false, true), (false, true, false)] {
            let kind = element_select::classify(shift, ctrl, alt, true).unwrap();
            for mode in [ui::SelectionMode::Points, ui::SelectionMode::Edges, ui::SelectionMode::Faces] {
                let mut selection = picking::Selection { mode, ..Default::default() };
                let hit = picking::Hit { tri: tri as usize, t: 1., bary: [0.4, 0.3, 0.3] };
                let point = [1.5, 1.05, 0.];
                topo.apply_click(&p, kind, Some((topo.pick(&p, &tris, hit, point, mode), tri, point)), mode, &mut selection);
                let picked = selection.clone();
                topo.apply_click(&p, kind, Some((topo.pick(&p, &tris, hit, [2.5, 2.05, 0.], mode), tri + 6, [2.5, 2.05, 0.])), mode, &mut selection);
                assert!(!selection.is_empty() && !picked.is_empty(), "{kind:?} {mode:?}");
                topo.grow_selection(&mut selection, false);
                topo.grow_selection(&mut selection, true);
            }
        }
        assert_eq!(doc, before);
        // the keymap routes those keys to selection commands, never to node creation
        for (key, cmd) in [(egui::Key::Plus, "select_more"), (egui::Key::Minus, "select_less")] {
            let modifiers = egui::Modifiers { ctrl: true, alt: true, ..Default::default() };
            let ctx = egui::Context::default();
            let mut actions = Vec::new();
            let _ = ctx.run(egui::RawInput { modifiers, events: vec![egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }], ..Default::default() }, |ctx| {
                actions = Keymap::blender().poll(ctx, &KeyContext { viewport_hovered: true, in_edit_mode: true, ..Default::default() });
            });
            assert!(matches!(actions.as_slice(), [UiAction::Command(name)] if name == cmd), "{actions:?}");
        }
    }

    #[test]
    fn selection_node_params_merge_into_the_added_node() {
        let mut value = json!({"nodes":{"grow1":{"type":"grow","inputs":{"geo":"box1"}}}});
        merge_params(&mut value, "grow1", json!({"group":"1 2","domain":"points"}));
        assert_eq!(value["nodes"]["grow1"]["params"], json!({"group":"1 2","domain":"points"}));
        merge_params(&mut value, "grow1", json!({"steps":2}));
        assert_eq!(value["nodes"]["grow1"]["params"]["domain"], "points");
        assert_eq!(value["nodes"]["grow1"]["params"]["steps"], 2);
        merge_params(&mut value, "missing", json!({"steps":2}));
        // the menu insertion wires the node after the source and before the output, like other viewport-added nodes
        let doc = json!({"version":1,"display":"output1","nodes":{"box1":{"type":"box"},"output1":{"type":"output","inputs":{"geo":"box1"}}}});
        let (value, id) = graph_edit::add_at(&doc, "grow", Some("box1"), [0.; 3]).unwrap();
        assert_eq!(value["nodes"][&id]["inputs"]["geo"], "box1");
        assert_eq!(value["nodes"]["output1"]["inputs"]["geo"], id.as_str());
    }

    #[test]
    fn viewport_move_inserts_transform_before_output_not_after() {
        let mut value = json!({"version":1,"display":"output1","nodes":{
            "box1":{"type":"box"},
            "output1":{"type":"output","inputs":{"geo":"box1"}}}});
        let id = ensure_transform_target(&mut value, "output1", true, None).unwrap();
        assert_eq!(value["nodes"][&id]["inputs"]["geo"], "box1");
        assert_eq!(value["nodes"]["output1"]["inputs"]["geo"], id.as_str());
        assert_eq!(value["display"], "output1");
    }

    #[test]
    fn sixty_transform_updates_use_param_edits_and_one_undo_step() {
        let mut doc = Document::parse(include_str!("default_graph.json").to_owned()).unwrap();
        let before = doc.clone();
        let source = doc.value()["display"].as_str().unwrap().to_owned();
        let mut value = doc.value().clone();
        let target = ensure_transform_target(&mut value, &source, true, None).unwrap();
        value["nodes"][&target]["params"] = json!({"translate":[0.,0.,0.]});
        let mut edits = apply_transform_step(&mut doc, true, value, &target, "translate", json!([0.,0.,0.])).unwrap();
        for i in 1..60 {
            let mut value = doc.value().clone();
            assert_eq!(ensure_transform_target(&mut value, &source, true, Some(&target)), Some(target.clone()));
            let param = json!([i as f32 / 10., 0., 0.]);
            edits.extend(apply_transform_step(&mut doc, false, value, &target, "translate", param).unwrap());
        }
        assert_eq!(doc.value()["nodes"].as_object().unwrap().len(), before.value()["nodes"].as_object().unwrap().len() + 1);
        assert_eq!(edits.iter().filter(|e| matches!(e, Edit::Replace(_))).count(), 1);
        assert_eq!(edits.iter().filter(|e| matches!(e, Edit::Param { .. })).count(), 60);
        let mut history = minihou_app::History::default();
        let final_doc = doc.clone();
        doc = before.clone();
        history.replace(&mut doc, final_doc);
        assert!(history.undo(&mut doc).is_some());
        assert_eq!(doc.value(), before.value());
        assert!(history.undo(&mut doc).is_none());
        let mut reused = before.value().clone();
        let id = ensure_transform_target(&mut reused, &source, true, None).unwrap();
        assert_eq!(ensure_transform_target(&mut reused, &source, true, None), Some(id.clone()));
        assert_eq!(ensure_transform_target(&mut reused, &id, true, None).as_deref(), Some(id.as_str()));
        assert_eq!(reused["nodes"].as_object().unwrap().len(), before.value()["nodes"].as_object().unwrap().len() + 1);
    }

    #[test]
    fn viewport_accepts_tool_drag_even_with_overlay_widgets() {
        use egui::{Event, PointerButton, RawInput};
        let ctx = egui::Context::default();
        let rect = egui::Rect::from_min_size(egui::pos2(400., 100.), egui::vec2(300., 300.));
        let at = egui::pos2(500., 200.);
        let _ = ctx.run(RawInput { events: vec![Event::PointerMoved(at)], ..Default::default() }, |ctx| {
            egui::DragAndDrop::set_payload(ctx, ui::ToolDrag("box".into()));
            assert!(viewport_tool_drop(ctx, rect).1);
        });
        let _ = ctx.run(RawInput { events: vec![Event::PointerButton { pos: at, button: PointerButton::Primary,
            pressed: false, modifiers: Default::default() }], ..Default::default() }, |ctx| {
            let (tool, hover, drop_pos) = viewport_tool_drop(ctx, rect);
            assert_eq!(tool.unwrap().0, "box");
            assert!(!hover);
            assert_eq!(drop_pos, Some(at));
        });
    }

    #[test]
    fn scaling_uses_the_drag_start_values() {
        assert_eq!(scaled_from_initial(&json!([2., 3., 4.]), [1., 2., 1.]), [2., 6., 4.]);
        assert_eq!(scaled_from_initial(&serde_json::Value::Null, [1., 0.5, 1.]), [1., 0.5, 1.]);
    }
    #[test]
    fn tab_restores_the_last_element_mode() {
        let mut last = ui::SelectionMode::Points;
        assert_eq!(toggle_edit_mode(ui::SelectionMode::Object, &mut last), ui::SelectionMode::Points);
        assert_eq!(toggle_edit_mode(ui::SelectionMode::Faces, &mut last), ui::SelectionMode::Object);
        assert_eq!(toggle_edit_mode(ui::SelectionMode::Object, &mut last), ui::SelectionMode::Faces);
    }
    /// Real routing: raw Ctrl+Tab -> raw_input_hook filter -> egui frame (with a focusable text
    /// field, as the params panel has) -> keymap poll -> toggle. Before the fix the first press
    /// moved egui focus into the text field and every later press was swallowed.
    #[test]
    fn ctrl_tab_toggles_object_and_element_modes_through_the_keymap() {
        use ui::SelectionMode as M;
        let ctx = egui::Context::default();
        let keymap = Keymap::blender();
        let (mut mode, mut last) = (M::Object, M::Points);
        let mut text = String::from("1.0");
        let mut press = |key: egui::Key, modifiers: egui::Modifiers, mode: &mut M, last: &mut M| {
            let mut raw = egui::RawInput { modifiers, events: vec![egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }], ..Default::default() };
            let held = hold_back_ctrl_tab(&mut raw, ctx.memory(|m| m.focused().is_some()));
            let mut actions = Vec::new();
            let _ = ctx.run(raw, |ctx| {
                ctx.input_mut(|i| i.events.extend(held.clone()));
                actions = keymap.poll(ctx, &KeyContext { viewport_hovered: true, in_edit_mode: *mode != M::Object, ..Default::default() });
                egui::CentralPanel::default().show(ctx, |ui| { ui.text_edit_singleline(&mut text); let _ = ui.button("mode"); });
            });
            for action in actions {
                match action {
                    UiAction::Command(name) if name == "toggle_edit" => *mode = toggle_edit_mode(*mode, last),
                    UiAction::Editor(ui::EditorChange::Selection(m)) => { remember_element_mode(m, last); *mode = m; }
                    other => panic!("unexpected {other:?}"),
                }
            }
            assert!(ctx.memory(|m| m.focused().is_none()), "Ctrl+Tab must not move egui focus");
        };
        let ctrl = egui::Modifiers::CTRL;
        let mut seq = Vec::new();
        for _ in 0..4 { press(egui::Key::Tab, ctrl, &mut mode, &mut last); seq.push(mode); }
        assert_eq!(seq, [M::Points, M::Object, M::Points, M::Object]);
        // 3 (faces) in object mode enters faces and becomes the remembered element mode
        press(egui::Key::Num3, egui::Modifiers::NONE, &mut mode, &mut last); assert_eq!(mode, M::Faces);
        press(egui::Key::Tab, ctrl, &mut mode, &mut last); assert_eq!(mode, M::Object);
        press(egui::Key::Tab, ctrl, &mut mode, &mut last); assert_eq!(mode, M::Faces);
        press(egui::Key::Num2, egui::Modifiers::NONE, &mut mode, &mut last); assert_eq!(mode, M::Edges);
        press(egui::Key::Tab, ctrl, &mut mode, &mut last); assert_eq!(mode, M::Object);
        press(egui::Key::Tab, ctrl, &mut mode, &mut last); assert_eq!(mode, M::Edges);
        // a mode button (UiAction::Editor) also updates the remembered mode; Object button keeps it
        remember_element_mode(M::Points, &mut last);
        remember_element_mode(M::Object, &mut last); mode = M::Object;
        press(egui::Key::Tab, ctrl, &mut mode, &mut last); assert_eq!(mode, M::Points);
    }
    #[test]
    fn ctrl_tab_is_left_to_a_focused_text_field() {
        let mods = egui::Modifiers::CTRL;
        let mut raw = egui::RawInput { events: vec![egui::Event::Key { key: egui::Key::Tab, physical_key: None, pressed: true, repeat: false, modifiers: mods }], ..Default::default() };
        assert!(hold_back_ctrl_tab(&mut raw, true).is_empty());
        assert_eq!(raw.events.len(), 1);
        let plain = egui::Event::Key { key: egui::Key::Tab, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        raw.events = vec![plain.clone(), raw.events[0].clone()];
        assert_eq!(hold_back_ctrl_tab(&mut raw, false).len(), 1);
        assert_eq!(raw.events, vec![plain]);
    }
    #[test]
    fn viewport_and_network_selection_keep_the_last_added_object_active() {
        let mut active = String::new();
        let mut set = std::collections::BTreeSet::new();
        select_active_nodes(&mut active, &mut set, vec!["grid".into()], false, false);
        select_active_nodes(&mut active, &mut set, vec!["torus".into()], true, false);
        assert_eq!(active, "torus");
        assert_eq!(set.len(), 2);
        select_active_nodes(&mut active, &mut set, vec!["grid".into()], true, false);
        assert_eq!(active, "grid");
        assert_eq!(set.len(), 2);
        select_active_nodes(&mut active, &mut set, vec!["grid".into()], true, true);
        assert_eq!(active, "torus");
        select_active_nodes(&mut active, &mut set, vec!["grid".into()], false, false);
        assert_eq!(set, std::collections::BTreeSet::from(["grid".into()]));
        select_active_nodes(&mut active, &mut set, vec![], false, false);
        assert!(active.is_empty() && set.is_empty());
    }
    #[test]
    fn modal_tools_route_two_and_three_displayed_objects_to_last_selected_stream() {
        for count in [2,3] {
            let mut nodes=serde_json::Map::new();
            let mut outputs=Vec::new();
            for i in 0..count {
                let source=format!("box{i}");let output=format!("out{i}");
                nodes.insert(source.clone(),json!({"type":"box"}));
                nodes.insert(output.clone(),json!({"type":"output","inputs":{"geo":source}}));
                outputs.push(output);
            }
            let doc=json!({"version":1,"nodes":nodes,"display":outputs});
            let mut active=String::new();let mut selected=std::collections::BTreeSet::new();
            select_active_nodes(&mut active,&mut selected,vec!["out0".into()],false,false);
            select_active_nodes(&mut active,&mut selected,vec!["out1".into()],true,false);
            assert_eq!(active,"out1");
            for tool in ["polysplit","edge_slide","knife","polyextrude","inset","polybevel","pushpull","mirror","grow","select_path","select_loop","select_ring"] {
                let (next,id)=graph_edit::add(&doc,tool,Some(&active)).unwrap_or_else(||panic!("{tool} should insert on active output"));
                assert_eq!(next["nodes"]["out1"]["inputs"]["geo"],id);
                for other in [&"out0",&"out2"] {
                    if doc["nodes"].get(*other).is_some() {assert_eq!(next["nodes"][*other],doc["nodes"][*other],"{tool} changed {other}");}
                }
            }
            select_active_nodes(&mut active,&mut selected,vec!["out0".into()],true,false);
            assert_eq!(active,"out0");
        }
    }
    #[test]
    fn modal_picking_and_elements_ignore_other_displayed_objects() {
        for count in [2,3] {
            let mut points=Vec::new();let mut vertices=Vec::new();let mut offsets=vec![0];
            for i in 0..count {
                let z=i as f32;
                points.extend([0.,0.,z,1.,0.,z,0.,1.,z]);
                vertices.extend([i*3,i*3+1,i*3+2]);offsets.push(vertices.len() as i32);
            }
            let geo=minihou_core::Geometry::from_arrays(points,vertices,offsets,None).unwrap();
            let scene=Scene::new(&geo);
            let ranges=(0..count).map(|i|(format!("out{i}"),(i*3) as u32..(i*3+3) as u32)).collect::<Vec<_>>();
            let selected=std::collections::BTreeSet::from(["out0".into(),format!("out{}",count-1)]);
            let active=std::collections::BTreeSet::from(["out0".into()]);
            let hit=pick_displayed(&scene,[0.2,0.2,10.],[0.,0.,-1.],viewport::ObjectVisibility::HideOthers,&ranges,&active).unwrap();
            assert_eq!(picked_object(&ranges,scene.triangle_indices(),hit),Some("out0"));
            let all=picking::Selection{mode:ui::SelectionMode::Points,points:(0..count*3).map(|i|i as u32).collect(),edges:vec![(0,1),(3,4)],tris:(0..count).map(|i|i as u32).collect()};
            let only=selection_in_range(&all,&scene,&ranges[0].1);
            assert_eq!(only.points,vec![0,1,2]);assert_eq!(only.edges,vec![(0,1)]);assert_eq!(only.tris,vec![0]);
            assert_eq!(selected.len(),2);
        }
    }
    #[test]
    fn viewport_picks_the_nearest_display_output() {
        let points = [0.,0.,0., 1.,0.,0., 0.,1.,0., 0.,0.,2., 1.,0.,2., 0.,1.,2.];
        let triangles = [0,1,2, 3,4,5];
        let ranges = vec![("grid".into(), 0..3), ("torus".into(), 3..6)];
        let front = picking::pick_triangle(&points, &triangles, [0.2,0.2,5.], [0.,0.,-1.]).unwrap();
        assert_eq!(picked_object(&ranges, &triangles, front), Some("torus"));
        let back = picking::pick_triangle(&points, &triangles, [0.2,0.2,-5.], [0.,0.,1.]).unwrap();
        assert_eq!(picked_object(&ranges, &triangles, back), Some("grid"));
        assert!(picking::pick_triangle(&points, &triangles, [3.,3.,5.], [0.,0.,-1.]).is_none());
    }
    #[test]
    fn selected_hidden_node_adds_pickable_preview_only_while_selected() {
        let make = |z: f32| {
            let geo = minihou_core::Geometry::from_arrays(
                vec![0.,0.,z, 1.,0.,z, 0.,1.,z], vec![0,1,2], vec![0,3], None).unwrap();
            geo
        };
        let displayed = make(0.);
        let hidden = make(2.);
        let mut scene = Scene::new(&displayed);
        let mut ranges = vec![("output".to_owned(), 0..3)];
        assert!(include_selected_preview(&mut scene, &mut ranges, "garden_path", Some(&hidden)));
        assert_eq!(ranges, vec![("output".into(), 0..3), ("garden_path".into(), 3..6)]);
        let hit = pick_displayed(&scene, [0.2,0.2,5.], [0.,0.,-1.], viewport::ObjectVisibility::HideOthers,
            &ranges, &std::collections::BTreeSet::from(["garden_path".to_owned()])).unwrap();
        assert_eq!(picked_object(&ranges, scene.triangle_indices(), hit), Some("garden_path"));
        assert!(!include_selected_preview(&mut scene, &mut ranges, "", Some(&hidden)));
        assert!(!include_selected_preview(&mut scene, &mut ranges, "output", Some(&displayed)));
        let mut deselected_scene = Scene::new(&displayed);
        let mut deselected_ranges = vec![("output".to_owned(), 0..3)];
        assert!(!include_selected_preview(&mut deselected_scene, &mut deselected_ranges, "", Some(&hidden)));
        assert_eq!(deselected_ranges, vec![("output".into(), 0..3)]);
        assert_eq!(deselected_scene.positions().len(), 9);

        let curve = minihou_core::Geometry::from_arrays(
            vec![0.,0.,2., 1.,0.,2., 1.,1.,2.], vec![0,1,2], vec![0,3], Some(vec![false])).unwrap();
        let mut curve_scene = Scene::new(&displayed);
        let mut curve_ranges = vec![("output".to_owned(), 0..3)];
        assert!(include_selected_preview(&mut curve_scene, &mut curve_ranges, "garden_path", Some(&curve)));
        assert_eq!(curve_ranges[1].1, 3..6);
        assert!(curve_scene.line_primitive_indices().chunks_exact(2)
            .any(|edge| edge.iter().all(|id| curve_ranges[1].1.contains(id))));
    }
    #[test]
    fn active_origin_follows_its_transform_chain_instead_of_scene_bounds() {
        let doc = json!({"nodes": {
            "grid": {"type":"grid", "params":{"center":[1.,0.,0.]}},
            "moved": {"type":"transform", "inputs":{"geo":"grid"}, "params":{"translate":[4.,2.,0.]}},
            "turned": {"type":"transform", "inputs":{"geo":"moved"}, "params":{"rotate":[0.,0.,90.]}},
            "torus": {"type":"torus", "params":{"center":[100.,0.,0.]}}
        }, "display":["turned","torus"]});
        let origin = object_origin(&doc, "turned");
        assert!((origin[0] + 2.).abs() < 1e-5 && (origin[1] - 5.).abs() < 1e-5);
        assert_eq!(object_origin(&doc, "torus"), [100.,0.,0.]);
    }
    #[test]
    fn element_groups_use_point_ids_in_each_selection_mode() {
        let selection = picking::Selection { mode: ui::SelectionMode::Faces,
            points: vec![99], edges: vec![(8, 7)], tris: vec![1, 0] };
        let triangles = [2, 1, 0, 2, 3, 4];
        assert_eq!(selected_point_ids(&selection, ui::SelectionMode::Points, &triangles), [99]);
        assert_eq!(selected_point_ids(&selection, ui::SelectionMode::Edges, &triangles), [7, 8]);
        assert_eq!(selected_point_ids(&selection, ui::SelectionMode::Faces, &triangles), [0, 1, 2, 3, 4]);
        assert_eq!(group_text(&selected_point_ids(&selection, ui::SelectionMode::Faces, &triangles)), "0,1,2,3,4");
        assert!(selected_point_ids(&selection, ui::SelectionMode::Object, &triangles).is_empty());
    }
    #[test]
    fn proportional_settings_use_core_parameter_names() {
        let mut state = EditorState::default();
        state.proportional.enabled = true;
        state.proportional.radius = 2.5;
        state.proportional.falloff = ui::Falloff::InverseSquare;
        state.proportional.connected_only = true;
        let mut params = json!({});
        proportional_params(&mut params, &state);
        assert_eq!(params, json!({"proportional_enabled": true, "proportional_radius": 2.5,
            "proportional_falloff": "inverse_square", "proportional_connected": true}));
        state.proportional.falloff = ui::Falloff::Random;
        proportional_params(&mut params, &state);
        assert_eq!(params["proportional_falloff"], "random");
    }
    #[test]
    fn network_select_all_and_deselect_all_manage_node_set_without_affecting_viewport_mesh_selection() {
        let doc = json!({
            "nodes": {
                "box1": {"type": "box"},
                "transform1": {"type": "transform", "inputs": {"geo": "box1"}},
                "out": {"type": "null", "inputs": {"geo": "transform1"}}
            }
        });
        let all_nodes: std::collections::BTreeSet<String> = doc["nodes"].as_object().unwrap().keys().cloned().collect();
        assert_eq!(all_nodes.len(), 3);

        let mut selected_set = std::collections::BTreeSet::from(["box1".to_string()]);
        let mut selected = "box1".to_string();
        assert_eq!(selected_set.len(), 1);
        assert_eq!(selected, "box1");
        let mut mesh_selection = picking::Selection {
            mode: ui::SelectionMode::Points,
            points: vec![0, 1, 2],
            edges: vec![],
            tris: vec![],
        };

        // Select all nodes in network (A)
        selected_set = all_nodes.clone();
        selected = selected_set.iter().next().cloned().unwrap_or_default();
        assert_eq!(selected_set.len(), 3);
        assert!(!selected.is_empty());
        assert_eq!(mesh_selection.points, vec![0, 1, 2]);

        // Deselect all nodes in network (Alt+A)
        selected_set.clear();
        selected.clear();
        assert!(selected_set.is_empty());
        assert!(selected.is_empty());
        assert_eq!(mesh_selection.points, vec![0, 1, 2]);

        // Deselect in viewport (viewport Alt+A) clears mesh selection
        mesh_selection.clear();
        assert!(mesh_selection.is_empty());
    }
}
