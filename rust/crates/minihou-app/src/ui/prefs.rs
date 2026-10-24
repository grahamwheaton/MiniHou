//! User preferences persisted independently of the document.
use serde_json::{json, Value};
use std::{fs, io, path::{Path, PathBuf}};

use crate::units::{LengthUnit, UnitSettings};

pub const SCHEMA: u32 = 1;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemePref { Dark, Light, System }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Navigation { Blender, Houdini }
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PackedInstances { #[default] Full, Auto }
impl PackedInstances {
    pub const ALL: [Self; 2] = [Self::Full, Self::Auto];
    pub fn label(self) -> &'static str { match self { Self::Full => "Full", Self::Auto => "Auto" } }
    fn parse(s: &str) -> Self { if s == "Auto" { Self::Auto } else { Self::Full } }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PackedProxyStyle { Solid, #[default] Wireframe }
impl PackedProxyStyle {
    pub const ALL: [Self; 2] = [Self::Solid, Self::Wireframe];
    pub fn label(self) -> &'static str { match self { Self::Solid => "Solid", Self::Wireframe => "Wireframe" } }
    fn parse(s: &str) -> Self { if s == "Solid" { Self::Solid } else { Self::Wireframe } }
}
impl ThemePref {
    pub const ALL: [Self; 3] = [Self::Dark, Self::Light, Self::System];
    pub fn label(self) -> &'static str { match self { Self::Dark => "Dark", Self::Light => "Light", Self::System => "System" } }
}
#[derive(Clone, Debug, PartialEq)]
pub struct ViewportPreferences { pub overlays: crate::ui::Overlays, pub xray: bool, pub default_shading: String, pub object_visibility: crate::viewport::ObjectVisibility, pub matcap:crate::viewport::MatCap, pub matcap_flat:bool, pub object_color:crate::viewport::ObjectColor, pub textures:bool, pub navigation: Navigation, pub fly_navigation: bool, pub zoom_to_mouse: bool, pub orbit_to_mouse: bool, pub show_grid: bool, pub show_axes: bool, pub auto_budget: u32, pub packed_instances: PackedInstances, pub packed_full_percent: u8, pub packed_proxy_style: PackedProxyStyle, pub clip_start: f32, pub clip_end: f32 }
impl Default for ViewportPreferences { fn default() -> Self { Self { overlays: Default::default(), xray: false, default_shading: "Smooth Shaded".into(), object_visibility: crate::viewport::ObjectVisibility::ShowAll, matcap:Default::default(), matcap_flat:false, object_color:Default::default(), textures:true, navigation: Navigation::Blender, fly_navigation: true, zoom_to_mouse:true, orbit_to_mouse:true, show_grid: true, show_axes: true, auto_budget: 400_000, packed_instances: PackedInstances::Full, packed_full_percent: 50, packed_proxy_style: PackedProxyStyle::Wireframe, clip_start: 0.01, clip_end: 1000.0 } } }
#[derive(Clone, Debug, PartialEq)]
pub struct Preferences {
    pub theme: ThemePref, pub ui_scale: f32, pub emulate_numpad: bool, pub snap_increment: f32,
    pub snap_targets: crate::ui::SnapTargets,
    pub show_stats: bool, pub autosave_minutes: Option<u32>, pub keymap_overrides: Value,
    pub show_parameter_links: bool,
    pub recent_files: Vec<PathBuf>, pub viewport: ViewportPreferences,
    pub units: UnitSettings,
    pub node_physics_snappiness: f32,
    /// User-adjustable performance budgets. Copy memory is stored in MiB for the existing copy node reader.
    pub copy_memory_limit_mib: u64,
    pub loop_iteration_budget: usize,
    pub solver_steps: usize,
    pub solver_substeps: usize,
    pub viewport_normal_lines: usize,
    /// Security: run `python` nodes (trusted code, user privileges). Off by default; only this window changes it.
    pub allow_python_nodes: bool,
    /// Wall-clock limit for one python node cook, in seconds.
    pub python_timeout_s: u32,
    /// Viewport inferencing (snap tolerance, markers, colours, tool keys); see docs/RUST_INFERENCE.md.
    pub inference: minihou_app::inference::InferenceSettings,
    /// The live panel layout.
    pub ui_layout: crate::ui::layout::UiLayout,
    /// The one custom slot written by "Save UI Layout" and read by "Load UI Layout".
    pub ui_layout_custom: Option<crate::ui::layout::UiLayout>,
}
impl Default for Preferences { fn default() -> Self {
    let limits = minihou_core::limits::Limits::default();
    Self {
    copy_memory_limit_mib: limits.copy_memory_bytes / (1024 * 1024),
    loop_iteration_budget: limits.loop_iterations,
    solver_steps: limits.goal_solver_steps,
    solver_substeps: limits.rbd_substeps,
    viewport_normal_lines: limits.normal_lines,
    theme: ThemePref::System, ui_scale: 1., emulate_numpad: false, snap_increment: 0.25,
    snap_targets: crate::ui::SnapTargets::default(),
    show_stats: true, show_parameter_links: true, autosave_minutes: Some(5), keymap_overrides: json!({"bindings": []}),
    recent_files: Vec::new(), viewport: ViewportPreferences::default(),
    units: UnitSettings::default(),
    node_physics_snappiness: 0.7,
    allow_python_nodes: false, python_timeout_s: 30,
    inference: Default::default(),
    ui_layout: Default::default(), ui_layout_custom: None,
} } }
impl Preferences {
    pub fn path() -> PathBuf {
        std::env::var_os("APPDATA").filter(|s| !s.is_empty()).map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(".")).join("MiniHou").join("preferences.json")
    }
    pub fn load() -> Self { Self::load_from(&Self::path()) }
    fn load_from(path: &Path) -> Self {
        let Ok(data) = fs::read(path) else { return Self::default(); };
        match serde_json::from_slice::<Value>(&data) {
            Ok(value) if value.is_object() => Self::from_json(&value),
            _ => { let _ = fs::rename(path, path.with_extension("json.bak")); Self::default() }
        }
    }
    pub fn save(&self) -> io::Result<()> { self.save_to(&Self::path()) }
    fn save_to(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() { fs::create_dir_all(parent)?; }
        let temp = path.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(&self.to_json()).map_err(io::Error::other)?;
        fs::write(&temp, bytes)?;
        fs::rename(temp, path)
    }
    fn to_json(&self) -> Value { json!({
        "schema": SCHEMA, "theme": self.theme.label(), "ui_scale": self.ui_scale,
        "emulate_numpad": self.emulate_numpad, "snap_increment": self.snap_increment, "show_parameter_links": self.show_parameter_links,
        "snap_targets": self.snap_targets.bits(),
        "node_physics_snappiness": self.node_physics_snappiness,
        "copy_memory_limit_mib": self.copy_memory_limit_mib,
        "loop_iteration_budget": self.loop_iteration_budget,
        "solver_steps": self.solver_steps,
        "solver_substeps": self.solver_substeps,
        "viewport_normal_lines": self.viewport_normal_lines,
        "show_stats": self.show_stats, "autosave_minutes": self.autosave_minutes,
        "keymap_overrides": self.keymap_overrides,
        "recent_files": self.recent_files.iter().take(10).collect::<Vec<_>>(),
        "viewport": { "overlays": self.viewport.overlays, "xray": self.viewport.xray, "default_shading": self.viewport.default_shading,
            "object_visibility": self.viewport.object_visibility.label(),
            "matcap": self.viewport.matcap.label(), "matcap_flat":self.viewport.matcap_flat, "object_color": self.viewport.object_color.label(), "textures":self.viewport.textures,
            "fly_navigation": self.viewport.fly_navigation,
            "zoom_to_mouse": self.viewport.zoom_to_mouse,
            "orbit_to_mouse": self.viewport.orbit_to_mouse,
            "navigation": match self.viewport.navigation { Navigation::Blender => "blender", Navigation::Houdini => "houdini" },
            "show_grid": self.viewport.show_grid, "show_axes": self.viewport.show_axes, "auto_budget": self.viewport.auto_budget,
            "packed_instances": self.viewport.packed_instances.label(), "packed_full_percent": self.viewport.packed_full_percent,
            "packed_proxy_style": self.viewport.packed_proxy_style.label(),
            "clip_start": self.viewport.clip_start, "clip_end": self.viewport.clip_end },
        "inference": self.inference.to_json(),
        "ui_layout": self.ui_layout.to_json(),
        "ui_layout_custom": self.ui_layout_custom.as_ref().map(crate::ui::layout::UiLayout::to_json),
        "security": { "allow_python_nodes": self.allow_python_nodes, "python_timeout_s": self.python_timeout_s },
        "length_unit": self.units.display.symbol(),
        "unit_scale": self.units.unit_scale,
        "units": { "length_unit": self.units.display.symbol(), "unit_scale": self.units.unit_scale },
    }) }
    fn from_json(v: &Value) -> Self {
        let mut p = Self { theme: match v.get("theme").and_then(Value::as_str) {
            Some("Dark") => ThemePref::Dark, Some("Light") => ThemePref::Light, _ => ThemePref::System }, ..Self::default() };
        if let Some(x) = v.get("ui_scale").and_then(Value::as_f64) { p.ui_scale = (x as f32).clamp(0.5, 3.); }
        if let Some(x) = v.get("emulate_numpad").and_then(Value::as_bool) { p.emulate_numpad = x; }
        if let Some(x) = v.get("show_parameter_links").and_then(Value::as_bool) { p.show_parameter_links = x; }
        if let Some(x) = v.get("snap_increment").and_then(Value::as_f64) { p.snap_increment = (x as f32).max(0.0001); }
        if let Some(x) = v.get("snap_targets").and_then(Value::as_u64) { p.snap_targets = crate::ui::SnapTargets::from_bits((x & 63) as u8); }
        if let Some(x) = v.get("node_physics_snappiness").and_then(Value::as_f64) { p.node_physics_snappiness = (x as f32).clamp(0.0, 1.0); }
        if let Some(x) = v.get("copy_memory_limit_mib").and_then(Value::as_u64).filter(|n| (16..=16384).contains(n)) { p.copy_memory_limit_mib = x; }
        if let Some(x) = v.get("loop_iteration_budget").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()).filter(|n| *n > 0) { p.loop_iteration_budget = x; }
        if let Some(x) = v.get("solver_steps").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()).filter(|n| *n > 0) { p.solver_steps = x; }
        if let Some(x) = v.get("solver_substeps").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()).filter(|n| *n > 0) { p.solver_substeps = x; }
        if let Some(x) = v.get("viewport_normal_lines").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()).filter(|n| *n > 0) { p.viewport_normal_lines = x; }
        if let Some(x) = v.get("show_stats").and_then(Value::as_bool) { p.show_stats = x; }
        if let Some(x) = v.get("ui_layout") { p.ui_layout = crate::ui::layout::UiLayout::from_json(x); }
        p.ui_layout_custom = v.get("ui_layout_custom").and_then(crate::ui::layout::UiLayout::try_from_json);
        if let Some(x) = v.get("inference") { p.inference = minihou_app::inference::InferenceSettings::from_json(x); }
        if let Some(sec) = v.get("security") {
            // Only a literal `true` enables Python; anything else keeps it off.
            p.allow_python_nodes = sec.get("allow_python_nodes") == Some(&Value::Bool(true));
            if let Some(t) = sec.get("python_timeout_s").and_then(Value::as_u64) { p.python_timeout_s = t.clamp(1, 3600) as u32; }
        }
        if let Some(x) = v.get("autosave_minutes") { p.autosave_minutes = x.as_u64().and_then(|n| u32::try_from(n).ok()).filter(|n| *n > 0); }
        if let Some(x) = v.get("keymap_overrides").filter(|x| x.is_object()) { p.keymap_overrides = x.clone(); }
        if let Some(xs) = v.get("recent_files").and_then(Value::as_array) {
            p.recent_files = xs.iter().filter_map(Value::as_str).take(10).map(PathBuf::from).collect();
        }
        if let Some(view) = v.get("viewport") {
            if let Some(value)=view.get("overlays") { if let Ok(mut o)=serde_json::from_value::<crate::ui::Overlays>(value.clone()) {
                o.point_size=o.point_size.clamp(1.,16.); o.line_width=o.line_width.clamp(1.,10.); o.uv_checker_cells=o.uv_checker_cells.clamp(1,256); p.viewport.show_grid=o.grid; p.viewport.show_axes=o.axes; p.show_stats=o.stats; p.viewport.overlays=o;
            } }
            if let Some(x)=view.get("xray").and_then(Value::as_bool) { p.viewport.xray=x; }
            if let Some(x)=view.get("object_visibility").and_then(Value::as_str) { p.viewport.object_visibility=crate::viewport::ObjectVisibility::parse(x); }
            if let Some(x)=view.get("matcap").and_then(Value::as_str) { p.viewport.matcap=crate::viewport::MatCap::parse(x); }
            if let Some(x)=view.get("matcap_flat").and_then(Value::as_bool) { p.viewport.matcap_flat=x; }
            if let Some(x)=view.get("object_color").and_then(Value::as_str) { p.viewport.object_color=crate::viewport::ObjectColor::parse(x); }
            if let Some(x)=view.get("textures").and_then(Value::as_bool) {p.viewport.textures=x;}
            if let Some(x) = view.get("fly_navigation").and_then(Value::as_bool) { p.viewport.fly_navigation = x; }
            if let Some(x) = view.get("zoom_to_mouse").and_then(Value::as_bool) { p.viewport.zoom_to_mouse = x; }
            if let Some(x) = view.get("orbit_to_mouse").and_then(Value::as_bool) { p.viewport.orbit_to_mouse = x; }
            if let Some(x) = view.get("navigation").and_then(Value::as_str) { p.viewport.navigation = if x == "houdini" { Navigation::Houdini } else { Navigation::Blender }; }
            if let Some(x) = view.get("default_shading").and_then(Value::as_str) { p.viewport.default_shading = crate::viewport::Mode::parse(x).label().to_owned(); }
            if let Some(x) = view.get("show_grid").and_then(Value::as_bool) { p.viewport.show_grid = x; }
            if let Some(x) = view.get("show_axes").and_then(Value::as_bool) { p.viewport.show_axes = x; }
            if let Some(x) = view.get("auto_budget").and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok()) { p.viewport.auto_budget = x.max(1); }
            if let Some(x) = view.get("packed_instances").and_then(Value::as_str) { p.viewport.packed_instances = PackedInstances::parse(x); }
            if let Some(x) = view.get("packed_full_percent").and_then(Value::as_u64) { p.viewport.packed_full_percent = x.min(100) as u8; }
            if let Some(x) = view.get("packed_proxy_style").and_then(Value::as_str) { p.viewport.packed_proxy_style = PackedProxyStyle::parse(x); }
            // Legacy view_distance was a scene-radius multiplier and cannot be converted without a scene.
            let near = view.get("clip_start").and_then(Value::as_f64).map_or(p.viewport.clip_start, |x| x as f32);
            let far = view.get("clip_end").and_then(Value::as_f64).map_or(p.viewport.clip_end, |x| x as f32);
            (p.viewport.clip_start, p.viewport.clip_end) = minihou_app::sanitize_clip_planes(near, far);
        }
        // Older preference files only stored these three switches.
        if v.get("viewport").and_then(|view|view.get("overlays")).is_none() {
            p.viewport.overlays.grid=p.viewport.show_grid; p.viewport.overlays.axes=p.viewport.show_axes;
            p.viewport.overlays.stats=p.show_stats;
        }
        else {
            p.viewport.show_grid=p.viewport.overlays.grid; p.viewport.show_axes=p.viewport.overlays.axes;
            p.show_stats=p.viewport.overlays.stats;
        }
        let unit_sym = v.get("length_unit").or_else(|| v.get("units").and_then(|u| u.get("length_unit"))).and_then(Value::as_str);
        if let Some(u) = unit_sym.and_then(LengthUnit::parse_symbol) { p.units.display = u; }
        let scale_val = v.get("unit_scale").or_else(|| v.get("units").and_then(|u| u.get("unit_scale"))).and_then(Value::as_f64);
        if let Some(scale) = scale_val { if (1e-9..=1e9).contains(&scale) { p.units.unit_scale = scale; } }
        p
    }
    /// Push the Security settings into the cook's Python gate (the only place the app flips it).
    pub fn apply_python(&self, gate: &minihou_core::python_bridge::PythonGate) {
        gate.set_allowed(self.allow_python_nodes);
        gate.set_timeout(std::time::Duration::from_secs(u64::from(self.python_timeout_s.clamp(1, 3600))));
    }
    /// "Save UI Layout": the live layout becomes the custom slot.
    pub fn save_ui_layout(&mut self) { self.ui_layout_custom = Some(self.ui_layout.clone()); }
    /// "Load UI Layout": the custom slot becomes the live layout. False when nothing was saved.
    pub fn load_ui_layout(&mut self) -> bool {
        match &self.ui_layout_custom { Some(layout) => { self.ui_layout = layout.clone(); true } None => false }
    }
    /// "Reset UI to Default": the built-in layout; the custom slot is kept.
    pub fn reset_ui_layout(&mut self) { self.ui_layout = crate::ui::layout::UiLayout::default(); }
    pub fn add_recent(&mut self, path: PathBuf) {
        self.recent_files.retain(|p| p != &path);
        self.recent_files.insert(0, path); self.recent_files.truncate(10);
    }
}

#[cfg(test)]
mod packed_display_tests {
    use super::*;

    #[test]
    fn uv_preview_defaults_legacy_round_trip_and_master_toggle() {
        let old = Preferences::from_json(&json!({"viewport":{"overlays":{"enabled":true,"points":false}}}));
        assert!(!old.viewport.overlays.uv_preview);
        assert_eq!(old.viewport.overlays.uv_checker_cells, 16);
        let mut p = Preferences::default();
        p.viewport.overlays.uv_preview = true;
        p.viewport.overlays.uv_checker_cells = 42;
        let mut loaded = Preferences::from_json(&p.to_json());
        assert_eq!(loaded.viewport.overlays, p.viewport.overlays);
        loaded.viewport.overlays.enabled = false;
        assert!(!loaded.viewport.overlays.uv_preview_active());
        assert!(loaded.viewport.overlays.uv_preview);
        loaded.viewport.overlays.enabled = true;
        assert!(loaded.viewport.overlays.uv_preview_active());
        for (input, expected) in [(0, 1), (1000, 256)] {
            let mut value = p.to_json();
            value["viewport"]["overlays"]["uv_checker_cells"] = json!(input);
            assert_eq!(Preferences::from_json(&value).viewport.overlays.uv_checker_cells, expected);
        }
    }

    #[test]
    fn packed_display_defaults_and_round_trip() {
        let mut prefs = Preferences::default();
        assert_eq!(prefs.viewport.packed_instances, PackedInstances::Full);
        assert_eq!(prefs.viewport.packed_full_percent, 50);
        assert_eq!(prefs.viewport.packed_proxy_style, PackedProxyStyle::Wireframe);
        prefs.viewport.packed_instances = PackedInstances::Auto;
        prefs.viewport.packed_full_percent = 25;
        prefs.viewport.packed_proxy_style = PackedProxyStyle::Solid;
        let restored = Preferences::from_json(&prefs.to_json());
        assert_eq!(restored.viewport.packed_instances, PackedInstances::Auto);
        assert_eq!(restored.viewport.packed_full_percent, 25);
        assert_eq!(restored.viewport.packed_proxy_style, PackedProxyStyle::Solid);
    }
}
#[cfg(test)] mod tests {
    use super::*;
    #[test] fn display_preferences_round_trip_and_load_legacy_names() {
        use crate::viewport::{Mode,ObjectVisibility};
        for mode in Mode::ALL { for visibility in ObjectVisibility::ALL {
            let p=Preferences::from_json(&json!({"viewport":{"default_shading":mode.label(),"object_visibility":visibility.label()}}));
            assert_eq!(p.viewport.default_shading,mode.label()); assert_eq!(p.viewport.object_visibility,visibility);
            assert_eq!(Preferences::from_json(&p.to_json()),p);
        } }
        for (old,mode) in [("Solid",Mode::FlatShaded),("Shaded",Mode::SmoothShaded),("Wire",Mode::Wireframe)] {
            let p=Preferences::from_json(&json!({"viewport":{"default_shading":old}}));
            assert_eq!(p.viewport.default_shading,mode.label()); assert_eq!(p.viewport.object_visibility,ObjectVisibility::ShowAll);
        }
        for removed in ["Shaded Bounding Box", "MatCap Wire Shaded", "Flat Wire Shaded", "Smooth Wire Shaded", "ShadedWire"] {
            let p=Preferences::from_json(&json!({"viewport":{"default_shading":removed}}));
            assert_eq!(p.viewport.default_shading,Mode::SmoothShaded.label());
        }
        assert_eq!(Preferences::from_json(&json!({"viewport":{"object_visibility":"future mode"}})).viewport.object_visibility,ObjectVisibility::ShowAll);
    }
    #[test] fn geometry_overlays_round_trip() {
        let mut p=Preferences::default();
        p.viewport.overlays=crate::ui::Overlays {enabled:false,points:false,lines:false,faces:false,point_size:9.,line_width:5.,normals:true,bounds:true,..Default::default()};
        p.viewport.xray=true;
        assert_eq!(Preferences::from_json(&p.to_json()),p);
        let partial=Preferences::from_json(&json!({"viewport":{"overlays":{"points":false,"point_size":999.,"line_width":-5.}}}));
        assert!(!partial.viewport.overlays.points && partial.viewport.overlays.lines && partial.viewport.overlays.faces);
        assert_eq!((partial.viewport.overlays.point_size,partial.viewport.overlays.line_width),(16.,1.));
    }
    #[test] fn matcap_and_object_color_round_trip() {
        let mut p=Preferences::default();
        p.viewport.matcap=crate::viewport::MatCap::SketchUpClean;
        p.viewport.matcap_flat=true;
        p.viewport.object_color=crate::viewport::ObjectColor::Random;
        assert_eq!(p.to_json()["viewport"]["matcap"],"SketchUp clean");
        assert_eq!(Preferences::from_json(&p.to_json()),p);
        assert_eq!(Preferences::from_json(&json!({})).viewport.object_color,crate::viewport::ObjectColor::Single);
    }
    #[test] fn fly_defaults_on_and_round_trips_disabled() {
        assert!(Preferences::from_json(&json!({})).viewport.fly_navigation);
        let p = Preferences::from_json(&json!({"viewport":{"fly_navigation":false}}));
        assert!(!p.viewport.fly_navigation);
        assert_eq!(Preferences::from_json(&p.to_json()), p);
    }
    #[test] fn zoom_to_mouse_defaults_on_and_round_trips() {
        let mut p = Preferences::default();
        assert!(p.viewport.zoom_to_mouse);
        p.viewport.zoom_to_mouse = false;
        assert_eq!(Preferences::from_json(&p.to_json()), p);
    }
    #[test] fn orbit_to_mouse_defaults_on_and_round_trips() {
        let mut p=Preferences::default();
        assert!(p.viewport.orbit_to_mouse);
        p.viewport.orbit_to_mouse=false;
        assert_eq!(Preferences::from_json(&p.to_json()),p);
    }
    #[test] fn node_physics_snappiness_defaults_clamps_and_round_trips() {
        assert_eq!(Preferences::from_json(&json!({})).node_physics_snappiness, 0.7);
        for (input, expected) in [(-1.0, 0.0), (0.3, 0.3), (2.0, 1.0)] {
            let p = Preferences::from_json(&json!({"node_physics_snappiness": input}));
            assert_eq!(p.node_physics_snappiness, expected);
            assert_eq!(Preferences::from_json(&p.to_json()).node_physics_snappiness, expected);
        }
    }
    fn test_path(name: &str) -> PathBuf { std::env::temp_dir().join(format!("minihou-{name}-{}",std::process::id())).join("preferences.json") }
    #[test] fn performance_defaults_round_trip_through_save_and_load() {
        let path = test_path("prefs-performance");
        let defaults = minihou_core::limits::Limits::default();
        let p = Preferences::default();
        assert_eq!(p.copy_memory_limit_mib, defaults.copy_memory_bytes / (1024 * 1024));
        assert_eq!(p.loop_iteration_budget, defaults.loop_iterations);
        assert_eq!(p.solver_steps, defaults.goal_solver_steps);
        assert_eq!(p.solver_substeps, defaults.rbd_substeps);
        assert_eq!(p.viewport_normal_lines, defaults.normal_lines);
        p.save_to(&path).unwrap();
        assert_eq!(Preferences::load_from(&path), p);

        let mut custom = p;
        custom.copy_memory_limit_mib = 512;
        custom.loop_iteration_budget = 20_000;
        custom.solver_steps = 2_000;
        custom.solver_substeps = 64;
        custom.viewport_normal_lines = 30_000;
        custom.save_to(&path).unwrap();
        assert_eq!(Preferences::load_from(&path), custom);
        let _ = fs::remove_file(path);
    }
    #[test] fn round_trip() {
        let path = test_path("prefs-roundtrip");
        let mut p = Preferences { theme: ThemePref::Light, ui_scale: 1.25, recent_files: vec![PathBuf::from("a.hou")], ..Default::default() };
        p.viewport.auto_budget = 250_000;
        p.snap_targets.toggle(crate::ui::SnapTo::Edge);
        p.snap_targets.toggle(crate::ui::SnapTo::Face);
        p.viewport.clip_start = 0.2;
        p.viewport.clip_end = 5000.0;
        p.units = UnitSettings { display: LengthUnit::Cm, unit_scale: 0.1 };
        p.save_to(&path).unwrap(); p.save_to(&path).unwrap(); assert_eq!(Preferences::load_from(&path),p);
        let _ = fs::remove_file(path);
    }
    #[test] fn ui_layout_custom_slot_persists_through_save_and_load() {
        use crate::ui::layout::{PanelId, UiLayout};
        let path = test_path("prefs-ui-layout");
        let mut p = Preferences::default();
        assert_eq!(p.ui_layout_custom, None);
        let mut custom = UiLayout::default(); custom.hidden.push(PanelId::GeoInfo);
        p.ui_layout_custom = Some(custom.clone());
        p.save_to(&path).unwrap();
        let loaded = Preferences::load_from(&path);
        assert_eq!(loaded.ui_layout_custom, Some(custom));
        assert_eq!(loaded, p);
        // An unreadable slot is dropped and an unreadable live layout becomes the default.
        let junk = Preferences::from_json(&json!({"ui_layout": {"version": 99}, "ui_layout_custom": {"version": 99}}));
        assert_eq!((junk.ui_layout, junk.ui_layout_custom), (UiLayout::default(), None));
        let _ = fs::remove_file(path);
    }
    #[test] fn save_and_load_ui_layout_actions_use_the_custom_slot() {
        use crate::ui::layout::{PanelId, UiLayout};
        let mut p = Preferences::default();
        assert!(!p.load_ui_layout(), "nothing saved yet");
        assert_eq!(p.ui_layout, UiLayout::default());
        p.ui_layout.hidden.push(PanelId::Spreadsheet);
        let saved = p.ui_layout.clone();
        p.save_ui_layout();
        p.ui_layout = UiLayout::default();
        assert!(p.load_ui_layout());
        assert_eq!(p.ui_layout, saved);
        let back = Preferences::from_json(&p.to_json());
        assert_eq!(back.ui_layout_custom, Some(saved));
    }
    #[test] fn reset_ui_layout_returns_the_default_and_keeps_the_slot() {
        use crate::ui::layout::{PanelId, UiLayout};
        let path = test_path("prefs-ui-reset");
        let mut p = Preferences::default();
        p.ui_layout.hidden.push(PanelId::Network);
        p.ui_layout.shelf_locked = false;
        p.save_ui_layout();
        p.reset_ui_layout();
        assert_eq!(p.ui_layout, UiLayout::default());
        assert!(p.ui_layout_custom.is_some());
        p.save_to(&path).unwrap();
        assert_eq!(Preferences::load_from(&path).ui_layout, UiLayout::default());
        let _ = fs::remove_file(path);
    }
    #[test] fn corrupt_file_is_backed_up() {
        let path = test_path("prefs-corrupt"); fs::create_dir_all(path.parent().unwrap()).unwrap(); fs::write(&path,b"{").unwrap();
        assert_eq!(Preferences::load_from(&path),Preferences::default());
        assert_eq!(fs::read(path.with_extension("json.bak")).unwrap(),b"{");
        let _ = fs::remove_file(path.with_extension("json.bak"));
    }
    #[test] fn missing_fields_default_and_unknown_fields_ignored() {
        assert_eq!(Preferences::from_json(&json!({})).snap_targets,crate::ui::SnapTargets::default());
        assert_eq!(Preferences::from_json(&json!({"snap_targets":0})).snap_targets,crate::ui::SnapTargets::default());
        let p = Preferences::from_json(&json!({"schema":0,"theme":"Dark","unknown":42}));
        assert_eq!(p.theme,ThemePref::Dark); assert_eq!(p.ui_scale,1.); assert!(p.viewport.show_grid);
        assert_eq!((p.viewport.clip_start, p.viewport.clip_end), (0.01, 1000.0));
        assert_eq!(p.units.display, LengthUnit::Mm);
        assert_eq!(p.units.unit_scale, 1.0);
    }
    #[test] fn python_permission_defaults_off_round_trips_and_feeds_the_gate() {
        use minihou_core::python_bridge::{Decision, PythonGate};
        let d = Preferences::default();
        assert!(!d.allow_python_nodes); assert_eq!(d.python_timeout_s, 30);
        for junk in [json!({}), json!({"security":{}}), json!({"security":{"allow_python_nodes":"yes"}}), json!({"security":{"allow_python_nodes":1}}), json!({"allow_python_nodes":true})] {
            assert!(!Preferences::from_json(&junk).allow_python_nodes, "{junk}");
        }
        let p = Preferences::from_json(&json!({"security":{"allow_python_nodes":true,"python_timeout_s":0}}));
        assert!(p.allow_python_nodes); assert_eq!(p.python_timeout_s, 1);
        assert_eq!(Preferences::from_json(&p.to_json()), p);
        let gate = PythonGate::denied();
        d.apply_python(&gate);
        assert_eq!(gate.check("x"), Decision::Disabled);
        p.apply_python(&gate);
        assert_eq!(gate.check("x"), Decision::NeedsConfirmation);
        assert_eq!(gate.timeout(), std::time::Duration::from_secs(1));
        d.apply_python(&gate);
        assert_eq!(gate.check("x"), Decision::Disabled);
    }
    #[test] fn navigation_defaults_and_round_trips() {
        assert_eq!(Preferences::default().viewport.navigation, Navigation::Blender);
        let mut p = Preferences::default(); p.viewport.navigation = Navigation::Houdini;
        assert_eq!(Preferences::from_json(&p.to_json()).viewport.navigation, Navigation::Houdini);
    }
    #[test] fn clip_planes_defaults_round_trip_and_validation() {
        let legacy = Preferences::from_json(&json!({"viewport": {"view_distance": 50.0}}));
        assert_eq!((legacy.viewport.clip_start, legacy.viewport.clip_end), (0.01, 1000.0));
        let p = Preferences::from_json(&json!({"viewport": {"clip_start": 0.25, "clip_end": 2500.0}}));
        assert_eq!((p.viewport.clip_start, p.viewport.clip_end), (0.25, 2500.0));
        assert_eq!(Preferences::from_json(&p.to_json()), p);
        for (near, far) in [(-1.0, -2.0), (10.0, 1.0), (0.0, 0.0), (1e30, 1e30)] {
            let p = Preferences::from_json(&json!({"viewport": {"clip_start": near, "clip_end": far}}));
            assert!(p.viewport.clip_start > 0.0 && p.viewport.clip_end > p.viewport.clip_start);
        }
    }
    #[test] fn units_preferences_deserialize() {
        let p = Preferences::from_json(&json!({"length_unit": "in", "unit_scale": 0.0254}));
        assert_eq!(p.units.display, LengthUnit::In);
        assert!((p.units.unit_scale - 0.0254).abs() < 1e-9);
        let p2 = Preferences::from_json(&json!({"units": {"length_unit": "ft", "unit_scale": 0.3048}}));
        assert_eq!(p2.units.display, LengthUnit::Ft);
        assert!((p2.units.unit_scale - 0.3048).abs() < 1e-9);
    }
}
