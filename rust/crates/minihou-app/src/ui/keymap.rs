//! Keyboard commands for the editor. Alt plus a digit stands in for a numpad key in egui.
#[path = "prefs.rs"] mod prefs;
#[path = "prefs_ui.rs"] mod prefs_ui;
#[allow(unused_imports)]
pub use prefs::{Navigation, Preferences, ThemePref, ViewportPreferences, SCHEMA};
#[allow(unused_imports)]
pub use prefs_ui::show as show_preferences;

use eframe::egui::{self, Key, Modifiers};
use serde_json::{json, Value};
use super::{EditorChange, SelectionMode, Tool, UiAction, ViewPreset};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Chord { pub key: Key, pub ctrl: bool, pub shift: bool, pub alt: bool }
impl Chord {
    pub const fn new(key: Key, ctrl: bool, shift: bool, alt: bool) -> Self { Self { key, ctrl, shift, alt } }
    fn modifiers(self) -> Modifiers { Modifiers { ctrl: self.ctrl, shift: self.shift, alt: self.alt, ..Default::default() } }
    pub fn label(self) -> String {
        let mut s = String::new();
        if self.ctrl { s.push_str("Ctrl+"); }
        if self.shift { s.push_str("Shift+"); }
        if self.alt { s.push_str("Alt+"); }
        s.push_str(self.key.name()); s
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Context { Everywhere, Viewport, Network }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Command {
    Points, Edges, Faces, Object, ToggleEdit, SelectAll, SelectNone, Hide, Reveal, AddMenu, ToggleParams, FrameAll, FrameSelected,
    ToolSelect, ToolMove, ToolRotate, ToolScale, ToolTransform,
    Front, Back, Right, Left, Top, Bottom, ToggleProjection,
    OrbitDown, OrbitLeft, OrbitRight, OrbitUp,
    PanLeft, PanRight, PanUp, PanDown, ZoomIn, ZoomOut,
    Xray, Snap, Proportional, ShadeNext, ShadePrevious,
    Undo, Redo, Save, SaveAs, Open, Copy, Paste, DeleteNode, DissolveNode, DuplicateNode,
    TidyNetwork, Preferences, BoxSelect, CircleSelect, Paint, Extrude, Inset, Bevel, PushPull, LoopCut, FlipFaces, OppositeView, NetworkBox, PieMenu, DisplayFlag, MuteFlag, TemplateFlag, SelectableTemplateFlag, FindNode,
    TimelineToggle, TimelinePrevious, TimelineNext, TimelineStart, TimelineEnd,
}
impl Command {
    pub const ALL: [Self; 74] = [Self::Points, Self::Edges, Self::Faces, Self::Object, Self::ToggleEdit, Self::SelectAll, Self::SelectNone, Self::Hide, Self::Reveal, Self::AddMenu, Self::ToggleParams,
        Self::ToolSelect, Self::ToolMove, Self::ToolRotate, Self::ToolScale, Self::ToolTransform,
        Self::FrameAll, Self::FrameSelected, Self::Front, Self::Back, Self::Right, Self::Left,
        Self::Top, Self::Bottom, Self::ToggleProjection, Self::OrbitDown, Self::OrbitLeft,
        Self::OrbitRight, Self::OrbitUp, Self::PanLeft, Self::PanRight, Self::PanUp,
        Self::PanDown, Self::ZoomIn, Self::ZoomOut, Self::Xray, Self::Snap,
        Self::Proportional, Self::ShadeNext, Self::ShadePrevious, Self::Undo, Self::Redo,
        Self::Save, Self::SaveAs, Self::Open, Self::Copy, Self::Paste, Self::DeleteNode, Self::DissolveNode, Self::DuplicateNode,
        Self::TidyNetwork, Self::Preferences, Self::BoxSelect, Self::CircleSelect, Self::Paint, Self::Extrude, Self::Inset, Self::Bevel, Self::PushPull, Self::LoopCut, Self::FlipFaces, Self::OppositeView, Self::NetworkBox, Self::PieMenu, Self::DisplayFlag, Self::MuteFlag, Self::TemplateFlag, Self::SelectableTemplateFlag, Self::FindNode,
        Self::TimelineToggle, Self::TimelinePrevious, Self::TimelineNext, Self::TimelineStart, Self::TimelineEnd];
    pub fn label(self) -> &'static str { match self {
        Self::Points => "Points", Self::Edges => "Edges", Self::Faces => "Faces", Self::Object => "Object", Self::ToggleEdit => "Toggle Object / edit mode",
        Self::SelectAll => "Select all", Self::SelectNone => "Select none", Self::Hide => "Hide selected", Self::Reveal => "Reveal hidden", Self::AddMenu => "Add node menu", Self::ToggleParams => "Toggle parameters",
        Self::ToolSelect => "Select tool", Self::ToolMove => "Move (modal)", Self::ToolRotate => "Rotate (modal)",
        Self::ToolScale => "Scale (modal)", Self::ToolTransform => "Transform tool",
        Self::FrameAll => "Frame all", Self::FrameSelected => "Frame selected",
        Self::Front => "Front", Self::Back => "Back", Self::Right => "Right", Self::Left => "Left",
        Self::Top => "Top", Self::Bottom => "Bottom", Self::ToggleProjection => "Toggle perspective / ortho",
        Self::OrbitDown => "Orbit down", Self::OrbitLeft => "Orbit left", Self::OrbitRight => "Orbit right", Self::OrbitUp => "Orbit up",
        Self::PanLeft => "Pan left", Self::PanRight => "Pan right", Self::PanUp => "Pan up", Self::PanDown => "Pan down",
        Self::ZoomIn => "Zoom in", Self::ZoomOut => "Zoom out", Self::Xray => "X-ray", Self::Snap => "Snapping",
        Self::Proportional => "Proportional editing", Self::ShadeNext => "Next shading", Self::ShadePrevious => "Previous shading",
        Self::Undo => "Undo", Self::Redo => "Redo", Self::Save => "Save", Self::SaveAs => "Save as", Self::Open => "Open",
        Self::Copy => "Copy nodes", Self::Paste => "Paste nodes",
        Self::DeleteNode => "Delete nodes", Self::DissolveNode => "Dissolve nodes", Self::DuplicateNode => "Duplicate nodes", Self::TidyNetwork => "Tidy network",
        Self::Preferences => "Preferences",
        Self::BoxSelect => "Box select", Self::CircleSelect => "Circle select", Self::Paint => "Paint",
        Self::Extrude => "Extrude", Self::Inset => "Inset", Self::Bevel => "Bevel", Self::PushPull => "Push / pull",
        Self::LoopCut => "Loop cut", Self::FlipFaces => "Flip faces", Self::OppositeView => "Opposite view",
        Self::NetworkBox => "Network box", Self::PieMenu => "Modeling pie menu", Self::DisplayFlag => "Display flag",
        Self::MuteFlag => "Mute flag", Self::TemplateFlag => "Template flag", Self::SelectableTemplateFlag => "Selectable template flag",
        Self::FindNode => "Find node", Self::TimelineToggle => "Play / pause", Self::TimelinePrevious => "Previous frame",
        Self::TimelineNext => "Next frame", Self::TimelineStart => "First frame", Self::TimelineEnd => "Last frame",
    } }
    fn id(self) -> &'static str { match self {
        Self::Points => "points", Self::Edges => "edges", Self::Faces => "faces", Self::Object => "object", Self::ToggleEdit => "toggle_edit",
        Self::SelectAll => "select_all", Self::SelectNone => "select_none", Self::Hide => "hide", Self::Reveal => "reveal", Self::AddMenu => "add_menu", Self::ToggleParams => "toggle_params",
        Self::ToolSelect => "tool_select", Self::ToolMove => "tool_move", Self::ToolRotate => "tool_rotate",
        Self::ToolScale => "tool_scale", Self::ToolTransform => "tool_transform",
        Self::FrameAll => "frame_all", Self::FrameSelected => "frame_selected", Self::Front => "front", Self::Back => "back",
        Self::Right => "right", Self::Left => "left", Self::Top => "top", Self::Bottom => "bottom",
        Self::ToggleProjection => "toggle_projection", Self::OrbitDown => "orbit_down", Self::OrbitLeft => "orbit_left",
        Self::OrbitRight => "orbit_right", Self::OrbitUp => "orbit_up", Self::PanLeft => "pan_left", Self::PanRight => "pan_right",
        Self::PanUp => "pan_up", Self::PanDown => "pan_down", Self::ZoomIn => "zoom_in", Self::ZoomOut => "zoom_out",
        Self::Xray => "xray", Self::Snap => "snap", Self::Proportional => "proportional", Self::ShadeNext => "shade_next",
        Self::ShadePrevious => "shade_previous", Self::Undo => "undo", Self::Redo => "redo", Self::Save => "save",
        Self::SaveAs => "save_as", Self::Open => "open", Self::Copy => "copy", Self::Paste => "paste", Self::DeleteNode => "delete_node", Self::DissolveNode => "dissolve_node", Self::DuplicateNode => "duplicate_node",
        Self::TidyNetwork => "tidy_network", Self::Preferences => "preferences",
        Self::BoxSelect => "box_select", Self::CircleSelect => "circle_select", Self::Paint => "paint",
        Self::Extrude => "extrude", Self::Inset => "inset", Self::Bevel => "bevel", Self::PushPull => "pushpull",
        Self::LoopCut => "loop_cut", Self::FlipFaces => "flip_faces", Self::OppositeView => "opposite_view",
        Self::NetworkBox => "network_box", Self::PieMenu => "pie_menu", Self::DisplayFlag => "display_flag",
        Self::MuteFlag => "mute_flag", Self::TemplateFlag => "template_flag", Self::SelectableTemplateFlag => "selectable_template_flag",
        Self::FindNode => "find_node", Self::TimelineToggle => "timeline_toggle", Self::TimelinePrevious => "timeline_previous",
        Self::TimelineNext => "timeline_next", Self::TimelineStart => "timeline_start", Self::TimelineEnd => "timeline_end",
    } }
    pub fn to_action(self, kctx: &KeyContext<'_>) -> Option<UiAction> { Some(match self {
        Self::Points => UiAction::Editor(EditorChange::Selection(SelectionMode::Points)),
        Self::Edges => UiAction::Editor(EditorChange::Selection(SelectionMode::Edges)),
        Self::Faces => UiAction::Editor(EditorChange::Selection(SelectionMode::Faces)),
        Self::Object => UiAction::Editor(EditorChange::Selection(SelectionMode::Object)),
        Self::ToggleEdit | Self::SelectAll | Self::SelectNone | Self::Hide | Self::Reveal | Self::AddMenu | Self::ToggleParams => UiAction::Command(self.id().to_owned()),
        Self::ToolSelect => UiAction::Editor(EditorChange::Tool(Tool::Select)),
        Self::ToolMove => UiAction::StartModal(super::modal_transform::Kind::Move),
        Self::ToolRotate => UiAction::StartModal(super::modal_transform::Kind::Rotate),
        Self::ToolScale => UiAction::StartModal(super::modal_transform::Kind::Scale),
        Self::ToolTransform => UiAction::Editor(EditorChange::Tool(Tool::Transform)),
        Self::FrameAll => if kctx.network_hovered { UiAction::FrameNetwork } else { UiAction::Frame },
        Self::FrameSelected => UiAction::FrameSelected,
        Self::Front => UiAction::ViewPreset(ViewPreset::Front), Self::Back => UiAction::ViewPreset(ViewPreset::Back),
        Self::Right => UiAction::ViewPreset(ViewPreset::Right), Self::Left => UiAction::ViewPreset(ViewPreset::Left),
        Self::Top => UiAction::ViewPreset(ViewPreset::Top), Self::Bottom => UiAction::ViewPreset(ViewPreset::Bottom),
        Self::OrbitDown => UiAction::Orbit { yaw: 0., pitch: -15. }, Self::OrbitLeft => UiAction::Orbit { yaw: -15., pitch: 0. },
        Self::OrbitRight => UiAction::Orbit { yaw: 15., pitch: 0. }, Self::OrbitUp => UiAction::Orbit { yaw: 0., pitch: 15. },
        Self::PanLeft => UiAction::Pan { dx: -1., dy: 0. }, Self::PanRight => UiAction::Pan { dx: 1., dy: 0. },
        Self::PanUp => UiAction::Pan { dx: 0., dy: -1. }, Self::PanDown => UiAction::Pan { dx: 0., dy: 1. },
        Self::ZoomIn => UiAction::Zoom(1.2), Self::ZoomOut => UiAction::Zoom(1. / 1.2),
        Self::Undo => UiAction::Undo, Self::Redo => UiAction::Redo, Self::Save => UiAction::Save,
        Self::SaveAs => UiAction::SaveAs, Self::Open => UiAction::Open,
        Self::DeleteNode => UiAction::Command("delete_selection".into()),
        Self::DissolveNode => UiAction::Command("dissolve_selection".into()),
        Self::DuplicateNode => UiAction::Command("duplicate_selection".into()),
        Self::Copy | Self::Paste => UiAction::Command(self.id().to_owned()),
        Self::TidyNetwork => UiAction::TidyNetwork, Self::Preferences => UiAction::OpenPreferences,
        _ => UiAction::Command(self.id().to_owned()),
    }) }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Binding { pub chord: Chord, pub command: Command, pub context: Context }
#[derive(Clone, Copy, Debug, Default)]
pub struct KeyContext<'a> { pub viewport_hovered: bool, pub network_hovered: bool, pub in_edit_mode: bool, pub selected: Option<&'a str> }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict { pub command: Command, pub chord: Chord, pub context: Context }
#[derive(Clone, Debug)]
pub struct Keymap { pub bindings: Vec<Binding>, pub emulate_numpad: bool }
impl Keymap {
    pub fn blender() -> Self {
        use Command as C; use Context as X; use Key as K;
        let mut b = Vec::new();
        let mut add = |key, ctrl, shift, alt, command, context| b.push(Binding { chord: Chord::new(key, ctrl, shift, alt), command, context });
        // editor.js viewport keydown: component modes, live transforms, modeling, selection and frame.
        for (key, cmd) in [(K::Num1,C::Points),(K::Num2,C::Edges),(K::Num3,C::Faces),(K::Num4,C::Object),
            (K::G,C::ToolMove),(K::R,C::ToolRotate),(K::S,C::ToolScale),(K::B,C::BoxSelect),(K::C,C::CircleSelect),
            (K::A,C::SelectAll),(K::F,C::FrameSelected),(K::O,C::Proportional),(K::Num5,C::Paint),
            (K::E,C::Extrude),(K::I,C::Inset),(K::Escape,C::SelectNone)] {
            add(key,false,false,false,cmd,X::Viewport);
        }
        for (key,ctrl,alt,cmd) in [(K::B,true,false,C::Bevel),(K::E,false,true,C::PushPull),
            (K::R,true,false,C::LoopCut),(K::N,false,true,C::FlipFaces)] { add(key,ctrl,false,alt,cmd,X::Viewport); }
        for (key,cmd) in [(K::Tab,C::AddMenu),(K::Home,C::FrameSelected)] {
            add(key,false,false,false,cmd,X::Viewport);
        }
        for (key,shift,cmd) in [(K::Space,false,C::TimelineToggle),(K::ArrowLeft,false,C::TimelinePrevious),
            (K::ArrowRight,false,C::TimelineNext),(K::ArrowLeft,true,C::TimelineStart),(K::ArrowRight,true,C::TimelineEnd)] {
            add(key,false,shift,false,cmd,X::Viewport);
        }
        add(K::Tab,false,false,false,C::AddMenu,X::Network);
        // Numpad 1/3/7 and their opposite views, 5 projection, 9 flip, decimal frame.
        // egui reports number-row and numpad digits alike; Alt is the default fallback.
        for (key,ctrl,cmd) in [(K::Num1,false,C::Front),(K::Num1,true,C::Back),
            (K::Num3,false,C::Right),(K::Num3,true,C::Left),(K::Num7,false,C::Top),(K::Num7,true,C::Bottom),
            (K::Num5,false,C::ToggleProjection),(K::Num9,false,C::OppositeView),
            (K::Period,false,C::FrameSelected)] { add(key,ctrl,false,true,cmd,X::Viewport); }
        for (key,cmd) in [(K::F,C::FrameAll),(K::L,C::TidyNetwork),(K::O,C::NetworkBox),
            (K::C,C::PieMenu),(K::R,C::DisplayFlag),(K::M,C::MuteFlag),(K::E,C::TemplateFlag),
            (K::W,C::SelectableTemplateFlag)] { add(key,false,false,false,cmd,X::Network); }
        add(K::R,false,true,false,C::DisplayFlag,X::Network);
        for (key,ctrl,shift,cmd) in [(K::Z,true,false,C::Undo),(K::Y,true,false,C::Redo),
            (K::Z,true,true,C::Redo),(K::S,true,false,C::Save),(K::S,true,true,C::SaveAs),
            (K::O,true,false,C::Open)] {
            add(key,ctrl,shift,false,cmd,X::Everywhere);
        }
        add(K::D,true,false,false,C::DuplicateNode,X::Network);
        add(K::X,true,false,false,C::DissolveNode,X::Network);
        for (key,cmd) in [(K::C,C::Copy),(K::V,C::Paste),(K::A,C::SelectAll),
            (K::F,C::FindNode)] { add(key,true,false,false,cmd,X::Network); }
        for key in [K::Delete,K::Backspace] { add(key,false,false,false,C::DeleteNode,X::Network); }
        add(K::X,false,false,false,C::DeleteNode,X::Network);
        for key in [K::X, K::Delete] { add(key,false,false,false,C::DeleteNode,X::Viewport); }
        Self { bindings: b, emulate_numpad: false }
    }
    pub fn reset(&mut self) { *self = Self::blender(); }
    pub fn rebind(&mut self, command: Command, chord: Chord) -> Result<(), Conflict> {
        let Some(index) = self.bindings.iter().position(|b| b.command == command) else { return Ok(()); };
        self.rebind_at(index,chord)
    }
    pub fn rebind_at(&mut self, index: usize, chord: Chord) -> Result<(), Conflict> {
        if self.bindings.get(index).is_none() { return Ok(()); }
        let context = self.bindings[index].context;
        if let Some(b) = self.bindings.iter().enumerate().find_map(|(i,b)| (i != index && b.chord == chord &&
            (b.context == context || b.context == Context::Everywhere || context == Context::Everywhere)).then_some(b)) {
            return Err(Conflict { command: b.command, chord, context: b.context });
        }
        self.bindings[index].chord = chord; Ok(())
    }
    fn effective(&self) -> Vec<Binding> {
        let mut b = self.bindings.clone();
        if self.emulate_numpad {
            let mut extra = Vec::new();
            for entry in &mut b {
                if matches!(entry.command, Command::Points | Command::Edges | Command::Faces | Command::Object | Command::Paint) { entry.chord.shift = true; }
                if entry.context == Context::Viewport && entry.chord.alt &&
                    matches!(entry.chord.key, Key::Num1 | Key::Num2 | Key::Num3 | Key::Num4 | Key::Num5 | Key::Num6 | Key::Num7 | Key::Num8 | Key::Num9 | Key::Plus | Key::Minus | Key::Period) {
                    let mut plain = *entry; plain.chord.alt = false; extra.push(plain);
                }
            }
            b.extend(extra);
        }
        b
    }
    pub fn poll(&self, ctx: &egui::Context, kctx: &KeyContext<'_>) -> Vec<UiAction> {
        if ctx.wants_keyboard_input() { return Vec::new(); }
        let mut bindings = self.effective();
        bindings.sort_by_key(|b| std::cmp::Reverse(u8::from(b.chord.ctrl) + u8::from(b.chord.shift) + u8::from(b.chord.alt)));
        let mut out = Vec::new();
        ctx.input_mut(|input| for b in bindings {
            if !(match b.context { Context::Everywhere => true, Context::Viewport => kctx.viewport_hovered, Context::Network => kctx.network_hovered }) { continue; }
            let exact = input.events.iter().any(|e| matches!(e, egui::Event::Key { key, pressed: true, modifiers, .. }
                if *key == b.chord.key && modifiers.ctrl == b.chord.ctrl && modifiers.shift == b.chord.shift && modifiers.alt == b.chord.alt));
            if exact && input.consume_key(b.chord.modifiers(), b.chord.key) {
                if let Some(action) = b.command.to_action(kctx) { out.push(action); }
            }
        });
        out
    }
    pub fn overrides_json(&self) -> Value {
        let defaults = Self::blender();
        let overrides: Vec<Value> = self.bindings.iter().enumerate()
            .filter(|(i,b)| defaults.bindings.get(*i).is_some_and(|d| d.chord != b.chord))
            .map(|(i,b)| json!({ "slot": i, "command": b.command.id(), "key": format!("{:?}",b.chord.key),
                "ctrl": b.chord.ctrl, "shift": b.chord.shift, "alt": b.chord.alt }))
            .collect();
        json!({"bindings": overrides})
    }
    pub fn from_overrides_json(value: &Value) -> Self {
        let mut map = Self::blender();
        if let Some(rows) = value.get("bindings").and_then(Value::as_array) {
            for row in rows {
                let Some(command) = row.get("command").and_then(Value::as_str).and_then(|s| Command::ALL.into_iter().find(|c| c.id() == s)) else { continue; };
                let Some(key) = row.get("key").and_then(Value::as_str).and_then(parse_key) else { continue; };
                let chord = Chord::new(key, row.get("ctrl").and_then(Value::as_bool).unwrap_or(false),
                    row.get("shift").and_then(Value::as_bool).unwrap_or(false), row.get("alt").and_then(Value::as_bool).unwrap_or(false));
                if let Some(index) = row.get("slot").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok())
                    .filter(|i| map.bindings.get(*i).is_some_and(|b| b.command == command)) {
                    let _ = map.rebind_at(index,chord);
                } else { let _ = map.rebind(command,chord); }
            }
        }
        map
    }
}
fn parse_key(name: &str) -> Option<Key> { Key::ALL.iter().copied().find(|k| format!("{k:?}") == name) }

#[cfg(test)]
fn default_keymap_table() -> String {
    let mut rows = String::from("| Shortcut | Command | Context |\n| --- | --- | --- |\n");
    for binding in Keymap::blender().bindings {
        rows.push_str(&format!("| {} | {} | {:?} |\n", binding.chord.label(), binding.command.label(), binding.context));
    }
    rows
}

#[cfg(test)]
fn modal_keymap_table() -> String {
    let mut rows=String::from("| Input | During modal transform |\n| --- | --- |\n");
    for (key,action) in MODAL_KEYS { rows.push_str(&format!("| {key} | {action} |\n")); }
    rows
}

/// These inputs are consumed by the modal operator before normal keymap dispatch.
pub const MODAL_KEYS: &[(&str,&str)] = &[
    ("G / R / S", "Begin move / rotate / scale while the pointer is in the viewport"),
    ("LMB / Enter / Space", "Confirm"), ("RMB / Escape", "Cancel"),
    ("X / Y / Z", "Axis; again local axis; third press clears"),
    ("Shift+X / Y / Z", "Plane excluding axis"),
    ("Middle mouse", "Choose closest motion axis; again clears"),
    ("Digits / . / - / Backspace", "Enter exact distance, angle in degrees, or scale factor"),
    ("Tab", "Next move numeric field"), ("Ctrl", "Invert snapping"),
    ("Shift", "Precision movement"), ("O", "Toggle proportional editing"),
    ("Wheel / PageUp / PageDown", "Change proportional radius"),
    ("B while moving", "Cycle Increment / Vertex / Edge / Face snap target"),
    ("Ctrl+Shift+LMB while moving", "Pick snap reference point"),
    ("R while rotating", "Toggle trackball"),
    ("B while rotating", "Pick two-point pivot, reference, then angle"),
    ("C while scaling", "Toggle scale cage; click a handle, move, click to confirm"),
    ("Alt while scaling", "Individual origins is not yet supported for multiple displayed objects"),
];

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn guide_keymap_matches_defaults() {
        let guide = include_str!("../../docs/NATIVE_EDITOR.md").replace("\r\n", "\n");
        let table = guide.split_once("<!-- keymap:start -->\n").unwrap().1.split_once("<!-- keymap:end -->").unwrap().0;
        assert_eq!(table, default_keymap_table());
    }
    #[test] fn guide_modal_table_matches_keys() {
        let guide=include_str!("../../docs/NATIVE_EDITOR.md").replace("\r\n","\n");
        let table=guide.split_once("<!-- modal:start -->\n").unwrap().1.split_once("<!-- modal:end -->").unwrap().0;
        assert_eq!(table,modal_keymap_table());
    }
    #[test] fn audit_added_keys_have_default_bindings() {
        let audit = include_str!("../../../../../docs/KEYMAP_AUDIT.md");
        let map = Keymap::blender();
        let mut checked = 0;
        for row in audit.lines().filter(|line| line.starts_with("| ") && line.contains("| added now")) {
            let cells: Vec<_> = row.split('|').map(str::trim).collect();
            let context = if cells[2].starts_with("Network") { Context::Network }
                else if cells[2].starts_with("Viewport") { Context::Viewport } else { continue };
            for label in cells[1].split(" / ") {
                if !["Ctrl+A", "Ctrl+C", "Ctrl+V", "Ctrl+D", "Ctrl+X", "X", "Delete", "Backspace", "M", "E"].contains(&label) { continue; }
                assert!(map.bindings.iter().any(|binding| binding.context == context && binding.chord.label() == label), "audit row has no binding: {row}");
                checked += 1;
            }
        }
        assert!(checked >= 11);
    }
    #[test] fn defaults_are_unambiguous() {
        let map = Keymap::blender();
        for (i,a) in map.bindings.iter().enumerate() { for b in &map.bindings[i+1..] {
            assert!(a.chord != b.chord || (a.context != b.context && a.context != Context::Everywhere && b.context != Context::Everywhere), "{a:?} / {b:?}");
        } }
    }
    #[test] fn views_and_overrides() {
        let mut map = Keymap::blender();
        for (key,ctrl,preset) in [(Key::Num1,false,ViewPreset::Front),(Key::Num1,true,ViewPreset::Back),
            (Key::Num3,false,ViewPreset::Right),(Key::Num3,true,ViewPreset::Left),
            (Key::Num7,false,ViewPreset::Top),(Key::Num7,true,ViewPreset::Bottom)] {
            let b = map.bindings.iter().find(|b| b.chord == Chord::new(key,ctrl,false,true)).unwrap();
            assert!(matches!(b.command.to_action(&KeyContext::default()), Some(UiAction::ViewPreset(p)) if p == preset));
        }
        let chord = Chord::new(Key::F12,false,false,false);
        map.rebind(Command::Save,chord).unwrap();
        assert_eq!(map.rebind(Command::Open,chord).unwrap_err().command,Command::Save);
        let value = map.overrides_json(); assert_eq!(value["bindings"].as_array().unwrap().len(),1);
        assert_eq!(Keymap::from_overrides_json(&value).overrides_json(),value);
        map.reset(); assert_eq!(map.overrides_json()["bindings"].as_array().unwrap().len(),0);
    }
    #[test] fn copy_and_paste_are_network_commands() {
        let map = Keymap::blender();
        for (key, command) in [(Key::C, "copy"), (Key::V, "paste")] {
            let binding = map.bindings.iter().find(|b| b.chord == Chord::new(key, true, false, false)).unwrap();
            assert_eq!(binding.context, Context::Network);
            assert!(matches!(binding.command.to_action(&KeyContext::default()), Some(UiAction::Command(name)) if name == command));
        }
    }
    #[test] fn web_shortcuts_keep_their_contexts() {
        let map = Keymap::blender();
        for (key, command) in [(Key::G, Command::ToolMove), (Key::R, Command::ToolRotate),
            (Key::S, Command::ToolScale), (Key::Num4, Command::Object), (Key::F, Command::FrameSelected),
            (Key::E, Command::Extrude), (Key::C, Command::CircleSelect)] {
            assert!(map.bindings.iter().any(|b| b.chord == Chord::new(key, false, false, false)
                && b.command == command && b.context == Context::Viewport));
        }
        for (key, command) in [(Key::Tab, Command::AddMenu), (Key::C, Command::PieMenu),
            (Key::R, Command::DisplayFlag), (Key::E, Command::TemplateFlag)] {
            assert!(map.bindings.iter().any(|b| b.chord == Chord::new(key, false, false, false)
                && b.command == command && b.context == Context::Network));
        }
        assert!(map.bindings.iter().any(|b| b.chord == Chord::new(Key::R, false, true, false)
            && b.command == Command::DisplayFlag && b.context == Context::Network));
        assert!(!map.bindings.iter().any(|b| b.command == Command::ToggleEdit));
    }
    #[test] fn alternate_shortcut_round_trips_and_emulation_moves_selection() {
        let mut map = Keymap::blender();
        let redo_y = map.bindings.iter().position(|b| b.command == Command::Redo && b.chord.key == Key::Y).unwrap();
        map.rebind_at(redo_y,Chord::new(Key::F11,false,false,false)).unwrap();
        let restored = Keymap::from_overrides_json(&map.overrides_json());
        assert_eq!(restored.bindings[redo_y].chord,Chord::new(Key::F11,false,false,false));
        assert!(restored.bindings.iter().any(|b| b.command == Command::Redo && b.chord == Chord::new(Key::Z,true,true,false)));
        map.emulate_numpad = true;
        let effective = map.effective();
        assert!(effective.iter().any(|b| b.command == Command::Front && b.chord == Chord::new(Key::Num1,false,false,false)));
        assert!(effective.iter().any(|b| b.command == Command::Points && b.chord == Chord::new(Key::Num1,false,true,false)));
    }
    #[test] fn poll_consumes_key() {
        let ctx = egui::Context::default();
        let input = egui::RawInput { events: vec![egui::Event::Key { key: Key::Num1, physical_key: None,
            pressed: true, repeat: false, modifiers: Modifiers::default() }], ..Default::default() };
        let mut found = Vec::new();
        let _ = ctx.run(input, |ctx| { found = Keymap::blender().poll(ctx,&KeyContext { viewport_hovered: true, ..Default::default() });
            assert!(!ctx.input(|i| i.key_pressed(Key::Num1))); });
        assert!(matches!(found.as_slice(),[UiAction::Editor(EditorChange::Selection(SelectionMode::Points))]));
        let input = egui::RawInput { events: vec![egui::Event::Key { key: Key::Num1, physical_key: None,
            pressed: true, repeat: false, modifiers: Modifiers::default() }], ..Default::default() };
        let _ = ctx.run(input, |ctx| { found = Keymap::blender().poll(ctx,&KeyContext { viewport_hovered: true, in_edit_mode: true, ..Default::default() }); });
        assert!(matches!(found.as_slice(),[UiAction::Editor(EditorChange::Selection(SelectionMode::Points))]));
    }
}
