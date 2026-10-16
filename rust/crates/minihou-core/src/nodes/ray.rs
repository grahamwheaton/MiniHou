//! Ray projection and hit attribute transfer from minihou/nodes/ray.py.
use super::{model::*, volume::triangulation, InputSpec, Inputs, NodeSpec};
use crate::{
    error::{Error, Result},
    geometry::{Attr, AttrData, Geometry},
    normals,
    params::Params,
};
use std::sync::Arc;
pub const RAY: NodeSpec = NodeSpec {
    name: "ray",
    inputs: &[
        InputSpec {
            name: "geo",
            multi: false,
            optional: false,
        },
        InputSpec {
            name: "target",
            multi: false,
            optional: false,
        },
    ],
    params: crate::params!("method":"direction";"direction":[0.,0.,-1.];"both_ways":false;"max_distance":1e6;"scale":1.;"lift":0.;"transform_points":true;"import_attribs":"";"delete_misses":false;"group":"";
        "entity":"points";"collision_group":"";"direction_from":"normal";"direction_attribute":"N";
        "show_guide_geometry":false;"intersect_farthest_surface":false;"point_intersection_normal":false;
        "point_intersection_distance":false;"reverse_rays":false;"ray_tolerance":0.01;"bias":0.;
        "max_distance_enabled":true;"sample":1;"jitter_scale":0.;"ray_combiner":"average";"seed":1;
        "create_point_group":true;"ray_hit_group":"rayHit";"prim_num_attribute_enabled":true;"prim_num_attribute":"hitprim";
        "prim_uvw_attribute_enabled":false;"prim_uvw_attribute":"hitprimuv";"import_attributes_from_hits":false;
        "point_attributes":"*";"vertex_attributes":"";"primitive_attributes":"";"detail_attributes":""),
    cook: ray,
};
type V = [f64; 3];
#[derive(Clone, Copy)]
struct Triangle {
    p: [V; 3],
    vertices: [usize; 3],
    prim: usize,
}
#[derive(Clone, Copy)]
struct Hit {
    p: V,
    dist: f64,
    tri: usize,
    bary: V,
}
struct Branch {
    lo: V,
    hi: V,
    left: Option<usize>,
    right: Option<usize>,
    tris: Vec<usize>,
}
struct Target {
    triangles: Vec<Triangle>,
    branches: Vec<Branch>,
    root: Option<usize>,
}
impl Target {
    fn new(g: &Geometry, selected: &[bool]) -> Self {
        let (tv, tp) = triangulation(g);
        let pos = positions(g);
        let triangles = tv
            .into_iter()
            .zip(tp)
            .filter(|(_, p)| selected[*p])
            .map(|(v, p)| Triangle {
                p: v.map(|v| pos[g.topology.vertex_point[v] as usize]),
                vertices: v,
                prim: p,
            })
            .filter(|t| len(cross(sub(t.p[1], t.p[0]), sub(t.p[2], t.p[0]))) > 1e-30)
            .collect::<Vec<_>>();
        let mut out = Self {
            triangles,
            branches: vec![],
            root: None,
        };
        let ids = (0..out.triangles.len()).collect::<Vec<_>>();
        if !ids.is_empty() {
            out.root = Some(out.build(ids));
        }
        out
    }
    fn build(&mut self, mut ids: Vec<usize>) -> usize {
        let mut lo = [f64::INFINITY; 3];
        let mut hi = [f64::NEG_INFINITY; 3];
        for &i in &ids {
            for p in self.triangles[i].p {
                for k in 0..3 {
                    lo[k] = lo[k].min(p[k]);
                    hi[k] = hi[k].max(p[k]);
                }
            }
        }
        let id = self.branches.len();
        self.branches.push(Branch {
            lo,
            hi,
            left: None,
            right: None,
            tris: vec![],
        });
        if ids.len() <= 8 {
            ids.sort_unstable();
            self.branches[id].tris = ids;
        } else {
            let axis = (0..3)
                .max_by(|&a, &b| (hi[a] - lo[a]).total_cmp(&(hi[b] - lo[b])))
                .unwrap();
            ids.sort_by(|&a, &b| {
                let c = |i: usize| self.triangles[i].p.iter().map(|p| p[axis]).sum::<f64>();
                c(a).total_cmp(&c(b)).then(a.cmp(&b))
            });
            let right = ids.split_off(ids.len() / 2);
            let left = self.build(ids);
            let right = self.build(right);
            self.branches[id].left = Some(left);
            self.branches[id].right = Some(right);
        }
        id
    }
    fn cast(&self, o: V, d: V, max: f64, farthest: bool, tolerance: f64) -> Option<Hit> {
        let d = mul(d, 1. / len(d).max(1e-30));
        let mut hit = None;
        let mut best = max;
        let mut stack = Vec::new();
        if let Some(r) = self.root {
            stack.push(r);
        }
        while let Some(i) = stack.pop() {
            let node = &self.branches[i];
            let mut lo = 0f64;
            let mut hi = if farthest { max } else { best };
            for k in 0..3 {
                if d[k] == 0. {
                    if o[k] < node.lo[k] - tolerance || o[k] > node.hi[k] + tolerance {
                        hi = -1.;
                    }
                } else {
                    let a = (node.lo[k] - tolerance - o[k]) / d[k];
                    let b = (node.hi[k] + tolerance - o[k]) / d[k];
                    lo = lo.max(a.min(b));
                    hi = hi.min(a.max(b));
                }
            }
            if lo > hi {
                continue;
            }
            for &ti in &node.tris {
                let t = self.triangles[ti];
                let e1 = sub(t.p[1], t.p[0]);
                let e2 = sub(t.p[2], t.p[0]);
                let pv = cross(d, e2);
                let det = dot(e1, pv);
                if det.abs() <= 1e-14 * (len(e1) * len(e2)).max(1e-300) {
                    continue;
                }
                let inv = 1. / det;
                let tv = sub(o, t.p[0]);
                let u = dot(tv, pv) * inv;
                let qv = cross(tv, e1);
                let v = dot(d, qv) * inv;
                let dist = dot(e2, qv) * inv;
                if dist < 0. || dist > max || (!farthest && dist > best) {
                    continue;
                }
                let hp = add(o, mul(d, dist));
                let weights = if u >= -1e-12 && v >= -1e-12 && u + v <= 1. + 1e-12 {
                    [1. - u - v, u, v]
                } else {
                    let cp = closest_on_tri(hp, t.p);
                    if len(sub(cp, hp)) > tolerance {
                        continue;
                    }
                    bary(cp, t.p)
                };
                if hit.is_none_or(|h: Hit| {
                    (if farthest {
                        dist > h.dist
                    } else {
                        dist < h.dist
                    }) || (dist == h.dist && ti < h.tri)
                }) {
                    best = dist;
                    hit = Some(Hit {
                        p: hp,
                        dist,
                        tri: ti,
                        bary: weights,
                    });
                }
            }
            if let Some(r) = node.right {
                stack.push(r);
            }
            if let Some(l) = node.left {
                stack.push(l);
            }
        }
        hit
    }
    fn closest(&self, p: V, max: f64) -> Option<Hit> {
        let mut best = max;
        let mut hit = None;
        let mut stack = Vec::new();
        if let Some(r) = self.root {
            stack.push(r);
        }
        while let Some(i) = stack.pop() {
            let node = &self.branches[i];
            let gap: V =
                std::array::from_fn(|k| (node.lo[k] - p[k]).max(p[k] - node.hi[k]).max(0.));
            if len(gap) > best {
                continue;
            }
            for &ti in &node.tris {
                let t = self.triangles[ti];
                let cp = closest_on_tri(p, t.p);
                let dist = len(sub(cp, p));
                if dist <= best
                    && hit.is_none_or(|h: Hit| dist < h.dist || (dist == h.dist && ti < h.tri))
                {
                    best = dist;
                    hit = Some(Hit {
                        p: cp,
                        dist,
                        tri: ti,
                        bary: bary(cp, t.p),
                    });
                }
            }
            if let Some(r) = node.right {
                stack.push(r);
            }
            if let Some(l) = node.left {
                stack.push(l);
            }
        }
        hit
    }
}
/// Shared exact triangle ray query for attribute nodes.
pub(super) struct RayMesh(Target);
impl RayMesh {
    pub(super) fn new(g: &Geometry) -> Self { Self(Target::new(g, &vec![true; g.nprims()])) }
    pub(super) fn first_hit_distance(&self, origin: [f64; 3], direction: [f64; 3], limit: f64) -> Option<f64> {
        self.0.cast(origin, direction, limit, false, 0.0).map(|h| h.dist)
    }
    pub(super) fn contains(&self, point:[f64;3], scale:f64)->bool {
        let dirs=[[0.5774,0.4122,0.7046],[-0.3313,0.8661,0.3742],[0.7421,-0.5167,-0.4271]];
        let mut votes=0;let eps=1e-7*scale.max(1e-12);
        for raw in dirs {let d=unit(raw);let mut origin=point;let mut parity=0;
            for _ in 0..1024 {let Some(t)=self.first_hit_distance(origin,d,f64::INFINITY) else{break};
                parity^=1;origin=add(origin,mul(d,t+eps));
            } votes+=parity;
        } votes>=2
    }
}
fn bary(p: V, t: [V; 3]) -> V {
    let a = sub(t[1], t[0]);
    let b = sub(t[2], t[0]);
    let c = sub(p, t[0]);
    let (d00, d01, d11, d20, d21) = (dot(a, a), dot(a, b), dot(b, b), dot(c, a), dot(c, b));
    let den = d00 * d11 - d01 * d01;
    let den = if den.abs() < 1e-30 { 1. } else { den };
    let v = (d11 * d20 - d01 * d21) / den;
    let w = (d00 * d21 - d01 * d20) / den;
    [1. - v - w, v, w]
}
fn closest_on_tri(p: V, t: [V; 3]) -> V {
    let [a, b, c] = t;
    let ab = sub(b, a);
    let ac = sub(c, a);
    let ap = sub(p, a);
    let (d1, d2) = (dot(ab, ap), dot(ac, ap));
    let bp = sub(p, b);
    let (d3, d4) = (dot(ab, bp), dot(ac, bp));
    let cp = sub(p, c);
    let (d5, d6) = (dot(ab, cp), dot(ac, cp));
    let vc = d1 * d4 - d3 * d2;
    let vb = d5 * d2 - d1 * d6;
    let va = d3 * d6 - d5 * d4;
    if d1 <= 0. && d2 <= 0. {
        return a;
    }
    if d3 >= 0. && d4 <= d3 {
        return b;
    }
    if d6 >= 0. && d5 <= d6 {
        return c;
    }
    if vc <= 0. && d1 >= 0. && d3 <= 0. {
        return add(a, mul(ab, d1 / (d1 - d3)));
    }
    if vb <= 0. && d2 >= 0. && d6 <= 0. {
        return add(a, mul(ac, d2 / (d2 - d6)));
    }
    if va <= 0. && d4 - d3 >= 0. && d5 - d6 >= 0. {
        return add(b, mul(sub(c, b), (d4 - d3) / ((d4 - d3) + (d5 - d6))));
    }
    let denom = 1. / (va + vb + vc);
    add(add(a, mul(ab, vb * denom)), mul(ac, vc * denom))
}
fn ray(i: &Inputs, p: &Params) -> Result<Geometry> {
    let g = RAY.one(i, 0)?;
    let target = RAY.one(i, 1)?;
    let method = p.e(
        "method",
        &["direction", "normal", "project_rays", "minimum_distance"],
    )?;
    let entity = p.e("entity", &["points", "primitives"])?;
    let sel = if entity == "points" {
        super::group::mask(&g, "point", &p.s("group")?)?
    } else {
        let prims = super::group::mask(&g, "prim", &p.s("group")?)?;
        let mut points = vec![false; g.npoints];
        for (i, selected) in prims.iter().enumerate() {
            if *selected {
                for v in range(&g, i) {
                    points[g.topology.vertex_point[v] as usize] = true;
                }
            }
        }
        points
    };
    let collision = super::group::mask(&target, "prim", &p.s("collision_group")?)?;
    let grid = Target::new(&target, &collision);
    let (scale, lift, bias, tolerance, jitter) = (
        p.f("scale")?,
        p.f("lift")?,
        p.f("bias")?,
        p.f("ray_tolerance")?,
        p.f("jitter_scale")?,
    );
    let maximum = p.f("max_distance")?;
    if [scale, lift, bias, tolerance, jitter, maximum]
        .iter()
        .any(|x| !x.is_finite())
        || scale < 0.
        || maximum < 0.
        || tolerance < 0.
        || jitter < 0.
    {
        return Err(Error::new("ray scale, max_distance, ray_tolerance and jitter_scale must be finite and nonnegative; lift and bias must be finite"));
    }
    let max = if p.b("max_distance_enabled")? {
        maximum
    } else {
        f64::INFINITY
    };
    let samples = p.i("sample")?;
    if !(1..=65536).contains(&samples) {
        return Err(Error::new("ray sample must be between 1 and 65536"));
    }
    let combiner = p.e(
        "ray_combiner",
        &["average", "median", "shortest", "longest"],
    )?;
    let both = p.b("both_ways")?;
    let reverse = p.b("reverse_rays")?;
    let farthest = p.b("intersect_farthest_surface")?;
    let transform = p.b("transform_points")?;
    let seed = p.i("seed")? as u64;
    let mut out = (*g).clone();
    let pos = positions(&g);
    let mut newp = g.p().to_vec();
    let mut hits = vec![Vec::<Hit>::new(); g.npoints];
    let source = p.e(
        "direction_from",
        &["normal", "specified_vector", "attribute"],
    )?;
    let mut dirs = vec![p.v3("direction")?; g.npoints];
    if method != "minimum_distance"
        && !grid.triangles.is_empty()
        && (sel.iter().any(|s| *s) || method != "project_rays")
    {
        if method == "normal" || (method == "project_rays" && source != "specified_vector") {
            let name = if source == "attribute" && method == "project_rays" {
                p.s("direction_attribute")?
            } else {
                "N".into()
            };
            dirs = if let Some(a) = g.point.get(&name) {
                if a.width != 3 {
                    return Err(Error::new(format!(
                        "ray direction attribute {name} must be a vector"
                    )));
                }
                (0..g.npoints)
                    .map(|i| {
                        std::array::from_fn(|c| match &a.data {
                            AttrData::F32(v) => v[i * 3 + c] as f64,
                            AttrData::I32(v) => v[i * 3 + c] as f64,
                        })
                    })
                    .collect()
            } else if method == "project_rays" && source == "normal" {
                normals::point_normals(g.p(), &g.topology, g.npoints)
                    .chunks_exact(3)
                    .map(|v| v.try_into().unwrap())
                    .map(|v: [f32; 3]| v.map(|x| x as f64))
                    .collect()
            } else {
                return Err(Error::new(format!("ray needs point {name} (method 'normal' needs a point N; add a normal node first)")));
            };
        }
        if dirs
            .iter()
            .enumerate()
            .any(|(i, d)| sel[i] && d.iter().any(|v| !v.is_finite()))
        {
            return Err(Error::new("ray direction must be finite"));
        }
        if !dirs.iter().enumerate().any(|(i, d)| sel[i] && len(*d) > 0.) {
            return Err(Error::new("the ray direction is zero"));
        }
    }
    for k in 0..g.npoints {
        if !sel[k] {
            continue;
        }
        if method == "minimum_distance" {
            if let Some(h) = grid.closest(pos[k], max) {
                hits[k].push(h);
            }
            continue;
        }
        dirs[k] = mul(unit(dirs[k]), if reverse { -1. } else { 1. });
        if len(dirs[k]) < 1e-20 {
            continue;
        }
        let mut rng = seed.wrapping_add((k as u64).wrapping_mul(0x9e3779b97f4a7c15));
        for s in 0..samples {
            let d = if s == 0 || jitter == 0. {
                dirs[k]
            } else {
                // A uniform disk in the tangent plane; independent of threading and platform RNGs.
                let u = unit(cross(
                    dirs[k],
                    if dirs[k][0].abs() < 0.9 {
                        [1., 0., 0.]
                    } else {
                        [0., 1., 0.]
                    },
                ));
                let v = cross(dirs[k], u);
                let radius = random01(&mut rng).sqrt() * jitter;
                let angle = random01(&mut rng) * std::f64::consts::TAU;
                unit(add(
                    dirs[k],
                    add(mul(u, radius * angle.cos()), mul(v, radius * angle.sin())),
                ))
            };
            // Legacy modes retain their exact intersections; tolerance is the new Project Rays control.
            let tol = if method == "project_rays" {
                tolerance
            } else {
                0.
            };
            let f = grid.cast(pos[k], d, max, farthest, tol);
            let h = if both {
                let b = grid.cast(pos[k], mul(d, -1.), max, farthest, tol);
                match (f, b) {
                    (Some(f), Some(b)) => Some(if b.dist < f.dist { b } else { f }),
                    (None, b) => b,
                    (f, None) => f,
                }
            } else {
                f
            };
            if let Some(h) = h {
                hits[k].push(h);
            }
        }
        hits[k].sort_by(|a, b| a.dist.total_cmp(&b.dist).then(a.tri.cmp(&b.tri)));
        if combiner != "average" && !hits[k].is_empty() {
            let index = match combiner.as_str() {
                "shortest" => 0,
                "longest" => hits[k].len() - 1,
                _ => hits[k].len() / 2,
            };
            let h = hits[k][index];
            hits[k] = vec![h];
        }
    }
    let ns = normals::prim_normals(target.p(), &target.topology)
        .into_iter()
        .map(unit32)
        .collect::<Vec<_>>();
    let mut dist = vec![-1.; g.npoints];
    let mut prim = vec![-1; g.npoints];
    let mut uvw = vec![0.; g.npoints * 3];
    let mut hn = vec![0.; g.npoints * 3];
    let mut ok = vec![false; g.npoints];
    for k in 0..g.npoints {
        let hs = &hits[k];
        if hs.is_empty() {
            continue;
        }
        let d = hs.iter().map(|h| h.dist).sum::<f64>() / hs.len() as f64;
        // A primitive number and its UVW must refer to ONE actual sample, even across primitive boundaries.
        let h = hs
            .iter()
            .min_by(|a, b| (a.dist - d).abs().total_cmp(&(b.dist - d).abs()))
            .unwrap();
        let n = mul(
            hs.iter()
                .fold([0.; 3], |n, h| add(n, ns[grid.triangles[h.tri].prim])),
            1. / hs.len() as f64,
        );
        let hp = if method == "minimum_distance" || samples == 1 {
            h.p
        } else {
            let signed = hs
                .iter()
                .map(|h| {
                    if dot(sub(h.p, pos[k]), dirs[k]) < 0. {
                        -h.dist
                    } else {
                        h.dist
                    }
                })
                .sum::<f64>()
                / hs.len() as f64;
            add(pos[k], mul(dirs[k], signed))
        };
        let displacement = sub(hp, pos[k]);
        let ray_dir = if len(displacement) > 1e-20 {
            unit(displacement)
        } else {
            dirs[k]
        };
        let lift_dir = if method == "project_rays" { ray_dir } else { n };
        ok[k] = true;
        dist[k] = d as f32;
        prim[k] = grid.triangles[h.tri].prim as i32;
        // Triangle parametric coordinates (u, v, 0); polygon fans use a documented triangle-local UVW.
        for a in 0..3 {
            uvw[k * 3 + a] = [h.bary[1], h.bary[2], 0.][a] as f32;
            hn[k * 3 + a] = n[a] as f32;
            if transform {
                newp[k * 3 + a] =
                    (pos[k][a] + (hp[a] - pos[k][a]) * scale + lift_dir[a] * lift + n[a] * bias)
                        as f32;
            }
        }
    }
    if transform {
        out.set_p(newp);
    }
    out.point
        .insert("hitdist".into(), Arc::new(Attr::f32(1, dist.clone())));
    out.point
        .insert("hitN".into(), Arc::new(Attr::f32(3, hn.clone())));
    if p.b("point_intersection_distance")? {
        out.point
            .insert("dist".into(), Arc::new(Attr::f32(1, dist)));
    }
    if p.b("point_intersection_normal")? {
        let mut n = g
            .point
            .get("N")
            .filter(|a| a.width == 3)
            .and_then(|a| a.as_f32())
            .map(|v| v.to_vec())
            .unwrap_or_else(|| vec![0.; g.npoints * 3]);
        for k in 0..g.npoints {
            if ok[k] {
                n[k * 3..k * 3 + 3].copy_from_slice(&hn[k * 3..k * 3 + 3]);
            }
        }
        out.point.insert("N".into(), Arc::new(Attr::f32(3, n)));
    }
    for (enabled, name, width, data) in [
        (
            "prim_num_attribute_enabled",
            "prim_num_attribute",
            1,
            AttrData::I32(prim),
        ),
        (
            "prim_uvw_attribute_enabled",
            "prim_uvw_attribute",
            3,
            AttrData::F32(uvw),
        ),
    ] {
        if p.b(enabled)? {
            let name = p.s(name)?;
            super::group::valid_name(&name)?;
            if name == "P" {
                return Err(Error::new("ray hit attributes cannot overwrite P"));
            }
            out.point.insert(name, Arc::new(Attr { width, data }));
        }
    }
    if p.b("create_point_group")? {
        let name = p.s("ray_hit_group")?;
        super::group::valid_name(&name)?;
        write_group(&mut out, "point", &name, &ok)?;
    }
    let legacy = p.s("import_attribs")?;
    let import = p.b("import_attributes_from_hits")?;
    for (dom, table, pattern) in [
        ("point", &target.point, "point_attributes"),
        ("prim", &target.prim, "primitive_attributes"),
        ("vertex", &target.vertex, "vertex_attributes"),
    ] {
        let patterns = if import {
            format!("{} {}", legacy, p.s(pattern)?)
        } else {
            legacy.clone()
        };
        for (name, a) in table {
            if name == "P" || !pattern_matches(&patterns, name) {
                continue;
            }
            let mut base = out
                .point
                .get(name)
                .filter(|b| b.width == a.width && b.dtype() == a.dtype())
                .map(|b| (**b).clone())
                .unwrap_or_else(|| a.zeros_like(g.npoints));
            for k in 0..g.npoints {
                if hits[k].is_empty() {
                    continue;
                }
                for c in 0..a.width {
                    let value = hits[k]
                        .iter()
                        .map(|h| {
                            let t = grid.triangles[h.tri];
                            let ix = if dom == "point" {
                                t.vertices.map(|v| target.topology.vertex_point[v] as usize)
                            } else if dom == "vertex" {
                                t.vertices
                            } else {
                                [t.prim; 3]
                            };
                            match &a.data {
                                AttrData::F32(v) => {
                                    if dom == "prim" {
                                        v[ix[0] * a.width + c] as f64
                                    } else {
                                        (0..3)
                                            .map(|j| v[ix[j] * a.width + c] as f64 * h.bary[j])
                                            .sum()
                                    }
                                }
                                AttrData::I32(v) => v[ix[0] * a.width + c] as f64,
                            }
                        })
                        .sum::<f64>()
                        / hits[k].len() as f64;
                    match &mut base.data {
                        AttrData::F32(v) => v[k * a.width + c] = value as f32,
                        AttrData::I32(v) => v[k * a.width + c] = value as i32,
                    }
                }
            }
            out.prim.remove(name);
            out.vertex.remove(name);
            out.point.insert(name.clone(), Arc::new(base));
        }
    }
    if import {
        let pattern = p.s("detail_attributes")?;
        for (name, value) in &target.detail {
            if pattern_matches(&pattern, name) && ok.iter().any(|v| *v) {
                // Numeric detail attributes broadcast to hit points. Structured/string values stay detail data.
                let values = if let Some(a) = value.as_array() {
                    a.iter().map(|v| v.as_f64()).collect::<Option<Vec<_>>>()
                } else {
                    value.as_f64().map(|v| vec![v])
                };
                if let Some(values) = values.filter(|v| !v.is_empty()) {
                    if name == "P" {
                        continue;
                    }
                    let width = values.len();
                    let mut a = out
                        .point
                        .get(name)
                        .filter(|a| a.width == width && a.as_f32().is_some())
                        .map(|a| a.as_f32().unwrap().to_vec())
                        .unwrap_or_else(|| vec![0.; g.npoints * width]);
                    for k in 0..g.npoints {
                        if ok[k] {
                            for c in 0..width {
                                a[k * width + c] = values[c] as f32;
                            }
                        }
                    }
                    out.point
                        .insert(name.clone(), Arc::new(Attr::f32(width, a)));
                } else {
                    out.detail.insert(name.clone(), value.clone());
                }
            }
        }
    }
    out.detail.remove("__ray_guide_segments");
    if p.b("show_guide_geometry")? {
        // Viewer data only: no added points/primitives, so guides never affect downstream topology.
        // Bound the overlay cost on production meshes; deterministic thinning retains spatial coverage.
        let mut edges = std::collections::BTreeSet::new();
        for (pr, selected) in collision.iter().enumerate() {
            if !*selected {
                continue;
            }
            let r = range(&target, pr);
            for v in r.clone() {
                let w = if v + 1 < r.end {
                    v + 1
                } else if target.topology.is_closed(pr) {
                    r.start
                } else {
                    continue;
                };
                let (a, b) = (
                    target.topology.vertex_point[v] as usize,
                    target.topology.vertex_point[w] as usize,
                );
                edges.insert((a.min(b), a.max(b)));
            }
        }
        let stride = edges.len().div_ceil(20000).max(1);
        let segments: Vec<_> = edges
            .iter()
            .step_by(stride)
            .map(|&(a, b)| {
                [a, b].map(|i| {
                    [
                        target.p()[i * 3],
                        target.p()[i * 3 + 1],
                        target.p()[i * 3 + 2],
                    ]
                })
            })
            .collect();
        out.detail
            .insert("__ray_guide_segments".into(), serde_json::json!(segments));
    }
    if p.b("delete_misses")? {
        let keep: Vec<_> = (0..g.npoints).map(|i| !sel[i] || ok[i]).collect();
        return super::modify::keep_points(&out, &keep);
    }
    Ok(out)
}

fn pattern_matches(pattern: &str, name: &str) -> bool {
    let mut matched = false;
    for pat in pattern
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|p| !p.is_empty())
    {
        if let Some(exclude) = pat.strip_prefix('^') {
            if super::attribs::matches(exclude, name) {
                matched = false;
            }
        } else if super::attribs::matches(pat, name) {
            matched = true;
        }
    }
    matched
}
fn random01(state: &mut u64) -> f64 {
    *state = state.wrapping_add(0x9e3779b97f4a7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
    ((z ^ (z >> 31)) >> 11) as f64 / ((1u64 << 53) as f64)
}

#[cfg(test)]
#[path = "ray_tests.rs"]
mod tests;
