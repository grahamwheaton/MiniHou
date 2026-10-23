//! The editor's panel layout: a split tree of panels and tab groups, saved in preferences.json.
//!
//! The tree is binary: every split has two children and the fraction of its extent given to the first.
//! Leaves are tab groups. The Viewport always exists; the Shelf is in the tree only while it is unlocked.
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Bump when the saved shape changes; layouts with another version fall back to the default.
pub const LAYOUT_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PanelId { Network, Viewport, Parameters, GeoInfo, Spreadsheet, Shelf }
impl PanelId {
    pub const ALL: [Self; 6] = [Self::Network, Self::Viewport, Self::Parameters, Self::GeoInfo, Self::Spreadsheet, Self::Shelf];
    pub fn label(self) -> &'static str {
        match self { Self::Network => "Network", Self::Viewport => "Viewport", Self::Parameters => "Parameters",
            Self::GeoInfo => "Geometry Info", Self::Spreadsheet => "Spreadsheet", Self::Shelf => "Shelf" }
    }
}

/// `Horizontal`: children side by side (left, right). `Vertical`: stacked (top, bottom).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Axis { Horizontal, Vertical }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum DockNode {
    Split { axis: Axis, fraction: f32, a: Box<DockNode>, b: Box<DockNode> },
    Tabs { panels: Vec<PanelId>, active: usize },
}
impl DockNode {
    pub fn tabs(panels: &[PanelId]) -> Self { Self::Tabs { panels: panels.to_vec(), active: 0 } }
    pub fn split(axis: Axis, fraction: f32, a: Self, b: Self) -> Self { Self::Split { axis, fraction, a: Box::new(a), b: Box::new(b) } }
    /// Every panel in the tree, in depth-first order.
    pub fn panels(&self) -> Vec<PanelId> {
        match self {
            Self::Tabs { panels, .. } => panels.clone(),
            Self::Split { a, b, .. } => { let mut out = a.panels(); out.extend(b.panels()); out }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UiLayout {
    pub version: u32,
    pub root: DockNode,
    /// Panels the user has hidden. They stay in the tree and are skipped when laying out.
    #[serde(default)]
    pub hidden: Vec<PanelId>,
    /// Locked: the Shelf is the fixed strip under the top bar. Unlocked: it is a dock panel in `root`.
    pub shelf_locked: bool,
}

impl Default for UiLayout {
    /// Today's layout: Network left, Viewport centre, Parameters over a [Geometry Info, Spreadsheet] tab group on the right.
    fn default() -> Self {
        use PanelId::*;
        let right = DockNode::split(Axis::Vertical, 0.68, DockNode::tabs(&[Parameters]), DockNode::tabs(&[GeoInfo, Spreadsheet]));
        let centre = DockNode::split(Axis::Horizontal, 0.6, DockNode::tabs(&[Viewport]), right);
        Self { version: LAYOUT_VERSION, root: DockNode::split(Axis::Horizontal, 0.28, DockNode::tabs(&[Network]), centre),
            hidden: Vec::new(), shelf_locked: true }
    }
}

impl UiLayout {
    pub fn to_json(&self) -> Value { serde_json::to_value(self).unwrap_or(Value::Null) }
    /// Parse a saved layout. Anything unreadable, from another version or structurally invalid gives `None`.
    pub fn try_from_json(v: &Value) -> Option<Self> {
        let layout: Self = serde_json::from_value(v.clone()).ok()?;
        (layout.version == LAYOUT_VERSION && layout.is_valid()).then_some(layout)
    }
    /// Like `try_from_json`, but old or unknown layouts fall back to the default.
    pub fn from_json(v: &Value) -> Self { Self::try_from_json(v).unwrap_or_default() }

    /// Each panel appears exactly once (the Shelf only while unlocked), tab groups are non-empty,
    /// fractions are finite and inside (0, 1), and the Viewport is alone in its tab group.
    pub fn is_valid(&self) -> bool {
        fn nodes_ok(n: &DockNode) -> bool {
            match n {
                DockNode::Tabs { panels, .. } => !panels.is_empty() && (panels.len() == 1 || !panels.contains(&PanelId::Viewport)),
                DockNode::Split { fraction, a, b, .. } => fraction.is_finite() && *fraction > 0. && *fraction < 1. && nodes_ok(a) && nodes_ok(b),
            }
        }
        let panels = self.root.panels();
        PanelId::ALL.iter().all(|p| {
            let want = if *p == PanelId::Shelf && self.shelf_locked { 0 } else { 1 };
            panels.iter().filter(|q| *q == p).count() == want
        }) && nodes_ok(&self.root)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn json_round_trip() {
        let mut layout = UiLayout::default();
        layout.hidden.push(PanelId::Spreadsheet);
        if let DockNode::Split { fraction, .. } = &mut layout.root { *fraction = 0.4; }
        let text = serde_json::to_string(&layout.to_json()).unwrap();
        let back = UiLayout::from_json(&serde_json::from_str(&text).unwrap());
        assert_eq!(back, layout);
        assert_eq!(layout.to_json()["version"], LAYOUT_VERSION);
    }

    #[test]
    fn unknown_panel_version_or_shape_falls_back_to_default() {
        let mut v = UiLayout::default().to_json();
        v["version"] = json!(LAYOUT_VERSION + 1);
        assert_eq!(UiLayout::try_from_json(&v), None);
        assert_eq!(UiLayout::from_json(&v), UiLayout::default());

        let mut v = UiLayout::default().to_json();
        v["root"]["a"]["panels"] = json!(["Outliner"]);
        assert_eq!(UiLayout::from_json(&v), UiLayout::default());

        // Valid JSON but a panel is missing.
        let mut v = UiLayout::default().to_json();
        v["root"]["a"]["panels"] = json!(["Viewport"]);
        assert_eq!(UiLayout::try_from_json(&v), None);

        for junk in [json!(null), json!(3), json!({}), json!({"version": 1})] {
            assert_eq!(UiLayout::from_json(&junk), UiLayout::default(), "{junk}");
        }
    }

    #[test]
    fn invalid_fractions_and_viewport_tabs_are_rejected() {
        let mut layout = UiLayout::default();
        if let DockNode::Split { fraction, .. } = &mut layout.root { *fraction = 1.5; }
        assert!(!layout.is_valid());
        let mut layout = UiLayout::default();
        if let DockNode::Split { a, .. } = &mut layout.root { **a = DockNode::tabs(&[PanelId::Network, PanelId::Viewport]); }
        assert!(!layout.is_valid());
    }
}
