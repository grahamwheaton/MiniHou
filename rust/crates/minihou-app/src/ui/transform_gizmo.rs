//! Projected transform handles over the cooked viewport image.
use super::{Actions, CameraInfo, Orientation, Pivot, Tool, TransformOp, UiAction, View};
use eframe::egui::{self, pos2, vec2, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2};

pub fn project(camera: CameraInfo, rect: Rect, point: [f32; 3]) -> Option<Pos2> {
    let m = camera.projection;
    let q = [point[0], point[1], point[2], 1.];
    let clip = [0, 1, 2, 3].map(|row| (0..4).map(|col| m[col][row] * q[col]).sum::<f32>());
    if clip[3] <= 1e-5 { return None; }
    let aspect = rect.width() / rect.height().max(1.);
    Some(pos2(rect.center().x + clip[0] / clip[3] / aspect * rect.width() * 0.5,
        rect.center().y - clip[1] / clip[3] * rect.height() * 0.5))
}

pub fn ray(camera: CameraInfo, rect: Rect, pos: Pos2) -> ([f32; 3], [f32; 3]) {
    let x = (pos.x - rect.center().x) * 2. / rect.width().max(1.);
    let y = (rect.center().y - pos.y) * 2. / rect.height().max(1.);
    let aspect = rect.width() / rect.height().max(1.);
    let combine = |base: [f32; 3]| [0, 1, 2].map(|k| base[k] + x * aspect * camera.ray_right[k] + y * camera.ray_up[k]);
    if camera.ortho { (combine(camera.ray_origin), camera.ray_forward) }
    else { (camera.ray_origin, normal(combine(camera.ray_forward))) }
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 { (0..3).map(|k| a[k] * b[k]).sum() }
fn normal(a: [f32; 3]) -> [f32; 3] { let n = dot(a, a).sqrt().max(1e-8); a.map(|x| x / n) }
fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] { [0, 1, 2].map(|k| a[k] + b[k]) }
fn mul(a: [f32; 3], x: f32) -> [f32; 3] { a.map(|v| v * x) }
fn screen_segment_distance(q: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let t = ((q - a).dot(ab) / ab.length_sq().max(1e-6)).clamp(0., 1.);
    (q - (a + ab * t)).length()
}
fn swept_degrees(previous: Pos2, current: Pos2, center: Pos2) -> f32 {
    let a = previous - center;
    let b = current - center;
    (a.x * b.y - a.y * b.x).atan2(a.dot(b)).to_degrees()
}
fn ring_points(camera: CameraInfo, rect: Rect, origin: [f32; 3], frame: [[f32; 3]; 3], axis: usize, span: f32) -> Vec<(Pos2, bool)> {
    let a = frame[(axis + 1) % 3];
    let b = frame[(axis + 2) % 3];
    let center = project(camera, rect, origin).unwrap_or(rect.center());
    let projected = [a, b].into_iter().filter_map(|basis| project(camera, rect, add(origin, mul(basis, span)))
        .map(|p| (p - center).length())).fold(0., f32::max);
    let radius = span * (50. / projected.max(1.)).clamp(0.25, 4.);
    (0..=96).filter_map(|i| {
        let theta = i as f32 * std::f32::consts::TAU / 96.;
        let offset = add(mul(a, radius * theta.cos()), mul(b, radius * theta.sin()));
        project(camera, rect, add(origin, offset)).map(|pos| (pos, dot(offset, camera.ray_forward) <= 0.))
    }).collect()
}
fn ring_hit(rings: &[Vec<(Pos2, bool)>; 3], pointer: Pos2) -> Option<usize> {
    rings.iter().enumerate().filter_map(|(axis, ring)| {
        let distance = ring.windows(2).map(|pair| screen_segment_distance(pointer, pair[0].0, pair[1].0)).fold(f32::INFINITY, f32::min);
        (distance < 7.).then_some((axis, distance))
    }).min_by(|a, b| a.1.total_cmp(&b.1)).map(|(axis, _)| axis)
}

#[derive(Clone)]
struct Drag { axis: usize, kind: Tool, start: Pos2, last_pointer: Pos2, swept: f32, last: f32, typed: String }
#[derive(Clone)]
struct PlaneDrag { a: usize, b: usize, start: Pos2, last: [f32; 3] }
pub fn active(ctx: &egui::Context) -> bool { ctx.data(|d| d.get_temp::<Drag>(egui::Id::new("transform_gizmo_drag")).is_some()
    || d.get_temp::<PlaneDrag>(egui::Id::new("transform_plane_drag")).is_some()) }
fn projected_distance(pixels: f32, world_span: f32, screen_span: f32) -> f32 {
    pixels * world_span / screen_span.max(1.)
}
fn value(kind: Tool, pixels: f32) -> f32 {
    match kind { Tool::Scale => (pixels / 120.).exp() - 1., _ => pixels }
}
fn snapped(x: f32, step: f32, enabled: bool) -> f32 { if enabled && step > 0. { (x / step).round() * step } else { x } }
fn op(kind: Tool, axis: usize, amount: f32) -> TransformOp {
    let mut v = if kind == Tool::Scale { [1.; 3] } else { [0.; 3] }; v[axis] = amount;
    match kind { Tool::Rotate => TransformOp::Rotate(v), Tool::Scale => TransformOp::Scale(v), _ => TransformOp::Move(v) }
}
fn axis_frame(v: &View) -> [[f32; 3]; 3] {
    match v.editor.orientation {
        Orientation::View => { let c = v.camera; [normal(c.ray_right), normal(c.ray_up), normal(c.ray_forward)] }
        Orientation::Local | Orientation::Gimbal => {
            let r = &v.doc["nodes"][v.display]["params"]["rotate"];
            let deg = [0,1,2].map(|i| r.get(i).and_then(|x| x.as_f64()).unwrap_or(0.) as f32);
            let [x,y,z] = deg.map(f32::to_radians); let (sx,cx) = x.sin_cos(); let (sy,cy) = y.sin_cos(); let (sz,cz) = z.sin_cos();
            [[cz*cy, sz*cy, -sy], [cz*sy*sx-sz*cx, sz*sy*sx+cz*cx, cy*sx], [cz*sy*cx+sz*sx, sz*sy*cx-cz*sx, cy*cx]]
        }
        _ => [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
    }
}
pub fn hit(rect: Rect, v: &View, pointer: Pos2) -> bool {
    if !matches!(v.editor.tool, Tool::Move | Tool::Rotate | Tool::Scale | Tool::Transform) || v.display.is_empty() { return false; }
    let origin = pivot(v);
    let Some(center) = project(v.camera, rect, origin) else { return false; };
    let frame = axis_frame(v);
    if v.editor.tool == Tool::Rotate {
        let rings = [0, 1, 2].map(|axis| ring_points(v.camera, rect, origin, frame, axis, v.bounds.1.max(0.1) * 0.4));
        return ring_hit(&rings, pointer).is_some();
    }
    for axis in 0..3 {
        {
            let world = add(origin, mul(frame[axis], v.bounds.1.max(0.1) * 0.4));
            let mut dir = project(v.camera, rect, world).map(|q| q - center).unwrap_or(Vec2::ZERO);
            if dir.length() < 3. { dir = [vec2(1., 0.), vec2(0., -1.), vec2(-0.7, 0.7)][axis]; }
            if (pointer - (center + dir.normalized() * 58.)).length() <= 12. { return true; }
        }
    }
    if matches!(v.editor.tool, Tool::Move | Tool::Transform) {
        for (a,b) in [(0,1),(0,2),(1,2)] { if plane_square(rect, v, center, frame, a, b).contains(pointer) { return true; } }
    }
    false
}
fn screen_axis(rect: Rect, v: &View, center: Pos2, frame: [[f32; 3]; 3], axis: usize) -> Vec2 {
    let origin = pivot(v);
    let world = add(origin, mul(frame[axis], v.bounds.1.max(0.1) * 0.4));
    let mut dir = project(v.camera, rect, world).map(|q| q - center).unwrap_or(Vec2::ZERO);
    if dir.length() < 3. { dir = [vec2(1., 0.), vec2(0., -1.), vec2(-0.7, 0.7)][axis]; }
    dir.normalized()
}
fn plane_square(rect: Rect, v: &View, center: Pos2, frame: [[f32; 3]; 3], a: usize, b: usize) -> Rect {
    Rect::from_center_size(center + (screen_axis(rect,v,center,frame,a) + screen_axis(rect,v,center,frame,b)) * 23., vec2(12.,12.))
}
fn plane_delta(delta: Vec2, a: Vec2, b: Vec2, unit: f32) -> (f32, f32) {
    let det = a.x*b.y - a.y*b.x;
    if det.abs() < 1e-3 { return (0., 0.); }
    ((delta.x*b.y - delta.y*b.x) / det * unit, (a.x*delta.y - a.y*delta.x) / det * unit)
}
fn pivot(v: &View) -> [f32; 3] {
    if v.editor.pivot == Pivot::Cursor { v.editor.cursor } else { v.selection_center.unwrap_or(v.bounds.0) }
}
pub fn cursor(ui: &egui::Ui, rect: Rect, v: &View) {
    let Some(at) = project(v.camera, rect, v.editor.cursor) else { return; };
    let p = ui.painter_at(rect);

    // Crosshair extending through the ring with dark outline for contrast
    let ch_len = 14.0;
    let ch_gap = 2.0;
    let ch_dark = Color32::from_black_alpha(200);
    let ch_light = Color32::WHITE;

    for (start, end) in [
        (at + vec2(-ch_len, 0.), at + vec2(-ch_gap, 0.)),
        (at + vec2(ch_gap, 0.), at + vec2(ch_len, 0.)),
        (at + vec2(0., -ch_len), at + vec2(0., -ch_gap)),
        (at + vec2(0., ch_gap), at + vec2(0., ch_len)),
    ] {
        p.line_segment([start, end], Stroke::new(2.5_f32, ch_dark));
        p.line_segment([start, end], Stroke::new(1.0_f32, ch_light));
    }
    p.circle_filled(at, 1.2, ch_light);

    // Blender's red/white dashed ring (16 alternating segments)
    let radius = 10.0;
    let n_segments = 16;
    let red = Color32::from_rgb(220, 40, 40);
    let white = Color32::WHITE;
    let stroke_width = 1.8_f32;

    let sub_steps = 3;
    for i in 0..n_segments {
        let color = if i % 2 == 0 { red } else { white };
        let mut pts = Vec::with_capacity(sub_steps + 1);
        for s in 0..=sub_steps {
            let t = (i as f32 + s as f32 / sub_steps as f32) / n_segments as f32;
            let angle = t * std::f32::consts::TAU;
            pts.push(at + vec2(angle.cos() * radius, angle.sin() * radius));
        }
        for w in pts.windows(2) {
            p.line_segment([w[0], w[1]], Stroke::new(stroke_width, color));
        }
    }
}

pub fn show(ui: &mut egui::Ui, rect: Rect, v: &View, out: &mut Actions) -> bool {
    let tool = v.editor.tool;
    if !matches!(tool, Tool::Move | Tool::Rotate | Tool::Scale | Tool::Transform) || v.display.is_empty() { return false; }
    let origin = pivot(v);
    let Some(center) = project(v.camera, rect, origin) else { return false; };
    let p = ui.painter_at(rect);
    let id = egui::Id::new("transform_gizmo_drag");
    let mut drag: Option<Drag> = ui.ctx().data_mut(|d| d.get_temp(id));
    let plane_id = egui::Id::new("transform_plane_drag");
    let mut plane_drag: Option<PlaneDrag> = ui.ctx().data_mut(|d| d.get_temp(plane_id));
    let colors = [v.theme.x_axis, v.theme.y_axis, v.theme.z_axis];
    let frame = axis_frame(v);
    let rings = [0, 1, 2].map(|axis| ring_points(v.camera, rect, origin, frame, axis, v.bounds.1.max(0.1) * 0.4));
    let pointer = ui.input(|i| i.pointer.hover_pos());
    let hovered_ring = if tool == Tool::Rotate { pointer.and_then(|q| ring_hit(&rings, q)) } else { None };
    let mut consumed = drag.is_some() || plane_drag.is_some();
    if matches!(tool, Tool::Move | Tool::Transform) {
        for (a,b) in [(0,1),(0,2),(1,2)] {
            let r = plane_square(rect, v, center, frame, a, b);
            let over = ui.input(|i| i.pointer.hover_pos()).is_some_and(|q| r.contains(q));
            p.rect(r, 0., if over { v.theme.accent_bg } else { v.theme.panel2 }, Stroke::new(1_f32, if over { v.theme.sel } else { v.theme.stroke2 }), egui::StrokeKind::Inside);
            if over && ui.input(|i| i.pointer.primary_pressed()) && plane_drag.is_none() {
                plane_drag = Some(PlaneDrag { a, b, start: ui.input(|i| i.pointer.hover_pos()).unwrap_or(r.center()), last: [0.;3] }); consumed = true;
            }
        }
    }
    for axis in 0..3 {
        let world = add(origin, mul(frame[axis], v.bounds.1.max(0.1) * 0.4));
        let mut dir = project(v.camera, rect, world).map(|q| q - center).unwrap_or(Vec2::ZERO);
        if dir.length() < 3. { dir = [vec2(1., 0.), vec2(0., -1.), vec2(-0.7, 0.7)][axis]; }
        dir = dir.normalized() * 58.;
        let tip = center + dir;
        let hit = Rect::from_center_size(tip, vec2(18., 18.));
        let response = ui.interact(hit, egui::Id::new(("gizmo", axis)), Sense::click_and_drag());
        let over = if tool == Tool::Rotate { hovered_ring == Some(axis) } else { pointer.is_some_and(|q| (q - tip).length() <= 12.) };
        let hover = over || (tool != Tool::Rotate && response.hovered()) || drag.as_ref().is_some_and(|d| d.axis == axis);
        let color = if hover { v.theme.sel } else { colors[axis] };
        if tool == Tool::Rotate {
            for pair in rings[axis].windows(2) {
                let col = if pair[0].1 && pair[1].1 { color } else { color.gamma_multiply(0.32) };
                p.line_segment([pair[0].0, pair[1].0], Stroke::new(if hover { 2.5_f32 } else { 1.5_f32 }, col));
            }
        } else {
            p.line_segment([center, tip], Stroke::new(if hover { 3_f32 } else { 2_f32 }, color));
            if tool == Tool::Scale { p.rect_filled(hit.shrink(4.), 1., color); }
            else { p.add(egui::Shape::convex_polygon(vec![tip, tip - dir.normalized() * 10. + dir.rot90().normalized() * 5., tip - dir.normalized() * 10. - dir.rot90().normalized() * 5.], color, Stroke::NONE)); }
        }
        if ((tool != Tool::Rotate && response.drag_started()) || (over && ui.input(|i| i.pointer.primary_pressed()))) && drag.is_none() { let start = pointer.unwrap_or(tip); drag = Some(Drag { axis, kind: if tool == Tool::Transform { Tool::Move } else { tool }, start, last_pointer: start, swept: 0., last: 0., typed: String::new() }); consumed = true; }
    }
    p.circle_filled(center, 4., v.theme.accent);
    if let Some(mut d) = drag {
        let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
        let down = ui.input(|i| i.pointer.primary_down());
        if escape || !down {
            ui.ctx().data_mut(|data| data.remove::<Drag>(id));
            if escape { out.push(UiAction::CancelTransform); }
            else if d.last.abs() > 1e-6 { out.push(UiAction::FinishTransform); }
        }
        else {
            let events = ui.input(|i| i.events.clone());
            for event in events { match event {
                egui::Event::Text(s) if s.chars().all(|c| c.is_ascii_digit() || c == '.' || c == '-' || matches!(c, 'm'|'c'|'k'|'i'|'n'|'f'|'t'|'M'|'C'|'K'|'I'|'N'|'F'|'T'|' ')) => d.typed.push_str(&s),
                egui::Event::Key { key: egui::Key::Backspace, pressed: true, .. } => { d.typed.pop(); },
                _ => {}
            } }
            let pointer = ui.input(|i| i.pointer.latest_pos()).unwrap_or(d.start);
            if d.kind == Tool::Rotate { d.swept += swept_degrees(d.last_pointer, pointer, center); d.last_pointer = pointer; }
            let span = v.bounds.1.max(0.1) * 0.4;
            let projected = project(v.camera, rect, add(origin, mul(frame[d.axis], span)))
                .map(|p| p - center).filter(|delta| delta.length() > 1.).unwrap_or(vec2(1., 0.));
            let pixels = (pointer - d.start).dot(projected.normalized());
            let typed = if d.kind == Tool::Move {
                crate::units::parse(&d.typed, v.units).map(|x| x as f32)
            } else {
                d.typed.parse::<f32>().ok().map(|x| if d.kind == Tool::Scale { x - 1. } else { x })
            };
            let raw = typed.unwrap_or_else(|| if d.kind == Tool::Rotate { d.swept }
                else if d.kind == Tool::Move { projected_distance(pixels, span, projected.length()) }
            else { value(d.kind, pixels) });
            let ctrl = ui.input(|i| i.modifiers.ctrl);
            let step = if d.kind == Tool::Rotate { 15. } else if d.kind == Tool::Scale { 0.1 } else { v.editor.snap.increment };
            let current = snapped(raw, step, v.editor.snap.enabled != ctrl);
            if (current - d.last).abs() > 1e-6 {
                let delta = current - d.last;
                let operation = if d.kind == Tool::Move { TransformOp::Move(mul(frame[d.axis], delta)) }
                    else if d.kind == Tool::Scale { op(d.kind, d.axis, current + 1.) }
                    else { op(d.kind, d.axis, delta) };
                out.push(UiAction::TransformNode { op: operation });
                d.last = current;
            }
            let shown = if d.kind == Tool::Scale { current + 1. } else { current };
            let shown_text = if d.kind == Tool::Move {
                crate::units::format(shown as f64, v.units)
            } else {
                format!("{shown:.3}")
            };
            p.text(pointer + vec2(12., -12.), Align2::LEFT_BOTTOM, shown_text, FontId::monospace(12.), v.theme.text);
            ui.ctx().data_mut(|data| data.insert_temp(id, d));
        }
    }
    if let Some(mut d) = plane_drag {
        let down = ui.input(|i| i.pointer.primary_down());
        if !down || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            ui.ctx().data_mut(|data| data.remove::<PlaneDrag>(plane_id));
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) { out.push(UiAction::CancelTransform); }
            else if d.last.iter().any(|x| x.abs() > 1e-6) { out.push(UiAction::FinishTransform); }
        }
        else {
            let pointer = ui.input(|i| i.pointer.hover_pos()).unwrap_or(d.start);
            let span = v.bounds.1.max(0.1) * 0.4;
            let projected_axis = |axis| project(v.camera, rect, add(origin, mul(frame[axis], span)))
                .map(|p| p - center).filter(|delta| delta.length() > 1.)
                .unwrap_or_else(|| screen_axis(rect, v, center, frame, axis) * 58.);
            let (x,y) = plane_delta(pointer - d.start, projected_axis(d.a), projected_axis(d.b), span);
            let snap = v.editor.snap.enabled != ui.input(|i| i.modifiers.ctrl);
            let world = add(mul(frame[d.a], snapped(x,v.editor.snap.increment,snap)), mul(frame[d.b], snapped(y,v.editor.snap.increment,snap)));
            let delta = [0,1,2].map(|k| world[k] - d.last[k]);
            if delta.iter().any(|x| x.abs() > 1e-6) { out.push(UiAction::TransformNode { op: TransformOp::Move(delta) }); d.last = world; }
            ui.ctx().data_mut(|data| data.insert_temp(plane_id, d));
        }
    }
    consumed
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn snap_and_op() { assert_eq!(snapped(0.62, 0.25, true), 0.5); assert_eq!(op(Tool::Move, 1, 2.), TransformOp::Move([0., 2., 0.])); assert_eq!(op(Tool::Scale, 1, 2.), TransformOp::Scale([1., 2., 1.])); }
    #[test] fn plane_projection() { assert_eq!(plane_delta(vec2(20., -10.), vec2(1.,0.), vec2(0.,-1.), 0.1), (2., 1.)); }
    #[test] fn axis_drag_uses_projected_world_span() { assert_eq!(projected_distance(40., 2., 80.), 1.); }
    #[test] fn rotation_sweep_continues_past_a_half_turn() {
        let c = Pos2::ZERO;
        let p = [pos2(1.,0.), pos2(0.,1.), pos2(-1.,0.), pos2(0.,-1.), pos2(1.,0.)];
        let total: f32 = p.windows(2).map(|w| swept_degrees(w[0], w[1], c)).sum();
        assert!((total - 360.).abs() < 1e-4);
    }
    #[test] fn rotate_rings_project_three_axis_planes_and_dim_the_back() {
        let camera = CameraInfo { projection: [[1.,0.,0.,0.],[0.,1.,0.,0.],[0.,0.,1.,0.],[0.,0.,0.,1.]],
            ray_forward: normal([0.3, 0.5, 0.8]), ..Default::default() };
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(400., 400.));
        let frame = [[1.,0.,0.],[0.,1.,0.],[0.,0.,1.]];
        let rings = [0,1,2].map(|axis| ring_points(camera, rect, [0.;3], frame, axis, 0.4));
        assert!(rings.iter().all(|r| r.len() == 97 && r.iter().any(|p| p.1) && r.iter().any(|p| !p.1)));
        let height = |ring: &Vec<(Pos2,bool)>| ring.iter().map(|p| p.0.y).fold(f32::NEG_INFINITY, f32::max)
            - ring.iter().map(|p| p.0.y).fold(f32::INFINITY, f32::min);
        assert!(height(&rings[0]) > 20. && height(&rings[1]) < 1. && height(&rings[2]) > 20.);
        assert_eq!(ring_hit(&rings, rings[2][12].0), Some(2));
    }
    #[test] fn move_drag_shows_units() {
        let mut f = crate::ui::testing::Fixture::new();
        f.camera.projection = [[1.,0.,0.,0.],[0.,1.,0.,0.],[0.,0.,1.,0.],[0.,0.,0.,1.]];
        let editor = crate::ui::EditorState { tool: Tool::Move, ..Default::default() };
        let mut view = f.view();
        view.editor = &editor;
        let mut actions = Actions::default();
        let id = egui::Id::new("transform_gizmo_drag");
        let d = Drag {
            axis: 0, kind: Tool::Move, start: Pos2::ZERO, last_pointer: Pos2::ZERO,
            swept: 0., typed: "1000".into(), last: 0.0,
        };
        let ctx = egui::Context::default();
        ctx.data_mut(|data| data.insert_temp(id, d));
        f.theme.apply(&ctx);
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1280., 800.))),
            events: vec![egui::Event::PointerButton { pos: Pos2::ZERO, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() }],
            ..Default::default()
        };
        let full = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                show(ui, ui.max_rect(), &view, &mut actions);
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
