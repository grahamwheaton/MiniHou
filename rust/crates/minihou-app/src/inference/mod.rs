//! Unified viewport inferencing engine (SketchUp-style), Phase 1. Pure and headless: tools give a
//! cursor ray + camera + tool context, the engine returns the resolved 3D point, the winning
//! inference, runner-ups for cycling, primitives to draw and a reason string.
//! Spec: docs/DESIGN_INFERENCING.md; architecture and API: docs/RUST_INFERENCE.md.
pub mod accel;
pub mod candidates;
pub mod constraints;
pub mod math;
pub mod presentation;
pub mod references;
pub mod scoring;
pub mod settings;
#[cfg(test)] mod tests;

pub use candidates::{Candidate, Guide, Kind};
pub use constraints::{Lock, PlaneSource, Typed, WorkingPlane};
pub use math::{Projector, Ray, V3};
pub use presentation::{Marker, Primitive};
pub use references::{RefDescriptor, Reference};
pub use settings::{cat, InferenceSettings, LinearMode};

use accel::{Accel, GeometryKey};
use candidates::{line_candidate, world_eps, Frame};
use constraints::AXES;
use math::*;

/// Borrowed evaluated geometry of the displayed node. `revision` is the host's geometry revision
/// (0 if it has none; pointer/length/sample checks still catch re-cooks).
#[derive(Clone, Copy, Debug)]
pub struct GeometryRef<'a> { pub positions: &'a [f32], pub triangles: &'a [u32], pub revision: u64 }

/// Degrees of freedom of a tool's next point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dof { /// Free 3D point (first click, Move base point).
    Point, /// From a start point (Line, Polyline, Measure second point, Move target).
    Line, /// Kept on the tool plane by the tool (Rectangle, Circle).
    Planar }

/// What a tool supports. Every tool goes through the same engine with its own spec.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ToolSpec { pub name: &'static str, pub categories: u32, pub dof: Dof }
impl ToolSpec {
    pub const LINE: Self = Self { name: "Line", categories: cat::ALL, dof: Dof::Line };
    pub const POLYLINE: Self = Self { name: "Polyline", categories: cat::ALL, dof: Dof::Line };
    pub const POLYGON: Self = Self { name: "Polygon", categories: cat::ALL, dof: Dof::Line };
    pub const RECTANGLE: Self = Self { name: "Rectangle", categories: cat::POINTS | cat::AXIS | cat::TRACKING, dof: Dof::Planar };
    pub const CIRCLE: Self = Self { name: "Circle", categories: cat::POINTS | cat::TRACKING, dof: Dof::Planar };
    pub const MEASURE: Self = Self { name: "Measure", categories: cat::ALL, dof: Dof::Line };
    pub const MOVE: Self = Self { name: "Move", categories: cat::ALL, dof: Dof::Line };
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolContext {
    pub spec: ToolSpec,
    /// Start point of the current segment (enables linear inferences).
    pub start: Option<V3>,
    /// Plane `(point, normal)` of the face the start point was snapped on.
    pub start_face: Option<(V3, V3)>,
    /// Ground plane `(point, normal)` (the sketch chip plane).
    pub ground: (V3, V3),
    /// The tool's own points (current stroke) as endpoint candidates.
    pub tool_points: Vec<V3>,
}
impl ToolContext {
    pub fn new(spec: ToolSpec) -> Self { Self { spec, start: None, start_face: None, ground: ([0.; 3], [0., 0., 1.]), tool_points: vec![] } }
}

pub struct InferenceInput<'a> {
    pub ray: Ray,
    pub cursor: [f32; 2],
    pub projector: Projector,
    /// Seconds (monotonic); drives dwell, tooltip delay.
    pub time: f64,
    pub geometry: Option<GeometryRef<'a>>,
    pub tool: ToolContext,
    pub settings: &'a InferenceSettings,
    /// Shift held: lock the current inference while held.
    pub shift_held: bool,
    pub typed: Option<Typed>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct InferenceResult {
    /// Resolved 3D point (None when the cursor ray misses every plane, e.g. looking at the horizon).
    pub point: Option<V3>,
    pub winner: Candidate,
    /// Ranked runner-ups (cycle with `cycle_candidates`).
    pub alternatives: Vec<Candidate>,
    pub primitives: Vec<Primitive>,
    pub reason: String,
    pub typed_status: Option<String>,
    pub plane: WorkingPlane,
    pub lock: Option<Lock>,
    /// Plane of the face under the resolved point (next segment's `start_face`).
    pub face: Option<(V3, V3)>,
    /// Static-snap reference metadata (vertex/edge/face/intersection) for the committed point.
    pub descriptor: Option<RefDescriptor>,
}

#[derive(Default)]
pub struct InferenceEngine {
    accel: Option<Accel>,
    pub tracker: references::Tracker,
    /// Persistent lock (arrow keys / Down).
    pub lock: Option<Lock>,
    held: Option<Lock>,
    prev_shift: bool,
    prev_key: Option<u64>,
    winner_since: f64,
    cycle: usize,
    cycle_anchor: [f32; 2],
    last: Option<InferenceResult>,
    last_start: Option<V3>,
    /// Most recent parallel/perpendicular seen in this stroke (Down arrow target).
    last_pp: Option<(V3, &'static str)>,
    linear_mode: Option<LinearMode>,
    pub builds: u32,
}

impl InferenceEngine {
    pub fn new() -> Self { Self::default() }
    pub fn linear_mode(&self, s: &InferenceSettings) -> LinearMode { self.linear_mode.unwrap_or(s.linear_mode) }
    pub fn cycle_linear_mode(&mut self, s: &InferenceSettings) -> LinearMode { let m = self.linear_mode(s).next(); self.linear_mode = Some(m); m }
    pub fn set_linear_mode(&mut self, m: LinearMode) { self.linear_mode = Some(m); }
    pub fn last(&self) -> Option<&InferenceResult> { self.last.as_ref() }
    pub fn active_lock(&self) -> Option<Lock> { self.held.or(self.lock) }
    pub fn references(&self) -> &[Reference] { &self.tracker.acquired }

    pub fn toggle_axis_lock(&mut self, axis: usize) {
        self.lock = if self.lock == Some(Lock::Axis(axis)) { None } else { Some(Lock::Axis(axis)) };
    }
    /// Down arrow: lock the current parallel/perpendicular, else the current inferred direction;
    /// pressing again (or with nothing inferred) releases the lock.
    pub fn lock_inferred(&mut self) {
        if matches!(self.lock, Some(Lock::Direction { .. })) { self.lock = None; return; }
        let from_winner = self.last.as_ref().and_then(|r| match r.winner.kind {
            Kind::Parallel | Kind::Perpendicular | Kind::Axis(_) | Kind::Extension | Kind::FromPoint(_) => r.winner.guides.first().map(|g| (g.dir, r.winner.kind.label())),
            _ => None,
        });
        let pp = self.last.as_ref().filter(|r| matches!(r.winner.kind, Kind::Parallel | Kind::Perpendicular)).and(from_winner).or(self.last_pp).or(from_winner);
        self.lock = match pp {
            Some((dir, label)) => Some(Lock::Direction { origin: None, dir, label: if label.starts_with("Along") { "Direction" } else { label } }),
            None => None,
        };
    }
    /// Tab: next overlapping candidate.
    pub fn cycle_candidates(&mut self) { self.cycle += 1; }
    /// Esc: clears typed-independent temporary state first. Returns true if something was cleared
    /// (the tool then keeps its operation); false means the tool should cancel.
    pub fn escape(&mut self) -> bool {
        let had = self.lock.take().is_some() | self.held.take().is_some() | self.tracker.clear() | self.tracker.transient_edge.take().is_some();
        self.cycle = 0; self.last_pp = None;
        had
    }
    /// A point was committed: per-segment state resets (references persist).
    pub fn segment_committed(&mut self) { self.lock = None; self.held = None; self.cycle = 0; self.last_pp = None; }
    /// Number of accelerator builds so far (cache test hook).
    pub fn accel_builds(&self) -> u32 { self.builds }

    fn ensure_accel(&mut self, g: GeometryRef) {
        let key = GeometryKey::of(g.revision, g.positions, g.triangles);
        if self.accel.as_ref().is_none_or(|a| a.key != key) {
            self.accel = Some(Accel::build(key, g.positions, g.triangles));
            self.builds += 1;
        }
    }

    fn wpp(input: &InferenceInput) -> (f32, f32) {
        let (o, d) = input.ray;
        let e = norm(cross(d, [0., 0., 1.])).or_else(|| norm(cross(d, [0., 1., 0.]))).unwrap_or([1., 0., 0.]);
        let sample = |t: f32| {
            let p = add(o, mul(d, t)); let delta = 1e-3 * t.max(1.);
            let (a, b) = (input.projector.project(p)?, input.projector.project(add(p, mul(e, delta)))?);
            let px = dist2(a, b);
            (px > 1e-6).then(|| delta / px)
        };
        match (sample(1.), sample(2.)) { (Some(w1), Some(w2)) => { let b = (w2 - w1).max(0.); ((w1 - b).max(0.), b) } _ => (1e-3, 1e-3) }
    }

    pub fn update(&mut self, input: &InferenceInput) -> InferenceResult {
        let s = input.settings;
        let mode = self.linear_mode(s);
        let start = input.tool.start;
        if self.last_start != start { self.last_start = start; self.last_pp = None; }
        let enabled = s.enabled & input.tool.spec.categories;
        // Shift: rising edge captures the current inference, release unlocks.
        if input.shift_held && !self.prev_shift { self.held = self.last.as_ref().and_then(|r| Self::capture(r, start)); }
        if !input.shift_held { self.held = None; }
        self.prev_shift = input.shift_held;
        let lock = self.held.or(self.lock);
        let geometry = input.geometry;
        if let Some(g) = geometry { self.ensure_accel(g); }
        let accel_store = if geometry.is_some() { self.accel.take() } else { None };
        let accel = accel_store.as_ref();
        let wpp = Self::wpp(input);
        let hit = accel.and_then(|a| a.ray_hit(input.ray));
        let scale = accel.map_or(1., |a| a.scale);
        let max_depth = match hit { Some((_, t, _)) if !s.include_hidden => t + 1e-3 * t + 10. * world_eps(scale), _ => f32::INFINITY };
        let frame = Frame { ray: input.ray, cursor: input.cursor, projector: &input.projector, tolerance: s.tolerance_px, wpp, max_depth, enabled, include_hidden: s.include_hidden };
        let plane_lock = match lock { Some(Lock::Plane { point, normal }) => Some((point, normal)), _ => None };
        let plane = constraints::working_plane(plane_lock, input.tool.start_face, input.tool.ground, start, input.ray);
        let hit_face = hit.and_then(|(tri, t, bary)| accel.map(|a| {
            let tr = a.tris[tri as usize]; let p = [a.p(tr[0]), a.p(tr[1]), a.p(tr[2])];
            let n = norm(cross(sub(p[1], p[0]), sub(p[2], p[0]))).unwrap_or([0., 0., 1.]);
            (add(input.ray.0, mul(input.ray.1, t)), n, tr, bary)
        }));

        // ---- candidates
        let mut cands: Vec<Candidate> = Vec::new();
        if let Some(a) = accel { candidates::point_candidates(&frame, a, &mut cands); }
        for &p in &input.tool.tool_points {
            if let Some((d, depth)) = frame.screen(p) { if d <= frame.tolerance { cands.push(Candidate::new(Kind::Endpoint, p, d, depth)); } }
        }
        if enabled & cat::ORIGIN != 0 { if let Some((d, depth)) = frame.screen([0.; 3]) { if d <= frame.tolerance { let mut c = Candidate::new(Kind::Origin, [0.; 3], d, depth); c.descriptor = Some(RefDescriptor::Origin); cands.push(c); } } }
        if let (Some((p, n, tr, bary)), true) = (hit_face, enabled & cat::FACE != 0) {
            let mut c = Candidate::new(Kind::OnFace, p, 0., dot(sub(p, input.ray.0), input.ray.1));
            c.descriptor = Some(RefDescriptor::Face { points: tr, weights: bary });
            c.cause = None; c.edge = None;
            let _ = n;
            cands.push(c);
        }
        let linear_ok = |k: u32| enabled & k != 0 && match mode { LinearMode::All => true, LinearMode::PointsOnly => false, LinearMode::ParallelPerpendicularOnly => k & (cat::PARALLEL | cat::PERPENDICULAR) != 0 };
        let mut start_lines: Vec<Guide> = Vec::new();
        let mut ref_lines: Vec<(Guide, Option<(V3, V3)>)> = Vec::new();
        if let Some(a) = start {
            let in_plane = |d: V3| plane.source != PlaneSource::Face || plane.contains_dir(d);
            if linear_ok(cat::AXIS) { for (i, d) in AXES.into_iter().enumerate() { if in_plane(d) { start_lines.push(Guide { origin: a, dir: d, kind: Kind::Axis(i as u8) }); } } }
            if let Some((ea, eb)) = self.tracker.transient_edge {
                let ed = sub(eb, ea);
                if let Some(ed) = norm(ed) {
                    let axis_like = AXES.iter().any(|x| dot(*x, ed).abs() > 0.9999);
                    if linear_ok(cat::PARALLEL) && in_plane(ed) && !(axis_like && linear_ok(cat::AXIS)) { start_lines.push(Guide { origin: a, dir: ed, kind: Kind::Parallel }); }
                    // Perpendicular in the working plane: perpendicular to the edge's in-plane projection.
                    let proj = sub(ed, mul(plane.normal, dot(ed, plane.normal)));
                    if let (true, Some(pd)) = (linear_ok(cat::PERPENDICULAR), norm(cross(plane.normal, proj)).filter(|_| len(proj) > 1e-3)) {
                        let axis_like = AXES.iter().any(|x| dot(*x, pd).abs() > 0.9999);
                        if !(axis_like && linear_ok(cat::AXIS)) { start_lines.push(Guide { origin: a, dir: pd, kind: Kind::Perpendicular }); }
                    }
                }
            }
        }
        if linear_ok(cat::TRACKING) {
            for r in &self.tracker.acquired {
                if let Reference::Point { point, .. } = r {
                    if start.is_some_and(|a| dist(a, *point) < 1e-7) { continue; }
                    for (i, d) in AXES.into_iter().enumerate() { ref_lines.push((Guide { origin: *point, dir: d, kind: Kind::FromPoint(i as u8) }, None)); }
                }
            }
        }
        if linear_ok(cat::EXTENSION) {
            let mut edges: Vec<(V3, V3)> = self.tracker.acquired.iter().filter_map(|r| match r { Reference::Edge { a, b, .. } => Some((*a, *b)), _ => None }).collect();
            if let Some(e) = self.tracker.transient_edge { if !edges.iter().any(|x| *x == e) { edges.push(e); } }
            for (ea, eb) in edges { ref_lines.push((Guide { origin: ea, dir: sub(eb, ea), kind: Kind::Extension }, Some((ea, eb)))); }
        }
        for g in &start_lines {
            let n = cands.len();
            line_candidate(&frame, *g, None, &mut cands);
            if let (Some(c), Kind::Parallel | Kind::Perpendicular) = (cands.get_mut(n), g.kind) { c.cause = self.tracker.transient_edge; }
        }
        for (g, edge) in &ref_lines {
            if g.kind == Kind::Extension {
                // Only beyond the endpoints (inside the edge it is On Edge).
                if let Some((t, _)) = line_line(g.origin, g.dir, input.ray.0, input.ray.1) {
                    if t < 0. || t > 1. {
                        let origin = if t < 0. { g.origin } else { add(g.origin, g.dir) };
                        let n = cands.len();
                        line_candidate(&frame, Guide { origin, dir: g.dir, kind: Kind::Extension }, None, &mut cands);
                        if let Some(c) = cands.get_mut(n) { c.cause = *edge; }
                    }
                }
            } else { line_candidate(&frame, *g, None, &mut cands); }
        }
        // Points where two inference lines really meet in 3D (start x reference, reference x reference).
        let eps = world_eps(scale);
        let lines: Vec<Guide> = start_lines.iter().copied().chain(ref_lines.iter().map(|x| x.0)).collect();
        for i in 0..lines.len() { for j in i + 1..lines.len() {
            let (g, h) = (lines[i], lines[j]);
            if i < start_lines.len() && j < start_lines.len() { continue; }
            if dist(g.origin, h.origin) < 1e-7 { continue; }
            let Some((s0, s1)) = line_line(g.origin, g.dir, h.origin, h.dir) else { continue; };
            let (p, q) = (add(g.origin, mul(g.dir, s0)), add(h.origin, mul(h.dir, s1)));
            if dist(p, q) > eps { continue; }
            if let Some((d, depth)) = frame.screen(p) { if d <= frame.tolerance {
                let mut c = Candidate::new(Kind::AlignIntersection, p, d, depth); c.guides = vec![g, h]; cands.push(c);
            } }
        } }

        // ---- ranking, hysteresis, cycling
        if let Some((_, n)) = plane_lock { cands.retain(|c| dot(sub(c.point, plane.point), n).abs() <= 1e-4 * scale.max(1.)); }
        let ranked = scoring::rank(cands);
        if dist2(input.cursor, self.cycle_anchor) > s.tolerance_px { self.cycle = 0; self.cycle_anchor = input.cursor; }
        let base = scoring::hysteresis(&ranked, self.prev_key, s.hysteresis_px, s.hysteresis_ratio);
        let chosen = if ranked.is_empty() { None } else { Some((base + self.cycle) % ranked.len()) };
        let free_point = plane.hit(input.ray).or_else(|| start.map(|a| add(input.ray.0, mul(input.ray.1, dot(sub(a, input.ray.0), input.ray.1).max(0.)))));
        let mut winner = chosen.map(|i| ranked[i].clone()).unwrap_or_else(|| Candidate::new(Kind::Free, free_point.unwrap_or([0.; 3]), 0., 0.));
        let new_key = chosen.map(|i| scoring::key(&ranked[i]));
        if new_key != self.prev_key { self.winner_since = input.time; }
        self.prev_key = new_key;
        if matches!(winner.kind, Kind::Parallel | Kind::Perpendicular) { if let Some(g) = winner.guides.first() { self.last_pp = Some((g.dir, winner.kind.label())); } }

        // ---- hover-to-reference (dwell) and transient edge
        let acquirable = match winner.kind {
            Kind::Endpoint | Kind::Midpoint | Kind::Intersection | Kind::Origin => Some(Reference::Point { point: winner.point, label: winner.kind.label(), descriptor: winner.descriptor.clone() }),
            Kind::OnEdge => winner.edge.map(|(a, b)| Reference::Edge { a, b, descriptor: None }),
            _ => None,
        };
        if enabled & cat::TRACKING != 0 { self.tracker.hover(acquirable, input.time, s.dwell_s, s.max_references); }
        if let (Some(e), Kind::OnEdge | Kind::Midpoint | Kind::Endpoint) = (winner.edge, winner.kind) { self.tracker.transient_edge = Some(e); }

        // ---- locks (outrank everything)
        let mut reason = String::new();
        let mut point = if winner.kind == Kind::Free { free_point } else { Some(winner.point) };
        let line_lock = lock.and_then(|l| match l {
            Lock::Axis(a) => start.map(|o| (o, AXES[a])),
            Lock::Direction { origin, dir, .. } => origin.or(start).map(|o| (o, dir)),
            Lock::Plane { .. } => None,
        });
        if let (Some(l), Some((o, d))) = (lock, line_lock) {
            let snapped = ranked.iter().find(|c| c.kind.is_exact_point());
            let p = match snapped {
                Some(c) => { reason = format!("{} from {}", l.label(), c.kind.label()); add(o, mul(d, dot(sub(c.point, o), d) / dot(d, d))) }
                None => { reason = l.label(); match line_line(o, d, input.ray.0, input.ray.1) { Some((t, _)) => add(o, mul(d, t)), None => o } }
            };
            let mut c = Candidate::new(Kind::Locked(l.tag()), p, 0., dot(sub(p, input.ray.0), input.ray.1));
            c.guides.push(Guide { origin: o, dir: d, kind: Kind::Locked(l.tag()) });
            if let Some(sc) = snapped { c.descriptor = None; c.cause = Some((p, sc.point)); }
            winner = c; point = Some(p);
        } else if let Some(l) = lock {
            if line_lock.is_none() && !matches!(l, Lock::Plane { .. }) { reason = format!("{} needs a start point", l.label()); }
            if matches!(l, Lock::Plane { .. }) { if winner.kind == Kind::Free { winner.kind = Kind::Locked(4); } reason = l.label(); }
        }
        if reason.is_empty() {
            reason = match winner.kind {
                Kind::Free => format!("On {}", plane.source.label()),
                k => format!("{}{}", k.label(), if ranked.len() > 1 { format!(" ({} more, Tab cycles)", ranked.len() - 1) } else { String::new() }),
            };
        }
        // Keep linear inferences from a face start on the face plane.
        // ---- typed values
        let mut typed_status = None;
        if let (Some(t), Some(cur)) = (input.typed, point) {
            let lock_dir = line_lock.filter(|(o, _)| start.is_some_and(|a| dist(*o, a) < 1e-7)).map(|(_, d)| d);
            let (p, msg) = constraints::apply_typed(t, start, cur, lock_dir, &plane);
            if let Some(p) = p { point = Some(p); winner.point = p; }
            typed_status = msg;
        }
        let face = hit_face.and_then(|(fp, n, _, _)| point.filter(|p| dot(sub(*p, fp), n).abs() <= 1e-4 * scale.max(1.)).map(|_| (fp, n)))
            .or_else(|| if plane.source == PlaneSource::Face && point.is_some_and(|p| dot(sub(p, plane.point), plane.normal).abs() <= 1e-4 * scale.max(1.)) { Some((plane.point, plane.normal)) } else { None });
        let tooltip = input.time - self.winner_since >= s.tooltip_delay_s;
        let primitives = presentation::build(&winner, start, &self.tracker.acquired, lock.is_some(), tooltip, s);
        let descriptor = if matches!(winner.kind, Kind::Locked(_)) || input.typed.is_some() { None } else { winner.descriptor.clone() };
        let alternatives = ranked.into_iter().take(8).collect();
        let result = InferenceResult { point, winner, alternatives, primitives, reason, typed_status, plane, lock, face, descriptor };
        self.last = Some(result.clone());
        if accel_store.is_some() { self.accel = accel_store; }
        result
    }

    /// Shift-lock capture from the last result.
    fn capture(r: &InferenceResult, start: Option<V3>) -> Option<Lock> {
        match r.winner.kind {
            Kind::Axis(a) => Some(Lock::Axis(a as usize)),
            Kind::Parallel | Kind::Perpendicular | Kind::Extension | Kind::FromPoint(_) => r.winner.guides.first().map(|g| Lock::Direction {
                origin: if matches!(r.winner.kind, Kind::Parallel | Kind::Perpendicular) { None } else { Some(g.origin) }, dir: g.dir, label: r.winner.kind.label() }),
            Kind::OnFace => r.face.map(|(point, normal)| Lock::Plane { point, normal }),
            Kind::Endpoint | Kind::Midpoint | Kind::Intersection | Kind::Origin | Kind::OnEdge => start.and_then(|a| norm(sub(r.winner.point, a)))
                .map(|dir| Lock::Direction { origin: None, dir, label: "Direction" }),
            _ => r.lock,
        }
    }
}
