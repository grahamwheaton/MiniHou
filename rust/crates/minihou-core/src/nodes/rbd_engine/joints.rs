use super::math::*;
use super::World;
use super::SETTLE_TIME;

/// Per-link data in body frames.
#[derive(Clone)]
pub(crate) struct Joint {
    /// anchor on A / B relative to each body's centre of mass, body frames (B: world point when B is the world)
    pub(crate) ra: V3,
    pub(crate) rb: V3,
    /// glue: relative orientation qB^-1 qA at rest; hinge: axis in A and B frames
    pub(crate) rel: Q,
    pub(crate) axis_a: V3,
    pub(crate) axis_b: V3,
    pub(crate) rest: f64,
}

#[derive(Clone, Copy)]
pub(crate) enum RowKind {
    Point,
    Weld,
    Axis,
    Damper(f64),
    Stop(V3, f64),
}

/// One joint constraint row (or 3-row block). For angular single rows `ra` holds the axis and `rb[0]` the limit sign.
pub(crate) struct Row {
    pub(crate) link: usize,
    pub(crate) a: usize,
    pub(crate) b: Option<usize>,
    pub(crate) ra: V3,
    pub(crate) rb: V3,
    pub(crate) kind: RowKind,
    pub(crate) mass: M3,
    pub(crate) acc: V3,
}

impl Row {
    pub(crate) fn point(link: usize, a: usize, b: Option<usize>, ra: V3, rb: V3, mass: M3) -> Row {
        Row { link, a, b, ra, rb, kind: RowKind::Point, mass, acc: [0.0; 3] }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn angular(link: usize, a: usize, b: Option<usize>, kind: RowKind, axis: V3, mass: M3, _bias: f64, limited: bool, sign: Option<f64>) -> Row {
        let rb = if limited { [sign.unwrap_or(0.0), 1.0, 0.0] } else { [0.0; 3] };
        Row { link, a, b, ra: axis, rb, kind, mass, acc: [0.0; 3] }
    }

    pub(crate) fn limit_sign(&self) -> Option<f64> {
        (self.rb[1] == 1.0).then_some(self.rb[0])
    }
}

impl World {
    // ------------------------------------------------------------------------------------------------------ joints

    pub(crate) fn prepare_joints(&mut self, iw: &[M3]) -> Vec<Row> {
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

    pub(crate) fn solve_row(&mut self, row: &mut Row, iw: &[M3]) {
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

    pub(crate) fn correct_joint(&mut self, k: usize, iw: &[M3]) {
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
    pub(crate) fn snap_links(&mut self, rows: &[Row], spring_force: &[f64]) {
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

}
