//! Persisted inference preferences (Preferences > Viewport > Inference) and tool key bindings.
use serde_json::{json, Value};

/// Three linear inference modes, cycled with F or chosen in the chip/preferences.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LinearMode { #[default] All, PointsOnly, ParallelPerpendicularOnly }
impl LinearMode {
    pub const ALL: [Self; 3] = [Self::All, Self::PointsOnly, Self::ParallelPerpendicularOnly];
    pub fn label(self) -> &'static str { match self { Self::All => "All inferences", Self::PointsOnly => "Linear off", Self::ParallelPerpendicularOnly => "Parallel/Perp only" } }
    pub fn id(self) -> &'static str { match self { Self::All => "all", Self::PointsOnly => "points", Self::ParallelPerpendicularOnly => "parallel_perpendicular" } }
    pub fn next(self) -> Self { match self { Self::All => Self::PointsOnly, Self::PointsOnly => Self::ParallelPerpendicularOnly, Self::ParallelPerpendicularOnly => Self::All } }
    fn parse(s: &str) -> Self { Self::ALL.into_iter().find(|m| m.id() == s).unwrap_or_default() }
}

/// Inference categories. Tools declare the subset they support; settings can disable any.
pub mod cat {
    pub const VERTEX: u32 = 1; pub const MIDPOINT: u32 = 2; pub const EDGE: u32 = 4; pub const FACE: u32 = 8;
    pub const INTERSECTION: u32 = 16; pub const ORIGIN: u32 = 32; pub const AXIS: u32 = 64;
    pub const PARALLEL: u32 = 128; pub const PERPENDICULAR: u32 = 256; pub const EXTENSION: u32 = 512;
    pub const TRACKING: u32 = 1024; pub const CENTRE: u32 = 2048;
    pub const POINTS: u32 = VERTEX | MIDPOINT | CENTRE | EDGE | FACE | INTERSECTION | ORIGIN;
    pub const LINEAR: u32 = AXIS | PARALLEL | PERPENDICULAR | EXTENSION | TRACKING;
    pub const ALL: u32 = POINTS | LINEAR;
    pub const NAMES: [(u32, &str); 12] = [(VERTEX, "Endpoints"), (MIDPOINT, "Midpoints"), (CENTRE, "Curve centres"), (EDGE, "On edge"), (FACE, "On face"), (INTERSECTION, "Intersections"), (ORIGIN, "Origin"), (AXIS, "Axes"), (PARALLEL, "Parallel"), (PERPENDICULAR, "Perpendicular"), (EXTENSION, "Extension"), (TRACKING, "Reference tracking")];
}

pub type Rgba = [u8; 4];

#[derive(Clone, Debug, PartialEq)]
pub struct InferenceColors { pub x: Rgba, pub y: Rgba, pub z: Rgba, pub parallel: Rgba, pub endpoint: Rgba, pub midpoint: Rgba, pub intersection: Rgba, pub on: Rgba, pub reference: Rgba, pub free: Rgba }
impl Default for InferenceColors { fn default() -> Self { Self {
    x: [230, 40, 40, 255], y: [40, 190, 60, 255], z: [50, 90, 240, 255], parallel: [220, 40, 220, 255],
    endpoint: [30, 200, 60, 255], midpoint: [30, 200, 220, 255], intersection: [235, 235, 235, 255],
    on: [60, 120, 255, 255], reference: [255, 200, 40, 255], free: [200, 200, 200, 255],
} } }
impl InferenceColors {
    pub const FIELDS: [&'static str; 10] = ["x", "y", "z", "parallel", "endpoint", "midpoint", "intersection", "on", "reference", "free"];
    pub fn field_mut(&mut self, name: &str) -> Option<&mut Rgba> { Some(match name {
        "x" => &mut self.x, "y" => &mut self.y, "z" => &mut self.z, "parallel" => &mut self.parallel, "endpoint" => &mut self.endpoint,
        "midpoint" => &mut self.midpoint, "intersection" => &mut self.intersection, "on" => &mut self.on, "reference" => &mut self.reference, "free" => &mut self.free, _ => return None }) }
}

/// Tool keys while a sketch/line tool is active. Values are egui key names; the global keymap is
/// suspended while a sketch tool is active, so these never collide with viewport commands.
#[derive(Clone, Debug, PartialEq)]
pub struct InferenceKeys { pub lock_x: String, pub lock_y: String, pub lock_z: String, pub lock_inferred: String, pub cycle_candidate: String, pub cycle_linear_mode: String }
impl Default for InferenceKeys { fn default() -> Self { Self {
    lock_x: "ArrowRight".into(), lock_y: "ArrowLeft".into(), lock_z: "ArrowUp".into(), lock_inferred: "ArrowDown".into(), cycle_candidate: "Tab".into(), cycle_linear_mode: "F".into(),
} } }
impl InferenceKeys {
    /// `(input, action)` rows of the NATIVE_EDITOR.md inference table (fixed modifiers included).
    pub fn table_rows(&self) -> Vec<(String, &'static str)> { vec![
        (self.lock_x.clone(), "Lock X (red); again unlocks"),
        (self.lock_y.clone(), "Lock Y (green); again unlocks"),
        (self.lock_z.clone(), "Lock Z (blue); again unlocks"),
        (self.lock_inferred.clone(), "Lock the current parallel/perpendicular (else the inferred direction); again unlocks"),
        ("Alt (hold)".into(), "Lock the current inferred direction or face plane until released"),
        (self.cycle_candidate.clone(), "Cycle overlapping candidates under the cursor"),
        (self.cycle_linear_mode.clone(), "Cycle linear inference mode: all / linear off / parallel-perpendicular only"),
        ("Hover ~0.4 s".into(), "Acquire a point or edge as a tracking reference (max 4, oldest replaced)"),
        ("Digits, units, `x,y,z`, `@dx,dy,dz`, `len<deg`".into(), "Exact length along the current direction or lock, absolute / relative point, length at angle in the working plane"),
        ("Enter".into(), "Apply typed value; confirm all pending chains"),
        ("Escape".into(), "Cancel all pending chains and return to Select"),
    ] }
    pub fn to_json(&self) -> Value { json!({"lock_x": self.lock_x, "lock_y": self.lock_y, "lock_z": self.lock_z, "lock_inferred": self.lock_inferred, "cycle_candidate": self.cycle_candidate, "cycle_linear_mode": self.cycle_linear_mode}) }
    pub fn from_json(v: &Value) -> Self {
        let mut k = Self::default();
        for (name, slot) in [("lock_x", &mut k.lock_x), ("lock_y", &mut k.lock_y), ("lock_z", &mut k.lock_z), ("lock_inferred", &mut k.lock_inferred), ("cycle_candidate", &mut k.cycle_candidate), ("cycle_linear_mode", &mut k.cycle_linear_mode)] {
            if let Some(s) = v.get(name).and_then(Value::as_str).filter(|s| !s.is_empty()) { *slot = s.to_owned(); }
        }
        k
    }
    /// Keys bound to more than one action (reported in Preferences, first binding wins).
    pub fn conflicts(&self) -> Vec<String> {
        let all = [&self.lock_x, &self.lock_y, &self.lock_z, &self.lock_inferred, &self.cycle_candidate, &self.cycle_linear_mode];
        let mut out: Vec<String> = all.iter().enumerate().filter(|(i, k)| all[..*i].contains(k)).map(|(_, k)| (*k).clone()).collect();
        out.dedup(); out
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct InferenceSettings {
    pub enabled: u32,
    pub tolerance_px: f32,
    pub hysteresis_px: f32,
    pub hysteresis_ratio: f32,
    pub dwell_s: f64,
    pub tooltip_delay_s: f64,
    pub max_references: usize,
    pub marker_size: f32,
    pub show_tooltips: bool,
    pub include_hidden: bool,
    pub linear_mode: LinearMode,
    pub colors: InferenceColors,
    pub keys: InferenceKeys,
}
impl Default for InferenceSettings { fn default() -> Self { Self {
    enabled: cat::ALL, tolerance_px: 10., hysteresis_px: 2., hysteresis_ratio: 0.15, dwell_s: 0.4, tooltip_delay_s: 0.25,
    max_references: 4, marker_size: 6., show_tooltips: true, include_hidden: false, linear_mode: LinearMode::All,
    colors: InferenceColors::default(), keys: InferenceKeys::default(),
} } }
impl InferenceSettings {
    pub fn to_json(&self) -> Value {
        let mut colors = serde_json::Map::new();
        let mut c = self.colors.clone();
        for f in InferenceColors::FIELDS { colors.insert(f.into(), json!(*c.field_mut(f).unwrap())); }
        json!({"enabled": self.enabled, "tolerance_px": self.tolerance_px, "dwell_s": self.dwell_s, "tooltip_delay_s": self.tooltip_delay_s,
            "max_references": self.max_references, "marker_size": self.marker_size, "show_tooltips": self.show_tooltips,
            "include_hidden": self.include_hidden, "linear_mode": self.linear_mode.id(), "colors": colors, "keys": self.keys.to_json()})
    }
    pub fn from_json(v: &Value) -> Self {
        let mut s = Self::default();
        let f = |k: &str| v.get(k).and_then(Value::as_f64);
        if let Some(x) = v.get("enabled").and_then(Value::as_u64) { s.enabled = x as u32 & cat::ALL; }
        if let Some(x) = f("tolerance_px") { s.tolerance_px = (x as f32).clamp(2., 40.); }
        if let Some(x) = f("dwell_s") { s.dwell_s = x.clamp(0.05, 3.); }
        if let Some(x) = f("tooltip_delay_s") { s.tooltip_delay_s = x.clamp(0., 3.); }
        if let Some(x) = v.get("max_references").and_then(Value::as_u64) { s.max_references = (x as usize).clamp(1, 16); }
        if let Some(x) = f("marker_size") { s.marker_size = (x as f32).clamp(2., 24.); }
        if let Some(x) = v.get("show_tooltips").and_then(Value::as_bool) { s.show_tooltips = x; }
        if let Some(x) = v.get("include_hidden").and_then(Value::as_bool) { s.include_hidden = x; }
        if let Some(x) = v.get("linear_mode").and_then(Value::as_str) { s.linear_mode = LinearMode::parse(x); }
        if let Some(obj) = v.get("colors").and_then(Value::as_object) {
            for (k, c) in obj { if let (Some(slot), Some(a)) = (s.colors.field_mut(k), c.as_array()) {
                if a.len() == 4 { for i in 0..4 { slot[i] = a[i].as_u64().unwrap_or(255).min(255) as u8; } }
            } }
        }
        if let Some(k) = v.get("keys") { s.keys = InferenceKeys::from_json(k); }
        s
    }
}
