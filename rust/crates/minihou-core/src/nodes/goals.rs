//! Deterministic geometric goal relaxation, ported from `minihou/nodes/procedural_systems.py` (goal_length,
//! goal_collision, goal_surface, goal_gravity, goal_pin, goal_solve). Goal nodes append a JSON goal to the detail list
//! `geometric_goals` and record the topology they were captured on; `goal_solve` replays them with position-based
//! relaxation in float64, in the same order and with the same accumulation as the NumPy code.

use serde_json::{json, Map, Value};

use super::sim_common::{float, topology_digest, vec3};
use super::{InputSpec, Inputs, NodeSpec, GEO};
use crate::error::{Error, Result};
use crate::geometry::Geometry;
use crate::params::{same_value, Params};

const GOALS: &str = "geometric_goals";
const TOPOLOGY: &str = "geometric_goal_topology";

pub const GOAL_LENGTH: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, name: "goal_length", inputs: GEO, params: crate::params!("scale": 1.0; "strength": 1.0), cook: goal_length };
pub const GOAL_COLLISION: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, name: "goal_collision", inputs: GEO, params: crate::params!("radius": 0.25; "strength": 1.0), cook: goal_collision };
pub const GOAL_SURFACE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None,
    name: "goal_surface",
    inputs: &[InputSpec { name: "geo", multi: false, optional: false }, InputSpec { name: "target", multi: false, optional: false }],
    params: crate::params!("group": ""; "strength": 1.0),
    cook: goal_surface,
};
pub const GOAL_GRAVITY: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, name: "goal_gravity", inputs: GEO, params: crate::params!("acceleration": [0.0, 0.0, -9.81]), cook: goal_gravity };
pub const GOAL_PIN: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, name: "goal_pin", inputs: GEO, params: crate::params!("group": "0"), cook: goal_pin };
pub const GOAL_SOLVE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None,
    name: "goal_solve",
    inputs: GEO,
    params: crate::params!("steps": 60; "iterations": 12; "timestep": 0.02; "damping": 0.9; "tolerance": 0.0001),
    cook: goal_solve,
};

type V3 = [f64; 3];

fn positions(g: &Geometry, what: &str) -> Result<Vec<V3>> {
    let p = g.p();
    if p.iter().any(|x| !x.is_finite()) {
        return Err(Error::new(format!("{what} must be finite")));
    }
    Ok(p.chunks_exact(3).map(|q| [q[0] as f64, q[1] as f64, q[2] as f64]).collect())
}

fn norm(a: V3) -> f64 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn signature(g: &Geometry) -> Value {
    json!([g.npoints, topology_digest(&g.topology)])
}

fn append_goal(g: &Geometry, goal: Value) -> Result<Geometry> {
    positions(g, "goal positions")?;
    if g.npoints > 10000 {
        return Err(Error::new("geometric goals support at most 10000 points"));
    }
    let sig = signature(g);
    if g.detail.contains_key(GOALS) && !g.detail.get(TOPOLOGY).is_some_and(|t| same_value(t, &sig)) {
        return Err(Error::new("topology changed after goals were captured; rebuild the goal chain"));
    }
    let mut previous = g.detail.get(GOALS).and_then(Value::as_array).cloned().unwrap_or_default();
    if previous.len() >= 32 {
        return Err(Error::new("at most 32 geometric goal nodes per chain"));
    }
    previous.push(goal);
    let mut out = g.clone();
    out.detail.insert(GOALS.into(), Value::Array(previous));
    out.detail.insert(TOPOLOGY.into(), sig);
    Ok(out)
}

fn selected(g: &Geometry, group: &str) -> Result<Vec<usize>> {
    let m = super::groups::point_mask(g, group)?;
    Ok((0..g.npoints).filter(|&i| m[i]).collect())
}

fn goal_length(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = GOAL_LENGTH.one(inputs, 0)?;
    let scale = float(p, "scale", Some(0.001), Some(100.0))?;
    let strength = float(p, "strength", Some(0.0), Some(1.0))?;
    let e = crate::topo::edges(&g.topology, g.npoints);
    let pos = positions(&g, "goal positions")?;
    let mut edges = Vec::with_capacity(e.len());
    let mut rest = Vec::with_capacity(e.len());
    for k in e.points.chunks_exact(2) {
        let (a, b) = (k[0] as usize, k[1] as usize);
        edges.push(json!([k[0], k[1]]));
        rest.push(json!(norm(sub(pos[b], pos[a])) * scale));
    }
    append_goal(&g, json!({"kind": "length", "edges": edges, "rest": rest, "strength": strength}))
}

fn goal_collision(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = GOAL_COLLISION.one(inputs, 0)?;
    let radius = float(p, "radius", Some(0.0001), Some(1000.0))?;
    let strength = float(p, "strength", Some(0.0), Some(1.0))?;
    if g.npoints > 1000 {
        return Err(Error::new("sphere collision goal supports at most 1000 points"));
    }
    append_goal(&g, json!({"kind": "collision", "radius": radius, "strength": strength}))
}

fn goal_surface(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = GOAL_SURFACE.one(inputs, 0)?;
    let target = GOAL_SURFACE.one(inputs, 1)?;
    let group = p.s("group")?;
    let strength = float(p, "strength", Some(0.0), Some(1.0))?;
    if target.topology.has_open() {
        return Err(Error::new("surface goal requires polygon target faces"));
    }
    let tp = target.p();
    let vp = &target.topology.vertex_point;
    let mut tris: Vec<[V3; 3]> = Vec::new();
    for w in target.topology.prim_offsets.windows(2) {
        let (s, e) = (w[0] as usize, w[1] as usize);
        for k in s + 1..e.saturating_sub(1) {
            let pt = |v: usize| -> V3 { let i = vp[v] as usize * 3; [tp[i] as f64, tp[i + 1] as f64, tp[i + 2] as f64] };
            tris.push([pt(s), pt(k), pt(k + 1)]);
        }
    }
    if tris.iter().flatten().flatten().any(|x| !x.is_finite()) {
        return Err(Error::new("target positions must be finite"));
    }
    if tris.is_empty() || tris.len() > 50000 {
        return Err(Error::new("surface target needs 1 to 50000 triangles"));
    }
    tris.retain(|t| {
        let (u, v) = (sub(t[1], t[0]), sub(t[2], t[0]));
        norm([u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]]) > 1e-12
    });
    if tris.is_empty() {
        return Err(Error::new("surface target has no nondegenerate triangles"));
    }
    let indices = selected(&g, &group)?;
    append_goal(&g, json!({"kind": "surface", "indices": indices, "triangles": tris, "strength": strength}))
}

fn goal_gravity(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = GOAL_GRAVITY.one(inputs, 0)?;
    let a = vec3(p, "acceleration")?;
    append_goal(&g, json!({"kind": "gravity", "acceleration": a}))
}

fn goal_pin(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = GOAL_PIN.one(inputs, 0)?;
    let ids = selected(&g, &p.s("group")?)?;
    let pp = g.p();
    let positions: Vec<Value> = ids.iter().map(|&i| json!([pp[i * 3] as f64, pp[i * 3 + 1] as f64, pp[i * 3 + 2] as f64])).collect();
    append_goal(&g, json!({"kind": "pin", "indices": ids, "positions": positions}))
}

// ------------------------------------------------------------------------------------------------ closest point on triangles

/// Closest point on triangle `t` to `p` (Ericson 5.1.5), the same branch order as `raycast._closest_on_tri`.
fn closest_on_tri(p: V3, t: &[V3; 3]) -> V3 {
    let (a, b, c) = (t[0], t[1], t[2]);
    let dot = |x: V3, y: V3| x[0] * y[0] + x[1] * y[1] + x[2] * y[2];
    let (ab, ac, ap) = (sub(b, a), sub(c, a), sub(p, a));
    let (d1, d2) = (dot(ab, ap), dot(ac, ap));
    let bp = sub(p, b);
    let (d3, d4) = (dot(ab, bp), dot(ac, bp));
    let cp = sub(p, c);
    let (d5, d6) = (dot(ab, cp), dot(ac, cp));
    let vc = d1 * d4 - d3 * d2;
    let vb = d5 * d2 - d1 * d6;
    let va = d3 * d6 - d5 * d4;
    let lin = |o: V3, d: V3, s: f64| [o[0] + d[0] * s, o[1] + d[1] * s, o[2] + d[2] * s];
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return lin(a, ab, d1 / (d1 - d3));
    }
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return lin(a, ac, d2 / (d2 - d6));
    }
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        return lin(b, sub(c, b), (d4 - d3) / ((d4 - d3) + (d5 - d6)));
    }
    let denom = 1.0 / (va + vb + vc);
    let (v, w) = (vb * denom, vc * denom);
    let q = lin(a, ab, v);
    [q[0] + ac[0] * w, q[1] + ac[1] * w, q[2] + ac[2] * w]
}

/// Exact nearest point on a triangle soup: an AABB tree with branch-and-bound (replaces Python's `TriGrid.closest`; both
/// seed the search with the nearest triangle corner and only take strictly nearer candidates).
struct TriTree {
    tris: Vec<[V3; 3]>,
    nodes: Vec<(V3, V3, usize, usize, Option<(usize, usize)>)>, // lo, hi, first, count, children
    order: Vec<usize>,
}

impl TriTree {
    fn new(tris: Vec<[V3; 3]>) -> TriTree {
        let mut t = TriTree { order: (0..tris.len()).collect(), tris, nodes: Vec::new() };
        let n = t.order.len();
        t.build(0, n);
        t
    }

    fn bounds(&self, first: usize, count: usize) -> (V3, V3) {
        let mut lo = [f64::INFINITY; 3];
        let mut hi = [f64::NEG_INFINITY; 3];
        for &i in &self.order[first..first + count] {
            for v in &self.tris[i] {
                for k in 0..3 {
                    lo[k] = lo[k].min(v[k]);
                    hi[k] = hi[k].max(v[k]);
                }
            }
        }
        (lo, hi)
    }

    fn build(&mut self, first: usize, count: usize) -> usize {
        let (lo, hi) = self.bounds(first, count);
        let id = self.nodes.len();
        self.nodes.push((lo, hi, first, count, None));
        if count > 4 {
            let axis = (0..3).max_by(|&a, &b| (hi[a] - lo[a]).total_cmp(&(hi[b] - lo[b]))).unwrap();
            let tris = &self.tris;
            let key = |i: &usize| tris[*i][0][axis] + tris[*i][1][axis] + tris[*i][2][axis];
            self.order[first..first + count].sort_by(|a, b| key(a).total_cmp(&key(b)));
            let half = count / 2;
            let l = self.build(first, half);
            let r = self.build(first + half, count - half);
            self.nodes[id].4 = Some((l, r));
        }
        id
    }

    fn closest(&self, q: V3) -> V3 {
        // the nearest triangle corner bounds the answer (first corner in triangle order on ties, like argmin)
        let mut best = (f64::INFINITY, q);
        for t in &self.tris {
            for &v in t {
                let d = norm(sub(v, q));
                if d < best.0 {
                    best = (d, v);
                }
            }
        }
        if !self.tris.is_empty() {
            self.visit(0, q, &mut best);
        }
        best.1
    }

    fn visit(&self, node: usize, q: V3, best: &mut (f64, V3)) {
        let (lo, hi, first, count, kids) = &self.nodes[node];
        let gap: f64 = (0..3).map(|k| (lo[k] - q[k]).max(q[k] - hi[k]).max(0.0).powi(2)).sum();
        if gap >= best.0 * best.0 {
            return;
        }
        match kids {
            Some((l, r)) => {
                self.visit(*l, q, best);
                self.visit(*r, q, best);
            }
            None => {
                for &i in &self.order[*first..first + count] {
                    let cp = closest_on_tri(q, &self.tris[i]);
                    let d = norm(sub(cp, q));
                    if d < best.0 {
                        *best = (d, cp);
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------- solve

enum Prepared {
    /// edges (a, b), rest lengths; `length` goals compute coincident-point axes in int32 (NumPy's dtype there).
    Pairs { length: bool, a: Vec<usize>, b: Vec<usize>, rest: Vec<f64>, radius: f64, strength: f64 },
    Surface { indices: Vec<usize>, tree: TriTree, strength: f64 },
}

const KINDS: [&str; 4] = ["length", "collision", "surface", "pin"];

fn bad_goal() -> Error {
    Error::new("malformed geometric goal in the detail")
}

fn num(v: &Value) -> Result<f64> {
    v.as_f64().ok_or_else(bad_goal)
}

fn v3(v: &Value) -> Result<V3> {
    let a = v.as_array().filter(|a| a.len() == 3).ok_or_else(bad_goal)?;
    Ok([num(&a[0])?, num(&a[1])?, num(&a[2])?])
}

fn index(v: &Value, n: usize) -> Result<usize> {
    let i = v.as_i64().ok_or_else(bad_goal)?;
    if i < 0 || i as usize >= n {
        return Err(Error::new(format!("goal point index {i} is out of range")));
    }
    Ok(i as usize)
}

fn goal_solve(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = GOAL_SOLVE.one(inputs, 0)?;
    let steps = super::sim_common::int(p, "steps", Some(0), Some(1000))? as usize;
    let iterations = super::sim_common::int(p, "iterations", Some(1), Some(100))? as usize;
    let timestep = float(p, "timestep", Some(0.0001), Some(0.2))?;
    let damping = float(p, "damping", Some(0.0), Some(1.0))?;
    let tolerance = float(p, "tolerance", Some(0.0), Some(1.0))?;
    let mut pos = positions(&g, "solver positions")?;
    if !g.detail.get(TOPOLOGY).is_some_and(|t| same_value(t, &signature(&g))) {
        return Err(Error::new("no compatible goals: connect a goal chain without changing topology"));
    }
    let goals = g.detail.get(GOALS).and_then(Value::as_array).cloned().unwrap_or_default();
    if goals.is_empty() {
        return Err(Error::new("connect at least one geometric goal"));
    }
    let n = pos.len();
    let mut free = vec![1.0f64; n];
    let mut pins: Vec<(usize, V3)> = Vec::new();
    let mut acceleration = [0.0; 3];
    let mut prepared = Vec::new();
    let mut work: u128 = 0;
    for goal in &goals {
        let kind = goal.get("kind").and_then(Value::as_str).unwrap_or("");
        let strength = || goal.get("strength").map_or(Err(bad_goal()), num);
        match kind {
            "pin" => {
                let ids = goal["indices"].as_array().ok_or_else(bad_goal)?;
                let ps = goal["positions"].as_array().ok_or_else(bad_goal)?;
                for (i, q) in ids.iter().zip(ps) {
                    let (i, q) = (index(i, n)?, v3(q)?);
                    if let Some(old) = pins.iter().find(|(j, _)| *j == i) {
                        if (0..3).any(|k| (old.1[k] - q[k]).abs() > 1e-8) {
                            return Err(Error::new("conflicting pin positions"));
                        }
                    }
                    match pins.iter_mut().find(|(j, _)| *j == i) {
                        Some(e) => e.1 = q,
                        None => pins.push((i, q)),
                    }
                }
            }
            "gravity" => {
                let a = v3(&goal["acceleration"])?;
                for k in 0..3 {
                    acceleration[k] += a[k];
                }
            }
            "length" => {
                let edges = goal["edges"].as_array().ok_or_else(bad_goal)?;
                let rest = goal["rest"].as_array().ok_or_else(bad_goal)?;
                if rest.len() != edges.len() {
                    return Err(bad_goal());
                }
                let (mut a, mut b) = (Vec::new(), Vec::new());
                for e in edges {
                    let e = e.as_array().filter(|e| e.len() == 2).ok_or_else(bad_goal)?;
                    a.push(index(&e[0], n)?);
                    b.push(index(&e[1], n)?);
                }
                work += a.len() as u128;
                let rest = rest.iter().map(num).collect::<Result<_>>()?;
                prepared.push(Prepared::Pairs { length: true, a, b, rest, radius: 0.0, strength: strength()? });
            }
            "collision" => {
                let (mut a, mut b) = (Vec::new(), Vec::new());
                for i in 0..n {
                    for j in i + 1..n {
                        a.push(i);
                        b.push(j);
                    }
                }
                work += a.len() as u128;
                prepared.push(Prepared::Pairs { length: false, a, b, rest: Vec::new(), radius: num(&goal["radius"])?, strength: strength()? });
            }
            "surface" => {
                let indices = goal["indices"].as_array().ok_or_else(bad_goal)?.iter().map(|i| index(i, n)).collect::<Result<Vec<_>>>()?;
                let tris = goal["triangles"].as_array().ok_or_else(bad_goal)?.iter().map(|t| {
                    let t = t.as_array().filter(|t| t.len() == 3).ok_or_else(bad_goal)?;
                    Ok([v3(&t[0])?, v3(&t[1])?, v3(&t[2])?])
                }).collect::<Result<Vec<_>>>()?;
                work += indices.len() as u128 * ((tris.len() as f64).sqrt() as u128).max(1);
                prepared.push(Prepared::Surface { indices, tree: TriTree::new(tris), strength: strength()? });
            }
            _ => {}
        }
    }
    if steps.max(1) as u128 * iterations as u128 * work > 50_000_000 {
        return Err(Error::new("geometric solver work budget exceeded; reduce points, steps or iterations"));
    }
    pins.sort_by_key(|(i, _)| *i);
    for (i, _) in &pins {
        free[*i] = 0.0;
    }
    let enforce = |pos: &mut Vec<V3>| {
        for (i, q) in &pins {
            pos[*i] = *q;
        }
    };
    let residuals = |pos: &[V3]| -> [f64; 4] {
        let mut r = [0.0f64; 4];
        for prep in &prepared {
            match prep {
                Prepared::Surface { indices, tree, strength } => {
                    if *strength == 0.0 {
                        continue;
                    }
                    let mut value = 0.0f64;
                    for &i in indices {
                        value = value.max(norm(sub(tree.closest(pos[i]), pos[i])));
                    }
                    r[2] = r[2].max(value);
                }
                Prepared::Pairs { length, a, b, rest, radius, strength } => {
                    if *strength == 0.0 {
                        continue;
                    }
                    let mut value = 0.0f64;
                    for k in 0..a.len() {
                        let l = norm(sub(pos[b[k]], pos[a[k]]));
                        value = value.max(if *length { (l - rest[k]).abs() } else { (2.0 * radius - l).max(0.0) });
                    }
                    let slot = if *length { 0 } else { 1 };
                    r[slot] = r[slot].max(value);
                }
            }
        }
        for (i, q) in &pins {
            r[3] = r[3].max(norm(sub(pos[*i], *q)));
        }
        r
    };
    let mut velocity = vec![[0.0f64; 3]; n];
    let mut displacement = 0.0f64;
    let mut converged = false;
    let mut done = 0usize;
    let mut movement = vec![[0.0f64; 3]; n];
    let mut degree = vec![0usize; n];
    enforce(&mut pos);
    for step in 0..steps {
        let before = pos.clone();
        for i in 0..n {
            let t = timestep * free[i];
            for k in 0..3 {
                velocity[i][k] = velocity[i][k] * (1.0 - damping) + acceleration[k] * timestep;
                pos[i][k] += velocity[i][k] * t;
            }
        }
        enforce(&mut pos);
        for _ in 0..iterations {
            for prep in &prepared {
                match prep {
                    Prepared::Surface { indices, tree, strength } => {
                        if *strength == 0.0 || indices.is_empty() {
                            continue;
                        }
                        let cps: Vec<V3> = indices.iter().map(|&i| tree.closest(pos[i])).collect();
                        for (&i, cp) in indices.iter().zip(cps) {
                            let s = strength * free[i];
                            for k in 0..3 {
                                pos[i][k] += (cp[k] - pos[i][k]) * s;
                            }
                        }
                    }
                    Prepared::Pairs { length, a, b, rest, radius, strength } => {
                        if *strength == 0.0 || a.is_empty() {
                            continue;
                        }
                        movement.iter_mut().for_each(|m| *m = [0.0; 3]);
                        degree.iter_mut().for_each(|d| *d = 0);
                        let mut corrections: Vec<(usize, usize, V3)> = Vec::new();
                        for k in 0..a.len() {
                            let (ia, ib) = (a[k], b[k]);
                            let delta = sub(pos[ib], pos[ia]);
                            let l = norm(delta);
                            let m = l.max(1e-12);
                            let mut dir = [delta[0] / m, delta[1] / m, delta[2] / m];
                            if l < 1e-12 {
                                // stable arbitrary directions also separate exactly coincident particles
                                let axis = if *length {
                                    (ia as i32).wrapping_mul(73856093).wrapping_add((ib as i32).wrapping_mul(19349663)).rem_euclid(3) as usize
                                } else {
                                    ((ia as i64) * 73856093 + (ib as i64) * 19349663).rem_euclid(3) as usize
                                };
                                dir = [0.0; 3];
                                dir[axis] = 1.0;
                            }
                            let error = l - if *length { rest[k] } else { 2.0 * radius };
                            let denominator = free[ia] + free[ib];
                            if (*length || error < 0.0) && denominator > 0.0 {
                                let s = error * strength / denominator;
                                corrections.push((ia, ib, [dir[0] * s, dir[1] * s, dir[2] * s]));
                            }
                        }
                        if corrections.is_empty() {
                            continue;
                        }
                        // np.add.at(movement, a, ...) then np.add.at(movement, b, ...)
                        for &(ia, _, c) in &corrections {
                            for k in 0..3 {
                                movement[ia][k] += c[k] * free[ia];
                            }
                            degree[ia] += 1;
                        }
                        for &(_, ib, c) in &corrections {
                            for k in 0..3 {
                                movement[ib][k] += -c[k] * free[ib];
                            }
                            degree[ib] += 1;
                        }
                        for i in 0..n {
                            let d = degree[i].max(1) as f64;
                            for k in 0..3 {
                                pos[i][k] += movement[i][k] / d;
                            }
                        }
                    }
                }
            }
            enforce(&mut pos);
        }
        if pos.iter().flatten().any(|x| !x.is_finite() || x.abs() > 1e12) {
            return Err(Error::new("geometric solver diverged; reduce timestep or conflicting goals"));
        }
        displacement = 0.0;
        for i in 0..n {
            let d = sub(pos[i], before[i]);
            velocity[i] = [d[0] / timestep, d[1] / timestep, d[2] / timestep];
            displacement = displacement.max(norm(d));
        }
        done = step + 1;
        // only calculate expensive residuals for a candidate settled state
        if displacement <= tolerance && residuals(&pos).iter().copied().fold(f64::NEG_INFINITY, f64::max) <= tolerance {
            converged = true;
            break;
        }
    }
    let report = residuals(&pos);
    let max_residual = report.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let mut out = (*g).clone();
    out.set_p(pos.iter().flatten().map(|&x| x as f32).collect());
    out.point.remove("N");
    out.vertex.remove("N");
    out.prim.remove("N");
    let res: Map<String, Value> = KINDS.iter().zip(report).map(|(k, v)| (k.to_string(), json!(v))).collect();
    out.detail.insert("goal_report".into(), json!({"steps": done, "converged": converged, "max_displacement": displacement,
        "residuals": res, "max_residual": max_residual, "method": "position_based_relaxation"}));
    out.detail.insert("goal_converged".into(), json!(converged));
    out.detail.insert("goal_steps".into(), json!(done));
    out.detail.insert("goal_max_displacement".into(), json!(displacement));
    out.detail.insert("goal_max_residual".into(), json!(max_residual));
    for (k, v) in KINDS.iter().zip(report) {
        out.detail.insert(format!("goal_residual_{k}"), json!(v));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn cook(spec: &NodeSpec, geo: Vec<Geometry>, params: Value) -> Result<Geometry> {
        let inputs: Vec<Vec<Arc<Geometry>>> = geo.into_iter().map(|g| vec![Arc::new(g)]).collect();
        let p = Params::resolve(spec.params, params.as_object())?;
        (spec.cook)(&inputs, &p)
    }

    fn line(n: usize) -> Geometry {
        let p: Vec<f32> = (0..n).flat_map(|i| [i as f32 / (n - 1) as f32 * 2.0, 0.0, 0.0]).collect();
        Geometry::from_arrays(p, (0..n as i32).collect(), vec![0, n as i32], Some(vec![false])).unwrap()
    }

    fn quad() -> Geometry {
        Geometry::from_arrays(vec![-2., -2., 0., 2., -2., 0., 2., 2., 0., -2., 2., 0.], vec![0, 1, 2, 3], vec![0, 4], None).unwrap()
    }

    #[test]
    fn length_pin_converges_and_reports() {
        let g = cook(&GOAL_LENGTH, vec![line(5)], json!({"scale": 0.5})).unwrap();
        let g = cook(&GOAL_PIN, vec![g], json!({"group": "0"})).unwrap();
        let r = cook(&GOAL_SOLVE, vec![g], json!({"steps": 20, "iterations": 10})).unwrap();
        assert_eq!(r.detail["goal_report"]["converged"], json!(true));
        assert_eq!(&r.p()[..3], &[0.0, 0.0, 0.0]);
        assert!(r.detail["goal_residual_length"].as_f64().unwrap() < 1e-4);
    }

    #[test]
    fn errors_and_limits() {
        let g = cook(&GOAL_LENGTH, vec![quad()], json!({})).unwrap();
        let mut changed = g.clone();
        changed.topology = Arc::new(crate::geometry::Topology { vertex_point: vec![0, 1, 2], prim_offsets: vec![0, 3], closed: None });
        assert!(cook(&GOAL_SOLVE, vec![changed.clone()], json!({})).unwrap_err().0.contains("compatible"));
        assert!(cook(&GOAL_GRAVITY, vec![changed], json!({})).unwrap_err().0.contains("topology"));
        let a = cook(&GOAL_SOLVE, vec![cook(&GOAL_GRAVITY, vec![line(2)], json!({})).unwrap()], json!({"steps": 10})).unwrap();
        assert_eq!(a.detail["goal_converged"], json!(false));
        let big = Geometry::points_only(vec![0.0; 3 * 1001]).unwrap();
        assert!(cook(&GOAL_COLLISION, vec![big], json!({})).unwrap_err().0.contains("1000"));
        let pts = cook(&GOAL_COLLISION, vec![Geometry::points_only(vec![0.0; 600]).unwrap()], json!({})).unwrap();
        assert!(cook(&GOAL_SOLVE, vec![pts], json!({"steps": 1000, "iterations": 100})).unwrap_err().0.contains("budget"));
        assert!(cook(&GOAL_SOLVE, vec![line(3)], json!({"steps": -1})).unwrap_err().0.contains("below minimum"));
    }

    #[test]
    fn collision_separates_and_surface_projects() {
        let g = cook(&GOAL_COLLISION, vec![Geometry::points_only(vec![0.0; 6]).unwrap()], json!({"radius": 0.5})).unwrap();
        let a = cook(&GOAL_SOLVE, vec![g], json!({"steps": 20, "iterations": 10})).unwrap();
        assert!(a.detail["goal_report"]["residuals"]["collision"].as_f64().unwrap() < 1e-6);
        let q = a.p();
        assert!(((q[0] - q[3]).powi(2) + (q[1] - q[4]).powi(2) + (q[2] - q[5]).powi(2)).sqrt() >= 0.99999);
        assert!((0..3).all(|k| (q[k] + q[k + 3]).abs() < 2e-7));
        let mut p = line(4);
        p.set_p(vec![0.5, 0.3, 1.0, -0.2, 0.1, -3.0, 1.0, 1.0, 0.5, 3.0, 0.0, 0.2]);
        let g = cook(&GOAL_SURFACE, vec![p, quad()], json!({})).unwrap();
        let a = cook(&GOAL_SOLVE, vec![g], json!({"steps": 10, "damping": 1})).unwrap();
        assert!(a.detail["goal_report"]["residuals"]["surface"].as_f64().unwrap() < 1e-6);
        assert_eq!(a.p()[9..12], [2.0, 0.0, 0.0]);
    }

    #[test]
    fn closest_point_matches_bruteforce() {
        let tris: Vec<[V3; 3]> = (0..50).map(|i| {
            let x = i as f64 * 0.37;
            [[x, 0.0, (i % 3) as f64], [x + 1.0, 0.2, 0.1], [x, 1.0, -0.4]]
        }).collect();
        let tree = TriTree::new(tris.clone());
        for q in [[3.0, 0.4, 2.0], [-1.0, -1.0, 0.0], [10.0, 0.5, -1.0]] {
            let brute = tris.iter().map(|t| closest_on_tri(q, t)).min_by(|a, b| norm(sub(*a, q)).total_cmp(&norm(sub(*b, q)))).unwrap();
            assert!((norm(sub(tree.closest(q), q)) - norm(sub(brute, q))).abs() < 1e-12);
        }
    }
}
