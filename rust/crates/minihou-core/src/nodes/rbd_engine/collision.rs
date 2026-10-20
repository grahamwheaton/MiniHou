use std::collections::BTreeSet;

use super::input::Part;
use super::math::*;
use super::World;

#[derive(Clone)]
pub(crate) struct Shape {
    /// vertices relative to the body's centre of mass, in the body frame
    pub(crate) verts: Vec<V3>,
    pub(crate) faces: Vec<Vec<usize>>,
    pub(crate) normals: Vec<V3>,
    /// unique edge directions (body frame) and the edges (vertex pairs) themselves
    pub(crate) edges: Vec<(usize, usize)>,
    pub(crate) edge_dirs: Vec<V3>,
    pub(crate) radius: f64,
}

pub(crate) fn shape_of(part: &Part, com: V3) -> Shape {
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

pub(crate) struct Contact {
    pub(crate) key: (usize, usize, usize, usize),
    pub(crate) a: usize,
    pub(crate) b: usize,
    /// normal from A to B (world, at detection)
    pub(crate) n: V3,
    pub(crate) local_a: V3,
    pub(crate) local_b: V3,
    pub(crate) local_n: V3,
    pub(crate) friction: f64,
    pub(crate) restitution: f64,
    pub(crate) ra: V3,
    pub(crate) rb: V3,
    pub(crate) t: [V3; 2],
    pub(crate) mn: f64,
    pub(crate) mt: [f64; 2],
    pub(crate) bias: f64,
    pub(crate) ln: f64,
    pub(crate) lt: [f64; 2],
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

impl World {
    // ---------------------------------------------------------------------------------------------------- collision

    pub(crate) fn collide(&self) -> Vec<Contact> {
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

    pub(crate) fn prepare_contact(&mut self, c: &mut Contact, iw: &[M3]) {
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

    pub(crate) fn solve_contact(&mut self, c: &mut Contact, iw: &[M3]) {
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

    pub(crate) fn correct_contact(&mut self, c: &Contact, iw: &[M3]) {
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

}
