//! Projected transform handles over the cooked viewport image.
use super::{Actions, CameraInfo, Orientation, Pivot, SelectionMode, Tool, TransformOp, UiAction, View};
use eframe::egui::{self, pos2, vec2, Align2, Color32, FontId, Pos2, Rect, Stroke, Vec2};

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
    if camera.ortho {
        let origin = [0, 1, 2].map(|k| camera.ray_origin[k] + x * aspect * camera.ray_right[k] + y * camera.ray_up[k]);
        (origin, camera.ray_forward)
    } else {
        // CameraInfo stores the difference between the *normalised* corner ray and
        // the centre ray. Recover its unnormalised camera-plane basis before
        // interpolation; linear interpolation of unit rays misses off-centre
        // points, causing edge visibility tests to reject the visible surface.
        let basis = |delta: [f32; 3]| {
            let corner = [0, 1, 2].map(|k| camera.ray_forward[k] + delta[k]);
            let along = dot(corner, camera.ray_forward).max(1e-6);
            [0, 1, 2].map(|k| (corner[k] - along * camera.ray_forward[k]) / along)
        };
        let right = basis(camera.ray_right);
        let up = basis(camera.ray_up);
        let dir = [0, 1, 2].map(|k| camera.ray_forward[k] + x * aspect * right[k] + y * up[k]);
        (camera.ray_origin, normal(dir))
    }
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
/// Differential projection at the pivot keeps the widget's size independent of zoom and bounds.
fn projected_basis(camera: CameraInfo, rect: Rect, origin: [f32; 3], axis: [f32; 3]) -> Vec2 {
    let m = camera.projection;
    let clip = |row: usize| m[3][row] + (0..3).map(|k| m[k][row]*origin[k]).sum::<f32>();
    let d = |row: usize| (0..3).map(|k| m[k][row]*axis[k]).sum::<f32>();
    let w = clip(3).max(1e-5);
    vec2((d(0)*w-clip(0)*d(3))/w.powi(2), -(d(1)*w-clip(1)*d(3))/w.powi(2)) * rect.height()*0.5
}
fn gizmo_span(camera: CameraInfo, rect: Rect, origin: [f32;3], frame: [[f32;3];3]) -> f32 {
    82. / frame.into_iter().map(|a| projected_basis(camera,rect,origin,a).length()).fold(1e-6, f32::max)
}
fn ring_basis(camera: CameraInfo, rect: Rect, origin: [f32; 3], frame: [[f32; 3]; 3], axis: usize) -> (Vec2, Vec2) {
    let a = frame[(axis + 1) % 3];
    let b = frame[(axis + 2) % 3];
    let projected_a = projected_basis(camera, rect, origin, a);
    let projected_b = projected_basis(camera, rect, origin, b);
    // The world plane can project to a line. Use its visible direction to orient
    // a screen-facing ring, and a perpendicular direction for the collapsed one.
    let first = if projected_a.length_sq() > 1e-4 { projected_a.normalized() }
        else if projected_b.length_sq() > 1e-4 { -projected_b.rot90().normalized() }
        else { vec2(1., 0.) };
    let determinant = projected_a.x * projected_b.y - projected_a.y * projected_b.x;
    let winding = if determinant.abs() > 1e-3 { determinant.signum() }
        else { let facing = dot(frame[axis], camera.ray_forward); if facing.abs() > 1e-4 { facing.signum() } else { 1. } };
    (first, first.rot90() * winding)
}
fn ring_points(camera: CameraInfo, rect: Rect, origin: [f32; 3], frame: [[f32; 3]; 3], axis: usize) -> Vec<(Pos2, bool)> {
    let a = frame[(axis + 1) % 3];
    let b = frame[(axis + 2) % 3];
    let center = project(camera, rect, origin).unwrap_or(rect.center());
    // Separate the three screen-facing rings so each remains independently hittable.
    let radius = [42., 54., 66.][axis];
    let (screen_a, screen_b) = ring_basis(camera, rect, origin, frame, axis);
    (0..=96).map(|i| {
        let theta = i as f32 * std::f32::consts::TAU / 96.;
        let offset = add(mul(a, theta.cos()), mul(b, theta.sin()));
        (center + (screen_a * theta.cos() + screen_b * theta.sin()) * radius,
            dot(offset, camera.ray_forward) <= 0.)
    }).collect()
}
fn ring_handle(ring: &[(Pos2, bool)]) -> Pos2 { ring[0].0 }
fn ring_hit(rings: &[Vec<(Pos2, bool)>; 3], pointer: Pos2) -> Option<usize> {
    rings.iter().enumerate().filter_map(|(axis, ring)| {
        let distance = ring.windows(2).map(|pair| screen_segment_distance(pointer, pair[0].0, pair[1].0)).fold(f32::INFINITY, f32::min);
        (distance < 7.).then_some((axis, distance))
    }).min_by(|a, b| a.1.total_cmp(&b.1)).map(|(axis, _)| axis)
}
fn axis_sweep(camera: CameraInfo, rect: Rect, origin: [f32;3], frame: [[f32;3];3], axis: usize, previous: Pos2, current: Pos2) -> f32 {
    let center=project(camera,rect,origin).unwrap_or(rect.center());
    let (a,b)=ring_basis(camera,rect,origin,frame,axis);
    let point=|p: Pos2| { let delta=p-center; pos2(delta.dot(a),delta.dot(b)) };
    swept_degrees(point(previous),point(current),Pos2::ZERO)
}

#[derive(Clone)]
struct Drag { axis: usize, kind: Tool, start: Pos2, last_pointer: Pos2, swept: f32, last: f32, typed: String,
    origin: [f32; 3], frame: [[f32; 3]; 3], span: f32 }
#[derive(Clone)]
struct PlaneDrag { a: usize, b: usize, start: Pos2, last: [f32; 3],
    origin: [f32; 3], frame: [[f32; 3]; 3], span: f32 }
#[derive(Clone)]
struct FreeDrag { kind: Tool, trackball: bool, start: Pos2, pointer: Pos2, swept: f32,
    origin: [f32;3], frame: [[f32;3];3], last: [f32;3], changed: bool }

fn screen_rotation(axis: [f32;3], degrees: f32, frame: [[f32;3];3]) -> [f32;3] {
    let a = normal(frame.map(|basis|dot(axis,basis))).map(f64::from);
    let (s,c) = (degrees as f64).to_radians().sin_cos();
    let mut m = [[0.;3];3];
    for i in 0..3 { for j in 0..3 { m[i][j] = a[i]*a[j]*(1.-c) + if i==j {c} else {0.}; } }
    m[0][1]-=a[2]*s; m[1][0]+=a[2]*s; m[0][2]+=a[1]*s;
    m[2][0]-=a[1]*s; m[1][2]-=a[0]*s; m[2][1]+=a[0]*s;
    crate::transform_math::matrix_to_euler_xyz(&m,[0.;3]).map(|x|x as f32)
}

fn snap_point(mesh: Option<(&[f32], &[u32])>, v: &View, rect: Rect, pointer: Pos2) -> Option<[f32;3]> {
    let (positions,indices) = mesh?;
    crate::picking::snap_feature(positions,indices,v.camera,rect,pointer,v.editor.snap.to,v.editor.snap.backface_culling).map(|f|f.point)
}
pub fn active(ctx: &egui::Context) -> bool { ctx.data(|d| d.get_temp::<Drag>(egui::Id::new("transform_gizmo_drag")).is_some()
    || d.get_temp::<PlaneDrag>(egui::Id::new("transform_plane_drag")).is_some()
    || d.get_temp::<FreeDrag>(egui::Id::new("transform_free_drag")).is_some()) }
pub fn active_kind(ctx: &egui::Context) -> Option<Tool> { ctx.data(|d|
    d.get_temp::<Drag>(egui::Id::new("transform_gizmo_drag")).map(|drag|drag.kind)
        .or_else(||d.get_temp::<PlaneDrag>(egui::Id::new("transform_plane_drag")).map(|_|Tool::Move))
        .or_else(||d.get_temp::<FreeDrag>(egui::Id::new("transform_free_drag")).map(|drag|drag.kind))) }
/// Release input ownership while retaining the last applied transform.
pub fn finish_input(ctx: &egui::Context) {
    ctx.data_mut(|d| {
        d.remove::<Drag>(egui::Id::new("transform_gizmo_drag"));
        d.remove::<PlaneDrag>(egui::Id::new("transform_plane_drag"));
        d.remove::<FreeDrag>(egui::Id::new("transform_free_drag"));
    });
}
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
        Orientation::Cursor => {
            let r = crate::transform_math::euler_xyz(v.editor.cursor_rotation.map(f64::from));
            crate::transform_math::Frame::from_rotation(&r).axes.map(|axis| axis.map(|x| x as f32))
        }
        Orientation::Normal => v.normal_frame,
        Orientation::Local | Orientation::Gimbal => {
            let trs = crate::transform_math::TrsParams::from_params(&v.doc["nodes"][if v.editor.selection == SelectionMode::Object { v.selected } else { v.display }]["params"]);
            crate::transform_math::Frame::from_rotation(&trs.world_rotation()).axes.map(|v| v.map(|x| x as f32))
        }
        _ => [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
    }
}
pub fn hit(rect: Rect, v: &View, pointer: Pos2) -> bool {
    if !matches!(v.editor.tool, Tool::Move | Tool::Rotate | Tool::Scale | Tool::Transform) || v.display.is_empty()
        || v.selection_center.is_none() { return false; }
    let origin = pivot(v);
    let Some(center) = project(v.camera, rect, origin) else { return false; };
    let frame = axis_frame(v);
    if v.editor.tool == Tool::Rotate {
        let rings = [0, 1, 2].map(|axis| ring_points(v.camera, rect, origin, frame, axis));
        return ring_hit(&rings, pointer).is_some() || (pointer-center).length() <= 80.;
    }
    for axis in 0..3 {
        {
            let dir = screen_axis(rect,v,center,frame,axis);
            if dir.length() > 10. && screen_segment_distance(pointer,center+dir.normalized()*16.,center+dir) <= 9. { return true; }
        }
    }
    if matches!(v.editor.tool, Tool::Move | Tool::Transform) {
        for (a,b) in [(0,1),(0,2),(1,2)] { if plate_hit(plane_plate(rect, v, center, frame, a, b),pointer) { return true; } }
    }
    (pointer-center).length() <= 12.
}
fn screen_axis(rect: Rect, v: &View, _center: Pos2, frame: [[f32; 3]; 3], axis: usize) -> Vec2 {
    let origin = pivot(v);
    projected_basis(v.camera,rect,origin,frame[axis])*gizmo_span(v.camera,rect,origin,frame)
}
fn plane_plate(rect: Rect, v: &View, center: Pos2, frame: [[f32; 3]; 3], a: usize, b: usize) -> [Pos2;4] {
    let (a,b) = (screen_axis(rect,v,center,frame,a),screen_axis(rect,v,center,frame,b));
    [(0.26,0.26),(0.42,0.26),(0.42,0.42),(0.26,0.42)].map(|(x,y)|center+a*x+b*y)
}
fn plate_hit(plate: [Pos2;4], pointer: Pos2) -> bool {
    let a = plate[1]-plate[0]; let b = plate[3]-plate[0];
    let (x,y) = plane_delta(pointer-plate[0],a,b,1.);
    (a.x*b.y-a.y*b.x).abs() > 2. && (0. ..=1.).contains(&x) && (0. ..=1.).contains(&y)
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
    // the viewport UI (toolbars, header, timeline, overlay menus) sits above the cursor
    if !super::viewport_ui::selection_canvas_contains(rect, at) { return; }
    let p = ui.painter_at(super::viewport_ui::cursor_clip(rect));

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
    show_with_snap(ui,rect,v,out,None)
}
pub fn show_with_snap(ui: &mut egui::Ui, rect: Rect, v: &View, out: &mut Actions, mesh: Option<(&[f32], &[u32])>) -> bool {
    let tool = v.editor.tool;
    if !matches!(tool, Tool::Move | Tool::Rotate | Tool::Scale | Tool::Transform) || v.display.is_empty()
        || v.selection_center.is_none() { return false; }
    let origin = pivot(v);
    let Some(center) = project(v.camera, rect, origin) else { return false; };
    let p = ui.painter_at(rect);
    let id = egui::Id::new("transform_gizmo_drag");
    let mut drag: Option<Drag> = ui.ctx().data_mut(|d| d.get_temp(id));
    let plane_id = egui::Id::new("transform_plane_drag");
    let mut plane_drag: Option<PlaneDrag> = ui.ctx().data_mut(|d| d.get_temp(plane_id));
    let colors = [Color32::from_rgb(242,75,91), Color32::from_rgb(132,204,30), Color32::from_rgb(73,144,255)];
    let frame = axis_frame(v);
    let rings = [0, 1, 2].map(|axis| ring_points(v.camera, rect, origin, frame, axis));
    let pointer = ui.input(|i| i.pointer.hover_pos());
    let free_id = egui::Id::new("transform_free_drag");
    let mut free_drag: Option<FreeDrag> = ui.ctx().data_mut(|d|d.get_temp(free_id));
    let span = gizmo_span(v.camera,rect,origin,frame);
    let hovered_ring = if tool == Tool::Rotate { pointer.and_then(|q| ring_hit(&rings, q)) } else { None };
    let hovered_axis=pointer.and_then(|q|(0..3).filter_map(|axis| {
        let dir=screen_axis(rect,v,center,frame,axis);
        let distance=screen_segment_distance(q,center+dir.normalized()*16.,center+dir);
        (dir.length()>10. && distance<=9.).then_some((axis,distance))
    }).min_by(|a,b|a.1.total_cmp(&b.1)).map(|(axis,_)|axis));
    let mut consumed = drag.is_some() || plane_drag.is_some() || free_drag.is_some();
    if matches!(tool, Tool::Move | Tool::Transform) {
        for (a,b) in [(0,1),(0,2),(1,2)] {
            let plate = plane_plate(rect, v, center, frame, a, b);
            let over = pointer.is_some_and(|q| plate_hit(plate,q));
            let hover = over || plane_drag.as_ref().is_some_and(|d|d.a==a && d.b==b);
            let color = colors[3-a-b];
            p.add(egui::Shape::convex_polygon(plate.to_vec(), Color32::from_rgba_unmultiplied(color.r(),color.g(),color.b(),if hover {150} else {65}), Stroke::new(if hover {1.5_f32} else {1_f32},color)));
            if over && ui.input(|i| i.pointer.primary_pressed()) && !consumed {
                plane_drag = Some(PlaneDrag { a, b, start: pointer.unwrap_or(center),
                    last: [0.;3], origin, frame, span }); consumed = true;
            }
        }
    }
    for axis in 0..3 {
        let dir = screen_axis(rect,v,center,frame,axis);
        let tip = if tool == Tool::Rotate { ring_handle(&rings[axis]) } else { center + dir };
        let over = if tool == Tool::Rotate { hovered_ring == Some(axis) } else { hovered_axis == Some(axis) };
        let hover = over || drag.as_ref().is_some_and(|d| d.axis == axis);
        let color = if hover { colors[axis].lerp_to_gamma(Color32::WHITE,0.45) } else { colors[axis] };
        if hover { p.text(tip+vec2(8.,-8.),Align2::LEFT_BOTTOM,["X","Y","Z"][axis],FontId::proportional(13.),color); }
        if tool == Tool::Rotate {
            for pair in rings[axis].windows(2) {
                let col = if pair[0].1 && pair[1].1 { color } else { color.gamma_multiply(0.32) };
                p.line_segment([pair[0].0, pair[1].0], Stroke::new(if hover { 2.5_f32 } else { 1.5_f32 }, col));
            }
        } else {
            if dir.length() < 10. { continue; }
            p.line_segment([center+dir.normalized()*14., tip], Stroke::new(if hover { 3_f32 } else { 2.3_f32 }, color));
            if tool == Tool::Scale {
                let offsets=frame.map(|a|projected_basis(v.camera,rect,origin,a)*span*0.055);
                for (a,b,c) in [(0,1,2),(1,2,0),(2,0,1)] {
                    let side=offsets[c]*if dot(frame[c],v.camera.ray_forward)>0. {-1.} else {1.};
                    p.add(egui::Shape::convex_polygon(vec![tip+side-offsets[a]-offsets[b],tip+side+offsets[a]-offsets[b],tip+side+offsets[a]+offsets[b],tip+side-offsets[a]+offsets[b]],color.gamma_multiply(0.65+a as f32*0.15),Stroke::new(0.6_f32,color)));
                }
            } else {
                let base=tip-dir.normalized()*14.; let side=dir.rot90().normalized()*5.;
                p.add(egui::Shape::convex_polygon(vec![tip,base+side,base-side],color,Stroke::NONE));
                p.add(egui::Shape::convex_polygon(vec![tip,base,base-side],color.gamma_multiply(0.72),Stroke::NONE));
                p.line_segment([base-side,base+side],Stroke::new(1_f32,color));
            }
        }
        if over && ui.input(|i| i.pointer.primary_pressed()) && !consumed { let start = pointer.unwrap_or(tip); drag = Some(Drag { axis, kind: if tool == Tool::Transform { Tool::Move } else { tool }, start, last_pointer: start, swept: 0., last: 0., typed: String::new(),
            origin, frame, span }); consumed = true; }
    }
    let free_over = pointer.is_some_and(|q| if tool==Tool::Rotate {
        hovered_ring.is_none() && (q-center).length() <= 80.
    } else { (q-center).length() <= 12. });
    let white = if free_over || free_drag.is_some() { Color32::WHITE } else { Color32::from_white_alpha(190) };
    if tool==Tool::Rotate {
        p.circle_stroke(center,76.,Stroke::new(1.8_f32,white));
        p.circle_stroke(center,60.,Stroke::new(1_f32,Color32::from_white_alpha(35)));
        if free_over && pointer.is_some_and(|q|(q-center).length()<68.) { p.circle_filled(center,60.,Color32::from_white_alpha(12)); }
    } else if tool==Tool::Scale { p.rect_stroke(Rect::from_center_size(center,vec2(12.,12.)),0.,Stroke::new(2_f32,white),egui::StrokeKind::Inside); }
    else { p.circle_stroke(center,12.,Stroke::new(1.8_f32,white)); }
    if free_over && ui.input(|i|i.pointer.primary_pressed()) && !consumed {
        let start=pointer.unwrap_or(center);
        free_drag=Some(FreeDrag { kind:tool, trackball:tool==Tool::Rotate && (start-center).length()<68., start,pointer:start,swept:0.,origin,frame,last:[0.;3],changed:false });
        consumed=true;
    }
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
            let precision=if ui.input(|i|i.modifiers.shift) {0.1} else {1.};
            if d.kind == Tool::Rotate { d.swept += axis_sweep(v.camera,rect,d.origin,d.frame,d.axis,d.last_pointer,pointer)*precision; d.last_pointer = pointer; }
            let span = d.span;
            let projected = projected_basis(v.camera,rect,d.origin,d.frame[d.axis])*span;
            let pixels = (pointer - d.start).dot(projected.normalized())*precision;
            let typed = if d.kind == Tool::Move {
                crate::units::parse(&d.typed, v.units).map(|x| x as f32)
            } else {
                d.typed.parse::<f32>().ok().map(|x| if d.kind == Tool::Scale { x - 1. } else { x })
            };
            let raw = typed.unwrap_or_else(|| if d.kind == Tool::Rotate { d.swept }
                else if d.kind == Tool::Move { projected_distance(pixels, span, projected.length()) }
            else { value(d.kind, pixels) });
            let ctrl = ui.input(|i| i.modifiers.ctrl);
            let step = if d.kind == Tool::Rotate { 5. } else if d.kind == Tool::Scale { 0.1 } else { v.editor.snap.increment };
            let snap = v.editor.snap.enabled != ctrl;
            let mut current = snapped(raw, step, snap && (d.kind!=Tool::Move || v.editor.snap.to.contains(super::SnapTo::Increment)));
            if d.kind==Tool::Move && snap && typed.is_none() {
                if let Some(target) = snap_point(mesh,v,rect,pointer) {
                    let amount=dot(add(target,mul(d.origin,-1.)),d.frame[d.axis]);
                    if !v.editor.snap.to.contains(super::SnapTo::Increment) || project(v.camera,rect,add(d.origin,mul(d.frame[d.axis],amount))).is_some_and(|p|
                        p.distance(pointer) < project(v.camera,rect,add(d.origin,mul(d.frame[d.axis],current))).map(|p|p.distance(pointer)).unwrap_or(f32::INFINITY)) { current=amount; }
                }
            }
            if (current - d.last).abs() > 1e-6 {
                let operation = if d.kind == Tool::Move { TransformOp::Move(mul(d.frame[d.axis], current)) }
                    else if d.kind == Tool::Scale { op(d.kind, d.axis, current + 1.) }
                    else {
                        let mut v = [0.; 3];
                        v[d.axis] = current;
                        TransformOp::Rotate(v)
                    };
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
            let span = d.span;
            let projected_axis = |axis| projected_basis(v.camera,rect,d.origin,d.frame[axis])*span;
            let precision=if ui.input(|i|i.modifiers.shift) {0.1} else {1.};
            let (x,y) = plane_delta((pointer - d.start)*precision, projected_axis(d.a), projected_axis(d.b), span);
            let snap = v.editor.snap.enabled != ui.input(|i| i.modifiers.ctrl);
            let grid = snap && v.editor.snap.to.contains(super::SnapTo::Increment);
            let mut world = add(mul(d.frame[d.a], snapped(x,v.editor.snap.increment,grid)), mul(d.frame[d.b], snapped(y,v.editor.snap.increment,grid)));
            if snap { if let Some(target)=snap_point(mesh,v,rect,pointer) {
                let delta=add(target,mul(d.origin,-1.));
                let candidate=add(mul(d.frame[d.a],dot(delta,d.frame[d.a])),mul(d.frame[d.b],dot(delta,d.frame[d.b])));
                if !grid || project(v.camera,rect,add(d.origin,candidate)).is_some_and(|p|p.distance(pointer)<project(v.camera,rect,add(d.origin,world)).map(|p|p.distance(pointer)).unwrap_or(f32::INFINITY)) { world=candidate; }
            } }
            if (0..3).any(|k| (world[k] - d.last[k]).abs() > 1e-6) { out.push(UiAction::TransformNode { op: TransformOp::Move(world) }); d.last = world; }
            ui.ctx().data_mut(|data| data.insert_temp(plane_id, d));
        }
    }
    if let Some(mut d)=free_drag {
        let escape=ui.input(|i|i.key_pressed(egui::Key::Escape));
        if escape || !ui.input(|i|i.pointer.primary_down()) {
            ui.ctx().data_mut(|data|data.remove::<FreeDrag>(free_id));
            if escape { out.push(UiAction::CancelTransform); }
            else if d.changed { out.push(UiAction::FinishTransform); }
        } else {
            let pointer=ui.input(|i|i.pointer.latest_pos()).unwrap_or(d.start);
            let center=project(v.camera,rect,d.origin).unwrap_or(center);
            let delta=(pointer-d.start)*if ui.input(|i|i.modifiers.shift) {0.1} else {1.};
            let snap=v.editor.snap.enabled ^ ui.input(|i|i.modifiers.ctrl);
            let step=v.editor.snap.increment;
            let values=match d.kind {
                Tool::Rotate => {
                    if d.trackball {
                        let axis=add(mul(normal(v.camera.ray_up),delta.x),mul(normal(v.camera.ray_right),delta.y));
                        screen_rotation(axis,snapped(delta.length()*0.6,5.,snap),d.frame)
                    } else {
                        d.swept+=swept_degrees(d.pointer,pointer,center)*if ui.input(|i|i.modifiers.shift) {0.1} else {1.};
                        screen_rotation(normal(v.camera.ray_forward),snapped(d.swept,5.,snap),d.frame)
                    }
                }
                Tool::Scale => { let factor=snapped(1.+(delta.x-delta.y)/120.,0.1,snap); [factor;3] }
                _ => {
                    let right=normal(v.camera.ray_right); let up=normal(v.camera.ray_up);
                    let (x,y)=plane_delta(delta,projected_basis(v.camera,rect,d.origin,right),projected_basis(v.camera,rect,d.origin,up),1.);
                    let grid=snap && v.editor.snap.to.contains(super::SnapTo::Increment);
                    let mut world=add(mul(right,x),mul(up,y)).map(|x|snapped(x,step,grid));
                    if snap { if let Some(target)=snap_point(mesh,v,rect,pointer) {
                        if !grid || project(v.camera,rect,add(d.origin,world)).is_none_or(|p|p.distance(pointer)>project(v.camera,rect,target).map(|p|p.distance(pointer)).unwrap_or(f32::INFINITY)) { world=add(target,mul(d.origin,-1.)); }
                    } }
                    world
                }
            };
            d.pointer=pointer;
            let identity=if d.kind==Tool::Scale {[1.;3]} else {[0.;3]};
            if values!=d.last && (d.changed || values!=identity) {
                let op=match d.kind { Tool::Rotate=>TransformOp::Rotate(values),Tool::Scale=>TransformOp::Scale(values),_=>TransformOp::Move(values) };
                out.push(UiAction::TransformNode {op}); d.last=values; d.changed=true;
            }
            ui.ctx().data_mut(|data|data.insert_temp(free_id,d));
        }
    }
    consumed
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn free_handles_drag_and_escape_cancels() {
        use egui::{Event,PointerButton,RawInput};
        for (tool,trackball) in [(Tool::Move,false),(Tool::Scale,false),(Tool::Rotate,false),(Tool::Rotate,true)] {
            let mut f=crate::ui::testing::Fixture::new();
            f.editor.tool=tool;
            f.camera=CameraInfo {projection:[[1.,0.,0.,0.],[0.,1.,0.,0.],[0.,0.,1.,0.],[0.,0.,0.,1.]],
                ortho:true,ray_right:[1.,0.,0.],ray_up:[0.,1.,0.],ray_forward:[0.,0.,-1.],..Default::default()};
            let mut v=f.view(); v.selection_center=Some([0.;3]);
            let ctx=egui::Context::default();
            let run=|events| {
                let mut out=Actions::default(); let mut rect=Rect::NOTHING;
                let _=ctx.run(RawInput {screen_rect:Some(Rect::from_min_size(Pos2::ZERO,vec2(500.,500.))),events,..Default::default()},|ctx| {
                    egui::CentralPanel::default().show(ctx,|ui| { rect=ui.max_rect(); show(ui,rect,&v,&mut out); });
                });
                (out.0,rect)
            };
            let (_,rect)=run(Vec::new());
            let center=rect.center();
            let start=center+if tool==Tool::Rotate {if trackball {vec2(20.,20.)} else {vec2(76.,0.)}} else {Vec2::ZERO};
            assert!(hit(rect,&v,start));
            let (out,_)=run(vec![Event::PointerMoved(start),Event::PointerButton {pos:start,button:PointerButton::Primary,pressed:true,modifiers:Default::default()}]);
            assert!(out.is_empty()); assert!(active(&ctx));
            let end=if tool==Tool::Rotate && !trackball {center+vec2(0.,76.)} else {start+vec2(15.,-10.)};
            let (out,_)=run(vec![Event::PointerMoved(end)]);
            assert!(out.iter().any(|action|matches!(action,UiAction::TransformNode {..})),"{tool:?} trackball={trackball}");
            let (out,_)=run(vec![Event::Key {key:egui::Key::Escape,physical_key:None,pressed:true,repeat:false,modifiers:Default::default()}]);
            assert!(out.iter().any(|action|matches!(action,UiAction::CancelTransform)));
            assert!(!active(&ctx));
        }
    }
    #[test] fn snap_and_op() { assert_eq!(snapped(0.62, 0.25, true), 0.5); assert_eq!(op(Tool::Move, 1, 2.), TransformOp::Move([0., 2., 0.])); assert_eq!(op(Tool::Scale, 1, 2.), TransformOp::Scale([1., 2., 1.])); }
    #[test] fn object_pivot_uses_active_origin_and_no_selection_has_no_handles() {
        let mut f = crate::ui::testing::Fixture::new();
        f.editor.tool = Tool::Move;
        f.editor.selection = SelectionMode::Object;
        let mut v = f.view();
        v.bounds = ([50.; 3], 100.);
        v.selection_center = Some([2., 3., 4.]);
        assert_eq!(pivot(&v), [2., 3., 4.]);
        v.selection_center = None;
        assert!(!hit(Rect::from_min_size(Pos2::ZERO, vec2(400., 400.)), &v, Pos2::ZERO));
    }
    #[test] fn plane_projection() { assert_eq!(plane_delta(vec2(20., -10.), vec2(1.,0.), vec2(0.,-1.), 0.1), (2., 1.)); }
    #[test] fn plates_follow_the_projected_axes_and_reject_bounding_box_corners() {
        let plate=[pos2(0.,0.),pos2(12.,4.),pos2(16.,16.),pos2(4.,12.)];
        assert!(plate_hit(plate,pos2(8.,8.)));
        assert!(!plate_hit(plate,pos2(0.,16.)));
        assert!(!plate_hit([Pos2::ZERO;4],Pos2::ZERO));
    }
    #[test] fn gizmo_size_is_constant_across_zoom_and_perspective_depth() {
        let rect=Rect::from_min_size(Pos2::ZERO,vec2(400.,400.));
        let frame=[[1.,0.,0.],[0.,1.,0.],[0.,0.,1.]];
        for zoom in [0.01,1.,100.] {
            let camera=CameraInfo {projection:[[zoom,0.,0.,0.],[0.,zoom,0.,0.],[0.,0.,1.,1.],[0.,0.,0.,0.]],..Default::default()};
            for depth in [0.1,1.,1000.] {
                let origin=[0.,0.,depth];
                let size=projected_basis(camera,rect,origin,frame[0]).length()*gizmo_span(camera,rect,origin,frame);
                assert!((size-82.).abs()<1e-4);
            }
        }
        let rotated=screen_rotation([0.,0.,1.],90.,frame);
        assert!((rotated[2]-90.).abs()<1e-4);
    }
    #[test] fn axis_drag_uses_projected_world_span() { assert_eq!(projected_distance(40., 2., 80.), 1.); }
    #[test] fn rotation_sweep_continues_past_a_half_turn() {
        let c = Pos2::ZERO;
        let p = [pos2(1.,0.), pos2(0.,1.), pos2(-1.,0.), pos2(0.,-1.), pos2(1.,0.)];
        let total: f32 = p.windows(2).map(|w| swept_degrees(w[0], w[1], c)).sum();
        assert!((total - 360.).abs() < 1e-4);
    }
    #[test] fn rotate_rings_face_camera_and_dim_the_back() {
        let camera = CameraInfo { projection: [[1.,0.,0.,0.],[0.,1.,0.,0.],[0.,0.,1.,0.],[0.,0.,0.,1.]],
            ray_forward: normal([0.3, 0.5, 0.8]), ..Default::default() };
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(400., 400.));
        let frame = [[1.,0.,0.],[0.,1.,0.],[0.,0.,1.]];
        let rings = [0,1,2].map(|axis| ring_points(camera, rect, [0.;3], frame, axis));
        assert!(rings.iter().all(|r| r.len() == 97 && r.iter().any(|p| p.1) && r.iter().any(|p| !p.1)));
        assert_eq!(rings[0][0].0.distance(rect.center()).round(), 42.);
        assert_eq!(rings[1][0].0.distance(rect.center()).round(), 54.);
        assert_eq!(rings[2][0].0.distance(rect.center()).round(), 66.);
        assert_eq!(ring_hit(&rings, rings[2][12].0), Some(2));
    }
    #[test] fn rotation_rings_and_handles_stay_round_at_camera_angles() {
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(400., 400.));
        let frame = [[1.,0.,0.],[0.,1.,0.],[0.,0.,1.]];
        for forward in [[0.,0.,1.], [1.,0.,0.], [0.,1.,0.], [1.,0.0001,0.], [1.,1.,1.]] {
            let forward = normal(forward);
            let reference = if forward[1].abs() > 0.9 { [0.,0.,1.] } else { [0.,1.,0.] };
            let cross = |a: [f32; 3], b: [f32; 3]| [a[1]*b[2]-a[2]*b[1], a[2]*b[0]-a[0]*b[2], a[0]*b[1]-a[1]*b[0]];
            let right = normal(cross(reference, forward));
            let up = cross(forward, right);
            let camera = CameraInfo {
                projection: [[right[0],up[0],forward[0],0.], [right[1],up[1],forward[1],0.],
                    [right[2],up[2],forward[2],0.], [0.,0.,0.,1.]],
                ray_forward: forward, ..Default::default()
            };
            let rings = [0,1,2].map(|axis| ring_points(camera,rect,[0.;3],frame,axis));
            for (axis, ring) in rings.iter().enumerate() {
                let x = ring.iter().map(|p|p.0.x).fold(f32::NEG_INFINITY,f32::max)
                    - ring.iter().map(|p|p.0.x).fold(f32::INFINITY,f32::min);
                let y = ring.iter().map(|p|p.0.y).fold(f32::NEG_INFINITY,f32::max)
                    - ring.iter().map(|p|p.0.y).fold(f32::INFINITY,f32::min);
                assert!(x.min(y)/x.max(y) > 0.95, "axis={axis} forward={forward:?} bounds={x}x{y}");
                let handle = ring_handle(ring);
                assert!((handle.distance(rect.center()) - [42.,54.,66.][axis]).abs() < 0.01);
                let distance = ring.windows(2).map(|pair|screen_segment_distance(handle,pair[0].0,pair[1].0))
                    .fold(f32::INFINITY,f32::min);
                assert!(distance < 0.01, "axis={axis} forward={forward:?} distance={distance}");
                assert_eq!(ring_hit(&rings,handle),Some(axis));
            }
        }
    }
    #[test] fn perspective_ring_sweep_matches_its_screen_direction() {
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(600., 400.));
        let camera = CameraInfo {
            projection: [[1.,0.,0.,0.],[0.,1.,0.,0.],[0.,0.,1.,1.],[0.,0.,0.,0.]],
            ray_forward: [0.,0.,1.], ..Default::default()
        };
        let origin = [0.3,0.4,3.];
        let frame = [[1.,0.,0.],[0.,1.,0.],[0.,0.,1.]];
        for axis in 0..3 {
            let ring = ring_points(camera,rect,origin,frame,axis);
            let center = project(camera,rect,origin).unwrap();
            let radius = [42.,54.,66.][axis];
            assert!(ring.iter().all(|p| (p.0.distance(center)-radius).abs()<0.01));
            assert!((axis_sweep(camera,rect,origin,frame,axis,ring[0].0,ring[24].0)-90.).abs()<0.01);
        }
    }
    #[test] fn move_drag_shows_units() {
        let mut f = crate::ui::testing::Fixture::new();
        f.camera.projection = [[1.,0.,0.,0.],[0.,1.,0.,0.],[0.,0.,1.,0.],[0.,0.,0.,1.]];
        let editor = crate::ui::EditorState { tool: Tool::Move, ..Default::default() };
        let mut view = f.view();
        view.editor = &editor;
        view.selection_center = Some([0.; 3]);
        let mut actions = Actions::default();
        let id = egui::Id::new("transform_gizmo_drag");
        let d = Drag {
            axis: 0, kind: Tool::Move, start: Pos2::ZERO, last_pointer: Pos2::ZERO,
            swept: 0., typed: "1000".into(), last: 0.0,
            origin: [0.; 3], frame: [[1.,0.,0.],[0.,1.,0.],[0.,0.,1.]], span: 0.4,
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
