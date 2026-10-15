//! Keyboard commands for the editor. Alt plus a digit stands in for a numpad key in egui.
#[path = "prefs.rs"] mod prefs;
#[path = "prefs_ui.rs"] mod prefs_ui;
#[allow(unused_imports)]
pub use prefs::{Preferences, ThemePref, ViewportPreferences, SCHEMA};
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
    Points, Edges, Faces, Object, FrameAll, FrameSelected,
    ToolSelect, ToolMove, ToolRotate, ToolScale, ToolTransform,
    Front, Back, Right, Left, Top, Bottom, ToggleProjection,
    OrbitDown, OrbitLeft, OrbitRight, OrbitUp,
    PanLeft, PanRight, PanUp, PanDown, ZoomIn, ZoomOut,
    Xray, Snap, Proportional, ShadeNext, ShadePrevious,
    Undo, Redo, Save, SaveAs, Open, Copy, Paste, DeleteNode, DuplicateNode,
    TidyNetwork, Preferences,
}
impl Command {
    pub const ALL: [Self; 44] = [Self::Points, Self::Edges, Self::Faces, Self::Object,
        Self::ToolSelect, Self::ToolMove, Self::ToolRotate, Self::ToolScale, Self::ToolTransform,
        Self::FrameAll, Self::FrameSelected, Self::Front, Self::Back, Self::Right, Self::Left,
        Self::Top, Self::Bottom, Self::ToggleProjection, Self::OrbitDown, Self::OrbitLeft,
        Self::OrbitRight, Self::OrbitUp, Self::PanLeft, Self::PanRight, Self::PanUp,
        Self::PanDown, Self::ZoomIn, Self::ZoomOut, Self::Xray, Self::Snap,
        Self::Proportional, Self::ShadeNext, Self::ShadePrevious, Self::Undo, Self::Redo,
        Self::Save, Self::SaveAs, Self::Open, Self::Copy, Self::Paste, Self::DeleteNode, Self::DuplicateNode,
        Self::TidyNetwork, Self::Preferences];
    pub fn label(self) -> &'static str { match self {
        Self::Points => "Points", Self::Edges => "Edges", Self::Faces => "Faces", Self::Object => "Object",
        Self::ToolSelect => "Select tool", Self::ToolMove => "Move tool", Self::ToolRotate => "Rotate tool",
        Self::ToolScale => "Scale tool", Self::ToolTransform => "Transform tool",
        Self::FrameAll => "Frame all", Self::FrameSelected => "Frame selected",
        Self::Front => "Front", Self::Back => "Back", Self::Right => "Right", Self::Left => "Left",
        Self::Top => "Top", Self::Bottom => "Bottom", Self::ToggleProjection => "Toggle perspective / ortho",
        Self::OrbitDown => "Orbit down", Self::OrbitLeft => "Orbit left", Self::OrbitRight => "Orbit right", Self::OrbitUp => "Orbit up",
        Self::PanLeft => "Pan left", Self::PanRight => "Pan right", Self::PanUp => "Pan up", Self::PanDown => "Pan down",
        Self::ZoomIn => "Zoom in", Self::ZoomOut => "Zoom out", Self::Xray => "X-ray", Self::Snap => "Snapping",
        Self::Proportional => "Proportional editing", Self::ShadeNext => "Next shading", Self::ShadePrevious => "Previous shading",
        Self::Undo => "Undo", Self::Redo => "Redo", Self::Save => "Save", Self::SaveAs => "Save as", Self::Open => "Open",
        Self::Copy => "Copy nodes", Self::Paste => "Paste nodes",
        Self::DeleteNode => "Delete node", Self::DuplicateNode => "Duplicate node", Self::TidyNetwork => "Tidy network",
        Self::Preferences => "Preferences",
    } }
    fn id(self) -> &'static str { match self {
        Self::Points => "points", Self::Edges => "edges", Self::Faces => "faces", Self::Object => "object",
        Self::ToolSelect => "tool_select", Self::ToolMove => "tool_move", Self::ToolRotate => "tool_rotate",
        Self::ToolScale => "tool_scale", Self::ToolTransform => "tool_transform",
        Self::FrameAll => "frame_all", Self::FrameSelected => "frame_selected", Self::Front => "front", Self::Back => "back",
        Self::Right => "right", Self::Left => "left", Self::Top => "top", Self::Bottom => "bottom",
        Self::ToggleProjection => "toggle_projection", Self::OrbitDown => "orbit_down", Self::OrbitLeft => "orbit_left",
        Self::OrbitRight => "orbit_right", Self::OrbitUp => "orbit_up", Self::PanLeft => "pan_left", Self::PanRight => "pan_right",
        Self::PanUp => "pan_up", Self::PanDown => "pan_down", Self::ZoomIn => "zoom_in", Self::ZoomOut => "zoom_out",
        Self::Xray => "xray", Self::Snap => "snap", Self::Proportional => "proportional", Self::ShadeNext => "shade_next",
        Self::ShadePrevious => "shade_previous", Self::Undo => "undo", Self::Redo => "redo", Self::Save => "save",
        Self::SaveAs => "save_as", Self::Open => "open", Self::Copy => "copy", Self::Paste => "paste", Self::DeleteNode => "delete_node", Self::DuplicateNode => "duplicate_node",
        Self::TidyNetwork => "tidy_network", Self::Preferences => "preferences",
    } }
    pub fn to_action(self, kctx: &KeyContext<'_>) -> Option<UiAction> { Some(match self {
        Self::Points => UiAction::Editor(EditorChange::Selection(SelectionMode::Points)),
        Self::Edges => UiAction::Editor(EditorChange::Selection(SelectionMode::Edges)),
        Self::Faces => UiAction::Editor(EditorChange::Selection(SelectionMode::Faces)),
        Self::Object => UiAction::Editor(EditorChange::Selection(SelectionMode::Object)),
        Self::ToolSelect => UiAction::Editor(EditorChange::Tool(Tool::Select)),
        Self::ToolMove => UiAction::Editor(EditorChange::Tool(Tool::Move)),
        Self::ToolRotate => UiAction::Editor(EditorChange::Tool(Tool::Rotate)),
        Self::ToolScale => UiAction::Editor(EditorChange::Tool(Tool::Scale)),
        Self::ToolTransform => UiAction::Editor(EditorChange::Tool(Tool::Transform)),
        Self::FrameAll | Self::FrameSelected => UiAction::Frame,
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
        Self::DeleteNode => UiAction::DeleteNode(kctx.selected?.to_owned()),
        Self::DuplicateNode => UiAction::DuplicateNode(kctx.selected?.to_owned()),
        Self::Copy | Self::Paste => UiAction::Command(self.id().to_owned()),
        Self::TidyNetwork => UiAction::TidyNetwork, Self::Preferences => UiAction::OpenPreferences,
        _ => UiAction::Command(self.id().to_owned()),
    }) }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Binding { pub chord: Chord, pub command: Command, pub context: Context }
#[derive(Clone, Copy, Debug, Default)]
pub struct KeyContext<'a> { pub viewport_hovered: bool, pub network_hovered: bool, pub selected: Option<&'a str> }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict { pub command: Command, pub chord: Chord, pub context: Context }
#[derive(Clone, Debug)]
pub struct Keymap { pub bindings: Vec<Binding>, pub emulate_numpad: bool }
impl Keymap {
    pub fn blender() -> Self {
        use Command as C; use Context as X; use Key as K;
        let mut b = Vec::new();
        let mut add = |key, ctrl, shift, alt, command, context| b.push(Binding { chord: Chord::new(key, ctrl, shift, alt), command, context });
        for (key, cmd) in [(K::Num1,C::Points),(K::Num2,C::Edges),(K::Num3,C::Faces),(K::Num4,C::Object)] { add(key,false,false,false,cmd,X::Viewport); }
        for (key, cmd) in [(K::Q,C::ToolSelect),(K::W,C::ToolMove),(K::E,C::ToolRotate),(K::R,C::ToolScale),(K::T,C::ToolTransform)] { add(key,false,false,false,cmd,X::Viewport); }
        add(K::Home,false,false,false,C::FrameAll,X::Viewport);
        add(K::Period,false,false,true,C::FrameSelected,X::Viewport);
        for (key, ctrl, cmd) in [(K::Num1,false,C::Front),(K::Num1,true,C::Back),(K::Num3,false,C::Right),
            (K::Num3,true,C::Left),(K::Num7,false,C::Top),(K::Num7,true,C::Bottom),
            (K::Num5,false,C::ToggleProjection),(K::Num2,false,C::OrbitDown),(K::Num4,false,C::OrbitLeft),
            (K::Num6,false,C::OrbitRight),(K::Num8,false,C::OrbitUp),(K::Num4,true,C::PanLeft),
            (K::Num6,true,C::PanRight),(K::Num8,true,C::PanUp),(K::Num2,true,C::PanDown),
            (K::Plus,false,C::ZoomIn),(K::Minus,false,C::ZoomOut)] { add(key,ctrl,false,true,cmd,X::Viewport); }
        for (key,ctrl,shift,alt,cmd) in [(K::Z,false,false,true,C::Xray),(K::Tab,false,true,false,C::Snap),
            (K::O,false,false,false,C::Proportional),(K::Z,false,false,false,C::ShadeNext),
            (K::Z,false,true,false,C::ShadePrevious)] { add(key,ctrl,shift,alt,cmd,X::Viewport); }
        for (key,ctrl,shift,cmd) in [(K::Z,true,false,C::Undo),(K::Y,true,false,C::Redo),
            (K::Z,true,true,C::Redo),(K::S,true,false,C::Save),(K::S,true,true,C::SaveAs),
            (K::O,true,false,C::Open),(K::Comma,true,false,C::Preferences)] { add(key,ctrl,shift,false,cmd,X::Everywhere); }
        for (key,ctrl,shift,cmd) in [(K::C,true,false,C::Copy),(K::V,true,false,C::Paste),
            (K::X,false,false,C::DeleteNode),(K::Delete,false,false,C::DeleteNode),
            (K::D,false,true,C::DuplicateNode),(K::L,true,false,C::TidyNetwork)] { add(key,ctrl,shift,false,cmd,X::Network); }
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
                if matches!(entry.command, Command::Points | Command::Edges | Command::Faces | Command::Object) { entry.chord.shift = true; }
                if entry.context == Context::Viewport && entry.chord.alt &&
                    matches!(entry.chord.key, Key::Num1 | Key::Num2 | Key::Num3 | Key::Num4 | Key::Num5 | Key::Num6 | Key::Num7 | Key::Num8 | Key::Plus | Key::Minus | Key::Period) {
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
        if kctx.viewport_hovered {
            for (key, tool) in [(Key::M, Tool::Measure), (Key::C, Tool::Cursor)] {
                if ctx.input(|i| i.key_pressed(key) && !i.modifiers.any()) { out.push(UiAction::Editor(EditorChange::Tool(tool))); }
            }
            for (key, mode) in [(Key::L, super::sketch::SketchMode::Line), (Key::P, super::sketch::SketchMode::Pen), (Key::G, super::sketch::SketchMode::Polygon)] {
                if ctx.input(|i| i.key_pressed(key) && !i.modifiers.any()) { out.push(UiAction::StartSketch(mode)); }
            }
            for (key, mode) in [(Key::R, super::sketch::SketchMode::Rectangle), (Key::C, super::sketch::SketchMode::Circle)] {
                if ctx.input(|i| i.key_pressed(key) && i.modifiers.shift && !i.modifiers.ctrl && !i.modifiers.alt) { out.push(UiAction::StartSketch(mode)); }
            }
        }
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

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn guide_keymap_matches_defaults() {
        let guide = include_str!("../../docs/NATIVE_EDITOR.md").replace("\r\n", "\n");
        let table = guide.split_once("<!-- keymap:start -->\n").unwrap().1.split_once("<!-- keymap:end -->").unwrap().0;
        assert_eq!(table, default_keymap_table());
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
    #[test] fn tool_shortcuts_are_viewport_bindings() {
        let map = Keymap::blender();
        for (key, tool) in [(Key::Q, Tool::Select), (Key::W, Tool::Move), (Key::E, Tool::Rotate),
            (Key::R, Tool::Scale), (Key::T, Tool::Transform)] {
            let binding = map.bindings.iter().find(|b| b.chord == Chord::new(key, false, false, false)).unwrap();
            assert_eq!(binding.context, Context::Viewport);
            assert!(matches!(binding.command.to_action(&KeyContext::default()),
                Some(UiAction::Editor(EditorChange::Tool(t))) if t == tool));
        }
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
    }
}
