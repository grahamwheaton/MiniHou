//! Screen-space transform handles over the cooked viewport image.
use super::{Actions, CameraInfo, Orientation, Pivot, Tool, TransformOp, UiAction, View};
use eframe::egui::{self, pos2, vec2, Align2, FontId, Pos2, Rect, Sense, Stroke, Vec2};

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

#[derive(Clone)]
struct Drag { axis: usize, kind: Tool, start: Pos2, last: f32, typed: String }
#[derive(Clone)]
struct PlaneDrag { a: usize, b: usize, start: Pos2, last: [f32; 3] }
pub fn active(ctx: &egui::Context) -> bool { ctx.data(|d| d.get_temp::<Drag>(egui::Id::new("transform_gizmo_drag")).is_some()
    || d.get_temp::<PlaneDrag>(egui::Id::new("transform_plane_drag")).is_some()) }
fn value(kind: Tool, pixels: f32, radius: f32) -> f32 {
    match kind { Tool::Rotate => pixels * 0.65, Tool::Scale => (pixels / 120.).exp() - 1., _ => pixels * radius / 120. }
}
fn snapped(x: f32, step: f32, enabled: bool) -> f32 { if enabled && step > 0. { (x / step).round() * step } else { x } }
fn op(kind: Tool, axis: usize, amount: f32) -> TransformOp {
    let mut v = [0.; 3]; v[axis] = amount;
    match kind { Tool::Rotate => TransformOp::Rotate(v), Tool::Scale => TransformOp::Scale(v), _ => TransformOp::Move(v) }
}
fn axis_frame(v: &View) -> [[f32; 3]; 3] {
    match v.editor.orientation {
        Orientation::View => { let c = v.camera; [normal(c.ray_right), normal(c.ray_up), normal(c.ray_forward)] }
        Orientation::Local | Orientation::Gimbal => {
            let r = &v.doc["nodes"][v.selected]["params"]["rotate"];
            let deg = [0,1,2].map(|i| r.get(i).and_then(|x| x.as_f64()).unwrap_or(0.) as f32);
            let [x,y,z] = deg.map(f32::to_radians); let (sx,cx) = x.sin_cos(); let (sy,cy) = y.sin_cos(); let (sz,cz) = z.sin_cos();
            [[cz*cy, sz*cy, -sy], [cz*sy*sx-sz*cx, sz*sy*sx+cz*cx, cy*sx], [cz*sy*cx+sz*sx, sz*sy*cx-cz*sx, cy*cx]]
        }
        _ => [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
    }
}
pub fn hit(rect: Rect, v: &View, pointer: Pos2) -> bool {
    if !matches!(v.editor.tool, Tool::Move | Tool::Rotate | Tool::Scale | Tool::Transform) || v.selected.is_empty() { return false; }
    let origin = pivot(v);
    let Some(center) = project(v.camera, rect, origin) else { return false; };
    let frame = axis_frame(v);
    for axis in 0..3 {
        if v.editor.tool == Tool::Rotate {
            if ((pointer - center).length() - (42. + axis as f32 * 8.)).abs() < 7. { return true; }
        } else {
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
    let p = ui.painter_at(rect); let color = v.theme.accent;
    p.circle_stroke(at, 6., Stroke::new(1_f32, color));
    p.line_segment([at + vec2(-11., 0.), at + vec2(11., 0.)], Stroke::new(1_f32, color));
    p.line_segment([at + vec2(0., -11.), at + vec2(0., 11.)], Stroke::new(1_f32, color));
}

pub fn show(ui: &mut egui::Ui, rect: Rect, v: &View, out: &mut Actions) -> bool {
    let tool = v.editor.tool;
    if !matches!(tool, Tool::Move | Tool::Rotate | Tool::Scale | Tool::Transform) || v.selected.is_empty() { return false; }
    let origin = pivot(v);
    let Some(center) = project(v.camera, rect, origin) else { return false; };
    let p = ui.painter_at(rect);
    let id = egui::Id::new("transform_gizmo_drag");
    let mut drag: Option<Drag> = ui.ctx().data_mut(|d| d.get_temp(id));
    let plane_id = egui::Id::new("transform_plane_drag");
    let mut plane_drag: Option<PlaneDrag> = ui.ctx().data_mut(|d| d.get_temp(plane_id));
    let colors = [v.theme.x_axis, v.theme.y_axis, v.theme.z_axis];
    let frame = axis_frame(v);
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
        let pointer = ui.input(|i| i.pointer.hover_pos());
        let over = pointer.is_some_and(|q| if tool == Tool::Rotate { ((q - center).length() - (42. + axis as f32 * 8.)).abs() < 7. } else { (q - tip).length() <= 12. });
        let hover = over || response.hovered() || drag.as_ref().is_some_and(|d| d.axis == axis);
        let color = if hover { v.theme.sel } else { colors[axis] };
        if tool == Tool::Rotate {
            p.circle_stroke(center, 42. + axis as f32 * 8., Stroke::new(if hover { 2.5_f32 } else { 1.5_f32 }, color));
        } else {
            p.line_segment([center, tip], Stroke::new(if hover { 3_f32 } else { 2_f32 }, color));
            if tool == Tool::Scale { p.rect_filled(hit.shrink(4.), 1., color); }
            else { p.add(egui::Shape::convex_polygon(vec![tip, tip - dir.normalized() * 10. + dir.rot90().normalized() * 5., tip - dir.normalized() * 10. - dir.rot90().normalized() * 5.], color, Stroke::NONE)); }
        }
        if (response.drag_started() || (over && ui.input(|i| i.pointer.primary_pressed()))) && drag.is_none() { drag = Some(Drag { axis, kind: if tool == Tool::Transform { Tool::Move } else { tool }, start: pointer.unwrap_or(tip), last: 0., typed: String::new() }); consumed = true; }
    }
    p.circle_filled(center, 4., v.theme.accent);
    if let Some(mut d) = drag {
        let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
        let down = ui.input(|i| i.pointer.primary_down());
        if escape || !down { ui.ctx().data_mut(|data| data.remove::<Drag>(id)); if escape { out.push(UiAction::CancelTransform); } }
        else {
            let events = ui.input(|i| i.events.clone());
            for event in events { match event {
                egui::Event::Text(s) if s.chars().all(|c| c.is_ascii_digit() || c == '.' || c == '-') => d.typed.push_str(&s),
                egui::Event::Key { key: egui::Key::Backspace, pressed: true, .. } => { d.typed.pop(); },
                _ => {}
            } }
            let pointer = ui.input(|i| i.pointer.interact_pos()).unwrap_or(d.start);
            let axis_screen = project(v.camera, rect, add(origin, mul(frame[d.axis], v.bounds.1.max(0.1) * 0.4))).map(|p| p - center).unwrap_or(vec2(1., 0.)).normalized();
            let pixels = (pointer - d.start).dot(axis_screen);
            let typed = d.typed.parse::<f32>().ok().map(|x| if d.kind == Tool::Scale { x - 1. } else { x });
            let raw = typed.unwrap_or_else(|| if d.kind == Tool::Rotate {
                let a = d.start - center; let b = pointer - center;
                (a.x * b.y - a.y * b.x).atan2(a.dot(b)).to_degrees()
            } else { value(d.kind, pixels, v.bounds.1.max(0.1)) });
            let ctrl = ui.input(|i| i.modifiers.ctrl);
            let step = if d.kind == Tool::Rotate { 15. } else if d.kind == Tool::Scale { 0.1 } else { v.editor.snap.increment };
            let current = snapped(raw, step, v.editor.snap.enabled != ctrl);
            if (current - d.last).abs() > 1e-6 {
                let delta = current - d.last;
                let operation = if d.kind == Tool::Move { TransformOp::Move(mul(frame[d.axis], delta)) } else { op(d.kind, d.axis, delta) };
                out.push(UiAction::TransformNode { node: v.selected.into(), op: operation, continuous: true });
                d.last = current;
            }
            let shown = if d.kind == Tool::Scale { current + 1. } else { current };
            p.text(pointer + vec2(12., -12.), Align2::LEFT_BOTTOM, format!("{shown:.3}"), FontId::monospace(12.), v.theme.text);
            ui.ctx().data_mut(|data| data.insert_temp(id, d));
        }
    }
    if let Some(mut d) = plane_drag {
        let down = ui.input(|i| i.pointer.primary_down());
        if !down || ui.input(|i| i.key_pressed(egui::Key::Escape)) { ui.ctx().data_mut(|data| data.remove::<PlaneDrag>(plane_id)); if ui.input(|i| i.key_pressed(egui::Key::Escape)) { out.push(UiAction::CancelTransform); } }
        else {
            let pointer = ui.input(|i| i.pointer.hover_pos()).unwrap_or(d.start);
            let unit = v.bounds.1.max(0.1) * 0.4 / 58.;
            let (x,y) = plane_delta(pointer - d.start, screen_axis(rect,v,center,frame,d.a), screen_axis(rect,v,center,frame,d.b), unit);
            let snap = v.editor.snap.enabled != ui.input(|i| i.modifiers.ctrl);
            let world = add(mul(frame[d.a], snapped(x,v.editor.snap.increment,snap)), mul(frame[d.b], snapped(y,v.editor.snap.increment,snap)));
            let delta = [0,1,2].map(|k| world[k] - d.last[k]);
            if delta.iter().any(|x| x.abs() > 1e-6) { out.push(UiAction::TransformNode { node: v.selected.into(), op: TransformOp::Move(delta), continuous: true }); d.last = world; }
            ui.ctx().data_mut(|data| data.insert_temp(plane_id, d));
        }
    }
    consumed
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn snap_and_op() { assert_eq!(snapped(0.62, 0.25, true), 0.5); assert_eq!(op(Tool::Move, 1, 2.), TransformOp::Move([0., 2., 0.])); }
    #[test] fn plane_projection() { assert_eq!(plane_delta(vec2(20., -10.), vec2(1.,0.), vec2(0.,-1.), 0.1), (2., 1.)); }
}
