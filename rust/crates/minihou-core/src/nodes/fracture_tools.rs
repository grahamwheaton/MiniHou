//! Fracture tools: assemble and face-contact adjacency (minihou/nodes/fracture_tools.py).
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use super::{InputSpec, Inputs, NodeSpec, GEO};
use crate::error::{Error, Result};
use crate::geometry::{Attr, AttrData, Geometry};
use crate::params::Params;
use super::model::{add, sub, mul, dot, len, positions, range, polygons, outgroup};
use super::np_random::Rng;
use super::sim_common::{float, int};

mod adjacency;

pub(super) use adjacency::{extent, pairs, contacts};
use adjacency::voronoi_adjacency;

pub const VORONOI_ADJACENCY: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "voronoi_adjacency",
    inputs: &[
        InputSpec { name: "geo", multi: false, optional: false },
        InputSpec { name: "points", multi: false, optional: true },
    ],
    params: crate::params!("piece_attrib": "piece"; "max_gap": 0.0; "min_area": 0.0; "strong_area": 0.5;
        "outgrp_connected": false; "outgrp_connected_name": "connected";
        "outgrp_isolated": false; "outgrp_isolated_name": "isolated";
        "outgrp_strong": false; "outgrp_strong_name": "stronglinks"),
    cook: voronoi_adjacency,
};

pub const ASSEMBLE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "assemble",
    inputs: GEO,
    params: crate::params!(
        "source": "attribute";
        "piece_attrib": "piece";
        "groups": "*";
        "output_attrib": "piece";
        "renumber": true;
        "unshare_points": true;
        "pivot": "centroid";
        "name_prefix": "piece";
        "piece_groups": false;
        "remove_source_groups": false;
        "outgrp_unshared": false;
        "outgrp_unshared_name": "splitpoints";
        "outgrp_shared": false;
        "outgrp_shared_name": "sharedpoints"
    ),
    cook: assemble,
};

fn assemble(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = ASSEMBLE.one(inputs, 0)?;

    if geo.topology.has_open() {
        let n = geo.topology.closed_mask().iter().filter(|&&c| !c).count();
        return Err(Error::new(format!(
            "assemble works on polygons, but the input has {n} open curve(s); remove them first, e.g. blast with group '@closed == 0'"
        )));
    }
    if geo.nprims() == 0 {
        return Ok((*geo).clone());
    }

    let source = p.e("source", &["attribute", "connectivity", "groups"])?;
    let piece_attrib = p.s("piece_attrib")?;
    let groups_pat = p.s("groups")?;
    let output_attrib = p.s("output_attrib")?;
    let renumber = p.b("renumber")?;
    let unshare_points = p.b("unshare_points")?;
    let pivot_mode = p.e("pivot", &["centroid", "origin", "bounds_bottom"])?;
    let name_prefix = p.s("name_prefix")?;
    let piece_groups = p.b("piece_groups")?;
    let remove_source_groups = p.b("remove_source_groups")?;

    let mut used_groups = Vec::new();
    let mut ids: Vec<i64> = match source.as_str() {
        "attribute" => {
            if piece_attrib.is_empty() {
                return Err(Error::new(
                    "no primitive attribute \"\": fracture first, or leave piece attribute empty to use connectivity",
                ));
            }
            let attr = geo.prim.get(&piece_attrib).ok_or_else(|| {
                Error::new(format!(
                    "no primitive attribute {piece_attrib:?}: fracture first, or leave piece attribute empty to use connectivity"
                ))
            })?;
            if attr.width != 1 {
                return Err(Error::new(format!(
                    "{piece_attrib:?} must be a scalar integer primitive attribute"
                )));
            }
            match &attr.data {
                AttrData::I32(v) => v.iter().map(|&x| x as i64).collect(),
                _ => {
                    return Err(Error::new(format!(
                        "{piece_attrib:?} must be a scalar integer primitive attribute"
                    )));
                }
            }
        }
        "connectivity" => {
            let (_, prim_labels) = super::attrib_tools::piece_labels(&geo);
            prim_labels.into_iter().map(|x| x as i64).collect()
        }
        "groups" => {
            let pat_str = if groups_pat.is_empty() {
                "*"
            } else {
                groups_pat.as_str()
            };
            let mut available_groups = Vec::new();
            for k in geo.prim.keys() {
                if let Some(stripped) = k.strip_prefix("group_") {
                    available_groups.push(stripped.to_string());
                }
            }
            available_groups.sort();

            let mut matched_names = Vec::new();
            for pat in pat_str.split_whitespace() {
                for gname in &available_groups {
                    if super::groups::point_mask(
                        &Geometry::points_only(vec![0.0, 0.0, 0.0]).unwrap(),
                        pat,
                    )
                    .is_ok()
                    {
                        // Match wildcard pattern against group name
                        let is_match = match_pattern(pat.as_bytes(), gname.as_bytes());
                        if is_match && !matched_names.contains(gname) {
                            matched_names.push(gname.clone());
                        }
                    }
                }
            }

            if matched_names.is_empty() {
                return Err(Error::new(format!(
                    "no primitive group matches {groups_pat:?}; have {available_groups:?}"
                )));
            }

            let mut group_ids = vec![-1i64; geo.nprims()];
            for (k, name) in matched_names.iter().enumerate() {
                if let Some(attr) = geo.prim.get(&format!("group_{name}")) {
                    let m = super::group::flags(attr);
                    for i in 0..geo.nprims() {
                        if m[i] && group_ids[i] < 0 {
                            group_ids[i] = k as i64;
                        }
                    }
                }
            }
            for i in 0..geo.nprims() {
                if group_ids[i] < 0 {
                    group_ids[i] = matched_names.len() as i64;
                }
            }
            used_groups = matched_names;
            group_ids
        }
        _ => unreachable!(),
    };

    if renumber {
        let mut map = HashMap::new();
        let mut next_id = 0i64;
        let mut new_ids = Vec::with_capacity(ids.len());
        for &id in &ids {
            let assigned = *map.entry(id).or_insert_with(|| {
                let nid = next_id;
                next_id += 1;
                nid
            });
            new_ids.push(assigned);
        }
        ids = new_ids;
    }

    let count = if ids.is_empty() {
        0
    } else {
        (*ids.iter().max().unwrap() + 1) as usize
    };

    let mut out = (*geo).clone();
    let mut shared_mask_opt = None;
    let mut copied_mask_opt = None;

    if unshare_points {
        let nvert = geo.nvertices();
        let mut pv = Vec::with_capacity(nvert);
        for i in 0..geo.nprims() {
            let start = geo.topology.prim_offsets[i] as usize;
            let end = geo.topology.prim_offsets[i + 1] as usize;
            pv.extend(std::iter::repeat(i).take(end - start));
        }

        let stride = (count + 1) as i64;
        let mut keys = Vec::with_capacity(nvert + geo.npoints);
        let mut used_points = vec![false; geo.npoints];

        for v in 0..nvert {
            let pt = geo.topology.vertex_point[v] as usize;
            used_points[pt] = true;
            let piece = ids[pv[v]];
            keys.push(pt as i64 * stride + piece);
        }

        for pt in 0..geo.npoints {
            if !used_points[pt] {
                keys.push(pt as i64 * stride + count as i64);
            }
        }

        let mut uniq = keys.clone();
        uniq.sort_unstable();
        uniq.dedup();

        let inv: Vec<i32> = keys
            .iter()
            .map(|k| uniq.binary_search(k).unwrap() as i32)
            .collect();
        let src: Vec<usize> = uniq.iter().map(|&k| (k / stride) as usize).collect();

        let mut copies = vec![0usize; geo.npoints];
        for &s in &src {
            copies[s] += 1;
        }

        let mut shared_mask = vec![false; src.len()];
        let mut copied_mask = vec![false; src.len()];
        for i in 0..src.len() {
            shared_mask[i] = copies[src[i]] > 1;
            copied_mask[i] = i > 0 && src[i] == src[i - 1];
        }

        let p_src = geo.p();
        let mut p_out = Vec::with_capacity(src.len() * 3);
        for &s in &src {
            p_out.extend_from_slice(&p_src[s * 3..s * 3 + 3]);
        }

        let mut new_point = HashMap::new();
        for (name, attr) in &geo.point {
            if name != "P" {
                new_point.insert(name.clone(), Arc::new(super::group::gather(attr, &src)));
            }
        }

        out = Geometry::from_arrays(
            p_out,
            inv[..nvert].to_vec(),
            geo.topology.prim_offsets.clone(),
            geo.topology.closed.clone(),
        )?;
        out.point.extend(new_point);
        out.vertex = geo.vertex.clone();
        out.prim = geo.prim.clone();
        out.detail = geo.detail.clone();

        shared_mask_opt = Some(shared_mask);
        copied_mask_opt = Some(copied_mask);
    }

    if !output_attrib.is_empty() {
        let ids32: Vec<i32> = ids.iter().map(|&x| x as i32).collect();
        out.prim
            .insert(output_attrib, Arc::new(Attr::i32(1, ids32)));
    }

    let mut pv2 = Vec::with_capacity(out.nvertices());
    for i in 0..out.nprims() {
        let start = out.topology.prim_offsets[i] as usize;
        let end = out.topology.prim_offsets[i + 1] as usize;
        pv2.extend(std::iter::repeat(i).take(end - start));
    }

    let p_out = out.p();
    let vp2 = &out.topology.vertex_point;

    let mut vert_counts = vec![0usize; count];
    let mut sum_p = vec![[0.0f64; 3]; count];
    let mut min_z = vec![f64::INFINITY; count];

    for v in 0..out.nvertices() {
        let piece = ids[pv2[v]] as usize;
        let pt = vp2[v] as usize;
        vert_counts[piece] += 1;
        sum_p[piece][0] += p_out[pt * 3] as f64;
        sum_p[piece][1] += p_out[pt * 3 + 1] as f64;
        sum_p[piece][2] += p_out[pt * 3 + 2] as f64;
        min_z[piece] = min_z[piece].min(p_out[pt * 3 + 2] as f64);
    }

    let mut pivot_arr = vec![[0.0f32; 3]; count];
    if pivot_mode != "origin" {
        for k in 0..count {
            let cnt = vert_counts[k].max(1) as f64;
            pivot_arr[k][0] = (sum_p[k][0] / cnt) as f32;
            pivot_arr[k][1] = (sum_p[k][1] / cnt) as f32;
            pivot_arr[k][2] = if pivot_mode == "bounds_bottom" {
                if min_z[k].is_finite() {
                    min_z[k] as f32
                } else {
                    0.0
                }
            } else {
                (sum_p[k][2] / cnt) as f32
            };
        }
    }

    let mut prim_pivots = Vec::with_capacity(out.nprims() * 3);
    for i in 0..out.nprims() {
        let piece = ids[i] as usize;
        prim_pivots.extend_from_slice(&pivot_arr[piece]);
    }
    out.prim
        .insert("pivot".into(), Arc::new(Attr::f32(3, prim_pivots)));

    let piece_names: Vec<serde_json::Value> = (0..count)
        .map(|k| serde_json::json!(format!("{name_prefix}{k}")))
        .collect();
    out.detail
        .insert("piece_names".into(), serde_json::Value::Array(piece_names));

    if piece_groups {
        if count > 1000 {
            return Err(Error::new("piece_groups is limited to 1000 pieces"));
        }
        for k in 0..count {
            let mask: Vec<bool> = ids.iter().map(|&id| id == k as i64).collect();
            super::model::write_group(&mut out, "prim", &format!("{name_prefix}{k}"), &mask)?;
        }
    }

    if remove_source_groups {
        for name in used_groups {
            out.prim.remove(&format!("group_{name}"));
        }
    }

    if let (Some(shared), Some(copied)) = (shared_mask_opt, copied_mask_opt) {
        super::model::outgroup(&mut out, p, "unshared", "point", &copied)?;
        super::model::outgroup(&mut out, p, "shared", "point", &shared)?;
    }

    Ok(out)
}

fn match_pattern(pattern: &[u8], name: &[u8]) -> bool {
    match pattern {
        [] => name.is_empty(),
        [b'*', rest @ ..] => (0..=name.len()).any(|n| match_pattern(rest, &name[n..])),
        [b'?', rest @ ..] => !name.is_empty() && match_pattern(rest, &name[1..]),
        [c, rest @ ..] => name.first() == Some(c) && match_pattern(rest, &name[1..]),
    }
}

pub const GLUE_CLUSTER: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "glue_cluster",
    inputs: GEO,
    params: crate::params!(
        "piece_attrib": "piece";
        "output": "pieces";
        "strength": 4.0e6;
        "strength_variation": 0.2;
        "seed": 0;
        "max_gap": 0.0;
        "min_area": 0.0;
        "min_share": 0.0;
        "clusters": 0;
        "weak": 0.5;
        "outgrp_glued": false;
        "outgrp_glued_name": "glued";
        "outgrp_loose": false;
        "outgrp_loose_name": "loose";
        "outgrp_interface": false;
        "outgrp_interface_name": "interface";
        "outgrp_internal": false;
        "outgrp_internal_name": "internal";
        "outgrp_external": false;
        "outgrp_external_name": "external";
        "outgrp_weak": false;
        "outgrp_weak_name": "weaklinks"
    ),
    cook: glue_cluster,
};

fn cluster_pieces(ids: &[i32], a: &[i32], b: &[i32], area: &[f64], target: i64) -> Vec<i32> {
    let n = ids.len();
    if n == 0 {
        return Vec::new();
    }
    let mut parent: Vec<usize> = (0..n).collect();
    let mut size: Vec<usize> = vec![1; n];
    let mut clusters = n;

    fn find(parent: &mut [usize], mut x: usize) -> usize {
        while parent[x] != x {
            parent[x] = parent[parent[x]];
            x = parent[x];
        }
        x
    }

    if target > 0 && target < n as i64 {
        let target = target as usize;
        let cap = (1.5 * n as f64 / target as f64).ceil() as usize;
        let mut order: Vec<usize> = (0..area.len()).collect();
        order.sort_by(|&i, &j| (-area[i]).total_cmp(&-area[j]));

        for &limit in &[cap, n] {
            for &k in &order {
                if clusters <= target {
                    break;
                }
                let slot_a = ids.binary_search(&a[k]).unwrap();
                let slot_b = ids.binary_search(&b[k]).unwrap();
                let ra = find(&mut parent, slot_a);
                let rb = find(&mut parent, slot_b);
                if ra != rb && size[ra] + size[rb] <= limit {
                    let lo = ra.min(rb);
                    let hi = ra.max(rb);
                    parent[hi] = lo;
                    size[lo] += size[hi];
                    clusters -= 1;
                }
            }
        }
    }

    let root: Vec<usize> = (0..n).map(|i| find(&mut parent, i)).collect();
    let mut uniq_roots = root.clone();
    uniq_roots.sort_unstable();
    uniq_roots.dedup();
    root.iter().map(|r| uniq_roots.binary_search(r).unwrap() as i32).collect()
}

fn glue_cluster(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = GLUE_CLUSTER.one(inputs, 0)?;
    if geo.topology.has_open() {
        let n = geo.topology.closed_mask().iter().filter(|&&c| !c).count();
        return Err(Error::new(format!(
            "glue_cluster works on polygons, but the input has {n} open curve(s); remove them first, e.g. blast with group '@closed == 0'"
        )));
    }

    let piece_attrib = p.s("piece_attrib")?;
    let output = p.e("output", &["pieces", "constraints"])?;
    let strength = float(p, "strength", Some(0.0), None)?;
    let strength_variation = float(p, "strength_variation", Some(0.0), Some(1.0))?;
    let seed = p.i("seed")?;
    let max_gap = float(p, "max_gap", Some(0.0), None)?;
    let min_area = float(p, "min_area", Some(0.0), None)?;
    let min_share = float(p, "min_share", Some(0.0), Some(1.0))?;
    let clusters = int(p, "clusters", Some(0), None)?;
    let weak = float(p, "weak", Some(0.0), Some(1.0))?;

    let pieces: Vec<i32> = if piece_attrib.is_empty() {
        super::attrib_tools::piece_labels(&geo).1
    } else {
        let attr = geo.prim.get(&piece_attrib).ok_or_else(|| {
            Error::new(format!(
                "no primitive attribute {piece_attrib:?}: fracture first, or leave piece attribute empty to use connectivity"
            ))
        })?;
        if attr.width != 1 {
            return Err(Error::new(format!("{piece_attrib:?} must be a scalar integer primitive attribute")));
        }
        match &attr.data {
            AttrData::I32(v) => v.clone(),
            _ => return Err(Error::new(format!("{piece_attrib:?} must be a scalar integer primitive attribute"))),
        }
    };

    let ids: Vec<i32> = pieces.iter().copied().collect::<BTreeSet<_>>().into_iter().collect();

    let contact_list = contacts(&geo, &pieces, max_gap);
    let mut pair_map = BTreeMap::<(i32, i32), f64>::new();
    for (fa, fb, ar) in &contact_list {
        *pair_map.entry((pieces[*fa], pieces[*fb])).or_default() += *ar;
    }

    let max_area = pair_map.values().copied().fold(0.0f64, f64::max);
    let cutoff = min_area.max(min_share * max_area).max(1e-300);

    let mut a = Vec::new();
    let mut b = Vec::new();
    let mut area = Vec::new();
    for ((pa, pb), ar) in pair_map {
        if ar >= cutoff {
            a.push(pa);
            b.push(pb);
            area.push(ar);
        }
    }

    let mut rng = Rng::new(&[(seed & 0xFFFF_FFFF) as u64, 41]);
    let force: Vec<f64> = (0..area.len()).map(|i| {
        let noise = rng.random() * 2.0 - 1.0;
        strength * area[i] * (1.0 + strength_variation * noise).max(0.01)
    }).collect();

    let label = cluster_pieces(&ids, &a, &b, &area, clusters);
    let body: &[i32] = if clusters > 0 { &label } else { &ids };

    if output == "pieces" {
        let mut out = (*geo).clone();
        let prim_clusters: Vec<i32> = if ids.is_empty() {
            Vec::new()
        } else {
            pieces.iter().map(|&p| {
                let slot = ids.binary_search(&p).unwrap();
                body[slot]
            }).collect()
        };
        out.prim.insert("cluster".into(), Arc::new(Attr::i32(1, prim_clusters)));

        let mut linked = vec![false; ids.len()];
        if !a.is_empty() {
            for &x in &a {
                if let Ok(slot) = ids.binary_search(&x) {
                    linked[slot] = true;
                }
            }
            for &x in &b {
                if let Ok(slot) = ids.binary_search(&x) {
                    linked[slot] = true;
                }
            }
        }

        let mut face = vec![false; geo.nprims()];
        for (fa, fb, _) in contact_list {
            let pa = pieces[fa];
            if let Ok(slot) = ids.binary_search(&pa) {
                if linked[slot] {
                    face[fa] = true;
                    face[fb] = true;
                }
            }
        }

        let per_prim: Vec<bool> = if ids.is_empty() {
            Vec::new()
        } else {
            pieces.iter().map(|&p| {
                let slot = ids.binary_search(&p).unwrap();
                linked[slot]
            }).collect()
        };
        let loose: Vec<bool> = per_prim.iter().map(|&x| !x).collect();

        outgroup(&mut out, p, "glued", "prim", &per_prim)?;
        outgroup(&mut out, p, "loose", "prim", &loose)?;
        outgroup(&mut out, p, "interface", "prim", &face)?;
        return Ok(out);
    }

    // output == "constraints"
    let pos = positions(&geo);
    let mut centres = vec![[0.0f64; 3]; ids.len()];
    let mut counts = vec![0usize; ids.len()];
    for (i, &piece) in pieces.iter().enumerate() {
        if let Ok(k) = ids.binary_search(&piece) {
            for v in range(&geo, i) {
                let pt = geo.topology.vertex_point[v] as usize;
                centres[k] = add(centres[k], pos[pt]);
                counts[k] += 1;
            }
        }
    }
    for (c, n) in centres.iter_mut().zip(counts) {
        *c = mul(*c, 1.0 / n.max(1) as f64);
    }

    let ba: Vec<i32> = a.iter().map(|&x| body[ids.binary_search(&x).unwrap()]).collect();
    let bb: Vec<i32> = b.iter().map(|&x| body[ids.binary_search(&x).unwrap()]).collect();

    let n_links = a.len();
    let mut vp = Vec::with_capacity(n_links * 2);
    for k in 0..n_links {
        let sa = ids.binary_search(&a[k]).unwrap() as i32;
        let sb = ids.binary_search(&b[k]).unwrap() as i32;
        vp.push(sa);
        vp.push(sb);
    }
    let prim_offsets: Vec<i32> = (0..=n_links).map(|i| (i * 2) as i32).collect();
    let closed = vec![false; n_links];
    let p_flat: Vec<f32> = centres.into_iter().flatten().map(|v| v as f32).collect();

    let mut out = Geometry::from_arrays(p_flat, vp, prim_offsets, Some(closed))?;
    out.point.insert("piece".into(), Arc::new(Attr::i32(1, ids)));
    out.point.insert("cluster".into(), Arc::new(Attr::i32(1, label)));

    out.prim.insert("piece_a".into(), Arc::new(Attr::i32(1, a)));
    out.prim.insert("piece_b".into(), Arc::new(Attr::i32(1, b)));
    out.prim.insert("rbd_a".into(), Arc::new(Attr::i32(1, ba.clone())));
    out.prim.insert("rbd_b".into(), Arc::new(Attr::i32(1, bb.clone())));
    out.prim.insert("area".into(), Arc::new(Attr::f32(1, area.iter().map(|&x| x as f32).collect())));
    let strength_attr: Vec<f32> = if strength > 0.0 {
        force.iter().map(|&x| x as f32).collect()
    } else {
        vec![0.0f32; n_links]
    };
    out.prim.insert("strength".into(), Arc::new(Attr::f32(1, strength_attr)));
    let length_attr: Vec<f32> = area.iter().map(|&x| x.sqrt() as f32).collect();
    out.prim.insert("length".into(), Arc::new(Attr::f32(1, length_attr)));

    let (internal, external): (Vec<bool>, Vec<bool>) = if clusters > 0 {
        let int_mask: Vec<bool> = (0..n_links).map(|i| ba[i] == bb[i]).collect();
        let ext_mask: Vec<bool> = int_mask.iter().map(|&x| !x).collect();
        (int_mask, ext_mask)
    } else {
        (vec![false; n_links], vec![true; n_links])
    };
    outgroup(&mut out, p, "internal", "prim", &internal)?;
    outgroup(&mut out, p, "external", "prim", &external)?;

    let ext_count = external.iter().filter(|&&x| x).count();
    let mean = if ext_count > 0 {
        let sum: f64 = (0..n_links).filter(|&i| external[i]).map(|i| force[i]).sum();
        sum / ext_count as f64
    } else {
        0.0
    };
    let is_weak: Vec<bool> = (0..n_links).map(|i| force[i] < weak * mean).collect();
    outgroup(&mut out, p, "weak", "prim", &is_weak)?;

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_geo() -> Geometry {
        // Two disjoint triangles sharing no points
        let p = vec![
            0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0,
            2.0, 0.0, 0.0, 3.0, 0.0, 0.0, 2.0, 1.0, 0.0,
        ];
        let vp = vec![0, 1, 2, 3, 4, 5];
        let offs = vec![0, 3, 6];
        Geometry::from_arrays(p, vp, offs, None).unwrap()
    }

    #[test]
    fn test_assemble_connectivity() {
        let g = test_geo();
        let p = Params::resolve(
            ASSEMBLE.params,
            serde_json::json!({"source": "connectivity", "piece_attrib": ""}).as_object(),
        )
        .unwrap();
        let out = assemble(&[vec![Arc::new(g)]], &p).unwrap();
        assert_eq!(out.prim["piece"].data, AttrData::I32(vec![0, 1]));
        assert_eq!(out.detail["piece_names"].as_array().unwrap().len(), 2);
        assert!(out.prim.contains_key("pivot"));
    }

    #[test]
    fn test_assemble_missing_attrib_error() {
        let g = test_geo();
        let p = Params::resolve(
            ASSEMBLE.params,
            serde_json::json!({"source": "attribute", "piece_attrib": "nope"}).as_object(),
        )
        .unwrap();
        assert!(assemble(&[vec![Arc::new(g)]], &p).is_err());
    }

    fn geom_from_encoded(val: &serde_json::Value) -> Geometry {
        let ints = |v: &serde_json::Value| v.as_array().unwrap().iter().map(|x| x.as_i64().unwrap() as i32).collect::<Vec<_>>();
        let floats = |v: &serde_json::Value| v.as_array().unwrap().iter().map(|x| x.as_f64().unwrap() as f32).collect::<Vec<_>>();
        let p_data = floats(&val["point"]["P"]["data"]);
        let vp = ints(&val["vp"]);
        let offsets = ints(&val["offsets"]);
        let closed: Vec<bool> = val["closed"].as_array().unwrap().iter().map(|x| x.as_bool().unwrap()).collect();
        let mut g = Geometry::from_arrays(p_data, vp, offsets, Some(closed)).unwrap();
        if let Some(pts) = val["point"].as_object() {
            for (name, a) in pts {
                if name == "P" { continue; }
                let width = a["width"].as_u64().unwrap() as usize;
                let dtype = a["dtype"].as_str().unwrap();
                let attr = if dtype == "int32" {
                    Attr::i32(width, ints(&a["data"]))
                } else {
                    Attr::f32(width, floats(&a["data"]))
                };
                g.point.insert(name.clone(), Arc::new(attr));
            }
        }
        if let Some(prims) = val["prim"].as_object() {
            for (name, a) in prims {
                let width = a["width"].as_u64().unwrap() as usize;
                let dtype = a["dtype"].as_str().unwrap();
                let attr = if dtype == "int32" {
                    Attr::i32(width, ints(&a["data"]))
                } else {
                    Attr::f32(width, floats(&a["data"]))
                };
                g.prim.insert(name.clone(), Arc::new(attr));
            }
        }
        g
    }

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol * (1.0 + a.abs().max(b.abs()))
    }

    fn check_parity(name: &str, got: &Geometry, want: &serde_json::Value, tol: f64) {
        let ints = |v: &serde_json::Value| v.as_array().unwrap().iter().map(|x| x.as_i64().unwrap() as i32).collect::<Vec<_>>();
        assert_eq!(got.topology.vertex_point, ints(&want["vp"]), "{name}: vertex_point");
        assert_eq!(got.topology.prim_offsets, ints(&want["offsets"]), "{name}: offsets");
        let closed: Vec<bool> = want["closed"].as_array().unwrap().iter().map(|x| x.as_bool().unwrap()).collect();
        assert_eq!(got.topology.closed_mask(), closed, "{name}: closed");
        for (dom, table) in [("point", &got.point), ("prim", &got.prim)] {
            let w = want[dom].as_object().unwrap();
            let mut names: Vec<&String> = table.keys().collect();
            let mut expected: Vec<&String> = w.keys().collect();
            names.sort();
            expected.sort();
            assert_eq!(names, expected, "{name}: {dom} attribute names");
            for (attr, a) in table {
                let e = &w[attr];
                assert_eq!(a.width as u64, e["width"].as_u64().unwrap(), "{name}: {dom}/{attr} width");
                assert_eq!(a.dtype(), e["dtype"].as_str().unwrap(), "{name}: {dom}/{attr} dtype");
                let data = e["data"].as_array().unwrap();
                match &a.data {
                    AttrData::F32(v) => {
                        assert_eq!(v.len(), data.len(), "{name}: {dom}/{attr} len");
                        for (i, (x, y)) in v.iter().zip(data).enumerate() {
                            let yf = y.as_f64().unwrap();
                            assert!(close(*x as f64, yf, tol), "{name}: {dom}/{attr}[{i}] {x} != {yf}");
                        }
                    }
                    AttrData::I32(v) => assert_eq!(*v, ints(&e["data"]), "{name}: {dom}/{attr}"),
                }
            }
        }
    }

    #[test]
    fn test_glue_cluster_golden_parity() {
        let cases_json: serde_json::Value =
            serde_json::from_str(include_str!("test_data/glue_cluster_golden.json")).unwrap();
        let s = &GLUE_CLUSTER;
        for c in cases_json.as_array().unwrap() {
            let name = c["name"].as_str().unwrap();
            let input_geo = geom_from_encoded(&c["input"]);
            let p = Params::resolve(s.params, c["params"].as_object()).unwrap();
            let got = (s.cook)(&[vec![Arc::new(input_geo)]], &p).unwrap();
            check_parity(name, &got, &c["expected"], 1e-4);
        }
    }

    #[test]
    fn test_glue_cluster_open_curves_error() {
        // An open line should produce an error
        let p = vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0];
        let vp = vec![0, 1];
        let offs = vec![0, 2];
        let g = Geometry::from_arrays(p, vp, offs, Some(vec![false])).unwrap();
        let s = &GLUE_CLUSTER;
        let p = Params::resolve(s.params, None).unwrap();
        let err = (s.cook)(&[vec![Arc::new(g)]], &p).unwrap_err();
        assert!(err.0.contains("glue_cluster works on polygons, but the input has 1 open curve(s)"));
    }

    #[test]
    fn test_glue_cluster_seeded_variation_and_zero_strength() {
        let cases_json: serde_json::Value =
            serde_json::from_str(include_str!("test_data/glue_cluster_golden.json")).unwrap();
        let s = &GLUE_CLUSTER;
        let c0 = &cases_json[0]; // voronoi_constraints
        let input_geo = geom_from_encoded(&c0["input"]);

        // Same seed => same strength
        let p1 = Params::resolve(s.params, serde_json::json!({"output": "constraints", "strength": 1000.0, "strength_variation": 0.5, "seed": 7}).as_object()).unwrap();
        let p2 = Params::resolve(s.params, serde_json::json!({"output": "constraints", "strength": 1000.0, "strength_variation": 0.5, "seed": 7}).as_object()).unwrap();
        let out1 = (s.cook)(&[vec![Arc::new(input_geo.clone())]], &p1).unwrap();
        let out2 = (s.cook)(&[vec![Arc::new(input_geo.clone())]], &p2).unwrap();
        assert_eq!(out1.prim["strength"].as_f32().unwrap(), out2.prim["strength"].as_f32().unwrap());

        // Different seed => different strength
        let p3 = Params::resolve(s.params, serde_json::json!({"output": "constraints", "strength": 1000.0, "strength_variation": 0.5, "seed": 8}).as_object()).unwrap();
        let out3 = (s.cook)(&[vec![Arc::new(input_geo.clone())]], &p3).unwrap();
        assert_ne!(out1.prim["strength"].as_f32().unwrap(), out3.prim["strength"].as_f32().unwrap());

        // Zero strength => all strength is 0.0
        let p_zero = Params::resolve(s.params, serde_json::json!({"output": "constraints", "strength": 0.0}).as_object()).unwrap();
        let out_zero = (s.cook)(&[vec![Arc::new(input_geo.clone())]], &p_zero).unwrap();
        for &st in out_zero.prim["strength"].as_f32().unwrap() {
            assert_eq!(st, 0.0);
        }
    }

    #[test]
    fn test_glue_cluster_filters() {
        let cases_json: serde_json::Value =
            serde_json::from_str(include_str!("test_data/glue_cluster_golden.json")).unwrap();
        let s = &GLUE_CLUSTER;
        let c0 = &cases_json[0];
        let input_geo = geom_from_encoded(&c0["input"]);

        // Regular cook
        let p_base = Params::resolve(s.params, serde_json::json!({"output": "constraints"}).as_object()).unwrap();
        let out_base = (s.cook)(&[vec![Arc::new(input_geo.clone())]], &p_base).unwrap();

        // Huge min_area drops all links
        let p_min_area = Params::resolve(s.params, serde_json::json!({"output": "constraints", "min_area": 1e9}).as_object()).unwrap();
        let out_area = (s.cook)(&[vec![Arc::new(input_geo.clone())]], &p_min_area).unwrap();
        assert_eq!(out_area.nprims(), 0);

        // min_share = 1.0 keeps only the largest link(s)
        let p_share = Params::resolve(s.params, serde_json::json!({"output": "constraints", "min_share": 1.0}).as_object()).unwrap();
        let out_share = (s.cook)(&[vec![Arc::new(input_geo.clone())]], &p_share).unwrap();
        assert!(out_share.nprims() >= 1 && out_share.nprims() < out_base.nprims());
    }

    #[test]
    fn test_glue_cluster_invalid_attrib_error() {
        let g = test_geo();
        let s = &GLUE_CLUSTER;
        let p = Params::resolve(s.params, serde_json::json!({"piece_attrib": "nonexistent"}).as_object()).unwrap();
        let err = (s.cook)(&[vec![Arc::new(g)]], &p).unwrap_err();
        assert!(err.0.contains("no primitive attribute \"nonexistent\""));
    }

    #[test]
    fn test_glue_cluster_empty_and_single_element() {
        let s = &GLUE_CLUSTER;
        let empty = Geometry::points_only(vec![]).unwrap();

        // Empty geometry in pieces mode
        let p_pcs = Params::resolve(s.params, serde_json::json!({"output": "pieces", "piece_attrib": ""}).as_object()).unwrap();
        let out_pcs = (s.cook)(&[vec![Arc::new(empty.clone())]], &p_pcs).unwrap();
        assert_eq!(out_pcs.npoints, 0);
        assert_eq!(out_pcs.nprims(), 0);

        // Empty geometry in constraints mode
        let p_cons = Params::resolve(s.params, serde_json::json!({"output": "constraints", "piece_attrib": ""}).as_object()).unwrap();
        let out_cons = (s.cook)(&[vec![Arc::new(empty)]], &p_cons).unwrap();
        assert_eq!(out_cons.nprims(), 0);

        // Open curve fails
        let curve = Geometry::from_arrays(
            vec![0., 0., 0., 1., 0., 0.],
            vec![0, 1],
            vec![0, 2],
            Some(vec![false]),
        ).unwrap();
        let err_curve = (s.cook)(&[vec![Arc::new(curve)]], &p_pcs).unwrap_err();
        assert!(err_curve.0.contains("open curve"));

        // Single triangle (single piece)
        let tri = Geometry::from_arrays(
            vec![0., 0., 0., 1., 0., 0., 0., 1., 0.],
            vec![0, 1, 2],
            vec![0, 3],
            None,
        ).unwrap();
        let out_tri = (s.cook)(&[vec![Arc::new(tri)]], &p_pcs).unwrap();
        assert_eq!(out_tri.nprims(), 1);
        assert_eq!(out_tri.prim["cluster"].as_i32().unwrap(), &[0]);
    }

    #[test]
    fn test_glue_cluster_invalid_params_and_determinism() {
        let s = &GLUE_CLUSTER;
        let g = test_geo();

        // Invalid output enum
        let p_bad_out = Params::resolve(s.params, serde_json::json!({"output": "everything", "piece_attrib": ""}).as_object()).unwrap();
        assert!((s.cook)(&[vec![Arc::new(g.clone())]], &p_bad_out).is_err());

        // Negative strength or clusters
        let p_neg_str = Params::resolve(s.params, serde_json::json!({"strength": -1.0, "piece_attrib": ""}).as_object()).unwrap();
        assert!((s.cook)(&[vec![Arc::new(g.clone())]], &p_neg_str).is_err());

        // Determinism: seeded random gives identical clusters and strength
        let p1 = Params::resolve(s.params, serde_json::json!({"seed": 42, "clusters": 2, "output": "pieces", "piece_attrib": ""}).as_object()).unwrap();
        let p2 = Params::resolve(s.params, serde_json::json!({"seed": 42, "clusters": 2, "output": "pieces", "piece_attrib": ""}).as_object()).unwrap();
        let out1 = (s.cook)(&[vec![Arc::new(g.clone())]], &p1).unwrap();
        let out2 = (s.cook)(&[vec![Arc::new(g)]], &p2).unwrap();
        assert_eq!(out1.prim["cluster"].data, out2.prim["cluster"].data);
    }
}
