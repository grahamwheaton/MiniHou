use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::sync::Arc;

use serde_json::{json, Value};

use crate::error::{Error, Result};
use crate::geometry::{Attr, Geometry};
use crate::nodes::rbd::CONSTRAINT_KINDS;
use crate::params::Params;
use crate::timectx;

use super::collision::{shape_of, Contact, Shape};
use super::input::{link_table, pieces, Link, Piece};
use super::joints::{Joint, Row};
use super::math::*;
use super::{BACKEND, BACKEND_VERSION};

#[derive(Clone)]
pub(crate) struct Body {
    pub(crate) inv_mass: f64,
    pub(crate) inv_inertia: M3, // body frame
    /// piece centre (Python's body origin) relative to the centre of mass, body frame
    pub(crate) origin: V3,
    pub(crate) shapes: Vec<Shape>,
    pub(crate) friction: f64,
    pub(crate) restitution: f64,
    pub(crate) linear_damping: f64,
    pub(crate) angular_damping: f64,
}

#[derive(Clone)]
pub(crate) struct State {
    pub(crate) x: Vec<V3>,
    pub(crate) q: Vec<Q>,
    pub(crate) v: Vec<V3>,
    pub(crate) w: Vec<V3>,
    pub(crate) broken: Vec<bool>,
    pub(crate) excluded: BTreeSet<(usize, usize)>,
    pub(crate) since_start: u64,
    pub(crate) frame: i64,
    pub(crate) warm: HashMap<(usize, usize, usize, usize), Vec<(V3, f64, f64, f64)>>,
}


pub(crate) struct Settings {
    pub(crate) gravity: V3,
    pub(crate) start_frame: i64,
    pub(crate) substeps: i64,
    pub(crate) iterations: i64,
    pub(crate) margin: f64,
    pub(crate) cache_mb: f64,
    pub(crate) include_colliders: bool,
}

type Pose = (Vec<V3>, Vec<M3>, Vec<[f64; 6]>, Vec<bool>);

pub(crate) struct World {
    pub(crate) settings: Settings,
    pub(crate) bodies_geo: Arc<Geometry>,
    pub(crate) colliders: Option<Arc<Geometry>>,
    pub(crate) pieces: Vec<Piece>,
    pub(crate) links: Vec<Link>,
    pub(crate) reach: Vec<f64>,
    pub(crate) bodies: Vec<Body>,
    pub(crate) joints: Vec<Joint>,
    /// linked pairs whose contacts are off while one of their non-colliding links holds
    pub(crate) pairs: BTreeMap<(usize, usize), Vec<usize>>,
    pub(crate) initial: State,
    pub(crate) state: State,
    pub(crate) dt: f64,
    pub(crate) frames: BTreeMap<i64, Pose>,
    pub(crate) order: VecDeque<i64>,
    pub(crate) cache_bytes: usize,
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


impl World {
    pub(crate) fn new(bodies_geo: Arc<Geometry>, colliders: Option<Arc<Geometry>>, constraints: Option<&Geometry>, settings: Settings) -> Result<World> {
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
    pub(crate) fn anchor_state(&self, k: usize) -> (V3, V3, V3, V3) {
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

    pub(crate) fn apply_angular(&mut self, i: usize, impulse: V3, iw: &[M3]) {
        if self.bodies[i].inv_mass == 0.0 {
            return;
        }
        self.state.w[i] = add(self.state.w[i], mv(&iw[i], impulse));
    }


    /// Move a body by a position-level impulse.
    pub(crate) fn shift(&mut self, i: usize, impulse: V3, r: V3, iw: &[M3]) {
        let m = self.bodies[i].inv_mass;
        if m == 0.0 {
            return;
        }
        self.state.x[i] = add(self.state.x[i], mul(impulse, m));
        self.state.q[i] = qrotate(self.state.q[i], mv(&iw[i], cross(r, impulse)));
    }

    pub(crate) fn shift_angular(&mut self, i: usize, impulse: V3, iw: &[M3]) {
        if self.bodies[i].inv_mass == 0.0 {
            return;
        }
        self.state.q[i] = qrotate(self.state.q[i], mv(&iw[i], impulse));
    }


    pub(crate) fn output(&mut self, target: i64) -> Result<Geometry> {
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
                let m = crate::nodes::get("merge").ok_or_else(|| Error::new("merge node missing"))?;
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


