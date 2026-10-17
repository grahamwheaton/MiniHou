//! Candidate generation: point snaps from the accelerated scene and linear (directional) inferences.
use super::accel::Accel;
use super::math::*;
use super::references::RefDescriptor;
use super::settings::cat;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Endpoint, Midpoint, Centre, Intersection, Origin, Reference,
    OnEdge, OnFace, Extension,
    Axis(u8), Parallel, Perpendicular, FromPoint(u8), AlignIntersection,
    /// Resolved on an explicit lock (axis index 0-2, 3 = inferred direction, 4 = plane).
    Locked(u8),
    Free,
}
impl Kind {
    pub fn label(self) -> &'static str { match self {
        Self::Endpoint => "Endpoint", Self::Midpoint => "Midpoint", Self::Centre => "Curve Centre", Self::Intersection => "Intersection", Self::Origin => "Origin",
        Self::Reference => "Reference Point", Self::OnEdge => "On Edge", Self::OnFace => "On Face", Self::Extension => "Extension",
        Self::Axis(0) => "Along X", Self::Axis(1) => "Along Y", Self::Axis(_) => "Along Z",
        Self::Parallel => "Parallel", Self::Perpendicular => "Perpendicular",
        Self::FromPoint(0) => "From Point (X)", Self::FromPoint(1) => "From Point (Y)", Self::FromPoint(_) => "From Point (Z)",
        Self::AlignIntersection => "From Points",
        Self::Locked(0) => "Locked X", Self::Locked(1) => "Locked Y", Self::Locked(2) => "Locked Z", Self::Locked(3) => "Locked Direction", Self::Locked(_) => "Locked Plane",
        Self::Free => "",
    } }
    /// Ranking tier (spec section 9, lower wins). Face is ranked under the directional inferences:
    /// it is area-based (always under the cursor over a surface), so ranking it above them would
    /// make axis/parallel inference impossible over any surface. Directions on a face plane keep
    /// the point on the face (see constraints::working_plane).
    pub fn tier(self) -> u8 { match self {
        Self::Locked(_) => 0,
        Self::Endpoint | Self::Midpoint | Self::Centre | Self::Origin | Self::Reference => 1,
        Self::Intersection | Self::AlignIntersection | Self::FromPoint(_) => 2,
        Self::OnEdge | Self::Extension => 3,
        Self::Axis(_) | Self::Parallel | Self::Perpendicular => 4,
        Self::OnFace => 5,
        Self::Free => 6,
    } }
    pub fn is_exact_point(self) -> bool { self.tier() <= 2 && !matches!(self, Self::Locked(_) | Self::FromPoint(_)) }
    pub fn is_linear(self) -> bool { matches!(self, Self::Axis(_) | Self::Parallel | Self::Perpendicular | Self::FromPoint(_) | Self::Extension | Self::AlignIntersection) }
}

/// A guide line explaining a linear inference: `origin + t*dir`, drawn from `origin` to the point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Guide { pub origin: V3, pub dir: V3, pub kind: Kind }

#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    pub kind: Kind,
    pub point: V3,
    /// Screen distance to the cursor, pixels.
    pub distance: f32,
    pub depth: f32,
    pub guides: Vec<Guide>,
    /// Geometry causing a parallel/perpendicular/extension inference (highlighted).
    pub cause: Option<(V3, V3)>,
    pub descriptor: Option<RefDescriptor>,
    /// Edge under a point/edge snap (becomes the transient direction reference).
    pub edge: Option<(V3, V3)>,
}
impl Candidate {
    pub fn new(kind: Kind, point: V3, distance: f32, depth: f32) -> Self { Self { kind, point, distance, depth, guides: vec![], cause: None, descriptor: None, edge: None } }
}

pub struct Frame<'a> {
    pub ray: Ray,
    pub cursor: [f32; 2],
    pub projector: &'a Projector,
    pub tolerance: f32,
    /// World units per pixel at ray depth t: `a + b*t`.
    pub wpp: (f32, f32),
    pub max_depth: f32,
    pub enabled: u32,
    pub include_hidden: bool,
}
impl Frame<'_> {
    pub fn screen(&self, p: V3) -> Option<(f32, f32)> {
        let depth = dot(sub(p, self.ray.0), self.ray.1);
        if depth <= 0. { return None; }
        self.projector.project(p).map(|s| (dist2(s, self.cursor), depth))
    }
    fn push(&self, out: &mut Vec<Candidate>, kind: Kind, p: V3, f: impl FnOnce(&mut Candidate)) {
        if let Some((d, depth)) = self.screen(p) {
            if d <= self.tolerance && depth <= self.max_depth && d.is_finite() { let mut c = Candidate::new(kind, p, d, depth); f(&mut c); out.push(c); }
        }
    }
}

/// Numeric (world) tolerance for "these two 3D lines really meet", separate from pixel tolerances.
pub fn world_eps(scale: f32) -> f32 { 1e-5 * scale.max(1.) }

/// Vertex / midpoint / on-edge / true 3D edge intersection candidates near the cursor.
pub fn point_candidates(frame: &Frame, accel: &Accel, out: &mut Vec<Candidate>) {
    if frame.enabled & cat::CENTRE != 0 { for &p in &accel.centers { frame.push(out, Kind::Centre, p, |_| {}); } }
    let mut near: Vec<(f32, u32)> = Vec::new();
    let (a, b) = frame.wpp;
    let pad = frame.tolerance * 1.5;
    accel.edges_near(frame.ray, a * pad, b * pad, frame.max_depth, |ei| {
        let e = accel.edges[ei as usize];
        if e.hidden && !frame.include_hidden { return; }
        let (pa, pb) = (accel.p(e.a), accel.p(e.b));
        if frame.enabled & cat::VERTEX != 0 {
            for (v, p) in [(e.a, pa), (e.b, pb)] { frame.push(out, Kind::Endpoint, p, |c| { c.descriptor = Some(RefDescriptor::Vertex { point: v }); c.edge = (!e.degenerate).then_some((pa, pb)); }); }
        }
        if e.degenerate { return; }
        if frame.enabled & cat::MIDPOINT != 0 {
            frame.push(out, Kind::Midpoint, accel.midpoints[ei as usize], |c| { c.descriptor = Some(RefDescriptor::Edge { a: e.a, b: e.b, t: 0.5 }); c.edge = Some((pa, pb)); });
        }
        let (s, _) = segment_ray(pa, pb, frame.ray);
        let p = lerp(pa, pb, s);
        if let Some((d, _)) = frame.screen(p) { if d <= frame.tolerance { near.push((d, ei)); } }
        if frame.enabled & cat::EDGE != 0 {
            frame.push(out, Kind::OnEdge, p, |c| { c.descriptor = Some(RefDescriptor::Edge { a: e.a, b: e.b, t: s }); c.edge = Some((pa, pb)); });
        }
    });
    if frame.enabled & cat::INTERSECTION == 0 { return; }
    near.sort_by(|x, y| x.0.total_cmp(&y.0));
    near.truncate(48);
    let eps = world_eps(accel.scale);
    for i in 0..near.len() { for j in i + 1..near.len() {
        let (e0, e1) = (accel.edges[near[i].1 as usize], accel.edges[near[j].1 as usize]);
        if [e0.a, e0.b].iter().any(|v| *v == e1.a || *v == e1.b) { continue; }
        let (a0, b0, a1, b1) = (accel.p(e0.a), accel.p(e0.b), accel.p(e1.a), accel.p(e1.b));
        let Some((s, t)) = line_line(a0, sub(b0, a0), a1, sub(b1, a1)) else { continue; };
        let te = 1e-4;
        if !(-te..=1. + te).contains(&s) || !(-te..=1. + te).contains(&t) { continue; }
        let (p, q) = (lerp(a0, b0, s), lerp(a1, b1, t));
        // True 3D intersection only: projection-only overlaps are rejected here.
        if dist(p, q) > eps { continue; }
        frame.push(out, Kind::Intersection, lerp(p, q, 0.5), |c| { c.descriptor = Some(RefDescriptor::Intersection { edges: [[e0.a, e0.b], [e1.a, e1.b]], t: [s, t] }); });
    } }
}

/// Point on `origin + t*dir` closest to the cursor ray (t >= min_t), as a candidate.
pub fn line_candidate(frame: &Frame, guide: Guide, min_t: Option<f32>, out: &mut Vec<Candidate>) -> Option<V3> {
    let (s, _) = line_line(guide.origin, guide.dir, frame.ray.0, frame.ray.1)?;
    if min_t.is_some_and(|m| s < m) { return None; }
    let p = add(guide.origin, mul(guide.dir, s));
    if dist(p, guide.origin) < 1e-7 { return None; }
    let n = out.len();
    frame.push(out, guide.kind, p, |c| c.guides.push(guide));
    (out.len() > n).then_some(p)
}
