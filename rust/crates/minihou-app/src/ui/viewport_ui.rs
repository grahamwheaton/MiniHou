//! Viewport chrome drawn over the 3-D view (design: Viewport.dc.html): header bar (selection modes, orientation, pivot,
//! snapping, proportional editing, overlays, X-ray, shading, Frame), left tool strip, bottom timeline, stats chip, view gizmo,
//! error and dense-mesh notes. `rect` is the viewport's area; the 3-D image is already painted into it.
use super::{theme::Theme, Actions, EditorChange, Falloff, Orientation, Pivot, SelectionMode, SnapTo, Tool, UiAction, View};
use crate::viewport::{Mode, ObjectVisibility};
use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
#[path = "view_gizmo.rs"] mod view_gizmo;
#[path = "ray_guide.rs"] mod ray_guide;

/// Render primitive handles over the viewport when a primitive generator is selected.
pub fn handles(ui: &mut egui::Ui, rect: Rect, v: &View, out: &mut Actions) -> bool {
    ray_guide::show(ui, rect, v);
    super::prim_handles::show(ui, rect, v, out)
}

const HEADER_H: f32 = 36.;
const TIMELINE_H: f32 = 44.;
/// Space for operator panels above the stats chip and timeline, beside the tools.
pub fn operator_panel_rect(rect: Rect) -> Rect {
    Rect::from_min_max(
        Pos2::new(rect.left() + 56., rect.top() + HEADER_H + 46.),
        Pos2::new(rect.right() - 12., rect.bottom() - TIMELINE_H - 50.),
    )
}

const TOOLS: [(&str, Icon); 14] = [
    ("Select (Q)", Icon::Select), ("3D cursor", Icon::Cursor), ("Move (G)", Icon::Move), ("Rotate (R)", Icon::Rotate),
    ("Scale (S)", Icon::Scale), ("Transform", Icon::Transform), ("Measure", Icon::Measure),
    ("Draw Line", Icon::Line), ("Draw Polyline (Enter finish)", Icon::Polyline),
    ("Draw Polygon (Enter finish)", Icon::Polygon), ("Draw Rectangle", Icon::Rectangle), ("Draw Circle", Icon::Circle),
    ("Knife (K): click points, Enter confirm", Icon::Knife), ("Loop Cut (Ctrl+R): hover a quad edge, wheel count, click then slide, click confirm", Icon::LoopCut),
];
/// Keep region presses off viewport chrome (header, timeline, tool strip and view gizmo).
pub fn selection_canvas_contains(rect: Rect, p: Pos2) -> bool {
    p.y > rect.top() + HEADER_H
        && !(p.x >= rect.left() + 56. && p.x <= rect.left() + 252. && p.y < rect.top() + HEADER_H + 34.) && p.y < rect.bottom() - TIMELINE_H
        && !(p.x < rect.left() + 52. && p.y < rect.top() + HEADER_H + 12. + TOOLS.len() as f32 * 28. + 28.)
        && !(p.x > rect.right() - 110. && p.y < rect.top() + HEADER_H + 115.)
}
const DENSE_PRIMS: usize = 40_000;
const FRAME_START: u32 = 0;
const FRAME_END: u32 = 240;

/// Viewport chrome state; frame changes are emitted as time edits.
#[derive(Clone)]
struct VpState { popover: Option<Popover>, frame: u32, playing: bool, clock: f32 }
pub fn timeline_command(ctx: &egui::Context, command: &str) -> Option<minihou_app::Edit> {
    let id = egui::Id::new("viewport_ui_state");
    let changed = ctx.data_mut(|d| {
        let mut st: VpState = d.get_temp(id).unwrap_or_default();
        let before = st.frame;
        match command {
            "timeline_toggle" => st.playing = !st.playing,
            "timeline_previous" => st.frame = st.frame.saturating_sub(1).max(FRAME_START),
            "timeline_next" => st.frame = (st.frame + 1).min(FRAME_END),
            "timeline_start" => { st.playing = false; st.frame = FRAME_START; }
            "timeline_end" => { st.playing = false; st.frame = FRAME_END; }
            _ => return None,
        }
        let edit = minihou_app::timeline_time_edit(before, st.frame, 24.);
        d.insert_temp(id, st);
        edit
    });
    ctx.request_repaint();
    changed
}
impl Default for VpState {
    fn default() -> Self {
        Self { popover: None, frame: 1, playing: false, clock: 0. }
    }
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Popover { Orientation, Pivot, Snap, Proportional, Overlays, Shading, Visibility }
fn toggle_popover(open: Option<Popover>, next: Popover) -> Option<Popover> { if open == Some(next) { None } else { Some(next) } }
const OVERLAYS: [(&str, &str); 6] = [("Grid floor", ""), ("Axis gizmo", ""), ("Edges in solid", ""), ("Stats", ""), ("Normals", ""), ("Bounding box", "")];

#[derive(Clone, Copy, PartialEq, Debug)]
enum Icon { Display(Mode), Visibility(ObjectVisibility), Object, Points, Edges, Faces, Orient, Pivot, Magnet, Prop, Overlays, Xray, Solid, Shaded, Wire, ShadedWire, Frame,
    Select, BoxSelect, CircleSelect, LassoSelect, Cursor, Move, Rotate, Scale, Transform, Measure, Line, Polyline, Polygon, Rectangle, Circle, Knife, LoopCut, ToStart, Prev, Play, Pause, Next, ToEnd }

/// Where every header control sits, from the viewport's left/top/width. Pure so it can be tested at any width.
#[derive(Debug)]
struct HeaderLayout { modes: [Rect; 4], orient: Rect, pivot: Rect, snap: Rect, prop: Rect, proxy: Rect, overlays: Rect, shading: Rect, visibility: Rect, frame: Rect, seps: Vec<f32>, groups: Vec<Rect> }
fn header_layout(left: f32, top: f32, width: f32, snap_on: bool) -> HeaderLayout {
    let cy = top + HEADER_H / 2.;
    let mk = |x: f32, w: f32, h: f32| Rect::from_min_size(Pos2::new(x, cy - h / 2.), Vec2::new(w, h));
    let mut x = left + 6.;
    let mut seps = vec![];
    let mut groups = vec![];
    let gstart = x;
    x += 2.;
    let modes = [0, 1, 2, 3].map(|i| mk(x + i as f32 * 25., 24., 22.));
    x += 4. * 25. - 1. + 2.;
    groups.push(Rect::from_min_max(Pos2::new(gstart, cy - 13.), Pos2::new(x, cy + 13.)));
    x += 4.; seps.push(x); x += 5.;
    let orient = mk(x, 82., 26.); x += 82. + 3.;
    let pivot = mk(x, 44., 26.); x += 44. + 3.;
    let snap_w = if snap_on && width > 720. { 76. } else { 44. };
    let snap = mk(x, snap_w, 26.); x += snap_w + 3.;
    let prop = mk(x, 44., 26.);
    // right end, laid from the right edge inwards
    let mut r = left + width - 6.;
    let frame = mk(r - 26., 26., 26.); r -= 26. + 3.;
    seps.push(r - 4.); r -= 9.;
    let visibility = mk(r - 42., 42., 26.); r -= 45.;
    let shading = mk(r - 42., 42., 26.); r -= 45.;
    seps.push(r - 4.); r -= 9.;
    let overlays = mk(r - 44., 44., 26.);
    let proxy_width = (overlays.left() - prop.right() - 16.).clamp(0., 100.);
    let proxy = mk(overlays.left() - 8. - proxy_width, proxy_width, 26.);
    HeaderLayout { modes, orient, pivot, snap, prop, proxy, overlays, shading, visibility, frame, seps, groups }
}

/// "node 'noise1' (noise): ValueError: ..." -> (Some("noise1"), "ValueError: ...")
fn split_error(error: &str) -> (Option<&str>, &str) {
    if let Some(rest) = error.strip_prefix("node '") {
        if let Some((id, tail)) = rest.split_once('\'') {
            let msg = tail.split_once("): ").map_or(tail.trim_start_matches([' ', ':']), |(_, m)| m);
            return (Some(id), msg);
        }
    }
    (None, error)
}

fn frame_at(x: f32, left: f32, right: f32) -> u32 {
    let t = ((x - left) / (right - left).max(1.)).clamp(0., 1.);
    FRAME_START + (t * (FRAME_END - FRAME_START) as f32).round() as u32
}

pub fn overlay(ui: &mut egui::Ui, rect: Rect, v: &View, out: &mut Actions) {
    show_shading_pie(ui.ctx(), rect, v, out);
    let ctx = ui.ctx().clone();
    let id = egui::Id::new("viewport_ui_state");
    let mut st: VpState = ctx.data_mut(|d| d.get_temp(id)).unwrap_or_default();
    let previous_frame = st.frame;
    let th = v.theme;
    let painter = ui.painter_at(rect);
    let stroke = Stroke::new(1.0_f32, th.stroke);

    // ---- header bar
    painter.rect_filled(Rect::from_min_size(rect.min, Vec2::new(rect.width(), HEADER_H)), 0., th.panel);
    painter.line_segment([Pos2::new(rect.left(), rect.top() + HEADER_H), Pos2::new(rect.right(), rect.top() + HEADER_H)], stroke);
    let ed = v.editor;
    let show = [ed.overlays.grid, ed.overlays.axes, ed.overlays.wireframe, ed.overlays.stats, ed.overlays.normals, ed.overlays.bounds].map(|b| b && ed.overlays.enabled);
    let h = header_layout(rect.left(), rect.top(), rect.width(), ed.snap.enabled);
    for g in &h.groups { painter.rect(*g, 5., th.panel2, stroke, StrokeKind::Inside); }
    for x in &h.seps { painter.line_segment([Pos2::new(*x, rect.top() + 9.), Pos2::new(*x, rect.top() + HEADER_H - 9.)], stroke); }
    for (i, r) in h.modes.iter().enumerate() {
        let mode = SelectionMode::ALL[i];
        if button(ui, th, *r, ("vp_mode", i), ed.selection == mode, ["Points (1)", "Edges (2)", "Faces (3)", "Objects (4)"][i], [Icon::Points, Icon::Edges, Icon::Faces, Icon::Object][i]) { out.push(UiAction::Editor(EditorChange::Selection(mode))); }
    }
    if button_text(ui, th, h.orient, "vp_orient", st.popover == Some(Popover::Orientation), "Transform orientation", Icon::Orient, (Some(ed.orientation.label()), true)) { st.popover = toggle_popover(st.popover, Popover::Orientation); }
    if button_text(ui, th, h.pivot, "vp_pivot", st.popover == Some(Popover::Pivot), "Pivot point", Icon::Pivot, (None, true)) { st.popover = toggle_popover(st.popover, Popover::Pivot); }
    let snap_label = if ed.snap.to == SnapTo::Increment.into() { crate::units::format(ed.snap.increment as f64, v.units) }
        else if ed.snap.to.iter().count()>1 { "Multi".into() }
        else { match ed.snap.to.iter().next().unwrap_or_default() {
            SnapTo::Vertex=>"Vtx",SnapTo::Edge=>"Edge",SnapTo::Face=>"Face",SnapTo::Volume=>"Vol",SnapTo::EdgeCenter=>"Mid",SnapTo::Increment=>"Grid"
        }.into() };
    let snap_main = Rect::from_min_max(h.snap.min, Pos2::new(h.snap.right() - 16., h.snap.bottom()));
    if button_text(ui, th, snap_main, "vp_snap_toggle", ed.snap.enabled, "Toggle snapping (Shift+Tab)", Icon::Magnet, (if ed.snap.enabled && h.snap.width() > 44. { Some(snap_label.as_str()) } else { None }, false)) { out.push(UiAction::Editor(EditorChange::SnapEnabled(!ed.snap.enabled))); }
    if caret_button(ui, th, h.snap, "vp_snap_menu", st.popover == Some(Popover::Snap)) { st.popover = toggle_popover(st.popover, Popover::Snap); }
    let prop_main = Rect::from_min_max(h.prop.min, Pos2::new(h.prop.right() - 16., h.prop.bottom()));
    if button_text(ui, th, prop_main, "vp_prop_toggle", ed.proportional.enabled, "Toggle proportional editing (O)", Icon::Prop, (None, false)) { out.push(UiAction::Editor(EditorChange::ProportionalEnabled(!ed.proportional.enabled))); }
    if caret_button(ui, th, h.prop, "vp_prop_menu", st.popover == Some(Popover::Proportional)) { st.popover = toggle_popover(st.popover, Popover::Proportional); }
    ui.scope_builder(egui::UiBuilder::new().max_rect(h.proxy), |ui| {
        ui.set_clip_rect(h.proxy);
        super::proxy_ui::menu(ui, v, out, h.proxy.width() < 80.);
    });
    let overlay_main=Rect::from_min_max(h.overlays.min,Pos2::new(h.overlays.right()-16.,h.overlays.bottom()));
    if button_text(ui,th,overlay_main,"vp_overlays_toggle",ed.overlays.enabled,"Toggle all overlays",Icon::Overlays,(None,false)) {
        let mut o=ed.overlays; o.enabled=!o.enabled; out.push(UiAction::Editor(EditorChange::Overlays(o)));
    }
    if caret_button(ui,th,h.overlays,"vp_overlays_menu",st.popover==Some(Popover::Overlays)) { st.popover=toggle_popover(st.popover,Popover::Overlays); }
    if button_text(ui, th, h.shading, "vp_shading", st.popover == Some(Popover::Shading), v.mode.label(), Icon::Display(v.mode), (None,true)) { st.popover=toggle_popover(st.popover,Popover::Shading); }
    if button_text(ui, th, h.visibility, "vp_visibility", st.popover == Some(Popover::Visibility), ed.object_visibility.label(), Icon::Visibility(ed.object_visibility), (None,true)) { st.popover=toggle_popover(st.popover,Popover::Visibility); }
    if button(ui, th, h.frame, ("vp_frame", 0), false, "Frame the mesh (Home)", Icon::Frame) { out.push(UiAction::Frame); }

    // ---- tool strip
    let strip_h = TOOLS.len() as f32 * 28. + 4. * 5. + 8.;
    let strip = Rect::from_min_size(Pos2::new(rect.left() + 12., rect.top() + HEADER_H + 12.), Vec2::new(36., strip_h));
    painter.rect(strip, 6., th.panel, stroke, StrokeKind::Inside);
    let mut y = strip.top() + 4.;
    for (i, (name, icon)) in TOOLS.iter().enumerate() {
        if i == 2 || i == 6 || i == 7 || i == 12 {
            painter.line_segment([Pos2::new(strip.left() + 6., y + 2.), Pos2::new(strip.right() - 6., y + 2.)], stroke);
            y += 5.;
        }
        let r = Rect::from_min_size(Pos2::new(strip.left() + 4., y), Vec2::splat(28.));
        if button(ui, th, r, ("vp_tool", i), ed.tool == Tool::ALL[i], name, *icon) { out.push(UiAction::Editor(EditorChange::Tool(Tool::ALL[i]))); }
        y += 28.;
    }

    // Active Select tool options, separate from the tool strip and one-shot hotkeys.
    if ed.tool == Tool::Select && !super::sketch::active(&ctx) {
        use super::selection_region::{self, Kind};
        use crate::picking::SelectOp;
        let mut options = selection_region::options(&ctx);
        let bar = Rect::from_min_size(Pos2::new(rect.left() + 56., rect.top() + HEADER_H + 4.), Vec2::new(196., 28.));
        painter.rect(bar, 4., th.panel, stroke, StrokeKind::Inside);
        for (i, (kind, ic, tip)) in [
            (Kind::Box, Icon::BoxSelect, "Select Box (B for one-shot box)"),
            (Kind::Circle, Icon::CircleSelect, "Select Circle (C for modal circle)"),
            (Kind::Lasso, Icon::LassoSelect, "Select Lasso (Ctrl+RMB drag)"),
        ].into_iter().enumerate() {
            let r = Rect::from_min_size(bar.min + Vec2::new(2. + i as f32 * 28., 2.), Vec2::splat(24.));
            if button(ui, th, r, ("vp_select_kind", i), options.kind == kind, tip, ic) {
                options.kind = kind;
                selection_region::set_options(&ctx, options);
            }
        }
        for (i, (op, label, tip)) in [
            (SelectOp::Replace, "=", "Set: replace selection"),
            (SelectOp::Add, "+", "Extend selection (Shift)"),
            (SelectOp::Subtract, "-", "Subtract selection (Ctrl)"),
        ].into_iter().enumerate() {
            let r = Rect::from_min_size(bar.min + Vec2::new(102. + i as f32 * 30., 2.), Vec2::new(28., 24.));
            if ui.put(r, egui::Button::new(label).selected(options.op == op)).on_hover_text(tip).clicked() {
                options.op = op;
                selection_region::set_options(&ctx, options);
            }
        }
    }

    // ---- timeline
    let tl = Rect::from_min_max(Pos2::new(rect.left(), rect.bottom() - TIMELINE_H), rect.max);
    painter.rect_filled(tl, 0., th.panel);
    painter.line_segment([tl.left_top(), tl.right_top()], stroke);
    let cy = tl.center().y;
    let mut x = tl.left() + 10.;
    let group = Rect::from_min_size(Pos2::new(x, cy - 13.), Vec2::new(5. * 25. + 3., 26.));
    painter.rect(group, 5., th.panel2, stroke, StrokeKind::Inside);
    x += 2.;
    for (i, (icon, name)) in [(Icon::ToStart, "Jump to start"), (Icon::Prev, "Previous frame"), (if st.playing { Icon::Pause } else { Icon::Play }, "Play (Space)"), (Icon::Next, "Next frame"), (Icon::ToEnd, "Jump to end")].into_iter().enumerate() {
        let r = Rect::from_min_size(Pos2::new(x + i as f32 * 25., cy - 11.), Vec2::new(24., 22.));
        if button(ui, th, r, ("vp_tl", i), i == 2 && st.playing, name, icon) {
            match i {
                0 => st.frame = FRAME_START,
                1 => st.frame = st.frame.saturating_sub(1).max(FRAME_START),
                2 => st.playing = !st.playing,
                3 => st.frame = (st.frame + 1).min(FRAME_END),
                _ => st.frame = FRAME_END,
            }
        }
    }
    x = group.right() + 12.;
    let mono = egui::FontId::monospace(11.);
    let label = painter.text(Pos2::new(x, cy), egui::Align2::LEFT_CENTER, "Frame", mono.clone(), th.dim);
    x = label.right() + 8.;
    let val = Rect::from_min_size(Pos2::new(x, cy - 11.), Vec2::new(44., 22.));
    painter.rect(val, 4., th.panel2, stroke, StrokeKind::Inside);
    painter.text(Pos2::new(val.right() - 6., cy), egui::Align2::RIGHT_CENTER, st.frame.to_string(), egui::FontId::monospace(12.), th.text);
    let readout = format!("Start {FRAME_START}  End {FRAME_END}  24 fps");
    let rg = painter.layout_no_wrap(readout, mono.clone(), th.dim);
    let rx = tl.right() - 10. - rg.size().x;
    painter.galley(Pos2::new(rx, cy - rg.size().y / 2.), rg, th.dim);
    let bar = Rect::from_min_max(Pos2::new(val.right() + 10., cy - 12.), Pos2::new(rx - 12., cy + 12.));
    if bar.width() > 40. {
        painter.rect(bar, 4., th.panel2, stroke, StrokeKind::Inside);
        let mut tx = bar.left() + 24.;
        while tx < bar.right() { painter.line_segment([Pos2::new(tx, bar.bottom() - 8.), Pos2::new(tx, bar.bottom() - 1.)], stroke); tx += 24.; }
        let at = |f: f32| bar.left() + f * bar.width();
        for f in [0., 0.35, 0.7] {
            let c = Pos2::new(at(f).max(bar.left() + 5.), bar.top() + 11.);
            painter.add(egui::Shape::convex_polygon(vec![c + Vec2::new(0., -4.), c + Vec2::new(4., 0.), c + Vec2::new(0., 4.), c + Vec2::new(-4., 0.)], th.warn, Stroke::NONE));
        }
        let resp = ui.interact(bar, egui::Id::new("vp_scrub"), Sense::click_and_drag());
        if let Some(p) = resp.interact_pointer_pos() { if resp.dragged() || resp.clicked() { st.frame = frame_at(p.x, bar.left(), bar.right()); } }
        let px = at((st.frame - FRAME_START) as f32 / (FRAME_END - FRAME_START) as f32);
        painter.rect_filled(Rect::from_min_max(Pos2::new(px - 1., bar.top() - 1.), Pos2::new(px + 1., bar.bottom() + 1.)), 0., th.accent);
    }
    if st.playing {
        st.clock += ctx.input(|i| i.stable_dt);
        let step = (st.clock * 24.) as u32;
        if step > 0 { st.clock -= step as f32 / 24.; st.frame = FRAME_START + (st.frame - FRAME_START + step) % (FRAME_END - FRAME_START + 1); }
        ctx.request_repaint();
    }

    // ---- stats chip, axis gizmo
    let (points, prims) = v.cooking.geometry.as_ref().map_or((0, 0), |g| (g.npoints, g.nprims()));
    if show[3] {
        let text = format!("Points {points}   Prims {prims}   Cook {:.1} ms", v.cooking.milliseconds);
        let g = painter.layout_no_wrap(text, mono.clone(), th.text);
        let chip = Rect::from_min_size(Pos2::new(rect.left() + 12., tl.top() - 12. - 26.), Vec2::new(g.size().x + 20., 26.));
        painter.rect(chip, 6., th.panel, stroke, StrokeKind::Inside);
        painter.galley(Pos2::new(chip.left() + 10., chip.center().y - g.size().y / 2.), g, th.text);
    }
    if show[1] { view_gizmo::show(ui, rect, v, out); }

    // ---- notes
    if let Some(error) = &v.cooking.error {
        let (node, msg) = split_error(error);
        let title = match node { Some(n) => format!("Cook failed at {n}."), None => "Cook failed.".into() };
        note(&painter, th, Pos2::new(rect.left() + 56., rect.top() + HEADER_H + 12.), (th.err, th.err_bg), (&title, &format!("{msg}  Showing the last good mesh.")), rect.width() - 68.);
    } else if prims > DENSE_PRIMS {
        note(&painter, th, Pos2::new(rect.left() + 56., tl.top() - 12. - 40.), (th.warn, th.warn_bg), ("Mesh is dense.", "Edges are fading out. Zoom in or lower rows and columns to see them."), rect.width() - 190.);
    }

    if let Some(which) = st.popover {
        let anchor = match which { Popover::Orientation => h.orient, Popover::Pivot => h.pivot, Popover::Snap => h.snap, Popover::Proportional => h.prop, Popover::Overlays => h.overlays, Popover::Shading => h.shading, Popover::Visibility => h.visibility };
        let before=out.0.len();
        let pop = show_popover(ui, rect, anchor, which, v, out);
        if out.0.len()>before && matches!(which,Popover::Shading | Popover::Visibility) { st.popover=None; }
        let dismiss = ctx.input(|i| i.key_pressed(egui::Key::Escape) || (i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !pop.contains(p) && !anchor.contains(p))));
        if dismiss { st.popover = None; }
    }
    if let Some(edit) = minihou_app::timeline_time_edit(previous_frame, st.frame, 24.) { out.push(UiAction::Time(edit)); }
    ctx.data_mut(|d| d.insert_temp(id, st));
}

fn note(p: &egui::Painter, th: &Theme, at: Pos2, (edge, fill): (Color32, Color32), (title, body): (&str, &str), max_w: f32) {
    let t = p.layout_no_wrap(title.to_owned(), egui::FontId::proportional(12.), th.text);
    let b = p.layout(body.to_owned(), egui::FontId::monospace(11.), th.dim, (max_w - 24.).max(80.));
    let w = t.size().x.max(b.size().x) + 24.;
    let r = Rect::from_min_size(at, Vec2::new(w, t.size().y + b.size().y + 16.));
    p.rect(r, 6., fill, Stroke::new(1.0_f32, edge), StrokeKind::Inside);
    p.galley(r.min + Vec2::new(12., 6.), t.clone(), th.text);
    p.galley(r.min + Vec2::new(12., 8. + t.size().y), b, th.dim);
}

fn caret_button(ui: &mut egui::Ui, th: &Theme, whole: Rect, id: &'static str, active: bool) -> bool {
    let r = Rect::from_min_max(Pos2::new(whole.right() - 16., whole.top()), whole.max);
    let resp = ui.interact(r, egui::Id::new(id), Sense::click());
    if active { ui.painter().rect_filled(r, 4., th.accent_bg); }
    else if resp.hovered() { ui.painter().rect_filled(r, 4., th.panel2); }
    let c = r.center();
    ui.painter().add(egui::Shape::line(vec![c + Vec2::new(-3., -1.5), c + Vec2::new(0., 1.5), c + Vec2::new(3., -1.5)], Stroke::new(1.3_f32, th.dim)));
    resp.clicked()
}

#[allow(clippy::too_many_arguments)] // one compact painter call keeps the menu rows consistent
fn pop_row(ui: &mut egui::Ui, r: Rect, id: impl std::hash::Hash, label: &str, selected: bool, check: bool, key: &str, th: &Theme) -> bool {
    let resp = ui.interact(r, egui::Id::new(("vp_pop_row", id)), Sense::click());
    let p = ui.painter();
    if resp.hovered() { p.rect_filled(r, 3., th.panel2); }
    let mark = Rect::from_center_size(Pos2::new(r.left() + 13., r.center().y), Vec2::splat(14.));
    if check {
        p.rect(mark, 3., if selected { th.accent } else { th.panel }, Stroke::new(1.0_f32, if selected { th.accent } else { th.dim }), StrokeKind::Inside);
        if selected { icon(p, Icon::Frame, mark.shrink(2.), th.panel, true); }
    } else if selected { p.text(mark.center(), egui::Align2::CENTER_CENTER, "✓", egui::FontId::proportional(13.), th.accent); }
    p.text(Pos2::new(r.left() + 30., r.center().y), egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(12.), th.text);
    if !key.is_empty() { p.text(Pos2::new(r.right() - 8., r.center().y), egui::Align2::RIGHT_CENTER, key, egui::FontId::monospace(11.), th.dim); }
    resp.clicked()
}

fn numeric_row(ui: &mut egui::Ui, r: Rect, id: &'static str, label: &str, value: f32, th: &Theme, units: Option<crate::units::UnitSettings>) -> Option<f32> {
    let p = ui.painter();
    p.text(Pos2::new(r.left() + 8., r.center().y), egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(12.), th.text);
    let box_r = Rect::from_min_size(Pos2::new(r.right() - 88., r.center().y - 11.), Vec2::new(80., 22.));
    p.rect(box_r, 4., th.input, Stroke::new(1.0_f32, th.stroke), StrokeKind::Inside);
    let edit_id = egui::Id::new(("vp_number_edit", id));
    if let Some(mut text) = ui.ctx().data(|d| d.get_temp::<String>(edit_id)) {
        let resp = ui.put(box_r.shrink2(Vec2::new(3., 1.)), egui::TextEdit::singleline(&mut text).font(egui::FontId::monospace(12.)).frame(false).id(edit_id.with("text")));
        if !resp.has_focus() && !resp.lost_focus() { resp.request_focus(); }
        if resp.lost_focus() {
            ui.ctx().data_mut(|d| d.remove::<String>(edit_id));
            let parsed = if let Some(u) = units {
                u.parse(&text).map(|x| x as f32)
            } else {
                text.trim().parse::<f32>().ok()
            };
            return parsed.filter(|x| x.is_finite()).map(|x| x.max(1e-4));
        }
        ui.ctx().data_mut(|d| d.insert_temp(edit_id, text));
    } else {
        let resp = ui.interact(box_r, edit_id, Sense::click_and_drag());
        if resp.double_clicked() {
            let initial = if let Some(u) = units {
                u.format(value as f64)
            } else {
                format!("{value}")
            };
            ui.ctx().data_mut(|d| d.insert_temp(edit_id, initial));
        }
        if resp.dragged() {
            let dx = ui.ctx().input(|i| i.pointer.delta().x);
            if dx != 0. { return Some((value + dx * value.max(0.1) * 0.01).max(1e-4)); }
        }
        let display_str = if let Some(u) = units {
            u.format(value as f64)
        } else {
            format!("{value:.2}")
        };
        p.text(Pos2::new(box_r.right() - 6., box_r.center().y), egui::Align2::RIGHT_CENTER, display_str, egui::FontId::monospace(12.), th.text);
    }
    None
}

fn falloff_value(f: Falloff, x: f32) -> f32 {
    let t = (1. - x).clamp(0., 1.);
    match f {
        Falloff::Smooth => t * t * (3. - 2. * t), Falloff::Sphere => (1. - x * x).max(0.).sqrt(),
        Falloff::Root => t.sqrt(), Falloff::InverseSquare => t * t, Falloff::Sharp => t * t * t,
        Falloff::Linear => t, Falloff::Constant => if x < 1. { 1. } else { 0. },
        Falloff::Random => ((x * 37.).sin() * 0.35 + 0.55) * t,
    }
}
fn falloff_samples(f: Falloff) -> [f32; 13] { std::array::from_fn(|i| falloff_value(f, i as f32 / 12.)) }

fn show_popover(ui: &mut egui::Ui, viewport: Rect, anchor: Rect, which: Popover, v: &View, out: &mut Actions) -> Rect {
    let th = v.theme;
    let ed = v.editor;
    let rows = match which { Popover::Orientation => 6, Popover::Pivot => 5, Popover::Snap => 11, Popover::Proportional => 12, Popover::Overlays => 19, Popover::Shading => 18, Popover::Visibility => 3 };
    let height = 30. + rows as f32 * 24. + 10.;
    let x = (anchor.right() - 232.).clamp(viewport.left() + 6., (viewport.right() - 238.).max(viewport.left() + 6.));
    let pop = Rect::from_min_size(Pos2::new(x, anchor.bottom() + 4.), Vec2::new(232., height));
    let p = ui.painter();
    p.rect(pop, 6., th.panel, Stroke::new(1.0_f32, th.stroke), StrokeKind::Inside);
    let title = match which { Popover::Orientation => "ORIENTATION", Popover::Pivot => "PIVOT POINT", Popover::Snap => "SNAPPING", Popover::Proportional => "PROPORTIONAL EDITING", Popover::Overlays => "OVERLAYS", Popover::Shading => "DISPLAY MODE", Popover::Visibility => "OBJECT VISIBILITY" };
    p.text(pop.min + Vec2::new(12., 17.), egui::Align2::LEFT_CENTER, title, egui::FontId::proportional(11.), th.dim);
    let row = |i: usize| Rect::from_min_size(pop.min + Vec2::new(6., 28. + i as f32 * 24.), Vec2::new(220., 24.));
    match which {
        Popover::Shading => {
            for (i,mode) in Mode::ALL.into_iter().enumerate() {
                if display_row(ui,row(i),("display",i),mode.label(),v.mode==mode,Icon::Display(mode),th) { out.push(UiAction::SetMode(mode)); }
            }
            section(ui,row(12),"MATCAP",th);
            for (i,matcap) in crate::viewport::MatCap::ALL.into_iter().enumerate() {
                let r=row(13+i);
                if pop_row(ui,r,("matcap",i),matcap.label(),ed.matcap==matcap,false,"",th) {
                    out.push(UiAction::Editor(EditorChange::MatCap(matcap)));
                }
                matcap_swatch(ui,r,matcap);
            }
            if pop_row(ui,row(15),"matcap_flat","Flat normals",ed.matcap_flat,true,"",th) {
                out.push(UiAction::Editor(EditorChange::MatCapFlat(!ed.matcap_flat)));
            }
            let active=matches!(v.mode,Mode::FlatShaded|Mode::FlatWireShaded|Mode::SmoothShaded|Mode::SmoothWireShaded);
            if pop_row(ui,row(16),"materials_textures","Materials / Textures",ed.textures,true,if active {""}else{"Shaded modes only"},th) {
                out.push(UiAction::Editor(EditorChange::Textures(!ed.textures)));
            }
            if pop_row(ui,row(17),"texture_inspector","Texture inspector",ed.texture_inspector,true,"",th) {
                out.push(UiAction::Editor(EditorChange::TextureInspector(!ed.texture_inspector)));
            }
        },
        Popover::Visibility => for (i,visibility) in ObjectVisibility::ALL.into_iter().enumerate() {
            if display_row(ui,row(i),("visibility",i),visibility.label(),ed.object_visibility==visibility,Icon::Visibility(visibility),th) { out.push(UiAction::Editor(EditorChange::ObjectVisibility(visibility))); }
        },
        Popover::Orientation => for (i, o) in Orientation::ALL.iter().enumerate() {
            if pop_row(ui, row(i), ("orientation", i), o.label(), *o == ed.orientation, false, "", th) { out.push(UiAction::Editor(EditorChange::Orientation(*o))); }
        },
        Popover::Pivot => for (i, pivot) in Pivot::ALL.iter().enumerate() {
            if pop_row(ui, row(i), ("pivot", i), pivot.label(), *pivot == ed.pivot, false, "", th) { out.push(UiAction::Editor(EditorChange::Pivot(*pivot))); }
        },
        Popover::Snap => {
            if pop_row(ui, row(0), ("snap_enabled", 0), "Enable snapping", ed.snap.enabled, true, "", th) { out.push(UiAction::Editor(EditorChange::SnapEnabled(!ed.snap.enabled))); }
            section(ui, row(1), "SNAP TO", th);
            for (i, to) in SnapTo::ALL.iter().enumerate() {
                if pop_row(ui, row(2 + i), ("snap_to", i), to.label(), ed.snap.to.contains(*to), true, "", th) { out.push(UiAction::Editor(EditorChange::SnapTo(*to))); }
            }
            if let Some(x) = numeric_row(ui, row(8), "snap_increment", "Increment", ed.snap.increment, th, Some(v.units)) { out.push(UiAction::Editor(EditorChange::SnapIncrement(x))); }
            // The final two rows extend the standard spacing so both toggles remain readable.
            let extra = row(9);
            if pop_row(ui, extra, ("snap_align", 0), "Align rotation to target", ed.snap.align_rotation, true, "", th) { out.push(UiAction::Editor(EditorChange::SnapAlignRotation(!ed.snap.align_rotation))); }
            if pop_row(ui, row(10), ("snap_backface", 0), "Backface culling", ed.snap.backface_culling, true, "", th) { out.push(UiAction::Editor(EditorChange::SnapBackfaceCulling(!ed.snap.backface_culling))); }
        }
        Popover::Proportional => {
            if pop_row(ui, row(0), ("prop_enabled", 0), "Enable proportional editing", ed.proportional.enabled, true, "", th) { out.push(UiAction::Editor(EditorChange::ProportionalEnabled(!ed.proportional.enabled))); }
            section(ui, row(1), "FALLOFF", th);
            for (i, falloff) in Falloff::ALL.iter().enumerate() {
                let r = row(2 + i);
                if pop_row(ui, r, ("falloff", i), falloff.label(), *falloff == ed.proportional.falloff, false, "", th) { out.push(UiAction::Editor(EditorChange::ProportionalFalloff(*falloff))); }
                let pts = falloff_samples(*falloff).iter().enumerate().map(|(j, y)| Pos2::new(r.right() - 34. + j as f32 * 2., r.center().y + 6. - y * 12.)).collect();
                ui.painter().add(egui::Shape::line(pts, Stroke::new(1.0_f32, th.dim)));
            }
            if let Some(x) = numeric_row(ui, row(10), "prop_radius", "Radius", ed.proportional.radius, th, Some(v.units)) { out.push(UiAction::Editor(EditorChange::ProportionalRadius(x))); }
            if pop_row(ui, row(11), ("prop_connected", 0), "Connected only", ed.proportional.connected_only, true, "", th) { out.push(UiAction::Editor(EditorChange::ProportionalConnected(!ed.proportional.connected_only))); }
        }
        Popover::Overlays => {
            section(ui,row(0),"GENERAL",th);
            let mut o=ed.overlays;
            let values=[o.grid,o.axes,o.stats,o.bounds,o.normals,o.wireframe];
            for (i,label) in ["Grid","Axes / Floor","Statistics","Bounds","Normals","Wireframe overlay"].iter().enumerate() {
                if pop_row(ui,row(i+1),("general_overlay",i),label,values[i],true,"",th) {
                    match i {0=>o.grid=!o.grid,1=>o.axes=!o.axes,2=>o.stats=!o.stats,3=>o.bounds=!o.bounds,4=>o.normals=!o.normals,_=>o.wireframe=!o.wireframe}
                }
            }
            if pop_row(ui,row(7),"overlay_xray","X-ray",ed.xray,true,"Alt+Z",th) { out.push(UiAction::Editor(EditorChange::Xray(!ed.xray))); }
            section(ui,row(8),"GEOMETRY",th);
            for (i,(label,value)) in [("Points",o.points),("Lines (curves / edges)",o.lines),("Faces",o.faces)].into_iter().enumerate() {
                if pop_row(ui,row(i+9),("geometry_overlay",i),label,value,true,"",th) {
                    match i {0=>o.points=!o.points,1=>o.lines=!o.lines,_=>o.faces=!o.faces}
                }
            }
            ui.scope_builder(egui::UiBuilder::new().max_rect(row(12)),|ui| {ui.add(egui::Slider::new(&mut o.point_size,1.0..=16.0).text("Point size"));});
            ui.scope_builder(egui::UiBuilder::new().max_rect(row(13)),|ui| {ui.add(egui::Slider::new(&mut o.line_width,1.0..=10.0).text("Line width"));});
            if o!=ed.overlays { out.push(UiAction::Editor(EditorChange::Overlays(o))); }
            section(ui,row(14),"COLOR",th);
            for (i,color) in crate::viewport::ObjectColor::ALL.into_iter().enumerate() {
                if pop_row(ui,row(15+i),("object_color",i),color.label(),ed.object_color==color,false,"",th) {
                    out.push(UiAction::Editor(EditorChange::ObjectColor(color)));
                }
            }
            if pop_row(ui,row(17),"overlay_materials","Materials / Textures",ed.textures,true,"",th) {
                out.push(UiAction::Editor(EditorChange::Textures(!ed.textures)));
            }
            if pop_row(ui,row(18),"overlay_texture_inspector","Texture inspector",ed.texture_inspector,true,"",th) {
                out.push(UiAction::Editor(EditorChange::TextureInspector(!ed.texture_inspector)));
            }
        }
    }
    pop
}

fn matcap_swatch(ui:&egui::Ui, row:Rect, matcap:crate::viewport::MatCap) {
    let centre=Pos2::new(row.left()+17.,row.center().y);
    let mut mesh=egui::Mesh::default();
    let rings=4; let sectors=20; let radius=9.;
    for ring in 0..=rings { for sector in 0..sectors {
        let r=ring as f32/rings as f32;
        let angle=std::f32::consts::TAU*sector as f32/sectors as f32;
        let (x,y)=(r*angle.cos(),r*angle.sin());
        let rgb=if matcap==crate::viewport::MatCap::Normal {
            crate::viewport::display_modes::normal_color([x,-y,(1.-r*r).sqrt()])
        } else { let v=0.68+0.23*(1.-r*r).sqrt()-0.14*y; [v*0.99,v*0.95,v*0.9] };
        mesh.vertices.push(egui::epaint::Vertex {pos:centre+Vec2::new(x*radius,y*radius),uv:egui::epaint::WHITE_UV,color:Color32::from_rgb((rgb[0]*255.) as u8,(rgb[1]*255.) as u8,(rgb[2]*255.) as u8)});
    } }
    for ring in 0..rings { for sector in 0..sectors {
        let a=(ring*sectors+sector) as u32;
        let b=(ring*sectors+(sector+1)%sectors) as u32;
        let c=((ring+1)*sectors+sector) as u32;
        let d=((ring+1)*sectors+(sector+1)%sectors) as u32;
        mesh.indices.extend([a,c,b,b,c,d]);
    } }
    ui.painter().add(egui::Shape::mesh(mesh));
}

fn section(ui: &egui::Ui, r: Rect, label: &str, th: &Theme) {
    ui.painter().line_segment([r.left_top() + Vec2::new(7., 1.), r.right_top() + Vec2::new(-7., 1.)], Stroke::new(1.0_f32, th.stroke));
    ui.painter().text(r.min + Vec2::new(8., 13.), egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(11.), th.dim);
}

/// Icon-only button with a tooltip; true when clicked.
fn button(ui: &mut egui::Ui, th: &Theme, r: Rect, id: (&str, usize), active: bool, tip: &str, ic: Icon) -> bool {
    button_text(ui, th, r, id, active, tip, ic, (None, false))
}
#[allow(clippy::too_many_arguments)] // a painter-style helper; a params struct would only add noise at the 6 call sites
fn button_text(ui: &mut egui::Ui, th: &Theme, r: Rect, id: impl std::hash::Hash, active: bool, tip: &str, ic: Icon, (text, caret): (Option<&str>, bool)) -> bool {
    let resp = ui.interact(r, egui::Id::new(("vp_btn", id)), Sense::click()).on_hover_text(tip);
    let p = ui.painter();
    if active { p.rect(r, 4., th.accent_bg, Stroke::new(1.0_f32, th.accent), StrokeKind::Inside); }
    else if resp.hovered() { p.rect_filled(r, 4., th.panel2); }
    let col = th.text;
    let mut x = r.left() + if text.is_some() || caret { 6. } else { (r.width() - 16.) / 2. };
    icon(p, ic, Rect::from_min_size(Pos2::new(x, r.center().y - 8.), Vec2::splat(16.)), col, false);
    x += 16. + 5.;
    if let Some(t) = text { p.text(Pos2::new(x, r.center().y), egui::Align2::LEFT_CENTER, t, egui::FontId::proportional(12.), th.text); }
    if caret {
        let c = Pos2::new(r.right() - 9., r.center().y);
        p.add(egui::Shape::line(vec![c + Vec2::new(-3., -1.5), c + Vec2::new(0., 1.5), c + Vec2::new(3., -1.5)], Stroke::new(1.3_f32, th.dim)));
    }
    resp.clicked()
}

/// The design's 16 px line icons, drawn with the painter (1.4 px stroke, round joins).
fn icon(p: &egui::Painter, ic: Icon, r: Rect, col: Color32, tick: bool) {
    let s = r.width() / 16.;
    let at = |x: f32, y: f32| r.min + Vec2::new(x * s, y * s);
    let st = Stroke::new(1.4_f32 * s.max(0.8), col);
    let line = |pts: &[(f32, f32)]| p.add(egui::Shape::line(pts.iter().map(|&(x, y)| at(x, y)).collect(), st));
    let poly = |pts: &[(f32, f32)]| p.add(egui::Shape::closed_line(pts.iter().map(|&(x, y)| at(x, y)).collect(), st));
    let fill_poly = |pts: &[(f32, f32)]| p.add(egui::Shape::convex_polygon(pts.iter().map(|&(x, y)| at(x, y)).collect(), col, Stroke::NONE));
    let circle = |x: f32, y: f32, rad: f32| p.circle_stroke(at(x, y), rad * s, st);
    let dot = |x: f32, y: f32, rad: f32| p.circle_filled(at(x, y), rad * s, col);
    if tick { line(&[(1.8, 8.2), (6., 12.2), (14.2, 3.4)]); return; }
    match ic {
        Icon::Display(mode) => {
            let faces=[vec![at(2.,4.),at(8.,1.5),at(14.,4.),at(8.,7.)],vec![at(2.,4.),at(8.,7.),at(8.,14.5),at(3.,11.5)],vec![at(8.,7.),at(14.,4.),at(13.,11.5),at(8.,14.5)]];
            let tint=if mode.bounds() { Color32::from_rgb(153,209,68) } else if mode.matcap() { Color32::from_rgb(172,186,214) } else { col };
            let filled=mode.shaded() || mode==Mode::WireframeGhost || mode==Mode::HiddenLineGhost;
            for (i,face) in faces.into_iter().enumerate() {
                let alpha=if matches!(mode,Mode::WireframeGhost | Mode::HiddenLineGhost) { 0.2 } else { [0.85,0.4,0.6][i] };
                p.add(egui::Shape::convex_polygon(face,if filled { tint.gamma_multiply(alpha) } else { Color32::TRANSPARENT },Stroke::new(0.9*s,tint)));
            }
            if mode.wire_shaded() || mode==Mode::Wireframe { line(&[(2.,4.),(13.,11.5)]); }
            if mode==Mode::HiddenLineGhost { line(&[(3.,11.5),(8.,9.),(13.,11.5)]); }
        }
        Icon::Visibility(visibility) => {
            let other=if visibility==ObjectVisibility::ShowAll { col } else if visibility==ObjectVisibility::GhostOthers { col.gamma_multiply(0.4) } else { Color32::TRANSPARENT };
            p.circle(at(5.,6.),4.*s,other,Stroke::new(s,if visibility==ObjectVisibility::HideOthers { Color32::TRANSPARENT } else { col.gamma_multiply(0.65) }));
            p.circle(at(10.5,10.),4.5*s,Color32::from_rgb(79,132,186),Stroke::new(s,col));
            p.circle_filled(at(9.,8.),1.3*s,col.gamma_multiply(0.7));
        }

        Icon::Object => { poly(&[(8., 1.5), (13.5, 4.3), (13.5, 11.7), (8., 14.5), (2.5, 11.7), (2.5, 4.3)]); line(&[(2.5, 4.3), (8., 7.2), (13.5, 4.3)]); line(&[(8., 7.2), (8., 14.5)]); }
        Icon::Points => { for (x, y) in [(4., 4.), (12., 4.), (4., 12.), (12., 12.)] { dot(x, y, 1.7); } }
        Icon::Edges => { poly(&[(3., 3.), (13., 3.), (13., 13.), (3., 13.)]); line(&[(3., 13.), (3., 3.), (13., 3.)]); }
        Icon::Faces => { fill_poly(&[(3., 3.), (13., 3.), (13., 13.), (3., 13.)]); }
        Icon::Orient => { line(&[(4., 12.), (4., 3.)]); line(&[(4., 12.), (13., 12.)]); line(&[(4., 12.), (1.8, 14.2)]); }
        Icon::Pivot => { line(&[(8., 2.), (8., 14.)]); line(&[(2., 8.), (14., 8.)]); circle(8., 8., 2.4); }
        Icon::Magnet => {
            // Closed horseshoe outline with separated pole caps, like Blender's magnet.
            poly(&[(2.5, 2.), (5.5, 2.), (5.5, 8.5), (6., 10.), (8., 10.8),
                (10., 10.), (10.5, 8.5), (10.5, 2.), (13.5, 2.), (13.5, 8.5),
                (13., 11.), (11., 13.), (8., 13.8), (5., 13.), (3., 11.), (2.5, 8.5)]);
            line(&[(2.5, 5.), (5.5, 5.)]); line(&[(10.5, 5.), (13.5, 5.)]);
        }
        Icon::Prop => { dot(8., 8., 2.); circle(8., 8., 6.); }
        Icon::Overlays => { circle(6., 8., 3.6); circle(10., 8., 3.6); }
        Icon::Xray => { poly(&[(3., 3.), (11., 3.), (11., 11.), (3., 11.)]); poly(&[(6., 6.), (13., 6.), (13., 13.), (6., 13.)]); }
        Icon::Solid => { p.circle_filled(at(8., 8.), 5.5 * s, col); }
        Icon::Shaded => { circle(8., 8., 5.5); fill_poly(&[(8., 2.5), (10.5, 3.2), (12.2, 5.), (13.5, 8.), (12.2, 11.), (10.5, 12.8), (8., 13.5)]); }
        Icon::Wire => { circle(8., 8., 5.5); line(&[(2.5, 8.), (5., 9.7), (8., 10.2), (11., 9.7), (13.5, 8.)]); line(&[(2.5, 8.), (5., 6.3), (8., 5.8), (11., 6.3), (13.5, 8.)]); line(&[(8., 2.5), (8., 13.5)]); }
        Icon::ShadedWire => { p.circle_filled(at(8., 8.), 5.5 * s, col.gamma_multiply(0.45)); circle(8., 8., 5.5); line(&[(2.5, 8.), (8., 10.2), (13.5, 8.)]); line(&[(2.5, 8.), (8., 5.8), (13.5, 8.)]); line(&[(8., 2.5), (8., 13.5)]); }
        Icon::Frame => { line(&[(1.5, 4.5), (1.5, 1.5), (4.5, 1.5)]); line(&[(11.5, 1.5), (14.5, 1.5), (14.5, 4.5)]); line(&[(14.5, 11.5), (14.5, 14.5), (11.5, 14.5)]); line(&[(4.5, 14.5), (1.5, 14.5), (1.5, 11.5)]); poly(&[(5.5, 5.5), (10.5, 5.5), (10.5, 10.5), (5.5, 10.5)]); }
        Icon::BoxSelect => { poly(&[(2., 2.), (14., 2.), (14., 14.), (2., 14.)]); }
        Icon::CircleSelect => { circle(8., 8., 6.); dot(8., 8., 1.); }
        Icon::LassoSelect => { line(&[(4., 12.), (1.5, 8.), (3., 3.), (9., 2.), (14., 6.), (12., 12.), (6., 14.), (4., 12.), (6., 9.), (8., 11.), (5., 15.)]); }
        Icon::Select => { poly(&[(4., 2.), (12., 7.6), (8.3, 8.5), (10.5, 12.2), (8.9, 13.1), (6.7, 9.4), (4., 12.6)]); }
        Icon::Cursor => { circle(8., 8., 3.); line(&[(8., 1.5), (8., 4.5)]); line(&[(8., 11.5), (8., 14.5)]); line(&[(1.5, 8.), (4.5, 8.)]); line(&[(11.5, 8.), (14.5, 8.)]); }
        Icon::Move => { line(&[(8., 1.5), (8., 14.5)]); line(&[(1.5, 8.), (14.5, 8.)]); line(&[(6., 3.5), (8., 1.5), (10., 3.5)]); line(&[(6., 12.5), (8., 14.5), (10., 12.5)]); line(&[(3.5, 6.), (1.5, 8.), (3.5, 10.)]); line(&[(12.5, 6.), (14.5, 8.), (12.5, 10.)]); }
        Icon::Rotate => { line(&[(13., 8.), (12.6, 10.), (11.2, 12.2), (9., 13.2), (6.5, 13.), (4.4, 11.6), (3.2, 9.4), (3., 7.), (4., 5.), (5.8, 3.6), (8., 3.)]); line(&[(13., 2.5), (13., 5.5), (10., 5.5)]); }
        Icon::Scale => { poly(&[(2., 7.), (9., 7.), (9., 14.), (2., 14.)]); line(&[(7., 9.), (14., 2.)]); line(&[(9.5, 2.), (14., 2.), (14., 6.5)]); }
        Icon::Transform => { poly(&[(3.5, 3.5), (12.5, 3.5), (12.5, 12.5), (3.5, 12.5)]); for (x, y) in [(3., 3.), (13., 3.), (3., 13.), (13., 13.)] { fill_poly(&[(x - 1.5, y - 1.5), (x + 1.5, y - 1.5), (x + 1.5, y + 1.5), (x - 1.5, y + 1.5)]); } }
        Icon::Line => { line(&[(2., 13.), (14., 3.)]); for q in [(2.,13.),(14.,3.)] { p.circle_filled(at(q.0,q.1), 1.8, col); } }
        Icon::Polyline => { line(&[(2., 13.), (5., 3.), (10., 10.), (14., 4.)]); }
        Icon::Polygon => { poly(&[(2., 6.), (7., 2.), (14., 6.), (12., 14.), (4., 13.)]); }
        Icon::Rectangle => { poly(&[(2., 3.), (14., 3.), (14., 13.), (2., 13.)]); }
        Icon::Circle => { p.circle_stroke(at(8.,8.), 6. * s, st); }
        Icon::Knife => { poly(&[(3., 14.), (7., 8.), (13., 2.), (14., 6.), (9., 11.)]); line(&[(5.,7.),(10.,12.)]); }
        Icon::LoopCut => { poly(&[(1.,4.),(11.,1.),(15.,11.),(5.,15.)]); line(&[(3.,9.),(13.,6.)]); line(&[(6.,2.5),(10.,13.)]); }
        Icon::Measure => { poly(&[(1.5, 11.5), (11.5, 1.5), (14.5, 4.5), (4.5, 14.5)]); line(&[(4., 9.), (5.5, 10.5)]); line(&[(6.5, 6.5), (8., 8.)]); line(&[(9., 4.), (10.5, 5.5)]); }
        Icon::ToStart => { line(&[(3.5, 3.), (3.5, 13.)]); poly(&[(13., 3.5), (13., 12.5), (6., 8.)]); }
        Icon::Prev => { poly(&[(12., 3.5), (12., 12.5), (5., 8.)]); }
        Icon::Play => { fill_poly(&[(5., 3.), (13., 8.), (5., 13.)]); }
        Icon::Pause => { fill_poly(&[(4.5, 3.), (7., 3.), (7., 13.), (4.5, 13.)]); fill_poly(&[(9., 3.), (11.5, 3.), (11.5, 13.), (9., 13.)]); }
        Icon::Next => { poly(&[(4., 3.5), (4., 12.5), (11., 8.)]); }
        Icon::ToEnd => { line(&[(12.5, 3.), (12.5, 13.)]); poly(&[(3., 3.5), (3., 12.5), (10., 8.)]); }
    }
}

/// Compact Blender-style status strip for the active viewport transform.
pub fn modal_status(ui: &egui::Ui, rect: egui::Rect, status: &str) {
    let painter = ui.painter_at(rect);
    let bar = egui::Rect::from_min_max(egui::pos2(rect.left(), rect.bottom()-29.), rect.right_bottom());
    painter.rect_filled(bar, 0., egui::Color32::from_black_alpha(205));
    painter.text(bar.left_center()+egui::vec2(10.,0.), egui::Align2::LEFT_CENTER, status,
        egui::FontId::proportional(13.), egui::Color32::WHITE);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn timeline_shortcuts_step_and_jump() {
        let ctx = egui::Context::default();
        timeline_command(&ctx, "timeline_start");
        timeline_command(&ctx, "timeline_previous");
        assert_eq!(ctx.data(|d| d.get_temp::<VpState>(egui::Id::new("viewport_ui_state")).unwrap().frame), FRAME_START);
        timeline_command(&ctx, "timeline_next");
        assert_eq!(ctx.data(|d| d.get_temp::<VpState>(egui::Id::new("viewport_ui_state")).unwrap().frame), FRAME_START + 1);
        timeline_command(&ctx, "timeline_end");
        assert_eq!(ctx.data(|d| d.get_temp::<VpState>(egui::Id::new("viewport_ui_state")).unwrap().frame), FRAME_END);
    }

    fn overlap(a: &Rect, b: &Rect) -> bool { a.shrink(0.5).intersects(b.shrink(0.5)) }

    #[test]
    fn header_fits_and_does_not_overlap_down_to_640() {
        for width in [640., 720., 920., 1280.] {
            for snap in [false, true] {
                let h = header_layout(0., 0., width, snap);
                let mut rects: Vec<Rect> = h.modes.to_vec();
                rects.extend([h.orient, h.pivot, h.snap, h.prop, h.proxy, h.overlays, h.frame]);
                rects.extend([h.shading,h.visibility]);
                for (i, a) in rects.iter().enumerate() {
                    assert!(a.left() >= 0. && a.right() <= width, "width {width}: {a:?} outside");
                    for b in &rects[i + 1..] { assert!(!overlap(a, b), "width {width} snap {snap}: {a:?} overlaps {b:?}"); }
                }
                // the left cluster ends before the right cluster starts
                assert!(h.prop.right() < h.overlays.left(), "width {width} snap {snap}: clusters collide");
                assert!(h.prop.right() + 8. <= h.proxy.left() && h.proxy.right() + 8. <= h.overlays.left());
                assert!(h.proxy.width() >= 50., "width {width} snap {snap}: proxy chip too narrow");
            }
        }
    }

    #[test]
    fn shading_and_visibility_dropdowns_are_adjacent() {
        let h = header_layout(0., 0., 800., false);
        assert!(h.shading.right() < h.visibility.left());
        assert_eq!(Mode::ALL.len(),12);
    }

    #[test]
    fn error_text_is_split_into_node_and_message() {
        assert_eq!(split_error("node 'noise1' (noise): ValueError: frequency must be greater than 0"), (Some("noise1"), "ValueError: frequency must be greater than 0"));
        assert_eq!(split_error("graph has a cycle"), (None, "graph has a cycle"));
    }

    #[test]
    fn scrubbing_maps_the_bar_to_frames() {
        assert_eq!(frame_at(100., 100., 300.), FRAME_START);
        assert_eq!(frame_at(300., 100., 300.), FRAME_END);
        assert_eq!(frame_at(-50., 100., 300.), FRAME_START);
        let mid = frame_at(200., 100., 300.);
        assert!((mid as i32 - 120).abs() <= 1);
    }

    #[test]
    fn the_overlay_draws_a_frame_without_asking_for_anything() {
        let f = super::super::testing::Fixture::new();
        let acts = f.run(|ui, v, out| overlay(ui, Rect::from_min_size(Pos2::ZERO, Vec2::new(900., 600.)), v, out));
        assert!(acts.is_empty());
    }

    #[test]
    fn popover_switches_and_closes() {
        let mut open = None;
        open = toggle_popover(open, Popover::Orientation);
        assert_eq!(open, Some(Popover::Orientation));
        open = toggle_popover(open, Popover::Pivot);
        assert_eq!(open, Some(Popover::Pivot));
        assert_eq!(toggle_popover(open, Popover::Pivot), None);
    }

    #[test]
    fn overlay_header_toggle_menu_and_geometry_checkbox_are_independent() {
        use egui::{Event,PointerButton,RawInput};
        let f=super::super::testing::Fixture::new(); let ctx=egui::Context::default(); f.theme.apply(&ctx);
        let viewport=Rect::from_min_size(Pos2::ZERO,Vec2::new(900.,600.));
        let h=header_layout(0.,0.,900.,false);
        let frame=|events:Vec<Event>| {
            let mut out=Actions::default();
            let _=ctx.run(RawInput {screen_rect:Some(viewport),events,..Default::default()},|ctx| {
                egui::CentralPanel::default().show(ctx,|ui|overlay(ui,viewport,&f.view(),&mut out));
            }); out
        };
        frame(vec![]);
        let click=|p:Pos2| {
            frame(vec![Event::PointerMoved(p),Event::PointerButton{pos:p,button:PointerButton::Primary,pressed:true,modifiers:Default::default()}]);
            frame(vec![Event::PointerButton{pos:p,button:PointerButton::Primary,pressed:false,modifiers:Default::default()}])
        };
        let toggle=click(h.overlays.left_center()+Vec2::new(12.,0.));
        assert!(toggle.0.iter().any(|a|matches!(a,UiAction::Editor(EditorChange::Overlays(o)) if !o.enabled && o.points && o.lines && o.faces)));
        let menu=click(h.overlays.right_center()-Vec2::new(8.,0.));
        assert!(!menu.0.iter().any(|a|matches!(a,UiAction::Editor(EditorChange::Overlays(_)))));
        assert_eq!(ctx.data(|d|d.get_temp::<VpState>(egui::Id::new("viewport_ui_state"))).unwrap().popover,Some(Popover::Overlays));
        let x=(h.overlays.right()-232.).clamp(6.,662.);
        let points=click(Pos2::new(x+60.,h.overlays.bottom()+4.+28.+9.*24.+12.));
        assert!(points.0.iter().any(|a|matches!(a,UiAction::Editor(EditorChange::Overlays(o)) if !o.points && o.enabled && o.lines && o.faces)));
    }

    #[test]
    fn popover_closes_on_escape_and_outside_click() {
        use egui::{Event, PointerButton, RawInput};
        let f = super::super::testing::Fixture::new();
        let ctx = egui::Context::default();
        f.theme.apply(&ctx);
        let id = egui::Id::new("viewport_ui_state");
        let viewport = Rect::from_min_size(Pos2::ZERO, Vec2::new(900., 600.));
        let mut frame = |events: Vec<Event>| {
            let input = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280., 800.))), events, ..Default::default() };
            let mut out = Actions::default();
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| overlay(ui, viewport, &f.view(), &mut out));
            });
        };
        let click = |p: Pos2, frame: &mut dyn FnMut(Vec<Event>)| {
            frame(vec![Event::PointerMoved(p), Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers: Default::default() }]);
            frame(vec![Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: false, modifiers: Default::default() }]);
        };
        let h = header_layout(0., 0., 900., false);
        frame(vec![]);
        click(h.orient.center(), &mut frame);
        assert_eq!(ctx.data(|d| d.get_temp::<VpState>(id)).unwrap().popover, Some(Popover::Orientation));
        frame(vec![Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }]);
        assert_eq!(ctx.data(|d| d.get_temp::<VpState>(id)).unwrap().popover, None);
        click(h.pivot.center(), &mut frame);
        assert_eq!(ctx.data(|d| d.get_temp::<VpState>(id)).unwrap().popover, Some(Popover::Pivot));
        click(Pos2::new(450., 400.), &mut frame);
        assert_eq!(ctx.data(|d| d.get_temp::<VpState>(id)).unwrap().popover, None);
    }

    #[test]
    fn falloff_curves_have_finite_samples_and_end_at_zero() {
        for falloff in Falloff::ALL {
            let samples = falloff_samples(falloff);
            assert!(samples.iter().all(|x| x.is_finite() && (0. ..=1.).contains(x)));
            assert_eq!(samples[12], 0.);
        }
        assert!((falloff_samples(Falloff::Linear)[6] - 0.5).abs() < 1e-5);
    }

    #[test]
    fn snap_increment_shows_units_in_header() {
        let mut f = super::super::testing::Fixture::new();
        let mut editor = crate::ui::EditorState::default();
        editor.snap.enabled = true;
        editor.snap.to = SnapTo::Increment.into();
        editor.snap.increment = 0.25;
        f.editor = editor;
        let viewport = Rect::from_min_size(Pos2::ZERO, Vec2::new(900., 600.));
        let mut out = Actions::default();
        let ctx = egui::Context::default();
        f.theme.apply(&ctx);
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280., 800.))), ..Default::default() };
        let full = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| overlay(ui, viewport, &f.view(), &mut out));
        });
        fn extract_texts(shapes: &[egui::epaint::ClippedShape], out: &mut Vec<String>) {
            for s in shapes {
                match &s.shape {
                    egui::epaint::Shape::Text(t) => out.push(t.galley.text().to_owned()),
                    egui::epaint::Shape::Vec(subs) => {
                        let clipped: Vec<egui::epaint::ClippedShape> = subs.iter().map(|sub| egui::epaint::ClippedShape { clip_rect: s.clip_rect, shape: sub.clone() }).collect();
                        extract_texts(&clipped, out);
                    }
                    _ => {}
                }
            }
        }
        let mut texts = Vec::new();
        extract_texts(&full.shapes, &mut texts);
        assert!(texts.iter().any(|t| t.contains("250 mm")), "expected 250 mm in {texts:?}");
    }
}

fn display_row(ui:&mut egui::Ui,r:Rect,id:(&str,usize),label:&str,selected:bool,ic:Icon,th:&Theme)->bool {
    let response=ui.interact(r,egui::Id::new(("display_row",id)),Sense::click());
    if selected || response.hovered() { ui.painter().rect_filled(r,2.,if selected { th.accent_bg } else { th.panel2 }); }
    icon(ui.painter(),ic,Rect::from_center_size(Pos2::new(r.left()+12.,r.center().y),Vec2::splat(18.)),th.text,false);
    ui.painter().text(Pos2::new(r.left()+28.,r.center().y),egui::Align2::LEFT_CENTER,label,egui::FontId::proportional(12.),th.text);
    response.clicked()
}
pub fn popup_open(ctx:&egui::Context)->bool {
    ctx.data(|d| d.get_temp::<VpState>(egui::Id::new("viewport_ui_state")).is_some_and(|s| s.popover.is_some())
        || d.get_temp::<Pos2>(egui::Id::new("shading_pie")).is_some())
}

pub fn open_shading_pie(ctx:&egui::Context) {
    let at=ctx.input(|i| i.pointer.hover_pos()).unwrap_or(Pos2::new(300.,200.));
    ctx.data_mut(|d| d.insert_temp(egui::Id::new("shading_pie"),at));
}
fn show_shading_pie(ctx:&egui::Context, viewport:Rect,v:&View,out:&mut Actions) {
    let id=egui::Id::new("shading_pie");
    let Some(at)=ctx.data(|d| d.get_temp::<Pos2>(id)) else { return; };
    let origin=Pos2::new((at.x-220.).clamp(viewport.left(),(viewport.right()-440.).max(viewport.left())),
        (at.y-110.).clamp(viewport.top()+HEADER_H,(viewport.bottom()-220.).max(viewport.top()+HEADER_H)));
    let mut close=ctx.input(|i| i.key_pressed(egui::Key::Escape));
    egui::Area::new(id).order(egui::Order::Foreground).fixed_pos(origin).show(ctx,|ui| {
        ui.set_min_size(Vec2::new(440.,220.));
        let c=ui.min_rect().min+Vec2::new(220.,110.);
        ui.painter().circle_filled(c,70.,v.theme.panel);
        ui.painter().text(c-Vec2::new(0.,16.),egui::Align2::CENTER_CENTER,"SHADING · Z",egui::FontId::proportional(12.),v.theme.dim);
        if display_row(ui,Rect::from_center_size(c+Vec2::new(0.,16.),Vec2::new(112.,26.)),("pie_all",0),"All modes…",false,Icon::Display(v.mode),v.theme) {
            let state_id=egui::Id::new("viewport_ui_state");
            ctx.data_mut(|d| { let mut state=d.get_temp::<VpState>(state_id).unwrap_or_default(); state.popover=Some(Popover::Shading); d.insert_temp(state_id,state); });
            close=true;
        }
        for (i,(mode,offset,key)) in [
            (Mode::Wireframe,Vec2::new(-140.,0.),egui::Key::Num4),
            (Mode::FlatShaded,Vec2::new(140.,0.),egui::Key::Num6),
            (Mode::SmoothShaded,Vec2::new(0.,-76.),egui::Key::Num8),
            (Mode::SmoothWireShaded,Vec2::new(0.,76.),egui::Key::Num2),
        ].into_iter().enumerate() {
            let row=Rect::from_center_size(c+offset,Vec2::new(158.,30.));
            ui.painter().rect_filled(row,4.,v.theme.panel);
            if display_row(ui,row,("pie",i),mode.label(),mode==v.mode,Icon::Display(mode),v.theme)
                || ctx.input(|i| i.key_pressed(key)) { out.push(UiAction::SetMode(mode)); close=true; }
        }
        if ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !ui.min_rect().contains(p))) { close=true; }
    });
    if close { ctx.data_mut(|d| d.remove::<Pos2>(id)); }
}

#[cfg(test)] mod display_menu_tests {
    use super::*;
    use egui::{Event,PointerButton,RawInput};
    fn frame(ctx:&egui::Context,f:&super::super::testing::Fixture,events:Vec<Event>)->Vec<UiAction> {
        let mut out=Actions::default();
        let _=ctx.run(RawInput { screen_rect:Some(Rect::from_min_size(Pos2::ZERO,Vec2::new(900.,600.))),events,..Default::default() },|ctx| {
            egui::CentralPanel::default().show(ctx,|ui| overlay(ui,Rect::from_min_size(Pos2::ZERO,Vec2::new(900.,600.)),&f.view(),&mut out));
        }); out.0
    }
    fn click(ctx:&egui::Context,f:&super::super::testing::Fixture,p:Pos2)->Vec<UiAction> {
        frame(ctx,f,vec![Event::PointerMoved(p),Event::PointerButton { pos:p,button:PointerButton::Primary,pressed:true,modifiers:Default::default() }]);
        frame(ctx,f,vec![Event::PointerButton { pos:p,button:PointerButton::Primary,pressed:false,modifiers:Default::default() }])
    }
    #[test] fn dropdowns_emit_every_display_and_visibility_choice() {
        let f=super::super::testing::Fixture::new(); let ctx=egui::Context::default(); f.theme.apply(&ctx);
        let h=header_layout(0.,0.,900.,false);
        frame(&ctx,&f,vec![]);
        for (i,mode) in Mode::ALL.into_iter().enumerate() {
            click(&ctx,&f,h.shading.center());
            assert!(popup_open(&ctx));
            let at=Pos2::new(h.shading.right()-100.,h.shading.bottom()+4.+28.+i as f32*24.+12.);
            let actions=click(&ctx,&f,at);
            assert!(actions.iter().any(|a| matches!(a,UiAction::SetMode(m) if *m==mode)),"{} row failed",mode.label());
            assert!(!popup_open(&ctx));
        }
        for (i,visibility) in ObjectVisibility::ALL.into_iter().enumerate() {
            click(&ctx,&f,h.visibility.center());
            let at=Pos2::new(h.visibility.right()-100.,h.visibility.bottom()+4.+28.+i as f32*24.+12.);
            let actions=click(&ctx,&f,at);
            assert!(actions.iter().any(|a| matches!(a,UiAction::Editor(EditorChange::ObjectVisibility(v)) if *v==visibility)));
            assert!(!popup_open(&ctx));
        }
    }
    #[test] fn z_pie_number_selects_mapped_mode_and_closes() {
        let f=super::super::testing::Fixture::new(); let ctx=egui::Context::default();
        frame(&ctx,&f,vec![]); open_shading_pie(&ctx);
        let actions=frame(&ctx,&f,vec![Event::Key { key:egui::Key::Num8,physical_key:None,pressed:true,repeat:false,modifiers:Default::default() }]);
        assert!(actions.iter().any(|a| matches!(a,UiAction::SetMode(Mode::SmoothShaded))));
        assert!(!popup_open(&ctx));
    }
}
