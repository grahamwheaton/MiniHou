//! The editor's panels. Every panel is a module with `show(ui, &View, &mut Actions)`:
//! it reads the immutable `View` and pushes `UiAction`s; `App::update` applies them. A panel never touches the engine, the
//! document or the GPU directly, which is what lets each one be built and tested alone (see `ui::testing`).
#![allow(dead_code)] // the skeleton: each panel is filled in one at a time
pub mod editor_state;
pub mod network;
pub mod params;
pub mod shelf;
pub mod status;
pub mod theme;
pub mod topbar;
pub mod viewport_ui;

#[allow(unused_imports)]
pub use editor_state::{EditorChange, EditorState, Falloff, Orientation, Overlays, Pivot, SelectionMode, Snap, SnapTo, ViewPreset, Proportional};
use crate::viewport::Mode;
use minihou_app::{CookState, Edit};
use minihou_core::Graph;
use serde_json::Value;
use std::path::Path;

/// What a panel asks the app to do.
#[derive(Clone, Debug)]
pub enum UiAction {
    Open,
    Save,
    SaveAs,
    Undo,
    Redo,
    /// Frame the mesh in the viewport.
    Frame,
    Select(String),
    /// "Visible" flag: make this node the displayed one.
    SetDisplay(String),
    /// A document edit (parameter, mute/bypass, display, whole-graph replace). `continuous` is true while a drag is in progress.
    Edit { edit: Edit, continuous: bool },
    /// A shelf tool: add a node of this type (after `after`, wired to it, when given).
    AddNode { node_type: String, after: Option<String> },
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
    /// Wire `from`'s output to `to`'s input `port`; `None` port = its first input.
    ConnectNodes { from: String, to: String, port: Option<String> },
    DisconnectInput { node: String, port: String },
    /// Re-run the automatic layout for the whole network (card positions are UI state).
    TidyNetwork,
    // ---- application
    OpenPreferences,
    /// A named command from the keymap that has no dedicated action yet (kept for forward compatibility).
    Command(String),
}

#[derive(Default)]
pub struct Actions(pub Vec<UiAction>);
impl Actions {
    pub fn push(&mut self, action: UiAction) { self.0.push(action); }
}

/// UI-only per-node state (not part of the graph file).
#[derive(Clone, Default, Debug)]
pub struct NodeFlags { pub locked: bool, pub template: bool }

pub struct View<'a> {
    pub graph: &'a Graph,
    /// The document JSON (parameter values, node positions if the file has them).
    pub doc: &'a Value,
    pub path: &'a Path,
    pub dirty: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    pub selected: &'a str,
    pub display: &'a str,
    pub cooking: &'a CookState,
    pub file_status: &'a str,
    pub mode: Mode,
    pub theme: &'a theme::Theme,
    pub editor: &'a EditorState,
    /// Flags per node id (Locked, Template); ids not present have none set.
    pub flags: &'a std::collections::HashMap<String, NodeFlags>,
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
    use std::collections::HashMap;

    pub struct Fixture { pub doc: Document, pub graph: Graph, pub cooking: CookState, pub theme: theme::Theme,
        pub flags: HashMap<String, NodeFlags>, pub selected: String, pub display: String, pub mode: Mode, pub editor: EditorState }
    impl Fixture {
        pub fn new() -> Self {
            let doc = Document::parse(include_str!("../default_graph.json").to_owned()).unwrap();
            let graph = Graph::from_value(doc.value()).unwrap();
            let display = graph.main_output().unwrap();
            Self { doc, graph, cooking: CookState::default(), theme: theme::Theme::dark(), flags: HashMap::new(),
                selected: display.clone(), display, mode: Mode::Shaded, editor: EditorState::default() }
        }
        pub fn view(&self) -> View<'_> {
            View { graph: &self.graph, doc: self.doc.value(), path: Path::new("untitled.json"), dirty: false, can_undo: false, can_redo: false,
                selected: &self.selected, display: &self.display, cooking: &self.cooking, file_status: "", mode: self.mode,
                theme: &self.theme, editor: &self.editor, flags: &self.flags }
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
}
