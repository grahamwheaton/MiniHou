//! Viewport chrome drawn over the 3-D view (design: Viewport.dc.html): header bar (selection modes, orientation, pivot,
//! snapping, proportional editing, overlays, X-ray, shading, Frame), left tool strip, bottom timeline, stats chip, view gizmo,
//! error and dense-mesh notes. `rect` is the viewport's area; the 3-D image is already painted into it.
use super::{theme::Theme, Actions, EditorChange, Falloff, Orientation, Pivot, SelectionMode, SnapTo, UiAction, View};
use crate::viewport::Mode;
use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
#[path = "view_gizmo.rs"] mod view_gizmo;

const HEADER_H: f32 = 36.;
const TIMELINE_H: f32 = 44.;
const TOOLS: [(&str, Icon); 7] = [
    ("Select (Q)", Icon::Select), ("3D cursor (C)", Icon::Cursor), ("Move (W)", Icon::Move), ("Rotate (E)", Icon::Rotate),
    ("Scale (R)", Icon::Scale), ("Transform (T)", Icon::Transform), ("Measure (M)", Icon::Measure),
];
const DENSE_PRIMS: usize = 40_000;
const FRAME_START: u32 = 1;
const FRAME_END: u32 = 240;

/// UI-only viewport state (nothing here reaches the engine yet).
#[derive(Clone)]
struct VpState { tool: usize, popover: Option<Popover>, frame: u32, playing: bool, clock: f32 }
impl Default for VpState {
    fn default() -> Self {
        Self { tool: 0, popover: None, frame: 48, playing: false, clock: 0. }
    }
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Popover { Orientation, Pivot, Snap, Proportional, Overlays }
fn toggle_popover(open: Option<Popover>, next: Popover) -> Option<Popover> { if open == Some(next) { None } else { Some(next) } }
const OVERLAYS: [(&str, &str); 6] = [("Grid floor", "G"), ("Axis gizmo", ""), ("Wireframe overlay", "W"), ("Stats", "I"), ("Normals", "N"), ("Bounding box", "B")];

#[derive(Clone, Copy, PartialEq, Debug)]
enum Icon { Object, Points, Edges, Faces, Orient, Pivot, Magnet, Prop, Overlays, Xray, Solid, Shaded, Wire, ShadedWire, Frame,
    Select, Cursor, Move, Rotate, Scale, Transform, Measure, ToStart, Prev, Play, Pause, Next, ToEnd }

/// Where every header control sits, from the viewport's left/top/width. Pure so it can be tested at any width.
#[derive(Debug)]
struct HeaderLayout { modes: [Rect; 4], orient: Rect, pivot: Rect, snap: Rect, prop: Rect, overlays: Rect, xray: Rect, shading: [Rect; 4], frame: Rect, seps: Vec<f32>, groups: Vec<Rect> }
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
    let snap_w = if snap_on { 76. } else { 44. };
    let snap = mk(x, snap_w, 26.); x += snap_w + 3.;
    let prop = mk(x, 44., 26.);
    // right end, laid from the right edge inwards
    let mut r = left + width - 6.;
    let frame = mk(r - 26., 26., 26.); r -= 26. + 3.;
    seps.push(r - 4.); r -= 9.;
    let gright = r;
    let gleft = gright - (4. * 25. - 1. + 4.);
    let shading = [0, 1, 2, 3].map(|i| mk(gleft + 2. + i as f32 * 25., 24., 22.));
    groups.push(Rect::from_min_max(Pos2::new(gleft, cy - 13.), Pos2::new(gright, cy + 13.)));
    r = gleft;
    seps.push(r - 4.); r -= 9.;
    let xray = mk(r - 26., 26., 26.); r -= 26. + 3.;
    let overlays = mk(r - 44., 44., 26.);
    HeaderLayout { modes, orient, pivot, snap, prop, overlays, xray, shading, frame, seps, groups }
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
    let ctx = ui.ctx().clone();
    let id = egui::Id::new("viewport_ui_state");
    let mut st: VpState = ctx.data_mut(|d| d.get_temp(id)).unwrap_or_default();
    let th = v.theme;
    let painter = ui.painter_at(rect);
    let stroke = Stroke::new(1.0_f32, th.stroke);

    // ---- header bar
    painter.rect_filled(Rect::from_min_size(rect.min, Vec2::new(rect.width(), HEADER_H)), 0., th.panel);
    painter.line_segment([Pos2::new(rect.left(), rect.top() + HEADER_H), Pos2::new(rect.right(), rect.top() + HEADER_H)], stroke);
    let ed = v.editor;
    let show = [ed.overlays.grid, ed.overlays.axes, ed.overlays.wireframe, ed.overlays.stats, ed.overlays.normals, ed.overlays.bounds];
    let h = header_layout(rect.left(), rect.top(), rect.width(), ed.snap.enabled);
    for g in &h.groups { painter.rect(*g, 5., th.panel2, stroke, StrokeKind::Inside); }
    for x in &h.seps { painter.line_segment([Pos2::new(*x, rect.top() + 9.), Pos2::new(*x, rect.top() + HEADER_H - 9.)], stroke); }
    for (i, r) in h.modes.iter().enumerate() {
        let mode = SelectionMode::ALL[i];
        if button(ui, th, *r, ("vp_mode", i), ed.selection == mode, ["Object mode (4)", "Points (1)", "Edges (2)", "Faces (3)"][i], [Icon::Object, Icon::Points, Icon::Edges, Icon::Faces][i]) { out.push(UiAction::Editor(EditorChange::Selection(mode))); }
    }
    if button_text(ui, th, h.orient, "vp_orient", st.popover == Some(Popover::Orientation), "Transform orientation", Icon::Orient, (Some(ed.orientation.label()), true)) { st.popover = toggle_popover(st.popover, Popover::Orientation); }
    if button_text(ui, th, h.pivot, "vp_pivot", st.popover == Some(Popover::Pivot), "Pivot point", Icon::Pivot, (None, true)) { st.popover = toggle_popover(st.popover, Popover::Pivot); }
    let snap_label = format!("{}", ed.snap.increment);
    let snap_main = Rect::from_min_max(h.snap.min, Pos2::new(h.snap.right() - 16., h.snap.bottom()));
    if button_text(ui, th, snap_main, "vp_snap_toggle", ed.snap.enabled, "Toggle snapping (Shift+Tab)", Icon::Magnet, (if ed.snap.enabled { Some(snap_label.as_str()) } else { None }, false)) { out.push(UiAction::Editor(EditorChange::SnapEnabled(!ed.snap.enabled))); }
    if caret_button(ui, th, h.snap, "vp_snap_menu", st.popover == Some(Popover::Snap)) { st.popover = toggle_popover(st.popover, Popover::Snap); }
    let prop_main = Rect::from_min_max(h.prop.min, Pos2::new(h.prop.right() - 16., h.prop.bottom()));
    if button_text(ui, th, prop_main, "vp_prop_toggle", ed.proportional.enabled, "Toggle proportional editing (O)", Icon::Prop, (None, false)) { out.push(UiAction::Editor(EditorChange::ProportionalEnabled(!ed.proportional.enabled))); }
    if caret_button(ui, th, h.prop, "vp_prop_menu", st.popover == Some(Popover::Proportional)) { st.popover = toggle_popover(st.popover, Popover::Proportional); }
    if button_text(ui, th, h.overlays, "vp_overlays", st.popover == Some(Popover::Overlays), "Overlays", Icon::Overlays, (None, true)) { st.popover = toggle_popover(st.popover, Popover::Overlays); }
    if button(ui, th, h.xray, ("vp_xray", 0), ed.xray, "X-ray (Alt+Z)", Icon::Xray) { out.push(UiAction::Editor(EditorChange::Xray(!ed.xray))); }
    for (i, (mode, name, icon)) in [(Mode::Solid, "Solid", Icon::Solid), (Mode::Shaded, "Shaded", Icon::Shaded), (Mode::Wire, "Wire", Icon::Wire), (Mode::ShadedWire, "Shaded + Wire", Icon::ShadedWire)].into_iter().enumerate() {
        if button(ui, th, h.shading[i], ("vp_shading", i), v.mode == mode, name, icon) { out.push(UiAction::SetMode(mode)); }
    }
    if button(ui, th, h.frame, ("vp_frame", 0), false, "Frame the mesh (F)", Icon::Frame) { out.push(UiAction::Frame); }

    // ---- tool strip
    let strip_h = TOOLS.len() as f32 * 28. + 2. * 5. + 8.;
    let strip = Rect::from_min_size(Pos2::new(rect.left() + 12., rect.top() + HEADER_H + 12.), Vec2::new(36., strip_h));
    painter.rect(strip, 6., th.panel, stroke, StrokeKind::Inside);
    let mut y = strip.top() + 4.;
    for (i, (name, icon)) in TOOLS.iter().enumerate() {
        if i == 2 || i == 6 {
            painter.line_segment([Pos2::new(strip.left() + 6., y + 2.), Pos2::new(strip.right() - 6., y + 2.)], stroke);
            y += 5.;
        }
        let r = Rect::from_min_size(Pos2::new(strip.left() + 4., y), Vec2::splat(28.));
        if button(ui, th, r, ("vp_tool", i), st.tool == i, name, *icon) { st.tool = i; }
        y += 28.;
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
        let anchor = match which { Popover::Orientation => h.orient, Popover::Pivot => h.pivot, Popover::Snap => h.snap, Popover::Proportional => h.prop, Popover::Overlays => h.overlays };
        let pop = show_popover(ui, rect, anchor, which, v, out);
        let dismiss = ctx.input(|i| i.key_pressed(egui::Key::Escape) || (i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !pop.contains(p) && !anchor.contains(p))));
        if dismiss { st.popover = None; }
    }
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

fn numeric_row(ui: &mut egui::Ui, r: Rect, id: &'static str, label: &str, value: f32, th: &Theme) -> Option<f32> {
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
            return text.trim().parse::<f32>().ok().filter(|x| x.is_finite()).map(|x| x.max(1e-4));
        }
        ui.ctx().data_mut(|d| d.insert_temp(edit_id, text));
    } else {
        let resp = ui.interact(box_r, edit_id, Sense::click_and_drag());
        if resp.double_clicked() { ui.ctx().data_mut(|d| d.insert_temp(edit_id, format!("{value}"))); }
        if resp.dragged() {
            let dx = ui.ctx().input(|i| i.pointer.delta().x);
            if dx != 0. { return Some((value + dx * value.max(0.1) * 0.01).max(1e-4)); }
        }
        p.text(Pos2::new(box_r.right() - 6., box_r.center().y), egui::Align2::RIGHT_CENTER, format!("{value:.2}"), egui::FontId::monospace(12.), th.text);
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
    let rows = match which { Popover::Orientation => 6, Popover::Pivot => 5, Popover::Snap => 11, Popover::Proportional => 12, Popover::Overlays => 7 };
    let height = 30. + rows as f32 * 24. + 10. + if which == Popover::Overlays { 38. } else { 0. };
    let x = (anchor.right() - 232.).clamp(viewport.left() + 6., (viewport.right() - 238.).max(viewport.left() + 6.));
    let pop = Rect::from_min_size(Pos2::new(x, anchor.bottom() + 4.), Vec2::new(232., height));
    let p = ui.painter();
    p.rect(pop, 6., th.panel, Stroke::new(1.0_f32, th.stroke), StrokeKind::Inside);
    let title = match which { Popover::Orientation => "ORIENTATION", Popover::Pivot => "PIVOT POINT", Popover::Snap => "SNAPPING", Popover::Proportional => "PROPORTIONAL EDITING", Popover::Overlays => "OVERLAYS" };
    p.text(pop.min + Vec2::new(12., 17.), egui::Align2::LEFT_CENTER, title, egui::FontId::proportional(11.), th.dim);
    let row = |i: usize| Rect::from_min_size(pop.min + Vec2::new(6., 28. + i as f32 * 24.), Vec2::new(220., 24.));
    match which {
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
                if pop_row(ui, row(2 + i), ("snap_to", i), to.label(), *to == ed.snap.to, false, "", th) { out.push(UiAction::Editor(EditorChange::SnapTo(*to))); }
            }
            if let Some(x) = numeric_row(ui, row(8), "snap_increment", "Increment", ed.snap.increment, th) { out.push(UiAction::Editor(EditorChange::SnapIncrement(x))); }
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
            if let Some(x) = numeric_row(ui, row(10), "prop_radius", "Radius", ed.proportional.radius, th) { out.push(UiAction::Editor(EditorChange::ProportionalRadius(x))); }
            if pop_row(ui, row(11), ("prop_connected", 0), "Connected only", ed.proportional.connected_only, true, "", th) { out.push(UiAction::Editor(EditorChange::ProportionalConnected(!ed.proportional.connected_only))); }
        }
        Popover::Overlays => {
            let values = [ed.overlays.grid, ed.overlays.axes, ed.overlays.wireframe, ed.overlays.stats, ed.overlays.normals, ed.overlays.bounds];
            for i in 0..4 {
                if pop_row(ui, row(i), ("overlay", i), OVERLAYS[i].0, values[i], true, OVERLAYS[i].1, th) { toggle_overlay(i, ed.overlays, out); }
            }
            section(ui, row(4), "MESH", th);
            for i in 4..6 {
                if pop_row(ui, row(i + 1), ("overlay", i), OVERLAYS[i].0, values[i], true, OVERLAYS[i].1, th) { toggle_overlay(i, ed.overlays, out); }
            }
            let foot_y = row(7).top() + 8.;
            ui.painter().line_segment([Pos2::new(pop.left() + 12., foot_y), Pos2::new(pop.right() - 12., foot_y)], Stroke::new(1.0_f32, th.stroke));
            ui.painter().text(Pos2::new(pop.left() + 12., foot_y + 9.), egui::Align2::LEFT_TOP, "Edges fade out above 40,000 prims\nto keep the view readable.", egui::FontId::proportional(11.), th.dim);
        }
    }
    pop
}

fn section(ui: &egui::Ui, r: Rect, label: &str, th: &Theme) {
    ui.painter().line_segment([r.left_top() + Vec2::new(7., 1.), r.right_top() + Vec2::new(-7., 1.)], Stroke::new(1.0_f32, th.stroke));
    ui.painter().text(r.min + Vec2::new(8., 13.), egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(11.), th.dim);
}

fn toggle_overlay(i: usize, mut o: super::Overlays, out: &mut Actions) {
    match i { 0 => o.grid = !o.grid, 1 => o.axes = !o.axes, 2 => o.wireframe = !o.wireframe, 3 => o.stats = !o.stats, 4 => o.normals = !o.normals, _ => o.bounds = !o.bounds }
    out.push(UiAction::Editor(EditorChange::Overlays(o)));
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
        Icon::Object => { poly(&[(8., 1.5), (13.5, 4.3), (13.5, 11.7), (8., 14.5), (2.5, 11.7), (2.5, 4.3)]); line(&[(2.5, 4.3), (8., 7.2), (13.5, 4.3)]); line(&[(8., 7.2), (8., 14.5)]); }
        Icon::Points => { for (x, y) in [(4., 4.), (12., 4.), (4., 12.), (12., 12.)] { dot(x, y, 1.7); } }
        Icon::Edges => { poly(&[(3., 3.), (13., 3.), (13., 13.), (3., 13.)]); line(&[(3., 13.), (3., 3.), (13., 3.)]); }
        Icon::Faces => { fill_poly(&[(3., 3.), (13., 3.), (13., 13.), (3., 13.)]); }
        Icon::Orient => { line(&[(4., 12.), (4., 3.)]); line(&[(4., 12.), (13., 12.)]); line(&[(4., 12.), (1.8, 14.2)]); }
        Icon::Pivot => { line(&[(8., 2.), (8., 14.)]); line(&[(2., 8.), (14., 8.)]); circle(8., 8., 2.4); }
        Icon::Magnet => { line(&[(4., 2.), (4., 8.)]); line(&[(12., 2.), (12., 8.)]); line(&[(4., 8.), (4.5, 10.), (6., 11.8), (8., 12.4), (10., 11.8), (11.5, 10.), (12., 8.)]); line(&[(6.5, 2.), (6.5, 8.)]); line(&[(9.5, 2.), (9.5, 8.)]); }
        Icon::Prop => { dot(8., 8., 2.); circle(8., 8., 6.); }
        Icon::Overlays => { circle(6., 8., 3.6); circle(10., 8., 3.6); }
        Icon::Xray => { poly(&[(3., 3.), (11., 3.), (11., 11.), (3., 11.)]); poly(&[(6., 6.), (13., 6.), (13., 13.), (6., 13.)]); }
        Icon::Solid => { p.circle_filled(at(8., 8.), 5.5 * s, col); }
        Icon::Shaded => { circle(8., 8., 5.5); fill_poly(&[(8., 2.5), (10.5, 3.2), (12.2, 5.), (13.5, 8.), (12.2, 11.), (10.5, 12.8), (8., 13.5)]); }
        Icon::Wire => { circle(8., 8., 5.5); line(&[(2.5, 8.), (5., 9.7), (8., 10.2), (11., 9.7), (13.5, 8.)]); line(&[(2.5, 8.), (5., 6.3), (8., 5.8), (11., 6.3), (13.5, 8.)]); line(&[(8., 2.5), (8., 13.5)]); }
        Icon::ShadedWire => { p.circle_filled(at(8., 8.), 5.5 * s, col.gamma_multiply(0.45)); circle(8., 8., 5.5); line(&[(2.5, 8.), (8., 10.2), (13.5, 8.)]); line(&[(2.5, 8.), (8., 5.8), (13.5, 8.)]); line(&[(8., 2.5), (8., 13.5)]); }
        Icon::Frame => { line(&[(1.5, 4.5), (1.5, 1.5), (4.5, 1.5)]); line(&[(11.5, 1.5), (14.5, 1.5), (14.5, 4.5)]); line(&[(14.5, 11.5), (14.5, 14.5), (11.5, 14.5)]); line(&[(4.5, 14.5), (1.5, 14.5), (1.5, 11.5)]); poly(&[(5.5, 5.5), (10.5, 5.5), (10.5, 10.5), (5.5, 10.5)]); }
        Icon::Select => { poly(&[(4., 2.), (12., 7.6), (8.3, 8.5), (10.5, 12.2), (8.9, 13.1), (6.7, 9.4), (4., 12.6)]); }
        Icon::Cursor => { circle(8., 8., 3.); line(&[(8., 1.5), (8., 4.5)]); line(&[(8., 11.5), (8., 14.5)]); line(&[(1.5, 8.), (4.5, 8.)]); line(&[(11.5, 8.), (14.5, 8.)]); }
        Icon::Move => { line(&[(8., 1.5), (8., 14.5)]); line(&[(1.5, 8.), (14.5, 8.)]); line(&[(6., 3.5), (8., 1.5), (10., 3.5)]); line(&[(6., 12.5), (8., 14.5), (10., 12.5)]); line(&[(3.5, 6.), (1.5, 8.), (3.5, 10.)]); line(&[(12.5, 6.), (14.5, 8.), (12.5, 10.)]); }
        Icon::Rotate => { line(&[(13., 8.), (12.6, 10.), (11.2, 12.2), (9., 13.2), (6.5, 13.), (4.4, 11.6), (3.2, 9.4), (3., 7.), (4., 5.), (5.8, 3.6), (8., 3.)]); line(&[(13., 2.5), (13., 5.5), (10., 5.5)]); }
        Icon::Scale => { poly(&[(2., 7.), (9., 7.), (9., 14.), (2., 14.)]); line(&[(7., 9.), (14., 2.)]); line(&[(9.5, 2.), (14., 2.), (14., 6.5)]); }
        Icon::Transform => { poly(&[(3.5, 3.5), (12.5, 3.5), (12.5, 12.5), (3.5, 12.5)]); for (x, y) in [(3., 3.), (13., 3.), (3., 13.), (13., 13.)] { fill_poly(&[(x - 1.5, y - 1.5), (x + 1.5, y - 1.5), (x + 1.5, y + 1.5), (x - 1.5, y + 1.5)]); } }
        Icon::Measure => { poly(&[(1.5, 11.5), (11.5, 1.5), (14.5, 4.5), (4.5, 14.5)]); line(&[(4., 9.), (5.5, 10.5)]); line(&[(6.5, 6.5), (8., 8.)]); line(&[(9., 4.), (10.5, 5.5)]); }
        Icon::ToStart => { line(&[(3.5, 3.), (3.5, 13.)]); poly(&[(13., 3.5), (13., 12.5), (6., 8.)]); }
        Icon::Prev => { poly(&[(12., 3.5), (12., 12.5), (5., 8.)]); }
        Icon::Play => { fill_poly(&[(5., 3.), (13., 8.), (5., 13.)]); }
        Icon::Pause => { fill_poly(&[(4.5, 3.), (7., 3.), (7., 13.), (4.5, 13.)]); fill_poly(&[(9., 3.), (11.5, 3.), (11.5, 13.), (9., 13.)]); }
        Icon::Next => { poly(&[(4., 3.5), (4., 12.5), (11., 8.)]); }
        Icon::ToEnd => { line(&[(12.5, 3.), (12.5, 13.)]); poly(&[(3., 3.5), (3., 12.5), (10., 8.)]); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn overlap(a: &Rect, b: &Rect) -> bool { a.shrink(0.5).intersects(b.shrink(0.5)) }

    #[test]
    fn header_fits_and_does_not_overlap_down_to_640() {
        for width in [640., 720., 920., 1280.] {
            for snap in [false, true] {
                let h = header_layout(0., 0., width, snap);
                let mut rects: Vec<Rect> = h.modes.to_vec();
                rects.extend([h.orient, h.pivot, h.snap, h.prop, h.overlays, h.xray, h.frame]);
                rects.extend(h.shading);
                for (i, a) in rects.iter().enumerate() {
                    assert!(a.left() >= 0. && a.right() <= width, "width {width}: {a:?} outside");
                    for b in &rects[i + 1..] { assert!(!overlap(a, b), "width {width} snap {snap}: {a:?} overlaps {b:?}"); }
                }
                // the left cluster ends before the right cluster starts
                assert!(h.prop.right() < h.overlays.left(), "width {width} snap {snap}: clusters collide");
            }
        }
    }

    #[test]
    fn shading_buttons_run_solid_to_shaded_wire_left_to_right() {
        let h = header_layout(0., 0., 800., false);
        assert!(h.shading.windows(2).all(|w| w[0].left() < w[1].left()));
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
}
