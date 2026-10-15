//! Plane sketch and tape tool. UI state stays in egui memory; commits are document actions.
use super::{transform_gizmo, Actions, UiAction, View};
use eframe::egui::{self, pos2, vec2, Align2, FontId, Pos2, Rect, Stroke};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SketchMode { Line, Pen, Polygon, Rectangle, Circle, Tape }
impl SketchMode { fn name(self) -> &'static str { match self { Self::Line => "Line", Self::Pen => "Polyline", Self::Polygon => "Polygon", Self::Rectangle => "Rectangle", Self::Circle => "Circle", Self::Tape => "Measure" } } }
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Plane { #[default] XY, XZ, YZ }
impl Plane {
    pub fn basis(self) -> ([f32; 3], [f32; 3], [f32; 3]) { match self {
        Self::XY => ([1.,0.,0.], [0.,1.,0.], [0.,0.,1.]),
        Self::XZ => ([1.,0.,0.], [0.,0.,1.], [0.,-1.,0.]),
        Self::YZ => ([0.,1.,0.], [0.,0.,1.], [1.,0.,0.]),
    } }
    fn label(self) -> &'static str { match self { Self::XY => "XY", Self::XZ => "XZ", Self::YZ => "YZ" } }
}
fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] { [0,1,2].map(|i| a[i] + b[i]) }
fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] { [0,1,2].map(|i| a[i] - b[i]) }
fn mul(a: [f32; 3], t: f32) -> [f32; 3] { a.map(|x| x * t) }
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 { (0..3).map(|i| a[i] * b[i]).sum() }
fn length(a: [f32; 3]) -> f32 { dot(a, a).sqrt() }
pub fn grid_snap(p: [f32; 3], plane: Plane, step: f32) -> [f32; 3] {
    if step <= 0. { return p; }
    let (_, _, n) = plane.basis();
    [0,1,2].map(|i| if n[i] != 0. { p[i] } else { (p[i] / step).round() * step })
}
pub fn axis_lock(p: [f32; 3], anchor: [f32; 3], axis: usize, plane: Plane) -> [f32; 3] {
    let (_, _, n) = plane.basis();
    if axis > 2 || n[axis] != 0. { return p; }
    [0,1,2].map(|i| if i == axis { p[i] } else { anchor[i] })
}
pub fn numeric_length(p: [f32; 3], anchor: [f32; 3], amount: f32) -> [f32; 3] {
    let d = sub(p, anchor); let len = length(d);
    if len < 1e-8 || amount <= 0. { p } else { add(anchor, mul(d, amount / len)) }
}
pub fn shape(mode: SketchMode, points: &[[f32; 3]], plane: Plane, segments: usize) -> Vec<[f32; 3]> {
    if points.len() < 2 { return points.to_vec(); }
    let (u, v, _) = plane.basis(); let a = points[0]; let d = sub(*points.last().unwrap(), a);
    match mode {
        SketchMode::Rectangle => { let x = mul(u, dot(d, u)); let y = mul(v, dot(d, v)); vec![a, add(a, x), add(add(a, x), y), add(a, y)] }
        SketchMode::Circle => { let r = length(d); (0..segments.clamp(3, 256)).map(|i| { let t = i as f32 * std::f32::consts::TAU / segments.clamp(3, 256) as f32; add(a, add(mul(u, r * t.cos()), mul(v, r * t.sin()))) }).collect() }
        _ => points.to_vec(),
    }
}
pub fn specification(mode: SketchMode, points: &[[f32; 3]], plane: Plane) -> Option<(Vec<[f32; 3]>, bool)> {
    let closed = matches!(mode, SketchMode::Polygon | SketchMode::Rectangle | SketchMode::Circle);
    let points = shape(mode, points, plane, 32);
    if points.len() < if closed { 3 } else { 2 } || points.iter().flatten().any(|x| !x.is_finite()) { return None; }
    if points.iter().enumerate().any(|(i,p)| points[..i].iter().any(|q| length(sub(*p,*q)) < 1e-7)) { return None; }
    if closed {
        let (u,v,n) = plane.basis(); let first = points[0]; let mut area = 0.;
        for (i,p) in points.iter().enumerate() {
            if dot(sub(*p,first),n).abs() > 1e-5 { return None; }
            let q = points[(i+1)%points.len()]; area += dot(*p,u)*dot(q,v) - dot(*p,v)*dot(q,u);
        }
        if area.abs() < 1e-8 { return None; }
    }
    Some((points,closed))
}
fn plane_hit(ray: ([f32; 3], [f32; 3]), plane: Plane, origin: [f32; 3]) -> Option<[f32; 3]> {
    let (_, _, n) = plane.basis(); let denom = dot(ray.1, n);
    if denom.abs() < 1e-5 { return None; }
    let t = dot(sub(origin, ray.0), n) / denom;
    (t >= 0.).then(|| add(ray.0, mul(ray.1, t)))
}
#[derive(Clone, Default)]
struct State { mode: Option<SketchMode>, plane: Plane, points: Vec<[f32; 3]>, hover: Option<[f32; 3]>, axis: Option<usize>, numeric: String, origin: [f32; 3] }
fn id() -> egui::Id { egui::Id::new("sketch_state") }
pub fn start(ctx: &egui::Context, mode: SketchMode) {
    ctx.data_mut(|d| { let mut s: State = d.get_temp(id()).unwrap_or_default(); s.mode = Some(mode); s.points.clear(); s.hover = None; s.numeric.clear(); s.axis = None; d.insert_temp(id(), s); });
}
pub fn active(ctx: &egui::Context) -> bool { ctx.data(|d| d.get_temp::<State>(id()).is_some_and(|s| s.mode.is_some())) }
fn finish(s: &mut State, out: &mut Actions) {
    let Some(mode) = s.mode else { return; };
    if mode == SketchMode::Tape {
        if s.points.len() == 2 { out.push(UiAction::AddMeasurement { a: s.points[0], b: s.points[1] }); s.mode = None; }
        return;
    }
    let Some((points, closed)) = specification(mode, &s.points, s.plane) else { return; };
    out.push(UiAction::SketchCommit { name: mode.name().into(), points, closed });
    s.mode = None; s.points.clear(); s.hover = None;
}
fn measurements(ui: &egui::Ui, rect: Rect, v: &View) {
    let Some(rows) = v.doc.get("measurements").and_then(|x| x.as_array()) else { return; };
    let p = ui.painter_at(rect);
    for row in rows {
        let coords = |key: &str| -> Option<[f32; 3]> { let a = row.get(key)?.as_array()?; Some([a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32, a.get(2)?.as_f64()? as f32]) };
        let (Some(a), Some(b)) = (coords("a"), coords("b")) else { continue; };
        let (Some(pa), Some(pb)) = (transform_gizmo::project(v.camera, rect, a), transform_gizmo::project(v.camera, rect, b)) else { continue; };
        p.line_segment([pa, pb], Stroke::new(1.5_f32, v.theme.tpl));
        p.text(pa.lerp(pb, 0.5) + vec2(8., -8.), Align2::LEFT_BOTTOM, format!("{:.3}", length(sub(b, a))), FontId::monospace(12.), v.theme.tpl);
    }
}
pub fn show(ui: &mut egui::Ui, rect: Rect, v: &View, out: &mut Actions) {
    measurements(ui, rect, v);
    let ctx = ui.ctx().clone(); let mut s: State = ctx.data_mut(|d| d.get_temp(id())).unwrap_or_default();
    if v.editor.tool == super::Tool::Measure && s.mode.is_none() { s.mode = Some(SketchMode::Tape); }
    if v.editor.tool != super::Tool::Measure && s.mode == Some(SketchMode::Tape) { s.mode = None; s.points.clear(); }
    let Some(mode) = s.mode else { return; };
    let pointer = ctx.input(|i| i.pointer.hover_pos());
    let safe = Rect::from_min_max(pos2(rect.left() + 55., rect.top() + 83.), pos2(rect.right() - 55., rect.bottom() - 50.));
    if let Some(at) = pointer.filter(|p| safe.contains(*p)) {
        let ray = transform_gizmo::ray(v.camera, rect, at);
        let anchor = s.points.last().copied();
        s.hover = plane_hit(ray, s.plane, s.origin).map(|p| {
            let ctrl = ctx.input(|i| i.modifiers.ctrl);
            let mut p = grid_snap(p, s.plane, if v.editor.snap.enabled != ctrl { v.editor.snap.increment } else { 0. });
            if let (Some(a), Some(axis)) = (anchor, s.axis) { p = axis_lock(p, a, axis, s.plane); }
            if let (Some(a), Ok(amount)) = (anchor, s.numeric.parse::<f32>()) { p = numeric_length(p, a, amount); }
            p
        });
    } else { s.hover = None; }
    let events = ctx.input(|i| i.events.clone());
    for event in events { match event {
        egui::Event::Key { key: egui::Key::Escape, pressed: true, .. } => { s.mode = None; s.points.clear(); },
        egui::Event::Key { key: egui::Key::Enter, pressed: true, .. } => { if !s.numeric.is_empty() { if let Some(p) = s.hover { s.points.push(p); } s.numeric.clear(); } finish(&mut s, out); },
        egui::Event::Key { key: egui::Key::Backspace, pressed: true, .. } => { if s.numeric.is_empty() { s.points.pop(); } else { s.numeric.pop(); } },
        egui::Event::Key { key, pressed: true, .. } if matches!(key, egui::Key::X | egui::Key::Y | egui::Key::Z) => { let axis = match key { egui::Key::X => 0, egui::Key::Y => 1, _ => 2 }; s.axis = if s.axis == Some(axis) { None } else { Some(axis) }; },
        egui::Event::Text(t) if t.chars().all(|c| c.is_ascii_digit() || c == '.' || c == '-') => s.numeric.push_str(&t),
        egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, .. } if safe.contains(pos) => {
            if let Some(p) = s.hover { if s.points.last().is_none_or(|a| length(sub(*a, p)) > 1e-7) {
                if mode == SketchMode::Polygon && s.points.len() >= 3 && length(sub(s.points[0], p)) < v.bounds.1 * 0.015 { finish(&mut s, out); }
                else { if s.points.is_empty() { s.origin = p; } s.points.push(p); s.numeric.clear(); if matches!(mode, SketchMode::Line | SketchMode::Rectangle | SketchMode::Circle | SketchMode::Tape) && s.points.len() == 2 { finish(&mut s, out); } }
            } } },
        _ => {}
    } }
    let p = ui.painter_at(rect);
    let mut preview = s.points.clone(); if let Some(h) = s.hover { preview.push(h); }
    let preview = shape(mode, &preview, s.plane, 32);
    let screen: Vec<Pos2> = preview.iter().filter_map(|q| transform_gizmo::project(v.camera, rect, *q)).collect();
    for pair in screen.windows(2) { p.line_segment([pair[0], pair[1]], Stroke::new(2_f32, v.theme.warn)); }
    if matches!(mode, SketchMode::Polygon | SketchMode::Rectangle | SketchMode::Circle) && screen.len() >= 3 { p.line_segment([*screen.last().unwrap(), screen[0]], Stroke::new(2_f32, v.theme.warn)); }
    for q in &screen { p.circle_filled(*q, 3., v.theme.warn); }
    let chip = Rect::from_min_size(pos2(rect.left() + 56., rect.top() + 48.), vec2(265., 28.));
    p.rect_filled(chip, 2., v.theme.panel); p.rect_stroke(chip, 2., Stroke::new(1_f32, v.theme.stroke), egui::StrokeKind::Inside);
    p.text(chip.left_center() + vec2(8., 0.), Align2::LEFT_CENTER, format!("{} · {}  {}  Enter finish · Esc cancel", mode.name(), s.plane.label(), s.numeric), FontId::monospace(10.), v.theme.text);
    for (i, plane) in [Plane::XY, Plane::XZ, Plane::YZ].into_iter().enumerate() {
        let r = Rect::from_min_size(pos2(chip.right() + 4. + i as f32 * 31., chip.top()), vec2(29., 28.));
        let resp = ui.interact(r, egui::Id::new(("sketch_plane", i)), egui::Sense::click());
        p.rect_filled(r, 2., if s.plane == plane { v.theme.accent_bg } else { v.theme.panel });
        p.text(r.center(), Align2::CENTER_CENTER, plane.label(), FontId::monospace(10.), if s.plane == plane { v.theme.accent } else { v.theme.dim });
        if resp.clicked() { s.plane = plane; s.points.clear(); s.origin = [0.; 3]; }
    }
    ctx.data_mut(|d| d.insert_temp(id(), s));
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn geometry_and_constraints() {
        assert_eq!(grid_snap([0.26,0.74,3.], Plane::XY, 0.25), [0.25,0.75,3.]);
        assert_eq!(axis_lock([2.,3.,0.], [1.,1.,0.], 0, Plane::XY), [2.,1.,0.]);
        assert_eq!(numeric_length([2.,0.,0.], [0.;3], 3.), [3.,0.,0.]);
        let r = shape(SketchMode::Rectangle, &[[0.;3], [2.,3.,0.]], Plane::XY, 32);
        assert_eq!(r, vec![[0.,0.,0.],[2.,0.,0.],[2.,3.,0.],[0.,3.,0.]]);
        assert_eq!(shape(SketchMode::Circle, &[[0.;3], [1.,0.,0.]], Plane::XY, 32).len(), 32);
        assert!(specification(SketchMode::Polygon, &[[0.;3],[1.,0.,0.],[2.,0.,0.]], Plane::XY).is_none());
        assert!(specification(SketchMode::Polygon, &[[0.;3],[1.,0.,0.],[0.,1.,0.]], Plane::XY).is_some());
    }
    #[test] fn headless_frame() { let f = crate::ui::testing::Fixture::new(); assert!(f.run(|ui,v,out| show(ui, ui.max_rect(), v, out)).is_empty()); }
}
