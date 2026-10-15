//! Editor-wide interaction state shared by the viewport, its popovers, the keymap and the renderer: selection mode,
//! transform orientation, pivot, snapping, proportional editing, overlays, X-ray and the current view preset.
//! The app owns one `EditorState`; panels read it through `View::editor` and change it with `UiAction::Editor(EditorChange)`.
//! The defaults and option lists follow Blender (the design guide's "copy blender").

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SelectionMode { #[default] Object, Points, Edges, Faces }
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Tool { #[default] Select, Cursor, Move, Rotate, Scale, Transform, Measure }
impl Tool { pub const ALL: [Self; 7] = [Self::Select, Self::Cursor, Self::Move, Self::Rotate, Self::Scale, Self::Transform, Self::Measure]; }
impl SelectionMode {
    pub const ALL: [SelectionMode; 4] = [Self::Object, Self::Points, Self::Edges, Self::Faces];
    pub fn label(self) -> &'static str { match self { Self::Object => "Object", Self::Points => "Points", Self::Edges => "Edges", Self::Faces => "Faces" } }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Orientation { #[default] Global, Local, Normal, Gimbal, View, Cursor }
impl Orientation {
    pub const ALL: [Orientation; 6] = [Self::Global, Self::Local, Self::Normal, Self::Gimbal, Self::View, Self::Cursor];
    pub fn label(self) -> &'static str { match self { Self::Global => "Global", Self::Local => "Local", Self::Normal => "Normal", Self::Gimbal => "Gimbal", Self::View => "View", Self::Cursor => "Cursor" } }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Pivot { #[default] MedianPoint, BoundingBoxCenter, Cursor, IndividualOrigins, ActiveElement }
impl Pivot {
    pub const ALL: [Pivot; 5] = [Self::MedianPoint, Self::BoundingBoxCenter, Self::Cursor, Self::IndividualOrigins, Self::ActiveElement];
    pub fn label(self) -> &'static str {
        match self { Self::MedianPoint => "Median Point", Self::BoundingBoxCenter => "Bounding Box Center", Self::Cursor => "3D Cursor",
            Self::IndividualOrigins => "Individual Origins", Self::ActiveElement => "Active Element" }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SnapTo { #[default] Increment, Vertex, Edge, Face, Volume, EdgeCenter }
impl SnapTo {
    pub const ALL: [SnapTo; 6] = [Self::Increment, Self::Vertex, Self::Edge, Self::Face, Self::Volume, Self::EdgeCenter];
    pub fn label(self) -> &'static str {
        match self { Self::Increment => "Increment", Self::Vertex => "Vertex", Self::Edge => "Edge", Self::Face => "Face", Self::Volume => "Volume", Self::EdgeCenter => "Edge Center" }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Falloff { #[default] Smooth, Sphere, Root, InverseSquare, Sharp, Linear, Constant, Random }
impl Falloff {
    pub const ALL: [Falloff; 8] = [Self::Smooth, Self::Sphere, Self::Root, Self::InverseSquare, Self::Sharp, Self::Linear, Self::Constant, Self::Random];
    pub fn label(self) -> &'static str {
        match self { Self::Smooth => "Smooth", Self::Sphere => "Sphere", Self::Root => "Root", Self::InverseSquare => "Inverse Square",
            Self::Sharp => "Sharp", Self::Linear => "Linear", Self::Constant => "Constant", Self::Random => "Random" }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Snap { pub enabled: bool, pub to: SnapTo, pub increment: f32, pub align_rotation: bool, pub backface_culling: bool }
impl Default for Snap { fn default() -> Self { Self { enabled: false, to: SnapTo::Increment, increment: 0.25, align_rotation: false, backface_culling: false } } }

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Proportional { pub enabled: bool, pub falloff: Falloff, pub radius: f32, pub connected_only: bool }
impl Default for Proportional { fn default() -> Self { Self { enabled: false, falloff: Falloff::Smooth, radius: 1.0, connected_only: false } } }

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Overlays { pub grid: bool, pub axes: bool, pub wireframe: bool, pub stats: bool, pub normals: bool, pub bounds: bool }
impl Default for Overlays { fn default() -> Self { Self { grid: true, axes: true, wireframe: true, stats: true, normals: false, bounds: false } } }

/// Standard views (numpad keys in Blender).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ViewPreset { #[default] Perspective, Front, Back, Right, Left, Top, Bottom }
impl ViewPreset {
    pub const ORTHO: [ViewPreset; 6] = [Self::Front, Self::Back, Self::Right, Self::Left, Self::Top, Self::Bottom];
    pub fn label(self) -> &'static str {
        match self { Self::Perspective => "Perspective", Self::Front => "Front", Self::Back => "Back", Self::Right => "Right", Self::Left => "Left", Self::Top => "Top", Self::Bottom => "Bottom" }
    }
}

/// Viewport/cook proxy: show (and later cook) a lighter version of a very large scene. Mirrors the old editor's
/// `/api/proxy` (level 1 = full, 0.5 = half ... ; `auto` picks the level from the triangle count; `link_tris` caps what is
/// sent to Blender, 0 = unlimited).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Proxy { pub level: f32, pub auto: bool, pub link_tris: u32, pub clustered: bool, pub view_distance: f32 }
impl Default for Proxy { fn default() -> Self { Self { level: 1.0, auto: false, link_tris: 0, clustered: false, view_distance: 20.0 } } }

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct EditorState {
    pub tool: Tool,
    pub cursor: [f32; 3],
    pub selection: SelectionMode,
    pub orientation: Orientation,
    pub pivot: Pivot,
    pub snap: Snap,
    pub proportional: Proportional,
    pub overlays: Overlays,
    pub xray: bool,
    /// Last standard view selected (the camera itself belongs to the app).
    pub view: ViewPreset,
    pub proxy: Proxy,
}

/// One change to the state, as a panel or the keymap asks for it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EditorChange {
    Tool(Tool),
    Cursor([f32; 3]),
    Selection(SelectionMode),
    Orientation(Orientation),
    Pivot(Pivot),
    SnapEnabled(bool),
    SnapTo(SnapTo),
    SnapIncrement(f32),
    SnapAlignRotation(bool),
    SnapBackfaceCulling(bool),
    ProportionalEnabled(bool),
    ProportionalFalloff(Falloff),
    ProportionalRadius(f32),
    ProportionalConnected(bool),
    Overlays(Overlays),
    Xray(bool),
    View(ViewPreset),
    ProxyLevel(f32),
    ProxyAuto(bool),
    ProxyLinkTris(u32),
    ProxyClustered(bool),
    ViewDistance(f32),
}

impl EditorState {
    pub fn apply(&mut self, change: EditorChange) {
        match change {
            EditorChange::Tool(t) => self.tool = t,
            EditorChange::Cursor(p) => self.cursor = p,
            EditorChange::Selection(m) => self.selection = m,
            EditorChange::Orientation(o) => self.orientation = o,
            EditorChange::Pivot(p) => self.pivot = p,
            EditorChange::SnapEnabled(b) => self.snap.enabled = b,
            EditorChange::SnapTo(t) => self.snap.to = t,
            EditorChange::SnapIncrement(x) => self.snap.increment = x.max(1e-4),
            EditorChange::SnapAlignRotation(b) => self.snap.align_rotation = b,
            EditorChange::SnapBackfaceCulling(b) => self.snap.backface_culling = b,
            EditorChange::ProportionalEnabled(b) => self.proportional.enabled = b,
            EditorChange::ProportionalFalloff(f) => self.proportional.falloff = f,
            EditorChange::ProportionalRadius(r) => self.proportional.radius = r.max(1e-4),
            EditorChange::ProportionalConnected(b) => self.proportional.connected_only = b,
            EditorChange::Overlays(o) => self.overlays = o,
            EditorChange::Xray(b) => self.xray = b,
            EditorChange::View(v) => self.view = v,
            EditorChange::ProxyLevel(l) => self.proxy.level = l.clamp(0.001, 1.0),
            EditorChange::ProxyAuto(a) => self.proxy.auto = a,
            EditorChange::ProxyLinkTris(n) => self.proxy.link_tris = n,
            EditorChange::ProxyClustered(b) => self.proxy.clustered = b,
            EditorChange::ViewDistance(d) => self.proxy.view_distance = d.clamp(3.0, 1000.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_follow_blender() {
        let s = EditorState::default();
        assert_eq!(s.selection, SelectionMode::Object);
        assert_eq!(s.tool, Tool::Select);
        assert_eq!(s.orientation, Orientation::Global);
        assert_eq!(s.pivot, Pivot::MedianPoint);
        assert!(!s.snap.enabled && !s.proportional.enabled && !s.xray);
        assert!(s.overlays.grid && s.overlays.axes && !s.overlays.normals && !s.overlays.bounds);
        assert_eq!(s.proxy.view_distance, 20.0);
    }

    #[test]
    fn changes_apply_and_radius_stays_positive() {
        let mut s = EditorState::default();
        s.apply(EditorChange::Selection(SelectionMode::Faces));
        s.apply(EditorChange::Tool(Tool::Move));
        s.apply(EditorChange::ProportionalRadius(-3.));
        s.apply(EditorChange::Xray(true));
        assert_eq!(s.selection, SelectionMode::Faces);
        assert_eq!(s.tool, Tool::Move);
        assert!(s.proportional.radius > 0.);
        assert!(s.xray);
    }

    #[test]
    fn view_distance_change_and_clamping() {
        let mut s = EditorState::default();
        assert_eq!(s.proxy.view_distance, 20.0);
        s.apply(EditorChange::ViewDistance(100.0));
        assert_eq!(s.proxy.view_distance, 100.0);
        s.apply(EditorChange::ViewDistance(1.0));
        assert_eq!(s.proxy.view_distance, 3.0);
        s.apply(EditorChange::ViewDistance(2000.0));
        assert_eq!(s.proxy.view_distance, 1000.0);
    }
}
