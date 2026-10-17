//! Locks, the working plane rule and typed (numeric) constraints.
use super::math::*;

pub const AXES: [V3; 3] = [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Lock {
    /// Through the start point along a world axis.
    Axis(usize),
    /// Along a captured direction (parallel / perpendicular / inferred) through `origin`
    /// (None = through the start point).
    Direction { origin: Option<V3>, dir: V3, label: &'static str },
    /// The resolved point stays on this plane.
    Plane { point: V3, normal: V3 },
}
impl Lock {
    pub fn label(&self) -> String { match self {
        Self::Axis(a) => format!("Locked {}", ["X", "Y", "Z"][*a]),
        Self::Direction { label, .. } => format!("Locked {label}"),
        Self::Plane { .. } => "Locked Plane".into(),
    } }
    pub fn line_dir(&self) -> Option<V3> { match self { Self::Axis(a) => Some(AXES[*a]), Self::Direction { dir, .. } => Some(*dir), Self::Plane { .. } => None } }
    pub fn tag(&self) -> u8 { match self { Self::Axis(a) => *a as u8, Self::Direction { .. } => 3, Self::Plane { .. } => 4 } }
}

/// Where the working plane came from (shown in the status text).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaneSource { Face, Ground, CameraAxis, Locked }
impl PlaneSource { pub fn label(self) -> &'static str { match self { Self::Face => "face plane", Self::Ground => "ground plane", Self::CameraAxis => "view axis plane", Self::Locked => "locked plane" } } }

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorkingPlane { pub point: V3, pub normal: V3, pub source: PlaneSource }
impl WorkingPlane {
    pub fn hit(&self, ray: Ray) -> Option<V3> {
        let den = dot(ray.1, self.normal);
        if den.abs() < 1e-6 { return None; }
        let t = dot(sub(self.point, ray.0), self.normal) / den;
        (t > 0.).then(|| add(ray.0, mul(ray.1, t)))
    }
    pub fn contains_dir(&self, d: V3) -> bool { dot(d, self.normal).abs() < 1e-3 * len(d).max(1e-12) }
    /// In-plane basis (u, v) used for typed angles: u is the world axis most aligned with the plane.
    pub fn basis(&self) -> (V3, V3) {
        // First (X before Y before Z) world axis with the largest in-plane component.
        let mut u = [1., 0., 0.]; let mut best = -1.;
        for a in AXES { let q = sub(a, mul(self.normal, dot(a, self.normal))); if len(q) > best + 1e-6 { best = len(q); u = q; } }
        let u = norm(u).unwrap_or([1., 0., 0.]);
        (u, cross(self.normal, u))
    }
}

/// Working plane rule (Phase 1):
/// 1. a held/locked plane, else
/// 2. the plane of the face under the start point (the face the stroke started on), else
/// 3. the tool's ground plane (the sketch chip's XY/XZ/YZ plane through the stroke origin)
///    unless it is seen edge-on (|cos| < 0.08), else
/// 4. the world axis plane through the start point that most faces the camera.
pub fn working_plane(locked: Option<(V3, V3)>, start_face: Option<(V3, V3)>, ground: (V3, V3), start: Option<V3>, ray: Ray) -> WorkingPlane {
    if let Some((p, n)) = locked { return WorkingPlane { point: p, normal: n, source: PlaneSource::Locked }; }
    if let Some((p, n)) = start_face { return WorkingPlane { point: p, normal: n, source: PlaneSource::Face }; }
    let (gp, gn) = ground;
    if dot(ray.1, gn).abs() >= 0.08 { return WorkingPlane { point: start.unwrap_or(gp), normal: gn, source: PlaneSource::Ground }; }
    let n = AXES.into_iter().max_by(|a, b| dot(*a, ray.1).abs().total_cmp(&dot(*b, ray.1).abs())).unwrap();
    WorkingPlane { point: start.unwrap_or(gp), normal: n, source: PlaneSource::CameraAxis }
}

/// A typed value from the measurement box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Typed { Length(f32), Absolute(V3), Relative(V3), Polar { length: Option<f32>, degrees: f32 } }

/// Parse `2.5m`, `x,y,z`, `@dx,dy,dz`, `len<deg` / `<deg` with the document units.
pub fn parse_typed(text: &str, units: crate::units::UnitSettings) -> Option<Typed> {
    let t = text.trim();
    if t.is_empty() { return None; }
    let len_of = |s: &str| crate::units::parse(s, units).map(|x| x as f32);
    if let Some((l, a)) = t.split_once('<') {
        let degrees: f32 = a.trim().parse().ok()?;
        let length = if l.trim().is_empty() { None } else { Some(len_of(l)?) };
        return Some(Typed::Polar { length, degrees });
    }
    let (rel, body) = match t.strip_prefix('@') { Some(r) => (true, r), None => (false, t) };
    if body.contains(',') {
        let v: Vec<f32> = body.split(',').map(len_of).collect::<Option<_>>()?;
        if v.len() != 3 { return None; }
        let p = [v[0], v[1], v[2]];
        return Some(if rel { Typed::Relative(p) } else { Typed::Absolute(p) });
    }
    if rel { return None; }
    len_of(body).map(Typed::Length)
}

/// Apply a typed value. Returns the point and an optional conflict message (never silently ignored).
pub fn apply_typed(typed: Typed, start: Option<V3>, current: V3, lock_dir: Option<V3>, plane: &WorkingPlane) -> (Option<V3>, Option<String>) {
    let on_lock = |p: V3, a: V3| lock_dir.is_none_or(|d| len(cross(sub(p, a), d)) <= 1e-4 * len(sub(p, a)).max(1.));
    match (typed, start) {
        (Typed::Absolute(p), s) => {
            let conflict = s.filter(|a| !on_lock(p, *a)).map(|_| "Typed point is off the locked direction: lock ignored".to_string());
            (Some(p), conflict)
        }
        (_, None) => (None, Some("Typed length needs a start point".into())),
        (Typed::Length(l), Some(a)) => {
            if l <= 0. { return (None, Some("Length must be positive".into())); }
            let raw = sub(current, a);
            let dir = match lock_dir { Some(d) => norm(d).map(|d| if dot(raw, d) < 0. { mul(d, -1.) } else { d }), None => norm(raw) };
            match dir { Some(d) => (Some(add(a, mul(d, l))), None), None => (None, Some("Move the cursor to give a direction".into())) }
        }
        (Typed::Relative(v), Some(a)) => {
            let p = add(a, v);
            (Some(p), (!on_lock(p, a)).then(|| "Typed offset is off the locked direction: lock ignored".into()))
        }
        (Typed::Polar { length, degrees }, Some(a)) => {
            let (u, v) = plane.basis();
            let r = degrees.to_radians();
            let d = add(mul(u, r.cos()), mul(v, r.sin()));
            let l = length.unwrap_or_else(|| dot(sub(current, a), d).abs().max(1e-6));
            let p = add(a, mul(d, l));
            (Some(p), (!on_lock(p, a)).then(|| "Typed angle conflicts with the locked direction: lock ignored".into()))
        }
    }
}
