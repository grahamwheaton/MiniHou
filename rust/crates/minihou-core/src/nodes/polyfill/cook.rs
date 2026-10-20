use super::*;

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
