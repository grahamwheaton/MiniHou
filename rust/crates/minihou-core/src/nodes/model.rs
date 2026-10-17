//! Faithful polygon operations from minihou/nodes/model.py. Shared builders follow _build.py.
use super::{GEO, Inputs, NodeSpec, groups};
use crate::{
    error::{Error, Result},
    geometry::{Attr, AttrData, AttrTable, Geometry, Topology},
    normals,
    params::Params,
    topo,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

pub(super) fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|k| a[k] * b[k]).sum()
}
pub(super) fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|k| a[k] + b[k])
}
pub(super) fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|k| a[k] - b[k])
}
pub(super) fn mul(a: [f64; 3], s: f64) -> [f64; 3] {
    a.map(|v| v * s)
}
pub(super) fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
pub(super) fn len(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}
pub(super) fn unit(a: [f64; 3]) -> [f64; 3] {
    let n = len(a);
    mul(a, 1.0 / if n < 1e-20 { 1.0 } else { n })
}
// Python normalize_rows returns float32 even when its input is float64.
pub(super) fn unit32(a: [f64; 3]) -> [f64; 3] {
    unit(a).map(|v| v as f32 as f64)
}
pub(super) fn positions(g: &Geometry) -> Vec<[f64; 3]> {
    g.p()
        .chunks_exact(3)
        .map(|p| [p[0] as f64, p[1] as f64, p[2] as f64])
        .collect()
}
pub(super) fn range(g: &Geometry, p: usize) -> std::ops::Range<usize> {
    g.topology.prim_offsets[p] as usize..g.topology.prim_offsets[p + 1] as usize
}
pub(super) fn corners(g: &Geometry) -> (Vec<usize>, Vec<usize>, Vec<usize>) {
    let mut pv = vec![0; g.nvertices()];
    let mut nx = pv.clone();
    let mut pr = pv.clone();
    for p in 0..g.nprims() {
        let r = range(g, p);
        for v in r.clone() {
            pv[v] = p;
            nx[v] = if v + 1 < r.end { v + 1 } else { r.start };
            pr[v] = if v > r.start { v - 1 } else { r.end - 1 };
        }
    }
    (pv, nx, pr)
}
pub(super) fn polygons(g: &Geometry, what: &str) -> Result<()> {
    if g.topology.has_open() {
        return Err(Error::new(format!(
            "{what} works on polygons, but the input has open curves; remove them first"
        )));
    }
    Ok(())
}
pub(super) fn gather(t: &AttrTable, ix: &[usize]) -> AttrTable {
    t.iter()
        .map(|(n, a)| {
            let w = a.width;
            let b = match &a.data {
                AttrData::F32(v) => Attr::f32(
                    w,
                    ix.iter()
                        .flat_map(|&i| v[i * w..(i + 1) * w].iter().copied())
                        .collect(),
                ),
                AttrData::I32(v) => Attr::i32(
                    w,
                    ix.iter()
                        .flat_map(|&i| v[i * w..(i + 1) * w].iter().copied())
                        .collect(),
                ),
            };
            (n.clone(), Arc::new(b))
        })
        .collect()
}

/// A merged mesh has a corner UV set if any input does. Materialize point UV
/// fallback on the other inputs before concatenating their vertex tables.
pub(super) fn promote_corner_uv_sets(geos: &mut [Geometry]) {
    let names: BTreeSet<_> = geos.iter().flat_map(|g| g.vertex.iter())
        .filter(|(name, a)| !matches!(name.as_str(), "N" | "Cd" | "P") && matches!(a.width, 2 | 3) && a.as_f32().is_some())
        .map(|(name, _)| name.clone()).collect();
    for g in geos {
        for name in &names {
            if g.vertex.contains_key(name) { continue; }
            if let Some((width, values)) = g.point.get(name).filter(|a| matches!(a.width, 2 | 3))
                .and_then(|a| a.as_f32().map(|v| (a.width, v))) {
                let data = g.topology.vertex_point.iter().flat_map(|&p| values[p as usize * width..(p as usize + 1) * width].iter().copied()).collect();
                g.vertex.insert(name.clone(), Arc::new(Attr::f32(width, data)));
            }
        }
    }
}
pub(super) fn concat(a: &AttrTable, b: &AttrTable) -> AttrTable {
    a.iter()
        .filter_map(|(n, x)| {
            let y = b.get(n)?;
            let data = match (&x.data, &y.data) {
                (AttrData::F32(v), AttrData::F32(w)) => {
                    AttrData::F32(v.iter().chain(w).copied().collect())
                }
                (AttrData::I32(v), AttrData::I32(w)) => {
                    AttrData::I32(v.iter().chain(w).copied().collect())
                }
                _ => return None,
            };
            Some((
                n.clone(),
                Arc::new(Attr {
                    width: x.width,
                    data,
                }),
            ))
        })
        .collect()
}
pub(super) fn lerp(t: &AttrTable, ia: &[usize], ib: &[usize], fr: &[f64]) -> AttrTable {
    t.iter()
        .map(|(n, a)| {
            let w = a.width;
            let b = match &a.data {
                AttrData::F32(v) => Attr::f32(
                    w,
                    ia.iter()
                        .zip(ib)
                        .zip(fr)
                        .flat_map(|((&i, &j), &f)| {
                            let f = f as f32;
                            (0..w).map(move |k| v[i * w + k] * (1.0 - f) + v[j * w + k] * f)
                        })
                        .collect(),
                ),
                AttrData::I32(v) => Attr::i32(
                    w,
                    ia.iter()
                        .flat_map(|&i| v[i * w..(i + 1) * w].iter().copied())
                        .collect(),
                ),
            };
            (n.clone(), Arc::new(b))
        })
        .collect()
}
pub(super) fn mean(t: &AttrTable, buckets: &[Vec<usize>]) -> AttrTable {
    t.iter()
        .map(|(n, a)| {
            let w = a.width;
            let b = match &a.data {
                AttrData::F32(v) => Attr::f32(
                    w,
                    buckets
                        .iter()
                        .flat_map(|ix| {
                            (0..w).map(move |k| {
                                (ix.iter().map(|&i| v[i * w + k] as f64).sum::<f64>()
                                    / ix.len().max(1) as f64) as f32
                            })
                        })
                        .collect(),
                ),
                AttrData::I32(v) => Attr::i32(
                    w,
                    buckets
                        .iter()
                        .flat_map(|ix| {
                            (0..w).map(move |k| ix.iter().map(|&i| v[i * w + k]).min().unwrap_or(0))
                        })
                        .collect(),
                ),
            };
            (n.clone(), Arc::new(b))
        })
        .collect()
}
pub(super) fn make(
    point: AttrTable,
    vp: Vec<i32>,
    sizes: &[usize],
    vertex: AttrTable,
    prim: AttrTable,
    detail: serde_json::Map<String, serde_json::Value>,
    closed: Option<Vec<bool>>,
) -> Result<Geometry> {
    let mut offsets = vec![0];
    for &s in sizes {
        let next = offsets.last().copied().unwrap() as usize + s;
        if next > i32::MAX as usize {
            return Err(Error::new("too many vertices"));
        }
        offsets.push(next as i32);
    }
    let npoints = point.get("P").map_or(0, |a| a.rows());
    let g = Geometry {
        npoints,
        topology: Arc::new(Topology {
            vertex_point: vp,
            prim_offsets: offsets,
            closed,
        }),
        point,
        vertex,
        prim,
        detail,
    };
    g.validate()?;
    Ok(g)
}
pub(super) fn rebuild(
    g: &Geometry,
    faces: &[Vec<usize>],
    sources: &[usize],
    closed: Option<Vec<bool>>,
) -> Result<Geometry> {
    let ix: Vec<_> = faces.iter().flatten().copied().collect();
    make(
        g.point.clone(),
        ix.iter().map(|&v| g.topology.vertex_point[v]).collect(),
        &faces.iter().map(Vec::len).collect::<Vec<_>>(),
        gather(&g.vertex, &ix),
        gather(&g.prim, sources),
        g.detail.clone(),
        closed,
    )
}
pub(super) fn compact(g: Geometry) -> Result<Geometry> {
    let used: BTreeSet<_> = g
        .topology
        .vertex_point
        .iter()
        .map(|&x| x as usize)
        .collect();
    if used.len() == g.npoints {
        return Ok(g);
    }
    let ix: Vec<_> = used.into_iter().collect();
    let mut map = vec![0; g.npoints];
    for (i, &p) in ix.iter().enumerate() {
        map[p] = i as i32;
    }
    let mut out = g.clone();
    out.npoints = ix.len();
    out.point = gather(&g.point, &ix);
    let mut t = (*g.topology).clone();
    for v in &mut t.vertex_point {
        *v = map[*v as usize];
    }
    out.topology = Arc::new(t);
    Ok(out)
}
pub(super) fn prim_mask(g: &Geometry, text: &str) -> Result<Vec<bool>> {
    // Reuse the core selection parser on primitive rows, including conversion of point groups by all corners.
    let mut proxy = Geometry::points_only(vec![0.0; g.nprims() * 3])?;
    proxy.point = g.prim.clone();
    let pos = positions(g);
    let mut cent = Vec::new();
    for p in 0..g.nprims() {
        let r = range(g, p);
        let mut c = [0.; 3];
        for v in r.clone() {
            c = add(c, pos[g.topology.vertex_point[v] as usize]);
        }
        cent.extend(mul(c, 1.0 / r.len().max(1) as f64).map(|x| x as f32));
    }
    proxy.set_p(cent);
    for (n, a) in &g.point {
        if !n.starts_with("group_") || proxy.point.contains_key(n) {
            continue;
        }
        let mut data = vec![0; g.nprims()];
        for p in 0..g.nprims() {
            data[p] = (!range(g, p).is_empty()
                && range(g, p).all(|v| match &a.data {
                    AttrData::F32(x) => x[g.topology.vertex_point[v] as usize * a.width] != 0.,
                    AttrData::I32(x) => x[g.topology.vertex_point[v] as usize * a.width] != 0,
                })) as i32;
        }
        proxy.point.insert(n.clone(), Arc::new(Attr::i32(1, data)));
    }
    groups::point_mask(&proxy, text)
}
pub(super) fn group_name(name: &str) -> Result<()> {
    if name.is_empty()
        || !name.chars().next().unwrap().is_ascii_alphabetic() && !name.starts_with('_')
        || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Err(Error::new("invalid group name"));
    }
    Ok(())
}
pub(super) fn write_group(g: &mut Geometry, domain: &str, name: &str, mask: &[bool]) -> Result<()> {
    group_name(name)?;
    let a = Arc::new(Attr::i32(1, mask.iter().map(|&v| v as i32).collect()));
    if domain == "point" {
        g.point.insert(format!("group_{name}"), a);
    } else {
        g.prim.insert(format!("group_{name}"), a);
    }
    Ok(())
}
pub(super) fn outgroup(
    g: &mut Geometry,
    p: &Params,
    key: &str,
    domain: &str,
    mask: &[bool],
) -> Result<()> {
    if p.b(&format!("outgrp_{key}"))? {
        let name = p.s(&format!("outgrp_{key}_name"))?;
        if !name.trim().is_empty() {
            if domain == "edge" {
                group_name(name.trim())?;
                super::edge_tools::write_values(
                    g,
                    &format!("edge_group_{}", name.trim()),
                    &mask.iter().map(|&x| x as u8 as f32).collect::<Vec<_>>(),
                    true,
                )?;
            } else {
                write_group(g, domain, name.trim(), mask)?;
            }
        }
    }
    Ok(())
}

pub const REVERSE: NodeSpec = NodeSpec {
    name: "reverse",
    inputs: GEO,
    params: crate::params!("group":"";"reverse_normals":true),
    cook: reverse,
};
pub const FACET: NodeSpec = NodeSpec {
    name: "facet",
    inputs: GEO,
    params: crate::params!("unique_points":true;"compute_normals":false),
    cook: facet,
};
pub const DIVIDE: NodeSpec = NodeSpec {
    name: "divide",
    inputs: GEO,
    params: crate::params!("group":""),
    cook: divide,
};
pub const POLYFILL: NodeSpec = NodeSpec {
    name: "polyfill",
    inputs: GEO,
    params: crate::params!("fill_group":""),
    cook: polyfill,
};
pub const MIRROR: NodeSpec = NodeSpec {
    name: "mirror",
    inputs: GEO,
    params: crate::params!("origin":[0.,0.,0.];"normal":[1.,0.,0.];"keep_original":true;"consolidate_seam":true;"distance":0.001;"outgrp_mirrored":false;"outgrp_mirrored_name":"mirrored";"outgrp_mirroredpoints":false;"outgrp_mirroredpoints_name":"mirroredpoints";"outgrp_seam":false;"outgrp_seam_name":"seampoints";"group":"";"operation":"mirror_all";"offset":0.0;"reverse":false;"cut":false;"unshared_only":false),
    cook: super::mirror::mirror,
};
pub const CLIP: NodeSpec = NodeSpec {
    name: "clip",
    inputs: GEO,
    params: crate::params!("origin":[0.,0.,0.];"normal":[0.,0.,1.];"keep":"above";"outgrp_above":true;"outgrp_above_name":"above";"outgrp_below":false;"outgrp_below_name":"below";"outgrp_cutpolys":false;"outgrp_cutpolys_name":"cutpolys";"outgrp_cutpoints":false;"outgrp_cutpoints_name":"cutpoints";"outgrp_cutedges":false;"outgrp_cutedges_name":"cutedges"),
    cook: clip,
};
pub const POLYEXTRUDE: NodeSpec = NodeSpec {
    name: "polyextrude",
    inputs: GEO,
    params: crate::params!("group":"";"edges":"";"points":"";"distance":0.5;"direction":[0.,0.,0.];"inset":0.;"mode":"connected";"output_back":false;"outgrp_front":true;"outgrp_front_name":"extrudeFront";"outgrp_back":false;"outgrp_back_name":"extrudeBack";"outgrp_side":true;"outgrp_side_name":"extrudeSide"),
    cook: polyextrude,
};
fn negate(t: &mut AttrTable, mask: &[bool]) {
    if let Some(a) = t.get("N") {
        let mut a = (**a).clone();
        match &mut a.data {
            AttrData::F32(v) => {
                for (i, row) in v.chunks_mut(a.width).enumerate() {
                    if mask[i] {
                        for x in row {
                            *x = -*x;
                        }
                    }
                }
            }
            AttrData::I32(v) => {
                for (i, row) in v.chunks_mut(a.width).enumerate() {
                    if mask[i] {
                        for x in row {
                            *x = -*x;
                        }
                    }
                }
            }
        }
        t.insert("N".into(), Arc::new(a));
    }
}
fn reverse(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = REVERSE.one(inputs, 0)?;
    let sel = prim_mask(&g, &p.s("group")?)?;
    let mut faces = Vec::new();
    for i in 0..g.nprims() {
        let mut f: Vec<_> = range(&g, i).collect();
        if sel[i] {
            f.reverse();
        }
        faces.push(f);
    }
    let mut out = rebuild(
        &g,
        &faces,
        &(0..g.nprims()).collect::<Vec<_>>(),
        g.topology.closed.clone(),
    )?;
    if p.b("reverse_normals")? {
        let (pv, _, _) = corners(&out);
        negate(
            &mut out.vertex,
            &pv.iter().map(|&i| sel[i]).collect::<Vec<_>>(),
        );
        negate(&mut out.prim, &sel);
        let mut ps = vec![false; g.npoints];
        for i in 0..g.nprims() {
            if sel[i] {
                for v in range(&g, i) {
                    ps[g.topology.vertex_point[v] as usize] = true;
                }
            }
        }
        negate(&mut out.point, &ps);
    }
    Ok(out)
}
fn facet(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = FACET.one(inputs, 0)?;
    let mut out = (*g).clone();
    let unique = p.b("unique_points")?;
    if unique {
        let ix: Vec<_> = g
            .topology
            .vertex_point
            .iter()
            .map(|&v| v as usize)
            .collect();
        out.point = gather(&g.point, &ix);
        out.npoints = ix.len();
        let mut t = (*g.topology).clone();
        t.vertex_point = (0..ix.len() as i32).collect();
        out.topology = Arc::new(t);
    }
    if p.b("compute_normals")? {
        let fn_ = normals::normalize_to_f32(&normals::prim_normals(out.p(), &out.topology));
        if unique {
            let (pv, _, _) = corners(&out);
            out.point.insert(
                "N".into(),
                Arc::new(Attr::f32(
                    3,
                    pv.iter()
                        .flat_map(|&p| fn_[3 * p..3 * p + 3].iter().copied())
                        .collect(),
                )),
            );
        } else {
            out.prim.insert("N".into(), Arc::new(Attr::f32(3, fn_)));
        }
    }
    Ok(out)
}
pub(super) fn ear_clip(xy: &[[f64; 2]]) -> Vec<[usize; 3]> {
    let n = xy.len();
    if n < 3 {
        return Vec::new();
    }
    let sign = if (0..n)
        .map(|i| xy[i][0] * xy[(i + 1) % n][1] - xy[(i + 1) % n][0] * xy[i][1])
        .sum::<f64>()
        >= 0.
    {
        1.
    } else {
        -1.
    };
    let turn = |a: usize, b: usize, c: usize| {
        ((xy[b][0] - xy[a][0]) * (xy[c][1] - xy[a][1])
            - (xy[b][1] - xy[a][1]) * (xy[c][0] - xy[a][0]))
            * sign
    };
    let mut ix: Vec<_> = (0..n).collect();
    let mut out = Vec::new();
    for _ in 0..10 * n {
        if ix.len() <= 3 {
            break;
        }
        let mut found = false;
        for k in 0..ix.len() {
            let a = ix[(k + ix.len() - 1) % ix.len()];
            let b = ix[k];
            let c = ix[(k + 1) % ix.len()];
            if turn(a, b, c) <= 1e-12 {
                continue;
            }
            if ix.iter().any(|&j| {
                j != a
                    && j != b
                    && j != c
                    && turn(a, b, j) >= 0.
                    && turn(b, c, j) >= 0.
                    && turn(c, a, j) >= 0.
            }) {
                continue;
            }
            out.push([a, b, c]);
            ix.remove(k);
            found = true;
            break;
        }
        if !found {
            break;
        }
    }
    for k in 1..ix.len() - 1 {
        out.push([ix[0], ix[k], ix[k + 1]]);
    }
    out
}
fn triangles(g: &Geometry, p: usize, pos: &[[f64; 3]], fn_: [f64; 3]) -> Vec<Vec<usize>> {
    let r = range(g, p);
    let q: Vec<_> = r
        .clone()
        .map(|v| pos[g.topology.vertex_point[v] as usize])
        .collect();
    if q.len() < 3 {
        return Vec::new();
    }
    let n = q.len();
    let concave = (0..n).any(|i| {
        dot(
            cross(sub(q[i], q[(i + n - 1) % n]), sub(q[(i + 1) % n], q[i])),
            fn_,
        ) < -1e-12
    });
    if !concave {
        return (1..n - 1)
            .map(|k| vec![r.start, r.start + k, r.start + k + 1])
            .collect();
    }
    let normal = mul(fn_, 1.0 / len(fn_).max(1e-30));
    let u = unit(cross(
        normal,
        if normal[0].abs() < 0.9 {
            [1., 0., 0.]
        } else {
            [0., 1., 0.]
        },
    ));
    let w = cross(normal, u);
    let xy: Vec<_> = q.iter().map(|&q| [dot(q, u), dot(q, w)]).collect();
    ear_clip(&xy)
        .iter()
        .map(|t| t.iter().map(|&i| r.start + i).collect())
        .collect()
}
fn divide(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = DIVIDE.one(inputs, 0)?;
    let s = prim_mask(&g, &p.s("group")?)?;
    let pos = positions(&g);
    let face_normals = normals::prim_normals(g.p(), &g.topology);
    let mut faces = Vec::new();
    let mut src = Vec::new();
    let mut closed = Vec::new();
    for i in 0..g.nprims() {
        let f = if s[i] && range(&g, i).len() > 3 && g.topology.is_closed(i) {
            triangles(&g, i, &pos, face_normals[i])
        } else {
            vec![range(&g, i).collect()]
        };
        for f in f {
            faces.push(f);
            src.push(i);
            closed.push(g.topology.is_closed(i));
        }
    }
    rebuild(
        &g,
        &faces,
        &src,
        if g.topology.has_open() {
            Some(closed)
        } else {
            None
        },
    )
}
fn polyfill(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = POLYFILL.one(inputs, 0)?;
    polygons(&g, "polyfill")?;
    let e = topo::edges(&g.topology, g.npoints);
    let (_, nx, _) = corners(&g);
    let vp = &g.topology.vertex_point;
    let mut succ = BTreeMap::new();
    let mut starts = Vec::new();
    for v in 0..g.nvertices() {
        if e.count[e.of_vertex[v] as usize] == 1 {
            let b = vp[nx[v]];
            if succ.insert(b, vp[v]).is_some() {
                return Err(Error::new("the boundary touches itself; cannot fill it"));
            }
            starts.push(b);
        }
    }
    let mut seen = BTreeSet::new();
    let mut loops = Vec::new();
    for start in starts {
        if seen.contains(&start) {
            continue;
        }
        let mut at = start;
        let mut f = Vec::new();
        while seen.insert(at) {
            f.push(at);
            let Some(&next) = succ.get(&at) else {
                break;
            };
            at = next;
        }
        if at == start && f.len() >= 3 {
            loops.push(f);
        }
    }
    if loops.is_empty() {
        return Ok((*g).clone());
    }
    let mut vp = vp.clone();
    let mut sizes: Vec<_> = (0..g.nprims()).map(|i| range(&g, i).len()).collect();
    for f in &loops {
        vp.extend(f);
        sizes.push(f.len());
    }
    let pad = |t: &AttrTable, n: usize| {
        let zero = t
            .iter()
            .map(|(k, a)| (k.clone(), Arc::new(a.zeros_like(n))))
            .collect();
        concat(t, &zero)
    };
    let nv = loops.iter().map(Vec::len).sum();
    let mut out = make(
        g.point.clone(),
        vp,
        &sizes,
        pad(&g.vertex, nv),
        pad(&g.prim, loops.len()),
        g.detail.clone(),
        None,
    )?;
    super::material_tables::neutral_new_faces(&mut out, g.nprims(), g.nvertices());
    let name = p.s("fill_group")?;
    if !name.is_empty() {
        write_group(
            &mut out,
            "prim",
            &name,
            &(0..sizes.len())
                .map(|i| i >= g.nprims())
                .collect::<Vec<_>>(),
        )?;
    }
    Ok(out)
}
fn clip(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = CLIP.one(inputs, 0)?;
    polygons(&g, "clip")?;
    let raw = p.v3("normal")?;
    let n = mul(raw, 1. / len(raw).max(1e-12));
    let o = p.v3("origin")?;
    let mut d: Vec<_> = positions(&g).iter().map(|&q| dot(sub(q, o), n)).collect();
    let eps = 1e-7 * d.iter().map(|d| d.abs()).fold(1.0, f64::max);
    for x in &mut d {
        if x.abs() < eps {
            *x = 0.;
        }
    }
    let e = topo::edges(&g.topology, g.npoints);
    let (_, nx, _) = corners(&g);
    let vp = &g.topology.vertex_point;
    let mut xindex = vec![0; e.len()];
    let mut ia = Vec::new();
    let mut ib = Vec::new();
    let mut fr = Vec::new();
    for i in 0..e.len() {
        let a = e.points[2 * i] as usize;
        let b = e.points[2 * i + 1] as usize;
        if d[a] * d[b] < 0. {
            xindex[i] = g.npoints + ia.len();
            ia.push(a);
            ib.push(b);
            fr.push(d[a] / (d[a] - d[b]));
        }
    }
    let mut pts = concat(&g.point, &lerp(&g.point, &ia, &ib, &fr)); // P uses a + (b-a)*t rather than B.lerp's a*(1-t)+b*t.
    let mut newp = g.p().to_vec();
    for ((&a, &b), &t) in ia.iter().zip(&ib).zip(&fr) {
        for k in 0..3 {
            newp.push(g.p()[3 * a + k] + (g.p()[3 * b + k] - g.p()[3 * a + k]) * t as f32);
        }
    }
    pts.insert("P".into(), Arc::new(Attr::f32(3, newp)));
    let keep = p.e("keep", &["above", "below", "both"])?;
    let sides = match keep.as_str() {
        "above" => vec![1.],
        "below" => vec![-1.],
        _ => vec![1., -1.],
    };
    let mut vertices = Vec::new();
    let mut va = Vec::new();
    let mut vb = Vec::new();
    let mut vf = Vec::new();
    let mut sizes = Vec::new();
    let mut src = Vec::new();
    let mut above = Vec::new();
    let mut cut = Vec::new();
    for s in sides {
        for i in 0..g.nprims() {
            let mut entries = Vec::new();
            let mut was_cut = false;
            for v in range(&g, i) {
                let a = vp[v] as usize;
                let b = vp[nx[v]] as usize;
                if s * d[a] >= 0. {
                    entries.push((a, v, 0.));
                }
                if d[a] * d[b] < 0. {
                    entries.push((xindex[e.of_vertex[v] as usize], v, d[a] / (d[a] - d[b])));
                    was_cut = true;
                }
            }
            if entries.len() < 3 {
                continue;
            }
            sizes.push(entries.len());
            src.push(i);
            above.push(s > 0.);
            cut.push(was_cut);
            for (q, v, f) in entries {
                vertices.push(q as i32);
                va.push(v);
                vb.push(nx[v]);
                vf.push(f);
            }
        }
    }
    let mut out = make(
        pts,
        vertices,
        &sizes,
        lerp(&g.vertex, &va, &vb, &vf),
        gather(&g.prim, &src),
        g.detail.clone(),
        None,
    )?;
    outgroup(&mut out, p, "above", "prim", &above)?;
    outgroup(
        &mut out,
        p,
        "below",
        "prim",
        &above.iter().map(|b| !b).collect::<Vec<_>>(),
    )?;
    outgroup(&mut out, p, "cutpolys", "prim", &cut)?;
    let on: Vec<_> = d
        .iter()
        .map(|&x| x == 0.)
        .chain(std::iter::repeat_n(true, ia.len()))
        .collect();
    outgroup(&mut out, p, "cutpoints", "point", &on)?;
    let used: BTreeSet<_> = out
        .topology
        .vertex_point
        .iter()
        .map(|&x| x as usize)
        .collect();
    let on: Vec<_> = used.iter().map(|&i| on[i]).collect();
    out = compact(out)?;
    let ed = topo::edges(&out.topology, out.npoints);
    let mask = ed
        .points
        .chunks_exact(2)
        .map(|q| on[q[0] as usize] && on[q[1] as usize])
        .collect::<Vec<_>>();
    outgroup(&mut out, p, "cutedges", "edge", &mask)?;
    Ok(out)
}
fn polyextrude(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = POLYEXTRUDE.one(inputs, 0)?;
    let direction = p.v3("direction")?;
    if !p.s("edges")?.is_empty() || !p.s("points")?.is_empty() {
        return component_extrude(&g, &p.s("edges")?, &p.s("points")?, p.f("distance")?, direction);
    }
    extrude_with_direction(
        &g,
        &p.s("group")?,
        p.f("distance")?,
        p.f("inset")?,
        &p.e("mode", &["connected", "individual"])?,
        p.b("output_back")?,
        Some(p),
        direction,
    )
}
fn component_extrude(g: &Geometry, edges: &str, points: &str, distance: f64, direction: [f64; 3]) -> Result<Geometry> {
    if distance == 0. { return Ok(g.clone()); }
    let edge_pairs: Vec<(usize,usize)> = edges.split(|c: char| c == ',' || c.is_whitespace())
        .filter(|s| !s.is_empty()).map(|s| {
            let (a,b)=s.split_once('-').ok_or_else(||Error::new("invalid extrusion edge"))?;
            let a=a.parse::<usize>().map_err(|_|Error::new("invalid extrusion edge"))?;
            let b=b.parse::<usize>().map_err(|_|Error::new("invalid extrusion edge"))?;
            Ok((a,b))
        }).collect::<Result<_>>()?;
    let point_ids: Vec<usize> = points.split(|c: char| c == ',' || c.is_whitespace())
        .filter(|s| !s.is_empty()).map(|s|s.parse::<usize>().map_err(|_|Error::new("invalid extrusion point")))
        .collect::<Result<_>>()?;
    if edge_pairs.is_empty() && point_ids.is_empty() { return Ok(g.clone()); }
    let mut owner=vec![None;g.npoints];
    let mut normals=vec![[0.;3];g.npoints];
    let mut curve_directions=vec![[0.;3];g.npoints];
    let pos=positions(g);
    let face_normals=normals::prim_normals(g.p(),&g.topology);
    let mut valid_edges=std::collections::BTreeMap::new();
    for prim in 0..g.nprims() {
        let verts: Vec<_>=range(g,prim).collect();
        for (i,&v) in verts.iter().enumerate() {
            let a=g.topology.vertex_point[v] as usize;
            owner[a]=Some((v,prim));
            normals[a]=add(normals[a],face_normals[prim]);
            let closed=g.topology.closed.as_ref().is_none_or(|closed|closed[prim]);
            if i+1<verts.len() || closed {
                let b=g.topology.vertex_point[verts[(i+1)%verts.len()]] as usize;
                valid_edges.entry((a.min(b),a.max(b))).or_insert((v,prim));
                if !closed {
                    curve_directions[a]=add(curve_directions[a],sub(pos[a],pos[b]));
                    curve_directions[b]=add(curve_directions[b],sub(pos[b],pos[a]));
                }
            }
        }
    }
    let mut selected=std::collections::BTreeSet::new();
    for &(a,b) in &edge_pairs {
        if !valid_edges.contains_key(&(a.min(b),a.max(b))) { return Err(Error::new("extrusion edge is not in the source mesh")); }
        selected.extend([a,b]);
    }
    for &p in &point_ids {
        if owner.get(p).copied().flatten().is_none() { return Err(Error::new("extrusion point is not in the source mesh")); }
        selected.insert(p);
    }
    let selected: Vec<_>=selected.into_iter().collect();
    let mut map=vec![0usize;g.npoints];
    for (i,&p) in selected.iter().enumerate() { map[p]=g.npoints+i; }
    let mut outp=g.p().to_vec();
    for &p in &selected {
        let normal=if len(direction)>0. { direction }
            else if len(normals[p])>1e-12 {unit32(normals[p])}
            else {unit32(curve_directions[p])};
        outp.extend(add(pos[p],mul(normal,distance)).map(|x|x as f32));
    }
    let mut point=concat(&g.point,&gather(&g.point,&selected));
    point.insert("P".into(),Arc::new(Attr::f32(3,outp)));
    let mut vp=g.topology.vertex_point.clone();
    let mut sizes: Vec<_>=(0..g.nprims()).map(|i|range(g,i).len()).collect();
    let mut vs: Vec<_>=(0..g.nvertices()).collect();
    let mut ps: Vec<_>=(0..g.nprims()).collect();
    let mut closed=g.topology.closed.clone().unwrap_or_else(||vec![true;g.nprims()]);
    for &(a,b) in &edge_pairs {
        let (v,prim)=valid_edges[&(a.min(b),a.max(b))];
        vp.extend([a as i32,b as i32,map[b] as i32,map[a] as i32]);
        vs.extend([v,v,v,v]); ps.push(prim); sizes.push(4); closed.push(true);
    }
    for &p in &point_ids {
        let (v,prim)=owner[p].unwrap();
        vp.extend([p as i32,map[p] as i32]);
        vs.extend([v,v]); ps.push(prim); sizes.push(2); closed.push(false);
    }
    let mut out=make(point,vp,&sizes,gather(&g.vertex,&vs),gather(&g.prim,&ps),g.detail.clone(),Some(closed))?;
    out.point.remove("N");
    Ok(out)
}
pub(super) fn extrude(
    g: &Geometry,
    group: &str,
    distance: f64,
    inset: f64,
    mode: &str,
    back: bool,
    params: Option<&Params>,
) -> Result<Geometry> {
    extrude_with_direction(g, group, distance, inset, mode, back, params, [0.; 3])
}
fn extrude_with_direction(
    g: &Geometry, group: &str, distance: f64, inset: f64, mode: &str,
    back: bool, params: Option<&Params>, direction: [f64; 3],
) -> Result<Geometry> {
    polygons(g, "polyextrude")?;
    let sel = prim_mask(g, group)?;
    let apply_groups =
        |out: &mut Geometry, front: &[bool], back: &[bool], side: &[bool]| -> Result<()> {
            if let Some(p) = params {
                outgroup(out, p, "front", "prim", front)?;
                outgroup(out, p, "back", "prim", back)?;
                outgroup(out, p, "side", "prim", side)?;
            } else {
                write_group(out, "prim", "extrudeFront", front)?;
                write_group(out, "prim", "extrudeSide", side)?;
            }
            Ok(())
        };
    if !sel.iter().any(|&b| b) || (distance == 0. && inset == 0. && !back) {
        let mut out = g.clone();
        apply_groups(
            &mut out,
            &sel,
            &vec![false; g.nprims()],
            &vec![false; g.nprims()],
        )?;
        return Ok(out);
    }
    let (pv, nx, _) = corners(g);
    let vp = &g.topology.vertex_point;
    let pos = positions(g);
    let fn_ = normals::prim_normals(g.p(), &g.topology);
    let fu: Vec<_> = fn_.iter().map(|&v| unit32(v)).collect();
    let selected: Vec<_> = (0..g.nvertices()).filter(|&v| sel[pv[v]]).collect();
    let individual = mode == "individual";
    let shifted = if inset != 0. {
        Some(super::planar_offset::offset_selected(
            g, &sel, inset, individual,
        )?)
    } else {
        None
    };
    let src: Vec<usize> = if individual {
        selected.iter().map(|&v| vp[v] as usize).collect()
    } else {
        selected
            .iter()
            .map(|&v| vp[v] as usize)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    };
    let mut nv = vec![0; g.nvertices()];
    let mut newp = Vec::new();
    if individual {
        for (i, &v) in selected.iter().enumerate() {
            nv[v] = g.npoints + i;
            let base = shifted.as_ref().map_or(pos[vp[v] as usize], |q| q[v]);
            newp.push(add(base, mul(if len(direction) > 0. { direction } else { fu[pv[v]] }, distance)));
        }
    } else {
        let mut map = vec![0; g.npoints];
        let mut acc = vec![[0.; 3]; g.npoints];
        let mut repr = vec![0; g.npoints];
        for (i, &q) in src.iter().enumerate() {
            map[q] = g.npoints + i;
        }
        for &v in &selected {
            let q = vp[v] as usize;
            nv[v] = map[q];
            acc[q] = add(acc[q], fn_[pv[v]]);
            repr[q] = v;
        }
        let pn: Vec<_> = acc.iter().map(|&v| unit32(v)).collect();
        let mut mincos = vec![1.0f64; g.npoints];
        for &v in &selected {
            let q = vp[v] as usize;
            mincos[q] = mincos[q].min(dot(pn[q], fu[pv[v]]).max(0.25));
        }
        for &q in &src {
            let mut point = add(pos[q], if len(direction) > 0. { mul(direction, distance) } else { mul(pn[q], distance / mincos[q]) });
            if let Some(sh) = &shifted {
                point = add(point, sub(sh[repr[q]], pos[q]));
            }
            newp.push(point);
        }
    }
    let boundary: Vec<_> = if individual {
        selected.clone()
    } else {
        let e = topo::edges(&g.topology, g.npoints);
        let mut cnt = vec![0; e.len()];
        for &v in &selected {
            cnt[e.of_vertex[v] as usize] += 1;
        }
        selected
            .iter()
            .copied()
            .filter(|&v| cnt[e.of_vertex[v] as usize] == 1)
            .collect()
    };
    let mut point = concat(&g.point, &gather(&g.point, &src));
    let mut p = g.p().to_vec();
    p.extend(newp.iter().flat_map(|q| q.map(|x| x as f32)));
    point.insert("P".into(), Arc::new(Attr::f32(3, p)));
    let mut outvp: Vec<_> = vp
        .iter()
        .enumerate()
        .map(|(v, &q)| if sel[pv[v]] { nv[v] as i32 } else { q })
        .collect();
    let mut vs: Vec<_> = (0..g.nvertices()).collect();
    let mut ps: Vec<_> = (0..g.nprims()).collect();
    let mut sizes: Vec<_> = (0..g.nprims()).map(|i| range(g, i).len()).collect();
    for &v in &boundary {
        outvp.extend([vp[v], vp[nx[v]], nv[nx[v]] as i32, nv[v] as i32]);
        vs.extend([v, nx[v], nx[v], v]);
        ps.push(pv[v]);
        sizes.push(4);
    }
    if back {
        for i in 0..g.nprims() {
            if !sel[i] {
                continue;
            }
            let r = range(g, i);
            sizes.push(r.len());
            ps.push(i);
            for v in r.rev() {
                outvp.push(vp[v]);
                vs.push(v);
            }
        }
    }
    let mut out = make(
        point,
        outvp,
        &sizes,
        gather(&g.vertex, &vs),
        gather(&g.prim, &ps),
        g.detail.clone(),
        None,
    )?;
    let total = sizes.len();
    apply_groups(
        &mut out,
        &(0..total)
            .map(|i| i < g.nprims() && sel[i])
            .collect::<Vec<_>>(),
        &(0..total)
            .map(|i| i >= g.nprims() + boundary.len())
            .collect::<Vec<_>>(),
        &(0..total)
            .map(|i| i >= g.nprims() && i < g.nprims() + boundary.len())
            .collect::<Vec<_>>(),
    )?;
    out.point.remove("N");
    compact(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    pub(super) fn quad() -> Geometry {
        Geometry::from_arrays(
            vec![0., 0., 0., 2., 0., 0., 2., 2., 0., 0., 2., 0.],
            vec![0, 1, 2, 3],
            vec![0, 4],
            None,
        )
        .unwrap()
    }
    fn cook(s: &NodeSpec, g: Geometry, v: serde_json::Value) -> Geometry {
        let p = Params::resolve(s.params, v.as_object()).unwrap();
        (s.cook)(&[vec![Arc::new(g)]], &p).unwrap()
    }
    #[test]
    fn reverse_and_divide_keep_corner_rows() {
        let mut g = quad();
        g.vertex
            .insert("id".into(), Arc::new(Attr::i32(1, vec![10, 11, 12, 13])));
        let r = cook(&REVERSE, g.clone(), json!({}));
        assert_eq!(r.topology.vertex_point, vec![3, 2, 1, 0]);
        assert_eq!(r.vertex["id"].data, AttrData::I32(vec![13, 12, 11, 10]));
        let d = cook(&DIVIDE, g, json!({}));
        assert_eq!(d.topology.vertex_point, vec![0, 1, 2, 0, 2, 3]);
        d.validate().unwrap();
    }
    #[test]
    fn extrusion_closed_shell_and_zero_noop() {
        let g = quad();
        let out = cook(
            &POLYEXTRUDE,
            g.clone(),
            json!({"distance":2.,"output_back":true}),
        );
        assert_eq!((out.npoints, out.nprims()), (8, 6));
        assert!(
            topo::edges(&out.topology, out.npoints)
                .count
                .iter()
                .all(|&c| c == 2)
        );
        assert_eq!(
            out.prim["group_extrudeSide"].data,
            AttrData::I32(vec![0, 1, 1, 1, 1, 0])
        );
        let z = cook(&POLYEXTRUDE, g.clone(), json!({"distance":0.}));
        assert_eq!(z.p(), g.p());
        assert_eq!(z.topology, g.topology);
    }
    #[test]
    fn polyextrude_axis_direction_changes_preview_geometry() {
        let g=quad();
        let normal=cook(&POLYEXTRUDE,g.clone(),json!({"distance":1.}));
        let axis=cook(&POLYEXTRUDE,g,json!({"distance":1.,"direction":[1.,0.,0.]}));
        assert_eq!(normal.nprims(),axis.nprims());
        assert!(axis.p().chunks_exact(3).any(|p|p[0]>2.9));
        assert_ne!(normal.p(),axis.p());
    }
    #[test]
    fn polyextrude_point_can_continue_along_its_new_open_segment() {
        let first=cook(&POLYEXTRUDE,quad(),json!({"points":"0","distance":0.05}));
        let second=cook(&POLYEXTRUDE,first.clone(),json!({"points":"4","distance":0.05}));
        assert_eq!(second.npoints,6);assert_eq!(second.nprims(),3);
        let a=&first.p()[12..15];let b=&second.p()[15..18];
        assert!((b[2]-a[2]-0.05).abs()<1e-6);
        assert_eq!(&second.topology.closed.as_ref().unwrap()[..],&[true,false,false]);
        second.validate().unwrap();
    }
    #[test]
    fn polyextrude_edge_and_point_previews_keep_original_geometry() {
        let g=quad();
        let edge=cook(&POLYEXTRUDE,g.clone(),json!({"edges":"0-1","distance":1.}));
        assert_eq!(edge.nprims(),g.nprims()+1);
        assert_eq!(edge.npoints,g.npoints+2);
        assert_eq!(edge.topology.vertex_point[0..4],g.topology.vertex_point[0..4]);
        assert!(edge.p().chunks_exact(3).any(|p|p[2]>0.9));
        let point=cook(&POLYEXTRUDE,g.clone(),json!({"points":"0","distance":1.}));
        assert_eq!(point.nprims(),g.nprims()+1);
        assert_eq!(point.npoints,g.npoints+1);
        assert_eq!(point.topology.closed.as_ref().unwrap(),&vec![true,false]);
        point.validate().unwrap();
    }
    #[test]
    fn clip_shared_intersections_and_int_sources() {
        let mut g = quad();
        g.point
            .insert("id".into(), Arc::new(Attr::i32(1, vec![7, 8, 9, 10])));
        let out = cook(
            &CLIP,
            g,
            json!({"origin":[1.,0.,0.],"normal":[1.,0.,0.],"keep":"both"}),
        );
        assert_eq!((out.npoints, out.nprims()), (6, 2));
        assert_eq!(out.point["id"].data, AttrData::I32(vec![7, 8, 9, 10, 7, 9]));
        out.validate().unwrap();
    }
    #[test]
    fn concave_divide_area() {
        let g = Geometry::from_arrays(
            vec![
                0., 0., 0., 2., 0., 0., 2., 1., 0., 1., 1., 0., 1., 2., 0., 0., 2., 0.,
            ],
            vec![0, 1, 2, 3, 4, 5],
            vec![0, 6],
            None,
        )
        .unwrap();
        let out = cook(&DIVIDE, g, json!({}));
        assert_eq!(out.nprims(), 4);
        let n = normals::prim_normals(out.p(), &out.topology);
        assert!((n.iter().map(|v| len(*v)).sum::<f64>() / 2. - 3.).abs() < 1e-6);
        assert!(n.iter().all(|v| v[2] > 0.));
    }
}
/// Eigenvectors (columns) of a symmetric 3x3 matrix. Used for SVD-equivalent least squares on local Jacobians.
pub(super) fn eigen(mut a: [[f64; 3]; 3]) -> ([f64; 3], [[f64; 3]; 3]) {
    let mut v = [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];
    for _ in 0..64 {
        let mut pair = (0, 1);
        for (i, j) in [(0, 2), (1, 2)] {
            if a[i][j].abs() > a[pair.0][pair.1].abs() {
                pair = (i, j);
            }
        }
        let (p, q) = pair;
        if a[p][q].abs() <= 1e-16 * (a[0][0].abs() + a[1][1].abs() + a[2][2].abs()).max(1e-300) {
            break;
        }
        let tau = (a[q][q] - a[p][p]) / (2. * a[p][q]);
        let t = if tau >= 0. {
            1. / (tau + (1. + tau * tau).sqrt())
        } else {
            -1. / (-tau + (1. + tau * tau).sqrt())
        };
        let c = 1. / (1. + t * t).sqrt();
        let s = t * c;
        let ap = a[p][p];
        let aq = a[q][q];
        let off = a[p][q];
        a[p][p] = ap - t * off;
        a[q][q] = aq + t * off;
        a[p][q] = 0.;
        a[q][p] = 0.;
        for k in 0..3 {
            if k != p && k != q {
                let x = a[k][p];
                let y = a[k][q];
                a[k][p] = c * x - s * y;
                a[p][k] = a[k][p];
                a[k][q] = s * x + c * y;
                a[q][k] = a[k][q];
            }
            let x = v[k][p];
            let y = v[k][q];
            v[k][p] = c * x - s * y;
            v[k][q] = s * x + c * y;
        }
    }
    ([a[0][0], a[1][1], a[2][2]], v)
}
#[cfg(test)]
mod python_oracles {
    use super::*;
    use serde_json::{Map, Value};

    fn table(v: &Value) -> AttrTable {
        v.as_object()
            .unwrap()
            .iter()
            .map(|(name, a)| {
                let width = a["width"].as_u64().unwrap() as usize;
                let rows = a["data"].as_array().unwrap();
                let attr = if a["dtype"] == "float32" {
                    Attr::f32(
                        width,
                        rows.iter().map(|x| x.as_f64().unwrap() as f32).collect(),
                    )
                } else {
                    Attr::i32(
                        width,
                        rows.iter().map(|x| x.as_i64().unwrap() as i32).collect(),
                    )
                };
                (name.clone(), Arc::new(attr))
            })
            .collect()
    }
    fn geometry(v: &Value) -> Geometry {
        let point = table(&v["point"]);
        let closed = v["closed"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_bool().unwrap())
            .collect();
        let topo = Topology {
            vertex_point: v["vp"]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_i64().unwrap() as i32)
                .collect(),
            prim_offsets: v["offsets"]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_i64().unwrap() as i32)
                .collect(),
            closed: Some(closed),
        };
        let g = Geometry {
            npoints: point["P"].rows(),
            topology: Arc::new(topo),
            point,
            vertex: table(&v["vertex"]),
            prim: table(&v["prim"]),
            detail: v["detail"].as_object().unwrap().clone(),
        };
        g.validate().unwrap();
        g
    }
    fn compare(name: &str, a: &Geometry, b: &Geometry) {
        assert_eq!(a.npoints, b.npoints, "{name}: point count");
        assert_eq!(
            a.topology.vertex_point, b.topology.vertex_point,
            "{name}: corner points"
        );
        assert_eq!(
            a.topology.prim_offsets, b.topology.prim_offsets,
            "{name}: offsets"
        );
        assert_eq!(
            a.topology.closed_mask(),
            b.topology.closed_mask(),
            "{name}: closed flags"
        );
        // Python lacks the new neutral slot and binding metadata; its detail keys still match exactly.
        super::super::oracle_material_detail::compare(name, a, b);
        let (original_a, original_b) = (a, b);
        for domain in ["point", "vertex", "prim"] {
            let (a, b) = (a.table(domain), b.table(domain));
            let neutral_index = domain == "prim" && super::super::oracle_material_detail::neutral_index_added(original_a, original_b);
            // `point_group_knife` is a Rust-only group written by the knife node; Python has no such attribute.
            let rust_only = |key: &str| key == "point_group_knife";
            assert_eq!(
                a.keys().filter(|key| (!neutral_index || key.as_str() != "material_index") && !rust_only(key)).collect::<Vec<_>>(),
                b.keys().collect::<Vec<_>>(),
                "{name}: {domain} attribute names"
            );
            for (key, a) in a {
                if neutral_index && key == "material_index" { continue; }
                if rust_only(key) { continue; }
                let b = &b[key];
                let invalid_uv = if domain == "vertex" {
                    super::super::oracle_material_detail::invalid_uv_rows(
                        original_a, original_b, key, &(0..a.rows()).collect::<Vec<_>>())
                } else { vec![false; a.rows()] };
                assert_eq!(a.width, b.width, "{name}: {domain} {key} width");
                match (&a.data, &b.data) {
                    (AttrData::F32(a), AttrData::F32(b)) => {
                        assert_eq!(a.len(), b.len(), "{name}: {domain} {key} length");
                        for (i, (&a, &b)) in a.iter().zip(b).enumerate() {
                            if invalid_uv[i / original_a.table(domain)[key].width] && a.is_nan() && b == 0. { continue; }
                            assert!(
                                (a - b).abs() <= 1e-5 + 1e-5 * b.abs(),
                                "{name}: {domain} {key}[{i}]: Rust {a}, Python {b}"
                            );
                        }
                    }
                    (AttrData::I32(a), AttrData::I32(b)) => {
                        assert_eq!(a, b, "{name}: {domain} {key}")
                    }
                    _ => panic!("{name}: {domain} {key} dtype"),
                }
            }
        }
    }
    #[test]
    fn modeling_matches_python_topology_attributes_and_defaults() {
        // Regenerate only with tools/dump_rust_modeling_fixtures.py; Python is the oracle.
        // New source-less faces may add a neutral slot and NaN UV where Python stored zero.
        let cases: Value =
            serde_json::from_str(include_str!("test_data/modeling_parity.json")).unwrap();
        for c in cases.as_array().unwrap() {
            let name = c["name"].as_str().unwrap();
            let spec = super::super::get(c["type"].as_str().unwrap()).unwrap();
            let defaults: Map<String, Value> = spec
                .params
                .iter()
                .map(|p| (p.name.into(), (p.default)()))
                .collect();
            let expected_defaults = c["defaults"].as_object().unwrap();
            // Rust may add Rust-only params; every Python param must exist with the same default.
            for (key, expected) in expected_defaults {
                assert!(
                    defaults
                        .get(key)
                        .is_some_and(|value| crate::params::same_value(value, expected)),
                    "{name}: parameter {key} default"
                );
            }
            let p = Params::resolve(spec.params, c["params"].as_object()).unwrap();
            let mut inputs: Vec<Vec<Arc<Geometry>>> = c["inputs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| vec![Arc::new(geometry(v))])
                .collect();
            inputs.resize_with(spec.inputs.len(), Vec::new);
            let result = (spec.cook)(&inputs, &p);
            if c.get("error").is_some() {
                assert!(result.is_err(), "{name}: expected rejection");
            } else {
                let actual = result.unwrap_or_else(|e| panic!("{name}: {e}"));
                actual.validate().unwrap();
                compare(name, &actual, &geometry(&c["expected"]));
            }
        }
    }
    #[test]
    fn loop_cut_is_polysplit() {
        let alias = super::super::get("loop_cut").unwrap();
        let canonical = super::super::get("polysplit").unwrap();
        assert_eq!(alias.params.len(), canonical.params.len());
        let input = super::tests::quad();
        let given = serde_json::json!({"split_edges":"p0-1"});
        let p = Params::resolve(alias.params, given.as_object()).unwrap();
        let inputs = [vec![Arc::new(input)]];
        compare(
            "loop_cut alias",
            &(alias.cook)(&inputs, &p).unwrap(),
            &(canonical.cook)(&inputs, &p).unwrap(),
        );
    }
}
/// Small least-squares solve with a one-sided Jacobi SVD. Avoid forming A^T A:
/// squaring the condition number loses the null-space behavior of numpy.pinv/lstsq.
pub(super) fn least_squares<const N: usize>(
    rows: &[[f64; N]],
    rhs: &[f64],
    rcond: f64,
) -> [f64; N] {
    let mut a = rows.to_vec();
    let mut v = [[0.; N]; N];
    for i in 0..N {
        v[i][i] = 1.;
    }
    for _ in 0..64 {
        let mut changed = false;
        for p in 0..N {
            for q in p + 1..N {
                let aa = a.iter().map(|r| r[p] * r[p]).sum::<f64>();
                let bb = a.iter().map(|r| r[q] * r[q]).sum::<f64>();
                let ab = a.iter().map(|r| r[p] * r[q]).sum::<f64>();
                if ab.abs() <= f64::EPSILON * (aa * bb).sqrt() || ab == 0. {
                    continue;
                }
                let tau = (bb - aa) / (2. * ab);
                let t = if tau >= 0. {
                    1. / (tau + tau.hypot(1.))
                } else {
                    -1. / (-tau + tau.hypot(1.))
                };
                let c = 1. / (1. + t * t).sqrt();
                let s = t * c;
                for r in &mut a {
                    let x = r[p];
                    let y = r[q];
                    r[p] = c * x - s * y;
                    r[q] = s * x + c * y;
                }
                for r in &mut v {
                    let x = r[p];
                    let y = r[q];
                    r[p] = c * x - s * y;
                    r[q] = s * x + c * y;
                }
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let norm: [f64; N] = std::array::from_fn(|k| a.iter().map(|r| r[k] * r[k]).sum::<f64>().sqrt());
    let limit = norm.iter().copied().fold(0., f64::max) * rcond;
    let mut result = [0.; N];
    for k in 0..N {
        if norm[k] > limit && norm[k] > 0. {
            let coeff =
                a.iter().zip(rhs).map(|(r, &b)| r[k] * b).sum::<f64>() / (norm[k] * norm[k]);
            for i in 0..N {
                result[i] += v[i][k] * coeff;
            }
        }
    }
    result
}
#[cfg(test)]
mod least_squares_tests {
    use super::*;
    #[test]
    fn singular_solve_returns_minimum_norm_solution() {
        let rows = [[1., 2., 0.], [2., 4., 0.], [0., 0., 0.]];
        let result = least_squares(&rows, &[3., 6., 1.], 1e-15);
        assert!(len(sub(result, [0.6, 1.2, 0.])) < 1e-12);
    }
    #[test]
    fn full_rank_solve_recovers_an_affine_field() {
        let rows = [
            [1., 0., 0., 1.],
            [0., 1., 0., 1.],
            [0., 0., 1., 1.],
            [1., 1., 1., 1.],
            [2., -1., 3., 1.],
        ];
        let exact = [2., -3., 4., 0.5];
        let rhs: Vec<_> = rows
            .iter()
            .map(|r| (0..4).map(|k| r[k] * exact[k]).sum())
            .collect();
        let result = least_squares(&rows, &rhs, 5. * f64::EPSILON);
        for i in 0..4 {
            assert!((result[i] - exact[i]).abs() < 1e-11);
        }
    }
}
