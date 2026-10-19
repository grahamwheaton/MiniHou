use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crate::error::{Error, Result};
use crate::geometry::{Attr, AttrData, Geometry};
use crate::nodes::{model, InputSpec, Inputs, NodeSpec, GEO};
use crate::params::Params;

pub(super) const NODE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None,
    name: "extract_centroid",
    inputs: GEO,
    params: crate::params!(
        "group": "";
        "domain": "detail";
        "method": "center_of_mass";
        "output": "points";
        "attribute_name": "centroid";
        "transfer_attribute": "";
    ),
    cook,
};

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn length(v: [f64; 3]) -> f64 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

fn prim_com(p: &[f32], vp: &[i32], start: usize, end: usize) -> ([f64; 3], f64) {
    let count = end - start;
    if count == 0 {
        return ([0.0, 0.0, 0.0], 0.0);
    }
    if count == 1 {
        let idx = vp[start] as usize * 3;
        return ([p[idx] as f64, p[idx + 1] as f64, p[idx + 2] as f64], 0.0);
    }
    if count == 2 {
        let i0 = vp[start] as usize * 3;
        let i1 = vp[start + 1] as usize * 3;
        let p0 = [p[i0] as f64, p[i0 + 1] as f64, p[i0 + 2] as f64];
        let p1 = [p[i1] as f64, p[i1 + 1] as f64, p[i1 + 2] as f64];
        let len = ((p1[0] - p0[0]).powi(2) + (p1[1] - p0[1]).powi(2) + (p1[2] - p0[2]).powi(2)).sqrt();
        return ([(p0[0] + p1[0]) * 0.5, (p0[1] + p1[1]) * 0.5, (p0[2] + p1[2]) * 0.5], len);
    }

    let i0 = vp[start] as usize * 3;
    let p0 = [p[i0] as f64, p[i0 + 1] as f64, p[i0 + 2] as f64];
    let mut total_area = 0.0;
    let mut weighted_center = [0.0, 0.0, 0.0];

    for i in 1..count - 1 {
        let i1 = vp[start + i] as usize * 3;
        let i2 = vp[start + i + 1] as usize * 3;
        let p1 = [p[i1] as f64, p[i1 + 1] as f64, p[i1 + 2] as f64];
        let p2 = [p[i2] as f64, p[i2 + 1] as f64, p[i2 + 2] as f64];

        let e1 = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
        let e2 = [p2[0] - p0[0], p2[1] - p0[1], p2[2] - p0[2]];
        let area = 0.5 * length(cross(e1, e2));
        let center = [
            (p0[0] + p1[0] + p2[0]) / 3.0,
            (p0[1] + p1[1] + p2[1]) / 3.0,
            (p0[2] + p1[2] + p2[2]) / 3.0,
        ];
        total_area += area;
        weighted_center[0] += center[0] * area;
        weighted_center[1] += center[1] * area;
        weighted_center[2] += center[2] * area;
    }

    if total_area > 1e-12 {
        (
            [
                weighted_center[0] / total_area,
                weighted_center[1] / total_area,
                weighted_center[2] / total_area,
            ],
            total_area,
        )
    } else {
        let mut avg = [0.0, 0.0, 0.0];
        for i in 0..count {
            let idx = vp[start + i] as usize * 3;
            avg[0] += p[idx] as f64;
            avg[1] += p[idx + 1] as f64;
            avg[2] += p[idx + 2] as f64;
        }
        (
            [
                avg[0] / count as f64,
                avg[1] / count as f64,
                avg[2] / count as f64,
            ],
            0.0,
        )
    }
}

fn prim_bbox(p: &[f32], vp: &[i32], start: usize, end: usize) -> [f64; 3] {
    let count = end - start;
    if count == 0 {
        return [0.0, 0.0, 0.0];
    }
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    for i in start..end {
        let idx = vp[i] as usize * 3;
        for k in 0..3 {
            let val = p[idx + k] as f64;
            min[k] = min[k].min(val);
            max[k] = max[k].max(val);
        }
    }
    [
        (min[0] + max[0]) * 0.5,
        (min[1] + max[1]) * 0.5,
        (min[2] + max[2]) * 0.5,
    ]
}

fn cook(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = NODE.one(inputs, 0)?;
    if geo.npoints == 0 {
        return Ok((*geo).clone());
    }

    let group = p.s("group")?;
    let domain = p.e("domain", &["detail", "pieces", "primitives"])?;
    let method = p.e("method", &["center_of_mass", "bounding_box"])?;
    let output = p.e("output", &["points", "attribute"])?;
    let attribute_name = p.s("attribute_name")?;
    let transfer_attribute = p.s("transfer_attribute")?;

    let p_slice = geo.p();
    let vp = &geo.topology.vertex_point;
    let offsets = &geo.topology.prim_offsets;
    let nprims = geo.nprims();

    match domain.as_str() {
        "primitives" => {
            let mask = if group.trim().is_empty() {
                vec![true; nprims]
            } else {
                model::prim_mask(&geo, &group)?
            };

            let mut centroids = Vec::new();
            let mut matching_prims = Vec::new();
            for f in 0..nprims {
                if !mask[f] {
                    continue;
                }
                matching_prims.push(f);
                let (s, e) = (offsets[f] as usize, offsets[f + 1] as usize);
                let c = if method == "bounding_box" {
                    prim_bbox(p_slice, vp, s, e)
                } else {
                    prim_com(p_slice, vp, s, e).0
                };
                centroids.push(c);
            }

            if output == "attribute" {
                let mut out = (*geo).clone();
                let mut attr_data = vec![0.0f32; nprims * 3];
                for (idx, &f) in matching_prims.iter().enumerate() {
                    let c = centroids[idx];
                    attr_data[f * 3] = c[0] as f32;
                    attr_data[f * 3 + 1] = c[1] as f32;
                    attr_data[f * 3 + 2] = c[2] as f32;
                }
                out.prim.insert(attribute_name, Arc::new(Attr::f32(3, attr_data)));
                Ok(out)
            } else {
                let mut pts = Vec::with_capacity(centroids.len() * 3);
                for c in &centroids {
                    pts.push(c[0] as f32);
                    pts.push(c[1] as f32);
                    pts.push(c[2] as f32);
                }
                let mut out = Geometry::points_only(pts)?;
                // Transfer primitive attributes if requested
                let transfer_names: Vec<&str> = transfer_attribute.split_whitespace().collect();
                if !transfer_names.is_empty() {
                    for (name, attr) in &geo.prim {
                        if transfer_names.contains(&name.as_str()) || transfer_names.contains(&"*") {
                            let w = attr.width;
                            match &attr.data {
                                AttrData::F32(v) => {
                                    let mut new_data = Vec::with_capacity(matching_prims.len() * w);
                                    for &f in &matching_prims {
                                        new_data.extend_from_slice(&v[f * w..(f + 1) * w]);
                                    }
                                    out.point.insert(name.clone(), Arc::new(Attr::f32(w, new_data)));
                                }
                                AttrData::I32(v) => {
                                    let mut new_data = Vec::with_capacity(matching_prims.len() * w);
                                    for &f in &matching_prims {
                                        new_data.extend_from_slice(&v[f * w..(f + 1) * w]);
                                    }
                                    out.point.insert(name.clone(), Arc::new(Attr::i32(w, new_data)));
                                }
                            }
                        }
                    }
                }
                Ok(out)
            }
        }
        "pieces" => {
            // Find piece labels for each primitive (or point)
            let (prim_pieces, point_pieces): (Vec<i64>, Vec<i64>) = if let Some(attr) = geo.prim.get("piece") {
                let pr: Vec<i64> = match &attr.data {
                    AttrData::I32(v) => v.iter().map(|&x| x as i64).collect(),
                    AttrData::F32(v) => v.iter().map(|&x| x as i64).collect(),
                };
                let mut pt = vec![-1i64; geo.npoints];
                for f in 0..nprims {
                    let label = pr[f];
                    for v in offsets[f] as usize..offsets[f + 1] as usize {
                        pt[vp[v] as usize] = label;
                    }
                }
                (pr, pt)
            } else if let Some(attr) = geo.point.get("piece") {
                let pt: Vec<i64> = match &attr.data {
                    AttrData::I32(v) => v.iter().map(|&x| x as i64).collect(),
                    AttrData::F32(v) => v.iter().map(|&x| x as i64).collect(),
                };
                let pr: Vec<i64> = (0..nprims)
                    .map(|f| {
                        let s = offsets[f] as usize;
                        if s < offsets[f + 1] as usize {
                            pt[vp[s] as usize]
                        } else {
                            -1
                        }
                    })
                    .collect();
                (pr, pt)
            } else {
                let (pt, pr) = crate::nodes::attrib_tools::piece_labels(&geo);
                (pr.into_iter().map(|x| x as i64).collect(), pt.into_iter().map(|x| x as i64).collect())
            };

            let mask = if group.trim().is_empty() {
                vec![true; nprims]
            } else {
                model::prim_mask(&geo, &group)?
            };

            // Group primitives and points by piece label
            let mut piece_ids = BTreeSet::new();
            for f in 0..nprims {
                if mask[f] && prim_pieces[f] >= 0 {
                    piece_ids.insert(prim_pieces[f]);
                }
            }
            if piece_ids.is_empty() {
                for i in 0..geo.npoints {
                    if point_pieces[i] >= 0 {
                        piece_ids.insert(point_pieces[i]);
                    }
                }
            }

            let sorted_pieces: Vec<i64> = piece_ids.into_iter().collect();
            let piece_to_idx: BTreeMap<i64, usize> = sorted_pieces
                .iter()
                .enumerate()
                .map(|(i, &p)| (p, i))
                .collect();

            let mut centroids = vec![[0.0, 0.0, 0.0]; sorted_pieces.len()];

            for (piece_idx, &piece_id) in sorted_pieces.iter().enumerate() {
                let mut prim_indices = Vec::new();
                for f in 0..nprims {
                    if mask[f] && prim_pieces[f] == piece_id {
                        prim_indices.push(f);
                    }
                }

                if method == "bounding_box" {
                    let mut min = [f64::INFINITY; 3];
                    let mut max = [f64::NEG_INFINITY; 3];
                    let mut count = 0;
                    if !prim_indices.is_empty() {
                        for &f in &prim_indices {
                            for v in offsets[f] as usize..offsets[f + 1] as usize {
                                let idx = vp[v] as usize * 3;
                                for k in 0..3 {
                                    min[k] = min[k].min(p_slice[idx + k] as f64);
                                    max[k] = max[k].max(p_slice[idx + k] as f64);
                                }
                                count += 1;
                            }
                        }
                    } else {
                        for i in 0..geo.npoints {
                            if point_pieces[i] == piece_id {
                                let idx = i * 3;
                                for k in 0..3 {
                                    min[k] = min[k].min(p_slice[idx + k] as f64);
                                    max[k] = max[k].max(p_slice[idx + k] as f64);
                                }
                                count += 1;
                            }
                        }
                    }
                    if count > 0 {
                        centroids[piece_idx] = [
                            (min[0] + max[0]) * 0.5,
                            (min[1] + max[1]) * 0.5,
                            (min[2] + max[2]) * 0.5,
                        ];
                    }
                } else {
                    // Center of mass
                    let mut total_area = 0.0;
                    let mut weighted_center = [0.0, 0.0, 0.0];
                    for &f in &prim_indices {
                        let (s, e) = (offsets[f] as usize, offsets[f + 1] as usize);
                        let (c, area) = prim_com(p_slice, vp, s, e);
                        if area > 1e-12 {
                            total_area += area;
                            weighted_center[0] += c[0] * area;
                            weighted_center[1] += c[1] * area;
                            weighted_center[2] += c[2] * area;
                        }
                    }
                    if total_area > 1e-12 {
                        centroids[piece_idx] = [
                            weighted_center[0] / total_area,
                            weighted_center[1] / total_area,
                            weighted_center[2] / total_area,
                        ];
                    } else {
                        // Average points in this piece
                        let mut sum = [0.0, 0.0, 0.0];
                        let mut cnt = 0;
                        if !prim_indices.is_empty() {
                            let mut seen_pts = BTreeSet::new();
                            for &f in &prim_indices {
                                for v in offsets[f] as usize..offsets[f + 1] as usize {
                                    seen_pts.insert(vp[v] as usize);
                                }
                            }
                            for pt in seen_pts {
                                let idx = pt * 3;
                                sum[0] += p_slice[idx] as f64;
                                sum[1] += p_slice[idx + 1] as f64;
                                sum[2] += p_slice[idx + 2] as f64;
                                cnt += 1;
                            }
                        } else {
                            for i in 0..geo.npoints {
                                if point_pieces[i] == piece_id {
                                    let idx = i * 3;
                                    sum[0] += p_slice[idx] as f64;
                                    sum[1] += p_slice[idx + 1] as f64;
                                    sum[2] += p_slice[idx + 2] as f64;
                                    cnt += 1;
                                }
                            }
                        }
                        if cnt > 0 {
                            centroids[piece_idx] = [
                                sum[0] / cnt as f64,
                                sum[1] / cnt as f64,
                                sum[2] / cnt as f64,
                            ];
                        }
                    }
                }
            }

            if output == "attribute" {
                let mut out = (*geo).clone();
                let mut prim_attr = vec![0.0f32; nprims * 3];
                for f in 0..nprims {
                    let pid = prim_pieces[f];
                    if let Some(&k) = piece_to_idx.get(&pid) {
                        let c = centroids[k];
                        prim_attr[f * 3] = c[0] as f32;
                        prim_attr[f * 3 + 1] = c[1] as f32;
                        prim_attr[f * 3 + 2] = c[2] as f32;
                    }
                }
                out.prim.insert(attribute_name.clone(), Arc::new(Attr::f32(3, prim_attr)));

                let mut pt_attr = vec![0.0f32; geo.npoints * 3];
                for i in 0..geo.npoints {
                    let pid = point_pieces[i];
                    if let Some(&k) = piece_to_idx.get(&pid) {
                        let c = centroids[k];
                        pt_attr[i * 3] = c[0] as f32;
                        pt_attr[i * 3 + 1] = c[1] as f32;
                        pt_attr[i * 3 + 2] = c[2] as f32;
                    }
                }
                out.point.insert(attribute_name, Arc::new(Attr::f32(3, pt_attr)));
                Ok(out)
            } else {
                let mut pts = Vec::with_capacity(centroids.len() * 3);
                for c in &centroids {
                    pts.push(c[0] as f32);
                    pts.push(c[1] as f32);
                    pts.push(c[2] as f32);
                }
                let mut out = Geometry::points_only(pts)?;
                // Write piece attribute
                let piece_i32: Vec<i32> = sorted_pieces.iter().map(|&x| x as i32).collect();
                out.point.insert("piece".to_string(), Arc::new(Attr::i32(1, piece_i32)));

                // Transfer primitive attributes if requested
                let transfer_names: Vec<&str> = transfer_attribute.split_whitespace().collect();
                if !transfer_names.is_empty() {
                    for (name, attr) in &geo.prim {
                        if transfer_names.contains(&name.as_str()) || transfer_names.contains(&"*") {
                            let w = attr.width;
                            match &attr.data {
                                AttrData::F32(v) => {
                                    let mut new_data = Vec::with_capacity(sorted_pieces.len() * w);
                                    for &pid in &sorted_pieces {
                                        let mut found = false;
                                        for f in 0..nprims {
                                            if prim_pieces[f] == pid {
                                                new_data.extend_from_slice(&v[f * w..(f + 1) * w]);
                                                found = true;
                                                break;
                                            }
                                        }
                                        if !found {
                                            new_data.extend(std::iter::repeat(0.0).take(w));
                                        }
                                    }
                                    out.point.insert(name.clone(), Arc::new(Attr::f32(w, new_data)));
                                }
                                AttrData::I32(v) => {
                                    let mut new_data = Vec::with_capacity(sorted_pieces.len() * w);
                                    for &pid in &sorted_pieces {
                                        let mut found = false;
                                        for f in 0..nprims {
                                            if prim_pieces[f] == pid {
                                                new_data.extend_from_slice(&v[f * w..(f + 1) * w]);
                                                found = true;
                                                break;
                                            }
                                        }
                                        if !found {
                                            new_data.extend(std::iter::repeat(0).take(w));
                                        }
                                    }
                                    out.point.insert(name.clone(), Arc::new(Attr::i32(w, new_data)));
                                }
                            }
                        }
                    }
                }
                Ok(out)
            }
        }
        _ => {
            // "detail"
            let mask = if group.trim().is_empty() {
                vec![true; nprims]
            } else {
                model::prim_mask(&geo, &group)?
            };

            let centroid = if method == "bounding_box" {
                let mut min = [f64::INFINITY; 3];
                let mut max = [f64::NEG_INFINITY; 3];
                let mut count = 0;
                if nprims > 0 {
                    for f in 0..nprims {
                        if !mask[f] {
                            continue;
                        }
                        for v in offsets[f] as usize..offsets[f + 1] as usize {
                            let idx = vp[v] as usize * 3;
                            for k in 0..3 {
                                min[k] = min[k].min(p_slice[idx + k] as f64);
                                max[k] = max[k].max(p_slice[idx + k] as f64);
                            }
                            count += 1;
                        }
                    }
                }
                if count == 0 {
                    for i in 0..geo.npoints {
                        let idx = i * 3;
                        for k in 0..3 {
                            min[k] = min[k].min(p_slice[idx + k] as f64);
                            max[k] = max[k].max(p_slice[idx + k] as f64);
                        }
                    }
                }
                [
                    (min[0] + max[0]) * 0.5,
                    (min[1] + max[1]) * 0.5,
                    (min[2] + max[2]) * 0.5,
                ]
            } else {
                // Center of mass
                let mut total_area = 0.0;
                let mut weighted_center = [0.0, 0.0, 0.0];
                for f in 0..nprims {
                    if !mask[f] {
                        continue;
                    }
                    let (s, e) = (offsets[f] as usize, offsets[f + 1] as usize);
                    let (c, area) = prim_com(p_slice, vp, s, e);
                    if area > 1e-12 {
                        total_area += area;
                        weighted_center[0] += c[0] * area;
                        weighted_center[1] += c[1] * area;
                        weighted_center[2] += c[2] * area;
                    }
                }
                if total_area > 1e-12 {
                    [
                        weighted_center[0] / total_area,
                        weighted_center[1] / total_area,
                        weighted_center[2] / total_area,
                    ]
                } else {
                    let mut sum = [0.0, 0.0, 0.0];
                    for i in 0..geo.npoints {
                        let idx = i * 3;
                        sum[0] += p_slice[idx] as f64;
                        sum[1] += p_slice[idx + 1] as f64;
                        sum[2] += p_slice[idx + 2] as f64;
                    }
                    [
                        sum[0] / geo.npoints as f64,
                        sum[1] / geo.npoints as f64,
                        sum[2] / geo.npoints as f64,
                    ]
                }
            };

            if output == "attribute" {
                let mut out = (*geo).clone();
                let c_f32 = [centroid[0] as f32, centroid[1] as f32, centroid[2] as f32];
                out.detail.insert(attribute_name.clone(), serde_json::json!(c_f32));
                let mut pt_data = Vec::with_capacity(geo.npoints * 3);
                for _ in 0..geo.npoints {
                    pt_data.extend_from_slice(&c_f32);
                }
                out.point.insert(attribute_name, Arc::new(Attr::f32(3, pt_data)));
                Ok(out)
            } else {
                let out = Geometry::points_only(vec![
                    centroid[0] as f32,
                    centroid[1] as f32,
                    centroid[2] as f32,
                ])?;
                Ok(out)
            }
        }
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
