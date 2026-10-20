use super::*;

#[derive(Clone, Copy)]
struct RuleValues {
    window_width: f64,
    window_height: f64,
    balcony_probability: f64,
}

pub(super) fn building_rules(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = BUILDING_RULES.one(inputs, 0)?;

    for &val in geo.p() {
        if !val.is_finite() {
            return Err(Error::new("footprint positions must be finite"));
        }
    }

    if geo.topology.has_open() {
        return Err(Error::new("building_rules needs closed footprint polygons"));
    }
    if geo.nprims() > 100 {
        return Err(Error::new("building_rules supports at most 100 footprints per cook"));
    }
    if geo.nprims() == 0 {
        return Ok(Geometry::points_only(Vec::new())?);
    }

    let storeys = p.i("storeys")?;
    if !(1..=100).contains(&storeys) {
        return Err(Error::new("storeys must be 1..100"));
    }
    let storeys = storeys as i32;
    let storey_height = p.f("storey_height")?;
    let bay_width = p.f("bay_width")?;
    let window_width = p.f("window_width")?;
    let window_height = p.f("window_height")?;
    let balcony_prob = p.f("balcony_probability")?;
    let balcony_depth = p.f("balcony_depth")?;
    let roof = p.e("roof", &["flat", "pyramid", "gable"])?;
    let roof_height = p.f("roof_height")?;
    let seed = p.i("seed")?;
    let rules_json = p.s("rules")?;

    let rules_val: serde_json::Value = serde_json::from_str(&rules_json).map_err(|_| {
        Error::new("rules must be a JSON list of floor override objects")
    })?;
    let rule_list = rules_val.as_array().ok_or_else(|| {
        Error::new("rules must be a JSON list of floor override objects")
    })?;
    if rule_list.len() > 100 {
        return Err(Error::new("rules must be a list with at most 100 entries"));
    }

    for r in rule_list {
        let obj = r.as_object().ok_or_else(|| {
            Error::new("each rule supports floor, window_width, window_height and balcony_probability only")
        })?;
        for k in obj.keys() {
            if k != "floor" && k != "window_width" && k != "window_height" && k != "balcony_probability" {
                return Err(Error::new("each rule supports floor, window_width, window_height and balcony_probability only"));
            }
        }
        let floor_val = obj.get("floor");
        let valid_floor = match floor_val {
            None => true,
            Some(v) => {
                if let Some(s) = v.as_str() {
                    s == "all" || s == "ground" || s == "top"
                } else if let Some(i) = v.as_i64() {
                    i >= 0 && i < storeys as i64
                } else {
                    false
                }
            }
        };
        if !valid_floor {
            return Err(Error::new("rule floor must be all, ground, top, or a valid zero-based storey"));
        }

        for (k, v) in obj {
            if k != "floor" {
                let num = v.as_f64().ok_or_else(|| Error::new(format!("invalid rule value for {k}")))?;
                let max_allowed = if k == "balcony_probability" { 1.0 } else { 0.95 };
                if !num.is_finite() || num < 0.0 || num > max_allowed {
                    return Err(Error::new(format!("invalid rule value for {k}")));
                }
            }
        }
    }

    let mut points: Vec<f32> = Vec::new();
    let mut vertex_point: Vec<i32> = Vec::new();
    let mut prim_offsets: Vec<i32> = vec![0];
    let mut tags: Vec<[i32; 5]> = Vec::new(); // (building_id, floor, facade, bay, component)

    let mut add_face = |verts: &[[f64; 3]], b: i32, f: i32, facade: i32, bay: i32, comp: i32| -> Result<()> {
        if tags.len() >= 200_000 {
            return Err(Error::new("building_rules exceeds 200000 faces; reduce storeys or increase bay width"));
        }
        let start = (points.len() / 3) as i32;
        for &v in verts {
            points.extend_from_slice(&[v[0] as f32, v[1] as f32, v[2] as f32]);
        }
        let end = (points.len() / 3) as i32;
        for idx in start..end {
            vertex_point.push(idx);
        }
        prim_offsets.push(vertex_point.len() as i32);
        tags.push([b, f, facade, bay, comp]);
        Ok(())
    };

    let geo_p = geo.p();
    let vp = &geo.topology.vertex_point;

    for bi in 0..geo.nprims() {
        let (p_start, p_end) = (
            geo.topology.prim_offsets[bi] as usize,
            geo.topology.prim_offsets[bi + 1] as usize,
        );
        let m = p_end - p_start;
        if !(3..=256).contains(&m) {
            return Err(Error::new("footprints need 3 to 256 vertices"));
        }

        let mut p_poly = Vec::with_capacity(m);
        for v in &vp[p_start..p_end] {
            let idx = *v as usize;
            p_poly.push([
                geo_p[idx * 3] as f64,
                geo_p[idx * 3 + 1] as f64,
                geo_p[idx * 3 + 2] as f64,
            ]);
        }

        let mut min_pt = [f64::INFINITY; 3];
        let mut max_pt = [f64::NEG_INFINITY; 3];
        for pt in &p_poly {
            for k in 0..3 {
                min_pt[k] = min_pt[k].min(pt[k]);
                max_pt[k] = max_pt[k].max(pt[k]);
            }
        }
        let extent = (max_pt[0] - min_pt[0])
            .max(max_pt[1] - min_pt[1])
            .max(max_pt[2] - min_pt[2])
            .max(1.0);

        if max_pt[2] - min_pt[2] > extent * 1e-6 {
            return Err(Error::new("footprints must be horizontal in XY"));
        }

        let mut area2 = 0.0;
        for j in 0..m {
            let next_j = (j + 1) % m;
            area2 += p_poly[j][0] * p_poly[next_j][1] - p_poly[j][1] * p_poly[next_j][0];
        }

        if area2.abs() < extent * extent * 1e-10 {
            return Err(Error::new("footprint has zero area"));
        }

        if area2 < 0.0 {
            p_poly.reverse();
        }

        let mut edge = Vec::with_capacity(m);
        for j in 0..m {
            let next_j = (j + 1) % m;
            edge.push([
                p_poly[next_j][0] - p_poly[j][0],
                p_poly[next_j][1] - p_poly[j][1],
                p_poly[next_j][2] - p_poly[j][2],
            ]);
        }

        // Strict convexity test
        for j in 0..m {
            let e = edge[j];
            let e_len = (e[0] * e[0] + e[1] * e[1]).sqrt();
            if e_len < extent * 1e-8 {
                return Err(Error::new(
                    "footprints must be strictly convex simple polygons (no holes or collinear corners)",
                ));
            }
            for k in 0..m {
                if k != j && k != (j + 1) % m {
                    let delta = [p_poly[k][0] - p_poly[j][0], p_poly[k][1] - p_poly[j][1]];
                    let signed = e[0] * delta[1] - e[1] * delta[0];
                    if signed <= extent * extent * 1e-10 {
                        return Err(Error::new(
                            "footprints must be strictly convex simple polygons (no holes or collinear corners)",
                        ));
                    }
                }
            }
        }

        if roof == "gable" {
            if m != 4 {
                return Err(Error::new("gable roof requires a rectangular footprint"));
            }
            for j in 0..4 {
                let e0 = edge[j];
                let e1 = edge[(j + 1) % 4];
                let dot_e = (e0[0] * e1[0] + e0[1] * e1[1]).abs();
                let len0 = (e0[0] * e0[0] + e0[1] * e0[1]).sqrt();
                let len1 = (e1[0] * e1[0] + e1[1] * e1[1]).sqrt();
                if dot_e > len0 * len1 * 1e-5 {
                    return Err(Error::new("gable roof requires a rectangular footprint"));
                }
            }
        }

        let mut rng = super::super::np_random::Rng::new(&[(seed as u32) as u64, bi as u64]);

        // Foundation face (reversed so normal points downwards in -Z)
        let mut found = p_poly.clone();
        found.reverse();
        add_face(&found, bi as i32, -1, -1, -1, 4)?;

        for floor in 0..storeys {
            let mut values = RuleValues {
                window_width,
                window_height,
                balcony_probability: balcony_prob,
            };

            for r in rule_list {
                let obj = r.as_object().unwrap();
                let f_match = match obj.get("floor").and_then(|v| v.as_str()) {
                    Some("all") => true,
                    Some("ground") => floor == 0,
                    Some("top") => floor == storeys - 1,
                    _ => match obj.get("floor").and_then(|v| v.as_i64()) {
                        Some(i) => i == floor as i64,
                        None => true,
                    },
                };
                if f_match {
                    if let Some(w) = obj.get("window_width").and_then(|v| v.as_f64()) {
                        values.window_width = w;
                    }
                    if let Some(h) = obj.get("window_height").and_then(|v| v.as_f64()) {
                        values.window_height = h;
                    }
                    if let Some(b) = obj.get("balcony_probability").and_then(|v| v.as_f64()) {
                        values.balcony_probability = b;
                    }
                }
            }

            let z = [0.0, 0.0, floor as f64 * storey_height];
            let up = [0.0, 0.0, storey_height];

            for (side, &e) in edge.iter().enumerate() {
                let length = (e[0] * e[0] + e[1] * e[1]).sqrt();
                let bays = (length / bay_width).floor().max(1.0) as usize;
                if bays > 2000 {
                    return Err(Error::new("more than 2000 bays on one facade; increase bay_width"));
                }
                let outward = [e[1] / length, -e[0] / length, 0.0];

                for bay in 0..bays {
                    let t0 = bay as f64 / bays as f64;
                    let t1 = (bay + 1) as f64 / bays as f64;
                    let mut a = [
                        p_poly[side][0] + e[0] * t0 + z[0],
                        p_poly[side][1] + e[1] * t0 + z[1],
                        p_poly[side][2] + e[2] * t0 + z[2],
                    ];
                    let mut b = [
                        p_poly[side][0] + e[0] * t1 + z[0],
                        p_poly[side][1] + e[1] * t1 + z[1],
                        p_poly[side][2] + e[2] * t1 + z[2],
                    ];

                    // Wall face
                    add_face(
                        &[
                            a,
                            b,
                            [b[0] + up[0], b[1] + up[1], b[2] + up[2]],
                            [a[0] + up[0], a[1] + up[1], a[2] + up[2]],
                        ],
                        bi as i32,
                        floor,
                        side as i32,
                        bay as i32,
                        0,
                    )?;

                    let (w, h) = (values.window_width, values.window_height);
                    if w > 0.0 && h > 0.0 {
                        let mid = [
                            (a[0] + b[0] + up[0]) / 2.0 + outward[0] * 0.015,
                            (a[1] + b[1] + up[1]) / 2.0 + outward[1] * 0.015,
                            (a[2] + b[2] + up[2]) / 2.0 + outward[2] * 0.015,
                        ];
                        let dx = [(b[0] - a[0]) * w / 2.0, (b[1] - a[1]) * w / 2.0, 0.0];
                        let dz = [0.0, 0.0, up[2] * h / 2.0];
                        add_face(
                            &[
                                [mid[0] - dx[0] - dz[0], mid[1] - dx[1] - dz[1], mid[2] - dx[2] - dz[2]],
                                [mid[0] + dx[0] - dz[0], mid[1] + dx[1] - dz[1], mid[2] + dx[2] - dz[2]],
                                [mid[0] + dx[0] + dz[0], mid[1] + dx[1] + dz[1], mid[2] + dx[2] + dz[2]],
                                [mid[0] - dx[0] + dz[0], mid[1] - dx[1] + dz[1], mid[2] - dx[2] + dz[2]],
                            ],
                            bi as i32,
                            floor,
                            side as i32,
                            bay as i32,
                            1,
                        )?;
                    }

                    if floor > 0 && balcony_depth > 0.0 && rng.random() < values.balcony_probability {
                        let old_a = a;
                        let old_b = b;
                        a = [
                            old_a[0] + (old_b[0] - old_a[0]) * 0.1,
                            old_a[1] + (old_b[1] - old_a[1]) * 0.1,
                            old_a[2] + (old_b[2] - old_a[2]) * 0.1,
                        ];
                        b = [
                            old_b[0] - (old_b[0] - old_a[0]) * 0.1,
                            old_b[1] - (old_b[1] - old_a[1]) * 0.1,
                            old_b[2] - (old_b[2] - old_a[2]) * 0.1,
                        ];
                        let front_a = [
                            a[0] + outward[0] * balcony_depth,
                            a[1] + outward[1] * balcony_depth,
                            a[2] + outward[2] * balcony_depth,
                        ];
                        let front_b = [
                            b[0] + outward[0] * balcony_depth,
                            b[1] + outward[1] * balcony_depth,
                            b[2] + outward[2] * balcony_depth,
                        ];

                        add_face(&[a, front_a, front_b, b], bi as i32, floor, side as i32, bay as i32, 2)?;
                        let rail_h = (storey_height * 0.4).min(1.0);
                        let rail = [0.0, 0.0, rail_h];
                        for &(u_pt, v_pt) in &[(a, front_a), (front_a, front_b), (front_b, b)] {
                            add_face(
                                &[
                                    u_pt,
                                    v_pt,
                                    [v_pt[0] + rail[0], v_pt[1] + rail[1], v_pt[2] + rail[2]],
                                    [u_pt[0] + rail[0], u_pt[1] + rail[1], u_pt[2] + rail[2]],
                                ],
                                bi as i32,
                                floor,
                                side as i32,
                                bay as i32,
                                2,
                            )?;
                        }
                    }
                }
            }
        }

        let z_top = storeys as f64 * storey_height;
        let mut top = Vec::with_capacity(m);
        for pt in &p_poly {
            top.push([pt[0], pt[1], pt[2] + z_top]);
        }

        if roof == "flat" || roof_height == 0.0 {
            add_face(&top, bi as i32, storeys, -1, -1, 3)?;
        } else if roof == "pyramid" {
            let mut mean_top = [0.0; 3];
            for pt in &top {
                mean_top[0] += pt[0];
                mean_top[1] += pt[1];
                mean_top[2] += pt[2];
            }
            let peak = [
                mean_top[0] / m as f64,
                mean_top[1] / m as f64,
                mean_top[2] / m as f64 + roof_height,
            ];
            for j in 0..m {
                let next_j = (j + 1) % m;
                add_face(&[top[j], top[next_j], peak], bi as i32, storeys, j as i32, -1, 3)?;
            }
        } else {
            // gable
            let a_ridge = [
                (top[0][0] + top[1][0]) / 2.0,
                (top[0][1] + top[1][1]) / 2.0,
                (top[0][2] + top[1][2]) / 2.0 + roof_height,
            ];
            let b_ridge = [
                (top[2][0] + top[3][0]) / 2.0,
                (top[2][1] + top[3][1]) / 2.0,
                (top[2][2] + top[3][2]) / 2.0 + roof_height,
            ];
            add_face(&[top[0], top[1], a_ridge], bi as i32, storeys, -1, -1, 3)?;
            add_face(&[top[1], top[2], b_ridge, a_ridge], bi as i32, storeys, -1, -1, 3)?;
            add_face(&[top[2], top[3], b_ridge], bi as i32, storeys, -1, -1, 3)?;
            add_face(&[top[3], top[0], a_ridge, b_ridge], bi as i32, storeys, -1, -1, 3)?;
        }
    }

    let mut out = Geometry::from_arrays(points, vertex_point, prim_offsets, None)?;

    let mut b_id = Vec::with_capacity(tags.len());
    let mut fl_id = Vec::with_capacity(tags.len());
    let mut fac_id = Vec::with_capacity(tags.len());
    let mut bay_id = Vec::with_capacity(tags.len());
    let mut comp_id = Vec::with_capacity(tags.len());
    let mut cd = Vec::with_capacity(tags.len() * 3);

    let palette = [
        [0.65f32, 0.62, 0.55], // 0: wall
        [0.16, 0.35, 0.5],     // 1: window
        [0.42, 0.43, 0.45],    // 2: balcony
        [0.28, 0.16, 0.12],    // 3: roof
        [0.35, 0.35, 0.35],    // 4: foundation
    ];

    for t in &tags {
        b_id.push(t[0]);
        fl_id.push(t[1]);
        fac_id.push(t[2]);
        bay_id.push(t[3]);
        comp_id.push(t[4]);
        let c = t[4].clamp(0, 4) as usize;
        cd.extend_from_slice(&palette[c]);
    }

    out.prim.insert("building_id".into(), Arc::new(Attr::i32(1, b_id)));
    out.prim.insert("floor".into(), Arc::new(Attr::i32(1, fl_id)));
    out.prim.insert("facade".into(), Arc::new(Attr::i32(1, fac_id)));
    out.prim.insert("bay".into(), Arc::new(Attr::i32(1, bay_id)));
    out.prim.insert("component".into(), Arc::new(Attr::i32(1, comp_id.clone())));
    out.prim.insert("Cd".into(), Arc::new(Attr::f32(3, cd)));

    let group_names = ["walls", "windows", "balconies", "roof", "foundation"];
    for (i, name) in group_names.iter().enumerate() {
        let mask: Vec<bool> = comp_id.iter().map(|&c| c == i as i32).collect();
        super::super::model::write_group(&mut out, "prim", name, &mask)?;
    }

    let comp_names: Vec<serde_json::Value> = ["wall", "window", "balcony", "roof", "foundation"]
        .iter()
        .map(|s| serde_json::json!(s))
        .collect();
    out.detail.insert(
        "building_components".into(),
        serde_json::Value::Array(comp_names),
    );

    Ok(out)
}

