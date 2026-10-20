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

mod bridge;
mod revolve;
mod skin;
use bridge::bridge;
use revolve::revolve;
use skin::skin;

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
