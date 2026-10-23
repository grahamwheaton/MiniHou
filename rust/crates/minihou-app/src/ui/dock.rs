//! Placement of the `UiLayout` split tree inside a rectangle. Pure maths, so it is tested without a window.
use super::layout::{Axis, DockNode, PanelId};
use eframe::egui::{self, Pos2, Rect};

/// Smallest extent either side of a split keeps while there is room for both.
pub const MIN_EXTENT: f32 = 60.;

/// Address of a node: the child taken at each split (0 = `a`, 1 = `b`).
pub type NodePath = Vec<u8>;

#[derive(Clone, Debug, PartialEq)]
pub struct Leaf { pub path: NodePath, pub panels: Vec<PanelId>, pub active: PanelId, pub rect: Rect }

#[derive(Clone, Debug, PartialEq)]
pub struct Divider { pub path: NodePath, pub axis: Axis, /// The full rect the split divides.
    pub span: Rect, /// Coordinate of the boundary (x for Horizontal, y for Vertical).
    pub at: f32 }

fn visible(node: &DockNode, hidden: &[PanelId]) -> bool { node.panels().iter().any(|p| !hidden.contains(p)) }

/// Where a split of `extent` starting at `start` puts its boundary.
pub fn split_at(start: f32, extent: f32, fraction: f32) -> f32 {
    let at = extent * fraction.clamp(0., 1.);
    start + if extent > 2. * MIN_EXTENT { at.clamp(MIN_EXTENT, extent - MIN_EXTENT) } else { at }
}

fn halves(axis: Axis, rect: Rect, fraction: f32) -> (Rect, Rect, f32) {
    match axis {
        Axis::Horizontal => { let x = split_at(rect.left(), rect.width(), fraction);
            (Rect::from_min_max(rect.min, Pos2::new(x, rect.bottom())), Rect::from_min_max(Pos2::new(x, rect.top()), rect.max), x) }
        Axis::Vertical => { let y = split_at(rect.top(), rect.height(), fraction);
            (Rect::from_min_max(rect.min, Pos2::new(rect.right(), y)), Rect::from_min_max(Pos2::new(rect.left(), y), rect.max), y) }
    }
}

fn walk(node: &DockNode, rect: Rect, hidden: &[PanelId], path: &mut NodePath, leaves: &mut Vec<Leaf>, dividers: &mut Vec<Divider>) {
    match node {
        DockNode::Tabs { panels, active } => {
            let shown: Vec<PanelId> = panels.iter().copied().filter(|p| !hidden.contains(p)).collect();
            let Some(first) = shown.first().copied() else { return };
            let active = panels.get(*active).copied().filter(|p| shown.contains(p)).unwrap_or(first);
            leaves.push(Leaf { path: path.clone(), panels: shown, active, rect });
        }
        DockNode::Split { axis, fraction, a, b } => {
            let (va, vb) = (visible(a, hidden), visible(b, hidden));
            let (ra, rb, at) = match (va, vb) { (true, true) => halves(*axis, rect, *fraction), _ => (rect, rect, 0.) };
            if va && vb { dividers.push(Divider { path: path.clone(), axis: *axis, span: rect, at }); }
            if va { path.push(0); walk(a, ra, hidden, path, leaves, dividers); path.pop(); }
            if vb { path.push(1); walk(b, rb, hidden, path, leaves, dividers); path.pop(); }
        }
    }
}

/// Visible tab groups with their rects, and the dividers between visible siblings.
/// A split whose one side is entirely hidden gives the whole rect to the other side.
pub fn place(root: &DockNode, rect: Rect, hidden: &[PanelId]) -> (Vec<Leaf>, Vec<Divider>) {
    let (mut leaves, mut dividers) = (Vec::new(), Vec::new());
    walk(root, rect, hidden, &mut Vec::new(), &mut leaves, &mut dividers);
    (leaves, dividers)
}

pub fn node_at<'a>(root: &'a DockNode, path: &[u8]) -> Option<&'a DockNode> {
    path.iter().try_fold(root, |n, step| match n { DockNode::Split { a, b, .. } => Some(if *step == 0 { &**a } else { &**b }), _ => None })
}
pub fn node_at_mut<'a>(root: &'a mut DockNode, path: &[u8]) -> Option<&'a mut DockNode> {
    path.iter().try_fold(root, |n, step| match n { DockNode::Split { a, b, .. } => Some(if *step == 0 { &mut **a } else { &mut **b }), _ => None })
}

/// Paint a docked panel's background like a side panel and return the rect its contents use.
pub fn leaf_frame(ui: &egui::Ui, leaf: &Leaf) -> Rect {
    ui.painter().rect_filled(leaf.rect, 0., ui.visuals().panel_fill);
    leaf.rect - egui::Frame::side_top_panel(ui.style()).inner_margin
}

/// Thin separator lines where side panels used to draw theirs.
pub fn paint_dividers(ui: &egui::Ui, dividers: &[Divider]) {
    let stroke = ui.visuals().widgets.noninteractive.bg_stroke;
    for d in dividers {
        let seg = match d.axis {
            Axis::Horizontal => [Pos2::new(d.at, d.span.top()), Pos2::new(d.at, d.span.bottom())],
            Axis::Vertical => [Pos2::new(d.span.left(), d.at), Pos2::new(d.span.right(), d.at)],
        };
        ui.painter().line_segment(seg, stroke);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::layout::UiLayout;
    use PanelId::*;

    fn screen() -> Rect { Rect::from_min_size(Pos2::ZERO, eframe::egui::vec2(1000., 800.)) }

    #[test]
    fn default_layout_places_today_panels() {
        let (leaves, dividers) = place(&UiLayout::default().root, screen(), &[]);
        let by = |p: PanelId| leaves.iter().find(|l| l.panels.contains(&p)).unwrap().rect;
        assert_eq!(by(Network), Rect::from_min_max(Pos2::ZERO, Pos2::new(280., 800.)));
        assert_eq!(by(Viewport).left(), 280.);
        assert!((by(Viewport).width() - 720. * 0.6).abs() < 1e-3);
        assert_eq!(by(Parameters).right(), 1000.);
        assert!((by(Parameters).height() - 800. * 0.68).abs() < 1e-3);
        assert_eq!(by(GeoInfo), by(Spreadsheet));
        assert_eq!(by(GeoInfo).bottom(), 800.);
        assert_eq!(leaves.iter().find(|l| l.panels.contains(&GeoInfo)).unwrap().active, GeoInfo);
        assert_eq!(dividers.len(), 3);
    }

    #[test]
    fn hidden_panels_give_their_space_away() {
        let (leaves, dividers) = place(&UiLayout::default().root, screen(), &[Parameters, GeoInfo, Spreadsheet]);
        assert_eq!(leaves.len(), 2);
        assert_eq!(leaves[1].rect.right(), 1000.);
        assert_eq!(dividers.len(), 1);
        let (leaves, _) = place(&UiLayout::default().root, screen(), &[GeoInfo]);
        let tabs = leaves.iter().find(|l| l.panels == [Spreadsheet]).unwrap();
        assert_eq!(tabs.active, Spreadsheet);
    }

    #[test]
    fn split_keeps_minimum_extents() {
        assert_eq!(split_at(0., 1000., 0.01), MIN_EXTENT);
        assert_eq!(split_at(10., 1000., 0.99), 10. + 1000. - MIN_EXTENT);
        assert_eq!(split_at(0., 100., 0.5), 50.);
    }
}
