//! Viewport cut tools. Hover/preview is UI-only; confirmation is one document action.
//! Loop Cut uses the core ring walk for hover and a cook-backed preview for slide.
//! RMB/Esc during slide keeps centred cuts; during hover they cancel the operation.
use super::{transform_gizmo, Actions, CameraInfo, EditorChange, Tool, UiAction, View};
use crate::picking;
use eframe::egui::{self, pos2, vec2, FontId, Pos2, Rect, Stroke, Vec2};
use minihou_core::{nodes::{edge_slide_preview, loop_cut_preview, loop_slide_fractions, LoopTopology, LOOP_CUT_MAX}, Geometry};
use serde_json::{json, Value};
use minihou_app::inference::{self as inf, InferenceInput, Projector, ToolContext, ToolSpec};
use std::sync::Arc;

type Edge = (usize, usize);
/// Hover radius in pixels (preview.js `edgeAt` used 14).
const HOVER_RADIUS: f32 = 14.;
/// Nearest candidates tested for occlusion, front-most visible wins (preview.js `pickEdge` tested 8).
const HOVER_CANDIDATES: usize = 8;
/// Pointer travel that turns a held slide click into a drag that commits on release.
const DRAG_THRESHOLD: f32 = 4.;

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 { (0..3).map(|k| a[k] * b[k]).sum() }
fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] { [0, 1, 2].map(|k| a[k] + (b[k] - a[k]) * t) }
fn edge_world_fraction(camera: CameraInfo, a: [f32; 3], b: [f32; 3], screen_fraction: f32) -> f32 {
    if camera.ortho { return screen_fraction; }
    let w = |p: [f32; 3]| camera.projection[3][3] + (0..3).map(|k| camera.projection[k][3] * p[k]).sum::<f32>();
    let (wa, wb) = (w(a), w(b));
    let a_weight = (1. - screen_fraction) / wa.max(1e-6);
    let b_weight = screen_fraction / wb.max(1e-6);
    b_weight / (a_weight + b_weight).max(1e-6)
}

#[derive(Clone, Default)]
struct Mesh {
    geometry: Geometry,
    positions: Vec<f32>,
    /// Fan triangulation of closed faces, used only to test whether a hovered edge is hidden.
    tris: Vec<u32>,
    loops: LoopTopology,
    edges: Vec<Edge>,
    scale: f32,
}
impl Mesh {
    fn new(g: &Geometry) -> Self {
        let positions = g.p().to_vec();
        let mut tris = Vec::new();
        for (face, r) in g.topology.prim_offsets.windows(2).enumerate() {
            if !g.topology.is_closed(face) || r[1] - r[0] < 3 { continue; }
            let vp = &g.topology.vertex_point[r[0] as usize..r[1] as usize];
            for i in 1..vp.len() - 1 { tris.extend([vp[0] as u32, vp[i] as u32, vp[i + 1] as u32]); }
        }
        let loops = LoopTopology::new(g);
        let edges = loops.edges().collect();
        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        for q in positions.chunks_exact(3) { for k in 0..3 { lo[k] = lo[k].min(q[k]); hi[k] = hi[k].max(q[k]); } }
        let scale = if positions.is_empty() { 1. } else { (0..3).map(|k| (hi[k] - lo[k]).powi(2)).sum::<f32>().sqrt().max(1e-6) };
        Self { geometry:g.clone(), positions, tris, loops, edges, scale }
    }
    fn point(&self, i: usize) -> [f32; 3] { [self.positions[3 * i], self.positions[3 * i + 1], self.positions[3 * i + 2]] }
    /// Nothing displayed lies in front of `world` along the view ray through it (perspective and ortho alike).
    fn visible(&self, camera: CameraInfo, rect: Rect, world: [f32; 3]) -> bool {
        let Some(screen) = transform_gizmo::project(camera, rect, world) else { return false; };
        let (o, d) = transform_gizmo::ray(camera, rect, screen);
        let dd = dot(d, d).max(1e-12);
        let depth = dot([0, 1, 2].map(|k| world[k] - o[k]), d) / dd;
        let eps = self.scale * 1e-3 / dd.sqrt();
        picking::pick_triangle(&self.positions, &self.tris, o, d).is_none_or(|h| h.t >= depth - eps)
    }
    /// The edge under the pointer as preview.js `pickEdge` chose it: the nearest within the radius whose closest point
    /// is not hidden (unless X-ray), among the eight nearest. Only edges that have a quad strip to cut qualify.
    fn hover(&self, camera: CameraInfo, rect: Rect, pointer: Pos2, xray: bool) -> Option<Edge> {
        let mut cand = Vec::new();
        for &(a, b) in &self.edges {
            let (Some(pa), Some(pb)) = (transform_gizmo::project(camera, rect, self.point(a)), transform_gizmo::project(camera, rect, self.point(b))) else { continue; };
            let ab = pb - pa;
            let t = ((pointer - pa).dot(ab) / ab.length_sq().max(1e-8)).clamp(0., 1.);
            let d = pointer.distance(pa + ab * t);
            if d <= HOVER_RADIUS { cand.push((d, (a, b), t)); }
        }
        cand.sort_by(|x, y| x.0.total_cmp(&y.0));
        cand.into_iter().take(HOVER_CANDIDATES).find(|&(_, (a, b), t)| {
            let (pa, pb) = (self.point(a), self.point(b));
            let world_t = edge_world_fraction(camera, pa, pb, t);
            (xray || self.visible(camera, rect, lerp3(pa, pb, world_t))) && !self.loops.ring([(a, b)]).quads.is_empty()
        }).map(|c| c.1)
    }
    /// World segments of the new edges, computed by the node's own ring walk and fractions.
    fn preview(&self, seed: Edge, count: usize, factor: f64) -> Vec<([f32; 3], [f32; 3])> {
        self.loops.ring([seed]).cut_lines(&self.positions, &loop_slide_fractions(count, 0.5, factor, true))
    }
}
pub(crate) fn object_geometry(g:&Geometry,range:&std::ops::Range<u32>)->Option<Geometry> {
    if range.end as usize>g.npoints || range.start>=range.end {return None;}
    let mut vertices=Vec::new();let mut offsets=vec![0];let mut closed=Vec::new();
    for (face,bounds) in g.topology.prim_offsets.windows(2).enumerate() {
        let ids=&g.topology.vertex_point[bounds[0] as usize..bounds[1] as usize];
        if ids.iter().all(|&id|id>=range.start as i32&&id<range.end as i32) {
            vertices.extend(ids.iter().map(|id|*id-range.start as i32));
            offsets.push(vertices.len() as i32);closed.push(g.topology.is_closed(face));
        }
    }
    Geometry::from_arrays(g.p()[range.start as usize*3..range.end as usize*3].to_vec(),vertices,offsets,Some(closed)).ok()
}

/// Count after `steps` wheel notches or PageUp/PageDown presses, clamped to polysplit's 1..64.
fn step_count(count: usize, steps: i32) -> usize { (count as i64 + steps as i64).clamp(1, LOOP_CUT_MAX as i64) as usize }
/// Wheel notches in one event: lines count as notches, points as 50 per notch, pages as 3.
fn wheel_notches(unit: egui::MouseWheelUnit, delta: f32) -> f32 {
    match unit { egui::MouseWheelUnit::Line => delta, egui::MouseWheelUnit::Point => delta / 50., egui::MouseWheelUnit::Page => delta * 3. }
}
/// Advance the raw slide (-1..1, from the centre toward either rail) by a pointer delta measured along the
/// seed edge's projection, so the loop follows the cursor. Shift is 10x finer. A seed edge seen end-on (shorter than
/// 4 px) falls back to loop-tool.js's horizontal mapping (100 px per unit).
fn slide_step(raw: f64, delta: Vec2, a: Pos2, b: Pos2, fine: bool) -> f64 {
    let ab = b - a;
    let len = ab.length();
    let (along, span) = if len >= 4. { (delta.dot(ab) / len, len.max(20.)) } else { (delta.x, 100.) };
    (raw + 2. * along as f64 / span as f64 * if fine { 0.1 } else { 1. }).clamp(-1., 1.)
}
/// Project a small positive cooked slide to find the side chosen by topology winding.
fn slide_screen_motion(v: &super::View, rect: Rect, s: &State, edge: Edge) -> Option<Vec2> {
    let g = v.cooking.geometry.as_deref()?;
    if s.tool == Some(Tool::EdgeSlide) {
        let moved=edge_slide_preview(g,&s.edge_selection,0.1,s.even,s.flipped,s.clamp,true);
        let p=|geo:&Geometry, id:usize| [0,1,2].map(|k|geo.p()[id*3+k]);
        transform_gizmo::project(v.camera,rect,p(&moved,edge.0))
            .zip(transform_gizmo::project(v.camera,rect,p(g,edge.0)))
            .map(|(after,before)|after-before)
    } else {
        let before=loop_cut_preview(g,edge,s.count,0.,s.even,s.flipped,s.clamp).ok();
        let after=loop_cut_preview(g,edge,s.count,0.1,s.even,s.flipped,s.clamp).ok();
        before.as_ref().and_then(|lines|lines.first()).zip(after.as_ref().and_then(|lines|lines.first()))
            .and_then(|(a,b)|transform_gizmo::project(v.camera,rect,a.0)
                .zip(transform_gizmo::project(v.camera,rect,b.0)))
            .map(|(before,after)|after-before)
    }
}
/// The Blender-style signed slide factor. Ctrl snaps to tenths.
fn slide_factor(raw: f64, snap: bool) -> f64 {
    let x = if snap { (raw * 10.).round() / 10. } else { raw };
    (x.clamp(-1., 1.) * 1e4).round() / 1e4
}
/// Whole numbers choose the number of cuts; signed and decimal numbers slide the cuts.
fn numeric_slide(s: &mut State) {
    if s.numeric.is_empty() { return; }
    if s.tool == Some(Tool::LoopCut) && !s.numeric.chars().any(|c| c == '.' || c == '-') {
        if let Ok(count) = s.numeric.parse::<usize>() { s.count = count.clamp(1, LOOP_CUT_MAX); }
    } else if let Ok(factor) = s.numeric.parse::<f64>() {
        if let Some(slide) = s.slide.as_mut() { slide.raw = factor.clamp(-1., 1.); }
    }
}
fn loop_params(edge: Edge, count: usize, factor: f64, even: bool, flipped: bool, clamp: bool) -> Value {
    json!({"mode":"edge_loop","split_edges":format!("p{}-{}",edge.0,edge.1),"loops":count,
        "position":0.5,"factor":factor,"even":even,"flipped":flipped,"clamp":clamp,"correct_uvs":true,
        "outgrp_edges":true,"outgrp_edges_name":"viewport_loop_cut"})
}

/// Egui's camera projection omits the viewport aspect ratio. Store the exact NDC projection the kernel expects.
fn knife_params(camera: CameraInfo, rect: Rect, chains: &[Vec<KnifePoint>], through: bool, tolerance:f32) -> Value {
    let aspect = rect.width()/rect.height().max(1.);
    let matrix: Vec<f32> = camera.projection.iter().flat_map(|col| [col[0]/aspect,col[1],col[2],col[3]]).collect();
    let cuts: Vec<_> = chains.iter().filter(|c|c.len()>=2).map(|chain| {
        json!({"points":chain.iter().map(|p|json!({"position":p.world,"screen":p.screen,"snap":p.reference})).collect::<Vec<_>>(),"matrix":matrix})
    }).collect();
    json!({"cuts":Value::Array(cuts).to_string(),"cut_through":through,"snap_tolerance":tolerance})
}
#[derive(Clone, Debug)]
struct KnifePoint { screen: [f32;2], world: [f32;3], reference: Option<Value> }
#[derive(Clone, Copy, Debug, PartialEq)]
struct Slide { edge: Edge, raw: f64, press: Pos2, held: bool, dragged: bool }
#[derive(Clone, Default)]
struct State {
    tool: Option<Tool>, revision: u64, source: String, mesh: Arc<Mesh>, points: Vec<KnifePoint>, pending: Vec<Vec<KnifePoint>>, knife_hover: Option<KnifePoint>, cut_through: bool, angle: bool, snap_tolerance:f32, projection: [[f32; 4]; 4], aspect: f32,
    count: usize, hover: Option<Edge>, hover_key: Option<(Pos2, [[f32; 4]; 4], f32, bool, u64)>, slide: Option<Slide>, wheel: f32,
    even: bool, flipped: bool, clamp: bool, numeric: String,
    edge_selection: Vec<Edge>,
}
fn id() -> egui::Id { egui::Id::new("viewport_mesh_cut") }
fn stage_knife(s: &mut State) { if s.points.len() >= 2 { s.pending.push(std::mem::take(&mut s.points)); } else { s.points.clear(); } }
fn confirm_knife(s: &mut State, camera: CameraInfo, rect: Rect, out: &mut Actions) {
    stage_knife(s);
    if !s.pending.is_empty() {
        out.push(UiAction::MeshCutCommit { source:s.source.clone(), node_type:"knife".into(), params:knife_params(camera,rect,&s.pending,s.cut_through,s.snap_tolerance) });
        s.pending.clear();
    }
    out.push(UiAction::Editor(EditorChange::Tool(Tool::Select)));
}
pub fn cancel(ctx: &egui::Context) { ctx.data_mut(|d| d.remove::<State>(id())); }
pub fn start_edge_slide(ctx: &egui::Context, source: String, edges: Vec<Edge>) {
    if edges.is_empty() { return; }
    let mut state=State { tool:Some(Tool::EdgeSlide), source, count:1, clamp:true, edge_selection:edges, ..Default::default() };
    state.slide=Some(Slide { edge:state.edge_selection[0], raw:0., press:ctx.input(|i|i.pointer.hover_pos()).unwrap_or(Pos2::ZERO), held:false, dragged:false });
    ctx.data_mut(|d|d.insert_temp(id(),state));
}
pub fn blocks_navigation(ctx: &egui::Context) -> bool { ctx.data(|d| d.get_temp::<State>(id()).is_some_and(|s| s.slide.is_some())) }
/// Loop Cut owns the wheel throughout its modal session, including between edges.
pub fn captures_wheel(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<State>(id()).is_some_and(|s| s.tool == Some(Tool::LoopCut)))
}
pub fn show(ui: &mut egui::Ui, rect: Rect, v: &View, active:Option<(&str,&std::ops::Range<u32>)>, out: &mut Actions) {
    let tool = v.editor.tool;
    if !matches!(tool, Tool::Knife | Tool::LoopCut | Tool::EdgeSlide) { cancel(ui.ctx()); return; }
    let ctx = ui.ctx().clone();
    let mut s: State = ctx.data_mut(|d| d.get_temp(id())).unwrap_or_default();
    let ready = active.is_some() && !v.cooking.cooking() && v.cooking.error.is_none() && v.cooking.geometry.is_some();
    let source = active.map(|(id,_)|id.to_owned()).unwrap_or_default();
    let aspect = rect.width()/rect.height().max(1.);
    if s.tool != Some(tool) || s.source != source {
        s = State { tool: Some(tool), count: 1, clamp: true, ..Default::default() };
    }
    if s.revision != v.cooking.completed || s.source != source || s.mesh.positions.is_empty() && v.cooking.geometry.is_some() {
        // The cut count is a tool setting and survives re-cooks; a pending slide refers to the old topology.
        s.revision = v.cooking.completed; s.source = source;
        s.mesh = Arc::new(active.and_then(|(_,range)|v.cooking.geometry.as_deref().and_then(|g|object_geometry(g,range))).as_ref().map(Mesh::new).unwrap_or_default());
        s.slide = None; s.hover = None; s.hover_key = None;
    }
    s.count = s.count.clamp(1, LOOP_CUT_MAX);
    if tool==Tool::Knife && (s.projection!=v.camera.projection || s.aspect!=aspect) {
        let update=|p:&mut KnifePoint| { if let Some(q)=transform_gizmo::project(v.camera,rect,p.world) {
            p.screen=[(q.x-rect.center().x)*2./rect.width(),(rect.center().y-q.y)*2./rect.height()];
        } };
        for p in &mut s.points { update(p); }
        for chain in &mut s.pending { for p in chain { update(p); } }
    }
    if !ready { s.points.clear(); s.pending.clear(); }
    if !ready { s.slide = None; }
    if ready && tool==Tool::EdgeSlide && s.slide.is_none() && !s.edge_selection.is_empty() {
        s.slide=Some(Slide { edge:s.edge_selection[0], raw:0., press:ctx.input(|i|i.pointer.hover_pos()).unwrap_or(Pos2::ZERO), held:false, dragged:false });
    }
    s.projection = v.camera.projection; s.aspect = aspect;
    let canvas = |p: Pos2| super::viewport_ui::selection_canvas_contains(rect,p) && p.x > rect.left()+55. && p.y > rect.top()+83.;
    let (pointer, delta, modifiers) = ctx.input(|i| (i.pointer.hover_pos(), i.pointer.delta(), i.modifiers));
    let mut pointer = pointer.filter(|p| canvas(*p));
    if tool==Tool::Knife && s.angle { if let (Some(p),Some(last))=(pointer,s.points.last()) {
        let a=pos2(rect.center().x+last.screen[0]*rect.width()*0.5,rect.center().y-last.screen[1]*rect.height()*0.5);
        let d=p-a; let radius=d.length(); if radius>1. { let theta=(d.y.atan2(d.x)/std::f32::consts::FRAC_PI_4).round()*std::f32::consts::FRAC_PI_4;
            pointer=Some(a+vec2(theta.cos(),theta.sin())*radius); }
    } }
    let ndc = |p: Pos2| [(p.x-rect.center().x)*2./rect.width(),(rect.center().y-p.y)*2./rect.height()];
    let mut knife_primitives = Vec::new();
    if ready && tool == Tool::Knife {
        s.knife_hover = None;
        if let Some(at) = pointer {
            let settings = ctx.data(|d| d.get_temp::<inf::InferenceSettings>(super::sketch::settings_id())).unwrap_or_default();
            let mut settings = settings;
            s.snap_tolerance=settings.tolerance_px;
            settings.enabled &= inf::cat::VERTEX | inf::cat::MIDPOINT | inf::cat::EDGE | inf::cat::FACE | inf::cat::EXTENSION;
            if !modifiers.ctrl { settings.enabled &= !inf::cat::MIDPOINT; }
            settings.include_hidden = s.cut_through;
            let projector = Projector { matrix:v.camera.projection, rect_min:[rect.min.x,rect.min.y], rect_size:[rect.width(),rect.height()] };
            let ray = transform_gizmo::ray(v.camera,rect,at);
            {
                let engine_cell = super::sketch::engine(&ctx);
                let mut engine = engine_cell.lock().unwrap_or_else(|e|e.into_inner());
                let previous = s.pending.last();
                let tool = ToolContext { spec:ToolSpec::KNIFE, start:s.points.last().map(|p|p.world), start_face:None, ground:([0.;3],[0.,0.,1.]),
                    tool_points:s.points.iter().chain(previous.into_iter().flatten()).map(|p|p.world).collect(),
                    tool_segments:s.points.windows(2).chain(previous.into_iter().flat_map(|chain|chain.windows(2)))
                        .map(|pair|(pair[0].world,pair[1].world)).collect() };
                let input = InferenceInput { ray, cursor:[at.x,at.y], projector, time:ctx.input(|i|i.time),
                    geometry:Some(inf::GeometryRef { positions:&s.mesh.positions, triangles:&s.mesh.tris, curve_lines:&[], curve_centers:&[], revision:s.revision }),
                    tool, settings:&settings, alt_held:false, typed:None };
                let result = engine.update(&input);
                let hit = if modifiers.shift { picking::pick_triangle(&s.mesh.positions,&s.mesh.tris,ray.0,ray.1).map(|h| (inf::math::add(ray.0,inf::math::mul(ray.1,h.t)),None)) }
                    else { result.point.filter(|_|result.winner.kind != inf::Kind::Free).map(|p|(p,result.descriptor.as_ref().map(|d|d.to_json(&s.source,s.mesh.geometry.npoints)))) };
                if let Some((world,reference)) = hit {
                    if s.cut_through || s.mesh.visible(v.camera,rect,world) {
                        let mut screen = ndc(at);
                        if !modifiers.shift { if let Some(projected)=transform_gizmo::project(v.camera,rect,world) { screen=ndc(projected); } }
                        s.knife_hover=Some(KnifePoint {screen,world,reference});
                        if !modifiers.shift { knife_primitives=result.primitives; for primitive in &mut knife_primitives {
                            if let inf::Primitive::Tooltip {text,..}=primitive { *text=match text.as_str() {"Endpoint" if result.descriptor.is_some()=>"Vertex", "On Edge" if result.descriptor.is_some()=>"Edge", "On Face"=>"Face",other=>other}.into(); }
                        } }
                    }
                }
            }
        }
    }
    let loop_cut = ready && tool == Tool::LoopCut;
    let edge_slide = ready && tool == Tool::EdgeSlide;
    if loop_cut && s.slide.is_none() {
        let key = pointer.map(|p| (p, v.camera.projection, aspect, v.editor.xray, s.revision));
        if key != s.hover_key { s.hover = pointer.and_then(|p| s.mesh.hover(v.camera, rect, p, v.editor.xray)); s.hover_key = key; }
    } else { s.hover = None; s.hover_key = None; }
    // Loop Cut follows the seed edge; Edge Slide follows the cooked rail projection.
    if let (true, Some(edge)) = (loop_cut || edge_slide, s.slide.map(|sl|sl.edge)) {
        let (pa, pb) = (transform_gizmo::project(v.camera, rect, s.mesh.point(edge.0)), transform_gizmo::project(v.camera, rect, s.mesh.point(edge.1)));
        let motion=if delta==Vec2::ZERO {None}else{slide_screen_motion(v,rect,&s,edge)};
        let sl=s.slide.as_mut().unwrap();
        if s.numeric.is_empty() { if let (Some(pa), Some(pb)) = (pa, pb) {
            let (from,to)=if edge_slide {
                // Edge Slide moves across the selected edge, along a rail; a seed-edge projection is perpendicular.
                motion.filter(|d|d.length()>0.01).map(|d|(pa,pa+d*20.)).unwrap_or((pa,pb))
            }else{
                // Slide along the seed edge's own direction, signed by where the pointer moves across it, so the loop follows the mouse on every orientation.
                let sign=motion.map(|d|if d.dot(pb-pa)<0. {-1.} else {1.}).unwrap_or(1.);
                (pa,pa+(pb-pa)*sign)
            };
            sl.raw = slide_step(sl.raw, delta, from, to, modifiers.shift);
        } }
        if sl.held && pointer.is_some_and(|p| p.distance(sl.press) > DRAG_THRESHOLD) { sl.dragged = true; }
    }
    let commit = |s: &State, edge: Edge, factor: f64, out: &mut Actions| {
        let (node_type,params)=if s.tool==Some(Tool::EdgeSlide) {
            ("edge_slide",json!({"group":s.edge_selection.iter().map(|(a,b)|format!("p{a}-{b}")).collect::<Vec<_>>().join(" "),
                "factor":factor,"even":s.even,"flipped":s.flipped,"clamp":s.clamp,"mirror_editing":true,"correct_uvs":true,"output_group":true}))
        } else { ("polysplit",loop_params(edge,s.count,factor,s.even,s.flipped,s.clamp)) };
        out.push(UiAction::MeshCutCommit { source:s.source.clone(), node_type:node_type.into(), params });
        out.push(UiAction::Editor(EditorChange::Tool(Tool::Select)));
    };
    for event in ctx.input(|i| i.events.clone()) {
        match event {
            egui::Event::Key { key: egui::Key::Escape, pressed: true, .. } => {
                if loop_cut { if let Some(sl)=s.slide.take() { commit(&s,sl.edge,0.,out); cancel(&ctx); return; } }
                cancel(&ctx); out.push(UiAction::Editor(EditorChange::Tool(Tool::Select))); return;
            }
            egui::Event::PointerButton { button: egui::PointerButton::Secondary, pressed: true, modifiers, .. } if tool==Tool::Knife && !modifiers.alt => { stage_knife(&mut s); }
            egui::Event::PointerButton { button: egui::PointerButton::Secondary, pressed: true, modifiers, .. } if !modifiers.alt && !modifiers.ctrl => {
                if loop_cut { if let Some(sl)=s.slide.take() { commit(&s,sl.edge,0.,out); cancel(&ctx); return; } }
                cancel(&ctx); out.push(UiAction::Editor(EditorChange::Tool(Tool::Select))); return;
            }
            egui::Event::Key { key: egui::Key::Backspace, pressed: true, .. } if (loop_cut || edge_slide) && s.slide.is_some() => {
                s.numeric.pop(); numeric_slide(&mut s);
            }
            egui::Event::Key { key: egui::Key::Backspace, pressed: true, .. } => { s.points.pop(); }
            egui::Event::Key { key: egui::Key::Enter | egui::Key::Space, pressed: true, .. } if ready && tool == Tool::Knife => confirm_knife(&mut s,v.camera,rect,out),
            egui::Event::Key { key: egui::Key::E, pressed: true, .. } if tool==Tool::Knife => stage_knife(&mut s),
            egui::Event::Key { key: egui::Key::Z, pressed: true, .. } if tool==Tool::Knife => s.cut_through=!s.cut_through,
            egui::Event::Key { key: egui::Key::C, pressed: true, .. } if tool==Tool::Knife => s.angle=!s.angle,
            egui::Event::Key { key: egui::Key::Enter, pressed: true, modifiers, .. } if loop_cut || edge_slide => {
                if let Some(sl) = s.slide.take() { commit(&s, sl.edge, slide_factor(sl.raw, modifiers.ctrl), out); }
                else if let Some(edge) = s.hover { commit(&s, edge, 0., out); }
            }
            egui::Event::Key { key: egui::Key::E, pressed: true, .. } if (loop_cut || edge_slide) && s.slide.is_some() => s.even=!s.even,
            egui::Event::Key { key: egui::Key::F, pressed: true, .. } if (loop_cut || edge_slide) && s.slide.is_some() => s.flipped=!s.flipped,
            egui::Event::Key { key: egui::Key::C, pressed: true, .. } if (loop_cut || edge_slide) && s.slide.is_some() => s.clamp=!s.clamp,
            egui::Event::Text(ref value) if (loop_cut || edge_slide) && s.slide.is_some() && value.chars().all(|c| c.is_ascii_digit() || c=='.' || c=='-') => {
                s.numeric.push_str(value); numeric_slide(&mut s);
            }
            egui::Event::Key { key: key @ (egui::Key::PageUp | egui::Key::PageDown | egui::Key::Plus | egui::Key::Minus), pressed: true, .. } if loop_cut && s.slide.is_none() => {
                s.count = step_count(s.count, if matches!(key, egui::Key::PageUp | egui::Key::Plus) { 1 } else { -1 });
            }
            egui::Event::MouseWheel { unit, delta, .. } if loop_cut && s.slide.is_none() && pointer.is_some() => {
                s.wheel += wheel_notches(unit, delta.y);
                let steps = s.wheel.trunc();
                if steps != 0. { s.count = step_count(s.count, steps as i32); s.wheel -= steps; }
            }
            egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers } if ready && canvas(pos) && !modifiers.alt => {
                if tool == Tool::Knife {
                    if let Some(p)=s.knife_hover.clone() {
                        if s.points.len()<513 && s.points.last().is_none_or(|a|a.screen!=p.screen) { s.points.push(p); }
                    }
                } else if let Some(sl) = s.slide {
                    s.slide = None; commit(&s, sl.edge, slide_factor(sl.raw, modifiers.ctrl), out);
                } else if let Some(edge) = s.hover {
                    s.numeric.clear();
                    s.slide = Some(Slide { edge, raw: 0., press: pos, held: true, dragged: false });
                }
            }
            egui::Event::PointerButton { button: egui::PointerButton::Primary, pressed: false, modifiers, .. } if loop_cut || edge_slide => {
                if let Some(sl) = s.slide.as_mut() {
                    // Press-drag-release (Blender) commits on release; a plain click keeps sliding (loop-tool.js).
                    if sl.held && sl.dragged { let sl = s.slide.take().unwrap(); commit(&s, sl.edge, slide_factor(sl.raw, modifiers.ctrl), out); }
                    else { sl.held = false; }
                }
            }
            _ => {}
        }
    }
    let painter = ui.painter_at(rect); let stroke = Stroke::new(2.5_f32,v.theme.warn);
    let snap = modifiers.ctrl;
    let active = s.slide.map(|sl| (sl.edge, slide_factor(sl.raw, snap))).or(s.hover.map(|e| (e, 0.)));
    if let (true, Some((seed, factor))) = (loop_cut, active) {
        let lines=if s.slide.is_some() { loop_cut_preview(&s.mesh.geometry,seed,s.count,factor,s.even,s.flipped,s.clamp).unwrap_or_default() }
            else { s.mesh.preview(seed,s.count,0.) };
        for (a,b) in lines {
            if let (Some(a),Some(b)) = (transform_gizmo::project(v.camera,rect,a),transform_gizmo::project(v.camera,rect,b)) { painter.line_segment([a,b],stroke); }
        }
        if let (Some(a), Some(b)) = (transform_gizmo::project(v.camera,rect,s.mesh.point(seed.0)), transform_gizmo::project(v.camera,rect,s.mesh.point(seed.1))) {
            painter.line_segment([a,b],Stroke::new(1_f32,v.theme.text));
        }
        if let Some(p) = pointer {
            let label = match s.slide { Some(_) => format!("{} cut{}  Factor {factor:.3}{}", s.count, if s.count == 1 { "" } else { "s" }, if s.numeric.is_empty() { "" } else { " (numeric)" }), _ => format!("{} cut{}", s.count, if s.count == 1 { "" } else { "s" }) };
            let at = p + vec2(16., -18.);
            let galley = painter.layout_no_wrap(label, FontId::monospace(12.), v.theme.text);
            painter.rect_filled(Rect::from_min_size(at - vec2(3., 2.), galley.size() + vec2(6., 4.)), 2., v.theme.panel);
            painter.galley(at, galley, v.theme.text);
        }
    }
    if edge_slide {
        if let Some(sl)=s.slide {
            let g=&s.mesh.geometry;
            let preview=edge_slide_preview(g,&s.edge_selection,slide_factor(sl.raw,snap),s.even,s.flipped,s.clamp,true);
            for &(a,b) in &s.edge_selection {
                if a>=preview.npoints || b>=preview.npoints { continue; }
                let point=|i:usize| [preview.p()[3*i],preview.p()[3*i+1],preview.p()[3*i+2]];
                if let (Some(a),Some(b))=(transform_gizmo::project(v.camera,rect,point(a)),transform_gizmo::project(v.camera,rect,point(b))) {
                    painter.line_segment([a,b],stroke);
                }
            }
        }
    }
    if tool == Tool::Knife {
        let screen = |p: [f32;2]| pos2(rect.center().x+p[0]*rect.width()*0.5,rect.center().y-p[1]*rect.height()*0.5);
        for chain in &s.pending { let path:Vec<_>=chain.iter().map(|p|screen(p.screen)).collect();
            for pair in path.windows(2) { painter.line_segment([pair[0],pair[1]],stroke); }
            for p in path { painter.circle_filled(p,3.,v.theme.warn); }
        }
        let mut path: Vec<_> = s.points.iter().map(|p| screen(p.screen)).collect();
        if let Some(p) = s.knife_hover.as_ref() { if !path.is_empty() { path.push(screen(p.screen)); } }
        for pair in path.windows(2) { painter.line_segment([pair[0],pair[1]],stroke); }
        for p in &s.points { painter.circle_filled(screen(p.screen),3.,v.theme.warn); }
        super::sketch::paint_primitives(&painter,&knife_primitives,|q|transform_gizmo::project(v.camera,rect,q),v.theme.panel);
    }
    let hint = if !ready { "Cut tools: select a cooked active mesh stream".to_owned() }
        else if tool == Tool::Knife { format!("Knife: click | RMB/E next chain | Enter/Space apply | Backspace undo | Esc cancel | Ctrl midpoint, Shift free, Z through {} | C angle {}",s.cut_through,s.angle) }
        else if tool==Tool::EdgeSlide { format!("Edge Slide | Factor {:.3} | Shift precision, Ctrl snap, E Even, F Flip, C Clamp, type factor | LMB/Enter confirm | RMB/Esc cancel",s.slide.map(|sl|slide_factor(sl.raw,snap)).unwrap_or(0.)) }
        else { loop_hint(s.count, s.slide.map(|sl| slide_factor(sl.raw, snap)), s.hover.is_some()) };
    let galley = painter.layout_no_wrap(hint, FontId::monospace(11.), v.theme.text);
    let chip = Rect::from_min_size(rect.min+vec2(56.,48.),vec2(galley.size().x + 16., 28.));
    painter.rect_filled(chip,2.,v.theme.panel);
    painter.galley(chip.left_center()+vec2(8.,-galley.size().y*0.5),galley,v.theme.text);
    if s.slide.is_some() || s.hover.is_some() { ctx.request_repaint(); }
    ctx.data_mut(|d| d.insert_temp(id(),s));
}
/// Status text reflects the two Blender modal phases.
fn loop_hint(count: usize, slide: Option<f64>, hovering: bool) -> String {
    let cuts = format!("{count} cut{}", if count == 1 { "" } else { "s" });
    match slide {
        Some(f) => format!("Loop Cut | {cuts} | Factor {f:.3} | Shift precision, Ctrl snap, E Even, F Flip, C Clamp, type cuts or decimal factor | LMB/Enter confirm | RMB: keep centred | Esc: keep centred"),
        None if hovering => format!("Loop Cut | {cuts} | wheel/PgUp/PgDn/+/- count | LMB to slide | Esc/RMB cancel"),
        None => format!("Loop Cut | {cuts} | hover a quad edge | Esc/RMB exit"),
    }
}

#[cfg(test)] mod tests {
    use super::*;
    fn ortho() -> CameraInfo {
        CameraInfo { ortho: true, projection: [[1.,0.,0.,0.],[0.,1.,0.,0.],[0.,0.,1.,0.],[0.,0.,0.,1.]],
            ray_origin: [0.,0.,10.], ray_forward: [0.,0.,-1.], ray_right: [1.,0.,0.], ray_up: [0.,1.,0.], ..Default::default() }
    }
    fn geo(p: Vec<f32>, verts: Vec<i32>, offsets: Vec<i32>) -> Geometry { Geometry::from_arrays(p, verts, offsets, None).unwrap() }
    #[test] fn knife_chains_stage_and_confirm_once() {
        let point=|x|KnifePoint {screen:[x,0.],world:[x,0.,0.],reference:None};
        let mut state=State {source:"active".into(),points:vec![point(0.),point(1.)],..Default::default()};
        stage_knife(&mut state); assert_eq!(state.pending.len(),1); assert!(state.points.is_empty());
        state.points=vec![point(2.),point(3.)];
        let mut out=Actions::default();
        confirm_knife(&mut state,ortho(),Rect::from_min_size(Pos2::ZERO,vec2(200.,200.)),&mut out);
        assert_eq!(out.0.len(),2);
        let UiAction::MeshCutCommit {params,..}=&out.0[0] else {panic!("expected knife commit")};
        let chains:Value=serde_json::from_str(params["cuts"].as_str().unwrap()).unwrap();
        assert_eq!(chains.as_array().unwrap().len(),2);
    }
    /// Two quads side by side in z = 0: 0(0,0) 1(1,0) 2(2,0) 3(0,1) 4(1,1) 5(2,1).
    fn strip() -> Geometry { geo(vec![0.,0.,0., 1.,0.,0., 2.,0.,0., 0.,1.,0., 1.,1.,0., 2.,1.,0.], vec![0,1,4,3, 1,2,5,4], vec![0,4,8]) }
    #[test] fn count_clamps_to_polysplit_range() {
        assert_eq!(step_count(1, -1), 1);
        assert_eq!(step_count(1, 3), 4);
        assert_eq!(step_count(63, 5), 64);
        assert_eq!(wheel_notches(egui::MouseWheelUnit::Point, 100.), 2.);
    }
    #[test] fn slide_follows_the_projected_seed_edge() {
        let (a, b) = (pos2(0., 0.), pos2(200., 0.));
        assert!((slide_step(0., vec2(50., 0.), a, b, false) - 0.5).abs() < 1e-9);
        assert!((slide_step(0., vec2(-50., 0.), a, b, false) + 0.5).abs() < 1e-9);
        assert!((slide_step(0., vec2(50., 0.), a, b, true) - 0.05).abs() < 1e-9, "shift is 10x finer");
        assert_eq!(slide_step(0., vec2(0., 80.), a, b, false), 0.);
        assert_eq!(slide_step(0.9, vec2(500., 0.), a, b, false), 1.);
        assert_eq!(slide_step(0., vec2(50., 0.), a, a, false), 1., "end-on edge uses 100px horizontal mapping");
        assert_eq!(slide_factor(1., false), 1.);
        assert_eq!(slide_factor(0., false), 0.);
        assert_eq!(slide_factor(0.437, true), 0.4);
        assert_eq!(slide_factor(0.43712, false), 0.4371);
    }
    #[test] fn hover_preview_is_the_node_ring_and_ignores_hidden_edges() {
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(200., 200.));
        let cam = ortho();
        let m = Mesh::new(&strip());
        let over = |x: f32, y: f32| transform_gizmo::project(cam, rect, [x, y, 0.]).unwrap();
        assert_eq!(m.hover(cam, rect, over(1., 0.5), false), Some((1, 4)));
        assert_eq!(m.preview((1, 4), 2, 0.5).len(), 4, "two quads x two cuts");
        let lines = m.preview((1, 4), 2, 0.);
        let mut ys: Vec<_> = lines.iter().map(|(a, b)| { assert!((a[1] - b[1]).abs() < 1e-6); a[1] }).collect();
        ys.sort_by(f32::total_cmp);
        assert!((ys[0] - 1. / 3.).abs() < 1e-6 && (ys[3] - 2. / 3.).abs() < 1e-6,
            "all preview cuts start evenly spaced");
        let moved=m.preview((1,4),2,0.5);
        assert!(moved[0].0[1] > lines[0].0[1], "all cuts slide together");
        assert_eq!(m.hover(cam, rect, over(1., 0.5) + vec2(30., 0.), false), None, "outside the radius");
        // a larger quad in front (z = 1) hides the strip unless X-ray is on
        let mut g = strip();
        g = geo([g.p(), &[-1.,-1.,1., 3.,-1.,1., 3.,2.,1., -1.,2.,1.][..]].concat(), vec![0,1,4,3, 1,2,5,4, 6,7,8,9], vec![0,4,8,12]);
        let m = Mesh::new(&g);
        assert_eq!(m.hover(cam, rect, over(1., 0.5), false), None);
        assert_eq!(m.hover(cam, rect, over(1., 0.5), true), Some((1, 4)));
    }
    #[test] fn active_stream_preview_ignores_two_other_displayed_meshes() {
        let base=strip();
        let mut points=base.p().to_vec();
        points.extend([-1.,-1.,1.,3.,-1.,1.,3.,2.,1.,-1.,2.,1.]);
        points.extend([-1.,-1.,2.,3.,-1.,2.,3.,2.,2.,-1.,2.,2.]);
        let merged=geo(points,vec![0,1,4,3,1,2,5,4,6,7,8,9,10,11,12,13],vec![0,4,8,12,16]);
        let active=object_geometry(&merged,&(0..6)).unwrap();
        let mesh=Mesh::new(&active);
        let rect=Rect::from_min_size(Pos2::ZERO,vec2(200.,200.));
        let cam=ortho();
        let pointer=transform_gizmo::project(cam,rect,[1.,0.5,0.]).unwrap();
        assert_eq!(mesh.hover(cam,rect,pointer,false),Some((1,4)));
        assert_eq!(mesh.preview((1,4),2,0.).len(),4);
        assert_eq!(active.npoints,6);
    }
    #[test] fn perspective_hover_picks_the_front_edge() {
        // camera at z = 10 looking down -z with a simple perspective projection (w = 10 - z)
        let cam = CameraInfo { ortho: false, projection: [[1.,0.,0.,0.],[0.,1.,0.,0.],[0.,0.,1.,-1.],[0.,0.,0.,10.]],
            ray_origin: [0.,0.,10.], ray_forward: [0.,0.,-1.], ray_right: [1.,0.,0.], ray_up: [0.,1.,0.], ..Default::default() };
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(200., 200.));
        let m = Mesh::new(&strip());
        let p = transform_gizmo::project(cam, rect, [1., 0.5, 0.]).unwrap();
        assert_eq!(m.hover(cam, rect, p, false), Some((1, 4)));
        let (o, d) = transform_gizmo::ray(cam, rect, p);
        assert!((o[0] + d[0] * 10. / -d[2] - 1.).abs() < 1e-3, "ray goes through the projected point");
    }
    #[test] fn real_camera_off_centre_ray_hits_projected_edge() {
        let camera = minihou_app::Camera::frame([1., 0.5, 0.], 2.);
        let (projection, _) = camera.matrices(1.);
        let (ray_origin, ray_forward) = camera.ray(0., 0., 1.);
        let (_, dx) = camera.ray(1., 0., 1.);
        let (_, dy) = camera.ray(0., 1., 1.);
        let info = CameraInfo { projection, ray_origin, ray_forward,
            ray_right: [0, 1, 2].map(|k| dx[k] - ray_forward[k]),
            ray_up: [0, 1, 2].map(|k| dy[k] - ray_forward[k]), ..Default::default() };
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(1280., 800.));
        let point = [0., 0.5, 0.];
        let screen = transform_gizmo::project(info, rect, point).unwrap();
        let (origin, direction) = transform_gizmo::ray(info, rect, screen);
        let depth = dot([0, 1, 2].map(|k| point[k] - origin[k]), direction);
        let miss = [0, 1, 2].map(|k| origin[k] + depth * direction[k] - point[k]);
        assert!(dot(miss, miss).sqrt() < 1e-4, "off-centre ray must pass through its projected point: {miss:?}");
        let a = [0., 0., 0.]; let b = [0., 1., 0.];
        let t = edge_world_fraction(info, a, b, 0.5);
        let projected = transform_gizmo::project(info, rect, lerp3(a, b, t)).unwrap();
        let midpoint = transform_gizmo::project(info, rect, a).unwrap()
            + (transform_gizmo::project(info, rect, b).unwrap() - transform_gizmo::project(info, rect, a).unwrap()) * 0.5;
        assert!(projected.distance(midpoint) < 1e-3, "occlusion must test the point actually under the cursor");
        assert_eq!(Mesh::new(&strip()).hover(info, rect, screen, false), Some((0, 3)));
    }
    struct Harness { fixture: super::super::testing::Fixture, ctx: egui::Context, rect: Rect }
    impl Harness {
        fn new() -> Self {
            let mut fixture = super::super::testing::Fixture::new();
            fixture.editor.tool = Tool::LoopCut;
            fixture.camera = ortho();
            fixture.cooking.geometry = Some(Arc::new(strip()));
            Self { fixture, ctx: egui::Context::default(), rect: Rect::from_min_size(Pos2::ZERO, vec2(1280., 800.)) }
        }
        fn at(&self, x: f32, y: f32) -> Pos2 { transform_gizmo::project(self.fixture.camera, self.rect, [x, y, 0.]).unwrap() }
        fn frame(&self, events: Vec<egui::Event>) -> Vec<UiAction> {
            let mut out = Actions::default();
            let (rect, view) = (self.rect, self.fixture.view());
            let _ = self.ctx.run(egui::RawInput { screen_rect: Some(rect), events, ..Default::default() }, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| { let range = 0..view.cooking.geometry.as_ref().map_or(0, |g| g.npoints as u32); show(ui, rect, &view, Some((view.display, &range)), &mut out) });
            });
            out.0
        }
    }
    fn button(pos: Pos2, button: egui::PointerButton, pressed: bool) -> egui::Event {
        egui::Event::PointerButton { pos, button, pressed, modifiers: egui::Modifiers::NONE }
    }
    fn key(key: egui::Key) -> egui::Event { egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE } }
    fn committed(actions: &[UiAction]) -> Option<Value> {
        actions.iter().find_map(|a| match a { UiAction::MeshCutCommit { node_type, params, .. } if node_type == "polysplit" => Some(params.clone()), _ => None })
    }
    #[test] fn knife_next_chain_snaps_to_previous_endpoint() {
        let mut h=Harness::new(); h.fixture.editor.tool=Tool::Knife;
        let mut settings=inf::InferenceSettings::default(); settings.tooltip_delay_s=0.;
        h.ctx.data_mut(|d|d.insert_temp(super::super::sketch::settings_id(),settings));
        for x in [0.2,0.4] {
            let p=h.at(x,0.2);
            h.frame(vec![egui::Event::PointerMoved(p),button(p,egui::PointerButton::Primary,true)]);
            h.frame(vec![button(p,egui::PointerButton::Primary,false)]);
        }
        let endpoint=h.ctx.data(|d|d.get_temp::<State>(id()).unwrap().points[1].world);
        h.frame(vec![button(h.at(0.4,0.2),egui::PointerButton::Secondary,true)]);
        let near=h.at(0.4,0.2)+vec2(3.,2.);
        h.frame(vec![egui::Event::PointerMoved(near)]);
        let result=super::super::sketch::engine(&h.ctx).lock().unwrap().last().cloned().unwrap();
        assert_eq!(result.winner.kind,inf::Kind::Endpoint);
        assert_eq!(result.point,Some(endpoint));
        assert!(result.primitives.iter().any(|p|matches!(p,inf::Primitive::Tooltip {text,..} if text=="Endpoint")));
        h.frame(vec![button(near,egui::PointerButton::Primary,true)]);
        assert_eq!(h.ctx.data(|d|d.get_temp::<State>(id()).unwrap().points[0].world),endpoint);
    }
    #[test] fn knife_rmb_and_e_stage_two_chains_then_enter_commits_one_node() {
        let mut h=Harness::new(); h.fixture.editor.tool=Tool::Knife;
        let click=|h:&Harness,x,y| {let p=h.at(x,y);h.frame(vec![egui::Event::PointerMoved(p)]);
            let out=h.frame(vec![button(p,egui::PointerButton::Primary,true)]);h.frame(vec![button(p,egui::PointerButton::Primary,false)]);out};
        assert!(click(&h,0.2,0.2).is_empty());assert!(click(&h,0.8,0.2).is_empty());
        assert!(h.frame(vec![button(h.at(0.8,0.2),egui::PointerButton::Secondary,true)]).is_empty());
        h.frame(vec![button(h.at(0.8,0.2),egui::PointerButton::Secondary,false)]);
        assert_eq!(h.ctx.data(|d|d.get_temp::<State>(id()).unwrap().pending.len()),1);
        assert!(click(&h,0.2,0.7).is_empty());assert!(click(&h,0.8,0.7).is_empty());
        assert_eq!(h.ctx.data(|d|d.get_temp::<State>(id()).unwrap().points.len()),2);
        assert!(h.frame(vec![key(egui::Key::E)]).is_empty());
        assert_eq!(h.ctx.data(|d|d.get_temp::<State>(id()).unwrap().pending.len()),2);
        let out=h.frame(vec![key(egui::Key::Enter)]);
        let commits:Vec<_>=out.iter().filter_map(|a|if let UiAction::MeshCutCommit {params,..}=a {Some(params)} else {None}).collect();
        assert_eq!(commits.len(),1);
        let chains:Value=serde_json::from_str(commits[0]["cuts"].as_str().unwrap()).unwrap();
        assert_eq!(chains.as_array().unwrap().len(),2);
    }
    #[test] fn knife_backspace_and_escape_discard_pending_chains() {
        let mut h=Harness::new();h.fixture.editor.tool=Tool::Knife;
        for x in [0.2,0.8] {let p=h.at(x,0.2);h.frame(vec![egui::Event::PointerMoved(p),button(p,egui::PointerButton::Primary,true)]);
            h.frame(vec![button(p,egui::PointerButton::Primary,false)]);}
        h.frame(vec![key(egui::Key::Backspace)]);
        assert_eq!(h.ctx.data(|d|d.get_temp::<State>(id()).unwrap().points.len()),1);
        let out=h.frame(vec![key(egui::Key::Escape)]);
        assert!(!out.iter().any(|a|matches!(a,UiAction::MeshCutCommit {..})));
        assert!(h.ctx.data(|d|d.get_temp::<State>(id()).is_none()));
    }
    #[test] fn wheel_count_click_slide_and_confirm_make_one_node_edit() {
        let h = Harness::new();
        let p = h.at(1., 0.5);
        assert!(h.frame(vec![egui::Event::PointerMoved(p)]).is_empty());
        assert!(captures_wheel(&h.ctx));
        let wheel = egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Line, delta: vec2(0., 1.), modifiers: egui::Modifiers::NONE };
        assert!(h.frame(vec![wheel.clone(), wheel.clone(), key(egui::Key::PageDown), key(egui::Key::PageUp)]).is_empty());
        assert!(h.frame(vec![key(egui::Key::Plus), key(egui::Key::Minus)]).is_empty());
        assert!(h.frame(vec![button(p, egui::PointerButton::Primary, true)]).is_empty());
        assert!(h.frame(vec![button(p, egui::PointerButton::Primary, false)]).is_empty(), "plain click keeps sliding");
        assert!(blocks_navigation(&h.ctx));
        let params = committed(&h.frame(vec![key(egui::Key::Enter)])).unwrap();
        assert_eq!(params, loop_params((1,4),3,0.,false,false,true));
        assert!(!blocks_navigation(&h.ctx));
    }
    #[test] fn wheel_count_survives_moving_between_edges() {
        let h = Harness::new();
        let p = h.at(1., 0.5);
        h.frame(vec![egui::Event::PointerMoved(p)]);
        let away = p + vec2(160., 120.);
        h.frame(vec![egui::Event::PointerMoved(away)]);
        assert!(captures_wheel(&h.ctx));
        let wheel = egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Line,
            delta: vec2(0., 2.), modifiers: egui::Modifiers::NONE };
        h.frame(vec![wheel]);
        h.frame(vec![egui::Event::PointerMoved(p)]);
        let params = committed(&h.frame(vec![key(egui::Key::Enter)])).unwrap();
        assert_eq!(params["loops"], 3);
    }
    #[test] fn drag_release_commits_the_slid_factor_and_rmb_keeps_centre() {
        let h = Harness::new();
        let p = h.at(1., 0.5);
        h.frame(vec![egui::Event::PointerMoved(p)]);
        h.frame(vec![button(p, egui::PointerButton::Primary, true)]);
        // drag a quarter of the edge towards point 4 (screen up), then release
        let q = h.at(1., 0.75);
        h.frame(vec![egui::Event::PointerMoved(q)]);
        let params = committed(&h.frame(vec![button(q, egui::PointerButton::Primary, false)])).unwrap();
        assert!((params["factor"].as_f64().unwrap() - 0.5).abs() < 1e-3, "{params}");
        assert_eq!(params["loops"], 1);
        // RMB during a slide inserts the centred cut as one edit.
        h.frame(vec![egui::Event::PointerMoved(p)]);
        h.frame(vec![button(p, egui::PointerButton::Primary, true), button(p, egui::PointerButton::Primary, false)]);
        h.frame(vec![egui::Event::PointerMoved(q)]);
        let actions = h.frame(vec![button(q, egui::PointerButton::Secondary, true)]);
        assert_eq!(committed(&actions).unwrap()["factor"],0.);
    }
    #[test] fn flipped_drag_tracks_the_same_screen_side() {
        let h=Harness::new();
        let p=h.at(1.,0.5);
        h.frame(vec![egui::Event::PointerMoved(p)]);
        h.frame(vec![button(p,egui::PointerButton::Primary,true)]);
        h.frame(vec![key(egui::Key::F)]);
        let q=h.at(1.,0.75);
        h.frame(vec![egui::Event::PointerMoved(q)]);
        let params=committed(&h.frame(vec![button(q,egui::PointerButton::Primary,false)])).unwrap();
        assert_eq!(params["flipped"],true);
        assert!(params["factor"].as_f64().unwrap()<0.,"flipped rail must still follow screen-up drag: {params}");
    }
    #[test] fn escape_during_slide_keeps_centre_and_hover_escape_cancels() {
        let mut h = Harness::new();
        let p = h.at(1., 0.5);
        h.frame(vec![egui::Event::PointerMoved(p)]);
        h.frame(vec![button(p, egui::PointerButton::Primary, true)]);
        let out = h.frame(vec![key(egui::Key::Escape)]);
        assert_eq!(committed(&out).unwrap()["factor"],0.);
        h.frame(vec![egui::Event::PointerMoved(p)]);
        let out=h.frame(vec![key(egui::Key::Escape)]);
        assert!(committed(&out).is_none());
        h.fixture.editor.tool = Tool::Select;
        let wheel = egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Line, delta: vec2(0., 1.), modifiers: egui::Modifiers::NONE };
        assert!(h.frame(vec![egui::Event::PointerMoved(p), wheel, button(p, egui::PointerButton::Primary, true)]).is_empty());
        assert!(!captures_wheel(&h.ctx) && !blocks_navigation(&h.ctx));
    }
    #[test] fn slide_toggles_and_numeric_factor_commit_once() {
        let h=Harness::new(); let p=h.at(1.,0.5);
        h.frame(vec![egui::Event::PointerMoved(p)]);
        h.frame(vec![button(p,egui::PointerButton::Primary,true),button(p,egui::PointerButton::Primary,false)]);
        let params=committed(&h.frame(vec![key(egui::Key::E),key(egui::Key::F),key(egui::Key::C),
            egui::Event::Text("-0.4".into()),key(egui::Key::Enter)])).unwrap();
        assert_eq!(params["factor"],-0.4);
        assert_eq!((params["even"].as_bool(),params["flipped"].as_bool(),params["clamp"].as_bool()),(Some(true),Some(true),Some(false)));
    }
    #[test] fn typed_digits_set_cut_count_during_slide() {
        let h=Harness::new(); let p=h.at(1.,0.5);
        h.frame(vec![egui::Event::PointerMoved(p)]);
        h.frame(vec![button(p,egui::PointerButton::Primary,true),button(p,egui::PointerButton::Primary,false)]);
        h.frame(vec![egui::Event::Text("1".into()),egui::Event::Text("2".into())]);
        let state=h.ctx.data(|d|d.get_temp::<State>(id()).unwrap());
        assert_eq!(state.count,12);
        assert_eq!(state.slide.unwrap().raw,0.);
        let params=committed(&h.frame(vec![key(egui::Key::E),key(egui::Key::F),key(egui::Key::Enter)])).unwrap();
        assert_eq!(params["loops"],12);
        assert_eq!(params["even"],true);
        assert_eq!(params["flipped"],true);
    }
    #[test] fn edge_slide_selected_edges_confirm_and_cancel() {
        let mut h=Harness::new(); h.fixture.editor.tool=Tool::EdgeSlide;
        let source=minihou_app::display_nodes(h.fixture.view().doc)[0].clone();
        start_edge_slide(&h.ctx,source,vec![(1,4)]);
        let params=h.frame(vec![egui::Event::Text("0.5".into()),key(egui::Key::Enter)]);
        assert!(params.iter().any(|a|matches!(a,UiAction::MeshCutCommit {node_type,params,..} if node_type=="edge_slide" && params["factor"]==0.5 && params["group"]=="p1-4")));
        let source=minihou_app::display_nodes(h.fixture.view().doc)[0].clone();
        start_edge_slide(&h.ctx,source,vec![(1,4)]);
        assert!(h.frame(vec![key(egui::Key::Escape)]).iter().all(|a|!matches!(a,UiAction::MeshCutCommit {..})));
    }
    #[test] fn edge_slide_drag_uses_rail_direction() {
        let mut h=Harness::new();h.fixture.editor.tool=Tool::EdgeSlide;
        let p=h.at(1.,0.5);
        h.frame(vec![egui::Event::PointerMoved(p)]);
        let moved=edge_slide_preview(h.fixture.cooking.geometry.as_deref().unwrap(),&[(1,4)],0.1,false,false,true,true);
        let q=transform_gizmo::project(h.fixture.camera,h.rect,[moved.p()[3],moved.p()[4],moved.p()[5]]).unwrap();
        let origin=h.at(1.,0.);
        let toward=(q-origin).normalized()*30.;
        let source=minihou_app::display_nodes(h.fixture.view().doc)[0].clone();
        start_edge_slide(&h.ctx,source,vec![(1,4)]);
        h.frame(vec![egui::Event::PointerMoved(p+toward)]);
        let params=h.frame(vec![key(egui::Key::Enter)]).into_iter().find_map(|a|match a {
            UiAction::MeshCutCommit {node_type,params,..} if node_type=="edge_slide" => Some(params), _=>None,
        }).unwrap();
        assert!(params["factor"].as_f64().unwrap()>0.,"dragging along the visible rail must choose that rail: {params}");
    }
    #[test] fn knife_projection_matches_viewport_with_non_square_aspect() {
        let camera = CameraInfo { projection: [[1.,0.,0.,0.],[0.,1.,0.,0.],[0.,0.,1.,0.],[0.,0.,0.,1.]], ..Default::default() };
        let rect = Rect::from_min_size(pos2(30.,20.),vec2(800.,400.));
        let points=vec![KnifePoint {screen:[-0.5,0.],world:[0.;3],reference:None},KnifePoint {screen:[0.5,0.],world:[0.;3],reference:None}];
        let params = knife_params(camera,rect,&[points],false,10.);
        let cuts: Value = serde_json::from_str(params["cuts"].as_str().unwrap()).unwrap();
        assert_eq!(cuts[0]["matrix"][0],0.5); assert_eq!(cuts[0]["points"][0]["screen"],json!([-0.5,0.]));
        let screen = transform_gizmo::project(camera,rect,[1.,0.,0.]).unwrap();
        assert_eq!((screen.x-rect.center().x)*2./rect.width(),0.5);
    }
}
