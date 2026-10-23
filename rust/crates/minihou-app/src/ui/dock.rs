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

/// Height of the tab header strip on every docked panel except the Viewport.
pub const TAB_HEIGHT: f32 = 22.;

/// Paint a docked panel's background like a side panel and return the rect its contents use (below the tab header).
pub fn leaf_frame(ui: &egui::Ui, leaf: &Leaf) -> Rect {
    ui.painter().rect_filled(leaf.rect, 0., ui.visuals().panel_fill);
    let body = Rect::from_min_max(Pos2::new(leaf.rect.left(), (leaf.rect.top() + TAB_HEIGHT).min(leaf.rect.bottom())), leaf.rect.max);
    body - egui::Frame::side_top_panel(ui.style()).inner_margin
}

// ---- Editing the tree (pure) ----

/// Where a dragged tab lands relative to the panel under the pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropZone { Centre, Left, Right, Top, Bottom }

/// Outer quarter of each side is an edge (split) zone; the middle joins the tab group.
pub fn drop_zone(rect: Rect, pos: Pos2) -> DropZone {
    let u = (pos.x - rect.left()) / rect.width().max(1.);
    let w = (pos.y - rect.top()) / rect.height().max(1.);
    let edges = [(u, DropZone::Left), (1. - u, DropZone::Right), (w, DropZone::Top), (1. - w, DropZone::Bottom)];
    let (d, zone) = edges.into_iter().fold((f32::MAX, DropZone::Centre), |best, e| if e.0 < best.0 { e } else { best });
    if d < 0.25 { zone } else { DropZone::Centre }
}

/// The part of `rect` a drop into `zone` would occupy (used for the drop preview).
pub fn zone_rect(rect: Rect, zone: DropZone) -> Rect {
    let c = rect.center();
    match zone {
        DropZone::Centre => rect,
        DropZone::Left => Rect::from_min_max(rect.min, Pos2::new(c.x, rect.bottom())),
        DropZone::Right => Rect::from_min_max(Pos2::new(c.x, rect.top()), rect.max),
        DropZone::Top => Rect::from_min_max(rect.min, Pos2::new(rect.right(), c.y)),
        DropZone::Bottom => Rect::from_min_max(Pos2::new(rect.left(), c.y), rect.max),
    }
}

/// Fraction a divider gets when dragged to `pos`.
pub fn fraction_at(d: &Divider, pos: Pos2) -> f32 {
    let (start, extent, p) = match d.axis {
        Axis::Horizontal => (d.span.left(), d.span.width(), pos.x),
        Axis::Vertical => (d.span.top(), d.span.height(), pos.y),
    };
    ((p - start) / extent.max(1.)).clamp(0.02, 0.98)
}

pub fn set_fraction(root: &mut DockNode, path: &[u8], value: f32) -> bool {
    match node_at_mut(root, path) {
        Some(DockNode::Split { fraction, .. }) => { *fraction = value.clamp(0.02, 0.98); true }
        _ => false,
    }
}

pub fn set_active(root: &mut DockNode, panel: PanelId) {
    match root {
        DockNode::Tabs { panels, active } => if let Some(i) = panels.iter().position(|p| *p == panel) { *active = i; },
        DockNode::Split { a, b, .. } => { set_active(a, panel); set_active(b, panel); }
    }
}

/// Take `panel` out of the tree; empty tab groups disappear and their split collapses into the sibling.
pub fn remove(node: DockNode, panel: PanelId) -> Option<DockNode> {
    match node {
        DockNode::Tabs { mut panels, active } => {
            let at = panels.iter().position(|p| *p == panel);
            panels.retain(|p| *p != panel);
            if panels.is_empty() { return None; }
            let active = match at { Some(i) if i < active => active - 1, _ => active }.min(panels.len() - 1);
            Some(DockNode::Tabs { panels, active })
        }
        DockNode::Split { axis, fraction, a, b } => match (remove(*a, panel), remove(*b, panel)) {
            (Some(a), Some(b)) => Some(DockNode::split(axis, fraction, a, b)),
            (one, other) => one.or(other),
        },
    }
}

/// Insert `panel` next to (or into) the tab group holding `target`. Returns false if `target` is absent.
pub fn insert(node: &mut DockNode, panel: PanelId, target: PanelId, zone: DropZone) -> bool {
    match node {
        DockNode::Split { a, b, .. } => insert(a, panel, target, zone) || insert(b, panel, target, zone),
        DockNode::Tabs { panels, active } if panels.contains(&target) => {
            if zone == DropZone::Centre { panels.push(panel); *active = panels.len() - 1; return true; }
            let old = std::mem::replace(node, DockNode::tabs(&[]));
            let new = DockNode::tabs(&[panel]);
            *node = match zone {
                DropZone::Left => DockNode::split(Axis::Horizontal, 0.5, new, old),
                DropZone::Right => DockNode::split(Axis::Horizontal, 0.5, old, new),
                DropZone::Top => DockNode::split(Axis::Vertical, 0.5, new, old),
                _ => DockNode::split(Axis::Vertical, 0.5, old, new),
            };
            true
        }
        DockNode::Tabs { .. } => false,
    }
}

/// Move the dragged `panel` onto the group holding `target`. Centre joins that tab group, an edge splits it.
/// `None` when the move is not allowed or changes nothing: the Viewport never moves and never shares a tab group,
/// and a panel is never dropped on itself.
pub fn retarget(root: &DockNode, panel: PanelId, target: PanelId, zone: DropZone) -> Option<DockNode> {
    if panel == PanelId::Viewport || panel == target { return None; }
    if zone == DropZone::Centre && target == PanelId::Viewport { return None; }
    let all = root.panels();
    if !all.contains(&panel) || !all.contains(&target) { return None; }
    let mut tree = remove(root.clone(), panel)?;
    insert(&mut tree, panel, target, zone).then_some(tree)
}

/// Drop the dragged `panel` on the tab header of `target`: it joins `target`'s tab group at `target`'s
/// position (just before it) and becomes the active tab. Works within one group too (reorders).
pub fn join_at(root: &DockNode, panel: PanelId, target: PanelId) -> Option<DockNode> {
    if panel == PanelId::Viewport || target == PanelId::Viewport || panel == target { return None; }
    let all = root.panels();
    if !all.contains(&panel) || !all.contains(&target) { return None; }
    fn put(n: &mut DockNode, panel: PanelId, target: PanelId) -> bool {
        match n {
            DockNode::Split { a, b, .. } => put(a, panel, target) || put(b, panel, target),
            DockNode::Tabs { panels, active } => match panels.iter().position(|p| *p == target) {
                Some(i) => { panels.insert(i, panel); *active = i; true }
                None => false,
            },
        }
    }
    let mut tree = remove(root.clone(), panel)?;
    (put(&mut tree, panel, target) && tree != *root).then_some(tree)
}

/// Close a tab: the panel is hidden (it stays in the tree and can be added back from any "+" menu).
/// A group whose panels are all closed takes no space. `None` for the Viewport or an already closed panel.
pub fn close_tab(hidden: &[PanelId], panel: PanelId) -> Option<Vec<PanelId>> {
    if panel == PanelId::Viewport || hidden.contains(&panel) { return None; }
    let mut out = hidden.to_vec();
    out.push(panel);
    Some(out)
}

/// Panels a "+" menu offers: closed panels that are in the tree (a locked Shelf is not), never the Viewport.
pub fn addable(root: &DockNode, hidden: &[PanelId]) -> Vec<PanelId> {
    let all = root.panels();
    PanelId::ALL.into_iter().filter(|p| *p != PanelId::Viewport && hidden.contains(p) && all.contains(p)).collect()
}

/// Add a closed `panel` from the "+" menu to the tab group holding `target`, as its last and active tab.
/// The group it was closed in collapses if it is left empty.
pub fn add_tab(root: &DockNode, hidden: &[PanelId], panel: PanelId, target: PanelId) -> Option<(DockNode, Vec<PanelId>)> {
    if !addable(root, hidden).contains(&panel) || target == PanelId::Viewport || !root.panels().contains(&target) { return None; }
    let mut tree = remove(root.clone(), panel)?;
    if !insert(&mut tree, panel, target, DropZone::Centre) { return None; }
    Some((tree, hidden.iter().copied().filter(|p| *p != panel).collect()))
}

/// A panel in `leaf` other than `dragged`, used as the drop target for that group.
pub fn target_in(leaf: &Leaf, dragged: PanelId) -> Option<PanelId> { leaf.panels.iter().copied().find(|p| *p != dragged) }

fn drag_id() -> egui::Id { egui::Id::new("dock_drag_panel") }

/// Small Lock/Unlock toggle at the top-right of the Shelf (its strip when locked, its tab header when docked).
pub fn shelf_lock_button(ctx: &egui::Context, rect: Rect, locked: bool, out: &mut super::Actions) {
    let pos = Pos2::new(rect.right() - 58., rect.top() + 2.);
    egui::Area::new(egui::Id::new("shelf_lock_toggle")).fixed_pos(pos).order(egui::Order::Foreground).show(ctx, |ui| {
        let (label, tip) = if locked { ("Unlock", "Shelf is locked in place. Unlock to drag it as a panel.") }
            else { ("Lock", "Lock the Shelf back as the fixed strip under the top bar.") };
        if ui.small_button(label).on_hover_text(tip).clicked() { out.push(super::UiAction::ShelfLock(!locked)); }
    });
}

/// "Reset UI to Default": forget panel state kept in egui memory (an unfinished tab drag, the docked
/// Geometry Info collapse/ranges state, and remembered window/area positions).
pub fn reset_memory(ctx: &egui::Context) {
    let geo = super::geo_info::dock_id();
    ctx.data_mut(|d| {
        d.remove::<PanelId>(drag_id());
        d.remove::<bool>(geo.with("ranges"));
        d.remove::<bool>(geo.with("open"));
    });
    ctx.memory_mut(|m| m.reset_areas());
}

/// Draw tab headers and divider handles; handle tab clicks, tab drags and divider drags.
/// Each tab shows "Name ×" (the cross closes it); a "+" after the tabs adds a closed panel to that group.
/// Returns the edited tree and closed-panel list when the user changed them this frame.
pub fn interact(ui: &mut egui::Ui, root: &DockNode, hidden: &[PanelId], leaves: &[Leaf], dividers: &[Divider]) -> Option<(DockNode, Vec<PanelId>)> {
    let mut edited: Option<DockNode> = None;
    let mut closed: Option<Vec<PanelId>> = None;
    let mut headers: Vec<(PanelId, Rect)> = Vec::new();
    let font = egui::FontId::proportional(12.);
    let ctx = ui.ctx().clone();
    let pointer = ctx.input(|i| i.pointer.interact_pos());
    let visuals = ui.visuals().clone();
    for d in dividers {
        let line = match d.axis {
            Axis::Horizontal => Rect::from_min_max(Pos2::new(d.at - 3., d.span.top()), Pos2::new(d.at + 3., d.span.bottom())),
            Axis::Vertical => Rect::from_min_max(Pos2::new(d.span.left(), d.at - 3.), Pos2::new(d.span.right(), d.at + 3.)),
        };
        let r = ui.interact(line, egui::Id::new(("dock_divider", d.path.clone())), egui::Sense::drag());
        if r.hovered() || r.dragged() {
            ctx.set_cursor_icon(if d.axis == Axis::Horizontal { egui::CursorIcon::ResizeHorizontal } else { egui::CursorIcon::ResizeVertical });
        }
        if r.dragged() { if let Some(pos) = pointer {
            let mut tree = edited.take().unwrap_or_else(|| root.clone());
            set_fraction(&mut tree, &d.path, fraction_at(d, pos));
            edited = Some(tree);
        } }
    }
    for leaf in leaves {
        if leaf.active == PanelId::Viewport { continue; }
        let strip = Rect::from_min_size(leaf.rect.min, egui::vec2(leaf.rect.width(), TAB_HEIGHT));
        ui.painter().rect_filled(strip, 0., visuals.faint_bg_color);
        let mut x = strip.left() + 4.;
        for panel in &leaf.panels {
            let width = ui.painter().layout_no_wrap(panel.label().into(), font.clone(), visuals.text_color()).size().x;
            let tab = Rect::from_min_size(Pos2::new(x, strip.top()), egui::vec2(width + 30., TAB_HEIGHT));
            x = tab.right() + 2.;
            headers.push((*panel, tab));
            let cross = Rect::from_center_size(Pos2::new(tab.right() - 10., tab.center().y), egui::vec2(14., 14.));
            let r = ui.interact(tab, egui::Id::new(("dock_tab", *panel)), egui::Sense::click_and_drag());
            let rc = ui.interact(cross, egui::Id::new(("dock_tab_close", *panel)), egui::Sense::click()).on_hover_text(format!("Close {}", panel.label()));
            let active = *panel == leaf.active;
            if active { ui.painter().rect_filled(tab, 3., visuals.panel_fill); }
            let color = if active || r.hovered() { visuals.strong_text_color() } else { visuals.weak_text_color() };
            ui.painter().text(Pos2::new(tab.left() + 8., tab.center().y), egui::Align2::LEFT_CENTER, panel.label(), font.clone(), color);
            if rc.hovered() { ui.painter().rect_filled(cross, 2., visuals.widgets.hovered.bg_fill); }
            ui.painter().text(cross.center(), egui::Align2::CENTER_CENTER, "×", font.clone(), if rc.hovered() { visuals.strong_text_color() } else { visuals.weak_text_color() });
            if rc.clicked() {
                let base = closed.take().unwrap_or_else(|| hidden.to_vec());
                closed = Some(close_tab(&base, *panel).unwrap_or(base));
            } else if r.clicked() && !active {
                let mut tree = edited.take().unwrap_or_else(|| root.clone());
                set_active(&mut tree, *panel);
                edited = Some(tree);
            }
            if r.drag_started() { ctx.data_mut(|d| d.insert_temp(drag_id(), *panel)); }
        }
        // "+": add a closed panel to this group.
        let plus = Rect::from_min_size(Pos2::new(x, strip.top() + 2.), egui::vec2(TAB_HEIGHT - 4., TAB_HEIGHT - 4.));
        let rp = ui.interact(plus, egui::Id::new(("dock_tab_add", leaf.path.clone())), egui::Sense::click()).on_hover_text("Add a panel to this section");
        if rp.hovered() { ui.painter().rect_filled(plus, 2., visuals.widgets.hovered.bg_fill); }
        ui.painter().text(plus.center(), egui::Align2::CENTER_CENTER, "+", egui::FontId::proportional(14.), visuals.text_color());
        let options = addable(root, hidden);
        let target = leaf.active;
        egui::Popup::menu(&rp).show(|ui| {
            if options.is_empty() { ui.weak("All panels are shown"); }
            for p in &options {
                if ui.button(p.label()).clicked() {
                    if let Some((tree, h)) = add_tab(root, hidden, *p, target) { edited = Some(tree); closed = Some(h); }
                    ui.close();
                }
            }
        });
    }
    let dragged = ctx.data(|d| d.get_temp::<PanelId>(drag_id()));
    if let (Some(dragged), Some(pos)) = (dragged, pointer) {
        // Over a tab header: join that group at that tab's position. Elsewhere: drop zones.
        let on_tab = headers.iter().find(|(p, r)| *p != dragged && r.contains(pos)).copied();
        let result = if let Some((target, tab)) = on_tab {
            let result = join_at(root, dragged, target);
            if result.is_some() {
                let bar = Rect::from_min_max(Pos2::new(tab.left() - 2., tab.top()), Pos2::new(tab.left() + 1., tab.bottom()));
                ui.painter().rect_filled(bar, 1., visuals.selection.bg_fill);
            }
            result
        } else {
            let hover = leaves.iter().find(|l| l.rect.contains(pos));
            let drop = hover.and_then(|leaf| target_in(leaf, dragged).map(|t| (leaf, t, drop_zone(leaf.rect, pos))));
            let result = drop.and_then(|(_, target, zone)| retarget(root, dragged, target, zone));
            if let (Some((leaf, _, zone)), Some(_)) = (drop, &result) {
                ui.painter().rect_filled(zone_rect(leaf.rect, zone), 2., visuals.selection.bg_fill.gamma_multiply(0.35));
            }
            result
        };
        ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
        if ctx.input(|i| i.pointer.any_released()) {
            ctx.data_mut(|d| d.remove::<PanelId>(drag_id()));
            if result.is_some() { edited = result; }
        }
    } else if dragged.is_some() && !ctx.input(|i| i.pointer.any_down()) {
        ctx.data_mut(|d| d.remove::<PanelId>(drag_id()));
    }
    if edited.is_none() && closed.is_none() { return None; }
    Some((edited.unwrap_or_else(|| root.clone()), closed.unwrap_or_else(|| hidden.to_vec())))
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

    fn valid(root: DockNode) -> bool { UiLayout { root, ..UiLayout::default() }.is_valid() }

    #[test]
    fn drop_zones_and_divider_fraction() {
        let r = screen();
        assert_eq!(drop_zone(r, Pos2::new(500., 400.)), DropZone::Centre);
        assert_eq!(drop_zone(r, Pos2::new(10., 400.)), DropZone::Left);
        assert_eq!(drop_zone(r, Pos2::new(990., 400.)), DropZone::Right);
        assert_eq!(drop_zone(r, Pos2::new(500., 10.)), DropZone::Top);
        assert_eq!(drop_zone(r, Pos2::new(500., 790.)), DropZone::Bottom);
        assert_eq!(zone_rect(r, DropZone::Right).left(), 500.);
        let (_, dividers) = place(&UiLayout::default().root, r, &[]);
        assert!((fraction_at(&dividers[0], Pos2::new(400., 0.)) - 0.4).abs() < 1e-6);
        let mut root = UiLayout::default().root;
        assert!(set_fraction(&mut root, &dividers[0].path, 0.4));
        let (leaves, _) = place(&root, r, &[]);
        assert!((leaves[0].rect.right() - 400.).abs() < 1e-3);
        assert!(!set_fraction(&mut root, &[0], 0.5), "a tab group has no fraction");
    }

    #[test]
    fn centre_drop_joins_tab_group() {
        let root = retarget(&UiLayout::default().root, Spreadsheet, Network, DropZone::Centre).unwrap();
        assert!(valid(root.clone()));
        assert_eq!(node_at(&root, &[0]), Some(&DockNode::Tabs { panels: vec![Network, Spreadsheet], active: 1 }));
        assert_eq!(node_at(&root, &[1, 1, 1]), Some(&DockNode::tabs(&[GeoInfo])));
    }

    #[test]
    fn edge_drop_splits_and_emptied_group_collapses() {
        // Parameters to the left of the Viewport: its old split collapses into the tab group.
        let root = retarget(&UiLayout::default().root, Parameters, Viewport, DropZone::Left).unwrap();
        assert!(valid(root.clone()));
        let DockNode::Split { a, b, .. } = node_at(&root, &[1]).unwrap() else { panic!() };
        assert_eq!(**a, DockNode::split(Axis::Horizontal, 0.5, DockNode::tabs(&[Parameters]), DockNode::tabs(&[Viewport])));
        assert_eq!(**b, DockNode::tabs(&[GeoInfo, Spreadsheet]));
        // GeoInfo below Network.
        let root = retarget(&UiLayout::default().root, GeoInfo, Network, DropZone::Bottom).unwrap();
        assert!(valid(root.clone()));
        assert_eq!(node_at(&root, &[0, 1]), Some(&DockNode::tabs(&[GeoInfo])));
        assert_eq!(node_at(&root, &[1, 1, 1]), Some(&DockNode::tabs(&[Spreadsheet])));
    }

    #[test]
    fn viewport_stays_central_and_no_op_drops_are_refused() {
        let root = UiLayout::default().root;
        assert_eq!(retarget(&root, Viewport, Network, DropZone::Left), None);
        assert_eq!(retarget(&root, Network, Viewport, DropZone::Centre), None);
        assert_eq!(retarget(&root, Network, Network, DropZone::Centre), None);
        assert_eq!(retarget(&root, Shelf, Network, DropZone::Centre), None, "locked shelf is not in the tree");
        // Splitting a panel off its own group works when the group has others.
        let split = retarget(&root, Spreadsheet, GeoInfo, DropZone::Right).unwrap();
        assert!(valid(split.clone()));
        assert_eq!(node_at(&split, &[1, 1, 1]), Some(&DockNode::split(Axis::Horizontal, 0.5, DockNode::tabs(&[GeoInfo]), DockNode::tabs(&[Spreadsheet]))));
        let mut root = root;
        set_active(&mut root, Spreadsheet);
        assert_eq!(node_at(&root, &[1, 1, 1]), Some(&DockNode::Tabs { panels: vec![GeoInfo, Spreadsheet], active: 1 }));
        let (leaves, _) = place(&root, screen(), &[]);
        assert_eq!(target_in(leaves.last().unwrap(), Spreadsheet), Some(GeoInfo));
    }

    #[test]
    fn closing_tabs_hides_them_and_an_empty_group_gives_its_space_away() {
        let root = UiLayout::default().root;
        let h = close_tab(&[], GeoInfo).unwrap();
        assert_eq!(h, [GeoInfo]);
        assert_eq!(close_tab(&h, GeoInfo), None, "already closed");
        assert_eq!(close_tab(&[], Viewport), None, "the Viewport has no close cross");
        let (leaves, _) = place(&root, screen(), &h);
        assert_eq!(leaves.iter().find(|l| l.panels.contains(&Spreadsheet)).unwrap().panels, [Spreadsheet]);
        let h = close_tab(&h, Spreadsheet).unwrap();
        let (leaves, dividers) = place(&root, screen(), &h);
        assert!(leaves.iter().all(|l| !l.panels.contains(&GeoInfo) && !l.panels.contains(&Spreadsheet)));
        assert_eq!(leaves.iter().find(|l| l.panels == [Parameters]).unwrap().rect.bottom(), 800.);
        assert_eq!(dividers.len(), 2);
        assert!(UiLayout { hidden: h, ..UiLayout::default() }.is_valid());
    }

    #[test]
    fn plus_menu_lists_closed_panels_and_adds_them_to_a_group() {
        let root = UiLayout::default().root;
        assert!(addable(&root, &[]).is_empty());
        assert_eq!(addable(&root, &[Shelf, Spreadsheet]), [Spreadsheet], "a locked Shelf is not in the tree");
        let hidden = [GeoInfo, Spreadsheet];
        assert_eq!(addable(&root, &hidden), [GeoInfo, Spreadsheet]);
        // Add Spreadsheet to the Network group; GeoInfo stays closed in its old group.
        let (tree, h) = add_tab(&root, &hidden, Spreadsheet, Network).unwrap();
        assert_eq!(h, [GeoInfo]);
        assert_eq!(node_at(&tree, &[0]), Some(&DockNode::Tabs { panels: vec![Network, Spreadsheet], active: 1 }));
        assert!(valid(tree.clone()));
        // Then GeoInfo: its old group is now empty and collapses into Parameters.
        let (tree, h) = add_tab(&tree, &h, GeoInfo, Network).unwrap();
        assert!(h.is_empty() && valid(tree.clone()));
        assert_eq!(node_at(&tree, &[1, 1]), Some(&DockNode::tabs(&[Parameters])));
        // Refused: not closed, or into the Viewport.
        assert_eq!(add_tab(&root, &[], GeoInfo, Network), None);
        assert_eq!(add_tab(&root, &hidden, GeoInfo, Viewport), None);
        // Re-adding to the group it was closed in just reopens it as the active tab.
        let (tree, h) = add_tab(&root, &[GeoInfo], GeoInfo, Spreadsheet).unwrap();
        assert!(h.is_empty());
        assert_eq!(node_at(&tree, &[1, 1, 1]), Some(&DockNode::Tabs { panels: vec![Spreadsheet, GeoInfo], active: 1 }));
    }

    #[test]
    fn dropping_on_a_tab_header_joins_at_that_position() {
        let root = UiLayout::default().root;
        let tree = join_at(&root, Network, Spreadsheet).unwrap();
        assert!(valid(tree.clone()));
        assert_eq!(node_at(&tree, &[1, 1]), Some(&DockNode::Tabs { panels: vec![GeoInfo, Network, Spreadsheet], active: 1 }));
        let tree = join_at(&root, Parameters, GeoInfo).unwrap();
        assert_eq!(node_at(&tree, &[1, 1]), Some(&DockNode::Tabs { panels: vec![Parameters, GeoInfo, Spreadsheet], active: 0 }));
        // Reordering within a group.
        let tree = join_at(&root, Spreadsheet, GeoInfo).unwrap();
        assert_eq!(node_at(&tree, &[1, 1, 1]), Some(&DockNode::Tabs { panels: vec![Spreadsheet, GeoInfo], active: 0 }));
        assert_eq!(join_at(&root, GeoInfo, Spreadsheet), None, "already just before it");
        assert_eq!(join_at(&root, Viewport, Network), None);
        assert_eq!(join_at(&root, Network, Viewport), None);
        assert_eq!(join_at(&root, Network, Network), None);
    }

    #[test]
    fn reset_memory_forgets_dock_state() {
        let ctx = egui::Context::default();
        let geo = crate::ui::geo_info::dock_id();
        ctx.data_mut(|d| { d.insert_temp(drag_id(), Spreadsheet); d.insert_persisted(geo.with("ranges"), true); d.insert_persisted(geo.with("open"), false); });
        reset_memory(&ctx);
        assert_eq!(ctx.data(|d| d.get_temp::<PanelId>(drag_id())), None);
        assert_eq!(ctx.data_mut(|d| d.get_persisted::<bool>(geo.with("ranges"))), None);
        assert_eq!(ctx.data_mut(|d| d.get_persisted::<bool>(geo.with("open"))), None);
    }

    #[test]
    fn split_keeps_minimum_extents() {
        assert_eq!(split_at(0., 1000., 0.01), MIN_EXTENT);
        assert_eq!(split_at(10., 1000., 0.99), 10. + 1000. - MIN_EXTENT);
        assert_eq!(split_at(0., 100., 0.5), 50.);
    }
}
