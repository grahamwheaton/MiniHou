//! Iso-lines and cut surfaces from minihou/nodes/contour.py.
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use super::{GEO, InputSpec, Inputs, NodeSpec};
use crate::{
    error::{Error, Result},
    geometry::{Attr, AttrData, Geometry, Topology},
    params::Params,
};

pub const CONTOUR: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "contour",
    inputs: GEO,
    params: crate::params!(
        "mode": "lines";
        "attribute": "P.z";
        "level": 0.0;
        "levels": 1;
        "spacing": 1.0;
        "cells": "polygons";
        "keep_above": true;
        "fill": false;
        "group": "";
        "outgrp_cutpoints": false;
        "outgrp_cutpoints_name": "contourpoints"
    ),
    cook: contour,
};

pub const SLICE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "slice",
    inputs: GEO,
    params: crate::params!(
        "origin": [0.0, 0.0, 0.0];
        "normal": [0.0, 0.0, 1.0];
        "spacing": 0.25;
        "count": 1;
        "cover": false;
        "fill": false;
        "group": "";
    ),
    cook: slice,
};

fn extract_values(geo: &Geometry, attr_name: &str) -> Result<Vec<f64>> {
    let (name, comp) = if let Some((n, c)) = attr_name.split_once('.') {
        (n, Some(c))
    } else {
        (attr_name, None)
    };

    let p_slice = geo.p();
    let n = geo.npoints;
    if name == "P" {
        if let Some(c) = comp {
            let k = match c {
                "x" | "0" => 0,
                "y" | "1" => 1,
                "z" | "2" => 2,
                _ => return Err(Error::new(format!("contour: component {c:?} should be x, y or z"))),
            };
            return Ok((0..n).map(|i| p_slice[i * 3 + k] as f64).collect());
        }
        return Ok((0..n).map(|i| {
            (0..3).map(|k| (p_slice[i * 3 + k] as f64).powi(2)).sum::<f64>().sqrt()
        }).collect());
    }

    let attr = geo.point.get(name)
        .ok_or_else(|| Error::new(format!("contour: no point attribute {name:?}")))?;

    let src = attr.as_f32().ok_or_else(|| Error::new("contour: attribute must be float"))?;
    let w = attr.width;
    if w == 1 && comp.is_none() {
        return Ok(src.iter().map(|&x| x as f64).collect());
    }
    if let Some(c) = comp {
        let k = match c {
            "x" | "0" => 0,
            "y" | "1" => 1,
            "z" | "2" => 2,
            _ => return Err(Error::new(format!("contour: component {c:?} should be x, y or z"))),
        };
        if k >= w {
            return Err(Error::new("contour: component index out of range"));
        }
        return Ok((0..n).map(|i| src[i * w + k] as f64).collect());
    }
    Ok((0..n).map(|i| {
        (0..w).map(|k| (src[i * w + k] as f64).powi(2)).sum::<f64>().sqrt()
    }).collect())
}

struct Cells {
    vp: Vec<i32>,
    offsets: Vec<i32>,
    prim_owner: Vec<usize>,
    vert_owner: Vec<usize>,
}

fn prepare_cells(geo: &Geometry, cells: &str, group_mask: &[bool]) -> Cells {
    let t = &geo.topology;
    let limit = if cells == "triangles" { 3 } else { 4 };

    let mut vp = Vec::new();
    let mut offsets = vec![0i32];
    let mut prim_owner = Vec::new();
    let mut vert_owner = Vec::new();

    for p in 0..geo.nprims() {
        if !group_mask[p] {
            continue;
        }
        let start = t.prim_offsets[p] as usize;
        let end = t.prim_offsets[p + 1] as usize;
        let sz = end - start;
        if sz < 3 {
            continue;
        }
        if sz <= limit {
            for v in start..end {
                vp.push(t.vertex_point[v]);
                vert_owner.push(v);
            }
            offsets.push(vp.len() as i32);
            prim_owner.push(p);
        } else {
            // Fan triangulate
            for k in 1..sz - 1 {
                let v0 = start;
                let v1 = start + k;
                let v2 = start + k + 1;
                vp.push(t.vertex_point[v0]);
                vp.push(t.vertex_point[v1]);
                vp.push(t.vertex_point[v2]);
                vert_owner.push(v0);
                vert_owner.push(v1);
                vert_owner.push(v2);
                offsets.push(vp.len() as i32);
                prim_owner.push(p);
            }
        }
    }

    Cells { vp, offsets, prim_owner, vert_owner }
}

fn contour(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = CONTOUR.one(inputs, 0)?;
    let mode = p.e("mode", &["lines", "surface"])?;
    let attribute = p.s("attribute")?;
    let level = p.f("level")?;
    let levels_count = p.i("levels")?;
    if levels_count < 1 {
        return Err(Error::new("levels must be at least 1"));
    }
    let spacing = p.f("spacing")?;
    if spacing <= 0.0 {
        return Err(Error::new("spacing must be positive"));
    }
    let cells_mode = p.e("cells", &["polygons", "triangles"])?;
    let keep_above = p.b("keep_above")?;
    let fill = p.b("fill")?;

    let group_mask = super::group::mask(&geo, "prim", &p.s("group")?)?;
    let values = extract_values(&geo, &attribute)?;

    if mode == "lines" {
        let levels: Vec<f64> = (0..levels_count).map(|i| level + i as f64 * spacing).collect();
        iso_lines(&geo, &values, &levels, &cells_mode, fill, &group_mask)
    } else {
        let (out, new_pts) = clip_surface(&geo, &values, level, keep_above, &cells_mode, &group_mask)?;
        if p.b("outgrp_cutpoints")? {
            let grp_name = p.s("outgrp_cutpoints_name")?;
            if !grp_name.is_empty() {
                let mut out = out;
                let vals: Vec<i32> = new_pts.into_iter().map(|b| if b { 1 } else { 0 }).collect();
                out.point.insert(grp_name, Arc::new(Attr::i32(1, vals)));
                return Ok(out);
            }
        }
        Ok(out)
    }
}

fn iso_lines(
    geo: &Geometry,
    values: &[f64],
    levels: &[f64],
    cells_mode: &str,
    fill: bool,
    group_mask: &[bool],
) -> Result<Geometry> {
    let cells = prepare_cells(geo, cells_mode, group_mask);
    let nprims = cells.offsets.len().saturating_sub(1);
    if nprims == 0 {
        return Geometry::from_arrays(vec![], vec![], vec![0], None);
    }

    struct Segment {
        lv_idx: usize,
        a0: usize,
        a1: usize,
        b0: usize,
        b1: usize,
    }

    let mut segments = Vec::new();
    for (lv_idx, &lvl) in levels.iter().enumerate() {
        for p in 0..nprims {
            let start = cells.offsets[p] as usize;
            let end = cells.offsets[p + 1] as usize;
            let sz = end - start;
            if sz != 3 && sz != 4 {
                continue;
            }

            let c: Vec<usize> = cells.vp[start..end].iter().map(|&x| x as usize).collect();
            let inside: Vec<bool> = c.iter().map(|&pt| values[pt] >= lvl).collect();

            let mut crosses = Vec::new();
            for i in 0..sz {
                let next_i = (i + 1) % sz;
                if inside[i] != inside[next_i] {
                    crosses.push(i);
                }
            }

            if crosses.len() == 2 {
                let e0 = crosses[0];
                let e1 = crosses[1];
                segments.push(Segment {
                    lv_idx,
                    a0: c[e0],
                    a1: c[(e0 + 1) % sz],
                    b0: c[e1],
                    b1: c[(e1 + 1) % sz],
                });
            } else if sz == 4 && crosses.len() == 4 {
                let centre_val = c.iter().map(|&pt| values[pt]).sum::<f64>() / 4.0;
                let centre_inside = centre_val >= lvl;
                let same = centre_inside == inside[0];
                let pairs: [((usize, usize), (usize, usize)); 2] = if same {
                    [((0, 1), (0, 1)), ((2, 3), (2, 3))]
                } else {
                    [((1, 2), (1, 2)), ((3, 0), (3, 0))]
                };
                for ((e0, _), (e1, _)) in pairs {
                    let next_e0 = (e0 + 1) % 4;
                    let next_e1 = (e1 + 1) % 4;
                    segments.push(Segment {
                        lv_idx,
                        a0: c[e0],
                        a1: c[next_e0],
                        b0: c[e1],
                        b1: c[next_e1],
                    });
                }
            }
        }
    }

    if segments.is_empty() {
        return Geometry::from_arrays(vec![], vec![], vec![0], None);
    }

    // Deduplicate endpoints on mesh edges
    // Key: (lv_idx, min(v0, v1), max(v0, v1))
    let mut cut_map: HashMap<(usize, usize, usize), usize> = HashMap::new();
    let mut cut_pts: Vec<([f64; 3], f32, usize, usize, f64)> = Vec::new(); // pos, level_val, ia, ib, t
    let p_slice = geo.p();

    let mut get_cut_vertex = |lv_idx: usize, v0: usize, v1: usize| -> usize {
        let key = (lv_idx, v0.min(v1), v0.max(v1));
        if let Some(&idx) = cut_map.get(&key) {
            return idx;
        }
        let (ia, ib) = (key.1, key.2);
        let va = values[ia];
        let vb = values[ib];
        let lvl = levels[lv_idx];
        let tt = if (vb - va).abs() > 1e-30 {
            ((lvl - va) / (vb - va)).clamp(0.0, 1.0)
        } else {
            0.5
        };
        let pa = [p_slice[ia * 3] as f64, p_slice[ia * 3 + 1] as f64, p_slice[ia * 3 + 2] as f64];
        let pb = [p_slice[ib * 3] as f64, p_slice[ib * 3 + 1] as f64, p_slice[ib * 3 + 2] as f64];
        let pos = [
            pa[0] + (pb[0] - pa[0]) * tt,
            pa[1] + (pb[1] - pa[1]) * tt,
            pa[2] + (pb[2] - pa[2]) * tt,
        ];
        let idx = cut_pts.len();
        cut_pts.push((pos, lvl as f32, ia, ib, tt));
        cut_map.insert(key, idx);
        idx
    };

    let mut seg_edges = Vec::with_capacity(segments.len());
    for s in &segments {
        let u = get_cut_vertex(s.lv_idx, s.a0, s.a1) as i32;
        let v = get_cut_vertex(s.lv_idx, s.b0, s.b1) as i32;
        seg_edges.push([u, v]);
    }

    let n_cuts = cut_pts.len();
    let (paths, cl) = super::curves::chains(n_cuts, &seg_edges);

    let mut out_pos = Vec::new();
    let mut out_vp = Vec::new();
    let mut out_offsets = vec![0i32];
    let mut out_level = Vec::new();
    let mut out_is_closed = Vec::new();
    let mut out_prim_level = Vec::new();
    let mut old_point_indices = Vec::new();

    for (path, is_loop) in paths.into_iter().zip(cl) {
        let mut p_verts = path;
        let closed_flag = is_loop && fill;
        if is_loop && !fill {
            if let Some(&first_v) = p_verts.first() {
                p_verts.push(first_v);
            }
        }
        let prim_lvl = cut_pts[p_verts[0] as usize].1;
        out_prim_level.push(prim_lvl);
        out_is_closed.push(closed_flag);

        for &v in &p_verts {
            let cut = &cut_pts[v as usize];
            out_pos.push(cut.0[0] as f32);
            out_pos.push(cut.0[1] as f32);
            out_pos.push(cut.0[2] as f32);
            out_level.push(cut.1);
            old_point_indices.push((cut.2, cut.3, cut.4 as f32));
            out_vp.push(out_vp.len() as i32);
        }
        out_offsets.push(out_vp.len() as i32);
    }

    let total_pts = out_pos.len() / 3;
    let mut out = Geometry::from_arrays(out_pos, out_vp, out_offsets, Some(out_is_closed))?;

    // Interpolate point attributes
    for (name, attr) in &geo.point {
        if name == "P" || name == "N" {
            continue;
        }
        let w = attr.width;
        let mut dst_attr = attr.zeros_like(total_pts);
        match (&attr.data, &mut dst_attr.data) {
            (AttrData::F32(src), AttrData::F32(dst)) => {
                for (i, &(ia, ib, t)) in old_point_indices.iter().enumerate() {
                    for k in 0..w {
                        dst[i * w + k] = src[ia * w + k] * (1.0 - t) + src[ib * w + k] * t;
                    }
                }
            }
            (AttrData::I32(src), AttrData::I32(dst)) => {
                for (i, &(ia, _, _)) in old_point_indices.iter().enumerate() {
                    dst[i * w..(i + 1) * w].copy_from_slice(&src[ia * w..(ia + 1) * w]);
                }
            }
            _ => unreachable!(),
        }
        out.point.insert(name.clone(), Arc::new(dst_attr));
    }

    out.point.insert("level".into(), Arc::new(Attr::f32(1, out_level)));
    out.prim.insert("level".into(), Arc::new(Attr::f32(1, out_prim_level)));
    out.detail = geo.detail.clone();

    Ok(out)
}

fn clip_surface(
    geo: &Geometry,
    values: &[f64],
    level: f64,
    keep_above: bool,
    cells_mode: &str,
    group_mask: &[bool],
) -> Result<(Geometry, Vec<bool>)> {
    let cells = prepare_cells(geo, cells_mode, group_mask);
    let nprims = cells.offsets.len().saturating_sub(1);
    if nprims == 0 {
        return Ok((Geometry::from_arrays(vec![], vec![], vec![0], None)?, Vec::new()));
    }

    let mut s: Vec<f64> = values.iter().map(|&v| v - level).collect();
    if !keep_above {
        for v in &mut s {
            *v = -*v;
        }
    }

    let p_slice = geo.p();
    let n = geo.npoints;

    // Edge cut map
    let mut cut_map: HashMap<(usize, usize), usize> = HashMap::new();
    let mut cut_pts: Vec<([f64; 3], usize, usize, f64)> = Vec::new();

    let mut get_cut = |a: usize, b: usize| -> usize {
        let key = (a.min(b), a.max(b));
        if let Some(&idx) = cut_map.get(&key) {
            return idx;
        }
        let sa = s[key.0];
        let sb = s[key.1];
        let t = if (sb - sa).abs() > 1e-30 {
            (-sa / (sb - sa)).clamp(0.0, 1.0)
        } else {
            0.5
        };
        let pa = [p_slice[key.0 * 3] as f64, p_slice[key.0 * 3 + 1] as f64, p_slice[key.0 * 3 + 2] as f64];
        let pb = [p_slice[key.1 * 3] as f64, p_slice[key.1 * 3 + 1] as f64, p_slice[key.1 * 3 + 2] as f64];
        let pos = [
            pa[0] + (pb[0] - pa[0]) * t,
            pa[1] + (pb[1] - pa[1]) * t,
            pa[2] + (pb[2] - pa[2]) * t,
        ];
        let idx = cut_pts.len();
        cut_pts.push((pos, key.0, key.1, t));
        cut_map.insert(key, idx);
        idx
    };

    let mut out_faces = Vec::new();
    let mut prim_src = Vec::new();

    for p in 0..nprims {
        let start = cells.offsets[p] as usize;
        let end = cells.offsets[p + 1] as usize;
        let sz = end - start;
        let corners: Vec<usize> = cells.vp[start..end].iter().map(|&x| x as usize).collect();

        let mut poly = Vec::new();
        for i in 0..sz {
            let next_i = (i + 1) % sz;
            let a = corners[i];
            let b = corners[next_i];
            let in_a = s[a] >= 0.0;
            let in_b = s[b] >= 0.0;

            if in_a && in_b {
                poly.push(b);
            } else if in_a && !in_b {
                let cut_idx = get_cut(a, b);
                poly.push(n + cut_idx);
            } else if !in_a && in_b {
                let cut_idx = get_cut(a, b);
                poly.push(n + cut_idx);
                poly.push(b);
            }
        }

        if poly.len() >= 3 {
            out_faces.push(poly);
            prim_src.push(cells.prim_owner[p]);
        }
    }

    if out_faces.is_empty() {
        return Ok((Geometry::from_arrays(vec![], vec![], vec![0], None)?, Vec::new()));
    }

    // Combine original and cut positions
    let mut all_pos = Vec::with_capacity((n + cut_pts.len()) * 3);
    all_pos.extend_from_slice(p_slice);
    for cut in &cut_pts {
        all_pos.push(cut.0[0] as f32);
        all_pos.push(cut.0[1] as f32);
        all_pos.push(cut.0[2] as f32);
    }

    let mut flat_vp = Vec::new();
    let mut offsets = vec![0i32];
    for face in &out_faces {
        flat_vp.extend(face.iter().map(|&v| v as i32));
        offsets.push(flat_vp.len() as i32);
    }

    let mut temp = Geometry::from_arrays(all_pos, flat_vp, offsets, None)?;

    // Interpolate point attributes for cut points
    for (name, attr) in &geo.point {
        if name == "P" || name == "N" {
            continue;
        }
        let w = attr.width;
        let mut new_attr = attr.zeros_like(n + cut_pts.len());
        match (&attr.data, &mut new_attr.data) {
            (AttrData::F32(src), AttrData::F32(dst)) => {
                dst[..n * w].copy_from_slice(src);
                for (i, cut) in cut_pts.iter().enumerate() {
                    let dst_idx = (n + i) * w;
                    let t = cut.3 as f32;
                    for k in 0..w {
                        dst[dst_idx + k] = src[cut.1 * w + k] * (1.0 - t) + src[cut.2 * w + k] * t;
                    }
                }
            }
            (AttrData::I32(src), AttrData::I32(dst)) => {
                dst[..n * w].copy_from_slice(src);
                for (i, cut) in cut_pts.iter().enumerate() {
                    let dst_idx = (n + i) * w;
                    dst[dst_idx..dst_idx + w].copy_from_slice(&src[cut.1 * w..cut.1 * w + w]);
                }
            }
            _ => unreachable!(),
        }
        temp.point.insert(name.clone(), Arc::new(new_attr));
    }

    // Gather prim attributes
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
        temp.prim.insert(name.clone(), Arc::new(gathered));
    }

    temp.detail = geo.detail.clone();

    // Compact unused points
    let mut used = vec![false; temp.npoints];
    for &v in &temp.topology.vertex_point {
        used[v as usize] = true;
    }
    let mut remap = vec![-1i32; temp.npoints];
    let mut new_pts = Vec::new();
    let mut is_cut_point = Vec::new();
    let mut new_count = 0usize;
    let old_p = temp.p();

    for i in 0..temp.npoints {
        if used[i] {
            remap[i] = new_count as i32;
            new_count += 1;
            new_pts.extend_from_slice(&old_p[i * 3..i * 3 + 3]);
            is_cut_point.push(i >= n);
        }
    }

    let compacted_vp: Vec<i32> = temp.topology.vertex_point.iter().map(|&v| remap[v as usize]).collect();
    let mut out = Geometry::from_arrays(new_pts, compacted_vp, temp.topology.prim_offsets.clone(), None)?;

    for (name, attr) in &temp.point {
        let w = attr.width;
        let mut compacted_attr = attr.zeros_like(new_count);
        match (&attr.data, &mut compacted_attr.data) {
            (AttrData::F32(src), AttrData::F32(dst)) => {
                for i in 0..temp.npoints {
                    if remap[i] >= 0 {
                        let target = remap[i] as usize;
                        dst[target * w..(target + 1) * w].copy_from_slice(&src[i * w..(i + 1) * w]);
                    }
                }
            }
            (AttrData::I32(src), AttrData::I32(dst)) => {
                for i in 0..temp.npoints {
                    if remap[i] >= 0 {
                        let target = remap[i] as usize;
                        dst[target * w..(target + 1) * w].copy_from_slice(&src[i * w..(i + 1) * w]);
                    }
                }
            }
            _ => unreachable!(),
        }
        out.point.insert(name.clone(), Arc::new(compacted_attr));
    }

    out.prim = temp.prim.clone();
    out.detail = temp.detail.clone();

    Ok((out, is_cut_point))
}

fn slice(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = SLICE.one(inputs, 0)?;
    let origin = p.v3("origin")?;
    let normal = p.v3("normal")?;
    let n_len = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
    if n_len < 1e-12 {
        return Err(Error::new("slice: the normal is zero"));
    }
    let nrm = [normal[0] / n_len, normal[1] / n_len, normal[2] / n_len];

    let p_slice = geo.p();
    let n = geo.npoints;
    let d: Vec<f64> = (0..n).map(|i| {
        let px = p_slice[i * 3] as f64 - origin[0];
        let py = p_slice[i * 3 + 1] as f64 - origin[1];
        let pz = p_slice[i * 3 + 2] as f64 - origin[2];
        px * nrm[0] + py * nrm[1] + pz * nrm[2]
    }).collect();

    let spacing = p.f("spacing")?;
    if spacing <= 0.0 {
        return Err(Error::new("spacing must be positive"));
    }
    let cover = p.b("cover")?;
    let count = p.i("count")?;
    if count < 1 && !cover {
        return Err(Error::new("count must be at least 1"));
    }

    let steps: Vec<f64> = if cover {
        if n == 0 {
            return Ok((*geo).clone());
        }
        let mut d_min = f64::INFINITY;
        let mut d_max = f64::NEG_INFINITY;
        for &val in &d {
            d_min = d_min.min(val);
            d_max = d_max.max(val);
        }
        let lo = (d_min / spacing).floor() as i64;
        let hi = (d_max / spacing).ceil() as i64;
        let mut s = Vec::new();
        for k in lo..=hi {
            let step = k as f64 * spacing;
            if step >= d_min - 1e-9 && step <= d_max + 1e-9 {
                s.push(step);
            }
        }
        s
    } else {
        (0..count).map(|i| i as f64 * spacing).collect()
    };

    if steps.len() > 100_000 {
        return Err(Error::new("slice planes would exceed limit; use a larger spacing"));
    }

    let levels: Vec<f64> = steps.iter().map(|&s| s + 1e-9 * spacing).collect();
    let fill = p.b("fill")?;
    let group = p.s("group")?;
    let group_mask = super::group::mask(&geo, "prim", &group)?;

    iso_lines(&geo, &d, &levels, "polygons", fill, &group_mask)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_contour_lines_on_quad() {
        let pts = vec![
            0.0, 0.0, 0.0,
            1.0, 0.0, 1.0,
            1.0, 1.0, 1.0,
            0.0, 1.0, 0.0,
        ];
        let vp = vec![0, 1, 2, 3];
        let offs = vec![0, 4];
        let geo = Geometry::from_arrays(pts, vp, offs, None).unwrap();
        let p = Params::resolve(CONTOUR.params, serde_json::json!({"level": 0.5}).as_object()).unwrap();
        let out = contour(&[vec![Arc::new(geo)]], &p).unwrap();
        assert_eq!(out.nprims(), 1);
        assert_eq!(out.npoints, 2);
    }

    #[test]
    fn test_contour_surface_clip() {
        let pts = vec![
            0.0, 0.0, 0.0,
            1.0, 0.0, 1.0,
            1.0, 1.0, 1.0,
            0.0, 1.0, 0.0,
        ];
        let vp = vec![0, 1, 2, 3];
        let offs = vec![0, 4];
        let geo = Geometry::from_arrays(pts, vp, offs, None).unwrap();
        let p = Params::resolve(CONTOUR.params, serde_json::json!({"mode": "surface", "level": 0.5}).as_object()).unwrap();
        let out = contour(&[vec![Arc::new(geo)]], &p).unwrap();
        assert_eq!(out.nprims(), 1);
        assert_eq!(out.npoints, 4);
    }

    #[test]
    fn test_slice_planes_on_cube() {
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
        // 6 quad faces
        let vp = vec![
            0, 1, 2, 3,
            4, 5, 6, 7,
            0, 1, 5, 4,
            2, 3, 7, 6,
            0, 3, 7, 4,
            1, 2, 6, 5,
        ];
        let offs = vec![0, 4, 8, 12, 16, 20, 24];
        let geo = Geometry::from_arrays(pts, vp, offs, None).unwrap();
        let p = Params::resolve(SLICE.params, serde_json::json!({
            "origin": [0.0, 0.0, 0.5],
            "normal": [0.0, 0.0, 1.0],
            "count": 1,
            "fill": true
        }).as_object()).unwrap();
        let out = slice(&[vec![Arc::new(geo)]], &p).unwrap();
        assert_eq!(out.nprims(), 1);
        assert_eq!(out.npoints, 4);
        for i in 0..4 {
            assert!((out.p()[i * 3 + 2] - 0.5).abs() < 1e-4);
        }
    }
}
