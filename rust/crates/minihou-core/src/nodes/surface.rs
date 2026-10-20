//! Surface generation and modeling operations from minihou/nodes/surface.py.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use super::{GEO, Inputs, NodeSpec, linspace};
use crate::{
    error::{Error, Result},
    geometry::{Attr, AttrData, Geometry},
    params::Params,
    topo,
};

pub const CONVEX_HULL: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "convex_hull",
    inputs: GEO,
    params: crate::params!(
        "group": "";
        "type": "3d";
        "positions_2d": "fit_plane";
        "shrink_amount": 0.0;
        "preserve_attributes": true;
        "remove_inline_points": true
    ),
    cook: convex_hull,
};

pub const BRIDGE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "bridge",
    inputs: GEO,
    params: crate::params!(
        "align": true;
        "twist": 0;
        "pairing": "mutual_nearest";
        "outgrp_bridge": false;
        "outgrp_bridge_name": "bridgepolys"
    ),
    cook: bridge,
};

pub const REVOLVE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "revolve",
    inputs: GEO,
    params: crate::params!(
        "origin": [0.0, 0.0, 0.0];
        "axis": [0.0, 0.0, 1.0];
        "angle": 360.0;
        "divisions": 32;
        "start_angle": 0.0;
        "end_caps": false;
        "outgrp_endcaps": false;
        "outgrp_endcaps_name": "endcaps"
    ),
    cook: revolve,
};

pub const SKIN: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "skin",
    inputs: GEO,
    params: crate::params!(
        "samples": 0;
        "loop": false;
        "cap": false;
        "outgrp_surface": false;
        "outgrp_surface_name": "skinsurface";
        "outgrp_cap": false;
        "outgrp_cap_name": "skincap"
    ),
    cook: skin,
};

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn len3(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}

fn sub3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

// ---------------------------------------------------------------------------
// BRIDGE
// ---------------------------------------------------------------------------

struct DisjointSet {
    parent: Vec<usize>,
}

impl DisjointSet {
    fn new(n: usize) -> Self {
        Self { parent: (0..n).collect() }
    }
    fn find(&mut self, mut x: usize) -> usize {
        while self.parent[x] != x {
            self.parent[x] = self.parent[self.parent[x]];
            x = self.parent[x];
        }
        x
    }
    fn union(&mut self, a: usize, b: usize) {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra != rb {
            self.parent[rb] = ra;
        }
    }
}

fn boundary_loops(geo: &Geometry) -> Result<Vec<Vec<usize>>> {
    let e = topo::edges(&geo.topology, geo.npoints);
    let vp = &geo.topology.vertex_point;
    let mut succ = BTreeMap::new();
    let mut starts = Vec::new();

    for p in 0..geo.nprims() {
        let start = geo.topology.prim_offsets[p] as usize;
        let end = geo.topology.prim_offsets[p + 1] as usize;
        let n = end - start;
        for i in 0..n {
            let v = start + i;
            let nx = start + (i + 1) % n;
            let edge_idx = e.of_vertex[v];
            if edge_idx >= 0 && e.count[edge_idx as usize] == 1 {
                let a = vp[v] as usize;
                let b = vp[nx] as usize;
                if succ.insert(b, a).is_some() {
                    return Err(Error::new("the boundary touches itself (a point with several open edges); cannot fill it"));
                }
                starts.push(b);
            }
        }
    }

    let mut seen = BTreeSet::new();
    let mut loops = Vec::new();
    for start in starts {
        if seen.contains(&start) {
            continue;
        }
        let mut loop_pts = Vec::new();
        let mut at = start;
        while !seen.contains(&at) {
            seen.insert(at);
            loop_pts.push(at);
            if let Some(&nxt) = succ.get(&at) {
                at = nxt;
            } else {
                break;
            }
        }
        if at == start && loop_pts.len() >= 3 {
            loops.push(loop_pts);
        }
    }
    Ok(loops)
}

fn bridge_pair(la: &[usize], lb: &[usize], p_slice: &[f32], align: bool, twist: i64) -> Vec<Vec<i32>> {
    let mut lb_rev = lb.to_vec();
    lb_rev.reverse();
    let na = la.len();
    let nb = lb_rev.len();
    let mut s = 0usize;

    if align && na == nb {
        let mut best: Option<(f64, usize)> = None;
        for k in 0..nb {
            let mut d = 0.0;
            for i in 0..na {
                let pa = &p_slice[la[i] * 3..la[i] * 3 + 3];
                let pb = &p_slice[lb_rev[(i + k) % nb] * 3..lb_rev[(i + k) % nb] * 3 + 3];
                for c in 0..3 {
                    d += (pa[c] as f64 - pb[c] as f64).powi(2);
                }
            }
            if best.is_none() || d < best.unwrap().0 - 1e-12 {
                best = Some((d, k));
            }
        }
        s = best.unwrap().1;
    }

    s = (s as i64 + twist).rem_euclid(nb as i64) as usize;
    lb_rev.rotate_left(s);

    let mut faces = Vec::new();
    if na == nb {
        for i in 0..na {
            let j = (i + 1) % na;
            faces.push(vec![la[i] as i32, la[j] as i32, lb_rev[j] as i32, lb_rev[i] as i32]);
        }
        return faces;
    }

    let mut i = 0usize;
    let mut j = 0usize;
    while i < na || j < nb {
        let a0 = la[i % na];
        let a1 = la[(i + 1) % na];
        let b0 = lb_rev[j % nb];
        let b1 = lb_rev[(j + 1) % nb];

        let p_a1 = [p_slice[a1 * 3] as f64, p_slice[a1 * 3 + 1] as f64, p_slice[a1 * 3 + 2] as f64];
        let p_b0 = [p_slice[b0 * 3] as f64, p_slice[b0 * 3 + 1] as f64, p_slice[b0 * 3 + 2] as f64];
        let p_a0 = [p_slice[a0 * 3] as f64, p_slice[a0 * 3 + 1] as f64, p_slice[a0 * 3 + 2] as f64];
        let p_b1 = [p_slice[b1 * 3] as f64, p_slice[b1 * 3 + 1] as f64, p_slice[b1 * 3 + 2] as f64];

        let adv_a = i < na && (j >= nb || len3(sub3(p_a1, p_b0)) <= len3(sub3(p_a0, p_b1)));
        if adv_a {
            faces.push(vec![a0 as i32, a1 as i32, b0 as i32]);
            i += 1;
        } else {
            faces.push(vec![a0 as i32, b1 as i32, b0 as i32]);
            j += 1;
        }
    }
    faces
}

fn bridge(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = BRIDGE.one(inputs, 0)?;
    if geo.nprims() == 0 {
        return Err(Error::new("bridge needs polygons"));
    }
    let loops = boundary_loops(&geo)?;
    if loops.len() < 2 {
        return Err(Error::new(format!("bridge needs at least two open loops, the input has {}", loops.len())));
    }

    let align = p.b("align")?;
    let twist = p.i("twist")? as i64;
    let pairing = p.e("pairing", &["mutual_nearest", "in_order"])?;

    let p_slice = geo.p();
    let mut cent = Vec::with_capacity(loops.len());
    for l in &loops {
        let mut c = [0.0; 3];
        for &idx in l {
            for k in 0..3 {
                c[k] += p_slice[idx * 3 + k] as f64;
            }
        }
        for k in 0..3 {
            c[k] /= l.len() as f64;
        }
        cent.push(c);
    }

    let mut pairs = Vec::new();
    if pairing == "in_order" {
        for i in (0..loops.len().saturating_sub(1)).step_by(2) {
            pairs.push((i, i + 1));
        }
    } else {
        let e = topo::edges(&geo.topology, geo.npoints);
        let mut uf = DisjointSet::new(geo.npoints);
        for edge_idx in 0..e.len() {
            uf.union(e.points[edge_idx * 2] as usize, e.points[edge_idx * 2 + 1] as usize);
        }
        let piece: Vec<usize> = loops.iter().map(|l| uf.find(l[0])).collect();
        let num_pieces = piece.iter().copied().collect::<BTreeSet<_>>().len();

        let mut dist = vec![vec![f64::INFINITY; loops.len()]; loops.len()];
        for i in 0..loops.len() {
            for j in 0..loops.len() {
                if i == j {
                    continue;
                }
                if num_pieces > 1 && piece[i] == piece[j] {
                    continue;
                }
                dist[i][j] = len3(sub3(cent[i], cent[j]));
            }
        }

        let mut nearest = vec![0usize; loops.len()];
        for i in 0..loops.len() {
            let mut min_d = f64::INFINITY;
            let mut min_j = 0usize;
            for j in 0..loops.len() {
                if dist[i][j] < min_d {
                    min_d = dist[i][j];
                    min_j = j;
                }
            }
            nearest[i] = min_j;
        }

        for i in 0..loops.len() {
            let j = nearest[i];
            if i < j && dist[i][j].is_finite() && nearest[j] == i {
                pairs.push((i, j));
            }
        }
        if pairs.is_empty() {
            return Err(Error::new("bridge found no two loops that are each other's nearest; wire pieces with matching ends"));
        }
    }

    let mut new_faces = Vec::new();
    for (a, b) in pairs {
        new_faces.extend(bridge_pair(&loops[a], &loops[b], p_slice, align, twist));
    }

    let mut vp = geo.topology.vertex_point.clone();
    let mut offsets = geo.topology.prim_offsets.clone();
    let old_nprims = geo.nprims();
    let old_nverts = geo.nvertices();

    for face in &new_faces {
        vp.extend(face);
        offsets.push(vp.len() as i32);
    }

    let mut out = Geometry::from_arrays(geo.p().to_vec(), vp, offsets, None)?;
    out.point = geo.point.clone();
    out.detail = geo.detail.clone();

    // Pad vertex attributes with zero
    let new_verts_count = out.nvertices() - old_nverts;
    for (name, attr) in &geo.vertex {
        let mut padded = (**attr).clone();
        match &mut padded.data {
            AttrData::F32(v) => v.resize(v.len() + new_verts_count * attr.width, 0.0),
            AttrData::I32(v) => v.resize(v.len() + new_verts_count * attr.width, 0),
        }
        out.vertex.insert(name.clone(), Arc::new(padded));
    }

    // Pad prim attributes with zero
    let new_prims_count = out.nprims() - old_nprims;
    for (name, attr) in &geo.prim {
        let mut padded = (**attr).clone();
        match &mut padded.data {
            AttrData::F32(v) => v.resize(v.len() + new_prims_count * attr.width, 0.0),
            AttrData::I32(v) => v.resize(v.len() + new_prims_count * attr.width, 0),
        }
        out.prim.insert(name.clone(), Arc::new(padded));
    }

    if p.b("outgrp_bridge")? {
        let grp_name = p.s("outgrp_bridge_name")?;
        if !grp_name.is_empty() {
            let mut vals = vec![0i32; out.nprims()];
            for v in &mut vals[old_nprims..] {
                *v = 1;
            }
            out.prim.insert(grp_name, Arc::new(Attr::i32(1, vals)));
        }
    }

    Ok(out)
}

// ---------------------------------------------------------------------------
// REVOLVE
// ---------------------------------------------------------------------------

fn compact_points(geo: Geometry) -> Result<Geometry> {
    let mut used = vec![false; geo.npoints];
    for &v in &geo.topology.vertex_point {
        if v >= 0 && (v as usize) < geo.npoints {
            used[v as usize] = true;
        }
    }
    let mut remap = vec![-1i32; geo.npoints];
    let mut new_pts = Vec::new();
    let mut new_count = 0usize;
    let old_p = geo.p();
    for i in 0..geo.npoints {
        if used[i] {
            remap[i] = new_count as i32;
            new_count += 1;
            new_pts.extend_from_slice(&old_p[i * 3..i * 3 + 3]);
        }
    }
    let new_vp: Vec<i32> = geo.topology.vertex_point.iter().map(|&v| remap[v as usize]).collect();
    let mut out = Geometry::from_arrays(new_pts, new_vp, geo.topology.prim_offsets.clone(), geo.topology.closed.clone())?;
    for (name, attr) in &geo.point {
        if name == "P" {
            continue;
        }
        let mut new_attr = attr.zeros_like(new_count);
        match (&attr.data, &mut new_attr.data) {
            (AttrData::F32(src), AttrData::F32(dst)) => {
                let w = attr.width;
                for i in 0..geo.npoints {
                    if remap[i] >= 0 {
                        let target = remap[i] as usize;
                        dst[target * w..(target + 1) * w].copy_from_slice(&src[i * w..(i + 1) * w]);
                    }
                }
            }
            (AttrData::I32(src), AttrData::I32(dst)) => {
                let w = attr.width;
                for i in 0..geo.npoints {
                    if remap[i] >= 0 {
                        let target = remap[i] as usize;
                        dst[target * w..(target + 1) * w].copy_from_slice(&src[i * w..(i + 1) * w]);
                    }
                }
            }
            _ => unreachable!(),
        }
        out.point.insert(name.clone(), Arc::new(new_attr));
    }
    out.vertex = geo.vertex.clone();
    out.prim = geo.prim.clone();
    out.detail = geo.detail.clone();
    Ok(out)
}

fn revolve(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = REVOLVE.one(inputs, 0)?;
    let origin = p.v3("origin")?;
    let axis = p.v3("axis")?;
    let ax_len = len3(axis);
    if ax_len <= 1e-12 {
        return Err(Error::new("revolve needs a non-zero axis"));
    }
    let ax = [axis[0] / ax_len, axis[1] / ax_len, axis[2] / ax_len];

    let angle = p.f("angle")?;
    let divisions = p.i("divisions")?;
    if divisions < 1 {
        return Err(Error::new("divisions must be at least 1"));
    }
    let start_angle = p.f("start_angle")?;
    let end_caps = p.b("end_caps")?;

    let t = &geo.topology;
    let mut usable = Vec::new();
    for prim_idx in 0..geo.nprims() {
        let size = t.prim_offsets[prim_idx + 1] - t.prim_offsets[prim_idx];
        if size >= 2 {
            usable.push(prim_idx);
        }
    }
    if usable.is_empty() {
        return Geometry::from_arrays(vec![], vec![], vec![0], None);
    }

    let full = angle.abs() >= 360.0 - 1e-9;
    let steps = divisions as usize;
    let copies = if full { steps } else { steps + 1 };

    let n = geo.npoints;
    let p_slice = geo.p();
    let mut on_axis = vec![false; n];
    let mut rel = Vec::with_capacity(n);
    let mut along = Vec::with_capacity(n);
    let mut radial = Vec::with_capacity(n);

    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for i in 0..n {
        for k in 0..3 {
            lo[k] = lo[k].min(p_slice[i * 3 + k] as f64);
            hi[k] = hi[k].max(p_slice[i * 3 + k] as f64);
        }
    }
    let ptp_max = if n > 0 {
        (hi[0] - lo[0]).max(hi[1] - lo[1]).max(hi[2] - lo[2]).max(1e-9)
    } else {
        1.0
    };
    let axis_tol = 1e-7 * ptp_max;

    for i in 0..n {
        let p_pt = [p_slice[i * 3] as f64, p_slice[i * 3 + 1] as f64, p_slice[i * 3 + 2] as f64];
        let r_pt = sub3(p_pt, origin);
        let al = dot(r_pt, ax);
        let rad = sub3(r_pt, [al * ax[0], al * ax[1], al * ax[2]]);
        if len3(rad) <= axis_tol {
            on_axis[i] = true;
        }
        rel.push(r_pt);
        along.push(al);
        radial.push(rad);
    }

    let mut pos = Vec::with_capacity(copies * n * 3);
    let mut pid = vec![vec![0usize; n]; copies];

    for c in 0..copies {
        let th = (start_angle + angle * (c as f64 / steps as f64)).to_radians();
        let (sn, cs) = th.sin_cos();
        for i in 0..n {
            if on_axis[i] {
                pid[c][i] = pid[0][i];
            } else {
                pid[c][i] = c * n + i;
            }
            let rad = radial[i];
            let kx = cross(ax, rad);
            let rot = [
                rad[0] * cs + kx[0] * sn,
                rad[1] * cs + kx[1] * sn,
                rad[2] * cs + kx[2] * sn,
            ];
            let pt = [
                origin[0] + along[i] * ax[0] + rot[0],
                origin[1] + along[i] * ax[1] + rot[1],
                origin[2] + along[i] * ax[2] + rot[2],
            ];
            pos.push(pt[0] as f32);
            pos.push(pt[1] as f32);
            pos.push(pt[2] as f32);
        }
    }

    // Profile segments
    struct Segment {
        a: usize,
        b: usize,
        prim: usize,
        ta: f64,
        tb: f64,
    }
    let mut segs = Vec::new();
    for &p_idx in &usable {
        let start = t.prim_offsets[p_idx] as usize;
        let end = t.prim_offsets[p_idx + 1] as usize;
        let is_cl = t.is_closed(p_idx);
        let mut v: Vec<usize> = t.vertex_point[start..end].iter().map(|&x| x as usize).collect();
        if is_cl {
            v.push(v[0]);
        }
        let mut s_cum = vec![0.0];
        let mut total_len = 0.0;
        for i in 0..v.len() - 1 {
            let p0 = [p_slice[v[i] * 3] as f64, p_slice[v[i] * 3 + 1] as f64, p_slice[v[i] * 3 + 2] as f64];
            let p1 = [p_slice[v[i + 1] * 3] as f64, p_slice[v[i + 1] * 3 + 1] as f64, p_slice[v[i + 1] * 3 + 2] as f64];
            total_len += len3(sub3(p1, p0));
            s_cum.push(total_len);
        }
        let norm_s: Vec<f64> = if total_len > 0.0 {
            s_cum.iter().map(|&x| x / total_len).collect()
        } else {
            linspace(0.0, 1.0, v.len())
        };
        for i in 0..v.len() - 1 {
            segs.push(Segment {
                a: v[i],
                b: v[i + 1],
                prim: p_idx,
                ta: norm_s[i],
                tb: norm_s[i + 1],
            });
        }
    }

    let mut flat_v = Vec::new();
    let mut flat_uv = Vec::new();
    let mut sizes_out = Vec::new();
    let mut prim_src = Vec::new();

    for seg in &segs {
        for j in 0..steps {
            let j1 = if full { (j + 1) % copies } else { j + 1 };
            let a0 = pid[j][seg.a];
            let a1 = pid[j1][seg.a];
            let b1 = pid[j1][seg.b];
            let b0 = pid[j][seg.b];

            let eq01 = a0 == a1;
            let eq23 = b1 == b0;
            if eq01 && eq23 {
                continue;
            }

            let u0 = j as f32 / steps as f32;
            let u1 = (j + 1) as f32 / steps as f32;
            let va = seg.ta as f32;
            let vb = seg.tb as f32;

            let (corners, uvs) = if eq01 {
                (vec![a0 as i32, b1 as i32, b0 as i32], vec![[u0, va], [u1, vb], [u0, vb]])
            } else if eq23 {
                (vec![a0 as i32, a1 as i32, b1 as i32], vec![[u0, va], [u1, va], [u1, vb]])
            } else {
                (vec![a0 as i32, a1 as i32, b1 as i32, b0 as i32], vec![[u0, va], [u1, va], [u1, vb], [u0, vb]])
            };

            let mut corners = corners;
            let mut uvs = uvs;
            if angle < 0.0 {
                corners.reverse();
                uvs.reverse();
            }

            sizes_out.push(corners.len() as i32);
            flat_v.extend(corners);
            flat_uv.extend(uvs);
            prim_src.push(seg.prim);
        }
    }

    let nbody = sizes_out.len();
    if end_caps && !full {
        for &p_idx in &usable {
            let start = t.prim_offsets[p_idx] as usize;
            let end = t.prim_offsets[p_idx + 1] as usize;
            let v = &t.vertex_point[start..end];
            if v.len() < 3 {
                continue;
            }
            for (copy, forwards) in [(0, angle >= 0.0), (steps, angle < 0.0)] {
                let ids: Vec<i32> = if forwards {
                    v.iter().map(|&pt| pid[copy][pt as usize] as i32).collect()
                } else {
                    v.iter().rev().map(|&pt| pid[copy][pt as usize] as i32).collect()
                };
                let count = ids.len();
                flat_v.extend(ids);
                flat_uv.extend(vec![[0.0f32, 0.0f32]; count]);
                sizes_out.push(count as i32);
                prim_src.push(p_idx);
            }
        }
    }

    let mut offsets = vec![0i32];
    for &sz in &sizes_out {
        offsets.push(offsets.last().unwrap() + sz);
    }

    let mut out = Geometry::from_arrays(pos, flat_v, offsets, None)?;

    // Point attributes replicated copies times
    for (name, attr) in &geo.point {
        if name == "P" {
            continue;
        }
        let w = attr.width;
        let mut rep = attr.zeros_like(copies * n);
        match (&attr.data, &mut rep.data) {
            (AttrData::F32(src), AttrData::F32(dst)) => {
                for c in 0..copies {
                    dst[c * n * w..(c + 1) * n * w].copy_from_slice(src);
                }
            }
            (AttrData::I32(src), AttrData::I32(dst)) => {
                for c in 0..copies {
                    dst[c * n * w..(c + 1) * n * w].copy_from_slice(src);
                }
            }
            _ => unreachable!(),
        }
        out.point.insert(name.clone(), Arc::new(rep));
    }

    // Primitive attributes gathered
    for (name, attr) in &geo.prim {
        let w = attr.width;
        let mut gathered = attr.zeros_like(prim_src.len());
        match (&attr.data, &mut gathered.data) {
            (AttrData::F32(src), AttrData::F32(dst)) => {
                for (out_i, &src_i) in prim_src.iter().enumerate() {
                    dst[out_i * w..(out_i + 1) * w].copy_from_slice(&src[src_i * w..(src_i + 1) * w]);
                }
            }
            (AttrData::I32(src), AttrData::I32(dst)) => {
                for (out_i, &src_i) in prim_src.iter().enumerate() {
                    dst[out_i * w..(out_i + 1) * w].copy_from_slice(&src[src_i * w..(src_i + 1) * w]);
                }
            }
            _ => unreachable!(),
        }
        out.prim.insert(name.clone(), Arc::new(gathered));
    }

    // Vertex uv attribute
    let mut uv_flat = Vec::with_capacity(flat_uv.len() * 2);
    for uv in flat_uv {
        uv_flat.push(uv[0]);
        uv_flat.push(uv[1]);
    }
    out.vertex.insert("uv".into(), Arc::new(Attr::f32(2, uv_flat)));

    if p.b("outgrp_endcaps")? {
        let grp_name = p.s("outgrp_endcaps_name")?;
        if !grp_name.is_empty() {
            let mut vals = vec![0i32; out.nprims()];
            for v in &mut vals[nbody..] {
                *v = 1;
            }
            out.prim.insert(grp_name, Arc::new(Attr::i32(1, vals)));
        }
    }

    compact_points(out)
}

// ---------------------------------------------------------------------------
// SKIN
// ---------------------------------------------------------------------------

fn skin(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = SKIN.one(inputs, 0)?;
    let t = &geo.topology;
    if geo.nprims() < 2 {
        return Err(Error::new("skin needs at least two curves (primitives)"));
    }
    let closed_mask = t.closed_mask();
    let has_closed = closed_mask.iter().any(|&c| c);
    let all_closed = closed_mask.iter().all(|&c| c);
    if has_closed && !all_closed {
        return Err(Error::new("skin: mix of open and closed curves; all curves must be open or all closed"));
    }
    for prim_idx in 0..geo.nprims() {
        let sz = t.prim_offsets[prim_idx + 1] - t.prim_offsets[prim_idx];
        if sz < 2 {
            return Err(Error::new("skin: every curve needs at least two points"));
        }
    }

    let isclosed = all_closed;
    let samples = p.i("samples")?;
    let mut max_sz = 0;
    for prim_idx in 0..geo.nprims() {
        let sz = (t.prim_offsets[prim_idx + 1] - t.prim_offsets[prim_idx]) as usize;
        max_sz = max_sz.max(sz);
    }
    let mut n = if samples > 0 { samples as usize } else { max_sz };
    n = n.max(if isclosed { 3 } else { 2 });

    let loop_param = p.b("loop")?;
    let cap_param = p.b("cap")?;

    let p_slice = geo.p();
    let mut rows: Vec<Vec<[f64; 3]>> = Vec::new();

    for prim_idx in 0..geo.nprims() {
        let start = t.prim_offsets[prim_idx] as usize;
        let end = t.prim_offsets[prim_idx + 1] as usize;
        let mut pts: Vec<[f64; 3]> = (start..end)
            .map(|v| {
                let pt = t.vertex_point[v] as usize;
                [p_slice[pt * 3] as f64, p_slice[pt * 3 + 1] as f64, p_slice[pt * 3 + 2] as f64]
            })
            .collect();
        if isclosed {
            pts.push(pts[0]);
        }
        let mut s_cum = vec![0.0];
        let mut total = 0.0;
        for i in 0..pts.len() - 1 {
            total += len3(sub3(pts[i + 1], pts[i]));
            s_cum.push(total);
        }
        let total = if total > 0.0 { total } else { 1.0 };

        let u_vals: Vec<f64> = if isclosed {
            (0..n).map(|i| i as f64 / n as f64).collect()
        } else {
            linspace(0.0, 1.0, n)
        };

        let mut row = Vec::with_capacity(n);
        for &u in &u_vals {
            let target = u * total;
            let idx = s_cum[..pts.len() - 1]
                .partition_point(|&val| val <= target)
                .saturating_sub(1)
                .min(pts.len() - 2);
            let seg_len = s_cum[idx + 1] - s_cum[idx];
            let frac = if seg_len > 0.0 { ((target - s_cum[idx]) / seg_len).clamp(0.0, 1.0) } else { 0.0 };
            row.push([
                pts[idx][0] * (1.0 - frac) + pts[idx + 1][0] * frac,
                pts[idx][1] * (1.0 - frac) + pts[idx + 1][1] * frac,
                pts[idx][2] * (1.0 - frac) + pts[idx + 1][2] * frac,
            ]);
        }
        rows.push(row);
    }

    let m_curves = rows.len();
    let mut pos = Vec::with_capacity(m_curves * n * 3);
    for row in &rows {
        for pt in row {
            pos.push(pt[0] as f32);
            pos.push(pt[1] as f32);
            pos.push(pt[2] as f32);
        }
    }

    let nu = if isclosed { n } else { n - 1 };
    let nv = if loop_param { m_curves } else { m_curves - 1 };

    let mut quads_vp = Vec::with_capacity(nv * nu * 4);
    for i in 0..nv {
        let i1 = (i + 1) % m_curves;
        for j in 0..nu {
            let j1 = (j + 1) % n;
            quads_vp.push((i * n + j) as i32);
            quads_vp.push((i * n + j1) as i32);
            quads_vp.push((i1 * n + j1) as i32);
            quads_vp.push((i1 * n + j) as i32);
        }
    }
    let nquads = quads_vp.len() / 4;

    let mut verts = quads_vp;
    let mut sizes_out = vec![4i32; nquads];

    if cap_param && isclosed && !loop_param {
        // First cap (reversed)
        for j in (0..n).rev() {
            verts.push(j as i32);
        }
        sizes_out.push(n as i32);

        // Last cap
        for j in 0..n {
            verts.push(((m_curves - 1) * n + j) as i32);
        }
        sizes_out.push(n as i32);
    }

    let mut offsets = vec![0i32];
    for &sz in &sizes_out {
        offsets.push(offsets.last().unwrap() + sz);
    }

    let mut uv = Vec::with_capacity(m_curves * n * 2);
    let u_denom = if isclosed { n as f32 } else { (n - 1) as f32 };
    let v_denom = if loop_param { m_curves as f32 } else { (m_curves - 1) as f32 };
    for i in 0..m_curves {
        let v_val = i as f32 / v_denom;
        for j in 0..n {
            let u_val = j as f32 / u_denom;
            uv.push(u_val);
            uv.push(v_val);
        }
    }

    let mut out = Geometry::from_arrays(pos, verts, offsets, None)?;
    out.point.insert("uv".into(), Arc::new(Attr::f32(2, uv)));

    if p.b("outgrp_surface")? {
        let grp_name = p.s("outgrp_surface_name")?;
        if !grp_name.is_empty() {
            let mut vals = vec![0i32; out.nprims()];
            for v in &mut vals[..nquads] {
                *v = 1;
            }
            out.prim.insert(grp_name, Arc::new(Attr::i32(1, vals)));
        }
    }

    if p.b("outgrp_cap")? {
        let grp_name = p.s("outgrp_cap_name")?;
        if !grp_name.is_empty() {
            let mut vals = vec![0i32; out.nprims()];
            for v in &mut vals[nquads..] {
                *v = 1;
            }
            out.prim.insert(grp_name, Arc::new(Attr::i32(1, vals)));
        }
    }

    Ok(out)
}

fn hull_fallback(geo: &Geometry, selected: &[usize], preserve_attributes: bool) -> Result<Geometry> {
    let p_slice = geo.p();
    let mut pts = Vec::with_capacity(selected.len() * 3);
    for &i in selected {
        pts.extend_from_slice(&p_slice[i * 3..i * 3 + 3]);
    }
    let mut out = Geometry::points_only(pts)?;
    if preserve_attributes {
        for (name, attr) in &geo.point {
            if name == "P" {
                continue;
            }
            out.point.insert(name.clone(), Arc::new(super::group::gather(attr, selected)));
        }
        out.detail = geo.detail.clone();
    }
    Ok(out)
}

fn hull_planes(p: &[[f64; 3]], f: &[[usize; 3]]) -> (Vec<[f64; 3]>, Vec<f64>) {
    let mut n = Vec::with_capacity(f.len());
    let mut d = Vec::with_capacity(f.len());
    for tri in f {
        let p0 = p[tri[0]];
        let p1 = p[tri[1]];
        let p2 = p[tri[2]];
        let norm = cross(
            [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]],
            [p2[0] - p0[0], p2[1] - p0[1], p2[2] - p0[2]],
        );
        let len = (norm[0] * norm[0] + norm[1] * norm[1] + norm[2] * norm[2]).sqrt().max(1e-300);
        let un = [norm[0] / len, norm[1] / len, norm[2] / len];
        let dist = un[0] * p0[0] + un[1] * p0[1] + un[2] * p0[2];
        n.push(un);
        d.push(dist);
    }
    (n, d)
}

fn orient_outward(p: &[[f64; 3]], mut f: Vec<[usize; 3]>) -> Vec<[usize; 3]> {
    let mut used_pts = std::collections::HashSet::new();
    for tri in &f {
        used_pts.insert(tri[0]);
        used_pts.insert(tri[1]);
        used_pts.insert(tri[2]);
    }
    let m = used_pts.len() as f64;
    let mut inside = [0.0; 3];
    for &idx in &used_pts {
        inside[0] += p[idx][0];
        inside[1] += p[idx][1];
        inside[2] += p[idx][2];
    }
    inside[0] /= m;
    inside[1] /= m;
    inside[2] /= m;

    let (normals, d) = hull_planes(p, &f);
    for i in 0..f.len() {
        let n = normals[i];
        let dot_inside = n[0] * inside[0] + n[1] * inside[1] + n[2] * inside[2];
        if dot_inside - d[i] > 0.0 {
            f[i] = [f[i][0], f[i][2], f[i][1]];
        }
    }
    f
}

fn closest_point_tri(p: [f64; 3], tri: &[[f64; 3]; 3]) -> [f64; 3] {
    let [a, b, c] = *tri;
    let ab = sub3(b, a);
    let ac = sub3(c, a);
    let ap = sub3(p, a);
    let d1 = dot(ab, ap);
    let d2 = dot(ac, ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = sub3(p, b);
    let d3 = dot(ab, bp);
    let d4 = dot(ac, bp);
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return [a[0] + v * ab[0], a[1] + v * ab[1], a[2] + v * ab[2]];
    }
    let cp = sub3(p, c);
    let d5 = dot(ab, cp);
    let d6 = dot(ac, cp);
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return [a[0] + w * ac[0], a[1] + w * ac[1], a[2] + w * ac[2]];
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        let bc = sub3(c, b);
        return [b[0] + w * bc[0], b[1] + w * bc[1], b[2] + w * bc[2]];
    }
    let denom = 1.0 / (va + vb + vc);
    let v = vb * denom;
    let w = vc * denom;
    [
        a[0] + ab[0] * v + ac[0] * w,
        a[1] + ab[1] * v + ac[1] * w,
        a[2] + ab[2] * v + ac[2] * w,
    ]
}

/// Remove collinear middle vertices along a closed polygon loop (in 3D).
fn remove_collinear_loop(loop_pts: Vec<i32>, pts64: &[[f64; 3]]) -> Vec<i32> {
    let mut cur = loop_pts;
    loop {
        let k = cur.len();
        if k <= 3 {
            break;
        }
        let mut next_loop = Vec::with_capacity(k);
        let mut changed = false;
        for i in 0..k {
            let prev = cur[(i + k - 1) % k] as usize;
            let curr = cur[i] as usize;
            let next = cur[(i + 1) % k] as usize;
            if prev == curr || curr == next {
                changed = true;
                continue;
            }
            let v1 = sub3(pts64[curr], pts64[prev]);
            let v2 = sub3(pts64[next], pts64[curr]);
            let l1 = len3(v1);
            let l2 = len3(v2);
            if l1 <= 1e-12 || l2 <= 1e-12 {
                changed = true;
                continue;
            }
            let sin_theta = len3(cross(v1, v2)) / (l1 * l2);
            let cos_theta = dot(v1, v2) / (l1 * l2);
            if sin_theta <= 1e-5 && cos_theta > 0.0 {
                changed = true;
                continue;
            }
            next_loop.push(cur[i]);
        }
        cur = next_loop;
        if !changed {
            break;
        }
    }
    cur
}

/// Merge coplanar adjacent triangles into n-gon faces and remove inline boundary/interior vertices.
fn merge_coplanar_hull_faces(tris: &[[i32; 3]], pts64: &[[f64; 3]]) -> (Vec<i32>, Vec<i32>) {
    let ntris = tris.len();
    if ntris == 0 {
        return (Vec::new(), vec![0]);
    }
    let mut normals = Vec::with_capacity(ntris);
    for t in tris {
        let p0 = pts64[t[0] as usize];
        let p1 = pts64[t[1] as usize];
        let p2 = pts64[t[2] as usize];
        let cr = cross(sub3(p1, p0), sub3(p2, p0));
        let l = len3(cr).max(1e-300);
        normals.push([cr[0] / l, cr[1] / l, cr[2] / l]);
    }

    // Map directed half-edge (u, v) -> triangle index
    let mut he_to_tri = BTreeMap::<(i32, i32), usize>::new();
    for (ti, t) in tris.iter().enumerate() {
        he_to_tri.insert((t[0], t[1]), ti);
        he_to_tri.insert((t[1], t[2]), ti);
        he_to_tri.insert((t[2], t[0]), ti);
    }

    let cos_tol = 1.0 - 1e-7;
    let mut pairs = Vec::new();
    for (&(u, v), &ta) in &he_to_tri {
        if u < v {
            if let Some(&tb) = he_to_tri.get(&(v, u)) {
                let d = dot(normals[ta], normals[tb]);
                if d >= cos_tol {
                    pairs.push((d, ta, tb));
                }
            }
        }
    }
    pairs.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));

    let mut parent: Vec<usize> = (0..ntris).collect();
    fn find(parent: &mut [usize], mut x: usize) -> usize {
        while parent[x] != x {
            parent[x] = parent[parent[x]];
            x = parent[x];
        }
        x
    }
    for (_, ta, tb) in pairs {
        let ra = find(&mut parent, ta);
        let rb = find(&mut parent, tb);
        if ra != rb && dot(normals[ra], normals[rb]) >= cos_tol {
            let (lo, hi) = if ra < rb { (ra, rb) } else { (rb, ra) };
            parent[hi] = lo;
        }
    }

    let mut groups = BTreeMap::<usize, Vec<usize>>::new();
    for ti in 0..ntris {
        let r = find(&mut parent, ti);
        groups.entry(r).or_default().push(ti);
    }

    let mut out_vp = Vec::new();
    let mut out_offs = vec![0i32];

    for (_rep, group_tris) in groups {
        if group_tris.len() == 1 {
            let t = tris[group_tris[0]];
            out_vp.extend_from_slice(&t);
            out_offs.push(out_vp.len() as i32);
            continue;
        }
        let group_set: BTreeSet<usize> = group_tris.iter().copied().collect();
        let mut succ = BTreeMap::<i32, i32>::new();
        let mut valid = true;
        for &ti in &group_tris {
            let t = tris[ti];
            for &(u, v) in &[(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                let twin_in_group = he_to_tri.get(&(v, u)).is_some_and(|&other| group_set.contains(&other));
                if !twin_in_group {
                    if succ.insert(u, v).is_some() {
                        valid = false;
                        break;
                    }
                }
            }
            if !valid {
                break;
            }
        }
        if valid && !succ.is_empty() {
            let start = *succ.keys().next().unwrap();
            let mut loop_pts = Vec::with_capacity(succ.len());
            let mut curr = start;
            let mut visited = BTreeSet::new();
            while visited.insert(curr) {
                loop_pts.push(curr);
                if let Some(&nxt) = succ.get(&curr) {
                    curr = nxt;
                } else {
                    valid = false;
                    break;
                }
            }
            if valid && curr == start && loop_pts.len() == succ.len() {
                let cleaned = remove_collinear_loop(loop_pts, pts64);
                if cleaned.len() >= 3 {
                    out_vp.extend(cleaned);
                    out_offs.push(out_vp.len() as i32);
                    continue;
                }
            }
        }
        // Fallback if non-simple boundary: keep individual triangles
        for &ti in &group_tris {
            let t = tris[ti];
            out_vp.extend_from_slice(&t);
            out_offs.push(out_vp.len() as i32);
        }
    }

    (out_vp, out_offs)
}

/// 2D convex hull using Andrew's monotone chain.
/// Returns indices into `pts2d` in counter-clockwise order.
fn convex_hull_2d(pts2d: &[[f64; 2]], scale: f64, remove_inline: bool) -> Vec<usize> {
    let n = pts2d.len();
    if n == 0 {
        return Vec::new();
    }
    let eps = 1e-10 * scale.max(1e-12);
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| {
        pts2d[a][0]
            .total_cmp(&pts2d[b][0])
            .then_with(|| pts2d[a][1].total_cmp(&pts2d[b][1]))
            .then_with(|| a.cmp(&b))
    });

    // Deduplicate identical 2D coordinates (keep first index)
    let mut uniq: Vec<usize> = Vec::with_capacity(n);
    for &idx in &order {
        if let Some(&last) = uniq.last() {
            let pa = pts2d[last];
            let pb = pts2d[idx];
            if (pa[0] - pb[0]).abs() <= eps && (pa[1] - pb[1]).abs() <= eps {
                continue;
            }
        }
        uniq.push(idx);
    }

    if uniq.len() < 3 {
        return Vec::new();
    }

    let cross2d = |o: [f64; 2], a: [f64; 2], b: [f64; 2]| -> f64 {
        (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
    };

    let should_pop = |o: [f64; 2], a: [f64; 2], b: [f64; 2]| -> bool {
        let cr = cross2d(o, a, b);
        if remove_inline {
            let la = ((a[0] - o[0]).powi(2) + (a[1] - o[1]).powi(2)).sqrt();
            let lb = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
            cr <= 1e-6 * la * lb + eps * eps
        } else {
            cr < -eps * eps
        }
    };

    let mut lower: Vec<usize> = Vec::new();
    for &idx in &uniq {
        while lower.len() >= 2
            && should_pop(
                pts2d[lower[lower.len() - 2]],
                pts2d[lower[lower.len() - 1]],
                pts2d[idx],
            )
        {
            lower.pop();
        }
        lower.push(idx);
    }

    let mut upper: Vec<usize> = Vec::new();
    for &idx in uniq.iter().rev() {
        while upper.len() >= 2
            && should_pop(
                pts2d[upper[upper.len() - 2]],
                pts2d[upper[upper.len() - 1]],
                pts2d[idx],
            )
        {
            upper.pop();
        }
        upper.push(idx);
    }

    lower.pop();
    upper.pop();
    lower.extend(upper);

    // Check area > 0 (not all collinear)
    if lower.len() < 3 {
        return Vec::new();
    }
    let mut area2 = 0.0;
    let p0 = pts2d[lower[0]];
    for i in 1..lower.len() - 1 {
        area2 += cross2d(p0, pts2d[lower[i]], pts2d[lower[i + 1]]);
    }
    if area2.abs() <= eps * scale.max(1e-12) {
        return Vec::new();
    }

    lower
}

/// Apply Houdini Shrinkwrap ("Shrink Amount") to the hull points.
///
/// Analytic / iterative relaxation toward the input point cloud and local hull neighborhood:
/// Each hull vertex computes a target position that blends its local neighborhood centroid on the hull
/// with the inverse-distance weighted attraction of nearby interior input points, then moves toward
/// that target by `shrink_amount` (clamped to `[0, 1]`). Because both the hull neighbors and the
/// interior point cloud lie within the convex hull, every displaced point is a convex combination of
/// points inside the original hull, guaranteeing the shrunk hull stays strictly within the original
/// convex hull volume.
fn apply_shrink(
    hull_pts: &mut [[f64; 3]],
    vp: &[i32],
    offs: &[i32],
    cloud_pts: &[[f64; 3]],
    shrink_amount: f64,
) {
    let s = shrink_amount.clamp(0.0, 1.0);
    if s <= 0.0 || hull_pts.is_empty() {
        return;
    }
    let n = hull_pts.len();

    // Build adjacency on the hull mesh
    let mut adj = vec![BTreeSet::<usize>::new(); n];
    for w in offs.windows(2) {
        let start = w[0] as usize;
        let end = w[1] as usize;
        let k = end.saturating_sub(start);
        if k < 2 {
            continue;
        }
        for i in 0..k {
            let a = vp[start + i] as usize;
            let b = vp[start + (i + 1) % k] as usize;
            if a != b {
                adj[a].insert(b);
                adj[b].insert(a);
            }
        }
    }

    // Overall centroid of the point cloud
    let mut cloud_centroid = [0.0; 3];
    if !cloud_pts.is_empty() {
        for p in cloud_pts {
            cloud_centroid[0] += p[0];
            cloud_centroid[1] += p[1];
            cloud_centroid[2] += p[2];
        }
        let inv = 1.0 / cloud_pts.len() as f64;
        cloud_centroid[0] *= inv;
        cloud_centroid[1] *= inv;
        cloud_centroid[2] *= inv;
    }

    // Identify strictly interior points (not coinciding with hull vertices)
    let mut interior_pts = Vec::new();
    for cp in cloud_pts {
        let is_hull = hull_pts.iter().any(|hp| len3(sub3(*cp, *hp)) <= 1e-7);
        if !is_hull {
            interior_pts.push(*cp);
        }
    }

    let iterations = 4;
    let step = 1.0 - (1.0 - 0.65 * s).powf(1.0 / iterations as f64);

    for _ in 0..iterations {
        let prev = hull_pts.to_vec();
        for i in 0..n {
            // 1. Laplacian neighbor average on the hull
            let mut lap = [0.0; 3];
            if !adj[i].is_empty() {
                for &nb in &adj[i] {
                    lap[0] += prev[nb][0];
                    lap[1] += prev[nb][1];
                    lap[2] += prev[nb][2];
                }
                let inv = 1.0 / adj[i].len() as f64;
                lap[0] *= inv;
                lap[1] *= inv;
                lap[2] *= inv;
            } else {
                lap = cloud_centroid;
            }

            // 2. Interior point cloud attraction (inverse-distance squared weighted centroid)
            let target = if !interior_pts.is_empty() {
                let mut w_sum = 0.0;
                let mut attr_pt = [0.0; 3];
                for ip in &interior_pts {
                    let d2 = dot(sub3(*ip, prev[i]), sub3(*ip, prev[i])).max(1e-12);
                    let w = 1.0 / d2;
                    w_sum += w;
                    attr_pt[0] += w * ip[0];
                    attr_pt[1] += w * ip[1];
                    attr_pt[2] += w * ip[2];
                }
                attr_pt[0] /= w_sum;
                attr_pt[1] /= w_sum;
                attr_pt[2] /= w_sum;
                [
                    0.5 * lap[0] + 0.5 * attr_pt[0],
                    0.5 * lap[1] + 0.5 * attr_pt[1],
                    0.5 * lap[2] + 0.5 * attr_pt[2],
                ]
            } else {
                lap
            };

            hull_pts[i] = [
                prev[i][0] * (1.0 - step) + target[0] * step,
                prev[i][1] * (1.0 - step) + target[1] * step,
                prev[i][2] * (1.0 - step) + target[2] * step,
            ];
        }
    }
}

fn build_hull_output(
    geo: &Geometry,
    vp: Vec<i32>,
    offs: Vec<i32>,
    selected_pts64: &[[f64; 3]],
    preserve_attributes: bool,
    shrink_amount: f64,
) -> Result<Geometry> {
    let mut used = vec![false; geo.npoints];
    for &v in &vp {
        if v >= 0 && (v as usize) < geo.npoints {
            used[v as usize] = true;
        }
    }
    let mut remap = vec![-1i32; geo.npoints];
    let mut kept_src = Vec::new();
    let mut hull_pts64 = Vec::new();
    let old_p = geo.p();
    for i in 0..geo.npoints {
        if used[i] {
            remap[i] = kept_src.len() as i32;
            kept_src.push(i);
            hull_pts64.push([
                old_p[i * 3] as f64,
                old_p[i * 3 + 1] as f64,
                old_p[i * 3 + 2] as f64,
            ]);
        }
    }

    let new_vp: Vec<i32> = vp.iter().map(|&v| remap[v as usize]).collect();

    if shrink_amount > 0.0 {
        apply_shrink(&mut hull_pts64, &new_vp, &offs, selected_pts64, shrink_amount);
    }

    let mut new_pts = Vec::with_capacity(hull_pts64.len() * 3);
    for pt in &hull_pts64 {
        new_pts.push(pt[0] as f32);
        new_pts.push(pt[1] as f32);
        new_pts.push(pt[2] as f32);
    }

    let mut out = Geometry::from_arrays(new_pts, new_vp, offs, None)?;
    if preserve_attributes {
        for (name, attr) in &geo.point {
            if name == "P" {
                continue;
            }
            out.point.insert(name.clone(), Arc::new(super::group::gather(attr, &kept_src)));
        }
        out.detail = geo.detail.clone();
    }
    Ok(out)
}

fn convex_hull_2d_mode(
    geo: &Geometry,
    selected: &[usize],
    positions_2d: &str,
    shrink_amount: f64,
    preserve_attributes: bool,
    remove_inline_points: bool,
) -> Result<Geometry> {
    if selected.len() < 3 {
        return hull_fallback(geo, selected, preserve_attributes);
    }
    let p_slice = geo.p();
    let sel_pts64: Vec<[f64; 3]> = selected
        .iter()
        .map(|&i| [p_slice[i * 3] as f64, p_slice[i * 3 + 1] as f64, p_slice[i * 3 + 2] as f64])
        .collect();

    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for p in &sel_pts64 {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    let scale = (hi[0] - lo[0]).max(hi[1] - lo[1]).max(hi[2] - lo[2]);
    if scale <= 1e-12 {
        return hull_fallback(geo, selected, preserve_attributes);
    }

    let (pts2d, plane_normal): (Vec<[f64; 2]>, [f64; 3]) = match positions_2d {
        "xy" => (
            sel_pts64.iter().map(|p| [p[0], p[1]]).collect(),
            [0.0, 0.0, 1.0],
        ),
        "yz" => (
            sel_pts64.iter().map(|p| [p[1], p[2]]).collect(),
            [1.0, 0.0, 0.0],
        ),
        "zx" => (
            sel_pts64.iter().map(|p| [p[2], p[0]]).collect(),
            [0.0, 1.0, 0.0],
        ),
        _ => {
            // fit_plane: best-fit plane through selected points via covariance eigenvectors
            let m = sel_pts64.len() as f64;
            let mut mean = [0.0; 3];
            for p in &sel_pts64 {
                mean[0] += p[0];
                mean[1] += p[1];
                mean[2] += p[2];
            }
            mean[0] /= m;
            mean[1] /= m;
            mean[2] /= m;

            let mut cov = [[0.0; 3]; 3];
            for p in &sel_pts64 {
                let r = sub3(*p, mean);
                for a in 0..3 {
                    for b in 0..3 {
                        cov[a][b] += r[a] * r[b];
                    }
                }
            }
            let (evals, v) = super::model::eigen(cov);
            let mut order = [0, 1, 2];
            order.sort_by(|&a, &b| evals[b].total_cmp(&evals[a]));
            let u = [v[0][order[0]], v[1][order[0]], v[2][order[0]]];
            let mut w = [v[0][order[1]], v[1][order[1]], v[2][order[1]]];
            let mut nrm = cross(u, w);
            // Canonicalize normal sign so highest-magnitude component is positive
            let mut max_k = 0;
            if nrm[1].abs() > nrm[max_k].abs() { max_k = 1; }
            if nrm[2].abs() > nrm[max_k].abs() { max_k = 2; }
            if nrm[max_k] < 0.0 {
                w = [-w[0], -w[1], -w[2]];
                nrm = [-nrm[0], -nrm[1], -nrm[2]];
            }
            let proj = sel_pts64
                .iter()
                .map(|p| {
                    let r = sub3(*p, mean);
                    [dot(r, u), dot(r, w)]
                })
                .collect();
            (proj, nrm)
        }
    };

    let hull_local = convex_hull_2d(&pts2d, scale, remove_inline_points);
    if hull_local.len() < 3 {
        return hull_fallback(geo, selected, preserve_attributes);
    }

    let mut vp: Vec<i32> = hull_local.iter().map(|&li| selected[li] as i32).collect();

    // Ensure consistent winding with plane_normal
    let p0 = sel_pts64[hull_local[0]];
    let mut poly_n = [0.0; 3];
    for i in 1..hull_local.len() - 1 {
        let p1 = sel_pts64[hull_local[i]];
        let p2 = sel_pts64[hull_local[i + 1]];
        let cr = cross(sub3(p1, p0), sub3(p2, p0));
        poly_n[0] += cr[0];
        poly_n[1] += cr[1];
        poly_n[2] += cr[2];
    }
    if dot(poly_n, plane_normal) < 0.0 {
        vp.reverse();
    }

    let offs = vec![0i32, vp.len() as i32];
    build_hull_output(geo, vp, offs, &sel_pts64, preserve_attributes, shrink_amount)
}

fn convex_hull(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = CONVEX_HULL.one(inputs, 0)?;
    let group_str = p.s("group")?;
    let hull_type = p.e("type", &["3d", "2d"])?;
    let positions_2d = p.e("positions_2d", &["fit_plane", "xy", "yz", "zx"])?;
    let shrink_amount = p.f("shrink_amount")?;
    let preserve_attributes = p.b("preserve_attributes")?;
    let remove_inline_points = p.b("remove_inline_points")?;

    let mask = super::groups::point_mask(&geo, &group_str)?;
    let selected: Vec<usize> = (0..geo.npoints).filter(|&i| mask[i]).collect();

    if hull_type == "2d" {
        return convex_hull_2d_mode(
            &geo,
            &selected,
            &positions_2d,
            shrink_amount,
            preserve_attributes,
            remove_inline_points,
        );
    }

    let n = selected.len();
    if n < 4 {
        return hull_fallback(&geo, &selected, preserve_attributes);
    }

    let p_slice = geo.p();
    let all_pts64: Vec<[f64; 3]> = (0..geo.npoints)
        .map(|i| [p_slice[i * 3] as f64, p_slice[i * 3 + 1] as f64, p_slice[i * 3 + 2] as f64])
        .collect();
    let sel_pts64: Vec<[f64; 3]> = selected.iter().map(|&i| all_pts64[i]).collect();

    let mut order: Vec<usize> = selected.clone();
    order.sort_by(|&a, &b| {
        let pa = all_pts64[a];
        let pb = all_pts64[b];
        pa[0]
            .total_cmp(&pb[0])
            .then_with(|| pa[1].total_cmp(&pb[1]))
            .then_with(|| pa[2].total_cmp(&pb[2]))
            .then_with(|| a.cmp(&b))
    });

    let mut uniq_pts = Vec::new();
    let mut first_indices: Vec<usize> = Vec::new();
    for &idx in &order {
        if let Some(&last_idx) = first_indices.last() {
            let p_last = all_pts64[last_idx];
            let p_curr = all_pts64[idx];
            if p_curr[0] == p_last[0] && p_curr[1] == p_last[1] && p_curr[2] == p_last[2] {
                continue;
            }
        }
        uniq_pts.push(all_pts64[idx]);
        first_indices.push(idx);
    }
    if uniq_pts.len() < 4 {
        return hull_fallback(&geo, &selected, preserve_attributes);
    }

    let m = uniq_pts.len() as f64;
    let mut mean = [0.0; 3];
    for pt in &uniq_pts {
        mean[0] += pt[0];
        mean[1] += pt[1];
        mean[2] += pt[2];
    }
    mean[0] /= m;
    mean[1] /= m;
    mean[2] /= m;

    let q: Vec<[f64; 3]> = uniq_pts
        .iter()
        .map(|pt| [pt[0] - mean[0], pt[1] - mean[1], pt[2] - mean[2]])
        .collect();
    let mut min_q = [f64::INFINITY; 3];
    let mut max_q = [f64::NEG_INFINITY; 3];
    for pt in &q {
        for k in 0..3 {
            min_q[k] = min_q[k].min(pt[k]);
            max_q[k] = max_q[k].max(pt[k]);
        }
    }
    let scale = (max_q[0] - min_q[0])
        .max(max_q[1] - min_q[1])
        .max(max_q[2] - min_q[2]);
    let eps = 1e-10 * scale;
    let q_len = q.len();

    let mut i0 = 0;
    let mut i1 = 0;
    for i in 1..q_len {
        if q[i][0] < q[i0][0] {
            i0 = i;
        }
        if q[i][0] > q[i1][0] {
            i1 = i;
        }
    }
    if i0 == i1 {
        return hull_fallback(&geo, &selected, preserve_attributes);
    }

    let mut d = [q[i1][0] - q[i0][0], q[i1][1] - q[i0][1], q[i1][2] - q[i0][2]];
    let d_len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    if d_len <= eps {
        return hull_fallback(&geo, &selected, preserve_attributes);
    }
    d[0] /= d_len;
    d[1] /= d_len;
    d[2] /= d_len;

    let mut max_line_dist = -1.0;
    let mut i2 = 0;
    for i in 0..q_len {
        let off = [q[i][0] - q[i0][0], q[i][1] - q[i0][1], q[i][2] - q[i0][2]];
        let proj = off[0] * d[0] + off[1] * d[1] + off[2] * d[2];
        let perp = [
            off[0] - proj * d[0],
            off[1] - proj * d[1],
            off[2] - proj * d[2],
        ];
        let dist = (perp[0] * perp[0] + perp[1] * perp[1] + perp[2] * perp[2]).sqrt();
        if dist > max_line_dist {
            max_line_dist = dist;
            i2 = i;
        }
    }
    if max_line_dist <= eps {
        return hull_fallback(&geo, &selected, preserve_attributes);
    }

    let mut nrm = cross(
        [q[i1][0] - q[i0][0], q[i1][1] - q[i0][1], q[i1][2] - q[i0][2]],
        [q[i2][0] - q[i0][0], q[i2][1] - q[i0][1], q[i2][2] - q[i0][2]],
    );
    let nrm_len = (nrm[0] * nrm[0] + nrm[1] * nrm[1] + nrm[2] * nrm[2]).sqrt();
    if nrm_len <= eps {
        return hull_fallback(&geo, &selected, preserve_attributes);
    }
    nrm[0] /= nrm_len;
    nrm[1] /= nrm_len;
    nrm[2] /= nrm_len;

    let mut max_plane_dist = -1.0;
    let mut i3 = 0;
    for i in 0..q_len {
        let off = [q[i][0] - q[i0][0], q[i][1] - q[i0][1], q[i][2] - q[i0][2]];
        let dist = (off[0] * nrm[0] + off[1] * nrm[1] + off[2] * nrm[2]).abs();
        if dist > max_plane_dist {
            max_plane_dist = dist;
            i3 = i;
        }
    }
    if max_plane_dist <= eps {
        return hull_fallback(&geo, &selected, preserve_attributes);
    }

    let mut faces = vec![[i0, i1, i2], [i0, i3, i1], [i1, i3, i2], [i2, i3, i0]];
    faces = orient_outward(&q, faces);

    let mut done = vec![false; q_len];
    done[i0] = true;
    done[i1] = true;
    done[i2] = true;
    done[i3] = true;

    let mut order_q: Vec<usize> = (0..q_len).collect();
    order_q.sort_by(|&a, &b| {
        let da = q[a][0] * q[a][0] + q[a][1] * q[a][1] + q[a][2] * q[a][2];
        let db = q[b][0] * q[b][0] + q[b][1] * q[b][1] + q[b][2] * q[b][2];
        db.total_cmp(&da)
    });

    let (mut n_planes, mut d_planes) = hull_planes(&q, &faces);

    for &pt_i in &order_q {
        if done[pt_i] {
            continue;
        }
        let qp = q[pt_i];
        let mut vis = Vec::with_capacity(faces.len());
        let mut any_vis = false;
        let vis_tol = if remove_inline_points { eps } else { -eps };
        for f in 0..faces.len() {
            let signed_dist =
                n_planes[f][0] * qp[0] + n_planes[f][1] * qp[1] + n_planes[f][2] * qp[2] - d_planes[f];
            let is_vis = if remove_inline_points {
                signed_dist > vis_tol
            } else {
                // When keeping inline/coplanar points, also split faces when point lies on the face/edge within eps
                if signed_dist > eps {
                    true
                } else if signed_dist >= -eps {
                    let tri = [q[faces[f][0]], q[faces[f][1]], q[faces[f][2]]];
                    let cp = closest_point_tri(qp, &tri);
                    len3(sub3(qp, cp)) <= 1e-7 * scale
                } else {
                    false
                }
            };
            vis.push(is_vis);
            if is_vis {
                any_vis = true;
            }
        }
        if !any_vis {
            continue;
        }

        let mut edges = std::collections::HashSet::new();
        for f in 0..faces.len() {
            if vis[f] {
                let tri = faces[f];
                edges.insert((tri[0], tri[1]));
                edges.insert((tri[1], tri[2]));
                edges.insert((tri[2], tri[0]));
            }
        }

        let mut horizon = Vec::new();
        for &(a, b) in &edges {
            if !edges.contains(&(b, a)) {
                horizon.push((a, b));
            }
        }

        let mut new_faces = Vec::new();
        for &(a, b) in &horizon {
            let nf = [a, b, pt_i];
            let cr = cross(sub3(q[b], q[a]), sub3(qp, q[a]));
            if len3(cr) > 1e-12 * scale * scale {
                new_faces.push(nf);
            }
        }
        if new_faces.is_empty() {
            continue;
        }

        let (new_n, new_d) = hull_planes(&q, &new_faces);

        let mut next_faces = Vec::new();
        let mut next_n = Vec::new();
        let mut next_d = Vec::new();
        for f in 0..faces.len() {
            if !vis[f] {
                next_faces.push(faces[f]);
                next_n.push(n_planes[f]);
                next_d.push(d_planes[f]);
            }
        }
        next_faces.extend(new_faces);
        next_n.extend(new_n);
        next_d.extend(new_d);

        faces = next_faces;
        n_planes = next_n;
        d_planes = next_d;
    }

    if faces.is_empty() {
        return hull_fallback(&geo, &selected, preserve_attributes);
    }

    faces = orient_outward(&q, faces);

    let tris_src: Vec<[i32; 3]> = faces
        .iter()
        .map(|tri| {
            [
                first_indices[tri[0]] as i32,
                first_indices[tri[1]] as i32,
                first_indices[tri[2]] as i32,
            ]
        })
        .collect();

    let (vp, offs) = if remove_inline_points {
        merge_coplanar_hull_faces(&tris_src, &all_pts64)
    } else {
        let mut vp = Vec::with_capacity(tris_src.len() * 3);
        let mut offs = Vec::with_capacity(tris_src.len() + 1);
        offs.push(0i32);
        for tri in &tris_src {
            vp.extend_from_slice(tri);
            offs.push(vp.len() as i32);
        }
        (vp, offs)
    };

    build_hull_output(&geo, vp, offs, &sel_pts64, preserve_attributes, shrink_amount)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_skin_two_lines() {
        let pts = vec![
            0.0, 0.0, 0.0,  1.0, 0.0, 0.0,
            0.0, 1.0, 0.0,  1.0, 1.0, 0.0,
        ];
        let vp = vec![0, 1, 2, 3];
        let offs = vec![0, 2, 4];
        let geo = Geometry::from_arrays(pts, vp, offs, Some(vec![false, false])).unwrap();
        let p = Params::resolve(SKIN.params, None).unwrap();
        let out = skin(&[vec![Arc::new(geo)]], &p).unwrap();
        assert_eq!(out.nprims(), 1);
        assert_eq!(out.topology.prim_offsets, vec![0, 4]);
    }

    #[test]
    fn test_revolve_line() {
        let pts = vec![1.0, 0.0, 0.0, 1.0, 0.0, 1.0];
        let vp = vec![0, 1];
        let offs = vec![0, 2];
        let geo = Geometry::from_arrays(pts, vp, offs, Some(vec![false])).unwrap();
        let p = Params::resolve(REVOLVE.params, serde_json::json!({"divisions": 4}).as_object()).unwrap();
        let out = revolve(&[vec![Arc::new(geo)]], &p).unwrap();
        assert_eq!(out.nprims(), 4);
    }

    #[test]
    fn test_convex_hull_cube() {
        let pts = vec![
            0.0, 0.0, 0.0,
            1.0, 0.0, 0.0,
            1.0, 1.0, 0.0,
            0.0, 1.0, 0.0,
            0.0, 0.0, 1.0,
            1.0, 0.0, 1.0,
            1.0, 1.0, 1.0,
            0.0, 1.0, 1.0,
        ];
        let geo = Arc::new(Geometry::points_only(pts).unwrap());
        // Default params have remove_inline_points = true, merging coplanar triangles into 6 quad faces like Houdini.
        let p_def = Params::resolve(CONVEX_HULL.params, None).unwrap();
        let out_def = convex_hull(&[vec![geo.clone()]], &p_def).unwrap();
        assert_eq!(out_def.nprims(), 6);
        assert_eq!(out_def.npoints, 8);
        assert_eq!(out_def.topology.prim_offsets, vec![0, 4, 8, 12, 16, 20, 24]);

        // With remove_inline_points = false, keeps the 12 triangles.
        let p_tri = Params::resolve(
            CONVEX_HULL.params,
            serde_json::json!({"remove_inline_points": false}).as_object(),
        ).unwrap();
        let out_tri = convex_hull(&[vec![geo]], &p_tri).unwrap();
        assert_eq!(out_tri.nprims(), 12);
        assert_eq!(out_tri.npoints, 8);
    }

    #[test]
    fn test_convex_hull_tetrahedron() {
        let pts = vec![
            0.0, 0.0, 0.0,
            1.0, 0.0, 0.0,
            0.0, 1.0, 0.0,
            0.0, 0.0, 1.0,
            0.2, 0.2, 0.2, // interior point
        ];
        let geo = Arc::new(Geometry::points_only(pts).unwrap());
        let p = Params::resolve(CONVEX_HULL.params, None).unwrap();
        let out = convex_hull(&[vec![geo]], &p).unwrap();
        assert_eq!(out.nprims(), 4);
        assert_eq!(out.npoints, 4);
        assert_eq!(out.topology.prim_offsets, vec![0, 3, 6, 9, 12]);
    }

    #[test]
    fn test_convex_hull_inline_and_coplanar_points_removed() {
        // 3x3x3 grid of points on a cube [0, 2]^3: 27 points total, 8 corners, 12 edge midpoints, 6 face centers, 1 center
        let mut pts = Vec::new();
        for x in 0..3 {
            for y in 0..3 {
                for z in 0..3 {
                    pts.extend_from_slice(&[x as f32, y as f32, z as f32]);
                }
            }
        }
        let geo = Arc::new(Geometry::points_only(pts).unwrap());
        let p = Params::resolve(CONVEX_HULL.params, None).unwrap();
        let out = convex_hull(&[vec![geo.clone()]], &p).unwrap();
        // Inline edge points and coplanar face centers are dropped, leaving only the 8 corners and 6 quad faces
        assert_eq!(out.npoints, 8);
        assert_eq!(out.nprims(), 6);
        assert_eq!(out.topology.prim_offsets, vec![0, 4, 8, 12, 16, 20, 24]);

        // With remove_inline_points = false, edge midpoints and face interior points on the boundary are retained
        let p_keep = Params::resolve(
            CONVEX_HULL.params,
            serde_json::json!({"remove_inline_points": false}).as_object(),
        ).unwrap();
        let out_keep = convex_hull(&[vec![geo]], &p_keep).unwrap();
        assert!(out_keep.npoints > 8);
        assert!(out_keep.nprims() > 6);
    }

    #[test]
    fn test_convex_hull_2d_coplanar() {
        // Square in XY plane with an edge midpoint and an interior point, tilted into 3D for fit_plane as well
        let pts = vec![
            0.0, 0.0, 1.0,
            2.0, 0.0, 1.0,
            2.0, 2.0, 1.0,
            0.0, 2.0, 1.0,
            1.0, 0.0, 1.0, // collinear on bottom edge
            1.0, 1.0, 1.0, // interior point
        ];
        let geo = Arc::new(Geometry::points_only(pts).unwrap());
        for mode in ["fit_plane", "xy"] {
            let p = Params::resolve(
                CONVEX_HULL.params,
                serde_json::json!({"type": "2d", "positions_2d": mode}).as_object(),
            ).unwrap();
            let out = convex_hull(&[vec![geo.clone()]], &p).unwrap();
            assert_eq!(out.nprims(), 1);
            assert_eq!(out.npoints, 4);
            assert_eq!(out.topology.prim_offsets, vec![0, 4]);
        }

        // With remove_inline_points = false in 2D, the collinear edge point (1, 0, 1) is kept (5 vertices)
        let p_keep = Params::resolve(
            CONVEX_HULL.params,
            serde_json::json!({"type": "2d", "positions_2d": "xy", "remove_inline_points": false}).as_object(),
        ).unwrap();
        let out_keep = convex_hull(&[vec![geo]], &p_keep).unwrap();
        assert_eq!(out_keep.nprims(), 1);
        assert_eq!(out_keep.npoints, 5);
    }

    #[test]
    fn test_convex_hull_group_restriction() {
        // Two cubes: points 0..8 at [0,1]^3, points 8..16 at [0,10]^3
        let mut pts = Vec::new();
        for &(x, y, z) in &[
            (0.0, 0.0, 0.0), (1.0, 0.0, 0.0), (1.0, 1.0, 0.0), (0.0, 1.0, 0.0),
            (0.0, 0.0, 1.0), (1.0, 0.0, 1.0), (1.0, 1.0, 1.0), (0.0, 1.0, 1.0),
        ] {
            pts.extend_from_slice(&[x, y, z]);
        }
        for &(x, y, z) in &[
            (0.0, 0.0, 0.0), (10.0, 0.0, 0.0), (10.0, 10.0, 0.0), (0.0, 10.0, 0.0),
            (0.0, 0.0, 10.0), (10.0, 0.0, 10.0), (10.0, 10.0, 10.0), (0.0, 10.0, 10.0),
        ] {
            pts.extend_from_slice(&[x, y, z]);
        }
        let mut geo = Geometry::points_only(pts).unwrap();
        let mut grp = vec![0i32; 16];
        for i in 0..8 {
            grp[i] = 1;
        }
        geo.point.insert("group_small".into(), Arc::new(Attr::i32(1, grp)));
        let geo = Arc::new(geo);

        let p = Params::resolve(
            CONVEX_HULL.params,
            serde_json::json!({"group": "small"}).as_object(),
        ).unwrap();
        let out = convex_hull(&[vec![geo]], &p).unwrap();
        assert_eq!(out.npoints, 8);
        assert_eq!(out.nprims(), 6);
        assert!(out.p().iter().all(|&v| v >= 0.0 && v <= 1.0));
    }

    #[test]
    fn test_convex_hull_attribute_and_group_preservation() {
        let pts = vec![
            0.0, 0.0, 0.0,
            1.0, 0.0, 0.0,
            0.0, 1.0, 0.0,
            0.0, 0.0, 1.0,
            0.2, 0.2, 0.2, // interior point (index 4)
        ];
        let mut geo = Geometry::points_only(pts).unwrap();
        geo.point.insert("id".into(), Arc::new(Attr::i32(1, vec![10, 20, 30, 40, 99])));
        geo.point.insert("group_corners".into(), Arc::new(Attr::i32(1, vec![1, 1, 1, 1, 0])));
        geo.detail.insert("meta".into(), serde_json::json!("kept"));
        let geo = Arc::new(geo);

        // preserve_attributes = true (default)
        let p_on = Params::resolve(CONVEX_HULL.params, None).unwrap();
        let out_on = convex_hull(&[vec![geo.clone()]], &p_on).unwrap();
        assert_eq!(out_on.point.get("id").unwrap().data, AttrData::I32(vec![10, 20, 30, 40]));
        assert_eq!(out_on.point.get("group_corners").unwrap().data, AttrData::I32(vec![1, 1, 1, 1]));
        assert_eq!(out_on.detail.get("meta"), Some(&serde_json::json!("kept")));

        // preserve_attributes = false
        let p_off = Params::resolve(
            CONVEX_HULL.params,
            serde_json::json!({"preserve_attributes": false}).as_object(),
        ).unwrap();
        let out_off = convex_hull(&[vec![geo]], &p_off).unwrap();
        assert!(!out_off.point.contains_key("id"));
        assert!(!out_off.point.contains_key("group_corners"));
        assert!(out_off.detail.is_empty());
    }

    #[test]
    fn test_convex_hull_shrink_stays_inside_original_hull() {
        let mut pts = vec![
            0.0, 0.0, 0.0,
            2.0, 0.0, 0.0,
            2.0, 2.0, 0.0,
            0.0, 2.0, 0.0,
            0.0, 0.0, 2.0,
            2.0, 0.0, 2.0,
            2.0, 2.0, 2.0,
            0.0, 2.0, 2.0,
            // Interior cluster near (0.8, 0.8, 0.8)
            0.8, 0.8, 0.8,
            0.9, 0.7, 0.8,
            0.7, 0.9, 0.8,
        ];
        let geo = Arc::new(Geometry::points_only(std::mem::take(&mut pts)).unwrap());
        let p0 = Params::resolve(CONVEX_HULL.params, None).unwrap();
        let out0 = convex_hull(&[vec![geo.clone()]], &p0).unwrap();

        let p_shrink = Params::resolve(
            CONVEX_HULL.params,
            serde_json::json!({"shrink_amount": 0.5}).as_object(),
        ).unwrap();
        let out_s = convex_hull(&[vec![geo]], &p_shrink).unwrap();
        assert_eq!(out_s.npoints, out0.npoints);
        assert_eq!(out_s.nprims(), out0.nprims());

        // Every shrunk point must lie strictly inside [0, 2]^3 and closer to the interior than the original corners
        for chunk in out_s.p().chunks_exact(3) {
            for &coord in chunk {
                assert!(coord > 0.0 && coord < 2.0, "shrunk coordinate {coord} not strictly inside (0, 2)");
            }
        }
    }
}
