//! Viewport Mirror tool: Houdini's Mirror tool state combined with a HardOps-style axis gizmo.
//!
//! Alt+M (or the shelf's Mirror button) starts a session on the selected node. Six cubes sit just outside the
//! object's bounds: +X/-X red, +Y/-Y green, +Z/-Z blue. Clicking a cube mirrors across that axis about the bounds
//! centre (Shift+click: about the world origin): the +X cube keeps the +X half (`cut`) and mirrors it onto -X, the
//! -X cube sets Reverse and keeps the -X half. Each new axis chains another `mirror` node after the previous one (so X
//! then Y gives a quarter mirrored twice). Clicking the cube of an axis that already has a node flips it to that side;
//! clicking the side it already uses removes the node again (toggle off).
//!
//! The active node's plane is drawn as a translucent square with an origin dot (drag: move the plane in the view),
//! an arrow along the kept side (drag: slide along the direction) and a ring (drag: rotate the direction freely;
//! Shift snaps to 15 degrees, which includes the axes). F then drag draws the plane: it passes through the start point
//! (the surface under the pointer, else the bounds centre's depth) with its direction along the drawn line; F then a
//! click on a face uses that face's normal. G toggles the input wireframe, H the plane. Enter or a click on empty
//! viewport confirms the whole session as one undo step; Esc or RMB restores the graph exactly.
use super::{transform_gizmo::{project, ray}, CameraInfo};
use eframe::egui::{self, Color32, Key, Pos2, Rect, Shape, Stroke};
use minihou_app::{Document, Edit};
use serde_json::{json, Value};

/// Half the on-screen size of an axis cube, in pixels.
pub const CUBE_PX: f32 = 7.;
/// Pointer distance that still hovers a cube or a plane handle.
pub const HIT_PX: f32 = 11.;
const RING_SEGMENTS: usize = 64;
/// Wireframe edges kept for the G overlay.
pub const WIRE_LIMIT: usize = 20_000;

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 { (0..3).map(|k| a[k] * b[k]).sum() }
fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] { [0, 1, 2].map(|k| a[k] - b[k]) }
fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] { [0, 1, 2].map(|k| a[k] + b[k]) }
fn mul(a: [f64; 3], s: f64) -> [f64; 3] { a.map(|x| x * s) }
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] { [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]] }
fn unit(a: [f64; 3]) -> Option<[f64; 3]> { let l = dot(a, a).sqrt(); (l > 1e-12).then(|| mul(a, 1. / l)) }
fn f64s(a: [f32; 3]) -> [f64; 3] { a.map(f64::from) }
fn f32s(a: [f64; 3]) -> [f32; 3] { a.map(|x| x as f32) }
fn round(a: [f64; 3]) -> [f64; 3] { a.map(|x| (x * 1e6).round() / 1e6) }

/// One of the six axis cubes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cube { pub axis: usize, pub negative: bool }
impl Cube {
    pub const ALL: [Cube; 6] = [Cube { axis: 0, negative: false }, Cube { axis: 0, negative: true },
        Cube { axis: 1, negative: false }, Cube { axis: 1, negative: true },
        Cube { axis: 2, negative: false }, Cube { axis: 2, negative: true }];
    pub fn color(self) -> Color32 {
        [Color32::from_rgb(230, 70, 70), Color32::from_rgb(110, 200, 70), Color32::from_rgb(70, 130, 240)][self.axis]
    }
    pub fn label(self) -> String { format!("{}{}", if self.negative { "-" } else { "+" }, ["X", "Y", "Z"][self.axis]) }
    /// The Direction written for this cube (always the positive axis; the side is carried by Reverse).
    pub fn direction(self) -> [f64; 3] { [0, 1, 2].map(|k| if k == self.axis { 1. } else { 0. }) }
    /// Reverse keeps the negative half: the -X cube mirrors the -X side onto +X.
    pub fn reverse(self) -> bool { self.negative }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds { pub min: [f32; 3], pub max: [f32; 3] }
impl Bounds {
    pub fn of(points: impl IntoIterator<Item = [f32; 3]>) -> Option<Self> {
        let mut b: Option<Self> = None;
        for p in points {
            if !p.iter().all(|x| x.is_finite()) { continue; }
            let c = b.get_or_insert(Self { min: p, max: p });
            for k in 0..3 { c.min[k] = c.min[k].min(p[k]); c.max[k] = c.max[k].max(p[k]); }
        }
        b
    }
    pub fn center(&self) -> [f32; 3] { [0, 1, 2].map(|k| (self.min[k] + self.max[k]) * 0.5) }
    /// Half the diagonal, never zero (a single point still gets a usable gizmo).
    pub fn radius(&self) -> f32 {
        ((0..3).map(|k| (self.max[k] - self.min[k]).powi(2)).sum::<f32>().sqrt() * 0.5).max(1e-3)
    }
    /// A cube's world position: just outside the matching bounds face.
    pub fn cube_at(&self, cube: Cube) -> [f32; 3] {
        let mut p = self.center();
        let half = (self.max[cube.axis] - self.min[cube.axis]) * 0.5 + self.radius() * 0.25;
        p[cube.axis] += if cube.negative { -half } else { half };
        p
    }
}

fn eye_depth(camera: CameraInfo, p: [f32; 3]) -> f32 { (0..3).map(|k| (p[k] - camera.ray_origin[k]) * camera.ray_forward[k]).sum() }
/// World units per pixel at `p` (along the camera's right vector).
pub fn world_per_px(camera: CameraInfo, rect: Rect, p: [f32; 3]) -> f32 {
    let r = camera.ray_right; let l = (r[0] * r[0] + r[1] * r[1] + r[2] * r[2]).sqrt().max(1e-9);
    let q = [0, 1, 2].map(|k| p[k] + r[k] / l);
    match (project(camera, rect, p), project(camera, rect, q)) {
        (Some(a), Some(b)) if a.distance(b) > 1e-6 => 1. / a.distance(b),
        _ => 0.01,
    }
}

/// The cube under the pointer. Overlapping cubes resolve to the one nearest the camera, then the nearest on screen.
pub fn hit_cube(camera: CameraInfo, rect: Rect, bounds: &Bounds, pointer: Pos2) -> Option<Cube> {
    Cube::ALL.into_iter().filter_map(|c| {
        let w = bounds.cube_at(c);
        let s = project(camera, rect, w)?;
        let d = s - pointer;
        (d.x.abs() <= HIT_PX && d.y.abs() <= HIT_PX).then(|| (eye_depth(camera, w), d.length(), c))
    }).min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1))).map(|x| x.2)
}

/// A mirror node's plane parameters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plane { pub origin: [f64; 3], pub normal: [f64; 3], pub offset: f64, pub reverse: bool }
impl Default for Plane { fn default() -> Self { Self { origin: [0.; 3], normal: [1., 0., 0.], offset: 0., reverse: false } } }
impl Plane {
    pub fn read(params: &Value) -> Self {
        let v3 = |n: &str, d: [f64; 3]| params.get(n).and_then(Value::as_array).filter(|a| a.len() == 3)
            .map(|a| [0, 1, 2].map(|k| a[k].as_f64().unwrap_or(d[k]))).unwrap_or(d);
        let d = Self::default();
        Self { origin: v3("origin", d.origin), normal: v3("normal", d.normal),
            offset: params.get("offset").and_then(Value::as_f64).unwrap_or(0.),
            reverse: params.get("reverse").and_then(Value::as_bool).unwrap_or(false) }
    }
    fn dir(&self) -> [f64; 3] { unit(self.normal).unwrap_or([1., 0., 0.]) }
    /// The plane the node cooks: origin moved by Offset, normal pointing at the kept side.
    pub fn effective(&self) -> ([f64; 3], [f64; 3]) {
        let d = self.dir();
        (add(self.origin, mul(d, self.offset)), if self.reverse { mul(d, -1.) } else { d })
    }
    /// The parameters that give effective plane (`o`, `n`), keeping Offset and Reverse.
    pub fn with_effective(&self, o: [f64; 3], n: [f64; 3]) -> Self {
        let n = unit(n).unwrap_or(self.effective().1);
        let d = if self.reverse { mul(n, -1.) } else { n };
        Self { origin: round(sub(o, mul(d, self.offset))), normal: round(d), ..*self }
    }
    pub fn params(&self) -> [(&'static str, Value); 4] {
        [("origin", json!(self.origin)), ("normal", json!(self.normal)), ("offset", json!(self.offset)), ("reverse", json!(self.reverse))]
    }
}

/// Snap a direction to 15-degree steps of azimuth and elevation (the axes are among them).
pub fn snap_direction(n: [f64; 3]) -> [f64; 3] {
    let Some(n) = unit(n) else { return [1., 0., 0.]; };
    let step = 15f64.to_radians();
    let el = (n[2].clamp(-1., 1.).asin() / step).round() * step;
    let az = (n[1].atan2(n[0]) / step).round() * step;
    round([el.cos() * az.cos(), el.cos() * az.sin(), el.sin()])
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle { Origin, Arrow, Ring }

/// Screen geometry of the plane overlay.
pub struct PlaneShape { pub corners: [[f32; 3]; 4], pub ring: Vec<[f32; 3]>, pub origin: [f32; 3], pub tip: [f32; 3] }
pub fn plane_shape(plane: &Plane, size: f32) -> PlaneShape {
    let (o, n) = plane.effective();
    let helper = if n[0].abs() < 0.9 { [1., 0., 0.] } else { [0., 1., 0.] };
    let u = unit(cross(n, helper)).unwrap_or([0., 1., 0.]);
    let v = cross(n, u);
    let s = size as f64;
    let at = |a: f64, b: f64| f32s(add(o, add(mul(u, a), mul(v, b))));
    PlaneShape {
        corners: [at(-s, -s), at(s, -s), at(s, s), at(-s, s)],
        ring: (0..RING_SEGMENTS).map(|i| { let t = i as f64 / RING_SEGMENTS as f64 * std::f64::consts::TAU; at(t.cos() * s * 0.8, t.sin() * s * 0.8) }).collect(),
        origin: f32s(o), tip: f32s(add(o, mul(n, s * 0.7))),
    }
}
fn segment_distance(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a; let t = ((p - a).dot(ab) / ab.length_sq().max(1e-8)).clamp(0., 1.);
    p.distance(a + ab * t)
}
/// The plane handle under the pointer: origin dot, then arrow tip/shaft, then ring.
pub fn hit_handle(camera: CameraInfo, rect: Rect, shape: &PlaneShape, pointer: Pos2) -> Option<Handle> {
    let o = project(camera, rect, shape.origin)?;
    if o.distance(pointer) <= HIT_PX { return Some(Handle::Origin); }
    if let Some(t) = project(camera, rect, shape.tip) {
        if segment_distance(pointer, o, t) <= HIT_PX * 0.6 || t.distance(pointer) <= HIT_PX { return Some(Handle::Arrow); }
    }
    let ring: Vec<_> = shape.ring.iter().filter_map(|&p| project(camera, rect, p)).collect();
    if ring.len() == shape.ring.len() && (0..ring.len()).any(|i| segment_distance(pointer, ring[i], ring[(i + 1) % ring.len()]) <= HIT_PX * 0.6) {
        return Some(Handle::Ring);
    }
    None
}

fn ray_plane(o: [f64; 3], d: [f64; 3], p: [f64; 3], n: [f64; 3]) -> Option<[f64; 3]> {
    let den = dot(d, n);
    (den.abs() > 1e-9).then(|| add(o, mul(d, dot(sub(p, o), n) / den)))
}
fn pointer_ray(camera: CameraInfo, rect: Rect, p: Pos2) -> ([f64; 3], [f64; 3]) { let (o, d) = ray(camera, rect, p); (f64s(o), f64s(d)) }
/// Where the pointer ray meets the camera-facing plane through `through`.
pub fn view_point(camera: CameraInfo, rect: Rect, p: Pos2, through: [f64; 3]) -> [f64; 3] {
    let (o, d) = pointer_ray(camera, rect, p);
    ray_plane(o, d, through, f64s(camera.ray_forward)).unwrap_or(through)
}

#[derive(Clone, Copy, Debug)]
pub struct Drag { pub handle: Handle, pub start: Pos2, pub plane: Plane, pub grab: [f64; 3] }

/// The plane for a handle drag from `drag.start` to `pointer`.
pub fn drag_plane(camera: CameraInfo, rect: Rect, drag: &Drag, size: f32, pointer: Pos2, shift: bool) -> Plane {
    let (o, n) = drag.plane.effective();
    match drag.handle {
        Handle::Origin => {
            let hit = view_point(camera, rect, pointer, o);
            drag.plane.with_effective(add(o, sub(hit, drag.grab)), n)
        }
        Handle::Arrow => {
            let tip = add(o, mul(n, size as f64 * 0.7));
            let (Some(a), Some(b)) = (project(camera, rect, f32s(o)), project(camera, rect, f32s(tip))) else { return drag.plane; };
            let axis = b - a;
            if axis.length() < 2. { return drag.plane; }
            let px = (pointer - drag.start).dot(axis.normalized());
            let t = px as f64 / axis.length() as f64 * size as f64 * 0.7 * if shift { 0.1 } else { 1. };
            drag.plane.with_effective(add(o, mul(n, t)), n)
        }
        Handle::Ring => {
            let (ro, rd) = pointer_ray(camera, rect, pointer);
            let r = size as f64 * 0.8;
            // Front intersection with the sphere through the ring; outside it, the closest point on the ray.
            let oc = sub(ro, o); let b = dot(oc, rd); let a = dot(rd, rd).max(1e-12);
            let disc = b * b - a * (dot(oc, oc) - r * r);
            let t = if disc >= 0. { (-b - disc.sqrt()) / a } else { -b / a };
            let Some(mut dir) = unit(sub(add(ro, mul(rd, t)), o)) else { return drag.plane; };
            // The ring lies in the plane: the grabbed ring point's direction maps to the normal.
            let grab = unit(sub(drag.grab, o)).unwrap_or(dir);
            let axis = cross(grab, dir);
            let s = dot(axis, axis).sqrt();
            if s > 1e-9 {
                let k = mul(axis, 1. / s); let c = dot(grab, dir); let angle = s.atan2(c);
                let (sn, cs) = angle.sin_cos();
                // Rodrigues rotation of the start normal by the grab-to-pointer rotation.
                dir = add(add(mul(n, cs), mul(cross(k, n), sn)), mul(k, dot(k, n) * (1. - cs)));
            } else { dir = n; }
            if shift { dir = snap_direction(dir); }
            drag.plane.with_effective(o, dir)
        }
    }
}

/// F + drag: the plane through the drawn start point whose direction is the drawn line. A click (no drag) on a face
/// uses the face normal instead. `start_hit` is the surface point and normal under the start, when there is one.
pub fn drawn_plane(camera: CameraInfo, rect: Rect, base: &Plane, center: [f32; 3], start: Pos2, end: Pos2,
    start_hit: Option<([f32; 3], [f32; 3])>) -> Option<Plane> {
    let anchor = start_hit.map(|h| f64s(h.0)).unwrap_or_else(|| view_point(camera, rect, start, f64s(center)));
    if start.distance(end) < 4. {
        let (p, n) = start_hit?;
        return Some(base.with_effective(f64s(p), f64s(n)));
    }
    let b = view_point(camera, rect, end, anchor);
    let n = unit(sub(b, anchor))?;
    Some(Plane { reverse: false, offset: 0., ..*base }.with_effective(anchor, n))
}

/// Parameters for a mirror node created by a cube click.
pub fn cube_params(cube: Cube, origin: [f32; 3], extra: &Value) -> Value {
    let mut p = json!({"origin": round(f64s(origin)), "normal": cube.direction(), "reverse": cube.reverse(), "cut": true});
    if let (Some(p), Some(e)) = (p.as_object_mut(), extra.as_object()) { for (k, v) in e { p.insert(k.clone(), v.clone()); } }
    p
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Click { Added(String), Flipped(String), Removed(String) }

/// Apply a cube click to the graph. `nodes` lists (axis, node id) of the mirror nodes this session controls.
pub fn click_axis(doc: &Document, nodes: &mut Vec<(usize, String)>, source: &str, cube: Cube, origin: [f32; 3], extra: &Value)
    -> Option<(Document, Click)> {
    if let Some(i) = nodes.iter().position(|n| n.0 == cube.axis) {
        let id = nodes[i].1.clone();
        let params = &doc.value()["nodes"][&id]["params"];
        if params.get("reverse").and_then(Value::as_bool).unwrap_or(false) == cube.reverse() {
            let v = minihou_app::graph_edit::dissolve_set(doc.value(), std::slice::from_ref(&id))?;
            let mut v = v;
            if let Some(all) = v.get_mut("nodes").and_then(Value::as_object_mut) { all.remove(&id); }
            let next = minihou_app::graph_edit::validated(v).ok()?;
            nodes.remove(i);
            return Some((next, Click::Removed(id)));
        }
        let mut next = doc.clone();
        next.edit(&Edit::Param { node: id.clone(), name: "reverse".into(), value: json!(cube.reverse()) }).ok()?;
        return Some((next, Click::Flipped(id)));
    }
    let after = nodes.last().map(|n| n.1.clone()).unwrap_or_else(|| source.to_owned());
    let (mut graph, id) = minihou_app::graph_edit::add(doc.value(), "mirror", Some(&after))?;
    graph["nodes"][&id]["params"] = cube_params(cube, origin, extra);
    let next = minihou_app::graph_edit::validated(graph).ok()?;
    nodes.push((cube.axis, id.clone()));
    Some((next, Click::Added(id)))
}

/// The main axis of a direction, used when adopting an existing mirror node.
pub fn dominant_axis(n: [f64; 3]) -> usize { (0..3).max_by(|&a, &b| n[a].abs().total_cmp(&n[b].abs())).unwrap_or(0) }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrawState { Off, Armed, Dragging(Pos2) }

/// Pointer and keyboard input for one frame.
#[derive(Clone, Debug, Default)]
pub struct Input {
    pub pointer: Option<Pos2>, pub pressed: bool, pub down: bool, pub released: bool, pub secondary: bool,
    pub keys: Vec<Key>, pub shift: bool, pub alt: bool, pub in_viewport: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Out { Confirm, Cancel, Axis { cube: Cube, world_origin: bool }, Plane(Plane) }

pub struct Session {
    pub before: Document,
    pub previous: String,
    pub source: String,
    pub nodes: Vec<(usize, String)>,
    pub active: Option<String>,
    pub bounds: Bounds,
    pub extra: Value,
    pub hover: Option<Cube>,
    pub handle_hover: Option<Handle>,
    pub drag: Option<Drag>,
    pub draw: DrawState,
    pub show_plane: bool,
    pub show_wire: bool,
    pub wire: Vec<([f32; 3], [f32; 3])>,
}
impl Session {
    pub fn new(before: Document, previous: String, source: String, bounds: Bounds, extra: Value) -> Self {
        Self { before, previous, source, nodes: Vec::new(), active: None, bounds, extra, hover: None, handle_hover: None,
            drag: None, draw: DrawState::Off, show_plane: true, show_wire: true, wire: Vec::new() }
    }
    pub fn size(&self) -> f32 { self.bounds.radius() * 1.15 }
    /// One frame of input. `plane` is the active node's current plane; `pick` returns the surface point and normal
    /// under a screen position.
    pub fn step(&mut self, input: &Input, plane: Option<Plane>, camera: CameraInfo, rect: Rect,
        pick: &dyn Fn(Pos2) -> Option<([f32; 3], [f32; 3])>) -> Vec<Out> {
        let mut out = Vec::new();
        for key in &input.keys {
            match key {
                Key::Escape => {
                    if let Some(d) = self.drag.take() { out.push(Out::Plane(d.plane)); }
                    else if self.draw != DrawState::Off { self.draw = DrawState::Off; }
                    else { return vec![Out::Cancel]; }
                }
                Key::Enter => return vec![Out::Confirm],
                Key::G => self.show_wire = !self.show_wire,
                Key::H => self.show_plane = !self.show_plane,
                Key::F => self.draw = DrawState::Armed,
                _ => {}
            }
        }
        if input.secondary {
            if let Some(d) = self.drag.take() { out.push(Out::Plane(d.plane)); return out; }
            if self.draw != DrawState::Off { self.draw = DrawState::Off; return out; }
            return vec![Out::Cancel];
        }
        let size = self.size();
        let shape = plane.filter(|_| self.show_plane).map(|p| plane_shape(&p, size));
        if let Some(pointer) = input.pointer {
            if let Some(drag) = self.drag {
                if input.down { out.push(Out::Plane(drag_plane(camera, rect, &drag, size, pointer, input.shift))); }
                if input.released || !input.down { self.drag = None; }
                return out;
            }
            if let DrawState::Dragging(start) = self.draw {
                if input.released || !input.down {
                    self.draw = DrawState::Off;
                    if let Some(p) = drawn_plane(camera, rect, &plane.unwrap_or_default(), self.bounds.center(), start, pointer, pick(start)) {
                        out.push(Out::Plane(p));
                    }
                }
                return out;
            }
            self.handle_hover = if input.in_viewport { shape.as_ref().and_then(|s| hit_handle(camera, rect, s, pointer)) } else { None };
            self.hover = if input.in_viewport && self.handle_hover.is_none() { hit_cube(camera, rect, &self.bounds, pointer) } else { None };
            if input.pressed && input.in_viewport && !input.alt {
                if self.draw == DrawState::Armed { self.draw = DrawState::Dragging(pointer); }
                else if let (Some(handle), Some(p)) = (self.handle_hover, plane) {
                    let (o, _) = p.effective();
                    let grab = match handle {
                        Handle::Ring => {
                            let s = shape.as_ref().unwrap();
                            let near = s.ring.iter().copied().min_by(|a, b| {
                                let d = |q: [f32; 3]| project(camera, rect, q).map_or(f32::MAX, |s| s.distance(pointer));
                                d(*a).total_cmp(&d(*b))
                            }).unwrap_or(s.origin);
                            f64s(near)
                        }
                        _ => view_point(camera, rect, pointer, o),
                    };
                    self.drag = Some(Drag { handle, start: pointer, plane: p, grab });
                } else if let Some(cube) = self.hover {
                    out.push(Out::Axis { cube, world_origin: input.shift });
                } else { out.push(Out::Confirm); }
            }
        }
        out
    }
    pub fn status(&self) -> String {
        let axes: Vec<String> = self.nodes.iter().map(|n| ["X", "Y", "Z"][n.0].to_string()).collect();
        let head = if axes.is_empty() { "Mirror".to_string() } else { format!("Mirror {}", axes.join(" + ")) };
        match self.draw {
            DrawState::Armed => format!("{head} | drag a line to draw the plane (click a face: its normal) | Esc back"),
            DrawState::Dragging(_) => format!("{head} | release to set the plane"),
            DrawState::Off => format!("{head} | click a cube: keep that side and mirror it (again: off, Shift: world origin) | drag dot/arrow/ring (Shift snaps) | F draw | G wire {} | H plane {} | Enter/click away confirm | Esc cancel",
                if self.show_wire { "on" } else { "off" }, if self.show_plane { "on" } else { "off" }),
        }
    }
}

fn cube_faces(center: [f32; 3], h: f32) -> [([f32; 3], [[f32; 3]; 4]); 6] {
    let c = |s: [f32; 3]| [0, 1, 2].map(|k| center[k] + s[k] * h);
    std::array::from_fn(|i| {
        let axis = i / 2; let sign = if i % 2 == 0 { 1. } else { -1. };
        let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
        let corner = |a: f32, b: f32| { let mut s = [0.; 3]; s[axis] = sign; s[u] = a; s[v] = b; c(s) };
        let mut n = [0.; 3]; n[axis] = sign;
        (n, [corner(-1., -1.), corner(1., -1.), corner(1., 1.), corner(-1., 1.)])
    })
}

/// Draw the wireframe, the plane and the axis cubes.
pub fn paint(ui: &egui::Ui, rect: Rect, camera: CameraInfo, s: &Session, plane: Option<Plane>) {
    let painter = ui.painter_at(rect);
    let p = |w: [f32; 3]| project(camera, rect, w);
    if s.show_wire {
        let stroke = Stroke::new(1_f32, Color32::from_rgba_unmultiplied(200, 200, 210, 70));
        for &(a, b) in &s.wire { if let (Some(a), Some(b)) = (p(a), p(b)) { painter.line_segment([a, b], stroke); } }
    }
    if let (Some(plane), true) = (plane, s.show_plane) {
        let shape = plane_shape(&plane, s.size());
        let gold = Color32::from_rgb(255, 196, 64);
        let hl = |h: Handle| if s.handle_hover == Some(h) || s.drag.is_some_and(|d| d.handle == h) { Color32::WHITE } else { gold };
        let quad: Vec<Pos2> = shape.corners.iter().filter_map(|&c| p(c)).collect();
        if quad.len() == 4 {
            painter.add(Shape::convex_polygon(quad, Color32::from_rgba_unmultiplied(255, 196, 64, 34), Stroke::new(1_f32, gold)));
        }
        let ring: Vec<Pos2> = shape.ring.iter().filter_map(|&c| p(c)).collect();
        if ring.len() == shape.ring.len() { painter.add(Shape::closed_line(ring, Stroke::new(2_f32, hl(Handle::Ring)))); }
        if let (Some(o), Some(t)) = (p(shape.origin), p(shape.tip)) {
            painter.line_segment([o, t], Stroke::new(2.5_f32, hl(Handle::Arrow)));
            let d = (t - o).normalized(); let n = egui::vec2(-d.y, d.x);
            painter.add(Shape::convex_polygon(vec![t + d * 8., t + n * 5., t - n * 5.], hl(Handle::Arrow), Stroke::NONE));
            painter.circle(o, 5_f32, hl(Handle::Origin), Stroke::new(1_f32, Color32::BLACK));
        }
    }
    if s.draw != DrawState::Off {
        if let (DrawState::Dragging(a), Some(b)) = (s.draw, ui.ctx().input(|i| i.pointer.hover_pos())) {
            painter.line_segment([a, b], Stroke::new(2_f32, Color32::from_rgb(255, 196, 64)));
        }
    }
    // Cubes back to front, visible faces only.
    let mut cubes: Vec<(f32, Cube)> = Cube::ALL.iter().map(|&c| (eye_depth(camera, s.bounds.cube_at(c)), c)).collect();
    cubes.sort_by(|a, b| b.0.total_cmp(&a.0));
    for (_, cube) in cubes {
        let w = s.bounds.cube_at(cube);
        let h = CUBE_PX * world_per_px(camera, rect, w);
        let used = s.nodes.iter().any(|n| n.0 == cube.axis);
        let base = cube.color();
        let col = if s.hover == Some(cube) { Color32::WHITE } else if used { base } else { base.gamma_multiply(0.75) };
        for (n, quad) in cube_faces(w, h) {
            let view = if camera.ortho { camera.ray_forward } else { [0, 1, 2].map(|k| w[k] - camera.ray_origin[k]) };
            if (0..3).map(|k| n[k] * view[k]).sum::<f32>() >= 0. { continue; }
            let shade = 0.6 + 0.4 * (n[1].abs() * 0.5 + n[cube.axis].abs() * 0.5);
            let pts: Vec<Pos2> = quad.iter().filter_map(|&c| p(c)).collect();
            if pts.len() == 4 { painter.add(Shape::convex_polygon(pts, col.gamma_multiply(shade), Stroke::new(1_f32, Color32::from_black_alpha(160)))); }
        }
        if s.hover == Some(cube) {
            if let Some(sp) = p(w) { painter.text(sp + egui::vec2(0., -CUBE_PX - 9.), egui::Align2::CENTER_CENTER, cube.label(), egui::FontId::proportional(12.), Color32::WHITE); }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{pos2, vec2};

    /// Ortho camera looking down -Z: world x/y map straight to the 200x200 viewport around its centre.
    fn ortho() -> CameraInfo {
        CameraInfo { ortho: true, projection: [[0.1, 0., 0., 0.], [0., 0.1, 0., 0.], [0., 0., 0.1, 0.], [0., 0., 0., 1.]],
            ray_origin: [0., 0., 10.], ray_forward: [0., 0., -1.], ray_right: [10., 0., 0.], ray_up: [0., 10., 0.], ..Default::default() }
    }
    fn rect() -> Rect { Rect::from_min_size(pos2(0., 0.), vec2(200., 200.)) }
    fn unit_box() -> Bounds { Bounds { min: [-1., -1., -1.], max: [1., 1., 1.] } }
    fn base_doc() -> Document {
        Document::parse(json!({"version":1,"nodes":{"box1":{"type":"box"},"out":{"type":"output","inputs":{"geo":"box1"}}},"display":"out"}).to_string()).unwrap()
    }

    #[test]
    fn mirror_gizmo_cubes_project_outside_the_bounds() {
        let b = Bounds { min: [2., 0., 0.], max: [4., 2., 2.] };
        assert_eq!(b.center(), [3., 1., 1.]);
        let px = b.cube_at(Cube { axis: 0, negative: false });
        assert!(px[0] > 4. && px[1] == 1. && px[2] == 1.);
        let nx = b.cube_at(Cube { axis: 0, negative: true });
        assert!(nx[0] < 2.);
        let s = project(ortho(), rect(), b.cube_at(Cube { axis: 1, negative: false })).unwrap();
        assert!(s.y < project(ortho(), rect(), [3., 1., 1.]).unwrap().y, "+Y is drawn above the centre");
    }

    #[test]
    fn mirror_gizmo_hover_picks_the_front_cube_and_maps_sides() {
        let (cam, r, b) = (ortho(), rect(), unit_box());
        // +Z and -Z overlap at the centre in a top view: the one nearer the camera (+Z) wins.
        let centre = project(cam, r, [0., 0., 0.]).unwrap();
        assert_eq!(hit_cube(cam, r, &b, centre), Some(Cube { axis: 2, negative: false }));
        let px = project(cam, r, b.cube_at(Cube { axis: 0, negative: false })).unwrap();
        assert_eq!(hit_cube(cam, r, &b, px + vec2(4., -3.)), Some(Cube { axis: 0, negative: false }));
        let nx = project(cam, r, b.cube_at(Cube { axis: 0, negative: true })).unwrap();
        assert_eq!(hit_cube(cam, r, &b, nx), Some(Cube { axis: 0, negative: true }));
        assert_eq!(hit_cube(cam, r, &b, px + vec2(HIT_PX + 2., 0.)), None);
        let p = cube_params(Cube { axis: 0, negative: true }, [1., 2., 3.], &json!({}));
        assert_eq!(p["normal"], json!([1., 0., 0.]));
        assert_eq!(p["reverse"], json!(true));
        assert_eq!(p["cut"], json!(true));
        assert_eq!(cube_params(Cube { axis: 1, negative: false }, [0.; 3], &json!({}))["reverse"], json!(false));
    }

    #[test]
    fn mirror_axis_clicks_chain_flip_and_remove_before_the_output() {
        let base = base_doc();
        let mut nodes = Vec::new();
        let (d1, c1) = click_axis(&base, &mut nodes, "out", Cube { axis: 0, negative: false }, [0.; 3], &json!({})).unwrap();
        let Click::Added(x) = c1 else { panic!() };
        assert_eq!(d1.value()["nodes"]["out"]["inputs"]["geo"], json!(x));
        assert_eq!(d1.value()["nodes"][&x]["inputs"]["geo"], json!("box1"));
        let (d2, c2) = click_axis(&d1, &mut nodes, "out", Cube { axis: 1, negative: false }, [0.; 3], &json!({})).unwrap();
        let Click::Added(y) = c2 else { panic!() };
        assert_eq!(d2.value()["nodes"][&y]["inputs"]["geo"], json!(x), "the Y mirror chains after X");
        assert_eq!(d2.value()["nodes"]["out"]["inputs"]["geo"], json!(y));
        let (d3, c3) = click_axis(&d2, &mut nodes, "out", Cube { axis: 0, negative: true }, [0.; 3], &json!({})).unwrap();
        assert_eq!(c3, Click::Flipped(x.clone()));
        assert_eq!(d3.value()["nodes"][&x]["params"]["reverse"], json!(true));
        let (d4, c4) = click_axis(&d3, &mut nodes, "out", Cube { axis: 0, negative: true }, [0.; 3], &json!({})).unwrap();
        assert_eq!(c4, Click::Removed(x.clone()));
        assert!(d4.value()["nodes"].get(&x).is_none());
        assert_eq!(d4.value()["nodes"][&y]["inputs"]["geo"], json!("box1"));
        assert_eq!(nodes, vec![(1, y)]);
    }

    #[test]
    fn mirror_session_cancel_restores_and_confirm_is_one_undo_step() {
        use minihou_app::History;
        let base = base_doc();
        let mut s = Session::new(base.clone(), "box1".into(), "out".into(), unit_box(), json!({"operation":"mirror_selected","group":"0"}));
        let (doc, _) = click_axis(&base, &mut s.nodes, "out", Cube { axis: 2, negative: false }, [0.; 3], &s.extra).unwrap();
        let id = s.nodes[0].1.clone();
        assert_eq!(doc.value()["nodes"][&id]["params"]["group"], json!("0"));
        let (mut doc, _) = click_axis(&doc, &mut s.nodes, "out", Cube { axis: 0, negative: false }, [0.; 3], &s.extra).unwrap();
        doc.edit(&Edit::Param { node: id, name: "offset".into(), value: json!(0.5) }).unwrap();
        // Cancel: the snapshot comes back untouched and nothing is recorded.
        let cancelled = s.before.clone();
        assert_eq!(cancelled.value(), base.value());
        // Confirm: one undo step back to the snapshot.
        let mut history = History::default();
        history.record(s.before.clone(), &doc);
        assert!(history.undo(&mut doc).is_some());
        assert_eq!(doc.value(), base.value());
        assert!(history.undo(&mut doc).is_none());
    }

    #[test]
    fn mirror_session_keys_and_clicks() {
        let (cam, r) = (ortho(), rect());
        let mut s = Session::new(base_doc(), String::new(), "out".into(), unit_box(), json!({}));
        let none = |_: Pos2| None;
        let key = |k| Input { keys: vec![k], ..Default::default() };
        assert!(s.step(&key(Key::G), None, cam, r, &none).is_empty()); assert!(!s.show_wire);
        s.step(&key(Key::H), None, cam, r, &none); assert!(!s.show_plane);
        s.step(&key(Key::H), None, cam, r, &none);
        s.step(&key(Key::F), None, cam, r, &none); assert_eq!(s.draw, DrawState::Armed);
        assert!(s.step(&key(Key::Escape), None, cam, r, &none).is_empty(), "Esc first leaves draw mode");
        assert_eq!(s.step(&key(Key::Escape), None, cam, r, &none), vec![Out::Cancel]);
        assert_eq!(s.step(&key(Key::Enter), None, cam, r, &none), vec![Out::Confirm]);
        let at = project(cam, r, unit_box().cube_at(Cube { axis: 0, negative: true })).unwrap();
        let click = |p: Pos2, shift| Input { pointer: Some(p), pressed: true, down: true, in_viewport: true, shift, ..Default::default() };
        assert_eq!(s.step(&click(at, true), None, cam, r, &none), vec![Out::Axis { cube: Cube { axis: 0, negative: true }, world_origin: true }]);
        assert_eq!(s.step(&click(pos2(5., 5.), false), None, cam, r, &none), vec![Out::Confirm], "click away confirms");
    }

    #[test]
    fn mirror_plane_drags_and_drawing() {
        let (cam, r) = (ortho(), rect());
        let plane = Plane { origin: [0.; 3], normal: [1., 0., 0.], offset: 0., reverse: false };
        let shape = plane_shape(&plane, 5.);
        let o = project(cam, r, shape.origin).unwrap();
        let tip = project(cam, r, shape.tip).unwrap();
        assert_eq!(hit_handle(cam, r, &shape, o), Some(Handle::Origin));
        assert_eq!(hit_handle(cam, r, &shape, tip), Some(Handle::Arrow));
        // Arrow: dragging one tip-length along it moves the origin by 3.5 (0.7 * size) along X.
        let d = Drag { handle: Handle::Arrow, start: tip, plane, grab: [0.; 3] };
        let moved = drag_plane(cam, r, &d, 5., tip + (tip - o), false);
        assert!((moved.origin[0] - 3.5).abs() < 1e-4 && moved.normal == [1., 0., 0.], "{moved:?}");
        // Origin: moves with the pointer in the view plane.
        let d = Drag { handle: Handle::Origin, start: o, plane, grab: [0.; 3] };
        let moved = drag_plane(cam, r, &d, 5., o + vec2(0., -10.), false);
        assert!((moved.origin[1] - 1.).abs() < 1e-4);
        // Drawing a vertical line gives a Y direction through the start.
        let p = drawn_plane(cam, r, &plane, [0.; 3], o, o + vec2(0., -30.), None).unwrap();
        assert!((p.normal[1] - 1.).abs() < 1e-6, "{p:?}");
        // A click on a face takes its normal.
        let p = drawn_plane(cam, r, &plane, [0.; 3], o, o, Some(([0., 0., 1.], [0., 0., 1.]))).unwrap();
        assert_eq!(p.normal, [0., 0., 1.]);
        assert!(drawn_plane(cam, r, &plane, [0.; 3], o, o, None).is_none());
    }

    #[test]
    fn mirror_reverse_offset_round_trip_and_snapping() {
        let p = Plane { origin: [1., 0., 0.], normal: [0., 2., 0.], offset: 0.5, reverse: true };
        let (o, n) = p.effective();
        assert_eq!(o, [1., 0.5, 0.]); assert_eq!(n, [0., -1., 0.]);
        let q = p.with_effective(o, n);
        assert_eq!(q.effective(), (o, n));
        assert_eq!(snap_direction([0.99, 0.05, 0.02]), [1., 0., 0.]);
        let s = snap_direction([1., 1., 0.]);
        assert!((s[0] - s[1]).abs() < 1e-6 && s[2] == 0.);
        assert_eq!(Plane::read(&json!({"origin":[1,2,3],"reverse":true})).origin, [1., 2., 3.]);
        assert_eq!(dominant_axis([0.1, -0.9, 0.2]), 1);
    }
}
