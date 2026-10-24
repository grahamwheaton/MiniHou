//! Editor-wide interaction state shared by the viewport, its popovers, the keymap and the renderer: selection mode,
//! transform orientation, pivot, snapping, proportional editing, overlays, X-ray and the current view preset.
//! The app owns one `EditorState`; panels read it through `View::editor` and change it with `UiAction::Editor(EditorChange)`.
//! The defaults and option lists follow Blender (the design guide's "copy blender").

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SelectionMode { #[default] Object, Points, Edges, Faces }
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Tool { #[default] Select, Cursor, Move, Rotate, Scale, Transform, Measure, Line, Polyline, Polygon, Rectangle, Circle, Knife, LoopCut, EdgeSlide }
impl Tool { pub const ALL: [Self; 15] = [Self::Select, Self::Cursor, Self::Move, Self::Rotate, Self::Scale, Self::Transform, Self::Measure, Self::Line, Self::Polyline, Self::Polygon, Self::Rectangle, Self::Circle, Self::Knife, Self::LoopCut, Self::EdgeSlide]; }
impl Tool {
    /// Own viewport clicks and modal keys as soon as the strip activates the tool.
    pub fn drawing(self) -> bool { matches!(self, Self::Line | Self::Polyline | Self::Polygon | Self::Rectangle | Self::Circle | Self::Knife | Self::LoopCut | Self::EdgeSlide) }
}
impl SelectionMode {
    pub const ALL: [SelectionMode; 4] = [Self::Points, Self::Edges, Self::Faces, Self::Object];
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
#[repr(u8)]
pub enum SnapTo { Increment, #[default] Vertex, Edge, Face, Volume, EdgeCenter, FaceCenter }
impl SnapTo {
    pub const ALL: [SnapTo; 7] = [Self::Increment, Self::Vertex, Self::Edge, Self::Face, Self::Volume, Self::EdgeCenter, Self::FaceCenter];
    pub fn label(self) -> &'static str {
        match self { Self::Increment => "Increment", Self::Vertex => "Vertex", Self::Edge => "Edge", Self::Face => "Face", Self::Volume => "Volume",
            Self::EdgeCenter => "Edge Center", Self::FaceCenter => "Face Center" }
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

/// Stable persisted bit positions follow `SnapTo::ALL`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SnapTargets(u8);
impl Default for SnapTargets { fn default() -> Self { SnapTo::Vertex.into() } }
impl From<SnapTo> for SnapTargets { fn from(to: SnapTo) -> Self { Self(1 << to as u8) } }
impl SnapTargets {
    pub fn bits(self) -> u8 { self.0 }
    pub fn from_bits(bits: u8) -> Self { let bits = bits & 127; if bits == 0 { Self::default() } else { Self(bits) } }
    pub fn contains(self, to: SnapTo) -> bool { self.0 & (1 << to as u8) != 0 }
    pub fn toggle(&mut self, to: SnapTo) { let bits = self.0 ^ (1 << to as u8); if bits != 0 { self.0 = bits; } }
    pub fn geometry(self) -> bool { self.0 & !1 != 0 }
    pub fn iter(self) -> impl Iterator<Item = SnapTo> { SnapTo::ALL.into_iter().filter(move |to| self.contains(*to)) }
    pub fn label(self) -> String { self.iter().map(SnapTo::label).collect::<Vec<_>>().join(" + ") }
}
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Snap { pub enabled: bool, pub to: SnapTargets, pub increment: f32, pub align_rotation: bool, pub backface_culling: bool }
impl Default for Snap { fn default() -> Self { Self { enabled: false, to: SnapTargets::default(), increment: 0.25, align_rotation: false, backface_culling: false } } }

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Proportional { pub enabled: bool, pub falloff: Falloff, pub radius: f32, pub connected_only: bool }
impl Default for Proportional { fn default() -> Self { Self { enabled: false, falloff: Falloff::Smooth, radius: 1.0, connected_only: false } } }

#[derive(Clone, Copy, PartialEq, Debug)]
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Overlays { pub enabled: bool, pub points: bool, pub lines: bool, pub faces: bool, pub point_size: f32, pub line_width: f32, pub grid: bool, pub axes: bool, pub wireframe: bool, pub stats: bool, pub normals: bool, pub bounds: bool, pub uv_preview: bool, pub uv_checker_cells: u32 }
impl Overlays { pub fn uv_preview_active(&self) -> bool { self.enabled && self.uv_preview } }
impl Default for Overlays { fn default() -> Self { Self { enabled: true, points: true, lines: true, faces: true, point_size: 4., line_width: 2., grid: true, axes: true, wireframe: true, stats: true, normals: false, bounds: false, uv_preview: false, uv_checker_cells: 16 } } }

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
pub struct Proxy { pub level: f32, pub auto: bool, pub link_tris: u32, pub clustered: bool, pub clip_start: f32, pub clip_end: f32 }
impl Default for Proxy { fn default() -> Self { Self { level: 1.0, auto: false, link_tris: 0, clustered: false, clip_start: 0.01, clip_end: 1000.0 } } }

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct EditorState {
    pub tool: Tool,
    pub cursor: [f32; 3],
    pub cursor_rotation: [f32; 3],
    pub selection: SelectionMode,
    pub orientation: Orientation,
    pub pivot: Pivot,
    pub snap: Snap,
    pub proportional: Proportional,
    pub overlays: Overlays,
    pub xray: bool,
    pub object_visibility: crate::viewport::ObjectVisibility,
    pub matcap: crate::viewport::MatCap,
    pub matcap_flat: bool,
    pub object_color: crate::viewport::ObjectColor,
    pub textures: bool,
    pub texture_inspector: bool,
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
    ObjectVisibility(crate::viewport::ObjectVisibility),
    MatCap(crate::viewport::MatCap),
    MatCapFlat(bool),
    ObjectColor(crate::viewport::ObjectColor),
    Textures(bool),
    TextureInspector(bool),
    View(ViewPreset),
    ProxyLevel(f32),
    ProxyAuto(bool),
    ProxyLinkTris(u32),
    ProxyClustered(bool),
    ClipPlanes(f32, f32),
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
            EditorChange::SnapTo(t) => self.snap.to.toggle(t),
            EditorChange::SnapIncrement(x) => self.snap.increment = x.max(1e-4),
            EditorChange::SnapAlignRotation(b) => self.snap.align_rotation = b,
            EditorChange::SnapBackfaceCulling(b) => self.snap.backface_culling = b,
            EditorChange::ProportionalEnabled(b) => self.proportional.enabled = b,
            EditorChange::ProportionalFalloff(f) => self.proportional.falloff = f,
            EditorChange::ProportionalRadius(r) => self.proportional.radius = r.max(1e-4),
            EditorChange::ProportionalConnected(b) => self.proportional.connected_only = b,
            EditorChange::Overlays(o) => self.overlays = o,
            EditorChange::Xray(b) => self.xray = b,
            EditorChange::ObjectVisibility(v) => self.object_visibility = v,
            EditorChange::MatCap(v) => self.matcap = v,
            EditorChange::MatCapFlat(v) => self.matcap_flat = v,
            EditorChange::ObjectColor(v) => self.object_color = v,
            EditorChange::Textures(v) => self.textures = v,
            EditorChange::TextureInspector(v) => self.texture_inspector = v,
            EditorChange::View(v) => self.view = v,
            EditorChange::ProxyLevel(l) => self.proxy.level = l.clamp(0.001, 1.0),
            EditorChange::ProxyAuto(a) => self.proxy.auto = a,
            EditorChange::ProxyLinkTris(n) => self.proxy.link_tris = n,
            EditorChange::ProxyClustered(b) => self.proxy.clustered = b,
            EditorChange::ClipPlanes(near, far) => (self.proxy.clip_start, self.proxy.clip_end) = minihou_app::sanitize_clip_planes(near, far),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_master_retains_individual_switches() {
        let mut s=EditorState::default(); let original=s.overlays;
        assert!(original.points && original.lines && original.faces);
        let mut o=original; o.enabled=false; s.apply(EditorChange::Overlays(o));
        assert!(!s.overlays.enabled); assert_eq!(s.overlays.points,original.points);
        o.enabled=true; s.apply(EditorChange::Overlays(o)); assert_eq!(s.overlays,original);
    }
    #[test]
    fn defaults_follow_blender() {
        let s = EditorState::default();
        assert_eq!(s.selection, SelectionMode::Object);
        assert_eq!(s.tool, Tool::Select);
        assert_eq!(s.orientation, Orientation::Global);
        assert_eq!(s.pivot, Pivot::MedianPoint);
        assert_eq!(s.snap.to, SnapTo::Vertex.into());
        assert!(!s.snap.enabled && !s.proportional.enabled && !s.xray);
        assert!(s.overlays.grid && s.overlays.axes && !s.overlays.normals && !s.overlays.bounds);
        assert_eq!((s.proxy.clip_start, s.proxy.clip_end), (0.01, 1000.0));
        assert_eq!(s.object_visibility, crate::viewport::ObjectVisibility::ShowAll);
    }

    #[test]
    fn changes_apply_and_radius_stays_positive() {
        let mut s = EditorState::default();
        s.apply(EditorChange::Selection(SelectionMode::Faces));
        s.apply(EditorChange::Tool(Tool::Move));
        s.apply(EditorChange::ProportionalRadius(-3.));
        s.apply(EditorChange::Xray(true));
        s.apply(EditorChange::ObjectVisibility(crate::viewport::ObjectVisibility::GhostOthers));
        assert_eq!(s.selection, SelectionMode::Faces);
        assert_eq!(s.tool, Tool::Move);
        assert!(s.proportional.radius > 0.);
        assert!(s.xray);
        assert_eq!(s.object_visibility, crate::viewport::ObjectVisibility::GhostOthers);
    }

    #[test]
    fn snap_targets_toggle_independently_and_keep_one_target() {
        let mut s=EditorState::default();
        s.apply(EditorChange::SnapTo(SnapTo::Edge));
        s.apply(EditorChange::SnapTo(SnapTo::Increment));
        assert_eq!(s.snap.to.iter().collect::<Vec<_>>(),vec![SnapTo::Increment,SnapTo::Vertex,SnapTo::Edge]);
        assert_eq!(SnapTargets::from_bits(s.snap.to.bits()),s.snap.to);
        s.apply(EditorChange::SnapTo(SnapTo::Vertex));
        s.apply(EditorChange::SnapTo(SnapTo::Increment));
        s.apply(EditorChange::SnapTo(SnapTo::Edge));
        assert_eq!(s.snap.to,SnapTo::Edge.into());
        assert_eq!(SnapTargets::from_bits(0),SnapTargets::default());
    }

    #[test]
    fn clip_planes_change_and_clamping() {
        let mut s = EditorState::default();
        assert_eq!((s.proxy.clip_start, s.proxy.clip_end), (0.01, 1000.0));
        s.apply(EditorChange::ClipPlanes(0.1, 5000.0));
        assert_eq!((s.proxy.clip_start, s.proxy.clip_end), (0.1, 5000.0));
        s.apply(EditorChange::ClipPlanes(-1.0, -2.0));
        assert!(s.proxy.clip_start > 0.0 && s.proxy.clip_end > s.proxy.clip_start);
    }
}
