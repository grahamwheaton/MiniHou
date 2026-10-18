//! Packed primitives and native instances (stage 4 of docs/TODO_PERFORMANCE_PACKED.md).
//!
//! Representation, shared by Pack, Unpack and Copy to Points (`pack_instances`):
//! * An instance is a loose point (used by no primitive). `P` is its translate.
//! * Point attribute [`XFORM`] (float x 9, row-major 3x3: rotation times scale) is its linear part; missing means identity.
//! * Point attribute [`SOURCE`] (int) is `k + 1` for an instance of `Geometry::packed[k]`, 0 for an ordinary point.
//! * `Geometry::packed` holds each source once as an `Arc<Geometry>`; every instance referencing it shares that pointer.
//! * Every other point attribute on an instance point is a per-instance attribute; unpacking writes it onto the expanded
//!   points (overriding the source's attribute of that name, as Copy to Points does).
//!
//! Because instances are points, nodes that clone their input (Group, Attribute nodes) carry instances through untouched,
//! Transform updates `P` and composes [`XFORM`] ([`transform_instances`]), and Merge concatenates the source lists
//! ([`merge_packed`]). A later per-instance override layer is one more point attribute (or a sparse table keyed by point
//! index) applied in [`expand_instance`]; the representation does not change.

use std::sync::Arc;

use crate::error::{Error, Result};
use crate::geometry::{Attr, AttrData, AttrTable, Geometry, Topology};
use crate::math::{self, M3, IDENTITY};

/// Point attribute: 1-based index into `Geometry::packed`; 0 is an ordinary point.
pub const SOURCE: &str = "packed_source";
/// Point attribute: row-major 3x3 linear transform of an instance.
pub const XFORM: &str = "packed_xform";

/// Number of instance points.
pub fn instance_count(g: &Geometry) -> usize {
    g.point.get(SOURCE).and_then(|a| a.as_i32()).map_or(0, |v| v.iter().filter(|&&k| k > 0).count())
}

/// Source index (0-based) of point `i`, when it is an instance.
pub fn source_of(g: &Geometry, i: usize) -> Option<usize> {
    let k = *g.point.get(SOURCE)?.as_i32()?.get(i)?;
    (k > 0 && (k as usize) <= g.packed.len()).then(|| k as usize - 1)
}

/// Linear part of instance `i` (identity when it has none).
pub fn xform(g: &Geometry, i: usize) -> M3 {
    match g.point.get(XFORM).and_then(|a| a.as_f32().filter(|_| a.width == 9)) {
        Some(v) => std::array::from_fn(|r| std::array::from_fn(|c| v[i * 9 + r * 3 + c] as f64)),
        None => IDENTITY,
    }
}

fn translate(g: &Geometry, i: usize) -> [f64; 3] {
    std::array::from_fn(|k| g.p()[i * 3 + k] as f64)
}

/// World bounding box of instance `i`, from its source's bounds and transform (no vertices are expanded).
pub fn instance_bounds(g: &Geometry, i: usize) -> Option<([f32; 3], [f32; 3])> {
    let src = &g.packed[source_of(g, i)?];
    if src.npoints == 0 {
        return None;
    }
    let (lo, hi) = crate::preview::bounds(src.p());
    let (m, t) = (xform(g, i), translate(g, i));
    let (mut a, mut b) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
    for c in 0..8 {
        let q = std::array::from_fn(|k| if (c >> k) & 1 == 1 { hi[k] as f64 } else { lo[k] as f64 });
        let w = math::apply(&m, q);
        for k in 0..3 {
            a[k] = a[k].min((w[k] + t[k]) as f32);
            b[k] = b[k].max((w[k] + t[k]) as f32);
        }
    }
    Some((a, b))
}

/// Instances of `sources[index[i]]` at `p[i]` with linear part `xf[i]` and per-instance point attributes `attrs`.
/// Sources are deduplicated by pointer, so passing one `Arc` many times stores it once.
pub fn instance_geometry(sources: &[Arc<Geometry>], index: &[usize], p: Vec<f32>, xf: &[M3], attrs: AttrTable) -> Result<Geometry> {
    let mut packed: Vec<Arc<Geometry>> = Vec::new();
    let mut ids = Vec::with_capacity(index.len());
    let mut slot = vec![None; sources.len()];
    for &k in index {
        let s = *slot[k].get_or_insert_with(|| {
            packed.iter().position(|q| Arc::ptr_eq(q, &sources[k])).unwrap_or_else(|| {
                packed.push(sources[k].clone());
                packed.len() - 1
            })
        });
        ids.push(s as i32 + 1);
    }
    let mut g = Geometry::points_only(p)?;
    for (name, a) in attrs {
        if name != "P" {
            g.point.insert(name, a);
        }
    }
    g.point.insert(SOURCE.into(), Arc::new(Attr::i32(1, ids)));
    g.point.insert(XFORM.into(), Arc::new(Attr::f32(9, xf.iter().flat_map(|m| m.iter().flatten().map(|&x| x as f32)).collect())));
    g.packed = Arc::new(packed);
    g.validate()?;
    Ok(g)
}

/// Transform node support: compose `m` onto the linear part of the selected instance points (their `P` is moved by the
/// caller). Geometry without instances is unchanged.
pub fn transform_instances(g: &mut Geometry, m: &M3, selected: &[bool]) {
    if instance_count(g) == 0 {
        return;
    }
    let mut v = Vec::with_capacity(g.npoints * 9);
    for i in 0..g.npoints {
        let x = xform(g, i);
        let x = if source_of(g, i).is_some() && selected.get(i).copied().unwrap_or(true) { math::mul(m, &x) } else { x };
        v.extend(x.iter().flatten().map(|&q| q as f32));
    }
    g.point.insert(XFORM.into(), Arc::new(Attr::f32(9, v)));
}

fn inverse_transpose(m: &M3) -> M3 {
    let c = |r: usize, k: usize| {
        let (r1, r2, k1, k2) = ((r + 1) % 3, (r + 2) % 3, (k + 1) % 3, (k + 2) % 3);
        m[r1][k1] * m[r2][k2] - m[r1][k2] * m[r2][k1]
    };
    let det = (0..3).map(|k| m[0][k] * c(0, k)).sum::<f64>();
    if det.abs() < 1e-30 {
        return IDENTITY;
    }
    std::array::from_fn(|r| std::array::from_fn(|k| c(r, k) / det))
}

/// Instance `i` as real geometry: its source transformed, nested instances composed, per-instance attributes applied.
pub fn expand_instance(g: &Geometry, i: usize) -> Result<Geometry> {
    let k = source_of(g, i).ok_or_else(|| Error::new(format!("point {i} is not a packed instance")))?;
    let mut e = (*g.packed[k]).clone();
    let (m, t) = (xform(g, i), translate(g, i));
    if e.npoints > 0 {
        let p: Vec<f32> = e.p().chunks_exact(3).flat_map(|q| {
            let w = math::apply(&m, [q[0] as f64, q[1] as f64, q[2] as f64]);
            [0, 1, 2].map(|k| (w[k] + t[k]) as f32)
        }).collect();
        e.set_p(p);
        let nm = inverse_transpose(&m);
        for (domain, n) in [("point", e.npoints), ("vertex", e.nvertices())] {
            let table = if domain == "point" { &mut e.point } else { &mut e.vertex };
            let rotated = table.get("N").and_then(|a| a.as_f32().filter(|v| a.width == 3 && v.len() == n * 3)).map(|a| {
                a.chunks_exact(3).flat_map(|q| {
                    let w = math::apply(&nm, [q[0] as f64, q[1] as f64, q[2] as f64]);
                    let l = (w[0] * w[0] + w[1] * w[1] + w[2] * w[2]).sqrt();
                    let l = if l < 1e-20 { 1.0 } else { l };
                    w.map(|x| (x / l) as f32)
                }).collect::<Vec<f32>>()
            });
            if let Some(v) = rotated {
                table.insert("N".into(), Arc::new(Attr::f32(3, v)));
            }
        }
        transform_instances(&mut e, &m, &[]);
    }
    for (name, a) in &g.point {
        if matches!(name.as_str(), "P" | "N" | SOURCE | XFORM) {
            continue;
        }
        let row = super::group::gather(a, &[i]);
        e.point.insert(name.clone(), Arc::new(super::group::gather(&row, &vec![0; e.npoints])));
    }
    Ok(e)
}

/// Points of `g` kept by `keep_point`, with every primitive (none of the dropped points may be used by a primitive).
fn drop_points(g: &Geometry, keep_point: &[bool]) -> Geometry {
    let rows: Vec<usize> = (0..g.npoints).filter(|&i| keep_point[i]).collect();
    let mut remap = vec![-1i32; g.npoints];
    for (n, &i) in rows.iter().enumerate() {
        remap[i] = n as i32;
    }
    let t = &g.topology;
    let mut out = g.clone();
    out.npoints = rows.len();
    out.topology = Arc::new(Topology { vertex_point: t.vertex_point.iter().map(|&v| remap[v as usize]).collect(),
        prim_offsets: t.prim_offsets.clone(), closed: t.closed.clone() });
    out.point = super::group::gather_table(&g.point, &rows);
    out
}

/// Remove the packed attributes and sources when no instance is left.
fn strip_if_unpacked(g: &mut Geometry) {
    if instance_count(g) == 0 {
        g.point.remove(SOURCE);
        g.point.remove(XFORM);
        g.packed = Arc::default();
    }
}

/// Concatenate parts the way Merge does (packed sources included).
pub(super) fn merge_all(parts: Vec<Geometry>) -> Result<Geometry> {
    if parts.len() == 1 {
        return Ok(parts.into_iter().next().unwrap());
    }
    let inputs = [parts.into_iter().map(Arc::new).collect::<Vec<_>>()];
    let p = crate::params::Params::resolve(super::merge::MERGE.params, None)?;
    super::merge::merge(&inputs, &p)
}

/// Expand the selected instance points (all when `selected` is `None`) into real geometry. Other points and primitives
/// are kept as they are, in front of the expanded instances. With `recursive`, nested instances are expanded as well.
pub fn unpack(g: &Geometry, selected: Option<&[bool]>, recursive: bool) -> Result<Geometry> {
    let chosen: Vec<usize> = (0..g.npoints)
        .filter(|&i| source_of(g, i).is_some() && selected.is_none_or(|s| s.get(i).copied().unwrap_or(false))).collect();
    if chosen.is_empty() {
        return Ok(g.clone());
    }
    let mut keep = vec![true; g.npoints];
    for &i in &chosen {
        keep[i] = false;
    }
    let mut rest = drop_points(g, &keep);
    strip_if_unpacked(&mut rest);
    let mut parts = Vec::with_capacity(chosen.len() + 1);
    let has_rest = rest.npoints > 0 || rest.nprims() > 0;
    if has_rest {
        parts.push(rest);
    }
    for &i in &chosen {
        let e = expand_instance(g, i)?;
        parts.push(if recursive { unpack(&e, None, true)? } else { e });
    }
    let mut out = merge_all(parts)?;
    for (k, v) in &g.detail {
        out.detail.entry(k.clone()).or_insert_with(|| v.clone());
    }
    strip_if_unpacked(&mut out);
    Ok(out)
}

/// Merge support: after Merge concatenated `parts`' point tables into `out`, give `out` the union of their sources
/// (deduplicated by pointer) and renumber [`SOURCE`] accordingly.
pub(super) fn merge_packed(parts: &[&Geometry], out: &mut Geometry) {
    if parts.iter().all(|g| g.packed.is_empty()) {
        return;
    }
    let mut packed: Vec<Arc<Geometry>> = Vec::new();
    let mut ids: Vec<i32> = Vec::with_capacity(out.npoints);
    for g in parts {
        let map: Vec<i32> = g.packed.iter().map(|s| {
            packed.iter().position(|q| Arc::ptr_eq(q, s)).unwrap_or_else(|| {
                packed.push(s.clone());
                packed.len() - 1
            }) as i32 + 1
        }).collect();
        match g.point.get(SOURCE).and_then(|a| a.as_i32()) {
            Some(v) => ids.extend(v.iter().map(|&k| if k > 0 { map.get(k as usize - 1).copied().unwrap_or(0) } else { 0 })),
            None => ids.extend(std::iter::repeat_n(0, g.npoints)),
        }
    }
    out.point.insert(SOURCE.into(), Arc::new(Attr::i32(1, ids)));
    out.packed = Arc::new(packed);
}

/// Points of `g` used by `prims`, those prims, and (with `loose`) the points no primitive uses.
fn extract(g: &Geometry, prims: &[usize], loose: bool) -> Geometry {
    let t = &g.topology;
    let mut used = vec![false; g.npoints];
    let mut any = vec![false; g.npoints];
    for &v in &t.vertex_point {
        any[v as usize] = true;
    }
    let mut verts = Vec::new();
    let mut offs = vec![0i32];
    for &i in prims {
        for v in t.prim_offsets[i] as usize..t.prim_offsets[i + 1] as usize {
            used[t.vertex_point[v] as usize] = true;
            verts.push(v);
        }
        offs.push(verts.len() as i32);
    }
    let rows: Vec<usize> = (0..g.npoints).filter(|&i| used[i] || (loose && !any[i])).collect();
    let mut remap = vec![0i32; g.npoints];
    for (n, &i) in rows.iter().enumerate() {
        remap[i] = n as i32;
    }
    let mut out = g.clone();
    out.npoints = rows.len();
    out.topology = Arc::new(Topology { vertex_point: verts.iter().map(|&v| remap[t.vertex_point[v] as usize]).collect(),
        prim_offsets: offs, closed: t.has_open().then(|| prims.iter().map(|&i| t.is_closed(i)).collect()) });
    out.point = super::group::gather_table(&g.point, &rows);
    out.vertex = super::group::gather_table(&g.vertex, &verts);
    out.prim = super::group::gather_table(&g.prim, prims);
    strip_if_unpacked(&mut out);
    out
}

/// Primitives grouped into connected pieces (sharing points), ordered by their first primitive.
fn connected_pieces(g: &Geometry) -> Vec<Vec<usize>> {
    let t = &g.topology;
    let mut parent: Vec<usize> = (0..g.npoints).collect();
    fn find(p: &mut [usize], mut x: usize) -> usize {
        while p[x] != x {
            p[x] = p[p[x]];
            x = p[x];
        }
        x
    }
    for i in 0..g.nprims() {
        let vs = &t.vertex_point[t.prim_offsets[i] as usize..t.prim_offsets[i + 1] as usize];
        for w in vs.windows(2) {
            let (a, b) = (find(&mut parent, w[0] as usize), find(&mut parent, w[1] as usize));
            parent[a] = b;
        }
    }
    let mut piece_of_root = std::collections::HashMap::new();
    let mut pieces: Vec<Vec<usize>> = Vec::new();
    for i in 0..g.nprims() {
        let Some(&v) = t.vertex_point.get(t.prim_offsets[i] as usize).filter(|_| t.prim_offsets[i + 1] > t.prim_offsets[i]) else { continue };
        let r = find(&mut parent, v as usize);
        let n = *piece_of_root.entry(r).or_insert_with(|| { pieces.push(Vec::new()); pieces.len() - 1 });
        pieces[n].push(i);
    }
    pieces
}

/// How [`pack`] splits its input.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum PackMode { All, Group, Connectivity }

/// Pack `g`: each piece becomes one shared source (its points moved so the pivot is the origin) and one instance point at
/// the pivot. `prim_mask` selects the primitives for [`PackMode::Group`]; everything outside the packed pieces stays as is.
pub fn pack(g: &Geometry, mode: PackMode, prim_mask: Option<&[bool]>, centroid_pivot: bool) -> Result<Geometry> {
    let (pieces, rest) = match mode {
        PackMode::All => (vec![g.clone()], None),
        PackMode::Group => {
            let mask = prim_mask.ok_or_else(|| Error::new("pack by group needs a primitive group"))?;
            let (inside, outside): (Vec<usize>, Vec<usize>) = (0..g.nprims()).partition(|&i| mask[i]);
            if inside.is_empty() {
                return Ok(g.clone());
            }
            (vec![extract(g, &inside, false)], Some(extract(g, &outside, true)))
        }
        PackMode::Connectivity => {
            let pieces = connected_pieces(g);
            (pieces.iter().map(|p| extract(g, p, false)).collect(), Some(extract(g, &[], true)))
        }
    };
    let mut sources = Vec::with_capacity(pieces.len());
    let mut p = Vec::with_capacity(pieces.len() * 3);
    for mut piece in pieces {
        let pivot = if centroid_pivot && piece.npoints > 0 {
            let (lo, hi) = crate::preview::bounds(piece.p());
            [0, 1, 2].map(|k| (lo[k] + hi[k]) * 0.5)
        } else { [0.0; 3] };
        if piece.npoints > 0 && pivot != [0.0; 3] {
            let q: Vec<f32> = piece.p().chunks_exact(3).flat_map(|q| [0, 1, 2].map(|k| q[k] - pivot[k])).collect();
            piece.set_p(q);
            // nested instances move with their points; their linear part is unchanged
        }
        p.extend(pivot);
        sources.push(Arc::new(piece));
    }
    let n = sources.len();
    let index: Vec<usize> = (0..n).collect();
    let mut inst = instance_geometry(&sources, &index, p, &vec![IDENTITY; n], AttrTable::new())?;
    inst.detail = g.detail.clone();
    let out = match rest {
        Some(r) if r.npoints > 0 || r.nprims() > 0 => merge_all(vec![r, inst])?,
        _ => inst,
    };
    Ok(out)
}

/// What the viewport draws for `g`: itself when it has no instances, otherwise every instance (nested ones too) placed
/// at its transform and batched with the ordinary geometry into one buffer (CPU batching: one draw for all instances;
/// the graph's geometry keeps one shared source). Instance points themselves are not drawn.
pub fn display_geometry(g: &Geometry) -> Result<std::borrow::Cow<'_, Geometry>> {
    if instance_count(g) == 0 {
        return Ok(std::borrow::Cow::Borrowed(g));
    }
    Ok(std::borrow::Cow::Owned(unpack(g, None, true)?))
}

/// Bytes held by the attribute arrays and topology of `g` itself (sources counted once, by pointer).
pub fn memory_bytes(g: &Geometry) -> usize {
    fn own(g: &Geometry) -> usize {
        let attrs = g.point.values().chain(g.vertex.values()).chain(g.prim.values()).map(|a| match &a.data {
            AttrData::F32(v) => v.len() * 4,
            AttrData::I32(v) => v.len() * 4,
        }).sum::<usize>();
        attrs + (g.topology.vertex_point.len() + g.topology.prim_offsets.len()) * 4
    }
    let mut seen: Vec<*const Geometry> = Vec::new();
    let mut stack = vec![g];
    let mut total = 0;
    while let Some(x) = stack.pop() {
        total += own(x);
        for s in x.packed.iter() {
            if !seen.contains(&Arc::as_ptr(s)) {
                seen.push(Arc::as_ptr(s));
                stack.push(s);
            }
        }
    }
    total
}

#[cfg(test)]
#[path = "packed_tests.rs"]
pub(crate) mod tests;
