use crate::nodes::{Inputs, NodeSpec, GEO};
use crate::nodes::attribs::matches;
use crate::{error::Result, geometry::Geometry, params::Params};
use std::collections::BTreeSet;

pub(super) const NODE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "clean",
    inputs: GEO,
    params: crate::params!(
        "consolidate_points": false;
        "consolidate_distance": 0.001;
        "fuse_distance": 0.0;
        "remove_degenerate": true;
        "degenerate_tolerance": 0.0001;
        "delete_unused_groups": false;
        "orient_polygons": false, unsupported;
        "reverse_winding": false;
        "fix_overlaps": false;
        "remove_unused_points": true;
        "remove_attributes": "";
        "remove_attribs": "";
        "remove_groups": "";
        "remove_nans": false;
        "manifold_topology": false, unsupported;
        "manifold_only_topology": false, unsupported;
    ),
    cook: clean,
};

fn remove_nan_points(g: &Geometry) -> Result<Geometry> {
    let p = g.p();
    let npoints = g.npoints;
    let mut is_nan = vec![false; npoints];
    let mut any_nan = false;
    for pt in 0..npoints {
        let x = p[3 * pt];
        let y = p[3 * pt + 1];
        let z = p[3 * pt + 2];
        if x.is_nan() || y.is_nan() || z.is_nan() {
            is_nan[pt] = true;
            any_nan = true;
        }
    }
    if !any_nan {
        return Ok(g.clone());
    }

    let mut valid_pt_indices = Vec::new();
    let mut pt_remap = vec![-1i32; npoints];
    for (pt, &nan) in is_nan.iter().enumerate() {
        if !nan {
            pt_remap[pt] = valid_pt_indices.len() as i32;
            valid_pt_indices.push(pt);
        }
    }

    if g.nprims() == 0 {
        let mut out = g.clone();
        out.npoints = valid_pt_indices.len();
        out.point = crate::nodes::model::gather(&g.point, &valid_pt_indices);
        return Ok(out);
    }

    let topo = &g.topology;
    let nprims = g.nprims();
    let vp = &topo.vertex_point;
    let offs = &topo.prim_offsets;

    let mut kept_vp = Vec::new();
    let mut kept_sizes = Vec::new();
    let mut kept_vert_indices = Vec::new();
    let mut kept_prims = Vec::new();
    let mut kept_closed = Vec::new();

    for prim in 0..nprims {
        let start = offs[prim] as usize;
        let end = offs[prim + 1] as usize;
        let count = end.saturating_sub(start);
        if count == 0 {
            continue;
        }
        let mut has_nan_point = false;
        for j in 0..count {
            let pt = vp[start + j] as usize;
            if is_nan[pt] {
                has_nan_point = true;
                break;
            }
        }
        if has_nan_point {
            continue;
        }
        for j in 0..count {
            let pt = vp[start + j] as usize;
            kept_vp.push(pt_remap[pt]);
            kept_vert_indices.push(start + j);
        }
        kept_sizes.push(count);
        kept_prims.push(prim);
        kept_closed.push(topo.is_closed(prim));
    }

    let closed_opt = if topo.has_open() {
        Some(kept_closed)
    } else {
        None
    };

    crate::nodes::model::make(
        crate::nodes::model::gather(&g.point, &valid_pt_indices),
        kept_vp,
        &kept_sizes,
        crate::nodes::model::gather(&g.vertex, &kept_vert_indices),
        crate::nodes::model::gather(&g.prim, &kept_prims),
        g.detail.clone(),
        closed_opt,
    )
}

fn drop_degenerate(g: &Geometry, tolerance: f64) -> Result<Geometry> {
    if g.nprims() == 0 {
        return Ok(g.clone());
    }
    let topo = &g.topology;
    let nprims = g.nprims();
    let vp = &topo.vertex_point;
    let offs = &topo.prim_offsets;
    let p = g.p();

    let mut kept_vp = Vec::new();
    let mut kept_sizes = Vec::new();
    let mut kept_vert_indices = Vec::new();
    let mut kept_prims = Vec::new();
    let mut kept_closed = Vec::new();

    let min_area = if tolerance > 0.0 { tolerance * tolerance } else { 1e-12 };
    let min_len = if tolerance > 0.0 { tolerance } else { 1e-6 };

    for prim in 0..nprims {
        let start = offs[prim] as usize;
        let end = offs[prim + 1] as usize;
        let count = end.saturating_sub(start);
        if count == 0 {
            continue;
        }
        let is_closed = topo.is_closed(prim);

        let mut corners = Vec::new();
        let mut vert_ix = Vec::new();
        for j in 0..count {
            let pt = vp[start + j];
            if j + 1 < count && pt == vp[start + j + 1] {
                continue;
            }
            corners.push(pt);
            vert_ix.push(start + j);
        }

        if is_closed && corners.len() > 1 && corners.first() == corners.last() {
            corners.pop();
            vert_ix.pop();
        }

        let min_corners = if is_closed { 3 } else { 2 };
        if corners.len() < min_corners {
            continue;
        }

        if is_closed {
            let p0 = [
                p[corners[0] as usize * 3] as f64,
                p[corners[0] as usize * 3 + 1] as f64,
                p[corners[0] as usize * 3 + 2] as f64,
            ];
            let mut n = [0.0; 3];
            for i in 1..(corners.len() - 1) {
                let v1 = [
                    p[corners[i] as usize * 3] as f64 - p0[0],
                    p[corners[i] as usize * 3 + 1] as f64 - p0[1],
                    p[corners[i] as usize * 3 + 2] as f64 - p0[2],
                ];
                let v2 = [
                    p[corners[i + 1] as usize * 3] as f64 - p0[0],
                    p[corners[i + 1] as usize * 3 + 1] as f64 - p0[1],
                    p[corners[i + 1] as usize * 3 + 2] as f64 - p0[2],
                ];
                n[0] += v1[1] * v2[2] - v1[2] * v2[1];
                n[1] += v2[0] * v1[2] - v2[2] * v1[0];
                n[2] += v1[0] * v2[1] - v1[1] * v2[0];
            }
            let area = 0.5 * (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            if area <= min_area {
                continue;
            }
        } else {
            let mut total_len = 0.0;
            for i in 0..(corners.len() - 1) {
                let dx = p[corners[i + 1] as usize * 3] - p[corners[i] as usize * 3];
                let dy = p[corners[i + 1] as usize * 3 + 1] - p[corners[i] as usize * 3 + 1];
                let dz = p[corners[i + 1] as usize * 3 + 2] - p[corners[i] as usize * 3 + 2];
                total_len += ((dx * dx + dy * dy + dz * dz) as f64).sqrt();
            }
            if total_len <= min_len {
                continue;
            }
        }

        kept_sizes.push(corners.len());
        kept_vp.extend(corners);
        kept_vert_indices.extend(vert_ix);
        kept_prims.push(prim);
        kept_closed.push(is_closed);
    }

    let closed_opt = if topo.has_open() {
        Some(kept_closed)
    } else {
        None
    };

    crate::nodes::model::make(
        g.point.clone(),
        kept_vp,
        &kept_sizes,
        crate::nodes::model::gather(&g.vertex, &kept_vert_indices),
        crate::nodes::model::gather(&g.prim, &kept_prims),
        g.detail.clone(),
        closed_opt,
    )
}

fn fix_overlapping_prims(g: &Geometry) -> Result<Geometry> {
    if g.nprims() <= 1 {
        return Ok(g.clone());
    }
    let topo = &g.topology;
    let nprims = g.nprims();
    let vp = &topo.vertex_point;
    let offs = &topo.prim_offsets;

    let mut seen = BTreeSet::new();
    let mut kept_vp = Vec::new();
    let mut kept_sizes = Vec::new();
    let mut kept_vert_indices = Vec::new();
    let mut kept_prims = Vec::new();
    let mut kept_closed = Vec::new();

    for prim in 0..nprims {
        let start = offs[prim] as usize;
        let end = offs[prim + 1] as usize;
        let count = end.saturating_sub(start);
        let mut key: Vec<i32> = vp[start..end].to_vec();
        key.sort_unstable();
        if !seen.insert(key) {
            continue;
        }
        kept_vp.extend(&vp[start..end]);
        kept_sizes.push(count);
        kept_vert_indices.extend(start..end);
        kept_prims.push(prim);
        kept_closed.push(topo.is_closed(prim));
    }

    let closed_opt = if topo.has_open() {
        Some(kept_closed)
    } else {
        None
    };

    crate::nodes::model::make(
        g.point.clone(),
        kept_vp,
        &kept_sizes,
        crate::nodes::model::gather(&g.vertex, &kept_vert_indices),
        crate::nodes::model::gather(&g.prim, &kept_prims),
        g.detail.clone(),
        closed_opt,
    )
}

fn reverse_winding_order(g: &Geometry) -> Result<Geometry> {
    if g.nprims() == 0 {
        return Ok(g.clone());
    }
    let topo = &g.topology;
    let nprims = g.nprims();
    let vp = &topo.vertex_point;
    let offs = &topo.prim_offsets;

    let mut new_vp = Vec::with_capacity(vp.len());
    let mut new_vert_indices = Vec::with_capacity(vp.len());
    let mut sizes = Vec::with_capacity(nprims);
    let mut prims = Vec::with_capacity(nprims);
    let mut closed = Vec::with_capacity(nprims);

    for prim in 0..nprims {
        let start = offs[prim] as usize;
        let end = offs[prim + 1] as usize;
        let count = end.saturating_sub(start);
        for j in (0..count).rev() {
            new_vp.push(vp[start + j]);
            new_vert_indices.push(start + j);
        }
        sizes.push(count);
        prims.push(prim);
        closed.push(topo.is_closed(prim));
    }

    let closed_opt = if topo.has_open() {
        Some(closed)
    } else {
        None
    };

    crate::nodes::model::make(
        g.point.clone(),
        new_vp,
        &sizes,
        crate::nodes::model::gather(&g.vertex, &new_vert_indices),
        g.prim.clone(),
        g.detail.clone(),
        closed_opt,
    )
}

fn remove_attributes_by_pattern(g: &mut Geometry, text: &str) {
    let pats: Vec<_> = text.split(|c: char| c == ',' || c.is_whitespace())
        .filter(|s| !s.is_empty()).collect();
    if pats.is_empty() {
        return;
    }
    for table in [&mut g.point, &mut g.vertex, &mut g.prim] {
        table.retain(|n, _| n == "P" || !pats.iter().any(|pat| matches(pat, n)));
    }
    g.detail.retain(|n, _| !pats.iter().any(|pat| matches(pat, n)));
}

fn remove_groups_by_pattern(g: &mut Geometry, text: &str) {
    let pats: Vec<_> = text.split(|c: char| c == ',' || c.is_whitespace())
        .filter(|s| !s.is_empty()).collect();
    if pats.is_empty() {
        return;
    }
    for table in [&mut g.point, &mut g.vertex, &mut g.prim] {
        table.retain(|n, _| {
            let (is_group, gname) = if let Some(stripped) = n.strip_prefix("group_") {
                (true, stripped)
            } else if let Some(stripped) = n.strip_prefix("edge_group_") {
                (true, stripped)
            } else {
                (false, n.as_str())
            };
            if is_group {
                !pats.iter().any(|pat| matches(pat, gname) || matches(pat, n))
            } else {
                true
            }
        });
    }
}

fn delete_unused_groups_fn(g: &mut Geometry) {
    for table in [&mut g.point, &mut g.prim] {
        table.retain(|n, attr| {
            if n.starts_with("group_") || n.starts_with("edge_group_") {
                let has_members = match &attr.data {
                    crate::geometry::AttrData::I32(v) => v.iter().any(|&x| x != 0),
                    crate::geometry::AttrData::F32(v) => v.iter().any(|&x| x != 0.0),
                };
                has_members
            } else {
                true
            }
        });
    }
}

fn clean(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = NODE.one(inputs, 0)?;
    let mut out = (*geo).clone();

    // 1. Remove NaNs
    if p.b("remove_nans")? {
        out = remove_nan_points(&out)?;
    }

    // 2. Consolidate Points
    let consolidate = p.b("consolidate_points")? || p.f("fuse_distance").unwrap_or(0.0) > 0.0;
    let consolidate_distance = if p.b("consolidate_points")? {
        p.f("consolidate_distance")?
    } else {
        p.f("fuse_distance").unwrap_or(0.0)
    };
    if consolidate && consolidate_distance > 0.0 {
        out = crate::nodes::primitives::fuse_points(&out, consolidate_distance, None)?;
    }

    // 3. Remove Degenerate Primitives
    if p.b("remove_degenerate")? {
        let tolerance = p.f("degenerate_tolerance").unwrap_or(0.0001);
        out = drop_degenerate(&out, tolerance)?;
    }

    // 4. Fix Overlaps
    if p.b("fix_overlaps")? {
        out = fix_overlapping_prims(&out)?;
    }

    // 5. Reverse Winding
    if p.b("reverse_winding")? {
        out = reverse_winding_order(&out)?;
    }

    // 6. Delete Unused Groups
    if p.b("delete_unused_groups")? {
        delete_unused_groups_fn(&mut out);
    }

    // 7. Remove Attributes
    let remove_attribs_text = format!("{} {}", p.s("remove_attributes")?, p.s("remove_attribs")?);
    remove_attributes_by_pattern(&mut out, &remove_attribs_text);

    // 8. Remove Groups
    let remove_groups_text = p.s("remove_groups")?;
    remove_groups_by_pattern(&mut out, &remove_groups_text);

    // 9. Remove Unused Points
    if p.b("remove_unused_points")? {
        out = crate::nodes::model::compact(out)?;
    }

    Ok(out)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
