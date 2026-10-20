use std::collections::HashMap;
use std::sync::Arc;

use super::ASSEMBLE;
use super::super::Inputs;

use crate::error::{Error, Result};
use crate::geometry::{Attr, AttrData, Geometry};
use crate::params::Params;

pub(super) fn assemble(inputs: &Inputs, p: &Params) -> Result<Geometry> {
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
            let (_, prim_labels) = super::super::attrib_tools::piece_labels(&geo);
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
                    if super::super::groups::point_mask(
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
                    let m = super::super::group::flags(attr);
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
                new_point.insert(name.clone(), Arc::new(super::super::group::gather(attr, &src)));
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
            super::super::model::write_group(&mut out, "prim", &format!("{name_prefix}{k}"), &mask)?;
        }
    }

    if remove_source_groups {
        for name in used_groups {
            out.prim.remove(&format!("group_{name}"));
        }
    }

    if let (Some(shared), Some(copied)) = (shared_mask_opt, copied_mask_opt) {
        super::super::model::outgroup(&mut out, p, "unshared", "point", &copied)?;
        super::super::model::outgroup(&mut out, p, "shared", "point", &shared)?;
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

