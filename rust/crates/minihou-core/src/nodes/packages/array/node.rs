//! Array copies following Blender 5.2's geometry-nodes Array (Line shape with Relative, Offset and Endpoint spacing).
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

fn layout(g: &Geometry, p: &Params) -> Result<Layout> {
    match p.e("mode", &["line", "grid", "radial"])?.as_str() {
        "line" => line_layout(g, p),
        other => Err(Error::new(format!("array mode {other:?} is not available yet"))),
    }
}

fn cook(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = NODE.one(inputs, 0)?;
    let dist = p.f("merge_distance")?;
    if !dist.is_finite() || dist < 0. { return Err(Error::new("array merge distance must be finite and nonnegative")); }
    let uv = uv_offset(p)?;
    let Layout { xf, mut pairs } = layout(&g, p)?;
    let count = xf.len();
    if g.npoints == 0 { return Ok((*g).clone()); }
    let estimated = g.npoints.checked_mul(count).ok_or_else(|| Error::new("array point count overflow"))?;
    if estimated > crate::geometry::MAX_ELEMENTS { return Err(Error::new("array exceeds geometry point limit")); }
    let mut parts = Vec::with_capacity(count);
    for (k, x) in xf.iter().enumerate() {
        parts.push(Arc::new(transformed(&g, *x, [uv[0] * k as f32, uv[1] * k as f32])?));
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
