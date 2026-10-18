//! Houdini-parity `polyfill` SOP: fills holes in polygon geometry.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use super::mesh_common::{append, NewPoint};
use super::{edge_tools, groups, model, Inputs, NodeSpec, GEO};
use crate::error::{Error, Result};
use crate::geometry::{Attr, AttrData, Geometry};
use crate::params::{ParamSpec, Params};
use crate::topo;

pub const PARAMS: &[ParamSpec] = crate::params!(
    "boundary_group": "";
    "group": "";
    "fill_group": "";
    "mode": "quadrilaterals";
    "fill_mode": "quadrilaterals";
    "auto_complete_boundaries": true;
    "autocomplete": true;
    "reverse_patches": false;
    "reverse": false;
    "unique_points": false;
    "uniquepoints": false;
    "update_point_normals": true;
    "updatenormals": true;
    "edge_loop": 0.25;
    "edgeloop": 0.25;
    "smooth": 50.0;
    "corner_offset": 0;
    "custom_corners": "";
    "deform_patch": true;
    "deform": true;
    "surface_offset": 0.0;
    "tangent_strength": 0.4;
    "patch_group": "";
    "patchgroup": "";
    "edge_loop_group": "loopgroup";
    "edgeloopgroup": "loopgroup";
    "append_to_groups": false;
    "append_groups": false;
);

pub const POLYFILL: NodeSpec = NodeSpec {
    name: "polyfill",
    inputs: GEO,
    params: PARAMS,
    cook: polyfill,
};

fn get_str_opt(p: &Params, names: &[&str]) -> String {
    for &n in names {
        if let Ok(s) = p.s(n) {
            let s = s.trim();
            if !s.is_empty() {
                return s.to_string();
            }
        }
    }
    String::new()
}

fn get_str(p: &Params, names: &[&str], default: &str) -> String {
    let s = get_str_opt(p, names);
    if s.is_empty() {
        default.to_string()
    } else {
        s
    }
}

fn get_bool(p: &Params, names: &[&str], default: bool) -> bool {
    for &n in names {
        if let Ok(b) = p.b(n) {
            if b != default {
                return b;
            }
        }
    }
    default
}

fn get_f64(p: &Params, names: &[&str], default: f64) -> f64 {
    for &n in names {
        if let Ok(v) = p.f(n) {
            if (v - default).abs() > 1e-9 {
                return v;
            }
        }
    }
    default
}

fn get_i64(p: &Params, names: &[&str], default: i64) -> i64 {
    for &n in names {
        if let Ok(v) = p.i(n) {
            if v != default {
                return v;
            }
        }
    }
    default
}

fn get_mode(p: &Params) -> String {
    for n in &["mode", "fill_mode"] {
        if let Ok(s) = p.s(n) {
            let s = s.trim().to_lowercase();
            if !s.is_empty() && s != "quadrilaterals" {
                return s;
            }
        }
        if let Ok(i) = p.i(n) {
            return match i {
                0 => "quadrilaterals".to_string(),
                1 => "triangles".to_string(),
                2 => "triangle_fan".to_string(),
                3 => "single_polygon".to_string(),
                4 => "quadrilateral_fan".to_string(),
                5 => "quadrilateral_grid".to_string(),
                _ => "quadrilaterals".to_string(),
            };
        }
    }
    "quadrilaterals".to_string()
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn mul(a: [f64; 3], s: f64) -> [f64; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn len(a: [f64; 3]) -> f64 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}

fn normalize(a: [f64; 3]) -> [f64; 3] {
    let l = len(a);
    if l < 1e-12 {
        [0., 0., 1.]
    } else {
        mul(a, 1.0 / l)
    }
}

fn dist(a: [f64; 3], b: [f64; 3]) -> f64 {
    len(sub(a, b))
}

fn angle(a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> f64 {
    let (u, v) = (sub(a, b), sub(c, b));
    let (lu, lv) = (len(u), len(v));
    if lu < 1e-12 || lv < 1e-12 {
        return std::f64::consts::FRAC_PI_2;
    }
    (dot(u, v) / (lu * lv)).clamp(-1., 1.).acos()
}

fn newell_normal(pts: &[[f64; 3]]) -> [f64; 3] {
    let mut n = [0.0; 3];
    let count = pts.len();
    for i in 0..count {
        let (a, b) = (pts[i], pts[(i + 1) % count]);
        n[0] += (a[1] - b[1]) * (a[2] + b[2]);
        n[1] += (a[2] - b[2]) * (a[0] + b[0]);
        n[2] += (a[0] - b[0]) * (a[1] + b[1]);
    }
    normalize(n)
}

struct BoundaryLoop {
    pts: Vec<usize>,
    corners: Vec<usize>,
    is_open: bool,
}

fn get_boundary_mask(g: &Geometry, pattern: &str) -> Result<Vec<bool>> {
    let e = topo::edges(&g.topology, g.npoints);
    let mut mask = vec![false; e.len()];
    let pattern = pattern.trim();
    if pattern.is_empty() {
        for i in 0..e.len() {
            if e.count[i] == 1 {
                mask[i] = true;
            }
        }
        return Ok(mask);
    }

    if let Ok(em) = edge_tools::edge_mask(g, pattern) {
        let mut any = false;
        for i in 0..e.len() {
            if e.count[i] == 1 && em[i] {
                mask[i] = true;
                any = true;
            }
        }
        if any {
            return Ok(mask);
        }
    }

    if let Ok(pm) = groups::point_mask(g, pattern) {
        let mut any = false;
        for i in 0..e.len() {
            if e.count[i] == 1 {
                let p0 = e.points[2 * i] as usize;
                let p1 = e.points[2 * i + 1] as usize;
                if pm[p0] && pm[p1] {
                    mask[i] = true;
                    any = true;
                }
            }
        }
        if any {
            return Ok(mask);
        }
    }

    let prim_key = format!("group_{pattern}");
    if let Some(prim_attr) = g.prim.get(&prim_key) {
        let (pr, _, _) = model::corners(g);
        let mut any = false;
        for v in 0..g.nvertices() {
            let prim_idx = pr[v];
            let in_group = match &prim_attr.data {
                AttrData::I32(d) => d[prim_idx] != 0,
                AttrData::F32(d) => d[prim_idx] != 0.0,
            };
            if in_group {
                let edge_id = e.of_vertex[v];
                if edge_id >= 0 && e.count[edge_id as usize] == 1 {
                    mask[edge_id as usize] = true;
                    any = true;
                }
            }
        }
        if any {
            return Ok(mask);
        }
    }

    Err(Error::new(format!("cannot resolve boundary group {pattern:?}")))
}

fn extract_loops(g: &Geometry, boundary_mask: &[bool], auto_complete: bool) -> Result<Vec<BoundaryLoop>> {
    let e = topo::edges(&g.topology, g.npoints);
    let (_, nx, _) = model::corners(g);
    let vp = &g.topology.vertex_point;

    let mut succ: BTreeMap<usize, (usize, usize)> = BTreeMap::new();
    let mut indeg: BTreeMap<usize, usize> = BTreeMap::new();
    let mut all_succ: BTreeMap<usize, (usize, usize)> = BTreeMap::new();
    let mut starts = Vec::new();

    for v in 0..g.nvertices() {
        let edge_id = e.of_vertex[v];
        if edge_id >= 0 && e.count[edge_id as usize] == 1 {
            let a = vp[v] as usize;
            let b = vp[nx[v]] as usize;
            all_succ.insert(b, (a, v));

            if boundary_mask[edge_id as usize] {
                succ.insert(b, (a, v));
                *indeg.entry(a).or_insert(0) += 1;
                indeg.entry(b).or_insert(0);
                starts.push(b);
            }
        }
    }

    let mut loops = Vec::new();
    let mut visited_edges: BTreeSet<(usize, usize)> = BTreeSet::new();

    let open_starts: Vec<usize> = succ
        .keys()
        .copied()
        .filter(|&k| indeg.get(&k).copied().unwrap_or(0) == 0)
        .collect();

    for start in open_starts {
        let mut cur = start;
        let mut pts = Vec::new();
        let mut corners = Vec::new();
        let mut chain_visited = BTreeSet::new();

        while let Some(&(next, corner)) = succ.get(&cur) {
            if !chain_visited.insert((cur, next)) {
                break;
            }
            visited_edges.insert((cur, next));
            pts.push(cur);
            corners.push(corner);
            cur = next;
        }

        if auto_complete {
            let chain_end = cur;
            let mut completed = false;
            let mut walk = chain_end;
            let mut auto_pts = Vec::new();
            let mut auto_corners = Vec::new();
            let mut auto_visited = BTreeSet::new();

            while let Some(&(next, corner)) = all_succ.get(&walk) {
                if !auto_visited.insert((walk, next)) {
                    break;
                }
                auto_pts.push(walk);
                auto_corners.push(corner);
                walk = next;
                if walk == start {
                    pts.extend(auto_pts);
                    corners.extend(auto_corners);
                    completed = true;
                    break;
                }
            }

            if completed {
                if pts.len() >= 3 {
                    loops.push(BoundaryLoop {
                        pts,
                        corners,
                        is_open: true,
                    });
                }
            } else {
                if pts.len() >= 2 {
                    pts.push(chain_end);
                    corners.push(corners.last().copied().unwrap_or(0));
                    loops.push(BoundaryLoop {
                        pts,
                        corners,
                        is_open: true,
                    });
                }
            }
        }
    }

    for &start in starts.iter().chain(succ.keys()) {
        if visited_edges.contains(&(start, succ[&start].0)) {
            continue;
        }
        let mut cur = start;
        let mut pts = Vec::new();
        let mut corners = Vec::new();
        let mut cycle = false;

        loop {
            let Some(&(next, corner)) = succ.get(&cur) else {
                break;
            };
            if visited_edges.contains(&(cur, next)) {
                break;
            }
            visited_edges.insert((cur, next));
            pts.push(cur);
            corners.push(corner);
            cur = next;
            if cur == start {
                cycle = true;
                break;
            }
        }

        if cycle && pts.len() >= 3 {
            loops.push(BoundaryLoop {
                pts,
                corners,
                is_open: false,
            });
        }
    }

    Ok(loops)
}

fn ear_clip_triangles(pts: &[usize], pos: &[[f64; 3]], n: [f64; 3]) -> Vec<Vec<usize>> {
    let count = pts.len();
    if count < 3 {
        return Vec::new();
    }
    if count == 3 {
        return vec![pts.to_vec()];
    }

    let mut axis = 2;
    if n[0].abs() > n[1].abs() && n[0].abs() > n[2].abs() {
        axis = 0;
    } else if n[1].abs() > n[2].abs() {
        axis = 1;
    }
    let (ax, ay) = match axis {
        0 => (1, 2),
        1 => (0, 2),
        _ => (0, 1),
    };

    let p2d: Vec<[f64; 2]> = pts.iter().map(|&i| [pos[i][ax], pos[i][ay]]).collect();
    let mut area2d = 0.0;
    for i in 0..count {
        let (p0, p1) = (p2d[i], p2d[(i + 1) % count]);
        area2d += p0[0] * p1[1] - p1[0] * p0[1];
    }
    let sign = if area2d >= 0.0 { 1.0 } else { -1.0 };

    let turn = |a: [f64; 2], b: [f64; 2], c: [f64; 2]| -> f64 {
        sign * ((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]))
    };

    let mut remaining: Vec<usize> = (0..count).collect();
    let mut tris = Vec::new();

    while remaining.len() > 3 {
        let r_len = remaining.len();
        let mut clipped = false;

        for k in 0..r_len {
            let prev = remaining[(k + r_len - 1) % r_len];
            let curr = remaining[k];
            let next = remaining[(k + 1) % r_len];

            if turn(p2d[prev], p2d[curr], p2d[next]) <= 1e-12 {
                continue;
            }

            let mut inside = false;
            for &j in &remaining {
                if j != prev && j != curr && j != next {
                    let p = p2d[j];
                    if turn(p2d[prev], p2d[curr], p) >= -1e-12
                        && turn(p2d[curr], p2d[next], p) >= -1e-12
                        && turn(p2d[next], p2d[prev], p) >= -1e-12
                    {
                        inside = true;
                        break;
                    }
                }
            }
            if !inside {
                tris.push(vec![pts[prev], pts[curr], pts[next]]);
                remaining.remove(k);
                clipped = true;
                break;
            }
        }

        if !clipped {
            let mut best_k = 0;
            let mut best_turn = -1e30;
            for k in 0..r_len {
                let prev = remaining[(k + r_len - 1) % r_len];
                let curr = remaining[k];
                let next = remaining[(k + 1) % r_len];
                let t = turn(p2d[prev], p2d[curr], p2d[next]);
                if t > best_turn {
                    best_turn = t;
                    best_k = k;
                }
            }
            let prev = remaining[(best_k + r_len - 1) % r_len];
            let curr = remaining[best_k];
            let next = remaining[(best_k + 1) % r_len];
            tris.push(vec![pts[prev], pts[curr], pts[next]]);
            remaining.remove(best_k);
        }
    }

    if remaining.len() == 3 {
        tris.push(vec![pts[remaining[0]], pts[remaining[1]], pts[remaining[2]]]);
    }
    tris
}

fn auto_layout(pos: &[[f64; 3]]) -> (usize, usize) {
    let n = pos.len();
    if n < 4 {
        return (1, 0);
    }
    let ang: Vec<f64> = (0..n)
        .map(|i| angle(pos[(i + n - 1) % n], pos[i], pos[(i + 1) % n]))
        .collect();
    let arc = |from: usize, len: usize| -> f64 {
        (0..len)
            .map(|k| dist(pos[(from + k) % n], pos[(from + k + 1) % n]))
            .sum::<f64>()
            .max(1e-12)
    };
    let mut best = (f64::INFINITY, 1, 0);
    for s in 1..n / 2 {
        let m = n / 2 - s;
        for o in 0..n {
            let corners = [o, (o + s) % n, (o + s + m) % n, (o + 2 * s + m) % n];
            let mut score = 0.;
            for (i, a) in ang.iter().enumerate() {
                score += (a
                    - if corners.contains(&i) {
                        std::f64::consts::FRAC_PI_2
                    } else {
                        std::f64::consts::PI
                    })
                .abs();
            }
            score += 0.5
                * ((arc(o, s) / arc((o + s) % n, m)).ln().abs()
                    + (arc((o + s + m) % n, s) / arc((o + 2 * s + m) % n, m))
                        .ln()
                        .abs());
            if score < best.0 - 1e-9 {
                best = (score, s, o);
            }
        }
    }
    (best.1, best.2)
}

fn params_along(c: &[[f64; 3]]) -> Vec<f64> {
    let mut a = vec![0.];
    for k in 1..c.len() {
        a.push(a[k - 1] + dist(c[k - 1], c[k]));
    }
    let tot = a.last().copied().unwrap_or(1.0).max(1e-12);
    a.iter().map(|x| x / tot).collect()
}

pub(crate) fn polyfill(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = POLYFILL.one(inputs, 0)?;
    model::polygons(&g, "polyfill")?;

    let boundary_group = get_str_opt(p, &["boundary_group", "group"]);
    let fill_group = get_str_opt(p, &["fill_group"]);
    let patch_group = {
        let s = get_str_opt(p, &["patch_group", "patchgroup"]);
        if s.is_empty() && fill_group.is_empty() {
            "patch".to_string()
        } else if !s.is_empty() {
            s
        } else {
            String::new()
        }
    };
    let edge_loop_group = get_str(p, &["edge_loop_group", "edgeloopgroup"], "loopgroup");
    let append_to_groups = get_bool(p, &["append_to_groups", "append_groups"], false);

    let mode = get_mode(p);
    let auto_complete = get_bool(p, &["auto_complete_boundaries", "autocomplete"], true);
    let reverse_patches = get_bool(p, &["reverse_patches", "reverse"], false);
    let unique_points = get_bool(p, &["unique_points", "uniquepoints"], false);
    let update_point_normals = get_bool(p, &["update_point_normals", "updatenormals"], true);
    let edge_loop = get_f64(p, &["edge_loop", "edgeloop"], 0.25).clamp(0.0, 1.0);
    let smooth_raw = get_f64(p, &["smooth"], 50.0);
    let smooth = if let Ok(b) = p.b("smooth") {
        if !b {
            0.0
        } else if (smooth_raw - 1.0).abs() < 1e-9 {
            50.0
        } else {
            smooth_raw
        }
    } else {
        smooth_raw
    }
    .max(0.0);

    let corner_offset = get_i64(p, &["corner_offset"], 0);
    let custom_corners = get_str_opt(p, &["custom_corners"]);
    let deform_patch = get_bool(p, &["deform_patch", "deform"], true);
    let surface_offset = get_f64(p, &["surface_offset"], 0.0);
    let tangent_strength = get_f64(p, &["tangent_strength"], 0.4);

    let boundary_mask = get_boundary_mask(&g, &boundary_group)?;
    let loops = extract_loops(&g, &boundary_mask, auto_complete)?;
    if loops.is_empty() {
        return Ok((*g).clone());
    }

    let pos = model::positions(&g);
    let custom_mask = if !custom_corners.is_empty() {
        groups::point_mask(&g, &custom_corners).ok()
    } else {
        None
    };

    let mut new_pts: Vec<NewPoint> = Vec::new();
    let mut loop_faces: Vec<Vec<usize>> = Vec::new();
    let mut patch_faces: Vec<Vec<usize>> = Vec::new();
    let mut all_srcs: Vec<Vec<Vec<usize>>> = Vec::new();

    let mut interior_pt_indices = BTreeSet::new();

    for l in &loops {
        let n = l.pts.len();
        if n < 3 {
            continue;
        }

        let mut pts = l.pts.clone();
        let mut corners = l.corners.clone();

        let mut start_offset = corner_offset.rem_euclid(n as i64) as usize;
        if let Some(cm) = &custom_mask {
            if let Some(idx) = pts.iter().position(|&pt| cm[pt]) {
                start_offset = idx;
            }
        }
        if start_offset != 0 {
            pts.rotate_left(start_offset);
            corners.rotate_left(start_offset);
        }

        let loop_coords: Vec<[f64; 3]> = pts.iter().map(|&i| pos[i]).collect();
        let norm = newell_normal(&loop_coords);

        let mut center = [0.0; 3];
        for &pt_pos in &loop_coords {
            center = add(center, pt_pos);
        }
        center = mul(center, 1.0 / n as f64);

        let mut active_pts = pts.clone();
        let mut active_corners = corners.clone();

        if l.is_open && edge_loop > 0.0 {
            let mut q_indices = Vec::new();
            for i in 0..n {
                let p_pos = loop_coords[i];
                let q_pos = add(mul(p_pos, 1.0 - edge_loop), mul(center, edge_loop));
                new_pts.push(NewPoint {
                    pos: q_pos,
                    src: vec![(pts[i], 1.0 - edge_loop), (pts[(i + 1) % n], edge_loop)],
                });
                let q_idx = g.npoints + new_pts.len() - 1;
                q_indices.push(q_idx);
                interior_pt_indices.insert(q_idx);
            }

            for i in 0..n {
                let j = (i + 1) % n;
                let quad = vec![active_pts[i], active_pts[j], q_indices[j], q_indices[i]];
                let quad_src = vec![
                    vec![active_corners[i]],
                    vec![active_corners[j]],
                    vec![active_corners[j]],
                    vec![active_corners[i]],
                ];
                loop_faces.push(quad);
                all_srcs.push(quad_src);
            }

            active_pts = q_indices;
            active_corners = vec![active_corners[0]; n];
        }

        let m = active_pts.len();
        let active_coords: Vec<[f64; 3]> = active_pts
            .iter()
            .map(|&idx| {
                if idx < g.npoints {
                    pos[idx]
                } else {
                    new_pts[idx - g.npoints].pos
                }
            })
            .collect();

        let mut active_center = [0.0; 3];
        for &pt_pos in &active_coords {
            active_center = add(active_center, pt_pos);
        }
        active_center = mul(active_center, 1.0 / m as f64);

        let inner_norm = newell_normal(&active_coords);

        match mode.as_str() {
            "single" | "single_polygon" => {
                let cnt = active_pts.len();
                patch_faces.push(active_pts.clone());
                all_srcs.push(vec![vec![]; cnt]);
            }
            "triangles" => {
                let tris = ear_clip_triangles(&active_pts, &pos, inner_norm);
                for t in tris {
                    let t_src = t
                        .iter()
                        .map(|&p_idx| {
                            active_pts
                                .iter()
                                .position(|&ap| ap == p_idx)
                                .map(|pos_idx| vec![active_corners[pos_idx]])
                                .unwrap_or_default()
                        })
                        .collect();
                    patch_faces.push(t);
                    all_srcs.push(t_src);
                }
            }
            "triangle_fan" | "trifan" => {
                new_pts.push(NewPoint {
                    pos: active_center,
                    src: active_pts
                        .iter()
                        .filter(|&&idx| idx < g.npoints)
                        .map(|&idx| (idx, 1.0 / m as f64))
                        .collect(),
                });
                let c_idx = g.npoints + new_pts.len() - 1;
                interior_pt_indices.insert(c_idx);

                for i in 0..m {
                    let j = (i + 1) % m;
                    patch_faces.push(vec![active_pts[i], active_pts[j], c_idx]);
                    all_srcs.push(vec![
                        vec![active_corners[i]],
                        vec![active_corners[j]],
                        vec![active_corners[i]],
                    ]);
                }
            }
            "quadrilateral_fan" | "quadfan" => {
                new_pts.push(NewPoint {
                    pos: active_center,
                    src: active_pts
                        .iter()
                        .filter(|&&idx| idx < g.npoints)
                        .map(|&idx| (idx, 1.0 / m as f64))
                        .collect(),
                });
                let c_idx = g.npoints + new_pts.len() - 1;
                interior_pt_indices.insert(c_idx);

                let pairs = m / 2;
                for k in 0..pairs {
                    let i0 = 2 * k;
                    let i1 = 2 * k + 1;
                    let i2 = (2 * k + 2) % m;
                    if k == pairs - 1 && m % 2 != 0 {
                        patch_faces.push(vec![
                            active_pts[i0],
                            active_pts[i1],
                            active_pts[i2],
                            active_pts[(2 * k + 3) % m],
                            c_idx,
                        ]);
                        all_srcs.push(vec![
                            vec![active_corners[i0]],
                            vec![active_corners[i1]],
                            vec![active_corners[i2]],
                            vec![active_corners[(2 * k + 3) % m]],
                            vec![active_corners[i0]],
                        ]);
                    } else {
                        patch_faces.push(vec![active_pts[i0], active_pts[i1], active_pts[i2], c_idx]);
                        all_srcs.push(vec![
                            vec![active_corners[i0]],
                            vec![active_corners[i1]],
                            vec![active_corners[i2]],
                            vec![active_corners[i0]],
                        ]);
                    }
                }
            }
            "quadrilateral_grid" | "grid" if m >= 4 && m % 2 == 0 => {
                let (s, o) = auto_layout(&active_coords);
                let k_span = s;
                let m_span = m / 2 - k_span;
                let at = |k: usize| (o + k) % m;
                let bidx = |i: usize, j: usize| -> Option<usize> {
                    if j == 0 {
                        Some(at(i))
                    } else if i == k_span {
                        Some(at(k_span + j))
                    } else if j == m_span {
                        Some(at(k_span + m_span + (k_span - i)))
                    } else if i == 0 {
                        Some(at(2 * k_span + m_span + (m_span - j)))
                    } else {
                        None
                    }
                };

                let b: Vec<[f64; 3]> = (0..=k_span).map(|i| active_coords[at(i)]).collect();
                let r_edge: Vec<[f64; 3]> = (0..=m_span).map(|j| active_coords[at(k_span + j)]).collect();
                let t: Vec<[f64; 3]> = (0..=k_span)
                    .map(|i| active_coords[at(k_span + m_span + (k_span - i))])
                    .collect();
                let lf: Vec<[f64; 3]> = (0..=m_span)
                    .map(|j| active_coords[at(2 * k_span + m_span + (m_span - j))])
                    .collect();
                let (ub, ut, vr, vl) = (
                    params_along(&b),
                    params_along(&t),
                    params_along(&r_edge),
                    params_along(&lf),
                );
                let cs = [at(0), at(k_span), at(k_span + m_span), at(2 * k_span + m_span)];
                let (c00, c10, c11, c01) = (
                    active_coords[cs[0]],
                    active_coords[cs[1]],
                    active_coords[cs[2]],
                    active_coords[cs[3]],
                );

                let mut grid = vec![vec![usize::MAX; m_span + 1]; k_span + 1];
                let mut gsrc: Vec<Vec<Vec<usize>>> = vec![vec![vec![]; m_span + 1]; k_span + 1];

                for i in 0..=k_span {
                    for j in 0..=m_span {
                        if let Some(bi) = bidx(i, j) {
                            grid[i][j] = active_pts[bi];
                            gsrc[i][j] = vec![active_corners[bi]];
                            continue;
                        }
                        let (u0, v0) = (i as f64 / k_span as f64, j as f64 / m_span as f64);
                        let (u, v) = ((1. - v0) * ub[i] + v0 * ut[i], (1. - u0) * vl[j] + u0 * vr[j]);
                        let mut q = [0.; 3];
                        for c in 0..3 {
                            q[c] = (1. - v) * b[i][c] + v * t[i][c] + (1. - u) * lf[j][c] + u * r_edge[j][c]
                                - ((1. - u) * (1. - v) * c00[c]
                                    + u * (1. - v) * c10[c]
                                    + u * v * c11[c]
                                    + (1. - u) * v * c01[c]);
                        }
                        let w = [(1. - u) * (1. - v), u * (1. - v), u * v, (1. - u) * v];
                        new_pts.push(NewPoint {
                            pos: q,
                            src: cs.iter().zip(w).filter(|&(&k, _)| active_pts[k] < g.npoints).map(|(&k, w)| (active_pts[k], w)).collect(),
                        });
                        let pt_idx = g.npoints + new_pts.len() - 1;
                        grid[i][j] = pt_idx;
                        interior_pt_indices.insert(pt_idx);
                        gsrc[i][j] = cs.iter().map(|&k| active_corners[k]).collect();
                    }
                }
                for i in 0..k_span {
                    for j in 0..m_span {
                        patch_faces.push(vec![
                            grid[i][j],
                            grid[i + 1][j],
                            grid[i + 1][j + 1],
                            grid[i][j + 1],
                        ]);
                        all_srcs.push(vec![
                            gsrc[i][j].clone(),
                            gsrc[i + 1][j].clone(),
                            gsrc[i + 1][j + 1].clone(),
                            gsrc[i][j + 1].clone(),
                        ]);
                    }
                }
            }
            _ => {
                // Default: "quadrilaterals"
                if m == 4 {
                    patch_faces.push(active_pts.clone());
                    all_srcs.push(vec![vec![]; 4]);
                } else if m % 2 == 0 {
                    new_pts.push(NewPoint {
                        pos: active_center,
                        src: active_pts
                            .iter()
                            .filter(|&&idx| idx < g.npoints)
                            .map(|&idx| (idx, 1.0 / m as f64))
                            .collect(),
                    });
                    let c_idx = g.npoints + new_pts.len() - 1;
                    interior_pt_indices.insert(c_idx);

                    let pairs = m / 2;
                    for k in 0..pairs {
                        let i0 = 2 * k;
                        let i1 = 2 * k + 1;
                        let i2 = (2 * k + 2) % m;
                        patch_faces.push(vec![active_pts[i0], active_pts[i1], active_pts[i2], c_idx]);
                        all_srcs.push(vec![
                            vec![active_corners[i0]],
                            vec![active_corners[i1]],
                            vec![active_corners[i2]],
                            vec![active_corners[i0]],
                        ]);
                    }
                } else {
                    // Odd m: Catmull-Clark quad subdivision
                    new_pts.push(NewPoint {
                        pos: active_center,
                        src: active_pts
                            .iter()
                            .filter(|&&idx| idx < g.npoints)
                            .map(|&idx| (idx, 1.0 / m as f64))
                            .collect(),
                    });
                    let c_idx = g.npoints + new_pts.len() - 1;
                    interior_pt_indices.insert(c_idx);

                    let mut mid_indices = Vec::new();
                    for i in 0..m {
                        let j = (i + 1) % m;
                        let mid_pos = mul(add(active_coords[i], active_coords[j]), 0.5);
                        let mut src = Vec::new();
                        if active_pts[i] < g.npoints {
                            src.push((active_pts[i], 0.5));
                        }
                        if active_pts[j] < g.npoints {
                            src.push((active_pts[j], 0.5));
                        }
                        new_pts.push(NewPoint {
                            pos: mid_pos,
                            src,
                        });
                        let m_idx = g.npoints + new_pts.len() - 1;
                        mid_indices.push(m_idx);
                        interior_pt_indices.insert(m_idx);
                    }

                    for i in 0..m {
                        let prev_m = mid_indices[(i + m - 1) % m];
                        let curr_m = mid_indices[i];
                        patch_faces.push(vec![active_pts[i], curr_m, c_idx, prev_m]);
                        all_srcs.push(vec![
                            vec![active_corners[i]],
                            vec![active_corners[i]],
                            vec![active_corners[i]],
                            vec![active_corners[(i + m - 1) % m]],
                        ]);
                    }
                }
            }
        }

        // Deform patch (Surface Offset & Tangent Strength)
        if deform_patch {
            let mut avg_tangent = [0.0; 3];
            for i in 0..m {
                let diff = sub(active_center, active_coords[i]);
                let l = len(diff);
                if l > 1e-12 {
                    avg_tangent = add(avg_tangent, mul(diff, 1.0 / l));
                }
            }
            avg_tangent = normalize(avg_tangent);

            for &pt_idx in &interior_pt_indices {
                if pt_idx >= g.npoints {
                    let np_idx = pt_idx - g.npoints;
                    let p_pos = new_pts[np_idx].pos;
                    let d = dist(p_pos, active_center);
                    let max_d = active_coords
                        .iter()
                        .map(|&c| dist(c, active_center))
                        .fold(0.0, f64::max)
                        .max(1e-12);
                    let weight = (1.0 - (d / max_d).clamp(0.0, 1.0)).max(0.1);

                    if surface_offset != 0.0 {
                        new_pts[np_idx].pos = add(
                            new_pts[np_idx].pos,
                            mul(norm, surface_offset * weight),
                        );
                    }
                    if tangent_strength != 0.0 {
                        new_pts[np_idx].pos = add(
                            new_pts[np_idx].pos,
                            mul(avg_tangent, tangent_strength * weight * 0.5),
                        );
                    }
                }
            }
        }
    }

    // Laplacian smoothing of interior points
    if smooth > 0.0 && !interior_pt_indices.is_empty() {
        let iters = (smooth.round() as usize).clamp(1, 100);
        let mut adj: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
        for f in loop_faces.iter().chain(patch_faces.iter()) {
            let flen = f.len();
            for i in 0..flen {
                let (u, v) = (f[i], f[(i + 1) % flen]);
                adj.entry(u).or_default().insert(v);
                adj.entry(v).or_default().insert(u);
            }
        }

        let get_pos = |idx: usize, pts_list: &[NewPoint]| -> [f64; 3] {
            if idx < g.npoints {
                pos[idx]
            } else {
                pts_list[idx - g.npoints].pos
            }
        };

        for _ in 0..iters {
            let mut updated_pos: Vec<(usize, [f64; 3])> = Vec::new();
            for &pt_idx in &interior_pt_indices {
                if let Some(neighbors) = adj.get(&pt_idx) {
                    if !neighbors.is_empty() {
                        let mut sum_p = [0.0; 3];
                        for &nbr in neighbors {
                            sum_p = add(sum_p, get_pos(nbr, &new_pts));
                        }
                        let avg_p = mul(sum_p, 1.0 / neighbors.len() as f64);
                        let cur_p = get_pos(pt_idx, &new_pts);
                        let next_p = add(mul(cur_p, 0.5), mul(avg_p, 0.5));
                        updated_pos.push((pt_idx, next_p));
                    }
                }
            }
            for (pt_idx, p_val) in updated_pos {
                if pt_idx >= g.npoints {
                    new_pts[pt_idx - g.npoints].pos = p_val;
                }
            }
        }
    }

    // Combine faces
    let num_loop_faces = loop_faces.len();
    let mut all_faces: Vec<Vec<usize>> = loop_faces;
    all_faces.extend(patch_faces);

    // Unique Points: duplicate boundary points referenced by new faces
    if unique_points {
        let mut boundary_remap: BTreeMap<usize, usize> = BTreeMap::new();
        for f in &mut all_faces {
            for v in f.iter_mut() {
                if *v < g.npoints {
                    let old_pt = *v;
                    let new_idx = *boundary_remap.entry(old_pt).or_insert_with(|| {
                        new_pts.push(NewPoint {
                            pos: pos[old_pt],
                            src: vec![(old_pt, 1.0)],
                        });
                        g.npoints + new_pts.len() - 1
                    });
                    *v = new_idx;
                }
            }
        }
    }

    // Reverse Patches
    if reverse_patches {
        for f in &mut all_faces {
            f.reverse();
        }
        for s in &mut all_srcs {
            s.reverse();
        }
    }

    // Append to geometry
    let mut out = append(&g, &new_pts, &all_faces, &all_srcs)?;

    // Update Point Normals
    if update_point_normals && out.point.contains_key("N") {
        let p_slice = out.p();
        let norms = crate::normals::point_normals(p_slice, &out.topology, out.npoints);
        out.point.insert("N".into(), Arc::new(Attr::f32(3, norms)));
    }

    // Write Output Groups
    let total_new_prims = all_faces.len();
    let old_nprims = g.nprims();
    let new_nprims = out.nprims();

    let write_group_with_append = |geo: &mut Geometry, name: &str, is_member: Box<dyn Fn(usize) -> bool>| -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            return Ok(());
        }
        let mut mask = vec![false; new_nprims];
        for i in 0..total_new_prims {
            if is_member(i) {
                mask[old_nprims + i] = true;
            }
        }
        if append_to_groups {
            let key = format!("group_{name}");
            if let Some(existing) = geo.prim.get(&key) {
                if let Some(vals) = existing.as_i32() {
                    for (m, &v) in mask.iter_mut().zip(vals) {
                        *m |= v != 0;
                    }
                }
            }
        }
        model::write_group(geo, "prim", name, &mask)
    };

    if !patch_group.is_empty() {
        write_group_with_append(&mut out, &patch_group, Box::new(|i| i >= num_loop_faces))?;
    }
    if !edge_loop_group.is_empty() && num_loop_faces > 0 {
        write_group_with_append(&mut out, &edge_loop_group, Box::new(|i| i < num_loop_faces))?;
    }
    if !fill_group.is_empty() {
        write_group_with_append(&mut out, &fill_group, Box::new(|_| true))?;
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cook(g: Geometry, options: serde_json::Value) -> Result<Geometry> {
        let params = Params::resolve(PARAMS, options.as_object())?;
        polyfill(&[vec![Arc::new(g)]], &params)
    }

    /// Cube with top face removed (face 5): 8 vertices, 5 quad faces, leaving 1 square boundary loop (points 4, 5, 6, 7).
    fn cube_missing_face() -> Geometry {
        let mut g = Geometry::from_arrays(
            vec![
                -1., -1., -1., 1., -1., -1., 1., 1., -1., -1., 1., -1.,
                -1., -1., 1., 1., -1., 1., 1., 1., 1., -1., 1., 1.,
            ],
            vec![
                0, 3, 2, 1,
                0, 1, 5, 4,
                1, 2, 6, 5,
                2, 3, 7, 6,
                3, 0, 4, 7,
            ],
            vec![0, 4, 8, 12, 16, 20],
            None,
        )
        .unwrap();
        g.vertex.insert(
            "uv".into(),
            Arc::new(Attr::f32(
                2,
                (0..5).flat_map(|_| [0., 0., 1., 0., 1., 1., 0., 1.]).collect(),
            )),
        );
        g.prim.insert("material_index".into(), Arc::new(Attr::i32(1, vec![1; 5])));
        g
    }

    /// Cylinder/ring of `n` quads around Z, open at z=0 (points 0..n) and z=1 (points n..2n).
    fn cylinder_ring(n: usize) -> Geometry {
        let mut p = Vec::new();
        for z in [0.0, 1.0] {
            for i in 0..n {
                let a = i as f64 / n as f64 * std::f64::consts::TAU;
                p.extend([a.cos() as f32, a.sin() as f32, z as f32]);
            }
        }
        let mut vp = Vec::new();
        for i in 0..n {
            let j = (i + 1) % n;
            vp.extend([i, j, n + j, n + i].map(|x| x as i32));
        }
        Geometry::from_arrays(p, vp, (0..=n as i32).map(|i| i * 4).collect(), None).unwrap()
    }

    /// Open quad strip of 2 quads: vertices 0, 1, 2 at y=0, vertices 3, 4, 5 at y=1.
    fn open_quad_strip() -> Geometry {
        Geometry::from_arrays(
            vec![
                0., 0., 0.,  1., 0., 0.,  2., 0., 0.,
                0., 1., 0.,  1., 1., 0.,  2., 1., 0.,
            ],
            vec![0, 1, 4, 3,  1, 2, 5, 4],
            vec![0, 4, 8],
            None,
        )
        .unwrap()
    }

    #[test]
    fn test_closed_boundary_fills() {
        let g = cube_missing_face();
        assert_eq!(g.nprims(), 5);
        let out = cook(g, json!({})).unwrap();
        assert_eq!(out.nprims(), 6);
        assert!(out.p().iter().all(|x| x.is_finite()));
    }

    #[test]
    fn test_existing_graphs_still_cook() {
        let g = cube_missing_face();
        let out = cook(g, json!({})).unwrap();
        assert_eq!(out.nprims(), 6);
        let cap = out.nprims() - 1;
        assert_eq!(out.prim["material_index"].as_i32().unwrap()[cap], 0);
        let start = out.topology.prim_offsets[cap] as usize * 2;
        assert!(out.vertex["uv"].as_f32().unwrap()[start..].iter().all(|v| v.is_nan()));
    }

    #[test]
    fn test_fill_group_param_backward_compat() {
        let g = cube_missing_face();
        let out = cook(g, json!({"fill_group": "my_cap"})).unwrap();
        assert!(out.prim.contains_key("group_my_cap"));
        let vals = out.prim["group_my_cap"].as_i32().unwrap();
        assert_eq!(vals.len(), 6);
        assert_eq!(vals[..5], [0, 0, 0, 0, 0]);
        assert_eq!(vals[5], 1);
    }

    #[test]
    fn test_boundary_group() {
        let g = cylinder_ring(6);
        assert_eq!(g.nprims(), 6);
        let out_both = cook(g.clone(), json!({"mode": "single_polygon", "edge_loop": 0.0})).unwrap();
        assert_eq!(out_both.nprims(), 8);

        let out_single = cook(g, json!({"boundary_group": "p0-5", "mode": "single_polygon", "edge_loop": 0.0})).unwrap();
        assert_eq!(out_single.nprims(), 7);
    }

    #[test]
    fn test_fill_modes_on_hex_hole() {
        let g = cylinder_ring(6);
        let out_single = cook(g.clone(), json!({"mode": "single_polygon"})).unwrap();
        assert_eq!(out_single.nprims(), 8);

        let out_tris = cook(g.clone(), json!({"mode": "triangles"})).unwrap();
        assert_eq!(out_tris.nprims(), 6 + 4 * 2);

        let out_trifan = cook(g.clone(), json!({"mode": "triangle_fan"})).unwrap();
        assert_eq!(out_trifan.nprims(), 6 + 6 * 2);

        let out_quadfan = cook(g.clone(), json!({"mode": "quadrilateral_fan"})).unwrap();
        assert_eq!(out_quadfan.nprims(), 6 + 3 * 2);

        let out_quads = cook(g, json!({"mode": "quadrilaterals"})).unwrap();
        assert_eq!(out_quads.nprims(), 6 + 3 * 2);
    }

    #[test]
    fn test_fill_mode_quadrilateral_grid() {
        let g = cylinder_ring(8);
        let out = cook(g, json!({"mode": "quadrilateral_grid", "boundary_group": "p0-7"})).unwrap();
        assert!(out.nprims() > 8);
        assert!(out.p().iter().all(|x| x.is_finite()));
    }

    #[test]
    fn test_auto_complete_boundaries() {
        let g = open_quad_strip();
        let out_on = cook(g.clone(), json!({"boundary_group": "p0-1, p1-2, p2-5", "auto_complete_boundaries": true})).unwrap();
        assert!(out_on.nprims() > g.nprims());

        let out_off = cook(g.clone(), json!({"boundary_group": "p0-1, p1-2, p2-5", "auto_complete_boundaries": false})).unwrap();
        assert_eq!(out_off.nprims(), g.nprims());
    }

    #[test]
    fn test_reverse_patches() {
        let g = cube_missing_face();
        let out_norm = cook(g.clone(), json!({"reverse_patches": false, "mode": "single_polygon"})).unwrap();
        let out_rev = cook(g, json!({"reverse_patches": true, "mode": "single_polygon"})).unwrap();

        let cap_norm: Vec<i32> = out_norm.topology.vertex_point[out_norm.topology.prim_offsets[5] as usize..].to_vec();
        let cap_rev: Vec<i32> = out_rev.topology.vertex_point[out_rev.topology.prim_offsets[5] as usize..].to_vec();
        let mut expected_rev = cap_norm;
        expected_rev.reverse();
        assert_eq!(cap_rev, expected_rev);
    }

    #[test]
    fn test_unique_points() {
        let g = cube_missing_face();
        let out_shared = cook(g.clone(), json!({"unique_points": false})).unwrap();
        let out_unique = cook(g.clone(), json!({"unique_points": true})).unwrap();

        assert_eq!(out_shared.npoints, g.npoints);
        assert!(out_unique.npoints > g.npoints);
        let cap_unique: Vec<i32> = out_unique.topology.vertex_point[out_unique.topology.prim_offsets[5] as usize..].to_vec();
        for &v in &cap_unique {
            assert!(v >= g.npoints as i32);
        }
    }

    #[test]
    fn test_update_point_normals() {
        let mut g = cube_missing_face();
        g.point.insert("N".into(), Arc::new(Attr::f32(3, vec![0.0; g.npoints * 3])));

        let out_updated = cook(g.clone(), json!({"update_point_normals": true})).unwrap();
        let n_updated = out_updated.point["N"].as_f32().unwrap();
        assert!(n_updated.iter().any(|&x| x.abs() > 0.01));

        let out_kept = cook(g, json!({"update_point_normals": false})).unwrap();
        let n_kept = out_kept.point["N"].as_f32().unwrap();
        assert!(n_kept.iter().all(|&x| x == 0.0));
    }

    #[test]
    fn test_edge_loop_on_open_boundary() {
        let g = open_quad_strip();
        let out_01 = cook(g.clone(), json!({"boundary_group": "p0-1, p1-2, p2-5", "edge_loop": 0.1, "auto_complete_boundaries": true})).unwrap();
        let out_05 = cook(g.clone(), json!({"boundary_group": "p0-1, p1-2, p2-5", "edge_loop": 0.5, "auto_complete_boundaries": true})).unwrap();

        assert!(out_01.prim.contains_key("group_loopgroup"));
        assert!(out_05.prim.contains_key("group_loopgroup"));
        assert_ne!(out_01.p(), out_05.p());
    }

    #[test]
    fn test_smooth() {
        let g = cylinder_ring(6);
        let out_unsmoothed = cook(g.clone(), json!({"mode": "triangle_fan", "smooth": 0.0, "deform_patch": false})).unwrap();
        let out_smoothed = cook(g, json!({"mode": "triangle_fan", "smooth": 50.0, "deform_patch": false})).unwrap();

        assert_eq!(out_unsmoothed.npoints, out_smoothed.npoints);
        assert!(out_unsmoothed.p().iter().all(|x| x.is_finite()));
        assert!(out_smoothed.p().iter().all(|x| x.is_finite()));
    }

    #[test]
    fn test_corner_offset() {
        let g = cylinder_ring(6);
        let out_0 = cook(g.clone(), json!({"mode": "quadrilateral_fan", "corner_offset": 0, "boundary_group": "p0-5"})).unwrap();
        let out_1 = cook(g, json!({"mode": "quadrilateral_fan", "corner_offset": 1, "boundary_group": "p0-5"})).unwrap();

        let cap_0: Vec<i32> = out_0.topology.vertex_point[out_0.topology.prim_offsets[6] as usize..out_0.topology.prim_offsets[7] as usize].to_vec();
        let cap_1: Vec<i32> = out_1.topology.vertex_point[out_1.topology.prim_offsets[6] as usize..out_1.topology.prim_offsets[7] as usize].to_vec();
        assert_ne!(cap_0, cap_1);
    }

    #[test]
    fn test_custom_corners() {
        let mut g = cylinder_ring(6);
        g.point.insert("group_mycorner".into(), Arc::new(Attr::i32(1, vec![0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0])));
        let out_custom = cook(g, json!({"mode": "quadrilateral_fan", "custom_corners": "mycorner", "boundary_group": "p0-5"})).unwrap();
        let cap_custom: Vec<i32> = out_custom.topology.vertex_point[out_custom.topology.prim_offsets[6] as usize..out_custom.topology.prim_offsets[7] as usize].to_vec();
        assert_eq!(cap_custom[0], 2);
    }

    #[test]
    fn test_deform_patch_and_surface_offset() {
        let g = cube_missing_face();
        let np = g.npoints;
        let out_flat = cook(g.clone(), json!({"mode": "triangle_fan", "surface_offset": 0.0, "tangent_strength": 0.0, "smooth": 0.0})).unwrap();
        let out_deformed = cook(g.clone(), json!({"mode": "triangle_fan", "surface_offset": 1.0, "tangent_strength": 0.0, "deform_patch": true, "smooth": 0.0})).unwrap();
        let out_disabled = cook(g, json!({"mode": "triangle_fan", "surface_offset": 1.0, "tangent_strength": 0.0, "deform_patch": false, "smooth": 0.0})).unwrap();

        let flat_center = &out_flat.p()[np * 3..];
        let def_center = &out_deformed.p()[np * 3..];
        let dis_center = &out_disabled.p()[np * 3..];

        assert_ne!(flat_center, def_center);
        assert_eq!(flat_center, dis_center);
    }

    #[test]
    fn test_output_groups_and_append() {
        let mut g = cube_missing_face();
        g.prim.insert("group_patch".into(), Arc::new(Attr::i32(1, vec![1, 0, 0, 0, 0])));

        let out_replace = cook(g.clone(), json!({"append_to_groups": false})).unwrap();
        let vals_replace = out_replace.prim["group_patch"].as_i32().unwrap();
        assert_eq!(vals_replace[0], 0);
        assert_eq!(vals_replace[5], 1);

        let out_append = cook(g, json!({"append_to_groups": true})).unwrap();
        let vals_append = out_append.prim["group_patch"].as_i32().unwrap();
        assert_eq!(vals_append[0], 1);
        assert_eq!(vals_append[5], 1);
    }

    #[test]
    fn test_no_nan_across_operations() {
        let g = cube_missing_face();
        for mode in &["quadrilaterals", "triangles", "triangle_fan", "quadrilateral_fan", "single_polygon"] {
            let out = cook(g.clone(), json!({
                "mode": mode,
                "smooth": 50.0,
                "surface_offset": 0.5,
                "tangent_strength": 0.4,
                "update_point_normals": true,
            })).unwrap();
            assert!(out.p().iter().all(|x| x.is_finite()));
            if let Some(n) = out.point.get("N") {
                assert!(n.as_f32().unwrap().iter().all(|x| x.is_finite()));
            }
        }
    }
}
