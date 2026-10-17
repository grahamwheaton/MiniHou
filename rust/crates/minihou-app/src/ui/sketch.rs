//! Plane sketch and tape tool. UI state stays in egui memory; commits are document actions.
use super::{transform_gizmo, Actions, UiAction, View};
use eframe::egui::{self, pos2, vec2, Align2, FontId, Pos2, Rect, Stroke};
use minihou_app::inference::{self as inf, InferenceEngine, InferenceInput, InferenceResult, InferenceSettings, Kind, Primitive, Projector, ToolContext, ToolSpec};
use std::sync::{Arc, Mutex};
use serde_json::{json, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SketchMode { Line, Pen, Polygon, Rectangle, Circle, Tape, Bezier, Arc }
impl SketchMode {
    /// The drawing modes of the old sketch panel, in its button order (Tape is the Measure tool).
    pub const DRAW: [Self; 5] = [Self::Line, Self::Pen, Self::Polygon, Self::Rectangle, Self::Circle];
    /// Command id that starts this mode (registered in `keymap.rs`, unbound by default like the old build).
    pub fn command_id(self) -> &'static str { match self { Self::Line => "sketch_line", Self::Pen => "sketch_polyline", Self::Polygon => "sketch_polygon", Self::Rectangle => "sketch_rectangle", Self::Circle => "sketch_circle", Self::Tape => "sketch_measure", Self::Bezier => "sketch_bezier", Self::Arc => "sketch_arc" } }
    pub fn name(self) -> &'static str { match self { Self::Line => "Line", Self::Pen => "Polyline", Self::Polygon => "Polygon", Self::Rectangle => "Rectangle", Self::Circle => "Circle", Self::Tape => "Measure", Self::Bezier => "Bezier", Self::Arc => "Arc" } }
    pub fn tool(self) -> super::Tool { match self {
        Self::Line => super::Tool::Line, Self::Pen => super::Tool::Polyline,
        Self::Polygon => super::Tool::Polygon, Self::Rectangle => super::Tool::Rectangle,
        Self::Circle => super::Tool::Circle, Self::Tape => super::Tool::Measure,
        Self::Bezier | Self::Arc => super::Tool::Polyline,
    } }
    pub fn from_tool(tool: super::Tool) -> Option<Self> {
        Self::DRAW.into_iter().chain([Self::Tape]).find(|m| m.tool() == tool)
    }
    fn short(self) -> &'static str { match self { Self::Line => "Line", Self::Pen => "Poly", Self::Polygon => "Polygon", Self::Rectangle => "Rect", Self::Circle => "Circle", Self::Tape => "Tape", Self::Bezier => "Bezier", Self::Arc => "Arc" } }
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
        SketchMode::Arc => {
            let mid = mul(add(a, *points.last().unwrap()), 0.5);
            let chord = sub(*points.last().unwrap(), a);
            let r = length(chord) * 0.5;
            if r < 1e-7 { return points.to_vec(); }
            let chord_dir = mul(chord, 1.0 / (r * 2.0));
            let (_, _, n) = plane.basis();
            let perp = [
                n[1] * chord_dir[2] - n[2] * chord_dir[1],
                n[2] * chord_dir[0] - n[0] * chord_dir[2],
                n[0] * chord_dir[1] - n[1] * chord_dir[0],
            ];
            let n_segs = segments.clamp(4, 64);
            (0..=n_segs).map(|i| {
                let angle = std::f32::consts::PI * (1.0 - i as f32 / n_segs as f32);
                add(mid, add(mul(chord_dir, -r * angle.cos()), mul(perp, r * angle.sin())))
            }).collect()
        }
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
struct State { mode: Option<SketchMode>, plane: Plane, points: Vec<[f32; 3]>, refs: Vec<Option<Value>>,
    /// Face plane under each committed point (the next segment's working plane).
    faces: Vec<Option<([f32;3],[f32;3])>>, hover: Option<[f32; 3]>, result: Option<InferenceResult>,
    closed: bool, axis: Option<usize>, numeric: String, origin: [f32; 3], alt_down: bool, alt_clean: bool, committed: usize }
fn id() -> egui::Id { egui::Id::new("sketch_state") }
/// The shared inference engine (one per viewport context; caches the scene accelerator).
pub fn engine(ctx: &egui::Context) -> Arc<Mutex<InferenceEngine>> {
    ctx.data_mut(|d| d.get_temp_mut_or_insert_with(egui::Id::new("inference_engine"), || Arc::new(Mutex::new(InferenceEngine::new()))).clone())
}
/// The app stores `Preferences::inference` here every frame.
pub fn settings_id() -> egui::Id { egui::Id::new("inference_settings") }
fn settings(ctx: &egui::Context) -> InferenceSettings { ctx.data(|d| d.get_temp(settings_id())).unwrap_or_default() }
fn spec(mode: SketchMode) -> ToolSpec { match mode {
    SketchMode::Line => ToolSpec::LINE, SketchMode::Pen | SketchMode::Bezier => ToolSpec::POLYLINE, SketchMode::Polygon => ToolSpec::POLYGON,
    SketchMode::Rectangle => ToolSpec::RECTANGLE, SketchMode::Circle | SketchMode::Arc => ToolSpec::CIRCLE, SketchMode::Tape => ToolSpec::MEASURE,
} }
fn color(c: [u8; 4]) -> egui::Color32 { egui::Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]) }
fn key_named(name: &str) -> Option<egui::Key> { egui::Key::from_name(name) }
/// Preferences > Viewport > Inference. Returns true when something changed.
pub fn inference_preferences(ui: &mut egui::Ui, s: &mut InferenceSettings) -> bool {
    let mut changed = false;
    ui.collapsing("Inference (Line / sketch tools)", |ui| {
        changed |= ui.add(egui::Slider::new(&mut s.tolerance_px, 2.0..=40.0).text("Snap tolerance (px)")).changed();
        changed |= ui.add(egui::Slider::new(&mut s.marker_size, 2.0..=24.0).text("Marker size")).changed();
        changed |= ui.add(egui::Slider::new(&mut s.dwell_s, 0.05..=3.0).text("Hover to reference (s)")).changed();
        changed |= ui.checkbox(&mut s.show_tooltips, "Show inference tooltips").changed();
        changed |= ui.add(egui::Slider::new(&mut s.tooltip_delay_s, 0.0..=3.0).text("Tooltip delay (s)")).changed();
        changed |= ui.checkbox(&mut s.include_hidden, "Snap to hidden / occluded geometry").changed();
        ui.horizontal(|ui| { ui.label("Linear inferences");
            egui::ComboBox::from_id_salt("inference_linear_mode").selected_text(s.linear_mode.label()).show_ui(ui, |ui| {
                for m in inf::LinearMode::ALL { changed |= ui.selectable_value(&mut s.linear_mode, m, m.label()).changed(); }
            });
        });
        ui.horizontal_wrapped(|ui| { for (bit, name) in inf::cat::NAMES {
            let mut on = s.enabled & bit != 0;
            if ui.checkbox(&mut on, name).changed() { s.enabled ^= bit; changed = true; }
        } });
        ui.horizontal_wrapped(|ui| { for f in inf::settings::InferenceColors::FIELDS {
            let slot = s.colors.field_mut(f).unwrap();
            let mut c = color(*slot);
            ui.label(f);
            if ui.color_edit_button_srgba(&mut c).changed() { *slot = c.to_srgba_unmultiplied(); changed = true; }
        } });
        ui.label("Tool keys (egui key names; the viewport keymap is suspended while a sketch tool is active):");
        for (label, slot) in [("Lock X", &mut s.keys.lock_x), ("Lock Y", &mut s.keys.lock_y), ("Lock Z", &mut s.keys.lock_z), ("Lock inferred", &mut s.keys.lock_inferred), ("Cycle candidates", &mut s.keys.cycle_candidate)] {
            ui.horizontal(|ui| { ui.label(label); let r = ui.text_edit_singleline(slot); if key_named(slot).is_none() { ui.label("unknown key"); } changed |= r.changed(); });
        }
        let conflicts = s.keys.conflicts();
        if !conflicts.is_empty() { ui.label(format!("Conflicting tool keys (first action wins): {}", conflicts.join(", "))); }
        if ui.button("Reset inference settings").clicked() { *s = InferenceSettings::default(); changed = true; }
    });
    changed
}
pub fn start(ctx: &egui::Context, mode: SketchMode) {
    ctx.data_mut(|d| { let mut s: State = d.get_temp(id()).unwrap_or_default(); s.mode = Some(mode); s.points.clear(); s.refs.clear(); s.faces.clear(); s.hover = None; s.numeric.clear(); s.axis = None; d.insert_temp(id(), s); });
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
        if s.points.len() == 2 { out.push(UiAction::AddMeasurement { a: s.points[0], b: s.points[1], refs: [s.refs.first().cloned().flatten(),s.refs.get(1).cloned().flatten()] }); s.mode = None; s.points.clear(); s.refs.clear(); s.faces.clear(); s.hover = None; s.numeric.clear(); s.axis = None; }
        return;
    }
    let spec_mode = if s.closed && matches!(mode,SketchMode::Line|SketchMode::Pen) { SketchMode::Polygon } else { mode };
    let Some((points, closed)) = specification(spec_mode, &s.points, s.plane) else { return; };
    out.push(UiAction::SketchCommit { name: mode.name().into(), points, closed, refs: s.refs.clone() });
    s.points.clear(); s.refs.clear(); s.faces.clear(); s.hover = None; s.numeric.clear(); s.axis = None; s.closed = false;
}
fn vec3(value: &Value) -> Option<[f32;3]> {
    let a=value.as_array()?; Some([a.first()?.as_f64()? as f32,a.get(1)?.as_f64()? as f32,a.get(2)?.as_f64()? as f32])
}
fn measurement_endpoint(row: &Value, slot: usize, doc: &Value, display: &str, positions: Option<&[f32]>) -> Option<([f32;3],bool)> {
    let fallback=vec3(row.get("points").and_then(|v|v.get(slot)).or_else(||row.get(if slot==0 {"a"} else {"b"}))?)?;
    let Some(reference)=row.get("refs").and_then(|v|v.get(slot)).filter(|v|v.is_object()) else {return Some((fallback,false));};
    let Some(node)=reference.get("node").and_then(Value::as_str) else {return Some((fallback,true));};
    let identity=reference.get("identity").and_then(Value::as_str);
    let valid=identity.is_some_and(|id|doc["nodes"][node]["measurement_identity"].as_str()==Some(id));
    let index=reference.get("point").and_then(Value::as_u64).unwrap_or(u64::MAX) as usize;
    let count=reference.get("point_count").and_then(Value::as_u64).unwrap_or(u64::MAX) as usize;
    if valid && node==display {
        if let Some(p)=positions.filter(|p|p.len()/3==count && index<count) {
            return Some(([p[index*3],p[index*3+1],p[index*3+2]],false));
        }
    }
    Some((fallback,true))
}
fn selected_id() -> egui::Id { egui::Id::new("sketch_selected_measurement") }
pub fn selected_measurement(ctx:&egui::Context) -> Option<usize> { ctx.data(|d|d.get_temp(selected_id())) }
pub fn clear_measurement_selection(ctx:&egui::Context) {ctx.data_mut(|d|d.remove::<usize>(selected_id()));}
fn measurements(ui: &mut egui::Ui, rect: Rect, v: &View, out:&mut Actions) -> bool {
    let Some(rows) = v.doc.get("measurements").and_then(|x| x.as_array()) else { return false; };
    if let Some(index) = selected_measurement(ui.ctx()) {
        if index >= rows.len() { clear_measurement_selection(ui.ctx()); }
    }
    let p = ui.painter_at(rect);
    let mut consumed=false;
    if !rows.is_empty() {
        let button=Rect::from_min_size(pos2(rect.left()+57.,rect.bottom()-88.),vec2(112.,24.));
        let response=ui.interact(button,egui::Id::new("sketch_clear_measurements"),egui::Sense::click());
        p.rect_filled(button,3.,v.theme.panel);
        p.text(button.center(),Align2::CENTER_CENTER,"Clear Measurements",FontId::monospace(10.),v.theme.text);
        if response.clicked() {out.push(UiAction::ClearMeasurements);consumed=true;}
        response.context_menu(|ui| {
            if ui.button("Clear all").clicked() {
                out.push(UiAction::ClearMeasurements);
                ui.close();
            }
        });
    }
    for (index,row) in rows.iter().enumerate() {
        let positions=v.cooking.geometry.as_ref().map(|g|g.p());
        let (Some((mut a,stale_a)), Some((mut b,stale_b))) = (measurement_endpoint(row,0,v.doc,v.display,positions),measurement_endpoint(row,1,v.doc,v.display,positions)) else { continue; };
        let cache_id=egui::Id::new(("measurement_last",row.get("id").and_then(Value::as_str).unwrap_or("legacy"),index));
        let mut last=ui.ctx().data(|d|d.get_temp::<[[f32;3];2]>(cache_id)).unwrap_or([a,b]);
        if stale_a {a=last[0];} else {last[0]=a;}
        if stale_b {b=last[1];} else {last[1]=b;}
        ui.ctx().data_mut(|d|d.insert_temp(cache_id,last));
        let (Some(pa), Some(pb)) = (transform_gizmo::project(v.camera, rect, a), transform_gizmo::project(v.camera, rect, b)) else { continue; };
        let color=if stale_a||stale_b {v.theme.dim} else {v.theme.tpl};
        p.line_segment([pa, pb], Stroke::new(1.5_f32, color));
        let label=pa.lerp(pb,0.5)+vec2(8.,-8.);
        p.text(label, Align2::LEFT_BOTTOM, format!("{}{}",crate::units::format(length(sub(b, a)) as f64, v.units),if stale_a||stale_b {" [detached]"} else {""}), FontId::monospace(12.), color);
        let target=Rect::from_min_size(label+vec2(-4.,-20.),vec2(160.,24.));
        let response=ui.interact(target,egui::Id::new(("measurement",index)),egui::Sense::click());
        if response.clicked() {ui.ctx().data_mut(|d|d.insert_temp(selected_id(),index));consumed=true;}
        response.context_menu(|ui| {
            if ui.button("Delete Measurement").clicked() {
                out.push(UiAction::DeleteMeasurement(index));ui.close();
            }
            if ui.button("Clear all").clicked() {
                out.push(UiAction::ClearMeasurements);ui.close();
            }
        });
        if selected_measurement(ui.ctx())==Some(index) {p.rect_stroke(target,2.,Stroke::new(1_f32,v.theme.accent),egui::StrokeKind::Inside);}
    }
    consumed
}
/// Paint engine primitives: markers by shape, solid/dashed guide lines, tooltips with a backdrop.
fn paint_primitives(p: &egui::Painter, prims: &[Primitive], project: impl Fn([f32; 3]) -> Option<Pos2>, backdrop: egui::Color32) {
    use inf::Marker as M;
    for prim in prims { match prim {
        Primitive::Line { a, b, color: c, width, dashed } => if let (Some(a), Some(b)) = (project(*a), project(*b)) {
            let stroke = Stroke::new(*width, color(*c));
            if *dashed { p.extend(egui::Shape::dashed_line(&[a, b], stroke, 6., 4.)); } else { p.line_segment([a, b], stroke); }
        },
        Primitive::Marker { at, shape, color: c, size } => if let Some(q) = project(*at) {
            let (c, r) = (color(*c), *size);
            let outline = Stroke::new(1.5_f32, egui::Color32::BLACK);
            match shape {
                M::Circle => { p.circle_filled(q, r, c); p.circle_stroke(q, r, outline); }
                M::Ring => { p.circle_stroke(q, r, Stroke::new(2_f32, c)); }
                M::Dot => { p.circle_filled(q, r * 0.6, c); }
                M::Square => { p.rect_filled(Rect::from_center_size(q, vec2(r * 2., r * 2.)), 0., c); }
                M::Diamond => { let pts = vec![q + vec2(0., -r), q + vec2(r, 0.), q + vec2(0., r), q + vec2(-r, 0.)]; p.add(egui::Shape::convex_polygon(pts, c, outline)); }
                M::Cross => { let s = Stroke::new(2.5_f32, c); p.line_segment([q + vec2(-r, -r), q + vec2(r, r)], s); p.line_segment([q + vec2(-r, r), q + vec2(r, -r)], s); }
            }
        },
        Primitive::Tooltip { at, text, color: c } => if let Some(q) = project(*at) {
            let galley = p.layout_no_wrap(text.clone(), FontId::proportional(12.), color(*c));
            let at = q + vec2(12., -12. - galley.size().y);
            p.rect_filled(Rect::from_min_size(at - vec2(3., 2.), galley.size() + vec2(6., 4.)), 3., backdrop);
            p.galley(at, galley, color(*c));
        },
    } }
}
pub fn show(ui: &mut egui::Ui, rect: Rect, v: &View, out: &mut Actions,
    mesh: Option<(&[f32], &[u32])>) {
    let measurement_click=measurements(ui, rect, v, out);
    let ctx = ui.ctx().clone();
    if let Some(index) = selected_measurement(&ctx) {
        if ctx.input(|i| i.key_pressed(egui::Key::Delete)) {
            out.push(UiAction::DeleteMeasurement(index));
            clear_measurement_selection(&ctx);
        }
    }
    let mut s: State = ctx.data_mut(|d| d.get_temp(id())).unwrap_or_default();
    let tool_mode = SketchMode::from_tool(v.editor.tool);
    if s.mode != tool_mode { s = State { mode: tool_mode, plane: s.plane, ..Default::default() }; }
    let Some(mode) = s.mode else { return; };
    let pointer = ctx.input(|i| i.pointer.hover_pos());
    let settings = settings(&ctx);
    let engine_cell = engine(&ctx);
    let mut engine = engine_cell.lock().unwrap_or_else(|e| e.into_inner());
    if s.committed != s.points.len() { if s.points.len() > s.committed { engine.segment_committed(); } s.committed = s.points.len(); }
    let safe = Rect::from_min_max(pos2(rect.left() + 55., rect.top() + 83.), pos2(rect.right() - 55., rect.bottom() - 50.));
    let projector = Projector { matrix: v.camera.projection, rect_min: [rect.min.x, rect.min.y], rect_size: [rect.width(), rect.height()] };
    let typed = inf::constraints::parse_typed(&s.numeric, v.units);
    if let Some(at) = pointer.filter(|p| safe.contains(*p)) {
        // Ray from the same matrix the overlay projects with (exact inverse of `project`).
        let ray = projector.ray([at.x, at.y]).map(|(o, d)| {
            if inf::math::dot(d, v.camera.ray_forward) < 0. { (o, inf::math::mul(d, -1.)) } else { (o, d) }
        }).unwrap_or_else(|| transform_gizmo::ray(v.camera, rect, at));
        let (_, _, normal) = s.plane.basis();
        let line_like = matches!(mode, SketchMode::Line | SketchMode::Pen | SketchMode::Polygon | SketchMode::Tape);
        let tool = ToolContext { spec: spec(mode), start: s.points.last().copied(),
            start_face: if line_like { s.faces.last().copied().flatten() } else { None },
            ground: (s.origin, normal), tool_points: s.points.clone() };
        let input = InferenceInput { ray, cursor: [at.x, at.y], projector, time: ctx.input(|i| i.time),
            geometry: mesh.map(|(positions, triangles)| inf::GeometryRef { positions, triangles, revision: 0 }),
            tool, settings: &settings, shift_held: ctx.input(|i| i.modifiers.shift), typed };
        let res = engine.update(&input);
        let ctrl = ctx.input(|i| i.modifiers.ctrl);
        s.hover = res.point.map(|p| if res.winner.kind == Kind::Free && typed.is_none() {
            grid_snap(p, s.plane, if v.editor.snap.enabled != ctrl { v.editor.snap.increment } else { 0. })
        } else { p });
        s.result = Some(res);
    } else { s.hover = None; s.result = None; }
    // Alt tap (press and release with nothing else) cycles the linear inference mode.
    let alt = ctx.input(|i| i.modifiers.alt);
    if alt && !s.alt_down { s.alt_down = true; s.alt_clean = true; }
    let keys = settings.keys.clone();
    let events = ctx.input(|i| i.events.clone());
    if s.alt_down && events.iter().any(|e| matches!(e, egui::Event::Key { .. } | egui::Event::PointerButton { .. })) { s.alt_clean = false; }
    if !alt && s.alt_down { s.alt_down = false; if s.alt_clean { engine.cycle_linear_mode(&settings); } }
    let bound = |key: egui::Key, name: &str| key_named(name) == Some(key);
    for event in events { match event {
        egui::Event::Key {key:egui::Key::Delete,pressed:true,..} if selected_measurement(&ctx).is_some() => {
            out.push(UiAction::DeleteMeasurement(selected_measurement(&ctx).unwrap()));
            clear_measurement_selection(&ctx);
        },
        // Esc state machine: typed value, then temporary locks/references, then the stroke, then the tool.
        egui::Event::Key { key: egui::Key::Escape, pressed: true, .. } => {
            if !s.numeric.is_empty() { s.numeric.clear(); }
            else if engine.escape() {}
            else if !s.points.is_empty() && mode != SketchMode::Tape { s.points.clear(); s.refs.clear(); s.faces.clear(); s.closed = false; }
            else { clear_measurement_selection(&ctx); s = State { plane: s.plane, ..Default::default() }; out.push(UiAction::Editor(super::EditorChange::Tool(super::Tool::Select))); break; }
        },
        egui::Event::PointerButton { button: egui::PointerButton::Secondary, pressed: true, .. } => {
            engine.escape();
            clear_measurement_selection(&ctx); s = State { plane: s.plane, ..Default::default() }; out.push(UiAction::Editor(super::EditorChange::Tool(super::Tool::Select))); break;
        },
        egui::Event::Key { key: egui::Key::Enter, pressed: true, .. } => { if !s.numeric.is_empty() { if let Some(p) = s.hover { s.points.push(p); s.refs.push(None); s.faces.push(s.result.as_ref().and_then(|r| r.face)); } s.numeric.clear(); } finish(&mut s, out); },
        egui::Event::Key { key: egui::Key::Backspace, pressed: true, .. } => { if s.numeric.is_empty() { s.points.pop(); s.refs.pop(); s.faces.pop(); } else { s.numeric.pop(); } },
        egui::Event::Key { key, pressed: true, .. } if bound(key, &keys.lock_x) => engine.toggle_axis_lock(0),
        egui::Event::Key { key, pressed: true, .. } if bound(key, &keys.lock_y) => engine.toggle_axis_lock(1),
        egui::Event::Key { key, pressed: true, .. } if bound(key, &keys.lock_z) => engine.toggle_axis_lock(2),
        egui::Event::Key { key, pressed: true, .. } if bound(key, &keys.lock_inferred) => engine.lock_inferred(),
        egui::Event::Key { key, pressed: true, .. } if bound(key, &keys.cycle_candidate) => engine.cycle_candidates(),
        egui::Event::Key { key, pressed: true, .. } if matches!(key, egui::Key::X | egui::Key::Y | egui::Key::Z) => { engine.toggle_axis_lock(match key { egui::Key::X => 0, egui::Key::Y => 1, _ => 2 }); },
        egui::Event::Text(t) if t.chars().all(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | ',' | '@' | '<' | 'm'|'c'|'k'|'i'|'n'|'f'|'t'|'M'|'C'|'K'|'I'|'N'|'F'|'T'|' ')) => s.numeric.push_str(&t),
        egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers } if safe.contains(pos) && !modifiers.alt && !measurement_click => {
            if let Some(p) = s.hover { if s.points.last().is_none_or(|a| length(sub(*a, p)) > 1e-7) {
                if matches!(mode,SketchMode::Line|SketchMode::Pen|SketchMode::Polygon) && s.points.len() >= 3 && length(sub(s.points[0], p)) < v.bounds.1 * 0.015 {
                    s.closed = true; finish(&mut s, out);
                } else { if s.points.is_empty() { s.origin = p; } s.points.push(p);
                    let res = s.result.as_ref();
                    let count = mesh.map_or(0, |(positions, _)| positions.len() / 3);
                    // Static snap: the numeric point is stored; the descriptor is optional live-reference metadata.
                    let reference = res.and_then(|r| r.descriptor.as_ref().filter(|_| mesh.is_some()).map(|d| (r, d))).and_then(|(r, d)| match (mode, d) {
                        (SketchMode::Tape, inf::RefDescriptor::Vertex { point }) if r.winner.kind == Kind::Endpoint && (*point as usize) < count =>
                            Some(json!({"node":v.display,"point":point,"point_count":count})),
                        (SketchMode::Tape, _) => None,
                        (_, d) => Some(d.to_json(v.display, count)),
                    });
                    s.faces.push(res.and_then(|r| r.face));
                    s.refs.push(reference); s.numeric.clear(); if matches!(mode, SketchMode::Rectangle | SketchMode::Circle | SketchMode::Tape | SketchMode::Arc) && s.points.len() == 2 { finish(&mut s, out); } }
            } } },
        _ => {}
    } }
    if s.committed != s.points.len() { if s.points.len() > s.committed { engine.segment_committed(); } s.committed = s.points.len(); }
    s.axis = match engine.active_lock() { Some(inf::Lock::Axis(a)) => Some(a), _ => None };
    let linear_mode = engine.linear_mode(&settings);
    let lock_label = engine.active_lock().map(|l| l.label());
    drop(engine);
    let p = ui.painter_at(rect);
    let mut preview = s.points.clone(); if let Some(h) = s.hover { preview.push(h); }
    let preview = shape(mode, &preview, s.plane, 32);
    let screen: Vec<Pos2> = preview.iter().filter_map(|q| transform_gizmo::project(v.camera, rect, *q)).collect();
    for pair in screen.windows(2) { p.line_segment([pair[0], pair[1]], Stroke::new(2_f32, v.theme.warn)); }
    if matches!(mode, SketchMode::Polygon | SketchMode::Rectangle | SketchMode::Circle) && screen.len() >= 3 { p.line_segment([*screen.last().unwrap(), screen[0]], Stroke::new(2_f32, v.theme.warn)); }
    for q in &screen { p.circle_filled(*q, 3., v.theme.warn); }
    if let Some(res) = &s.result { paint_primitives(&p, &res.primitives, |q| transform_gizmo::project(v.camera, rect, q), v.theme.panel); }
    if mode == SketchMode::Tape && screen.len() == 2 {
        if let (Some(&a), Some(h)) = (s.points.first(), s.hover) {
            p.text(screen[0].lerp(screen[1], 0.5) + vec2(8., -8.), Align2::LEFT_BOTTOM, crate::units::format(length(sub(h, a)) as f64, v.units), FontId::monospace(12.), v.theme.warn);
        }
    }
    let chip = Rect::from_min_size(pos2(rect.left() + 56., rect.top() + 48.), vec2(265., 28.));
    p.rect_filled(chip, 2., v.theme.panel); p.rect_stroke(chip, 2., Stroke::new(1_f32, v.theme.stroke), egui::StrokeKind::Inside);
    p.text(chip.left_center() + vec2(8., 0.), Align2::LEFT_CENTER, format!("{} | {} | {}", mode.name(), lock_label.as_deref().unwrap_or("free"), if s.numeric.is_empty() { "type length" } else { &s.numeric }), FontId::monospace(10.), v.theme.text);
    ui.interact(chip, egui::Id::new("sketch_help"), egui::Sense::hover()).on_hover_text("Click points; Enter finishes Polyline/Polygon. Type a length, x,y,z, @dx,dy,dz or len<deg (units supported). Right/Left/Up arrows (or X/Y/Z) lock an axis, Down locks parallel/perpendicular, hold Shift to lock the current inference, Tab cycles candidates, tap Alt for linear modes. Hover a point ~0.4 s to track it. Ctrl toggles grid snapping. Esc clears locks/references, then the stroke, then returns to Select.");
    // Status line: what a click will do, typed-value conflicts, working plane.
    if let Some(res) = &s.result {
        let mut status = res.reason.clone();
        if let Some(t) = &res.typed_status { status = format!("{status} | {t}"); }
        if res.winner.kind == Kind::Free || res.plane.source == inf::PlaneSource::Face { status = format!("{status} | {}", res.plane.source.label()); }
        let at = chip.left_bottom() + vec2(0., 4.);
        let text_color = if res.typed_status.is_some() { v.theme.warn } else { v.theme.text };
        let galley = p.layout_no_wrap(status, FontId::monospace(10.), text_color);
        p.rect_filled(Rect::from_min_size(at, galley.size() + vec2(12., 6.)), 2., v.theme.panel);
        p.galley(at + vec2(6., 3.), galley, text_color);
    }
    for (i, plane) in [Plane::XY, Plane::XZ, Plane::YZ].into_iter().enumerate() {
        let r = Rect::from_min_size(pos2(chip.right() + 4. + i as f32 * 31., chip.top()), vec2(29., 28.));
        let resp = ui.interact(r, egui::Id::new(("sketch_plane", i)), egui::Sense::click());
        p.rect_filled(r, 2., if s.plane == plane { v.theme.accent_bg } else { v.theme.panel });
        p.text(r.center(), Align2::CENTER_CENTER, plane.label(), FontId::monospace(10.), if s.plane == plane { v.theme.accent } else { v.theme.dim });
        if resp.clicked() { s.plane = plane; s.points.clear(); s.faces.clear(); s.origin = [0.; 3]; }
    }
    let x0 = chip.right() + 4. + 3. * 31. + 6.;
    for (axis, label) in ["X", "Y", "Z"].into_iter().enumerate() {
        let r = Rect::from_min_size(pos2(x0 + axis as f32 * 29., chip.top()), vec2(27.,28.));
        let resp = ui.interact(r, egui::Id::new(("sketch_axis",axis)), egui::Sense::click()).on_hover_text("Lock drawing to this axis (X/Y/Z or Right/Left/Up arrow)");
        p.rect_filled(r,2.,if s.axis == Some(axis) { v.theme.accent_bg } else { v.theme.panel });
        p.text(r.center(),Align2::CENTER_CENTER,label,FontId::monospace(10.),if s.axis == Some(axis) { v.theme.accent } else { v.theme.dim });
        if resp.clicked() { engine_cell.lock().unwrap_or_else(|x| x.into_inner()).toggle_axis_lock(axis); }
    }
    let r = Rect::from_min_size(pos2(x0 + 3. * 29. + 6., chip.top()), vec2(46., 28.));
    let resp = ui.interact(r, egui::Id::new("sketch_linear_mode"), egui::Sense::click()).on_hover_text(format!("Linear inferences: {} (click or tap Alt to cycle)", linear_mode.label()));
    p.rect_filled(r, 2., if linear_mode == inf::LinearMode::All { v.theme.panel } else { v.theme.accent_bg });
    p.text(r.center(), Align2::CENTER_CENTER, match linear_mode { inf::LinearMode::All => "All", inf::LinearMode::PointsOnly => "Pts", inf::LinearMode::ParallelPerpendicularOnly => "//|_" }, FontId::monospace(10.), v.theme.dim);
    if resp.clicked() { engine_cell.lock().unwrap_or_else(|x| x.into_inner()).cycle_linear_mode(&settings); }
    ctx.data_mut(|d| d.insert_temp(id(), s));
}

#[cfg(test)] mod tests {
    use super::*;
    // ---- inference routing through the real sketch input path (shared engine in ctx memory)
    const VP: Rect = Rect { min: Pos2::ZERO, max: Pos2 { x: 1280., y: 800. } };
    fn rig(mode: SketchMode) -> (egui::Context, crate::ui::testing::Fixture) {
        let mut f = crate::ui::testing::Fixture::new();
        f.editor.tool = mode.tool();
        f.camera.projection = inf::math::view_projection([4., -9., 7.], [1.5, 1., 0.], [0., 0., 1.], 0.8, None);
        (egui::Context::default(), f)
    }
    fn key(k: egui::Key) -> egui::Event { egui::Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() } }
    fn frame(ctx: &egui::Context, f: &crate::ui::testing::Fixture, events: Vec<egui::Event>, mesh: Option<(&[f32], &[u32])>, modifiers: egui::Modifiers) -> Actions {
        let mut out = Actions::default();
        let v = f.view();
        let _ = ctx.run(egui::RawInput { screen_rect: Some(VP), events, modifiers, ..Default::default() }, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| show(ui, VP, &v, &mut out, mesh));
        });
        out
    }
    fn scr(f: &crate::ui::testing::Fixture, p: [f32; 3]) -> Pos2 { transform_gizmo::project(f.camera, VP, p).unwrap() }
    fn click(ctx: &egui::Context, f: &crate::ui::testing::Fixture, at: Pos2, mesh: Option<(&[f32], &[u32])>) -> Actions {
        let mut out = frame(ctx, f, vec![egui::Event::PointerMoved(at), egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() }], mesh, Default::default());
        out.0.extend(frame(ctx, f, vec![egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() }], mesh, Default::default()).0);
        out
    }
    fn state(ctx: &egui::Context) -> State { ctx.data(|d| d.get_temp(id())).unwrap() }

    #[test] fn arrow_lock_and_typed_length_commit_exact_line() {
        let (ctx, f) = rig(SketchMode::Line);
        click(&ctx, &f, scr(&f, [1., 1., 0.]), None);
        let a = state(&ctx).points[0];
        frame(&ctx, &f, vec![key(egui::Key::ArrowRight)], None, Default::default());
        let m = frame(&ctx, &f, vec![egui::Event::PointerMoved(scr(&f, [3., 2.2, 0.])), egui::Event::Text("2.5m".into())], None, Default::default());
        assert!(m.0.is_empty(), "hovering/typing must not emit document actions");
        frame(&ctx, &f, vec![], None, Default::default());
        let s = state(&ctx);
        assert_eq!(s.axis, Some(0));
        let h = s.hover.unwrap();
        assert!(length(sub(h, add(a, [2.5, 0., 0.]))) < 1e-5, "{h:?} vs {a:?}");
        let out = frame(&ctx, &f, vec![key(egui::Key::Enter)], None, Default::default());
        assert!(matches!(&out.0[..], [UiAction::SketchCommit { points, .. }] if points.len() == 2 && points[1][1] == a[1] && points[1][2] == a[2]), "{:?}", out.0.len());
        // Up arrow locks Z (3D, not limited to the sketch plane).
        let (ctx, f) = rig(SketchMode::Line);
        click(&ctx, &f, scr(&f, [1., 1., 0.]), None);
        let a = state(&ctx).points[0];
        frame(&ctx, &f, vec![key(egui::Key::ArrowUp), egui::Event::PointerMoved(scr(&f, [1.3, 1.2, 1.5]))], None, Default::default());
        frame(&ctx, &f, vec![], None, Default::default());
        let h = state(&ctx).hover.unwrap();
        assert!(h[0] == a[0] && h[1] == a[1] && h[2] > 0.5, "{h:?}");
    }

    #[test] fn escape_clears_locks_then_stroke_then_tool() {
        let (ctx, f) = rig(SketchMode::Pen);
        click(&ctx, &f, scr(&f, [1., 1., 0.]), None);
        frame(&ctx, &f, vec![key(egui::Key::ArrowRight)], None, Default::default());
        let select = |o: &Actions| o.0.iter().any(|a| matches!(a, UiAction::Editor(super::super::EditorChange::Tool(super::super::Tool::Select))));
        let o = frame(&ctx, &f, vec![key(egui::Key::Escape)], None, Default::default());
        assert!(!select(&o) && state(&ctx).axis.is_none() && state(&ctx).points.len() == 1);
        let o = frame(&ctx, &f, vec![key(egui::Key::Escape)], None, Default::default());
        assert!(!select(&o) && state(&ctx).points.is_empty());
        assert!(select(&frame(&ctx, &f, vec![key(egui::Key::Escape)], None, Default::default())));
    }

    #[test] fn alt_tap_cycles_linear_mode_and_tab_cycles() {
        let (ctx, f) = rig(SketchMode::Line);
        frame(&ctx, &f, vec![], None, egui::Modifiers::ALT);
        frame(&ctx, &f, vec![], None, Default::default());
        let mode = engine(&ctx).lock().unwrap().linear_mode(&InferenceSettings::default());
        assert_eq!(mode, inf::LinearMode::PointsOnly);
        // Alt used with a click (Alt+LMB selection) does not cycle.
        frame(&ctx, &f, vec![], None, egui::Modifiers::ALT);
        frame(&ctx, &f, vec![egui::Event::PointerButton { pos: pos2(600., 400.), button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::ALT }], None, egui::Modifiers::ALT);
        frame(&ctx, &f, vec![], None, Default::default());
        assert_eq!(engine(&ctx).lock().unwrap().linear_mode(&InferenceSettings::default()), inf::LinearMode::PointsOnly);
    }

    #[test] fn mesh_snaps_record_reference_descriptors() {
        // Migrated from sketch_inference: endpoint priority within the radius, nothing far away.
        let positions = [0., 0., 0., 2., 0., 0., 0., 2., 0.5];
        let tris = [0u32, 1, 2];
        let mesh = Some((&positions[..], &tris[..]));
        let (ctx, f) = rig(SketchMode::Line);
        let at = scr(&f, [2., 0., 0.]) + vec2(3., -2.);
        frame(&ctx, &f, vec![egui::Event::PointerMoved(at)], mesh, Default::default());
        let r = state(&ctx).result.unwrap();
        assert_eq!(r.winner.kind, Kind::Endpoint);
        assert_eq!(state(&ctx).hover, Some([2., 0., 0.]));
        frame(&ctx, &f, vec![egui::Event::PointerMoved(scr(&f, [6., 5., 0.]))], mesh, Default::default());
        assert!(!matches!(state(&ctx).result.unwrap().winner.kind, Kind::Endpoint | Kind::Midpoint | Kind::OnEdge));
        click(&ctx, &f, at, mesh);
        let mid = scr(&f, [1., 1., 0.25]) + vec2(2., 2.);
        click(&ctx, &f, mid, mesh);
        let out = frame(&ctx, &f, vec![key(egui::Key::Enter)], mesh, Default::default());
        let Some(UiAction::SketchCommit { refs, points, .. }) = out.0.first() else { panic!("no commit") };
        assert_eq!(points[1], [1., 1., 0.25]);
        assert_eq!(refs[0].as_ref().unwrap()["kind"], "vertex");
        assert_eq!(refs[0].as_ref().unwrap()["element"], json!([1]));
        assert_eq!(refs[1].as_ref().unwrap()["kind"], "edge");
        // Measure keeps its anchor format (node / point / point_count).
        let (ctx, mut f) = rig(SketchMode::Tape);
        f.editor.tool = super::super::Tool::Measure;
        click(&ctx, &f, at, mesh);
        assert_eq!(state(&ctx).refs[0], Some(json!({"node": f.display, "point": 1, "point_count": 3})));
    }
    #[test] fn guide_inference_table_matches_default_tool_keys() {
        // Line endings differ between checkouts (CRLF on Windows): normalise before comparing.
        let guide = include_str!("../../docs/NATIVE_EDITOR.md").replace("\r\n", "\n");
        let table = guide.split_once("<!-- inference:start -->\n").unwrap().1.split_once("<!-- inference:end -->").unwrap().0;
        let mut rows = String::from("| Input | While a sketch tool (Line, Polyline, Polygon, Rectangle, Circle, Measure) is active |\n| --- | --- |\n");
        for (k, a) in inf::settings::InferenceKeys::default().table_rows() { rows.push_str(&format!("| {k} | {a} |\n")); }
        assert_eq!(table, rows);
        for name in [&InferenceSettings::default().keys.lock_x, &InferenceSettings::default().keys.cycle_candidate] { assert!(key_named(name).is_some()); }
    }
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
        assert!(matches!(&out.0[..], [UiAction::AddMeasurement { a, b, .. }] if *a == [0.; 3] && *b == [0., 3., 0.]));
    }
    #[test] fn closing_a_line_emits_a_face() {
        let mut line=State {mode:Some(SketchMode::Line),closed:true,
            points:vec![[0.,0.,0.],[2.,0.,0.],[0.,2.,0.]],..Default::default()};
        let mut out=Actions::default(); finish(&mut line,&mut out);
        assert!(matches!(&out.0[..],[UiAction::SketchCommit {closed:true,points,..}] if points.len()==3));
    }
    #[test] fn every_sketch_mode_emits_the_curve_it_draws() {
        for (mode, source, expected_len, closed) in [
            (SketchMode::Line, vec![[0.;3], [2.,0.,0.]], 2, false),
            (SketchMode::Pen, vec![[0.;3], [2.,0.,0.], [2.,2.,0.]], 3, false),
            (SketchMode::Polygon, vec![[0.;3], [2.,0.,0.], [2.,2.,0.]], 3, true),
            (SketchMode::Rectangle, vec![[0.;3], [2.,2.,0.]], 4, true),
            (SketchMode::Circle, vec![[0.;3], [2.,0.,0.]], 32, true),
            (SketchMode::Arc, vec![[0.;3], [2.,0.,0.]], 33, false),
            (SketchMode::Bezier, vec![[0.;3], [1.,1.,0.], [2.,0.,0.]], 3, false),
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
    #[test] fn measurement_anchor_tracks_point_and_detaches_on_topology_change() {
        let doc=json!({"nodes":{"box1":{"measurement_identity":"stable"}}});
        let row=json!({"points":[[1.,2.,3.],[0.,0.,0.]],"refs":[{"node":"box1","identity":"stable","point":1,"point_count":2},null]});
        assert_eq!(measurement_endpoint(&row,0,&doc,"box1",Some(&[0.,0.,0.,4.,5.,6.])),Some(([4.,5.,6.],false)));
        assert_eq!(measurement_endpoint(&row,0,&doc,"box1",Some(&[0.,0.,0.])),Some(([1.,2.,3.],true)));
    }
    #[test] fn measurement_selection_and_delete_key_remove_measurement() {
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
        let viewport = Rect::from_min_size(Pos2::ZERO, vec2(1280., 800.));
        let v = f.view();
        let frame = |events: Vec<egui::Event>| {
            let mut out = Actions::default();
            let _ = ctx.run(egui::RawInput { screen_rect: Some(viewport), events, ..Default::default() }, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    show(ui, viewport, &v, &mut out, None);
                });
            });
            out
        };
        let click = |p: Pos2| {
            frame(vec![egui::Event::PointerMoved(p), egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() }]);
            frame(vec![egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() }])
        };

        frame(vec![]);
        assert_eq!(selected_measurement(&ctx), None);

        // Click the measurement label target: (a+b)/2 = (840, 400), label is at (848, 392), target is (844..1004, 372..396).
        click(pos2(924., 384.));
        assert_eq!(selected_measurement(&ctx), Some(0));

        // Pressing Delete key removes the selected measurement
        let out = frame(vec![egui::Event::Key {
            key: egui::Key::Delete,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }]);
        assert!(matches!(out.0.as_slice(), [UiAction::DeleteMeasurement(0)]));
        assert_eq!(selected_measurement(&ctx), None);
    }
    #[test] fn measurement_clear_button_emits_clear_action() {
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
        let viewport = Rect::from_min_size(Pos2::ZERO, vec2(1280., 800.));
        let v = f.view();
        let frame = |events: Vec<egui::Event>| {
            let mut out = Actions::default();
            let _ = ctx.run(egui::RawInput { screen_rect: Some(viewport), events, ..Default::default() }, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    show(ui, viewport, &v, &mut out, None);
                });
            });
            out
        };
        let click = |p: Pos2| {
            frame(vec![egui::Event::PointerMoved(p), egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() }]);
            frame(vec![egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() }])
        };

        frame(vec![]);
        // Clear button center is (113, 724)
        let out = click(pos2(113., 724.));
        assert!(matches!(out.0.as_slice(), [UiAction::ClearMeasurements]));
    }
    #[test] fn measurement_stale_selection_is_cleared() {
        let mut f = crate::ui::testing::Fixture::new();
        let mut doc = f.doc.value().clone();
        doc["measurements"] = serde_json::json!([
            { "a": [0.0, 0.0, 0.0], "b": [1.0, 0.0, 0.0] }
        ]);
        f.doc = minihou_app::Document::parse(doc.to_string()).unwrap();
        f.graph = minihou_core::Graph::from_value(f.doc.value()).unwrap();
        let ctx = egui::Context::default();
        f.theme.apply(&ctx);
        // Simulate selection pointing to out-of-bounds index 5
        ctx.data_mut(|d| d.insert_temp(selected_id(), 5usize));
        assert_eq!(selected_measurement(&ctx), Some(5));

        let mut out = Actions::default();
        let v = f.view();
        let _ = ctx.run(egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1280., 800.))), ..Default::default() }, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                show(ui, ui.max_rect(), &v, &mut out, None);
            });
        });
        assert_eq!(selected_measurement(&ctx), None);
    }
}
