use super::*;

pub(super) const HEADER_H: f32 = 36.;
pub(super) const TIMELINE_H: f32 = 44.;
pub(super) const TOOLS: [(&str, Icon, Option<Tool>); 15] = [
    ("Select (Q)", Icon::Select, Some(Tool::Select)), ("3D cursor", Icon::Cursor, Some(Tool::Cursor)),
    ("Move (G)", Icon::Move, Some(Tool::Move)), ("Rotate (R)", Icon::Rotate, Some(Tool::Rotate)),
    ("Scale (S)", Icon::Scale, Some(Tool::Scale)), ("Scale Cage (Shift+S)", Icon::ScaleCage, None),
    ("Transform", Icon::Transform, Some(Tool::Transform)), ("Measure", Icon::Measure, Some(Tool::Measure)),
    ("Draw Line", Icon::Line, Some(Tool::Line)), ("Draw Polyline (Enter finish)", Icon::Polyline, Some(Tool::Polyline)),
    ("Draw Polygon (Enter finish)", Icon::Polygon, Some(Tool::Polygon)), ("Draw Rectangle", Icon::Rectangle, Some(Tool::Rectangle)),
    ("Draw Circle", Icon::Circle, Some(Tool::Circle)),
    ("Knife (K): click mesh points, RMB/E next chain, Enter confirm all", Icon::Knife, Some(Tool::Knife)),
    ("Loop Cut (Ctrl+R): hover a quad edge, wheel count, click then slide, click confirm", Icon::LoopCut, Some(Tool::LoopCut)),
];
pub(super) const DENSE_PRIMS: usize = 40_000;
pub(super) const FRAME_START: u32 = 0;
pub(super) const FRAME_END: u32 = 240;

/// Viewport chrome state; frame changes are emitted as time edits.
#[derive(Clone)]
pub(super) struct VpState { pub(super) popover: Option<Popover>, pub(super) frame: u32, pub(super) playing: bool, pub(super) clock: f32 }
impl Default for VpState {
    fn default() -> Self {
        Self { popover: None, frame: 1, playing: false, clock: 0. }
    }
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Popover { Orientation, Pivot, Snap, Proportional, Overlays, Shading, Visibility }
pub(super) fn toggle_popover(open: Option<Popover>, next: Popover) -> Option<Popover> { if open == Some(next) { None } else { Some(next) } }
pub(super) const OVERLAYS: [(&str, &str); 6] = [("Grid floor", ""), ("Axis gizmo", ""), ("Edges in solid", ""), ("Stats", ""), ("Normals", ""), ("Bounding box", "")];

#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Icon { Display(Mode), Visibility(ObjectVisibility), Object, Points, Edges, Faces, Orient, Pivot, Magnet, Prop, Overlays, Xray, Solid, Shaded, Wire, ShadedWire, Frame,
    Select, BoxSelect, CircleSelect, LassoSelect, Cursor, Move, Rotate, Scale, ScaleCage, Transform, Measure, Line, Polyline, Polygon, Rectangle, Circle, Knife, LoopCut, ToStart, Prev, Play, Pause, Next, ToEnd }

/// Where every header control sits, from the viewport's left/top/width. Pure so it can be tested at any width.
#[derive(Debug)]
pub(super) struct HeaderLayout { pub(super) modes: [Rect; 4], pub(super) orient: Rect, pub(super) pivot: Rect, pub(super) snap: Rect, pub(super) prop: Rect, pub(super) proxy: Rect, pub(super) overlays: Rect, pub(super) shading: Rect, pub(super) visibility: Rect, pub(super) frame: Rect, pub(super) seps: Vec<f32>, pub(super) groups: Vec<Rect> }
pub(super) fn header_layout(left: f32, top: f32, width: f32, snap_on: bool) -> HeaderLayout {
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
pub(super) fn split_error(error: &str) -> (Option<&str>, &str) {
    if let Some(rest) = error.strip_prefix("node '") {
        if let Some((id, tail)) = rest.split_once('\'') {
            let msg = tail.split_once("): ").map_or(tail.trim_start_matches([' ', ':']), |(_, m)| m);
            return (Some(id), msg);
        }
    }
    (None, error)
}

pub(super) fn frame_at(x: f32, left: f32, right: f32) -> u32 {
    let t = ((x - left) / (right - left).max(1.)).clamp(0., 1.);
    FRAME_START + (t * (FRAME_END - FRAME_START) as f32).round() as u32
}
pub(super) fn section(ui: &egui::Ui, r: Rect, label: &str, th: &Theme) {
    ui.painter().line_segment([r.left_top() + Vec2::new(7., 1.), r.right_top() + Vec2::new(-7., 1.)], Stroke::new(1.0_f32, th.stroke));
    ui.painter().text(r.min + Vec2::new(8., 13.), egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(11.), th.dim);
}

/// Icon-only button with a tooltip; true when clicked.
pub(super) fn button(ui: &mut egui::Ui, th: &Theme, r: Rect, id: (&str, usize), active: bool, tip: &str, ic: Icon) -> bool {
    button_text(ui, th, r, id, active, tip, ic, (None, false))
}
#[allow(clippy::too_many_arguments)] // a painter-style helper; a params struct would only add noise at the 6 call sites
pub(super) fn button_text(ui: &mut egui::Ui, th: &Theme, r: Rect, id: impl std::hash::Hash, active: bool, tip: &str, ic: Icon, (text, caret): (Option<&str>, bool)) -> bool {
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
pub(super) fn icon(p: &egui::Painter, ic: Icon, r: Rect, col: Color32, tick: bool) {
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
        Icon::ScaleCage => { poly(&[(2., 2.), (14., 2.), (14., 14.), (2., 14.)]); for (x, y) in [(2., 2.), (14., 2.), (2., 14.), (14., 14.)] { p.rect_filled(Rect::from_center_size(at(x, y), Vec2::splat(3. * s)), 0., col); } }
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
pub(super) fn display_row(ui:&mut egui::Ui,r:Rect,id:(&str,usize),label:&str,selected:bool,ic:Icon,th:&Theme)->bool {
    let response=ui.interact(r,egui::Id::new(("display_row",id)),Sense::click());
    if selected || response.hovered() { ui.painter().rect_filled(r,2.,if selected { th.accent_bg } else { th.panel2 }); }
    icon(ui.painter(),ic,Rect::from_center_size(Pos2::new(r.left()+12.,r.center().y),Vec2::splat(18.)),th.text,false);
    ui.painter().text(Pos2::new(r.left()+28.,r.center().y),egui::Align2::LEFT_CENTER,label,egui::FontId::proportional(12.),th.text);
    response.clicked()
}
pub(super) fn show_shading_pie(ctx:&egui::Context, viewport:Rect,v:&View,out:&mut Actions) {
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
