//! Array copies following Blender 5.2's geometry-nodes Array (Line, Grid and Radial shapes with Relative, Offset and Endpoint spacing).
use std::{collections::{BTreeSet, HashMap}, sync::Arc};
use serde_json::{Map, Value};
use crate::{error::{Error, Result}, geometry::{Attr, AttrData, Geometry, Topology},
    math::{self, M3, IDENTITY}, nodes::{merge::MERGE, model, packed, InputSpec, Inputs, NodeSpec}, params::Params};

pub(super) const NODE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "array",
    inputs: &[InputSpec { name: "geo", multi: false, optional: false }],
    params: crate::params!(
        "mode": "line"; "spacing": "relative"; "count": 2;
        "relative_offset": [1., 0., 0.]; "constant_offset": [0., 0., 0.]; "endpoint": [1., 0., 0.];
        "count_x": 2; "count_y": 2; "count_z": 1;
        "radial_mode": "count"; "distance": 1.0; "radius": 1.0; "axis": "z"; "angle": 360.0; "align_rotation": true;
        "random_seed": 0; "random_position": [0., 0., 0.]; "random_rotation": [0., 0., 0.];
        "random_scale": 0.0; "uniform_scale": true; "random_uv": 0.0;
        "legacy_fit_length": -1.0;
        "merge_vertices": true; "merge_distance": 0.0; "merge_first_last": false;
        "uv_offset_u": 0.0; "uv_offset_v": 0.0
    ),
    cook,
};
const MAX_COUNT: usize = 10_000;

/// Older graphs used the Array (Legacy) modifier's `fit_type`. Fixed Count keeps its offsets; Fit Length becomes
/// Endpoint spacing whose endpoint is found at cook time from the old step and length, so the geometry is unchanged.
pub(crate) fn migrate(given: &Map<String, Value>) -> Result<Option<Map<String, Value>>> {
    if !given.contains_key("fit_type") && !given.contains_key("fit_length") { return Ok(None); }
    let mut g = given.clone();
    let fit = g.remove("fit_type").and_then(|v| v.as_str().map(str::to_string)).unwrap_or_else(|| "fixed_count".into());
    let length = g.remove("fit_length").and_then(|v| v.as_f64()).unwrap_or(1.0);
    match fit.as_str() {
        "fixed_count" => {},
        "fit_length" => {
            // A spacing set after the old key was written wins (the editor keeps unknown keys in the document).
            if !g.contains_key("spacing") {
                g.insert("spacing".into(), Value::from("endpoint"));
                g.entry("legacy_fit_length").or_insert(Value::from(length.max(0.0)));
            }
        },
        other => return Err(Error::new(format!("array fit_type {other:?} was removed; use Line spacing instead"))),
    }
    Ok(Some(g))
}

#[derive(Clone, Copy)]
pub(super) struct Affine { pub m: M3, pub t: [f64; 3] }
impl Affine {
    fn identity() -> Self { Self { m: IDENTITY, t: [0.; 3] } }
    fn translate(t: [f64; 3]) -> Self { Self { m: IDENTITY, t } }
    fn point(self, p: [f64; 3]) -> [f64; 3] {
        let q = math::apply(&self.m, p);
        std::array::from_fn(|a| q[a] + self.t[a])
    }
    fn inverse(self) -> Option<Self> {
        let m = self.m;
        let co = |r: usize, k: usize| {
            let (r1,r2,k1,k2) = ((r+1)%3,(r+2)%3,(k+1)%3,(k+2)%3);
            m[r1][k1]*m[r2][k2]-m[r1][k2]*m[r2][k1]
        };
        let det: f64 = (0..3).map(|k| m[0][k]*co(0,k)).sum();
        if !det.is_finite() || det.abs() < 1e-12 { return None; }
        let inv: M3 = std::array::from_fn(|r| std::array::from_fn(|k| co(k,r)/det));
        Some(Self { m: inv, t: math::apply(&inv, self.t).map(|x| -x) })
    }
}

fn finite3(v: [f64; 3]) -> bool { v.iter().all(|x| x.is_finite()) }

fn uv_offset(p: &Params) -> Result<[f32; 2]> {
    let a = p.f("uv_offset_u")?;
    let b = p.f("uv_offset_v")?;
    if !a.is_finite() || !b.is_finite() || a.abs() > f32::MAX as f64 || b.abs() > f32::MAX as f64 {
        return Err(Error::new("uv_offset needs finite float32 numbers"));
    }
    Ok([a as f32, b as f32])
}

fn transformed(g: &Geometry, xf: Affine, uv: [f32; 2]) -> Result<Geometry> {
    let mut out = g.clone();
    let mut positions = Vec::with_capacity(g.npoints * 3);
    for q in g.p().chunks_exact(3) {
        let p = xf.point([q[0] as f64, q[1] as f64, q[2] as f64]);
        if p.iter().any(|v| !v.is_finite() || v.abs() > f32::MAX as f64) { return Err(Error::new("array position exceeds float32 range")); }
        positions.extend(p.map(|v| v as f32));
    }
    out.set_p(positions);
    if packed::instance_count(g) > 0 { packed::transform_instances(&mut out, &xf.m, &[]); }
    if xf.m != IDENTITY {
        let normal_matrix = xf.inverse().map(|inv| math::transpose(&inv.m)).unwrap_or(xf.m)
            .map(|row| row.map(|x| x as f32));
        for (source, target) in [(&g.point, &mut out.point), (&g.vertex, &mut out.vertex)] {
            if let Some(attr) = source.get("N") {
                let values = attr.as_f32().filter(|_| attr.width == 3)
                    .ok_or_else(|| Error::new("array normal attribute N must be float3"))?;
                let values = crate::xform::linear_normalized(values, normal_matrix);
                target.insert("N".into(), Arc::new(Attr::f32(3, values)));
            }
        }
    }
    if uv != [0., 0.] {
        for (name, attr) in &mut out.vertex {
            let name = name.to_ascii_lowercase();
            if attr.width != 2 || !(name == "uv" || name.starts_with("uv_")) { continue; }
            let Some(data) = attr.as_f32() else { continue; };
            let mut data = data.to_vec();
            for row in data.chunks_exact_mut(2) { row[0] += uv[0]; row[1] += uv[1]; }
            *attr = Arc::new(Attr::f32(2, data));
        }
    }
    Ok(out)
}

/// Merge's general path omits edge groups and crease flags; array copies have
/// disjoint point ranges, so their sorted edge rows concatenate in part order.
fn restore_edge_attributes(out: &mut Geometry, parts: &[Arc<Geometry>]) -> Result<()> {
    let names: BTreeSet<_> = parts.iter().flat_map(|g| g.edge.keys().cloned()).collect();
    for name in names {
        if out.edge.contains_key(&name) { continue; }
        let template = parts.iter().find_map(|g| g.edge.get(&name)).unwrap();
        let mut values = template.zeros_like(0);
        for part in parts {
            let attr = part.edge.get(&name);
            if attr.is_some_and(|a| a.width != template.width || a.dtype() != template.dtype()) {
                return Err(Error::new(format!("edge attribute {name:?} differs between array inputs")));
            }
            let zero;
            let row = match attr { Some(a) => a.as_ref(), None => {
                zero = template.zeros_like(part.topology.nedges()); &zero
            }};
            match (&mut values.data, &row.data) {
                (AttrData::F32(dst), AttrData::F32(src)) => dst.extend_from_slice(src),
                (AttrData::I32(dst), AttrData::I32(src)) => dst.extend_from_slice(src),
                _ => unreachable!(),
            }
        }
        out.edge.insert(name, Arc::new(values));
    }
    Ok(())
}

fn bounds(g: &Geometry) -> [f64; 3] {
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for q in g.p().chunks_exact(3) { for a in 0..3 { lo[a] = lo[a].min(q[a] as f64); hi[a] = hi[a].max(q[a] as f64); } }
    std::array::from_fn(|a| if g.npoints == 0 { 0. } else { hi[a] - lo[a] })
}

fn checked_count(p: &Params, name: &str) -> Result<usize> {
    let n = p.i(name)?;
    if n < 1 || n > MAX_COUNT as i64 { return Err(Error::new(format!("{name} must be 1..{MAX_COUNT}"))); }
    Ok(n as usize)
}

/// The copy layout: one transform per copy and the copy pairs whose touching points may weld.
pub(super) struct Layout { pub xf: Vec<Affine>, pub pairs: Vec<(usize, usize)> }

fn chain(n: usize) -> Vec<(usize, usize)> { (1..n).map(|k| (k - 1, k)).collect() }

fn line_layout(g: &Geometry, p: &Params) -> Result<Layout> {
    let rel = p.v3("relative_offset")?;
    let cst = p.v3("constant_offset")?;
    let end = p.v3("endpoint")?;
    if !finite3(rel) || !finite3(cst) || !finite3(end) { return Err(Error::new("array offsets must be finite")); }
    let size = bounds(g);
    let spacing = p.e("spacing", &["relative", "offset", "endpoint"])?;
    let (count, step) = match spacing.as_str() {
        // Relative also adds the constant offset, as the legacy modifier did, so migrated graphs keep their shape.
        "relative" => (checked_count(p, "count")?, std::array::from_fn(|a| rel[a] * size[a] + cst[a])),
        "offset" => (checked_count(p, "count")?, cst),
        _ => {
            let legacy = p.f("legacy_fit_length")?;
            if legacy.is_finite() && legacy >= 0. {
                // Migrated Fit Length: the old step and length give the count; the endpoint is the last copy.
                let step: [f64; 3] = std::array::from_fn(|a| rel[a] * size[a] + cst[a]);
                let d = step.iter().map(|x| x*x).sum::<f64>().sqrt();
                let n = if d <= 1e-6 { 1 } else { ((legacy + 1e-6)/d).floor().min((MAX_COUNT-1) as f64) as usize + 1 };
                (n, step)
            } else {
                let n = checked_count(p, "count")?;
                let div = if n > 1 { (n - 1) as f64 } else { 1. };
                (n, end.map(|x| x / div))
            }
        },
    };
    let xf = (0..count).map(|k| Affine::translate(step.map(|x| x * k as f64))).collect();
    Ok(Layout { xf, pairs: chain(count) })
}

/// Grid: copies on a lattice. Each axis steps by the matching component of the spacing vector: Relative multiplies
/// the input size on that axis (plus the constant offset), Offset is a distance, Endpoint is the far corner.
fn grid_layout(g: &Geometry, p: &Params) -> Result<Layout> {
    let rel = p.v3("relative_offset")?;
    let cst = p.v3("constant_offset")?;
    let end = p.v3("endpoint")?;
    if !finite3(rel) || !finite3(cst) || !finite3(end) { return Err(Error::new("array offsets must be finite")); }
    let n = [checked_count(p, "count_x")?, checked_count(p, "count_y")?, checked_count(p, "count_z")?];
    let total = n[0].checked_mul(n[1]).and_then(|v| v.checked_mul(n[2])).filter(|&v| v <= MAX_COUNT)
        .ok_or_else(|| Error::new(format!("array grid needs at most {MAX_COUNT} copies")))?;
    let size = bounds(g);
    let step: [f64; 3] = match p.e("spacing", &["relative", "offset", "endpoint"])?.as_str() {
        "relative" => std::array::from_fn(|a| rel[a] * size[a] + cst[a]),
        "offset" => cst,
        _ => std::array::from_fn(|a| if n[a] > 1 { end[a] / (n[a] - 1) as f64 } else { 0. }),
    };
    let mut xf = Vec::with_capacity(total);
    let mut pairs = Vec::new();
    let id = |i: usize, j: usize, k: usize| (k * n[1] + j) * n[0] + i;
    for k in 0..n[2] { for j in 0..n[1] { for i in 0..n[0] {
        xf.push(Affine::translate([i as f64 * step[0], j as f64 * step[1], k as f64 * step[2]]));
        if i > 0 { pairs.push((id(i-1,j,k), id(i,j,k))); }
        if j > 0 { pairs.push((id(i,j-1,k), id(i,j,k))); }
        if k > 0 { pairs.push((id(i,j,k-1), id(i,j,k))); }
    } } }
    Ok(Layout { xf, pairs })
}

/// Rotation by `theta` radians about the unit X, Y or Z axis.
fn axis_rotation(axis: usize, theta: f64) -> M3 {
    let (s, c) = theta.sin_cos();
    let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
    let mut m = IDENTITY;
    m[u][u] = c; m[u][v] = -s; m[v][u] = s; m[v][v] = c;
    m
}

/// Radial: copies around a circle about the X, Y or Z axis through the origin. Copy 0 sits at `radius` along the
/// next axis (X for Z, Y for X, Z for Y). A full turn spaces copies evenly without doubling the first; a partial
/// angle puts the last copy at the end of the arc. Align Rotation turns each copy to face along the circle.
fn radial_layout(p: &Params) -> Result<Layout> {
    let radius = p.f("radius")?;
    let angle = p.f("angle")?;
    if !radius.is_finite() || !angle.is_finite() { return Err(Error::new("array radius and angle must be finite")); }
    let axis_name = p.e("axis", &["x", "y", "z"])?;
    let axis = ["x", "y", "z"].iter().position(|a| *a == axis_name.as_str()).unwrap();
    let full = angle.abs() >= 360. - 1e-9;
    let count = match p.e("radial_mode", &["count", "distance"])?.as_str() {
        "count" => checked_count(p, "count")?,
        _ => {
            let d = p.f("distance")?;
            if !d.is_finite() || d <= 0. { return Err(Error::new("array radial distance must be positive")); }
            let arc = radius.abs() * angle.abs().min(360.).to_radians();
            let n = (arc / d + 1e-9).floor();
            (if full { n } else { n + 1. }).clamp(1., MAX_COUNT as f64) as usize
        },
    };
    let sweep = angle.clamp(-360., 360.).to_radians();
    let div = if full { count as f64 } else if count > 1 { (count - 1) as f64 } else { 1. };
    let mut base = [0.; 3];
    base[(axis + 1) % 3] = radius;
    let align = p.b("align_rotation")?;
    let xf = (0..count).map(|k| {
        let r = axis_rotation(axis, sweep * k as f64 / div);
        let t = math::apply(&r, base);
        Affine { m: if align { r } else { IDENTITY }, t }
    }).collect();
    Ok(Layout { xf, pairs: chain(count) })
}

/// A stable number in [0, 1) for (seed, copy, channel): adding copies never changes the existing ones.
fn hash01(seed: i64, copy: usize, channel: u64) -> f64 {
    let mut z = (seed as u64) ^ (copy as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ channel.wrapping_mul(0xD1B5_4A32_D192_ED03);
    for _ in 0..2 {
        z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
    }
    (z >> 11) as f64 / (1u64 << 53) as f64
}
fn signed(seed: i64, copy: usize, channel: u64) -> f64 { hash01(seed, copy, channel) * 2. - 1. }

/// Per-copy randomness, applied in the copy's own space before the layout places it (copy 0 included).
pub(super) struct Random { seed: i64, pos: [f64; 3], rot: [f64; 3], scale: f64, uniform: bool, uv: f64 }
impl Random {
    fn read(p: &Params) -> Result<Self> {
        let r = Self { seed: p.i("random_seed")?, pos: p.v3("random_position")?, rot: p.v3("random_rotation")?,
            scale: p.f("random_scale")?, uniform: p.b("uniform_scale")?, uv: p.f("random_uv")? };
        if !finite3(r.pos) || !finite3(r.rot) || !r.scale.is_finite() || r.scale < 0. || !r.uv.is_finite() {
            return Err(Error::new("array random ranges must be finite and random scale nonnegative"));
        }
        Ok(r)
    }
    fn local(&self, k: usize) -> Affine {
        let t = std::array::from_fn(|a| self.pos[a] * signed(self.seed, k, a as u64));
        let rot = math::euler_xyz(std::array::from_fn(|a| self.rot[a] * signed(self.seed, k, 3 + a as u64)));
        let sc: [f64; 3] = std::array::from_fn(|a| {
            let ch = if self.uniform { 6 } else { 6 + a as u64 };
            (1. + self.scale * signed(self.seed, k, ch)).max(1e-4)
        });
        Affine { m: std::array::from_fn(|r| std::array::from_fn(|c| rot[r][c] * sc[c])), t }
    }
    fn uv(&self, k: usize) -> [f32; 2] {
        [(self.uv * signed(self.seed, k, 9)) as f32, (self.uv * signed(self.seed, k, 10)) as f32]
    }
    fn active(&self) -> bool { self.pos != [0.; 3] || self.rot != [0.; 3] || self.scale != 0. }
}
fn compose(outer: Affine, inner: Affine) -> Affine {
    let t = math::apply(&outer.m, inner.t);
    Affine { m: math::mul(&outer.m, &inner.m), t: std::array::from_fn(|a| t[a] + outer.t[a]) }
}

fn layout(g: &Geometry, p: &Params) -> Result<Layout> {
    match p.e("mode", &["line", "grid", "radial"])?.as_str() {
        "line" => line_layout(g, p),
        "grid" => grid_layout(g, p),
        "radial" => radial_layout(p),
        other => Err(Error::new(format!("array mode {other:?} is not available yet"))),
    }
}

fn cook(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = NODE.one(inputs, 0)?;
    let dist = p.f("merge_distance")?;
    if !dist.is_finite() || dist < 0. { return Err(Error::new("array merge distance must be finite and nonnegative")); }
    let uv = uv_offset(p)?;
    let Layout { mut xf, mut pairs } = layout(&g, p)?;
    let random = Random::read(p)?;
    if random.active() { for (k, x) in xf.iter_mut().enumerate() { *x = compose(*x, random.local(k)); } }
    let count = xf.len();
    if g.npoints == 0 { return Ok((*g).clone()); }
    let estimated = g.npoints.checked_mul(count).ok_or_else(|| Error::new("array point count overflow"))?;
    if estimated > crate::geometry::MAX_ELEMENTS { return Err(Error::new("array exceeds geometry point limit")); }
    let mut parts = Vec::with_capacity(count);
    for (k, x) in xf.iter().enumerate() {
        parts.push(Arc::new(transformed(&g, *x, {
            let r = random.uv(k);
            [uv[0] * k as f32 + r[0], uv[1] * k as f32 + r[1]]
        })?));
    }
    let mut merged = (MERGE.cook)(&[parts.clone()], p)?;
    restore_edge_attributes(&mut merged, &parts)?;
    if p.b("merge_vertices")? && dist > 0. {
        if p.b("merge_first_last")? && count > 1 { pairs.push((count - 1, 0)); }
        weld(merged, g.npoints, &pairs, dist)
    } else { Ok(merged) }
}

/// Weld each point of copy `b` to the nearest point of copy `a` within `dist`, for every pair `(a, b)`.
fn weld(g: Geometry, n: usize, pairs: &[(usize, usize)], dist: f64) -> Result<Geometry> {
    let p = g.p();
    let pos = |i: usize| [p[3*i] as f64, p[3*i+1] as f64, p[3*i+2] as f64];
    let cell = |q: [f64; 3]| q.map(|x| (x/dist).floor() as i64);
    let mut parent: Vec<usize> = (0..g.npoints).collect();
    let root = |parent: &Vec<usize>, mut i: usize| { while parent[i] != i { i = parent[i]; } i };
    let mut pair = |from: std::ops::Range<usize>, to: std::ops::Range<usize>| {
        let mut grid: HashMap<[i64; 3], Vec<usize>> = HashMap::new();
        for i in from { grid.entry(cell(pos(i))).or_default().push(i); }
        for i in to {
            let q = pos(i); let c = cell(q);
            let mut best: Option<(f64, usize)> = None;
            for dx in -1..=1 { for dy in -1..=1 { for dz in -1..=1 {
                if let Some(v) = grid.get(&[c[0]+dx,c[1]+dy,c[2]+dz]) {
                    for &j in v {
                        let r = pos(j);
                        let d2 = (0..3).map(|a| (q[a]-r[a]).powi(2)).sum::<f64>();
                        if d2 <= dist*dist && best.is_none_or(|b| d2 < b.0) { best = Some((d2,j)); }
                    }
                }
            } } }
            if let Some((_,j)) = best { let a = root(&parent,i); let b = root(&parent,j); if a != b { parent[a] = b; } }
        }
    };
    for &(a, b) in pairs { pair(a*n..(a+1)*n, b*n..(b+1)*n); }
    let mut keep = Vec::new();
    let mut remap = vec![0i32; g.npoints];
    for i in 0..g.npoints { if root(&parent,i) == i { remap[i] = keep.len() as i32; keep.push(i); } }
    for i in 0..g.npoints { remap[i] = remap[root(&parent,i)]; }
    let mut out = Geometry { packed: g.packed.clone(), edge: Default::default(), npoints: keep.len(),
        topology: Arc::new(Topology { vertex_point: g.topology.vertex_point.iter().map(|&v| remap[v as usize]).collect(),
            prim_offsets: g.topology.prim_offsets.clone(), closed: g.topology.closed.clone(), ..Default::default() }),
        point: model::gather(&g.point, &keep), vertex: g.vertex, prim: g.prim, detail: g.detail };
    if !g.edge.is_empty() {
        let old = crate::topo::edges(&g.topology, g.npoints);
        let new = crate::topo::edges(&out.topology, out.npoints);
        let mut source = HashMap::new();
        for (i, pair) in old.points.chunks_exact(2).enumerate() {
            let a = remap[pair[0] as usize];
            let b = remap[pair[1] as usize];
            source.entry((a.min(b), a.max(b))).or_insert(i);
        }
        let indices: Vec<usize> = new.points.chunks_exact(2)
            .map(|pair| source[&(pair[0] as i32, pair[1] as i32)]).collect();
        out.edge = model::gather(&g.edge, &indices);
    }
    out.validate()?;
    Ok(out)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
