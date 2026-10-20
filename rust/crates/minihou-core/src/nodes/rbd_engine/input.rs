use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde_json::Value;

use super::math::*;
use crate::error::{Error, Result};
use crate::geometry::{Attr, AttrData, Geometry};
use crate::nodes::attrib_tools;
use crate::nodes::model;
use crate::nodes::rbd::{validate_link_values, LinkValues};

#[derive(Clone)]
pub(crate) struct PieceValues {
    pub(crate) active: bool,
    pub(crate) mass: f64,
    pub(crate) friction: f64,
    pub(crate) restitution: f64,
    pub(crate) linear_damping: f64,
    pub(crate) angular_damping: f64,
    pub(crate) v: V3,
    pub(crate) w: V3,
}

/// One convex collision part: vertices relative to the piece centre, triangles (mass) and polygon faces (collision).
#[derive(Clone)]
pub(crate) struct Part {
    pub(crate) local: Vec<V3>,
    pub(crate) triangles: Vec<[usize; 3]>,
    pub(crate) faces: Vec<Vec<usize>>,
}

#[derive(Clone)]
pub(crate) struct Piece {
    pub(crate) from_bodies: bool,
    pub(crate) configured: bool,
    pub(crate) id: i64,
    pub(crate) points: Vec<usize>,
    pub(crate) prims: Vec<usize>,
    pub(crate) corners: Vec<usize>,
    pub(crate) center: V3,
    pub(crate) local: Vec<V3>,
    pub(crate) active: bool,
    pub(crate) values: PieceValues,
    pub(crate) parts: Vec<Part>,
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
    let (vals, _) = model::eigen(g);
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
pub(crate) fn pieces(geo: Option<&Geometry>, is_static: bool, from_bodies: bool) -> Result<Vec<Piece>> {
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
    let components: Vec<i64> = attrib_tools::piece_labels(geo).1.iter().map(|&x| x as i64).collect();
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
pub(crate) struct Link {
    pub(crate) a: usize,
    pub(crate) b: Option<usize>,
    pub(crate) strength: f64,
    pub(crate) length: f64,
    pub(crate) v: LinkValues,
    pub(crate) collide_connected: bool,
    pub(crate) legacy_glue: bool,
}

/// `link_table(constraints, pieces)`: primitive links; a missing kind is the original breakable glue.
pub(crate) fn link_table(constraints: Option<&Geometry>, pieces: &[Piece]) -> Result<Vec<Link>> {
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
                if crate::nodes::is_time_dependent(&d.node_type, d.params.as_ref()) {
                    return Err(Error::new("RBD inputs must be static: animated/deforming inputs and nested solvers are unsupported")
                        .in_node(id, "rbd_solver"));
                }
            }
        }
    }
    Ok(())
}
