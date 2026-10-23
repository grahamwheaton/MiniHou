//! Array copies using Blender 5.2's Array (Legacy) modifier placement rules.
use std::{collections::{BTreeSet, HashMap}, sync::Arc};
use crate::{error::{Error, Result}, geometry::{Attr, AttrData, Geometry, Topology},
    math::{self, M3, IDENTITY}, nodes::{merge::MERGE, model, packed, InputSpec, Inputs, NodeSpec}, params::Params};

pub(super) const NODE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "array",
    inputs: &[
        InputSpec { name: "geo", multi: false, optional: false },
        InputSpec { name: "object_offset", multi: false, optional: true },
        InputSpec { name: "start_cap", multi: false, optional: true },
        InputSpec { name: "end_cap", multi: false, optional: true },
    ],
    params: crate::params!(
        "fit_type": "fixed_count"; "count": 2; "fit_length": 1.0;
        "relative_offset": [1., 0., 0.]; "constant_offset": [0., 0., 0.];
        "merge_vertices": true; "merge_distance": 0.0; "merge_first_last": false;
        "uv_offset_u": 0.0; "uv_offset_v": 0.0
    ),
    cook,
};
const MAX_COUNT: usize = 10_000;

#[derive(Clone, Copy)]
struct Affine { m: M3, t: [f64; 3] }
impl Affine {
    fn identity() -> Self { Self { m: IDENTITY, t: [0.; 3] } }
    fn then(self, step: Self) -> Self {
        let q = math::apply(&step.m, self.t);
        Self { m: math::mul(&step.m, &self.m), t: std::array::from_fn(|a| q[a] + step.t[a]) }
    }
    fn point(self, p: [f64; 3]) -> [f64; 3] {
        let q = math::apply(&self.m, p);
        std::array::from_fn(|a| q[a] + self.t[a])
    }
    fn inverse(self) -> Result<Self> {
        let m = self.m;
        let co = |r: usize, k: usize| {
            let (r1,r2,k1,k2) = ((r+1)%3,(r+2)%3,(k+1)%3,(k+2)%3);
            m[r1][k1]*m[r2][k2]-m[r1][k2]*m[r2][k1]
        };
        let det: f64 = (0..3).map(|k| m[0][k]*co(0,k)).sum();
        if !det.is_finite() || det.abs() < 1e-12 { return Err(Error::new("array step is not invertible for start cap")); }
        let inv: M3 = std::array::from_fn(|r| std::array::from_fn(|k| co(k,r)/det));
        Ok(Self { m: inv, t: math::apply(&inv, self.t).map(|x| -x) })
    }
}

fn object_transform(inputs: &Inputs) -> Result<Affine> {
    let Some(g) = inputs.get(1).and_then(|s| s.first()) else { return Ok(Affine::identity()); };
    if g.npoints != 1 { return Err(Error::new("object_offset needs exactly one point or packed instance")); }
    let t = std::array::from_fn(|a| g.p()[a] as f64);
    let m = if packed::source_of(g, 0).is_some() { packed::xform(g, 0) } else { IDENTITY };
    if !t.iter().chain(m.iter().flatten()).all(|x| x.is_finite()) { return Err(Error::new("object_offset transform must be finite")); }
    Ok(Affine { m, t })
}

fn uv_offset(p: &Params) -> Result<[f32; 2]> {
    let a = p.f("uv_offset_u")?;
    let b = p.f("uv_offset_v")?;
    if !a.is_finite() || !b.is_finite() || a.abs() > f32::MAX as f64 || b.abs() > f32::MAX as f64 {
        return Err(Error::new("uv_offset needs finite float32 numbers"));
    }
    Ok([a as f32, b as f32])
}

fn transformed(g: &Geometry, xf: Affine, copy: usize, uv: [f32; 2]) -> Result<Geometry> {
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
    if copy > 0 && uv != [0., 0.] {
        for (name, attr) in &mut out.vertex {
            let name = name.to_ascii_lowercase();
            if attr.width != 2 || !(name == "uv" || name.starts_with("uv_")) { continue; }
            let Some(data) = attr.as_f32() else { continue; };
            let mut data = data.to_vec();
            for row in data.chunks_exact_mut(2) {
                row[0] += uv[0] * copy as f32;
                row[1] += uv[1] * copy as f32;
            }
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

fn cook(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = NODE.one(inputs, 0)?;
    let rel = p.v3("relative_offset")?;
    let cst = p.v3("constant_offset")?;
    let dist = p.f("merge_distance")?;
    let length = p.f("fit_length")?;
    let uv = uv_offset(p)?;
    if !rel.iter().chain(&cst).all(|x| x.is_finite()) || !dist.is_finite() || dist < 0. || !length.is_finite() || length < 0. {
        return Err(Error::new("array offsets, merge distance and fit length must be finite; distances nonnegative"));
    }
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for q in g.p().chunks_exact(3) { for a in 0..3 { lo[a] = lo[a].min(q[a] as f64); hi[a] = hi[a].max(q[a] as f64); } }
    let mut step = object_transform(inputs)?;
    for a in 0..3 { step.t[a] += rel[a] * if g.npoints == 0 { 0. } else { hi[a]-lo[a] } + cst[a]; }
    let fit = p.e("fit_type", &["fixed_count", "fit_length", "fit_curve"])?;
    let count = match fit.as_str() {
        "fixed_count" => {
            let n = p.i("count")?;
            if n < 1 || n > MAX_COUNT as i64 { return Err(Error::new(format!("count must be 1..{MAX_COUNT}"))); }
            n as usize
        },
        "fit_length" => {
            let d = step.t.iter().map(|x| x*x).sum::<f64>().sqrt();
            if d <= 1e-6 { 1 } else { ((length + 1e-6)/d).floor().min((MAX_COUNT-1) as f64) as usize + 1 }
        },
        _ => return Err(Error::new("Fit Curve needs a curve path input and is deferred")),
    };
    let start = inputs.get(2).and_then(|s| s.first());
    let end = inputs.get(3).and_then(|s| s.first());
    if g.npoints == 0 && start.is_none() && end.is_none() { return Ok((*g).clone()); }
    let estimated = g.npoints.checked_mul(count).and_then(|n| n.checked_add(start.map_or(0, |x| x.npoints)))
        .and_then(|n| n.checked_add(end.map_or(0, |x| x.npoints))).ok_or_else(|| Error::new("array point count overflow"))?;
    if estimated > crate::geometry::MAX_ELEMENTS { return Err(Error::new("array exceeds geometry point limit")); }
    let mut parts = Vec::with_capacity(count + usize::from(start.is_some()) + usize::from(end.is_some()));
    let mut xf = Affine::identity();
    for k in 0..count { parts.push(Arc::new(transformed(&g, xf, k, uv)?)); xf = xf.then(step); }
    if let Some(cap) = start { parts.push(Arc::new(transformed(cap, step.inverse()?, 0, [0.,0.])?)); }
    if let Some(cap) = end { parts.push(Arc::new(transformed(cap, xf, 0, [0.,0.])?)); }
    let mut merged = (MERGE.cook)(&[parts.clone()], p)?;
    restore_edge_attributes(&mut merged, &parts)?;
    if p.b("merge_vertices")? && dist > 0. {
        weld(merged, g.npoints, count, start.map_or(0, |x| x.npoints),
            end.map_or(0, |x| x.npoints), dist, p.b("merge_first_last")?)
    } else { Ok(merged) }
}

fn weld(g: Geometry, n: usize, count: usize, start: usize, end: usize, dist: f64, first_last: bool) -> Result<Geometry> {
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
    for k in 1..count { pair((k-1)*n..k*n, k*n..(k+1)*n); }
    if first_last && count > 1 { pair(0..n, (count-1)*n..count*n); }
    if start > 0 { pair(0..n, count*n..count*n+start); }
    if end > 0 { pair((count-1)*n..count*n, count*n+start..count*n+start+end); }
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
