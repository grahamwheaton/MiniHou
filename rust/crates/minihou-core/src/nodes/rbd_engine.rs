//! Rigid-body worlds and pose caches for `rbd_solver`, ported from `minihou/rbd.py`.
//!
//! What is a straight port: input validation (`pieces`, `convex_part`, `link_table`, `validate_link_values`), the
//! world/frame semantics (integer frames with fixed substeps, fractional frames hold the preceding integer frame,
//! going back restarts from the initial state, never from a snapshot; byte-bounded pose cache; at most 8 resident
//! worlds), velocity damping, the 0.1 s settle time before links may break, legacy glue's one-break-per-substep rule,
//! contact exclusion between linked pairs until all their non-colliding links break, and the output geometry
//! (transformed points and normals, `rbd_v` / `rbd_w`, colliders merged, detail `rbd`).
//!
//! What is not: Python delegates the dynamics to MuJoCo 3.15.0, an optional dependency that is not in the Rust build.
//! The dynamics here are a native sequential-impulse solver: convex-convex contacts from SAT with face clipping
//! (`margin` is the contact distance), Coulomb friction (pair friction = max), restitution (pair = mean, applied above a
//! small approach speed), split position correction, and joints as velocity constraints (weld for glue, ball for pin,
//! ball + two angular rows + limit + damping for hinge, explicit Hooke spring + length stops for spring). Masses and
//! inertias are those of the closed (convex) part meshes at uniform density, like MuJoCo's `inertia="convex"`. Results
//! therefore behave like the Python solver (bodies fall, rest, stack, tumble, links hold and break) but are not
//! bit-identical to MuJoCo. `iterations` is the velocity iteration count per substep.

use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use super::rbd::{validate_link_values, LinkValues, CONSTRAINT_KINDS, RBD_SOLVER};
use super::sim_common::{float, int, vec3};
use super::Inputs;
use crate::error::{Error, Result};
use crate::geometry::{Attr, AttrData, Geometry};
use crate::params::Params;
use crate::timectx;

pub(super) const BACKEND: &str = "minihou-native";
pub(super) const BACKEND_VERSION: &str = "1";
/// Seconds after a (re)start in which links can not break: touching pieces settle first.
const SETTLE_TIME: f64 = 0.1;
const MAX_WORLDS: usize = 8;

type V3 = [f64; 3];
type M3 = [[f64; 3]; 3];

// ------------------------------------------------------------------------------------------------------------- math

fn add(a: V3, b: V3) -> V3 { [a[0] + b[0], a[1] + b[1], a[2] + b[2]] }
fn sub(a: V3, b: V3) -> V3 { [a[0] - b[0], a[1] - b[1], a[2] - b[2]] }
fn mul(a: V3, s: f64) -> V3 { [a[0] * s, a[1] * s, a[2] * s] }
fn dot(a: V3, b: V3) -> f64 { a[0] * b[0] + a[1] * b[1] + a[2] * b[2] }
fn cross(a: V3, b: V3) -> V3 { [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]] }
fn norm(a: V3) -> f64 { dot(a, a).sqrt() }
fn unit(a: V3) -> V3 { let n = norm(a); if n > 1e-300 { mul(a, 1.0 / n) } else { [0.0; 3] } }
fn mv(m: &M3, v: V3) -> V3 { [dot(m[0], v), dot(m[1], v), dot(m[2], v)] }
fn mt(m: &M3) -> M3 { [[m[0][0], m[1][0], m[2][0]], [m[0][1], m[1][1], m[2][1]], [m[0][2], m[1][2], m[2][2]]] }
fn mm(a: &M3, b: &M3) -> M3 {
    let bt = mt(b);
    [[dot(a[0], bt[0]), dot(a[0], bt[1]), dot(a[0], bt[2])],
     [dot(a[1], bt[0]), dot(a[1], bt[1]), dot(a[1], bt[2])],
     [dot(a[2], bt[0]), dot(a[2], bt[1]), dot(a[2], bt[2])]]
}
fn madd(a: &M3, b: &M3) -> M3 { std::array::from_fn(|i| add(a[i], b[i])) }
fn mscale(a: &M3, s: f64) -> M3 { std::array::from_fn(|i| mul(a[i], s)) }
const IDENTITY: M3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
const ZERO_M: M3 = [[0.0; 3]; 3];
fn skew(a: V3) -> M3 { [[0.0, -a[2], a[1]], [a[2], 0.0, -a[0]], [-a[1], a[0], 0.0]] }
fn inverse(m: &M3) -> M3 {
    let c0 = cross(m[1], m[2]);
    let det = dot(m[0], c0);
    if det.abs() < 1e-300 {
        return ZERO_M;
    }
    let c1 = cross(m[2], m[0]);
    let c2 = cross(m[0], m[1]);
    mscale(&mt(&[c0, c1, c2]), 1.0 / det)
}

/// Unit quaternion (w, x, y, z).
type Q = [f64; 4];
fn qmul(a: Q, b: Q) -> Q {
    [a[0] * b[0] - a[1] * b[1] - a[2] * b[2] - a[3] * b[3],
     a[0] * b[1] + a[1] * b[0] + a[2] * b[3] - a[3] * b[2],
     a[0] * b[2] - a[1] * b[3] + a[2] * b[0] + a[3] * b[1],
     a[0] * b[3] + a[1] * b[2] - a[2] * b[1] + a[3] * b[0]]
}
fn qconj(q: Q) -> Q { [q[0], -q[1], -q[2], -q[3]] }
fn qnorm(q: Q) -> Q { let n = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt(); [q[0] / n, q[1] / n, q[2] / n, q[3] / n] }
fn qmat(q: Q) -> M3 {
    let [w, x, y, z] = q;
    [[1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y - w * z), 2.0 * (x * z + w * y)],
     [2.0 * (x * y + w * z), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z - w * x)],
     [2.0 * (x * z - w * y), 2.0 * (y * z + w * x), 1.0 - 2.0 * (x * x + y * y)]]
}
/// Rotate `q` by the small rotation vector `d` (world frame).
fn qrotate(q: Q, d: V3) -> Q {
    let half = [0.0, d[0] * 0.5, d[1] * 0.5, d[2] * 0.5];
    let dq = qmul(half, q);
    qnorm([q[0] + dq[0], q[1] + dq[1], q[2] + dq[2], q[3] + dq[3]])
}
/// Rotation vector of a unit quaternion (shortest arc).
fn qlog(q: Q) -> V3 {
    let q = if q[0] < 0.0 { [-q[0], -q[1], -q[2], -q[3]] } else { q };
    let s = (q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    if s < 1e-12 {
        return [2.0 * q[1], 2.0 * q[2], 2.0 * q[3]];
    }
    let angle = 2.0 * s.atan2(q[0]);
    mul([q[1], q[2], q[3]], angle / s)
}

// ------------------------------------------------------------------------------------------------- input validation

#[derive(Clone)]
pub(super) struct PieceValues {
    active: bool,
    mass: f64,
    friction: f64,
    restitution: f64,
    linear_damping: f64,
    angular_damping: f64,
    v: V3,
    w: V3,
}

/// One convex collision part: vertices relative to the piece centre, triangles (mass) and polygon faces (collision).
#[derive(Clone)]
struct Part {
    local: Vec<V3>,
    triangles: Vec<[usize; 3]>,
    faces: Vec<Vec<usize>>,
}

#[derive(Clone)]
pub(super) struct Piece {
    from_bodies: bool,
    configured: bool,
    id: i64,
    points: Vec<usize>,
    prims: Vec<usize>,
    corners: Vec<usize>,
    center: V3,
    local: Vec<V3>,
    active: bool,
    values: PieceValues,
    parts: Vec<Part>,
}

fn prim_ints(a: &Attr) -> Option<Vec<i64>> {
    match &a.data {
        AttrData::I32(v) if a.width == 1 => Some(v.iter().map(|&x| x as i64).collect()),
        _ => None,
    }
}

fn row(a: &Attr, i: usize) -> Vec<f64> {
    let w = a.width;
    match &a.data {
        AttrData::F32(v) => v[i * w..(i + 1) * w].iter().map(|&x| x as f64).collect(),
        AttrData::I32(v) => v[i * w..(i + 1) * w].iter().map(|&x| x as f64).collect(),
    }
}

/// Singular values of an (n, 3) matrix: square roots of the eigenvalues of its 3x3 Gram matrix.
fn rank3(local: &[V3], tol: f64) -> usize {
    let mut g = ZERO_M;
    for p in local {
        for i in 0..3 {
            for j in 0..3 {
                g[i][j] += p[i] * p[j];
            }
        }
    }
    let (vals, _) = super::model::eigen(g);
    vals.iter().filter(|&&l| l.max(0.0).sqrt() > tol).count()
}

/// Validate one connected piece as a closed convex shell (`convex_part`); `points` sorted, `label` the piece id.
fn convex_part(geo: &Geometry, prims: &[usize], points: &[usize], label: i64, hull: bool) -> Result<Part> {
    let vp = &geo.topology.vertex_point;
    let offs = &geo.topology.prim_offsets;
    let gp = geo.p();
    let p: Vec<V3> = points.iter().map(|&i| [gp[i * 3] as f64, gp[i * 3 + 1] as f64, gp[i * 3 + 2] as f64]).collect();
    let n = p.len().max(1) as f64;
    let mean = p.iter().fold([0.0; 3], |s, q| add(s, *q)).map(|x| x / n);
    let local: Vec<V3> = p.iter().map(|q| sub(*q, mean)).collect();
    let mut extent = 0.0f64;
    for k in 0..3 {
        let lo = p.iter().map(|q| q[k]).fold(f64::INFINITY, f64::min);
        let hi = p.iter().map(|q| q[k]).fold(f64::NEG_INFINITY, f64::max);
        extent = extent.max(hi - lo);
    }
    let eps = (extent * 2e-5).max(1e-8);
    if points.len() < 4 || rank3(&local, eps) < 3 {
        return Err(Error::new(format!("RBD piece {label} has no solid volume")));
    }
    let mut edges: BTreeMap<(usize, usize), Vec<(usize, usize)>> = BTreeMap::new();
    let mut triangles = Vec::new();
    let mut faces = Vec::new();
    for &f in prims {
        let face: Vec<usize> = (offs[f] as usize..offs[f + 1] as usize)
            .map(|v| points.binary_search(&(vp[v] as usize)).unwrap_or(0)).collect();
        let distinct: BTreeSet<usize> = face.iter().copied().collect();
        if face.len() < 3 || distinct.len() != face.len() {
            return Err(Error::new(format!("RBD piece {label} has a degenerate face")));
        }
        let q: Vec<V3> = face.iter().map(|&i| local[i]).collect();
        let mut nrm = [0.0; 3];
        for k in 0..q.len() {
            nrm = add(nrm, cross(q[k], q[(k + 1) % q.len()]));
        }
        let length = norm(nrm);
        if length < eps * eps {
            return Err(Error::new(format!("RBD piece {label} has a zero-area face")));
        }
        let nrm = mul(nrm, 1.0 / length);
        if q.iter().map(|x| dot(sub(*x, q[0]), nrm).abs()).fold(0.0, f64::max) > eps {
            return Err(Error::new(format!("RBD piece {label} has non-planar faces; triangulate first")));
        }
        if !hull && local.iter().map(|x| dot(sub(*x, q[0]), nrm)).fold(f64::NEG_INFINITY, f64::max) > eps {
            return Err(Error::new(format!("RBD piece {label} is concave or inward wound; convex closed pieces only")));
        }
        for k in 0..face.len() {
            let (a, b) = (face[k], face[(k + 1) % face.len()]);
            edges.entry((a.min(b), a.max(b))).or_default().push((a, b));
        }
        for k in 1..face.len() - 1 {
            triangles.push([face[0], face[k], face[k + 1]]);
        }
        faces.push(face);
    }
    if edges.values().any(|e| e.len() != 2 || e[0] != (e[1].1, e[1].0)) {
        return Err(Error::new(format!("RBD piece {label} is open/non-manifold; use a closed solid (box for floor)")));
    }
    Ok(Part { local, triangles, faces })
}

/// `pieces(geo, static)`.
pub(super) fn pieces(geo: Option<&Geometry>, is_static: bool, from_bodies: bool) -> Result<Vec<Piece>> {
    let Some(geo) = geo else { return Ok(Vec::new()) };
    if geo.npoints == 0 && geo.nprims() == 0 {
        return Ok(Vec::new());
    }
    if geo.topology.has_open() || geo.nprims() == 0 {
        return Err(Error::new("RBD requires closed solid polygon meshes, not points, curves or open sheets"));
    }
    if geo.p().iter().any(|x| !x.is_finite()) {
        return Err(Error::new("RBD positions must be finite"));
    }
    if geo.detail.get("instance_objects").is_some_and(|v| match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
        Value::String(s) => !s.is_empty(),
        Value::Number(n) => n.as_f64() != Some(0.0),
    }) {
        return Err(Error::new("RBD needs actual mesh pieces, not Blender visual instances; use Copy to Points"));
    }
    let components: Vec<i64> = super::attrib_tools::piece_labels(geo).1.iter().map(|&x| x as i64).collect();
    let configured = geo.prim.contains_key("rbd_id");
    let ids = match geo.prim.get("rbd_id") {
        None if !is_static => return Err(Error::new("connect RBD Configure before RBD Solver")),
        None => components.clone(),
        Some(a) => prim_ints(a).ok_or_else(|| Error::new("rbd_id must be a scalar primitive integer attribute"))?,
    };
    let flag = |name: &str| geo.prim.get(name).map(|a| row_flags(a));
    let compound = flag("rbd_compound");
    let use_hull = flag("rbd_hull");
    let vp = &geo.topology.vertex_point;
    let offs = &geo.topology.prim_offsets;
    let gp = geo.p();
    let mut used = vec![false; geo.npoints];
    let mut result = Vec::new();
    let unique: BTreeSet<i64> = ids.iter().copied().collect();
    for ident in unique {
        let prims: Vec<usize> = (0..ids.len()).filter(|&i| ids[i] == ident).collect();
        let comps: BTreeSet<i64> = prims.iter().map(|&p| components[p]).collect();
        if comps.len() != 1 && !compound.as_ref().is_some_and(|c| prims.iter().all(|&p| c[p])) {
            return Err(Error::new(format!("RBD ID {ident} names disconnected pieces; use unique IDs/id_offset when merging \
                (or configure with 'compound' to move a glued cluster as one body)")));
        }
        let corners: Vec<usize> = prims.iter().flat_map(|&p| offs[p] as usize..offs[p + 1] as usize).collect();
        let points: Vec<usize> = corners.iter().map(|&v| vp[v] as usize).collect::<BTreeSet<_>>().into_iter().collect();
        if points.iter().any(|&p| used[p]) {
            return Err(Error::new("RBD pieces share points; split shared points before configuring separate bodies"));
        }
        for &p in &points {
            used[p] = true;
        }
        let pos: Vec<V3> = points.iter().map(|&i| [gp[i * 3] as f64, gp[i * 3 + 1] as f64, gp[i * 3 + 2] as f64]).collect();
        let n = pos.len() as f64;
        let center = pos.iter().fold([0.0; 3], |s, q| add(s, *q)).map(|x| x / n);
        let local: Vec<V3> = pos.iter().map(|q| sub(*q, center)).collect();
        let mut parts = Vec::new();
        for comp in &comps {
            let cprims: Vec<usize> = prims.iter().copied().filter(|&p| components[p] == *comp).collect();
            let cpoints: Vec<usize> = cprims.iter().flat_map(|&p| (offs[p] as usize..offs[p + 1] as usize).map(|v| vp[v] as usize))
                .collect::<BTreeSet<_>>().into_iter().collect();
            let hull = use_hull.as_ref().is_some_and(|h| cprims.iter().all(|&p| h[p]));
            let mut part = convex_part(geo, &cprims, &cpoints, ident, hull)?;
            part.local = cpoints.iter().map(|&i| sub([gp[i * 3] as f64, gp[i * 3 + 1] as f64, gp[i * 3 + 2] as f64], center)).collect();
            parts.push(part);
        }
        let get = |name: &str, default: &[f64]| -> Result<Vec<f64>> {
            let Some(a) = geo.prim.get(&format!("rbd_{name}")) else { return Ok(default.to_vec()) };
            let value = row(a, prims[0]);
            if prims.iter().any(|&p| row(a, p) != value) {
                return Err(Error::new(format!("rbd_{name} must be constant within piece {ident}")));
            }
            if value.len() != default.len() || value.iter().any(|x| !x.is_finite()) {
                return Err(Error::new(format!("invalid rbd_{name} on piece {ident}")));
            }
            Ok(value)
        };
        let active = get("active", &[1.0])?[0];
        let mass = get("mass", &[1.0])?[0];
        let friction = get("friction", &[0.6])?[0];
        let restitution = get("restitution", &[0.0])?[0];
        let linear_damping = get("linear_damping", &[0.05])?[0];
        let angular_damping = get("angular_damping", &[0.1])?[0];
        let v = get("v", &[0.0; 3])?;
        let w = get("w", &[0.0; 3])?;
        if mass <= 0.0 || !(0.0..=10.0).contains(&friction) || !(0.0..=1.0).contains(&restitution) {
            return Err(Error::new("RBD mass must be positive; friction 0..10 and restitution 0..1"));
        }
        if active != 0.0 && active != 1.0 {
            return Err(Error::new("rbd_active must be 0 or 1"));
        }
        if [linear_damping, angular_damping].iter().any(|d| !(0.0..=100.0).contains(d)) {
            return Err(Error::new("RBD damping must be 0..100 per second"));
        }
        let values = PieceValues { active: active == 1.0, mass, friction, restitution, linear_damping, angular_damping,
            v: [v[0], v[1], v[2]], w: [w[0], w[1], w[2]] };
        result.push(Piece { from_bodies, configured, id: ident, points, prims, corners, center, local,
            active: values.active && !is_static, values, parts });
    }
    if used.iter().any(|u| !u) {
        return Err(Error::new("RBD does not support loose points alongside solid pieces"));
    }
    Ok(result)
}

fn row_flags(a: &Attr) -> Vec<bool> {
    (0..a.rows()).map(|i| row(a, i).first().is_some_and(|&x| x != 0.0)).collect()
}

#[derive(Clone)]
pub(super) struct Link {
    a: usize,
    b: Option<usize>,
    strength: f64,
    length: f64,
    v: LinkValues,
    collide_connected: bool,
    legacy_glue: bool,
}

/// `link_table(constraints, pieces)`: primitive links; a missing kind is the original breakable glue.
pub(super) fn link_table(constraints: Option<&Geometry>, pieces: &[Piece]) -> Result<Vec<Link>> {
    let Some(c) = constraints.filter(|c| c.nprims() > 0) else { return Ok(Vec::new()) };
    let mut ab = Vec::new();
    for name in ["rbd_a", "rbd_b"] {
        let a = c.prim.get(name).and_then(|a| prim_ints(a))
            .ok_or_else(|| Error::new(format!("RBD constraints need an integer primitive attribute '{name}'")))?;
        ab.push(a);
    }
    let mut legacy_index = HashMap::new();
    let mut index = HashMap::new();
    let mut ambiguous = BTreeSet::new();
    for (i, p) in pieces.iter().enumerate() {
        legacy_index.insert(p.id, i);
        if !p.configured {
            continue; // unconfigured static collider IDs are internal, not new link targets
        }
        if index.contains_key(&p.id) {
            ambiguous.insert(p.id);
        }
        index.insert(p.id, i);
    }
    let get = |name: &str, k: usize, default: &[f64]| -> Result<Vec<f64>> {
        let Some(a) = c.prim.get(name) else { return Ok(default.to_vec()) };
        let value = row(a, k);
        if value.len() != default.len() {
            return Err(Error::new(format!("constraint {name} has the wrong attribute shape")));
        }
        Ok(value)
    };
    let mut out = Vec::new();
    for k in 0..c.nprims() {
        let raw_kind = get("rbd_constraint", k, &[0.0])?[0];
        if !(raw_kind.fract() == 0.0 && (0.0..4.0).contains(&raw_kind)) {
            return Err(Error::new(format!("constraint {k} has unknown rbd_constraint code {raw_kind}")));
        }
        let kind = raw_kind as usize;
        let strict = get("rbd_schema", k, &[0.0])?[0] != 0.0 || kind != 0;
        let lookup = if strict { &index } else { &legacy_index };
        let world = get("rbd_world", k, &[0.0])?[0] != 0.0;
        let mut requested = vec![ab[0][k]];
        if !world {
            requested.push(ab[1][k]);
        }
        if strict && requested.iter().any(|r| ambiguous.contains(r)) {
            return Err(Error::new("constraint body ID is ambiguous; configure unique IDs across bodies and colliders"));
        }
        let find = |id: i64| lookup.get(&id).copied().ok_or_else(|| Error::new(format!("constraint {k} names rbd_id {id}, which no body has")));
        let a = find(requested[0])?;
        let b = if world { None } else { Some(find(requested[1])?) };
        if Some(a) == b {
            if strict {
                return Err(Error::new("a constraint needs two different body IDs (or enable World)"));
            }
            continue; // legacy cluster links inside a single body do not constrain anything
        }
        let g3 = |name: &str, d: V3| -> Result<V3> { let v = get(name, k, &d)?; Ok([v[0], v[1], v[2]]) };
        let g1 = |name: &str, d: f64| -> Result<f64> { Ok(get(name, k, &[d])?[0]) };
        let mut v = LinkValues {
            kind,
            anchor: g3("anchor", [0.0; 3])?,
            anchor_b: g3("anchor_b", [0.0, 0.0, 1.0])?,
            axis: g3("axis", [0.0, 1.0, 0.0])?,
            limited: g1("limited", 0.0)? != 0.0,
            lower: g1("lower", 0.0)?,
            upper: g1("upper", 90.0)?,
            stiffness: g1("stiffness", 100.0)?,
            damping: g1("damping", 1.0)?,
            rest_length: g1("rest_length", -1.0)?,
            strength: g1("strength", 0.0)?,
            break_torque: g1("break_torque", 0.0)?,
        };
        let collide_connected = g1("collide_connected", 0.0)? != 0.0;
        // historical glue treats zero/negative/non-finite strength as unbreakable
        if kind == 0 && (!v.strength.is_finite() || v.strength < 0.0) {
            v.strength = 0.0;
        }
        validate_link_values(&v)?;
        if kind == 2 && !pieces[a].active {
            return Err(Error::new("hinge Body A must be active; swap A and B if the other body should rotate"));
        }
        let length = g1("length", 0.0)?;
        out.push(Link { a, b, strength: v.strength, length, v, collide_connected, legacy_glue: !strict });
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------------------- the world

#[derive(Clone)]
struct Shape {
    /// vertices relative to the body's centre of mass, in the body frame
    verts: Vec<V3>,
    faces: Vec<Vec<usize>>,
    normals: Vec<V3>,
    /// unique edge directions (body frame) and the edges (vertex pairs) themselves
    edges: Vec<(usize, usize)>,
    edge_dirs: Vec<V3>,
    radius: f64,
}

#[derive(Clone)]
struct Body {
    inv_mass: f64,
    inv_inertia: M3, // body frame
    /// piece centre (Python's body origin) relative to the centre of mass, body frame
    origin: V3,
    shapes: Vec<Shape>,
    friction: f64,
    restitution: f64,
    linear_damping: f64,
    angular_damping: f64,
}

#[derive(Clone)]
struct State {
    x: Vec<V3>,
    q: Vec<Q>,
    v: Vec<V3>,
    w: Vec<V3>,
    broken: Vec<bool>,
    excluded: BTreeSet<(usize, usize)>,
    since_start: u64,
    frame: i64,
    warm: HashMap<(usize, usize, usize, usize), Vec<(V3, f64, f64, f64)>>,
}

/// Per-link data in body frames.
#[derive(Clone)]
struct Joint {
    /// anchor on A / B relative to each body's centre of mass, body frames (B: world point when B is the world)
    ra: V3,
    rb: V3,
    /// glue: relative orientation qB^-1 qA at rest; hinge: axis in A and B frames
    rel: Q,
    axis_a: V3,
    axis_b: V3,
    rest: f64,
}

struct Settings {
    gravity: V3,
    start_frame: i64,
    substeps: i64,
    iterations: i64,
    margin: f64,
    cache_mb: f64,
    include_colliders: bool,
}

type Pose = (Vec<V3>, Vec<M3>, Vec<[f64; 6]>, Vec<bool>);

pub(super) struct World {
    settings: Settings,
    bodies_geo: Arc<Geometry>,
    colliders: Option<Arc<Geometry>>,
    pieces: Vec<Piece>,
    links: Vec<Link>,
    reach: Vec<f64>,
    bodies: Vec<Body>,
    joints: Vec<Joint>,
    /// linked pairs whose contacts are off while one of their non-colliding links holds
    pairs: BTreeMap<(usize, usize), Vec<usize>>,
    initial: State,
    state: State,
    dt: f64,
    frames: BTreeMap<i64, Pose>,
    order: VecDeque<i64>,
    cache_bytes: usize,
}

/// Mass properties of closed, outward-wound triangle meshes (uniform density): volume, centroid, covariance about the
/// origin (per unit density).
fn mesh_moments(local: &[V3], tris: &[[usize; 3]]) -> (f64, V3, M3) {
    let (mut vol, mut first, mut cov) = (0.0, [0.0; 3], ZERO_M);
    for t in tris {
        let (a, b, c) = (local[t[0]], local[t[1]], local[t[2]]);
        let det = dot(a, cross(b, c));
        vol += det / 6.0;
        first = add(first, mul(add(add(a, b), c), det / 24.0));
        let s = add(add(a, b), c);
        for i in 0..3 {
            for j in 0..3 {
                cov[i][j] += det / 120.0 * (a[i] * a[j] + b[i] * b[j] + c[i] * c[j] + s[i] * s[j]);
            }
        }
    }
    if vol < 0.0 {
        (-vol, mul(first, -1.0 / -vol), mscale(&cov, -1.0))
    } else if vol > 0.0 {
        (vol, mul(first, 1.0 / vol), cov)
    } else {
        (0.0, [0.0; 3], ZERO_M)
    }
}

fn shape_of(part: &Part, com: V3) -> Shape {
    let verts: Vec<V3> = part.local.iter().map(|p| sub(*p, com)).collect();
    let mut normals = Vec::new();
    for f in &part.faces {
        let mut n = [0.0; 3];
        for k in 0..f.len() {
            n = add(n, cross(verts[f[k]], verts[f[(k + 1) % f.len()]]));
        }
        normals.push(unit(n));
    }
    let mut edges = Vec::new();
    let mut edge_dirs: Vec<V3> = Vec::new();
    let mut seen = BTreeSet::new();
    for f in &part.faces {
        for k in 0..f.len() {
            let (a, b) = (f[k], f[(k + 1) % f.len()]);
            if seen.insert((a.min(b), a.max(b))) {
                edges.push((a, b));
                let d = unit(sub(verts[b], verts[a]));
                if !edge_dirs.iter().any(|e| norm(cross(*e, d)) < 1e-6) {
                    edge_dirs.push(d);
                }
            }
        }
    }
    let radius = verts.iter().map(|v| norm(*v)).fold(0.0, f64::max);
    Shape { verts, faces: part.faces.clone(), normals, edges, edge_dirs, radius }
}

impl World {
    fn new(bodies_geo: Arc<Geometry>, colliders: Option<Arc<Geometry>>, constraints: Option<&Geometry>, settings: Settings) -> Result<World> {
        let mut all = pieces(Some(&bodies_geo), false, true)?;
        all.extend(pieces(colliders.as_deref(), true, false)?);
        if all.len() > 2000 || all.iter().map(|p| p.points.len()).sum::<usize>() > 200000 {
            return Err(Error::new("RBD initial limit: 2000 bodies / 200000 collision vertices; simplify inputs"));
        }
        let dt = 1.0 / (timectx::fps() * settings.substeps as f64);
        let mut bodies = Vec::new();
        let mut st = State { x: vec![], q: vec![], v: vec![], w: vec![], broken: vec![], excluded: BTreeSet::new(), since_start: 0,
            frame: settings.start_frame, warm: HashMap::new() };
        for p in &all {
            // a glued cluster is one body with a convex part each; mass split by volume
            let moments: Vec<(f64, V3, M3)> = p.parts.iter().map(|part| mesh_moments(&part.local, &part.triangles)).collect();
            let total: f64 = moments.iter().map(|m| m.0).sum();
            let mass = p.values.mass;
            let (mut com, mut cov) = ([0.0; 3], ZERO_M);
            for (vol, c, cv) in &moments {
                let share = if p.parts.len() == 1 { 1.0 } else if total > 0.0 { vol / total } else { 1.0 / p.parts.len() as f64 };
                let m_part = mass * share;
                let density = if *vol > 1e-300 { m_part / vol } else { 0.0 };
                com = add(com, mul(*c, m_part / mass));
                cov = madd(&cov, &mscale(cv, density));
            }
            // covariance about the centre of mass -> inertia tensor
            let mut c = cov;
            for i in 0..3 {
                for j in 0..3 {
                    c[i][j] -= mass * com[i] * com[j];
                }
            }
            let tr = c[0][0] + c[1][1] + c[2][2];
            let mut inertia = mscale(&c, -1.0);
            let extent = p.local.iter().map(|v| norm(*v)).fold(1e-6, f64::max);
            let floor = mass * extent * extent * 1e-6;
            for i in 0..3 {
                inertia[i][i] += tr + floor;
            }
            let shapes = p.parts.iter().map(|part| shape_of(part, com)).collect();
            let active = p.active;
            bodies.push(Body {
                inv_mass: if active { 1.0 / mass } else { 0.0 },
                inv_inertia: if active { inverse(&inertia) } else { ZERO_M },
                origin: mul(com, -1.0),
                shapes,
                friction: p.values.friction,
                restitution: p.values.restitution,
                linear_damping: p.values.linear_damping,
                angular_damping: p.values.angular_damping,
            });
            st.x.push(add(p.center, com));
            st.q.push([1.0, 0.0, 0.0, 0.0]);
            // MuJoCo's free joint velocity is that of the body origin; angular velocity starts in world = local axes
            let (v, w) = if active { (p.values.v, p.values.w) } else { ([0.0; 3], [0.0; 3]) };
            st.v.push(add(v, cross(w, com)));
            st.w.push(w);
        }
        let links = link_table(constraints, &all)?;
        let extent: Vec<f64> = all.iter().map(|p| {
            let mut e = 0.0f64;
            for k in 0..3 {
                let lo = p.local.iter().map(|v| v[k]).fold(f64::INFINITY, f64::min);
                let hi = p.local.iter().map(|v| v[k]).fold(f64::NEG_INFINITY, f64::max);
                e = e.max(hi - lo);
            }
            e.max(1e-6)
        }).collect();
        let reach = links.iter().map(|l| if l.length > 0.0 { l.length } else { extent[l.a].min(l.b.map_or(extent[l.a], |b| extent[b])) }).collect();
        let mut pairs: BTreeMap<(usize, usize), Vec<usize>> = BTreeMap::new();
        let mut joints = Vec::new();
        for (k, l) in links.iter().enumerate() {
            let com = |i: usize| st.x[i];
            let anchor = l.v.anchor;
            let ra = sub(anchor, com(l.a));
            let rb = match l.b { Some(b) => sub(anchor, com(b)), None => anchor };
            let mut j = Joint { ra, rb, rel: [1.0, 0.0, 0.0, 0.0], axis_a: unit(l.v.axis), axis_b: unit(l.v.axis), rest: 0.0 };
            match l.v.kind {
                0 => {
                    // weld at A's origin preserving the initial relative pose
                    let origin = all[l.a].center;
                    j.ra = sub(origin, com(l.a));
                    j.rb = match l.b { Some(b) => sub(origin, com(b)), None => origin };
                }
                3 => {
                    j.rb = match l.b { Some(b) => sub(l.v.anchor_b, com(b)), None => l.v.anchor_b };
                    let initial = norm(sub(l.v.anchor_b, l.v.anchor));
                    j.rest = if l.v.rest_length == -1.0 { initial } else { l.v.rest_length };
                }
                _ => {}
            }
            joints.push(j);
            if let Some(b) = l.b {
                if !l.collide_connected {
                    pairs.entry((l.a.min(b), l.a.max(b))).or_default().push(k);
                }
            }
        }
        st.broken = vec![false; links.len()];
        st.excluded = pairs.keys().copied().collect();
        Ok(World { settings, bodies_geo, colliders, pieces: all, links, reach, bodies, joints, pairs, initial: st.clone(), state: st,
            dt, frames: BTreeMap::new(), order: VecDeque::new(), cache_bytes: 0 })
    }

    fn restart(&mut self) {
        self.state = self.initial.clone();
    }

    fn inv_inertia_world(&self, i: usize) -> M3 {
        let r = qmat(self.state.q[i]);
        mm(&mm(&r, &self.bodies[i].inv_inertia), &mt(&r))
    }

    fn pose(&mut self, target: i64) -> Result<Pose> {
        if let Some(p) = self.frames.get(&target) {
            let p = p.clone();
            self.order.retain(|&f| f != target);
            self.order.push_back(target);
            return Ok(p);
        }
        if target < self.state.frame {
            self.restart(); // rebuild all contact/warm-start state; never resume from a pose snapshot
        }
        while self.state.frame < target {
            for _ in 0..self.settings.substeps {
                self.substep();
                let s = &self.state;
                if s.x.iter().chain(s.v.iter()).chain(s.w.iter()).flatten().any(|x| !x.is_finite() || x.abs() > 1e9) {
                    return Err(Error::new("RBD numerical instability; reduce velocities/scale extremes or increase substeps"));
                }
            }
            self.state.frame += 1;
        }
        let n = self.bodies.len();
        let (mut pos, mut rot, mut vel) = (Vec::with_capacity(n), Vec::with_capacity(n), Vec::with_capacity(n));
        for i in 0..n {
            let r = qmat(self.state.q[i]);
            let arm = mv(&r, self.bodies[i].origin);
            pos.push(add(self.state.x[i], arm));
            rot.push(r);
            let w = self.state.w[i];
            let v = add(self.state.v[i], cross(w, arm));
            vel.push([w[0], w[1], w[2], v[0], v[1], v[2]]);
        }
        let value: Pose = (pos, rot, vel, self.state.broken.clone());
        let size = n * (24 + 72 + 48) + self.links.len();
        let budget = (self.settings.cache_mb * 1024.0 * 1024.0) as usize;
        if size <= budget {
            while !self.frames.is_empty() && self.cache_bytes + size > budget {
                if let Some(old) = self.order.pop_front() {
                    if self.frames.remove(&old).is_some() {
                        self.cache_bytes -= size;
                    }
                }
            }
            self.frames.insert(target, value.clone());
            self.order.push_back(target);
            self.cache_bytes += size;
        }
        Ok(value)
    }

    // ------------------------------------------------------------------------------------------------- one substep

    fn substep(&mut self) {
        let dt = self.dt;
        let n = self.bodies.len();
        let g = self.settings.gravity;
        for i in 0..n {
            if self.bodies[i].inv_mass == 0.0 {
                continue;
            }
            let b = &self.bodies[i];
            let (ld, ad) = ((-b.linear_damping * dt).exp(), (-b.angular_damping * dt).exp());
            self.state.v[i] = add(mul(self.state.v[i], ld), mul(g, dt));
            self.state.w[i] = mul(self.state.w[i], ad);
        }
        let iw: Vec<M3> = (0..n).map(|i| self.inv_inertia_world(i)).collect();
        let mut contacts = self.collide();
        // springs: explicit Hooke + damping force along the anchor line
        let mut spring_force = vec![0.0f64; self.links.len()];
        let mut spring_impulses = Vec::new();
        for (k, l) in self.links.iter().enumerate() {
            if l.v.kind != 3 || self.state.broken[k] {
                continue;
            }
            let (pa, pb, va, vb) = self.anchor_state(k);
            let d = sub(pb, pa);
            let len = norm(d);
            if len < 1e-12 {
                continue;
            }
            let u = mul(d, 1.0 / len);
            let ldot = dot(sub(vb, va), u);
            let passive = -l.v.stiffness * (len - self.joints[k].rest) - l.v.damping * ldot;
            spring_force[k] = passive.abs();
            // tension (negative passive force) pulls A towards B
            spring_impulses.push((l.a, l.b, mul(u, -passive * dt), pa, pb));
        }
        for (a, b, impulse, pa, pb) in spring_impulses {
            self.apply(a, impulse, pa, &iw);
            if let Some(b) = b {
                self.apply(b, mul(impulse, -1.0), pb, &iw);
            }
        }
        // joint rows
        let mut rows = self.prepare_joints(&iw);
        // contact preparation and warm start
        for c in contacts.iter_mut() {
            self.prepare_contact(c, &iw);
        }
        for _ in 0..self.settings.iterations.max(1) {
            for r in rows.iter_mut() {
                self.solve_row(r, &iw);
            }
            for c in contacts.iter_mut() {
                self.solve_contact(c, &iw);
            }
        }
        // remember contact impulses for the next substep (warm starting)
        self.state.warm.clear();
        for c in &contacts {
            self.state.warm.entry(c.key).or_default().push((c.local_a, c.ln, c.lt[0], c.lt[1]));
        }
        // integrate
        for i in 0..n {
            if self.bodies[i].inv_mass == 0.0 {
                continue;
            }
            self.state.x[i] = add(self.state.x[i], mul(self.state.v[i], dt));
            self.state.q[i] = qrotate(self.state.q[i], mul(self.state.w[i], dt));
        }
        // position correction (contacts and joints)
        let iw: Vec<M3> = (0..n).map(|i| self.inv_inertia_world(i)).collect();
        for _ in 0..4 {
            for c in &contacts {
                self.correct_contact(c, &iw);
            }
            for k in 0..self.links.len() {
                if !self.state.broken[k] {
                    self.correct_joint(k, &iw);
                }
            }
        }
        self.state.since_start += 1;
        self.snap_links(&rows, &spring_force);
    }

    /// World anchor points and their velocities for link `k` (A side, B side).
    fn anchor_state(&self, k: usize) -> (V3, V3, V3, V3) {
        let l = &self.links[k];
        let j = &self.joints[k];
        let s = &self.state;
        let ra = mv(&qmat(s.q[l.a]), j.ra);
        let pa = add(s.x[l.a], ra);
        let va = add(s.v[l.a], cross(s.w[l.a], ra));
        match l.b {
            Some(b) => {
                let rb = mv(&qmat(s.q[b]), j.rb);
                (pa, add(s.x[b], rb), va, add(s.v[b], cross(s.w[b], rb)))
            }
            None => (pa, j.rb, va, [0.0; 3]),
        }
    }

    fn apply(&mut self, i: usize, impulse: V3, at: V3, iw: &[M3]) {
        if self.bodies[i].inv_mass == 0.0 {
            return;
        }
        let r = sub(at, self.state.x[i]);
        self.state.v[i] = add(self.state.v[i], mul(impulse, self.bodies[i].inv_mass));
        self.state.w[i] = add(self.state.w[i], mv(&iw[i], cross(r, impulse)));
    }

    fn apply_angular(&mut self, i: usize, impulse: V3, iw: &[M3]) {
        if self.bodies[i].inv_mass == 0.0 {
            return;
        }
        self.state.w[i] = add(self.state.w[i], mv(&iw[i], impulse));
    }

    // ---------------------------------------------------------------------------------------------------- collision

    fn collide(&self) -> Vec<Contact> {
        let n = self.bodies.len();
        let margin = self.settings.margin;
        // bounding spheres per shape, world space
        let mut spheres = Vec::new();
        for i in 0..n {
            let r = qmat(self.state.q[i]);
            for (si, s) in self.bodies[i].shapes.iter().enumerate() {
                let c = s.verts.iter().fold([0.0; 3], |a, v| add(a, *v)).map(|x| x / s.verts.len().max(1) as f64);
                let rad = s.verts.iter().map(|v| norm(sub(*v, c))).fold(0.0, f64::max);
                spheres.push((i, si, add(self.state.x[i], mv(&r, c)), rad));
            }
        }
        let mut out = Vec::new();
        for a in 0..spheres.len() {
            for b in a + 1..spheres.len() {
                let (i, si, ca, ra) = spheres[a];
                let (j, sj, cb, rb) = spheres[b];
                if i == j || (self.bodies[i].inv_mass == 0.0 && self.bodies[j].inv_mass == 0.0) {
                    continue;
                }
                if self.state.excluded.contains(&(i.min(j), i.max(j))) {
                    continue;
                }
                if norm(sub(cb, ca)) > ra + rb + margin {
                    continue;
                }
                self.collide_pair(i, si, j, sj, &mut out);
            }
        }
        out
    }

    fn world_verts(&self, i: usize, s: usize) -> Vec<V3> {
        let r = qmat(self.state.q[i]);
        self.bodies[i].shapes[s].verts.iter().map(|v| add(self.state.x[i], mv(&r, *v))).collect()
    }

    fn collide_pair(&self, i: usize, si: usize, j: usize, sj: usize, out: &mut Vec<Contact>) {
        let margin = self.settings.margin;
        let (sa, sb) = (&self.bodies[i].shapes[si], &self.bodies[j].shapes[sj]);
        let (ra, rb) = (qmat(self.state.q[i]), qmat(self.state.q[j]));
        let (va, vb) = (self.world_verts(i, si), self.world_verts(j, sj));
        let ca = va.iter().fold([0.0; 3], |s, v| add(s, *v)).map(|x| x / va.len() as f64);
        let cb = vb.iter().fold([0.0; 3], |s, v| add(s, *v)).map(|x| x / vb.len() as f64);
        let support = |vs: &[V3], d: V3| vs.iter().map(|v| dot(*v, d)).fold(f64::NEG_INFINITY, f64::max);
        // face axes of A (normal out of A), face axes of B (normal out of B)
        let mut best_a = (f64::NEG_INFINITY, 0usize);
        for (f, n) in sa.normals.iter().enumerate() {
            let n = mv(&ra, *n);
            let d = dot(va[sa.faces[f][0]], n);
            let sep = -support(&vb, mul(n, -1.0)) - d;
            if sep > best_a.0 {
                best_a = (sep, f);
            }
            if sep > margin {
                return;
            }
        }
        let mut best_b = (f64::NEG_INFINITY, 0usize);
        for (f, n) in sb.normals.iter().enumerate() {
            let n = mv(&rb, *n);
            let d = dot(vb[sb.faces[f][0]], n);
            let sep = -support(&va, mul(n, -1.0)) - d;
            if sep > best_b.0 {
                best_b = (sep, f);
            }
            if sep > margin {
                return;
            }
        }
        // edge-edge axes
        let mut best_e = (f64::NEG_INFINITY, [0.0; 3]);
        for ea in &sa.edge_dirs {
            let ea = mv(&ra, *ea);
            for eb in &sb.edge_dirs {
                let eb = mv(&rb, *eb);
                let axis = cross(ea, eb);
                let l = norm(axis);
                if l < 1e-6 {
                    continue;
                }
                let mut axis = mul(axis, 1.0 / l);
                if dot(axis, sub(cb, ca)) < 0.0 {
                    axis = mul(axis, -1.0);
                }
                let sep = -support(&vb, mul(axis, -1.0)) - support(&va, axis);
                if sep > margin {
                    return;
                }
                if sep > best_e.0 {
                    best_e = (sep, axis);
                }
            }
        }
        let face_sep = best_a.0.max(best_b.0);
        let key = (i, si, j, sj);
        // prefer face contacts: an edge axis must separate clearly better
        if best_e.0 > face_sep + 1e-3 * (sa.radius + sb.radius).max(1e-3) {
            // edge contact: closest points of the two deepest edges along the axis
            let axis = best_e.1;
            let pick = |vs: &[V3], s: &Shape, d: V3| -> (V3, V3) {
                let mut best = (f64::NEG_INFINITY, (vs[0], vs[0]));
                for &(a, b) in &s.edges {
                    let e = unit(sub(vs[b], vs[a]));
                    if dot(e, d).abs() > 1e-3 {
                        continue;
                    }
                    let score = dot(vs[a], d) + dot(vs[b], d);
                    if score > best.0 {
                        best = (score, (vs[a], vs[b]));
                    }
                }
                best.1
            };
            let (a0, a1) = pick(&va, sa, axis);
            let (b0, b1) = pick(&vb, sb, mul(axis, -1.0));
            let (pa, pb) = closest_segments(a0, a1, b0, b1);
            let point_a = pa;
            let point_b = pb;
            out.push(self.contact(key, i, j, axis, point_a, point_b, true));
            return;
        }
        // face contact: the reference face is on the shape with the larger separation
        let (ref_body, inc_body, ref_shape, inc_shape, rr, ri, vr, vi, face) = if best_a.0 >= best_b.0 - 1e-9 {
            (i, j, sa, sb, ra, rb, &va, &vb, best_a.1)
        } else {
            (j, i, sb, sa, rb, ra, &vb, &va, best_b.1)
        };
        let n = mv(&rr, ref_shape.normals[face]);
        // incident face: the most anti-parallel face of the other shape
        let inc = (0..inc_shape.faces.len()).min_by(|&x, &y| {
            dot(mv(&ri, inc_shape.normals[x]), n).total_cmp(&dot(mv(&ri, inc_shape.normals[y]), n))
        }).unwrap();
        let mut poly: Vec<V3> = inc_shape.faces[inc].iter().map(|&k| vi[k]).collect();
        let rf = &ref_shape.faces[face];
        for k in 0..rf.len() {
            let (p0, p1) = (vr[rf[k]], vr[rf[(k + 1) % rf.len()]]);
            let side = unit(cross(sub(p1, p0), n));
            poly = clip(&poly, side, dot(side, p0));
            if poly.is_empty() {
                break;
            }
        }
        let d = dot(vr[rf[0]], n);
        let mut pts: Vec<(V3, f64)> = poly.into_iter().map(|p| (p, dot(p, n) - d)).filter(|(_, s)| *s <= margin).collect();
        if pts.len() > 4 {
            pts = reduce(pts, n);
        }
        for (p, sep) in pts {
            let on_ref = sub(p, mul(n, sep));
            // normal from body i to body j
            let (nab, pi, pj) = if ref_body == i { (n, on_ref, p) } else { (mul(n, -1.0), p, on_ref) };
            let _ = inc_body;
            out.push(self.contact(key, i, j, nab, pi, pj, false));
        }
    }

    fn contact(&self, key: (usize, usize, usize, usize), a: usize, b: usize, n: V3, pa: V3, pb: V3, edge: bool) -> Contact {
        let (qa, qb) = (qmat(self.state.q[a]), qmat(self.state.q[b]));
        let local_a = mv(&mt(&qa), sub(pa, self.state.x[a]));
        let local_b = mv(&mt(&qb), sub(pb, self.state.x[b]));
        let local_n = mv(&mt(&qa), n);
        let (ba, bb) = (&self.bodies[a], &self.bodies[b]);
        let _ = edge;
        Contact {
            key, a, b, n, local_a, local_b, local_n,
            friction: ba.friction.max(bb.friction),
            restitution: 0.5 * (ba.restitution + bb.restitution),
            ra: [0.0; 3], rb: [0.0; 3], t: [[0.0; 3]; 2], mn: 0.0, mt: [0.0; 2], bias: 0.0, ln: 0.0, lt: [0.0; 2],
        }
    }

    fn prepare_contact(&mut self, c: &mut Contact, iw: &[M3]) {
        let s = &self.state;
        let (qa, qb) = (qmat(s.q[c.a]), qmat(s.q[c.b]));
        let pa = add(s.x[c.a], mv(&qa, c.local_a));
        let pb = add(s.x[c.b], mv(&qb, c.local_b));
        let p = mul(add(pa, pb), 0.5);
        c.ra = sub(p, s.x[c.a]);
        c.rb = sub(p, s.x[c.b]);
        let (ma, mb) = (self.bodies[c.a].inv_mass, self.bodies[c.b].inv_mass);
        let k = |d: V3| -> f64 {
            let ca = cross(c.ra, d);
            let cb = cross(c.rb, d);
            ma + mb + dot(ca, mv(&iw[c.a], ca)) + dot(cb, mv(&iw[c.b], cb))
        };
        let t0 = if c.n[0].abs() < 0.57 { unit(cross(c.n, [1.0, 0.0, 0.0])) } else { unit(cross(c.n, [0.0, 1.0, 0.0])) };
        let t1 = cross(c.n, t0);
        c.t = [t0, t1];
        c.mn = 1.0 / k(c.n).max(1e-300);
        c.mt = [1.0 / k(t0).max(1e-300), 1.0 / k(t1).max(1e-300)];
        let vrel = self.relative_velocity(c);
        let vn = dot(vrel, c.n);
        c.bias = if c.restitution > 0.0 && vn < -0.2 { -c.restitution * vn } else { 0.0 };
        // warm start from a matching contact of the previous substep
        let tol = 0.02 * self.bodies[c.a].shapes[c.key.1].radius.max(1e-3);
        let found = self.state.warm.get(&c.key).and_then(|prev| prev.iter().find(|w| norm(sub(w.0, c.local_a)) < tol).copied());
        if let Some((_, ln, l0, l1)) = found {
            c.ln = ln * 0.9;
            c.lt = [l0 * 0.9, l1 * 0.9];
            let impulse = add(mul(c.n, c.ln), add(mul(c.t[0], c.lt[0]), mul(c.t[1], c.lt[1])));
            self.push(c.a, c.b, c.ra, c.rb, impulse, iw);
        }
    }

    fn relative_velocity(&self, c: &Contact) -> V3 {
        let s = &self.state;
        sub(add(s.v[c.b], cross(s.w[c.b], c.rb)), add(s.v[c.a], cross(s.w[c.a], c.ra)))
    }

    /// Apply `impulse` to B and its opposite to A at the given arms.
    fn push(&mut self, a: usize, b: usize, ra: V3, rb: V3, impulse: V3, iw: &[M3]) {
        let (ma, mb) = (self.bodies[a].inv_mass, self.bodies[b].inv_mass);
        if ma > 0.0 {
            self.state.v[a] = sub(self.state.v[a], mul(impulse, ma));
            self.state.w[a] = sub(self.state.w[a], mv(&iw[a], cross(ra, impulse)));
        }
        if mb > 0.0 {
            self.state.v[b] = add(self.state.v[b], mul(impulse, mb));
            self.state.w[b] = add(self.state.w[b], mv(&iw[b], cross(rb, impulse)));
        }
    }

    fn solve_contact(&mut self, c: &mut Contact, iw: &[M3]) {
        // friction first (uses the current normal impulse for its cone)
        let vrel = self.relative_velocity(c);
        let limit = c.friction * c.ln;
        let old = c.lt;
        let mut lt = [old[0] - c.mt[0] * dot(vrel, c.t[0]), old[1] - c.mt[1] * dot(vrel, c.t[1])];
        let mag = (lt[0] * lt[0] + lt[1] * lt[1]).sqrt();
        if mag > limit {
            let s = if mag > 0.0 { limit / mag } else { 0.0 };
            lt = [lt[0] * s, lt[1] * s];
        }
        c.lt = lt;
        let d = add(mul(c.t[0], lt[0] - old[0]), mul(c.t[1], lt[1] - old[1]));
        self.push(c.a, c.b, c.ra, c.rb, d, iw);
        let vn = dot(self.relative_velocity(c), c.n);
        let new = (c.ln - c.mn * (vn - c.bias)).max(0.0);
        let d = new - c.ln;
        c.ln = new;
        self.push(c.a, c.b, c.ra, c.rb, mul(c.n, d), iw);
    }

    fn correct_contact(&mut self, c: &Contact, iw: &[M3]) {
        let s = &self.state;
        let (qa, qb) = (qmat(s.q[c.a]), qmat(s.q[c.b]));
        let pa = add(s.x[c.a], mv(&qa, c.local_a));
        let pb = add(s.x[c.b], mv(&qb, c.local_b));
        let n = mv(&qa, c.local_n);
        let sep = dot(sub(pb, pa), n);
        let slop = 0.0005;
        let correction = (0.2 * (sep + slop)).clamp(-0.05, 0.0);
        if correction >= 0.0 {
            return;
        }
        let p = mul(add(pa, pb), 0.5);
        let (ra, rb) = (sub(p, s.x[c.a]), sub(p, s.x[c.b]));
        let (ma, mb) = (self.bodies[c.a].inv_mass, self.bodies[c.b].inv_mass);
        let (ca, cb) = (cross(ra, n), cross(rb, n));
        let k = ma + mb + dot(ca, mv(&iw[c.a], ca)) + dot(cb, mv(&iw[c.b], cb));
        if k <= 0.0 {
            return;
        }
        let impulse = mul(n, -correction / k);
        self.shift(c.a, mul(impulse, -1.0), ra, iw);
        self.shift(c.b, impulse, rb, iw);
    }

    /// Move a body by a position-level impulse.
    fn shift(&mut self, i: usize, impulse: V3, r: V3, iw: &[M3]) {
        let m = self.bodies[i].inv_mass;
        if m == 0.0 {
            return;
        }
        self.state.x[i] = add(self.state.x[i], mul(impulse, m));
        self.state.q[i] = qrotate(self.state.q[i], mv(&iw[i], cross(r, impulse)));
    }

    fn shift_angular(&mut self, i: usize, impulse: V3, iw: &[M3]) {
        if self.bodies[i].inv_mass == 0.0 {
            return;
        }
        self.state.q[i] = qrotate(self.state.q[i], mv(&iw[i], impulse));
    }

    // ------------------------------------------------------------------------------------------------------ joints

    fn prepare_joints(&mut self, iw: &[M3]) -> Vec<Row> {
        let mut rows = Vec::new();
        let dt = self.dt;
        for k in 0..self.links.len() {
            if self.state.broken[k] {
                continue;
            }
            let l = self.links[k].clone();
            let (pa, pb, _, _) = self.anchor_state(k);
            let a = l.a;
            let ra = sub(pa, self.state.x[a]);
            let rb = l.b.map_or([0.0; 3], |b| sub(pb, self.state.x[b]));
            match l.v.kind {
                0 | 1 | 2 => {
                    rows.push(Row::point(k, a, l.b, ra, rb, self.point_mass(a, l.b, ra, rb, iw)));
                    if l.v.kind == 0 {
                        rows.push(Row::angular(k, a, l.b, RowKind::Weld, [0.0; 3], inverse(&self.angular_k(a, l.b, iw)), 0.0, false, None));
                    } else if l.v.kind == 2 {
                        let axis = self.hinge_axis(k);
                        let t0 = if axis[0].abs() < 0.57 { unit(cross(axis, [1.0, 0.0, 0.0])) } else { unit(cross(axis, [0.0, 1.0, 0.0])) };
                        let t1 = cross(axis, t0);
                        let kk = self.angular_k(a, l.b, iw);
                        for t in [t0, t1] {
                            let m = 1.0 / dot(t, mv(&kk, t)).max(1e-300);
                            rows.push(Row::angular(k, a, l.b, RowKind::Axis, t, [[m, 0.0, 0.0], ZERO_M[1], ZERO_M[2]], 0.0, false, None));
                        }
                        let m = 1.0 / dot(axis, mv(&kk, axis)).max(1e-300);
                        if l.v.damping > 0.0 {
                            let gamma = 1.0 / (l.v.damping * dt);
                            let m_soft = 1.0 / (1.0 / m + gamma);
                            rows.push(Row::angular(k, a, l.b, RowKind::Damper(gamma), axis, [[m_soft, 0.0, 0.0], ZERO_M[1], ZERO_M[2]], 0.0, false, None));
                        }
                        if l.v.limited {
                            let angle = self.hinge_angle(k);
                            let (lo, hi) = (l.v.lower.to_radians(), l.v.upper.to_radians());
                            if angle <= lo + 1e-4 {
                                rows.push(Row::angular(k, a, l.b, RowKind::Axis, axis, [[m, 0.0, 0.0], ZERO_M[1], ZERO_M[2]], 0.0, true, Some(1.0)));
                            } else if angle >= hi - 1e-4 {
                                rows.push(Row::angular(k, a, l.b, RowKind::Axis, axis, [[m, 0.0, 0.0], ZERO_M[1], ZERO_M[2]], 0.0, true, Some(-1.0)));
                            }
                        }
                    }
                }
                3 if l.v.limited => {
                    let d = sub(pb, pa);
                    let len = norm(d);
                    if len < 1e-12 {
                        continue;
                    }
                    let u = mul(d, 1.0 / len);
                    let sign = if len <= l.v.lower { Some(1.0) } else if len >= l.v.upper { Some(-1.0) } else { None };
                    if let Some(sign) = sign {
                        let (ma, mb) = (self.bodies[a].inv_mass, l.b.map_or(0.0, |b| self.bodies[b].inv_mass));
                        let (ca, cb) = (cross(ra, u), cross(rb, u));
                        let kk = ma + mb + dot(ca, mv(&iw[a], ca)) + l.b.map_or(0.0, |b| dot(cb, mv(&iw[b], cb)));
                        rows.push(Row { link: k, a, b: l.b, ra, rb, kind: RowKind::Stop(u, sign), mass: [[1.0 / kk.max(1e-300), 0.0, 0.0], ZERO_M[1], ZERO_M[2]], acc: [0.0; 3] });
                    }
                }
                _ => {}
            }
        }
        rows
    }

    fn point_mass(&self, a: usize, b: Option<usize>, ra: V3, rb: V3, iw: &[M3]) -> M3 {
        let ma = self.bodies[a].inv_mass;
        let mut k = mscale(&IDENTITY, ma);
        let sa = skew(ra);
        k = madd(&k, &mscale(&mm(&mm(&sa, &iw[a]), &sa), -1.0));
        if let Some(b) = b {
            let sb = skew(rb);
            k = madd(&k, &mscale(&IDENTITY, self.bodies[b].inv_mass));
            k = madd(&k, &mscale(&mm(&mm(&sb, &iw[b]), &sb), -1.0));
        }
        inverse(&k)
    }

    fn angular_k(&self, a: usize, b: Option<usize>, iw: &[M3]) -> M3 {
        b.map_or(iw[a], |b| madd(&iw[a], &iw[b]))
    }

    fn hinge_axis(&self, k: usize) -> V3 {
        let l = &self.links[k];
        match l.b {
            Some(b) => mv(&qmat(self.state.q[b]), self.joints[k].axis_b),
            None => self.joints[k].axis_b,
        }
    }

    /// Rotation of A relative to B about the hinge axis since the start (radians, right-handed).
    fn hinge_angle(&self, k: usize) -> f64 {
        let l = &self.links[k];
        let qb = l.b.map_or([1.0, 0.0, 0.0, 0.0], |b| self.state.q[b]);
        let rel = qmul(qconj(qb), self.state.q[l.a]);
        let r = qlog(rel);
        dot(r, self.joints[k].axis_b)
    }

    fn velocity_of(&self, i: Option<usize>, r: V3) -> V3 {
        i.map_or([0.0; 3], |i| add(self.state.v[i], cross(self.state.w[i], r)))
    }

    fn solve_row(&mut self, row: &mut Row, iw: &[M3]) {
        let (ia, ib) = (row.a, row.b);
        let w_rel = move |s: &Self| sub(s.state.w[ia], ib.map_or([0.0; 3], |b| s.state.w[b]));
        match row.kind {
            RowKind::Point => {
                let vrel = sub(self.velocity_of(Some(row.a), row.ra), self.velocity_of(row.b, row.rb));
                let impulse = mul(mv(&row.mass, vrel), -1.0);
                row.acc = add(row.acc, impulse);
                // impulse acts on A, its opposite on B
                self.apply_at(row.a, impulse, row.ra, iw);
                if let Some(b) = row.b {
                    self.apply_at(b, mul(impulse, -1.0), row.rb, iw);
                }
            }
            RowKind::Weld => {
                let impulse = mul(mv(&row.mass, w_rel(self)), -1.0);
                row.acc = add(row.acc, impulse);
                self.apply_angular(row.a, impulse, iw);
                if let Some(b) = row.b {
                    self.apply_angular(b, mul(impulse, -1.0), iw);
                }
            }
            RowKind::Axis | RowKind::Damper(_) => {
                let t = row.ra; // the axis is stored in `ra` for angular rows
                let jv = dot(w_rel(self), t);
                let gamma = if let RowKind::Damper(g) = row.kind { g } else { 0.0 };
                let mut d = -row.mass[0][0] * (jv + gamma * row.acc[0]);
                if let Some(sign) = row.limit_sign() {
                    let new = if sign > 0.0 { (row.acc[0] + d).max(0.0) } else { (row.acc[0] + d).min(0.0) };
                    d = new - row.acc[0];
                }
                row.acc[0] += d;
                self.apply_angular(row.a, mul(t, d), iw);
                if let Some(b) = row.b {
                    self.apply_angular(b, mul(t, -d), iw);
                }
            }
            RowKind::Stop(u, sign) => {
                let vrel = dot(sub(self.velocity_of(row.b, row.rb), self.velocity_of(Some(row.a), row.ra)), u);
                let mut d = -row.mass[0][0] * vrel;
                let new = if sign > 0.0 { (row.acc[0] + d).max(0.0) } else { (row.acc[0] + d).min(0.0) };
                d = new - row.acc[0];
                row.acc[0] = new;
                // positive impulse separates the anchors (pushes B along u, A against it)
                self.apply_at(row.a, mul(u, -d), row.ra, iw);
                if let Some(b) = row.b {
                    self.apply_at(b, mul(u, d), row.rb, iw);
                }
            }
        }
    }

    fn apply_at(&mut self, i: usize, impulse: V3, r: V3, iw: &[M3]) {
        if self.bodies[i].inv_mass == 0.0 {
            return;
        }
        self.state.v[i] = add(self.state.v[i], mul(impulse, self.bodies[i].inv_mass));
        self.state.w[i] = add(self.state.w[i], mv(&iw[i], cross(r, impulse)));
    }

    fn correct_joint(&mut self, k: usize, iw: &[M3]) {
        let l = self.links[k].clone();
        let beta = 0.2;
        match l.v.kind {
            0 | 1 | 2 => {
                let (pa, pb, _, _) = self.anchor_state(k);
                let err = sub(pa, pb);
                if norm(err) > 1e-9 {
                    let ra = sub(pa, self.state.x[l.a]);
                    let rb = l.b.map_or([0.0; 3], |b| sub(pb, self.state.x[b]));
                    let impulse = mul(mv(&self.point_mass(l.a, l.b, ra, rb, iw), err), -beta);
                    self.shift(l.a, impulse, ra, iw);
                    if let Some(b) = l.b {
                        self.shift(b, mul(impulse, -1.0), rb, iw);
                    }
                }
                if l.v.kind == 0 {
                    let qb = l.b.map_or([1.0, 0.0, 0.0, 0.0], |b| self.state.q[b]);
                    let target = qmul(qb, self.joints[k].rel);
                    let e = qlog(qmul(self.state.q[l.a], qconj(target)));
                    if norm(e) > 1e-9 {
                        let impulse = mul(mv(&inverse(&self.angular_k(l.a, l.b, iw)), e), -beta);
                        self.shift_angular(l.a, impulse, iw);
                        if let Some(b) = l.b {
                            self.shift_angular(b, mul(impulse, -1.0), iw);
                        }
                    }
                } else if l.v.kind == 2 {
                    let axis_b = self.hinge_axis(k);
                    let axis_a = mv(&qmat(self.state.q[l.a]), self.joints[k].axis_a);
                    let mut e = cross(axis_a, axis_b);
                    let mut pos_limit = 0.0;
                    if l.v.limited {
                        let angle = self.hinge_angle(k);
                        let (lo, hi) = (l.v.lower.to_radians(), l.v.upper.to_radians());
                        if angle < lo {
                            pos_limit = angle - lo;
                        } else if angle > hi {
                            pos_limit = angle - hi;
                        }
                    }
                    e = mul(e, -1.0);
                    e = add(e, mul(axis_b, pos_limit));
                    if norm(e) > 1e-9 {
                        let impulse = mul(mv(&inverse(&self.angular_k(l.a, l.b, iw)), e), -beta);
                        self.shift_angular(l.a, impulse, iw);
                        if let Some(b) = l.b {
                            self.shift_angular(b, mul(impulse, -1.0), iw);
                        }
                    }
                }
            }
            3 if l.v.limited => {
                let (pa, pb, _, _) = self.anchor_state(k);
                let d = sub(pb, pa);
                let len = norm(d);
                if len < 1e-12 {
                    return;
                }
                let u = mul(d, 1.0 / len);
                let c = if len < l.v.lower { len - l.v.lower } else if len > l.v.upper { len - l.v.upper } else { return };
                let ra = sub(pa, self.state.x[l.a]);
                let rb = l.b.map_or([0.0; 3], |b| sub(pb, self.state.x[b]));
                let (ma, mb) = (self.bodies[l.a].inv_mass, l.b.map_or(0.0, |b| self.bodies[b].inv_mass));
                let (ca, cb) = (cross(ra, u), cross(rb, u));
                let kk = ma + mb + dot(ca, mv(&iw[l.a], ca)) + l.b.map_or(0.0, |b| dot(cb, mv(&iw[b], cb)));
                if kk <= 0.0 {
                    return;
                }
                let lambda = -beta * c / kk;
                self.shift(l.a, mul(u, -lambda), ra, iw);
                if let Some(b) = l.b {
                    self.shift(b, mul(u, lambda), rb, iw);
                }
            }
            _ => {}
        }
    }

    /// Break links whose reaction exceeds their thresholds (after the settle time).
    fn snap_links(&mut self, rows: &[Row], spring_force: &[f64]) {
        if self.links.is_empty() || (self.state.since_start as f64) * self.dt < SETTLE_TIME {
            return;
        }
        let dt = self.dt;
        let mut legacy_broke = false;
        for k in 0..self.links.len() {
            let l = &self.links[k];
            if legacy_broke && l.legacy_glue {
                continue; // the historical one-glue-break-per-substep behaviour
            }
            if self.state.broken[k] || (l.strength <= 0.0 && l.v.break_torque <= 0.0) {
                continue;
            }
            let mut lin = [0.0; 3];
            let mut ang = [0.0; 3];
            let mut stops = 0.0;
            for r in rows.iter().filter(|r| r.link == k) {
                match r.kind {
                    RowKind::Point => lin = add(lin, r.acc),
                    RowKind::Weld => ang = add(ang, r.acc),
                    RowKind::Axis | RowKind::Damper(_) => ang = add(ang, mul(r.ra, r.acc[0])),
                    RowKind::Stop(..) => stops += r.acc[0].abs(),
                }
            }
            let (force, torque) = match l.v.kind {
                3 => (spring_force[k] + stops / dt, 0.0),
                0 => {
                    let torque = norm(ang) / dt;
                    (norm(lin) / dt + torque / self.reach[k], torque)
                }
                _ => (norm(lin) / dt, norm(ang) / dt),
            };
            if (l.strength > 0.0 && force > l.strength) || (l.v.break_torque > 0.0 && torque > l.v.break_torque) {
                self.state.broken[k] = true;
                legacy_broke |= l.legacy_glue;
                for (pair, ids) in &self.pairs {
                    if ids.contains(&k) && ids.iter().all(|&i| self.state.broken[i]) {
                        self.state.excluded.remove(pair);
                    }
                }
            }
        }
    }

    fn output(&mut self, target: i64) -> Result<Geometry> {
        let (pos, rot, vel, broken) = self.pose(target)?;
        let mut out = (*self.bodies_geo).clone();
        let mut p: Vec<f32> = out.p().to_vec();
        let mut normals: Vec<(&str, Vec<f32>, usize)> = Vec::new();
        for dom in ["point", "vertex", "prim"] {
            if let Some(a) = out.table(dom).get("N") {
                let rows = match dom { "point" => out.npoints, "vertex" => out.nvertices(), _ => out.nprims() };
                if a.width != 3 || a.rows() != rows || a.as_f32().is_none() {
                    return Err(Error::new("RBD normal N must be a three-component vector"));
                }
                normals.push((dom, a.as_f32().unwrap().to_vec(), rows));
            }
        }
        let original_normals = normals.clone();
        let np = out.nprims();
        let mut v = vec![0.0f32; np * 3];
        let mut w = vec![0.0f32; np * 3];
        for (i, piece) in self.pieces.iter().enumerate() {
            if !piece.from_bodies {
                continue;
            }
            let r = &rot[i];
            for (k, &pt) in piece.points.iter().enumerate() {
                let q = add(mv(r, piece.local[k]), pos[i]);
                for c in 0..3 {
                    p[pt * 3 + c] = q[c] as f32;
                }
            }
            for (slot, (dom, data, _)) in normals.iter_mut().enumerate() {
                let src = &original_normals[slot].1;
                let indices: &[usize] = match *dom { "point" => &piece.points, "vertex" => &piece.corners, _ => &piece.prims };
                for &e in indices {
                    let nn = [src[e * 3] as f64, src[e * 3 + 1] as f64, src[e * 3 + 2] as f64];
                    let rn = mv(r, nn);
                    for c in 0..3 {
                        data[e * 3 + c] = rn[c] as f32;
                    }
                }
            }
            for &f in &piece.prims {
                for c in 0..3 {
                    w[f * 3 + c] = vel[i][c] as f32;
                    v[f * 3 + c] = vel[i][3 + c] as f32;
                }
            }
        }
        out.set_p(p);
        for (dom, data, _) in normals {
            let a = Arc::new(Attr::f32(3, data));
            match dom { "point" => out.point.insert("N".into(), a), "vertex" => out.vertex.insert("N".into(), a), _ => out.prim.insert("N".into(), a) };
        }
        out.prim.insert("rbd_v".into(), Arc::new(Attr::f32(3, v)));
        out.prim.insert("rbd_w".into(), Arc::new(Attr::f32(3, w)));
        if self.settings.include_colliders {
            if let Some(c) = &self.colliders {
                let m = super::get("merge").ok_or_else(|| Error::new("merge node missing"))?;
                let params = Params::resolve(m.params, None)?;
                out = (m.cook)(&[vec![Arc::new(out), c.clone()]], &params)?;
            }
        }
        let mut info = json!({"backend": BACKEND, "version": BACKEND_VERSION, "frame": target, "bodies": self.pieces.len()});
        if !self.links.is_empty() {
            for (kind_index, kind) in CONSTRAINT_KINDS.iter().enumerate() {
                let ids: Vec<usize> = (0..self.links.len()).filter(|&k| self.links[k].v.kind == kind_index).collect();
                if !ids.is_empty() {
                    let broken_links: Vec<usize> = ids.iter().copied().filter(|&k| broken[k]).collect();
                    info[*kind] = json!({"links": ids.len(), "broken": broken_links.len(), "broken_links": broken_links});
                }
            }
        }
        out.detail.insert("rbd".into(), info);
        Ok(out)
    }
}

#[derive(Clone, Copy)]
enum RowKind {
    Point,
    Weld,
    Axis,
    Damper(f64),
    Stop(V3, f64),
}

/// One joint constraint row (or 3-row block). For angular single rows `ra` holds the axis and `rb[0]` the limit sign.
struct Row {
    link: usize,
    a: usize,
    b: Option<usize>,
    ra: V3,
    rb: V3,
    kind: RowKind,
    mass: M3,
    acc: V3,
}

impl Row {
    fn point(link: usize, a: usize, b: Option<usize>, ra: V3, rb: V3, mass: M3) -> Row {
        Row { link, a, b, ra, rb, kind: RowKind::Point, mass, acc: [0.0; 3] }
    }

    #[allow(clippy::too_many_arguments)]
    fn angular(link: usize, a: usize, b: Option<usize>, kind: RowKind, axis: V3, mass: M3, _bias: f64, limited: bool, sign: Option<f64>) -> Row {
        let rb = if limited { [sign.unwrap_or(0.0), 1.0, 0.0] } else { [0.0; 3] };
        Row { link, a, b, ra: axis, rb, kind, mass, acc: [0.0; 3] }
    }

    fn limit_sign(&self) -> Option<f64> {
        (self.rb[1] == 1.0).then_some(self.rb[0])
    }
}

struct Contact {
    key: (usize, usize, usize, usize),
    a: usize,
    b: usize,
    /// normal from A to B (world, at detection)
    n: V3,
    local_a: V3,
    local_b: V3,
    local_n: V3,
    friction: f64,
    restitution: f64,
    ra: V3,
    rb: V3,
    t: [V3; 2],
    mn: f64,
    mt: [f64; 2],
    bias: f64,
    ln: f64,
    lt: [f64; 2],
}

/// Keep the part of polygon `poly` with `dot(side, p) <= d` (Sutherland-Hodgman, one plane).
fn clip(poly: &[V3], side: V3, d: f64) -> Vec<V3> {
    let mut out = Vec::new();
    for k in 0..poly.len() {
        let (p, q) = (poly[k], poly[(k + 1) % poly.len()]);
        let (dp, dq) = (dot(side, p) - d, dot(side, q) - d);
        if dp <= 0.0 {
            out.push(p);
        }
        if (dp < 0.0 && dq > 0.0) || (dp > 0.0 && dq < 0.0) {
            out.push(add(p, mul(sub(q, p), dp / (dp - dq))));
        }
    }
    out
}

/// At most four contact points: the deepest, the farthest from it, then the two spanning the largest area.
fn reduce(pts: Vec<(V3, f64)>, n: V3) -> Vec<(V3, f64)> {
    let first = (0..pts.len()).min_by(|&a, &b| pts[a].1.total_cmp(&pts[b].1)).unwrap();
    let second = (0..pts.len()).max_by(|&a, &b| norm(sub(pts[a].0, pts[first].0)).total_cmp(&norm(sub(pts[b].0, pts[first].0)))).unwrap();
    let area = |k: usize| dot(cross(sub(pts[second].0, pts[first].0), sub(pts[k].0, pts[first].0)), n);
    let third = (0..pts.len()).max_by(|&a, &b| area(a).total_cmp(&area(b))).unwrap();
    let fourth = (0..pts.len()).min_by(|&a, &b| area(a).total_cmp(&area(b))).unwrap();
    let mut keep: Vec<usize> = vec![first, second, third, fourth];
    keep.sort_unstable();
    keep.dedup();
    keep.into_iter().map(|k| pts[k]).collect()
}

/// Closest points of segments p0-p1 and q0-q1.
fn closest_segments(p0: V3, p1: V3, q0: V3, q1: V3) -> (V3, V3) {
    let (d1, d2, r) = (sub(p1, p0), sub(q1, q0), sub(p0, q0));
    let (a, e, f) = (dot(d1, d1), dot(d2, d2), dot(d2, r));
    let (mut s, mut t);
    if a <= 1e-18 && e <= 1e-18 {
        return (p0, q0);
    }
    if a <= 1e-18 {
        s = 0.0;
        t = (f / e).clamp(0.0, 1.0);
    } else {
        let c = dot(d1, r);
        if e <= 1e-18 {
            t = 0.0;
            s = (-c / a).clamp(0.0, 1.0);
        } else {
            let b = dot(d1, d2);
            let denom = a * e - b * b;
            s = if denom > 1e-18 { ((b * f - c * e) / denom).clamp(0.0, 1.0) } else { 0.0 };
            t = (b * s + f) / e;
            if t < 0.0 {
                t = 0.0;
                s = (-c / a).clamp(0.0, 1.0);
            } else if t > 1.0 {
                t = 1.0;
                s = ((b - c) / a).clamp(0.0, 1.0);
            }
        }
    }
    (add(p0, mul(d1, s)), add(q0, mul(d2, t)))
}

// -------------------------------------------------------------------------------------------- worlds and the node

struct Resident {
    signature: u64,
    world: Arc<Mutex<World>>,
}

static WORLDS: Mutex<Vec<Resident>> = Mutex::new(Vec::new());

fn hash_geometry(g: Option<&Geometry>, h: &mut DefaultHasher) {
    let Some(g) = g else { "none".hash(h); return };
    g.npoints.hash(h);
    g.topology.vertex_point.hash(h);
    g.topology.prim_offsets.hash(h);
    g.topology.closed.hash(h);
    for (dom, table) in [("point", &g.point), ("vertex", &g.vertex), ("prim", &g.prim)] {
        for (name, a) in table {
            (dom, name, a.width).hash(h);
            match &a.data {
                AttrData::F32(v) => v.iter().for_each(|x| x.to_bits().hash(h)),
                AttrData::I32(v) => v.hash(h),
            }
        }
    }
    Value::Object(g.detail.clone()).to_string().hash(h);
}

/// Forget every resident world (tests, or to free memory).
pub fn clear_worlds() {
    WORLDS.lock().unwrap_or_else(|e| e.into_inner()).clear();
}

pub(super) fn rbd_solver(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let bodies = RBD_SOLVER.one(inputs, 0)?;
    let colliders = inputs[1].first().cloned();
    let constraints = inputs[2].first().cloned();
    let settings = Settings {
        gravity: vec3(p, "gravity")?,
        start_frame: p.i("start_frame")?,
        substeps: int(p, "substeps", Some(1), Some(128))?,
        iterations: int(p, "iterations", Some(1), Some(500))?,
        margin: float(p, "margin", Some(0.0), Some(1.0))?,
        cache_mb: int(p, "cache_mb", Some(1), Some(1024))? as f64,
        include_colliders: p.b("include_colliders")?,
    };
    let reset = p.i("reset")?;
    let (frame, fps) = (timectx::frame(), timectx::fps());
    if !fps.is_finite() || fps <= 0.0 || !frame.is_finite() {
        return Err(Error::new("RBD frame must be finite and FPS positive"));
    }
    let mut h = DefaultHasher::new();
    (settings.gravity.map(f64::to_bits), settings.start_frame, settings.substeps, settings.iterations, settings.margin.to_bits(),
     settings.cache_mb.to_bits(), settings.include_colliders, reset, fps.to_bits(), BACKEND_VERSION).hash(&mut h);
    for g in [Some(&*bodies), colliders.as_deref(), constraints.as_deref()] {
        hash_geometry(g, &mut h);
    }
    let signature = h.finish();
    let target = settings.start_frame.max(frame.floor() as i64);
    if target - settings.start_frame > 100000 {
        return Err(Error::new("RBD frame range exceeds 100000 frames"));
    }
    let world = {
        let mut worlds = WORLDS.lock().unwrap_or_else(|e| e.into_inner());
        match worlds.iter().position(|r| r.signature == signature) {
            Some(i) => {
                let r = worlds.remove(i);
                let w = r.world.clone();
                worlds.push(r);
                w
            }
            None => {
                drop(worlds);
                let w = Arc::new(Mutex::new(World::new(bodies.clone(), colliders.clone(), constraints.as_deref(), settings)?));
                let mut worlds = WORLDS.lock().unwrap_or_else(|e| e.into_inner());
                worlds.push(Resident { signature, world: w.clone() });
                while worlds.len() > MAX_WORLDS {
                    worlds.remove(0);
                }
                w
            }
        }
    };
    let result = world.lock().unwrap_or_else(|e| e.into_inner()).output(target);
    if result.is_err() {
        WORLDS.lock().unwrap_or_else(|e| e.into_inner()).retain(|r| r.signature != signature);
    }
    result
}

/// Python's evaluator refuses animated inputs to an RBD solver: every node upstream of an `rbd_solver` that `target`
/// needs must be static (no time-dependent node, no nested solver).
pub(crate) fn check_static_inputs(graph: &crate::graph::Graph, target: &str) -> Result<()> {
    fn upstream(graph: &crate::graph::Graph, id: &str, seen: &mut BTreeSet<String>) {
        if !seen.insert(id.to_string()) {
            return;
        }
        if let Some(def) = graph.node(id) {
            for (_, refs) in &def.inputs {
                for r in refs {
                    upstream(graph, r, seen);
                }
            }
        }
    }
    let mut order = BTreeSet::new();
    upstream(graph, target, &mut order);
    for id in &order {
        let Some(def) = graph.node(id) else { continue };
        if def.node_type != "rbd_solver" {
            continue;
        }
        let mut up = BTreeSet::new();
        upstream(graph, id, &mut up);
        up.remove(id);
        for u in &up {
            if let Some(d) = graph.node(u) {
                if super::is_time_dependent(&d.node_type, d.params.as_ref()) {
                    return Err(Error::new("RBD inputs must be static: animated/deforming inputs and nested solvers are unsupported")
                        .in_node(id, "rbd_solver"));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::Graph;

    fn node(t: &str, params: Value, inputs: Value) -> Value {
        json!({"type": t, "params": params, "inputs": inputs})
    }

    fn drop_graph(settings: Value) -> Value {
        json!({"nodes": {
            "box": node("box", json!({"center": [0, 0, 3]}), json!({})),
            "body": node("rbd_configure", json!({}), json!({"geo": "box"})),
            "floor": node("box", json!({"size": [20, 20, 1], "center": [0, 0, -0.5]}), json!({})),
            "sim": node("rbd_solver", settings, json!({"bodies": "body", "colliders": "floor"})),
        }, "display": "sim"})
    }

    fn cook(g: &Value, frame: f64) -> Result<Geometry> {
        timectx::at(Some(frame), None, || Graph::from_value(g).unwrap().cook(None).map(|g| (*g).clone()))
    }

    fn pts(g: &Geometry, n: usize) -> Vec<V3> {
        g.p().chunks(3).take(n).map(|q| [q[0] as f64, q[1] as f64, q[2] as f64]).collect()
    }

    #[test]
    fn ground_contact_rigidity_and_replay() {
        let g = drop_graph(json!({}));
        let initial = cook(&g, 1.0).unwrap();
        let result = cook(&g, 90.0).unwrap();
        let (a, b) = (pts(&initial, 8), pts(&result, 8));
        let zmin = b.iter().map(|p| p[2]).fold(f64::INFINITY, f64::min);
        assert!(zmin > -0.005 && zmin < 0.015, "rest height {zmin}");
        for i in 0..8 {
            for j in 0..8 {
                assert!((norm(sub(a[i], a[j])) - norm(sub(b[i], b[j]))).abs() < 1e-5);
            }
        }
        assert_eq!(result.npoints, 16, "colliders merged");
        assert_eq!(result.detail["rbd"]["frame"], json!(90));
        // stepping frame by frame gives the same result as jumping; fractional frames hold
        let direct = cook(&g, 60.0).unwrap();
        for f in 5..=60 {
            cook(&g, f as f64).unwrap();
        }
        assert_eq!(cook(&g, 60.0).unwrap().p(), direct.p());
        assert_eq!(cook(&g, 60.9).unwrap().p(), direct.p());
        clear_worlds();
        cook(&g, 4.0).unwrap();
        assert_eq!(cook(&g, 60.0).unwrap().p(), direct.p());
    }

    #[test]
    fn spin_damping_and_velocity_attributes() {
        let mut g = drop_graph(json!({"gravity": [0, 0, 0], "include_colliders": false}));
        g["nodes"]["body"]["params"] = json!({"velocity": [3, 0, 0], "mass": 7, "linear_damping": 0});
        let fast = cook(&g, 25.0).unwrap();
        let mean_x = pts(&fast, 8).iter().map(|p| p[0]).sum::<f64>() / 8.0;
        assert!(mean_x > 2.9, "{mean_x}");
        assert_eq!(fast.npoints, 8);
        assert!((fast.prim["rbd_v"].as_f32().unwrap()[0] - 3.0).abs() < 1e-4);
        g["nodes"]["body"]["params"]["linear_damping"] = json!(2);
        let slow = cook(&g, 25.0).unwrap();
        assert!(pts(&slow, 8).iter().map(|p| p[0]).sum::<f64>() / 8.0 < 1.4);
        g["nodes"]["body"]["params"] = json!({"angular_velocity": [0, 0, 2]});
        let spun = cook(&g, 10.0).unwrap();
        assert!(spun.prim["rbd_w"].as_f32().unwrap()[2] > 1.5);
    }

    #[test]
    fn rejects_bad_geometry_and_animated_inputs() {
        for kind in ["torus", "grid"] {
            let mut g = drop_graph(json!({}));
            g["nodes"]["box"] = node(kind, json!({}), json!({}));
            let e = cook(&g, 1.0).unwrap_err();
            assert!(e.0.contains("concave") || e.0.contains("open") || e.0.contains("solid"), "{e}");
        }
        let graph = Graph::from_value(&json!({"nodes": {
            "box": node("box", json!({}), json!({})),
            "s": node("solver", json!({"body": "transform"}), json!({"geo": "box"})),
            "body": node("rbd_configure", json!({}), json!({"geo": "s"})),
            "sim": node("rbd_solver", json!({}), json!({"bodies": "body"})),
        }, "display": "sim"})).unwrap();
        let e = check_static_inputs(&graph, "sim").unwrap_err();
        assert!(e.0.contains("static") && e.0.contains("'sim' (rbd_solver)"), "{e}");
    }

    fn wall(velocity: V3) -> Value {
        let mut nodes = serde_json::Map::new();
        let mut names = Vec::new();
        for r in 0..6 {
            for c in 0..8 {
                let name = format!("b{r}_{c}");
                nodes.insert(name.clone(), node("box", json!({"size": [0.9, 0.4, 0.45], "center": [-3.5 + c as f64, 0, 0.225 + 0.46 * r as f64]}), json!({})));
                names.push(name);
            }
        }
        nodes.insert("row".into(), json!({"type": "merge", "params": {}, "inputs": {"geos": names}}));
        nodes.insert("bricks".into(), node("rbd_configure", json!({"mass": 1, "friction": 0.65}), json!({"geo": "row"})));
        nodes.insert("ball".into(), node("sphere", json!({"radius": 0.45, "rows": 8, "columns": 16, "center": [0.2, -5, 1.4]}), json!({})));
        nodes.insert("projectile".into(), node("rbd_configure", json!({"id_offset": 10000, "mass": 12, "velocity": velocity,
            "angular_velocity": [2, 0, 0], "restitution": 0.15}), json!({"geo": "ball"})));
        nodes.insert("bodies".into(), json!({"type": "merge", "params": {}, "inputs": {"geos": ["bricks", "projectile"]}}));
        nodes.insert("floor".into(), node("box", json!({"size": [80, 80, 0.5], "center": [0, 0, -0.25]}), json!({})));
        nodes.insert("ground".into(), node("rbd_configure", json!({"active": false, "friction": 0.8}), json!({"geo": "floor"})));
        nodes.insert("sim".into(), node("rbd_solver", json!({"substeps": 12}), json!({"bodies": "bodies", "colliders": "ground"})));
        json!({"nodes": nodes, "display": "sim"})
    }

    fn centers(g: &Geometry) -> Vec<V3> {
        pts(g, 384).chunks(8).map(|c| c.iter().fold([0.0; 3], |s, p| add(s, *p)).map(|x| x / 8.0)).collect()
    }

    #[test]
    fn brick_wall_projectile_knockdown() {
        let g = wall([0.0, 12.0, 0.0]);
        let c0 = centers(&cook(&g, 1.0).unwrap());
        let result = cook(&g, 72.0).unwrap();
        let c = centers(&result);
        let moved = c.iter().zip(&c0).filter(|(a, b)| norm(sub(**a, **b)) > 0.3).count();
        assert!(moved > 8, "moved {moved}");
        assert!(c.iter().map(|p| p[1]).fold(f64::NEG_INFINITY, f64::max) > 1.0);
        assert!(result.p().chunks(3).map(|q| q[2] as f64).fold(f64::INFINITY, f64::min) > -0.501);
        assert!(result.prim["rbd_w"].as_f32().unwrap()[..288 * 3].iter().map(|x| x.abs()).fold(0.0, f32::max) > 0.1);
        // a wall with no shot stays upright: the knockdown is caused by impact
        let quiet = centers(&cook(&wall([0.0; 3]), 72.0).unwrap());
        let drift = quiet.iter().zip(&c0).map(|(a, b)| norm(sub(*a, *b))).fold(0.0, f64::max);
        assert!(drift < 0.1, "quiet wall drift {drift}");
    }

    #[test]
    fn bounce_control() {
        let mut peaks = Vec::new();
        for bounce in [0.0, 0.8] {
            let mut g = drop_graph(json!({"substeps": 24}));
            g["nodes"]["body"]["params"] = json!({"restitution": bounce});
            g["nodes"]["ground"] = node("rbd_configure", json!({"active": false, "restitution": bounce}), json!({"geo": "floor"}));
            g["nodes"]["sim"]["inputs"]["colliders"] = json!("ground");
            let heights: Vec<f64> = (22..45).map(|f| pts(&cook(&g, f as f64).unwrap(), 8).iter().map(|p| p[2]).sum::<f64>() / 8.0).collect();
            peaks.push(heights.iter().copied().fold(f64::NEG_INFINITY, f64::max));
        }
        assert!(peaks[1] > peaks[0] + 0.25, "{peaks:?}");
    }

    #[test]
    fn spring_equilibrium_and_hinge_stop() {
        let spring = json!({"nodes": {
            "b": node("box", json!({"size": [0.2, 0.2, 0.2], "center": [0, 0, 1]}), json!({})),
            "body": node("rbd_configure", json!({}), json!({"geo": "b"})),
            "s": node("rbd_constraint", json!({"kind": "spring", "body_a": 0, "world": true, "anchor": [0, 0, 1], "anchor_b": [0, 0, 2],
                "stiffness": 100, "damping": 5, "rest_length": 1}), json!({})),
            "sim": node("rbd_solver", json!({"substeps": 20, "iterations": 100}), json!({"bodies": "body", "constraints": "s"})),
        }, "display": "sim"});
        let z = pts(&cook(&spring, 240.0).unwrap(), 8).iter().map(|p| p[2]).sum::<f64>() / 8.0;
        assert!((2.0 - z - 1.0981).abs() < 0.005, "spring length {}", 2.0 - z);
        let hinge = json!({"nodes": {
            "bar": node("box", json!({"size": [2, 0.2, 0.2], "center": [1, 0, 3]}), json!({})),
            "body": node("rbd_configure", json!({}), json!({"geo": "bar"})),
            "h": node("rbd_constraint", json!({"kind": "hinge", "body_a": 0, "world": true, "anchor": [0, 0, 3], "axis": [0, 1, 0],
                "limited": true, "lower": -25, "upper": 35, "damping": 0.5}), json!({})),
            "sim": node("rbd_solver", json!({"substeps": 20, "iterations": 100}), json!({"bodies": "body", "constraints": "h"})),
        }, "display": "sim"});
        let p = pts(&cook(&hinge, 120.0).unwrap(), 8);
        let c = p.iter().fold([0.0; 3], |s, q| add(s, *q)).map(|x| x / 8.0);
        let angle = (-(c[2] - 3.0)).atan2(c[0]).to_degrees();
        assert!((angle - 35.0).abs() < 0.8, "hinge stop at {angle}");
        assert!(c[1].abs() < 1e-3);
    }

    #[test]
    fn glue_welds_and_breaks() {
        let pair = |world_strength: f64| json!({"nodes": {
            "a": node("box", json!({"center": [0, 0, 3]}), json!({})),
            "b": node("box", json!({"center": [1, 0, 3]}), json!({})),
            "m": {"type": "merge", "params": {}, "inputs": {"geos": ["a", "b"]}},
            "body": node("rbd_configure", json!({}), json!({"geo": "m"})),
            "g": node("rbd_constraint", json!({"kind": "glue", "body_a": 0, "body_b": 1}), json!({})),
            "w": node("rbd_constraint", json!({"kind": "glue", "body_a": 0, "world": true, "strength": world_strength}), json!({})),
            "links": {"type": "merge", "params": {}, "inputs": {"geos": ["g", "w"]}},
            "sim": node("rbd_solver", json!({"substeps": 20, "iterations": 80}), json!({"bodies": "body", "constraints": "links"})),
        }, "display": "sim"});
        // unbreakable world weld on A, A-B glue: the cantilevered pair holds its pose
        let held = cook(&pair(0.0), 48.0).unwrap();
        let (p0, p) = (pts(&cook(&pair(0.0), 1.0).unwrap(), 16), pts(&held, 16));
        let drift = p.iter().zip(&p0).map(|(a, b)| norm(sub(*a, *b))).fold(0.0, f64::max);
        assert!(drift < 0.05, "glued drift {drift}");
        assert_eq!(held.detail["rbd"]["glue"]["links"], json!(2));
        // a weak world weld breaks: both fall, still glued to each other
        let fell = cook(&pair(5.0), 48.0).unwrap();
        assert_eq!(fell.detail["rbd"]["glue"]["broken_links"], json!([1]));
        let p = pts(&fell, 16);
        assert!(p.iter().map(|q| q[2]).fold(f64::NEG_INFINITY, f64::max) < 0.0);
        for i in 0..16 {
            for j in 0..16 {
                assert!((norm(sub(p[i], p[j])) - norm(sub(p0[i], p0[j]))).abs() < 0.02);
            }
        }
    }

    #[test]
    fn pin_holds_until_it_breaks() {
        let bar = |strength: f64| json!({"nodes": {
            "bar": node("box", json!({"size": [2, 0.2, 0.2], "center": [1, 0, 3]}), json!({})),
            "body": node("rbd_configure", json!({"id_offset": 0}), json!({"geo": "bar"})),
            "pin": node("rbd_constraint", json!({"kind": "pin", "body_a": 0, "world": true, "anchor": [0, 0, 3], "strength": strength}), json!({})),
            "sim": node("rbd_solver", json!({"substeps": 20, "iterations": 60}), json!({"bodies": "body", "constraints": "pin"})),
        }, "display": "sim"});
        let held = cook(&bar(0.0), 48.0).unwrap();
        // the pinned end stays at the pivot while the bar swings down
        let p = pts(&held, 8);
        let near = p.iter().map(|q| norm(sub(*q, [0.0, 0.0, 3.0]))).fold(f64::INFINITY, f64::min);
        assert!(near < 0.25, "pivot drift {near}");
        assert!(p.iter().map(|q| q[2]).fold(f64::INFINITY, f64::min) < 2.5);
        assert_eq!(held.detail["rbd"]["pin"]["broken"], json!(0));
        let broke = cook(&bar(5.0), 48.0).unwrap();
        assert_eq!(broke.detail["rbd"]["pin"]["broken"], json!(1));
        assert!(pts(&broke, 8).iter().map(|q| q[2]).fold(f64::NEG_INFINITY, f64::max) < 2.5);
    }
}
