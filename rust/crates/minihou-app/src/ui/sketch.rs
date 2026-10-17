//! Plane sketch and tape tool. UI state stays in egui memory; commits are document actions.
use super::{transform_gizmo, Actions, UiAction, View};
use eframe::egui::{self, pos2, vec2, Align2, FontId, Pos2, Rect, Stroke};
use super::sketch_inference::{self, Hit, Kind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SketchMode { Line, Pen, Polygon, Rectangle, Circle, Tape }
impl SketchMode {
    /// The drawing modes of the old sketch panel, in its button order (Tape is the Measure tool).
    pub const DRAW: [Self; 5] = [Self::Line, Self::Pen, Self::Polygon, Self::Rectangle, Self::Circle];
    /// Command id that starts this mode (registered in `keymap.rs`, unbound by default like the old build).
    pub fn command_id(self) -> &'static str { match self { Self::Line => "sketch_line", Self::Pen => "sketch_polyline", Self::Polygon => "sketch_polygon", Self::Rectangle => "sketch_rectangle", Self::Circle => "sketch_circle", Self::Tape => "sketch_measure" } }
    pub fn name(self) -> &'static str { match self { Self::Line => "Line", Self::Pen => "Polyline", Self::Polygon => "Polygon", Self::Rectangle => "Rectangle", Self::Circle => "Circle", Self::Tape => "Measure" } }
    pub fn tool(self) -> super::Tool { match self {
        Self::Line => super::Tool::Line, Self::Pen => super::Tool::Polyline,
        Self::Polygon => super::Tool::Polygon, Self::Rectangle => super::Tool::Rectangle,
        Self::Circle => super::Tool::Circle, Self::Tape => super::Tool::Measure,
    } }
    pub fn from_tool(tool: super::Tool) -> Option<Self> {
        Self::DRAW.into_iter().chain([Self::Tape]).find(|m| m.tool() == tool)
    }
    fn short(self) -> &'static str { match self { Self::Line => "Line", Self::Pen => "Poly", Self::Polygon => "Polygon", Self::Rectangle => "Rect", Self::Circle => "Circle", Self::Tape => "Tape" } }
}
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
        let xy: Vec<[f32; 2]> = points.iter().map(|p| [dot(*p,u), dot(*p,v)]).collect();
        if outline_crosses(&xy) { return None; }
    }
    Some((points,closed))
}
fn outline_crosses(points: &[[f32; 2]]) -> bool {
    let cross = |a: [f32; 2], b: [f32; 2], c: [f32; 2]| (b[0]-a[0])*(c[1]-a[1]) - (b[1]-a[1])*(c[0]-a[0]);
    for i in 0..points.len() {
        for j in i+2..points.len() {
            if i == 0 && j == points.len()-1 { continue; }
            let (a,b,c,d) = (points[i], points[(i+1)%points.len()], points[j], points[(j+1)%points.len()]);
            if cross(a,b,c)*cross(a,b,d) < 0. && cross(c,d,a)*cross(c,d,b) < 0. { return true; }
        }
    }
    false
}
fn plane_hit(ray: ([f32; 3], [f32; 3]), plane: Plane, origin: [f32; 3]) -> Option<[f32; 3]> {
    let (_, _, n) = plane.basis(); let denom = dot(ray.1, n);
    if denom.abs() < 1e-5 { return None; }
    let t = dot(sub(origin, ray.0), n) / denom;
    (t >= 0.).then(|| add(ray.0, mul(ray.1, t)))
}
#[derive(Clone, Default)]
struct State { mode: Option<SketchMode>, plane: Plane, points: Vec<[f32; 3]>, hover: Option<[f32; 3]>, inference: Option<Hit>, reference_edge: Option<([f32;3],[f32;3])>, reference_point: Option<[f32;3]>, axis: Option<usize>, numeric: String, origin: [f32; 3] }
fn id() -> egui::Id { egui::Id::new("sketch_state") }
pub fn start(ctx: &egui::Context, mode: SketchMode) {
    ctx.data_mut(|d| { let mut s: State = d.get_temp(id()).unwrap_or_default(); s.mode = Some(mode); s.points.clear(); s.hover = None; s.numeric.clear(); s.axis = None; d.insert_temp(id(), s); });
}
pub fn cancel(ctx: &egui::Context) { ctx.data_mut(|d| d.remove::<State>(id())); }
pub fn active(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<State>(id()).is_some_and(|s| s.mode.is_some() && (s.mode != Some(SketchMode::Tape) || !s.points.is_empty())))
}
/// The persistent ruler tool must allow orbit/pan, including between its two endpoints.
pub fn blocks_navigation(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<State>(id()).is_some_and(|s| s.mode.is_some() && s.mode != Some(SketchMode::Tape)))
}
fn finish(s: &mut State, out: &mut Actions) {
    let Some(mode) = s.mode else { return; };
    if mode == SketchMode::Tape {
        if s.points.len() == 2 { out.push(UiAction::AddMeasurement { a: s.points[0], b: s.points[1] }); s.mode = None; s.points.clear(); s.hover = None; s.numeric.clear(); s.axis = None; }
        return;
    }
    let Some((points, closed)) = specification(mode, &s.points, s.plane) else { return; };
    out.push(UiAction::SketchCommit { name: mode.name().into(), points, closed });
    s.points.clear(); s.hover = None; s.numeric.clear(); s.axis = None;
}
fn measurements(ui: &egui::Ui, rect: Rect, v: &View) {
    let Some(rows) = v.doc.get("measurements").and_then(|x| x.as_array()) else { return; };
    let p = ui.painter_at(rect);
    for row in rows {
        let coords = |key: &str| -> Option<[f32; 3]> { let a = row.get(key)?.as_array()?; Some([a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32, a.get(2)?.as_f64()? as f32]) };
        let (Some(a), Some(b)) = (coords("a"), coords("b")) else { continue; };
        let (Some(pa), Some(pb)) = (transform_gizmo::project(v.camera, rect, a), transform_gizmo::project(v.camera, rect, b)) else { continue; };
        p.line_segment([pa, pb], Stroke::new(1.5_f32, v.theme.tpl));
        p.text(pa.lerp(pb, 0.5) + vec2(8., -8.), Align2::LEFT_BOTTOM, crate::units::format(length(sub(b, a)) as f64, v.units), FontId::monospace(12.), v.theme.tpl);
    }
}
pub fn show(ui: &mut egui::Ui, rect: Rect, v: &View, out: &mut Actions,
    mesh: Option<(&[f32], &[u32])>) {
    measurements(ui, rect, v);
    let ctx = ui.ctx().clone(); let mut s: State = ctx.data_mut(|d| d.get_temp(id())).unwrap_or_default();
    let tool_mode = SketchMode::from_tool(v.editor.tool);
    if s.mode != tool_mode { s = State { mode: tool_mode, plane: s.plane, ..Default::default() }; }
    let Some(mode) = s.mode else { return; };
    let pointer = ctx.input(|i| i.pointer.hover_pos());
    let safe = Rect::from_min_max(pos2(rect.left() + 55., rect.top() + 83.), pos2(rect.right() - 55., rect.bottom() - 50.));
    if let Some(at) = pointer.filter(|p| safe.contains(*p)) {
        let ray = transform_gizmo::ray(v.camera, rect, at);
        let anchor = s.points.last().copied();
        let project = |p| transform_gizmo::project(v.camera, rect, p).map(|screen| (screen, dot(sub(p,ray.0),ray.1)));
        let mesh_hit = mesh.and_then(|(positions,tris)| sketch_inference::infer_mesh(positions,tris,at,project));
        if let (Some((positions,_)),Some(hit))=(mesh,mesh_hit.as_ref()) {
            if hit.indices.len()==2 {
                let at_index=|i:usize|[positions[i*3],positions[i*3+1],positions[i*3+2]];
                s.reference_edge=Some((at_index(hit.indices[0]),at_index(hit.indices[1])));
            } else if hit.kind==Kind::Endpoint { s.reference_point=Some(hit.point); }
        }
        let point_hit = s.points.iter().enumerate().filter_map(|(i,&p)| project(p).map(|(screen,depth)| Hit {
            point:p, kind:Kind::Endpoint, indices:vec![i], weights:vec![1.], distance:screen.distance(at), depth
        })).filter(|h| h.distance<=12.).min_by(|a,b| a.distance.total_cmp(&b.distance));
        let raw = plane_hit(ray, s.plane, s.origin);
        let axis_hit = s.points.last().copied().zip(raw).and_then(|(anchor,p)| sketch_inference::infer_axis(at,anchor,p,project));
        let direction_hit = s.points.last().copied().zip(raw).zip(s.reference_edge)
            .and_then(|((anchor,p),edge)|sketch_inference::infer_direction(at,anchor,p,edge,project));
        let from_hit = s.reference_point.zip(raw).and_then(|(reference,p)| {
            sketch_inference::infer_axis(at,reference,p,project).map(|mut h|{h.kind=Kind::FromPoint;h})
        });
        s.inference = point_hit.or(mesh_hit).or(direction_hit).or(from_hit).or(axis_hit);
        if ctx.input(|i| i.modifiers.shift) {
            if let Some(Kind::Axis(axis)) = s.inference.as_ref().map(|h| h.kind) { s.axis = Some(axis); }
        }
        s.hover = raw.map(|p| {
            let ctrl = ctx.input(|i| i.modifiers.ctrl);
            let mut p = s.inference.as_ref().map_or_else(|| grid_snap(p, s.plane, if v.editor.snap.enabled != ctrl { v.editor.snap.increment } else { 0. }), |h| h.point);
            if let (Some(a), Some(axis)) = (anchor, s.axis) { p = axis_lock(p, a, axis, s.plane); }
            if let (Some(a), Some(amount)) = (anchor, crate::units::parse(&s.numeric, v.units).map(|x| x as f32)) { p = numeric_length(p, a, amount); }
            p
        });
    } else { s.hover = None; s.inference = None; }
    let events = ctx.input(|i| i.events.clone());
    for event in events { match event {
        egui::Event::Key { key: egui::Key::Escape, pressed: true, .. } |
        egui::Event::PointerButton { button: egui::PointerButton::Secondary, pressed: true, .. } => {
            s = State { plane: s.plane, ..Default::default() }; out.push(UiAction::Editor(super::EditorChange::Tool(super::Tool::Select))); break;
        },
        egui::Event::Key { key: egui::Key::Enter, pressed: true, .. } => { if !s.numeric.is_empty() { if let Some(p) = s.hover { s.points.push(p); } s.numeric.clear(); } finish(&mut s, out); },
        egui::Event::Key { key: egui::Key::Backspace, pressed: true, .. } => { if s.numeric.is_empty() { s.points.pop(); } else { s.numeric.pop(); } },
        egui::Event::Key { key: egui::Key::ArrowRight, pressed: true, .. } => { s.axis = Some(0); },
        egui::Event::Key { key: egui::Key::ArrowLeft, pressed: true, .. } => { s.axis = Some(1); },
        egui::Event::Key { key: egui::Key::ArrowUp, pressed: true, .. } => { s.axis = Some(2); },
        egui::Event::Key { key: egui::Key::ArrowDown, pressed: true, .. } => { s.axis = None; },
        egui::Event::Key { key, pressed: true, .. } if matches!(key, egui::Key::X | egui::Key::Y | egui::Key::Z) => { let axis = match key { egui::Key::X => 0, egui::Key::Y => 1, _ => 2 }; s.axis = if s.axis == Some(axis) { None } else { Some(axis) }; },
        egui::Event::Text(t) if t.chars().all(|c| c.is_ascii_digit() || c == '.' || c == '-' || matches!(c, 'm'|'c'|'k'|'i'|'n'|'f'|'t'|'M'|'C'|'K'|'I'|'N'|'F'|'T'|' ')) => s.numeric.push_str(&t),
        egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers } if safe.contains(pos) && !modifiers.alt => {
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
    if let Some(hit) = &s.inference {
        if let Some(screen) = transform_gizmo::project(v.camera, rect, hit.point) {
            let color = match hit.kind { Kind::Axis(0) => egui::Color32::RED, Kind::Axis(1) => egui::Color32::GREEN,
                Kind::Axis(2) => egui::Color32::BLUE, _ => v.theme.accent };
            p.circle_filled(screen, 5., color);
            p.text(screen+vec2(10.,-12.),Align2::LEFT_BOTTOM,hit.kind.label(),FontId::monospace(12.),color);
            if let Some(anchor) = s.points.last().and_then(|q|transform_gizmo::project(v.camera,rect,*q)) {
                if matches!(hit.kind,Kind::Axis(_)) { p.line_segment([anchor,screen],Stroke::new(1.,color)); }
            }
        }
    }
    if mode == SketchMode::Tape && screen.len() == 2 {
        if let (Some(&a), Some(h)) = (s.points.first(), s.hover) {
            p.text(screen[0].lerp(screen[1], 0.5) + vec2(8., -8.), Align2::LEFT_BOTTOM, crate::units::format(length(sub(h, a)) as f64, v.units), FontId::monospace(12.), v.theme.warn);
        }
    }
    let chip = Rect::from_min_size(pos2(rect.left() + 56., rect.top() + 48.), vec2(265., 28.));
    p.rect_filled(chip, 2., v.theme.panel); p.rect_stroke(chip, 2., Stroke::new(1_f32, v.theme.stroke), egui::StrokeKind::Inside);
    p.text(chip.left_center() + vec2(8., 0.), Align2::LEFT_CENTER, format!("{} | {} | {}", mode.name(), s.axis.map(|a| ["X","Y","Z"][a]).unwrap_or("free"), if s.numeric.is_empty() { "type length" } else { &s.numeric }), FontId::monospace(10.), v.theme.text);
    ui.interact(chip, egui::Id::new("sketch_help"), egui::Sense::hover()).on_hover_text("Click points; Enter finishes Polyline/Polygon. Type a length (units supported), X/Y/Z locks an axis, Ctrl toggles grid snapping. Esc or RMB returns to Select.");
    for (i, plane) in [Plane::XY, Plane::XZ, Plane::YZ].into_iter().enumerate() {
        let r = Rect::from_min_size(pos2(chip.right() + 4. + i as f32 * 31., chip.top()), vec2(29., 28.));
        let resp = ui.interact(r, egui::Id::new(("sketch_plane", i)), egui::Sense::click());
        p.rect_filled(r, 2., if s.plane == plane { v.theme.accent_bg } else { v.theme.panel });
        p.text(r.center(), Align2::CENTER_CENTER, plane.label(), FontId::monospace(10.), if s.plane == plane { v.theme.accent } else { v.theme.dim });
        if resp.clicked() { s.plane = plane; s.points.clear(); s.origin = [0.; 3]; }
    }
    let x0 = chip.right() + 4. + 3. * 31. + 6.;
    let (_,_,normal) = s.plane.basis();
    for (axis, label) in ["X", "Y", "Z"].into_iter().enumerate() {
        let r = Rect::from_min_size(pos2(x0 + axis as f32 * 29., chip.top()), vec2(27.,28.));
        let resp = ui.interact(r, egui::Id::new(("sketch_axis",axis)), egui::Sense::click()).on_hover_text("Lock drawing to this axis (X/Y/Z)");
        p.rect_filled(r,2.,if s.axis == Some(axis) { v.theme.accent_bg } else { v.theme.panel });
        p.text(r.center(),Align2::CENTER_CENTER,label,FontId::monospace(10.),if s.axis == Some(axis) { v.theme.accent } else { v.theme.dim });
        if resp.clicked() && normal[axis] == 0. { s.axis = if s.axis == Some(axis) { None } else { Some(axis) }; }
    }
    ctx.data_mut(|d| d.insert_temp(id(), s));
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn ruler_finish_cancel_and_tool_switch_release_navigation() {
        let ctx = egui::Context::default();
        let mut f = crate::ui::testing::Fixture::new();
        f.editor.tool = super::super::Tool::Measure;
        let mut tape = State { mode: Some(SketchMode::Tape), points: vec![[0.; 3], [1., 0., 0.]], ..Default::default() };
        finish(&mut tape, &mut Actions::default());
        assert!(tape.mode.is_none() && tape.points.is_empty());
        ctx.data_mut(|d| d.insert_temp(id(), State { mode: Some(SketchMode::Tape), points: vec![[0.; 3]], ..Default::default() }));
        assert!(active(&ctx));
        assert!(!blocks_navigation(&ctx));
        ctx.data_mut(|d| d.insert_temp(id(), tape));
        assert!(!active(&ctx) && !blocks_navigation(&ctx));
        for event in [
            egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() },
            egui::Event::PointerButton { pos: pos2(200., 200.), button: egui::PointerButton::Secondary, pressed: true, modifiers: Default::default() },
        ] {
            start(&ctx, SketchMode::Tape);
            let _ = ctx.run(egui::RawInput { events: vec![event], ..Default::default() }, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| show(ui, ui.max_rect(), &f.view(), &mut Actions::default(), None));
            });
            assert!(!active(&ctx) && !blocks_navigation(&ctx));
        }
        start(&ctx, SketchMode::Tape);
        f.editor.tool = super::super::Tool::Select;
        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| show(ui, ui.max_rect(), &f.view(), &mut Actions::default(), None));
        });
        assert!(!active(&ctx) && !blocks_navigation(&ctx));
    }
    #[test] fn geometry_and_constraints() {
        assert_eq!(grid_snap([0.26,0.74,3.], Plane::XY, 0.25), [0.25,0.75,3.]);
        assert_eq!(axis_lock([2.,3.,0.], [1.,1.,0.], 0, Plane::XY), [2.,1.,0.]);
        assert_eq!(numeric_length([2.,0.,0.], [0.;3], 3.), [3.,0.,0.]);
        let r = shape(SketchMode::Rectangle, &[[0.;3], [2.,3.,0.]], Plane::XY, 32);
        assert_eq!(r, vec![[0.,0.,0.],[2.,0.,0.],[2.,3.,0.],[0.,3.,0.]]);
        assert_eq!(shape(SketchMode::Circle, &[[0.;3], [1.,0.,0.]], Plane::XY, 32).len(), 32);
        assert!(specification(SketchMode::Polygon, &[[0.;3],[1.,0.,0.],[2.,0.,0.]], Plane::XY).is_none());
        assert!(specification(SketchMode::Polygon, &[[0.;3],[1.,0.,0.],[0.,1.,0.]], Plane::XY).is_some());
        assert!(specification(SketchMode::Polygon, &[[0.,0.,0.],[3.,2.,0.],[0.,2.,0.],[2.,0.,0.]], Plane::XY).is_none());
    }
    #[test] fn headless_frame() { let f = crate::ui::testing::Fixture::new(); assert!(f.run(|ui,v,out| show(ui, ui.max_rect(), v, out, None)).is_empty()); }
    #[test] fn finishing_a_line_and_tape_emits_document_actions() {
        let mut line = State { mode: Some(SketchMode::Line), points: vec![[0.; 3], [2., 0., 0.]], ..Default::default() };
        let mut out = Actions::default();
        finish(&mut line, &mut out);
        assert!(matches!(&out.0[..], [UiAction::SketchCommit { points, closed: false, .. }] if points.len() == 2));
        let mut tape = State { mode: Some(SketchMode::Tape), points: vec![[0.; 3], [0., 3., 0.]], ..Default::default() };
        let mut out = Actions::default();
        finish(&mut tape, &mut out);
        assert!(matches!(&out.0[..], [UiAction::AddMeasurement { a, b }] if *a == [0.; 3] && *b == [0., 3., 0.]));
    }
    #[test] fn every_sketch_mode_emits_the_curve_it_draws() {
        for (mode, source, expected_len, closed) in [
            (SketchMode::Line, vec![[0.;3], [2.,0.,0.]], 2, false),
            (SketchMode::Pen, vec![[0.;3], [2.,0.,0.], [2.,2.,0.]], 3, false),
            (SketchMode::Polygon, vec![[0.;3], [2.,0.,0.], [2.,2.,0.]], 3, true),
            (SketchMode::Rectangle, vec![[0.;3], [2.,2.,0.]], 4, true),
            (SketchMode::Circle, vec![[0.;3], [2.,0.,0.]], 32, true),
        ] {
            let mut state = State { mode: Some(mode), points: source, ..Default::default() };
            let mut out = Actions::default();
            finish(&mut state, &mut out);
            assert!(matches!(&out.0[..], [UiAction::SketchCommit { points, closed: c, .. }]
                if points.len() == expected_len && *c == closed), "{mode:?}");
        }
    }
    #[test] fn quick_shapes_remain_active_across_commits() {
        for mode in SketchMode::DRAW {
            assert_eq!(SketchMode::from_tool(mode.tool()),Some(mode));
            let mut state = State { mode: Some(mode), plane: Plane::XZ, ..Default::default() };
            let mut out = Actions::default();
            for _ in 0..2 {
                state.points = vec![[0.;3],[2.,0.,0.],[2.,0.,2.]];
                state.numeric = "2m".into(); state.axis = Some(0);
                finish(&mut state,&mut out);
                assert_eq!(state.mode,Some(mode)); assert!(state.points.is_empty());
                assert!(state.numeric.is_empty()); assert_eq!(state.axis,None); assert_eq!(state.plane,Plane::XZ);
            }
            assert_eq!(out.0.len(),2);
        }
    }
    #[test] fn drawing_escape_and_rmb_request_select() {
        for mode in SketchMode::DRAW {
            for event in [
                egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() },
                egui::Event::PointerButton { pos: pos2(200.,200.), button: egui::PointerButton::Secondary, pressed: true, modifiers: Default::default() },
            ] {
                let mut f = crate::ui::testing::Fixture::new(); f.editor.tool = mode.tool();
                let ctx = egui::Context::default(); let mut out = Actions::default();
                let _ = ctx.run(egui::RawInput { events:vec![event], ..Default::default() }, |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| show(ui,ui.max_rect(),&f.view(),&mut out,None));
                });
                assert!(out.0.iter().any(|a| matches!(a,UiAction::Editor(super::super::EditorChange::Tool(super::super::Tool::Select)))));
                assert!(!active(&ctx));
            }
        }
    }
    #[test] fn every_draw_mode_has_a_distinct_command() {
        let ids: std::collections::BTreeSet<_> = SketchMode::DRAW.iter().map(|m| m.command_id()).collect();
        assert_eq!(ids.len(), SketchMode::DRAW.len());
    }
    #[test] fn starting_a_mode_replaces_the_previous_one() {
        let ctx = egui::Context::default();
        start(&ctx, SketchMode::Rectangle); assert!(active(&ctx));
        start(&ctx, SketchMode::Polygon);
        let s: State = ctx.data(|d| d.get_temp(id())).unwrap();
        assert_eq!(s.mode, Some(SketchMode::Polygon)); assert!(s.points.is_empty());
    }
    #[test] fn measurements_render_with_units() {
        let mut f = crate::ui::testing::Fixture::new();
        f.camera.projection = [[1.,0.,0.,0.],[0.,1.,0.,0.],[0.,0.,1.,0.],[0.,0.,0.,1.]];
        let mut doc = f.doc.value().clone();
        doc["measurements"] = serde_json::json!([
            { "a": [0.0, 0.0, 0.0], "b": [1.0, 0.0, 0.0] }
        ]);
        f.doc = minihou_app::Document::parse(doc.to_string()).unwrap();
        f.graph = minihou_core::Graph::from_value(f.doc.value()).unwrap();
        let ctx = egui::Context::default();
        f.theme.apply(&ctx);
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1280., 800.))), ..Default::default() };
        let mut out = Actions::default();
        let v = f.view();
        let full = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                show(ui, ui.max_rect(), &v, &mut out, None);
            });
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
        assert!(texts.iter().any(|t| t.contains("1000 mm")), "expected 1000 mm in {texts:?}");
    }
}
