//! User preferences persisted independently of the document.
use serde_json::{json, Value};
use std::{fs, io, path::{Path, PathBuf}};

pub const SCHEMA: u32 = 1;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemePref { Dark, Light, System }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Navigation { Blender, Houdini }
impl ThemePref {
    pub const ALL: [Self; 3] = [Self::Dark, Self::Light, Self::System];
    pub fn label(self) -> &'static str { match self { Self::Dark => "Dark", Self::Light => "Light", Self::System => "System" } }
}
#[derive(Clone, Debug, PartialEq)]
pub struct ViewportPreferences { pub default_shading: String, pub navigation: Navigation, pub show_grid: bool, pub show_axes: bool, pub auto_budget: u32, pub view_distance: f32 }
impl Default for ViewportPreferences { fn default() -> Self { Self { default_shading: "Shaded".into(), navigation: Navigation::Blender, show_grid: true, show_axes: true, auto_budget: 400_000, view_distance: 20.0 } } }
#[derive(Clone, Debug, PartialEq)]
pub struct Preferences {
    pub theme: ThemePref, pub ui_scale: f32, pub emulate_numpad: bool, pub snap_increment: f32,
    pub show_stats: bool, pub autosave_minutes: Option<u32>, pub keymap_overrides: Value,
    pub recent_files: Vec<PathBuf>, pub viewport: ViewportPreferences,
}
impl Default for Preferences { fn default() -> Self { Self {
    theme: ThemePref::System, ui_scale: 1., emulate_numpad: false, snap_increment: 0.25,
    show_stats: true, autosave_minutes: Some(5), keymap_overrides: json!({"bindings": []}),
    recent_files: Vec::new(), viewport: ViewportPreferences::default(),
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
        "emulate_numpad": self.emulate_numpad, "snap_increment": self.snap_increment,
        "show_stats": self.show_stats, "autosave_minutes": self.autosave_minutes,
        "keymap_overrides": self.keymap_overrides,
        "recent_files": self.recent_files.iter().take(10).collect::<Vec<_>>(),
        "viewport": { "default_shading": self.viewport.default_shading,
            "navigation": match self.viewport.navigation { Navigation::Blender => "blender", Navigation::Houdini => "houdini" },
            "show_grid": self.viewport.show_grid, "show_axes": self.viewport.show_axes, "auto_budget": self.viewport.auto_budget,
            "view_distance": self.viewport.view_distance },
    }) }
    fn from_json(v: &Value) -> Self {
        let mut p = Self { theme: match v.get("theme").and_then(Value::as_str) {
            Some("Dark") => ThemePref::Dark, Some("Light") => ThemePref::Light, _ => ThemePref::System }, ..Self::default() };
        if let Some(x) = v.get("ui_scale").and_then(Value::as_f64) { p.ui_scale = (x as f32).clamp(0.5, 3.); }
        if let Some(x) = v.get("emulate_numpad").and_then(Value::as_bool) { p.emulate_numpad = x; }
        if let Some(x) = v.get("snap_increment").and_then(Value::as_f64) { p.snap_increment = (x as f32).max(0.0001); }
        if let Some(x) = v.get("show_stats").and_then(Value::as_bool) { p.show_stats = x; }
        if let Some(x) = v.get("autosave_minutes") { p.autosave_minutes = x.as_u64().and_then(|n| u32::try_from(n).ok()).filter(|n| *n > 0); }
        if let Some(x) = v.get("keymap_overrides").filter(|x| x.is_object()) { p.keymap_overrides = x.clone(); }
        if let Some(xs) = v.get("recent_files").and_then(Value::as_array) {
            p.recent_files = xs.iter().filter_map(Value::as_str).take(10).map(PathBuf::from).collect();
        }
        if let Some(view) = v.get("viewport") {
            if let Some(x) = view.get("navigation").and_then(Value::as_str) { p.viewport.navigation = if x == "houdini" { Navigation::Houdini } else { Navigation::Blender }; }
            if let Some(x) = view.get("default_shading").and_then(Value::as_str) { p.viewport.default_shading = x.to_owned(); }
            if let Some(x) = view.get("show_grid").and_then(Value::as_bool) { p.viewport.show_grid = x; }
            if let Some(x) = view.get("show_axes").and_then(Value::as_bool) { p.viewport.show_axes = x; }
            if let Some(x) = view.get("auto_budget").and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok()) { p.viewport.auto_budget = x.max(1); }
            if let Some(x) = view.get("view_distance").and_then(Value::as_f64) { p.viewport.view_distance = (x as f32).clamp(3.0, 1000.0); }
        }
        p
    }
    pub fn add_recent(&mut self, path: PathBuf) {
        self.recent_files.retain(|p| p != &path);
        self.recent_files.insert(0, path); self.recent_files.truncate(10);
    }
}
#[cfg(test)] mod tests {
    use super::*;
    fn test_path(name: &str) -> PathBuf { std::env::temp_dir().join(format!("minihou-{name}-{}",std::process::id())).join("preferences.json") }
    #[test] fn round_trip() {
        let path = test_path("prefs-roundtrip");
        let mut p = Preferences { theme: ThemePref::Light, ui_scale: 1.25, recent_files: vec![PathBuf::from("a.hou")], ..Default::default() };
        p.viewport.auto_budget = 250_000;
        p.viewport.view_distance = 100.0;
        p.save_to(&path).unwrap(); p.save_to(&path).unwrap(); assert_eq!(Preferences::load_from(&path),p);
        let _ = fs::remove_file(path);
    }
    #[test] fn corrupt_file_is_backed_up() {
        let path = test_path("prefs-corrupt"); fs::create_dir_all(path.parent().unwrap()).unwrap(); fs::write(&path,b"{").unwrap();
        assert_eq!(Preferences::load_from(&path),Preferences::default());
        assert_eq!(fs::read(path.with_extension("json.bak")).unwrap(),b"{");
        let _ = fs::remove_file(path.with_extension("json.bak"));
    }
    #[test] fn missing_fields_default_and_unknown_fields_ignored() {
        let p = Preferences::from_json(&json!({"schema":0,"theme":"Dark","unknown":42}));
        assert_eq!(p.theme,ThemePref::Dark); assert_eq!(p.ui_scale,1.); assert!(p.viewport.show_grid);
        assert_eq!(p.viewport.view_distance, 20.0);
    }
    #[test] fn navigation_defaults_and_round_trips() {
        assert_eq!(Preferences::default().viewport.navigation, Navigation::Blender);
        let mut p = Preferences::default(); p.viewport.navigation = Navigation::Houdini;
        assert_eq!(Preferences::from_json(&p.to_json()).viewport.navigation, Navigation::Houdini);
    }
    #[test] fn view_distance_defaults_and_round_trips() {
        assert_eq!(Preferences::default().viewport.view_distance, 20.0);
        let p = Preferences::from_json(&json!({"viewport": {"view_distance": 50.0}}));
        assert_eq!(p.viewport.view_distance, 50.0);
        let p_clamp = Preferences::from_json(&json!({"viewport": {"view_distance": 1.0}}));
        assert_eq!(p_clamp.viewport.view_distance, 3.0);
        let p_max = Preferences::from_json(&json!({"viewport": {"view_distance": 2000.0}}));
        assert_eq!(p_max.viewport.view_distance, 1000.0);
    }
}
