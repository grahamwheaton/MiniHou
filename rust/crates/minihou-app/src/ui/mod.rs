//! The editor's panels. Every panel is a module with `show(ui, &View, &mut Actions)`:
//! it reads the immutable `View` and pushes `UiAction`s; `App::update` applies them. A panel never touches the engine, the
//! document or the GPU directly, which is what lets each one be built and tested alone (see `ui::testing`).
#![allow(dead_code)] // the skeleton: each panel is filled in one at a time
pub mod blender_ui;
pub mod editor_state;
pub mod geo_info;
pub mod keymap;
pub mod network;
pub mod params;
pub mod number_step;
pub mod prim_handles;
pub mod deform_gizmo;
pub mod curve_tool;
pub mod proxy_ui;
pub mod selection_region;
pub mod shelf;
pub mod status;
pub mod sketch;
pub mod mesh_cut;
pub mod theme;
pub mod topbar;
pub mod transform_gizmo;
pub mod modal_transform;
pub mod modal_model;
pub mod mirror_tool;
pub mod transform_overlay;
pub mod viewport_ui;
pub mod waiting;
pub mod object_search;

#[allow(unused_imports)]
pub use editor_state::{EditorChange, EditorState, Falloff, Orientation, Overlays, Pivot, Proxy, SelectionMode, Snap, SnapTo, SnapTargets, Tool, ViewPreset, Proportional};
use crate::viewport::Mode;
use minihou_app::{CookState, Edit};
use minihou_core::Graph;
use serde_json::Value;
use std::path::Path;
use std::collections::BTreeSet;

/// What a panel asks the app to do.
#[derive(Clone, Debug)]
pub enum UiAction {
    /// Attribute Create Domain dropdown: set the node's domain and convert its selection group to it.
    SetAttributeDomain { node: String, domain: String },
    PickParameter { node: String, name: String, mode: SelectionMode, pairs: bool },
    /// Output group target button clicked: select that group's elements on the node's own output.
    SelectOutputGroup { node: String, name: String },
    StartSketch(sketch::SketchMode),
    TransformNode { op: TransformOp },
    StartModal(modal_transform::Kind),
    StartScaleCage,
    FinishTransform,
    CancelTransform,
    MeshCutCommit { source: String, node_type: String, params: serde_json::Value },
    /// `refs`: per clicked point, optional inference reference descriptor (static snap metadata).
    SketchCommit { name: String, points: Vec<[f32; 3]>, closed: bool, refs: Vec<Option<Value>> },
    SketchBatch { strokes: Vec<sketch::SketchStroke>, selected: String, target: crate::graph_edit::SketchTarget },
    AddMeasurement { a: [f32; 3], b: [f32; 3], refs: [Option<serde_json::Value>; 2] },
    DeleteMeasurement(usize),
    ClearMeasurements,
    RegionSelect { shape: crate::picking::RegionShape, op: crate::picking::SelectOp },
    RestoreRegionSelection(selection_region::Snapshot),
    Open,
    Save,
    SaveAs,
    Undo,
    Redo,
    /// Frame the mesh in the viewport.
    Frame,
    FrameSelected,
    FrameNetwork,
    Select(String),
    NavigateLink { node: String, param: String },
    LinkVisibility(bool),
    OpenRenameNode(String),
    RenameNode { old: String, new: String },
    ToggleSelect(String),
    SelectSet { ids: Vec<String>, extend: bool },
    /// "Visible" flag: make this node the displayed one.
    SetDisplay(String),
    ToggleDisplay(String),
    /// A document edit (parameter, mute/bypass, display, whole-graph replace). `continuous` is true while a drag is in progress.
    Edit { edit: Edit, continuous: bool },
    /// A subnet boundary parameter-panel command; resolved against the current subnet scope.
    AddSubnetOutput(String),
    /// A timeline cook without changing the document or undo history.
    Time(Edit),
    /// A shelf or menu tool: add a node of this type (after `after`, wired to it, when given).
    /// `tab_menu` preserves the one Tab-menu-only multi-object Boolean shortcut.
    AddNode { node_type: String, after: Option<String>, tab_menu: bool },
    /// A shelf drop in network coordinates, independent of viewport/world placement.
    AddNetworkNode { node_type: String, pos: [f32; 2], wire: Option<crate::graph_edit::WireRef> },
    AddConnectedNode { node_type: String, connection: crate::graph_edit::AddConnection, pos: [f32; 2] },
    /// A shelf tool dropped with a specific world position.
    AddNodeAt { node_type: String, after: Option<String>, pos: [f32; 3] },
    SetMode(Mode),
    /// Selection mode, orientation, pivot, snapping, proportional editing, overlays, X-ray, view preset.
    Editor(EditorChange),
    /// UI-only node flags that are not in the file yet.
    ToggleLocked(String),
    ToggleTemplate(String),
    // ---- camera (the app owns the camera; panels and the keymap only ask)
    /// Orbit by degrees (yaw right, pitch up).
    Orbit { yaw: f32, pitch: f32 },
    /// Pan in screen units of the viewport.
    Pan { dx: f32, dy: f32 },
    /// Zoom by a factor (> 1 closer).
    Zoom(f32),
    /// Jump to a standard view (also set through `Editor(EditorChange::View)`; this one moves the camera).
    ViewPreset(ViewPreset),
    // ---- network editor
    DeleteNode(String),
    DuplicateNode(String),
    DisconnectAll(String),
    DetachNode(String),
    DissolveNode(String),
    /// Wire `from`'s output to `to`'s input `port`; `None` port = its first input.
    ConnectNodes { from: String, to: String, port: Option<String> },
    /// Atomic canvas gesture; expected guards against stale previews.
    NetworkGesture { expected: serde_json::Value, before: serde_json::Value, next: serde_json::Value },
    DisconnectInput { node: String, port: String },
    DisconnectWires(std::collections::BTreeSet<crate::graph_edit::WireRef>),
    /// Re-run the automatic layout for the whole network (card positions are UI state).
    TidyNetwork,
    /// Repeat the last confirmed operator with the same settings (Shift+R).
    RepeatLast,
    // ---- application
    OpenPreferences,
    /// Blender link bar: install the add-on into the chosen installation (index into `BlenderStatus::installs`).
    BlenderInstall(usize),
    BlenderOpen,
    BlenderDownloadAddon,
    BlenderDetect,
    BlenderRestartLink,
    /// Show or hide the Blender bar.
    BlenderBar(bool),
    /// A named command from the keymap that has no dedicated action yet (kept for forward compatibility).
    Command(String),
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TransformOp { Move([f32; 3]), Rotate([f32; 3]), Scale([f32; 3]) }

#[derive(Default)]
pub struct Actions(pub Vec<UiAction>);
impl Actions {
    pub fn push(&mut self, action: UiAction) { self.0.push(action); }
}

/// Drag payload of a shelf tool: dropped on the viewport or the network it adds that node type.
#[derive(Clone, Debug)]
pub struct ToolDrag(pub String);

/// UI-only per-node state (not part of the graph file).
#[derive(Clone, Default, Debug)]
pub struct NodeFlags { pub locked: bool, pub template: bool }

/// One attribute of a geometry, for the Geometry info panel (Houdini-style: name, type, tuple size, value range).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AttrInfo { pub name: String, pub kind: String, pub components: usize, pub min: Option<f64>, pub max: Option<f64> }
/// Counts, bounds and attributes of the geometry the app has for the selected node (the displayed node's cook today).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GeoSummary {
    pub node: String,
    pub points: usize, pub vertices: usize, pub prims: usize,
    pub bounds: Option<([f32; 3], [f32; 3])>,
    pub point_attrs: Vec<AttrInfo>, pub vertex_attrs: Vec<AttrInfo>, pub prim_attrs: Vec<AttrInfo>, pub detail_attrs: Vec<AttrInfo>,
}
/// The Blender live link as the top bar's indicator and bar show it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BlenderStatus {
    pub clients: Vec<String>,
    /// Blender installations the add-on can be installed into (name, path).
    pub installs: Vec<(String, String)>,
    pub addon_installed: bool,
    pub message: String,
    pub link_message: String,
    pub busy: bool,
}
impl BlenderStatus { pub fn connected(&self) -> bool { self.clients.iter().any(|c| c.to_lowercase().starts_with("blender")) } }

/// What the view gizmo needs of the camera.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CameraInfo { pub yaw: f32, pub pitch: f32, pub ortho: bool,
    pub projection: [[f32; 4]; 4], pub ray_origin: [f32; 3], pub ray_forward: [f32; 3], pub ray_right: [f32; 3], pub ray_up: [f32; 3] }

#[derive(Clone)]
pub struct View<'a> {
    pub graph: &'a Graph,
    /// The document JSON (parameter values, node positions if the file has them).
    pub doc: &'a Value,
    pub path: &'a Path,
    pub dirty: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    pub selected: &'a str,
    pub selected_set: &'a BTreeSet<String>,
    pub display: &'a str,
    pub cooking: &'a CookState,
    pub file_status: &'a str,
    pub mode: Mode,
    pub theme: &'a theme::Theme,
    pub editor: &'a EditorState,
    pub camera: CameraInfo,
    pub bounds: ([f32; 3], f32),
    pub selection_center: Option<[f32; 3]>,
    pub normal_frame: [[f32; 3]; 3],
    pub geo: Option<&'a GeoSummary>,
    pub blender: &'a BlenderStatus,
    /// Flags per node id (Locked, Template); ids not present have none set.
    pub flags: &'a std::collections::HashMap<String, NodeFlags>,
    pub units: crate::units::UnitSettings,
    /// Description of the repeatable last operator, if any.
    pub repeat_label: Option<&'a str>,
}
impl View<'_> {
    pub fn flags_of(&self, id: &str) -> NodeFlags { self.flags.get(id).cloned().unwrap_or_default() }
    /// True when the Rust core has a kernel for this node type (the green "Rs" chip).
    pub fn runs_on_rust(&self, node_type: &str) -> bool { minihou_core::nodes::get(node_type).is_some() }
}

/// Headless harness: run one panel for a frame against a document and collect what it asked for.
#[cfg(test)]
pub mod testing {
    use super::*;
    use eframe::egui;
    use minihou_app::Document;
    use std::collections::{BTreeSet, HashMap};

    pub struct Fixture { pub doc: Document, pub graph: Graph, pub cooking: CookState, pub theme: theme::Theme,
        pub flags: HashMap<String, NodeFlags>, pub selected: String, pub selected_set: BTreeSet<String>, pub display: String, pub mode: Mode, pub editor: EditorState, pub camera: CameraInfo, pub bounds: ([f32; 3], f32), pub geo: Option<GeoSummary>, pub blender: BlenderStatus, pub units: crate::units::UnitSettings }
    impl Fixture {
        pub fn new() -> Self {
            let doc = Document::parse(include_str!("../test_data/demo_graph.json").to_owned()).unwrap();
            let graph = Graph::from_value(doc.value()).unwrap();
            let display = graph.main_output().unwrap();
            Self { doc, graph, cooking: CookState::default(), theme: theme::Theme::dark(), flags: HashMap::new(),
                selected: display.clone(), selected_set: BTreeSet::from([display.clone()]), display, mode: Mode::Shaded, editor: EditorState::default(), camera: CameraInfo::default(), bounds: ([0.; 3], 1.), geo: None, blender: BlenderStatus::default(), units: crate::units::UnitSettings::default() }
        }
        pub fn view(&self) -> View<'_> {
            View { graph: &self.graph, doc: self.doc.value(), path: Path::new("untitled.json"), dirty: false, can_undo: false, can_redo: false,
                selected: &self.selected, selected_set: &self.selected_set, display: &self.display, cooking: &self.cooking, file_status: "", mode: self.mode,
                theme: &self.theme, editor: &self.editor, camera: self.camera, bounds: self.bounds, selection_center: None,
                normal_frame: [[1.,0.,0.],[0.,1.,0.],[0.,0.,1.]], geo: self.geo.as_ref(), blender: &self.blender, flags: &self.flags, units: self.units, repeat_label: None }
        }
        /// One frame of `panel` in a 1280 x 800 window; returns the actions it pushed.
        pub fn run(&self, panel: impl Fn(&mut egui::Ui, &View, &mut Actions)) -> Vec<UiAction> {
            let ctx = egui::Context::default();
            self.theme.apply(&ctx);
            let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::pos2(0., 0.), egui::vec2(1280., 800.))), ..Default::default() };
            let mut out = Actions::default();
            let v = self.view();
            let _ = ctx.run(input, |ctx| { egui::CentralPanel::default().show(ctx, |ui| panel(ui, &v, &mut out)); });
            out.0
        }
    }

    #[test]
    fn every_baseline_panel_runs_a_frame_without_asking_for_anything() {
        let f = Fixture::new();
        assert!(f.run(super::topbar::show).is_empty());
        assert!(f.run(super::shelf::show).is_empty());
        assert!(f.run(super::network::show).is_empty());
        assert!(f.run(super::params::show).is_empty());
        assert!(f.run(super::status::show).is_empty());
    }

    #[test]
    fn every_panel_renders_in_both_themes() {
        for theme in [theme::Theme::dark(), theme::Theme::light()] {
            let mut f = Fixture::new();
            f.theme = theme;
            for panel in [super::topbar::show as fn(&mut egui::Ui, &View, &mut Actions),
                super::shelf::show, super::network::show, super::params::show,
                super::status::show, super::geo_info::show, super::blender_ui::indicator] {
                assert!(f.run(panel).is_empty());
            }
            assert!(f.run(|ui, v, out| super::proxy_ui::menu(ui, v, out, false)).is_empty());
            assert!(f.run(|ui, v, out| super::viewport_ui::overlay(ui, ui.max_rect(), v, out)).is_empty());
            assert!(f.run(|ui, v, out| { let mut shown = true; super::blender_ui::bar(ui, v, out, &mut shown); }).is_empty());
            let ctx = egui::Context::default();
            f.theme.apply(&ctx);
            let mut open = true;
            let mut prefs = super::keymap::Preferences::default();
            let mut keymap = super::keymap::Keymap::blender();
            let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280., 800.))), ..Default::default() };
            let _ = ctx.run(input, |ctx| {
                assert!(!super::keymap::show_preferences(ctx, &mut open, &mut prefs, &mut keymap, &f.theme));
            });
        }
    }
    #[test]
    fn light_theme_ink_is_legible_on_selected_fills() {
        fn luminance(color: egui::Color32) -> f32 {
            let channel = |value: u8| {
                let x = f32::from(value) / 255.;
                if x <= 0.04045 { x / 12.92 } else { ((x + 0.055) / 1.055).powf(2.4) }
            };
            0.2126 * channel(color.r()) + 0.7152 * channel(color.g()) + 0.0722 * channel(color.b())
        }
        fn contrast(a: egui::Color32, b: egui::Color32) -> f32 {
            let (lo, hi) = (luminance(a), luminance(b));
            (hi.max(lo) + 0.05) / (hi.min(lo) + 0.05)
        }
        let t = theme::Theme::light();
        assert!(contrast(t.text, t.accent_bg) >= 4.5);
        for (ink, fill) in [(t.sel, t.warn), (t.card, t.dim), (t.sel, t.tpl), (t.sel, t.accent)] {
            assert!(contrast(ink, fill) >= 3., "{ink:?} on {fill:?}");
        }
    }
}
