//! Architecture and procedural systems (minihou/nodes/procedural_systems.py).
use std::sync::Arc;

use super::{InputSpec, Inputs, NodeSpec, GEO};
use crate::error::{Error, Result};
use crate::geometry::{Attr, AttrData, Geometry};
use crate::params::Params;

pub const BUILDING_RULES: NodeSpec = NodeSpec {
    name: "building_rules",
    inputs: GEO,
    params: crate::params!(
        "storeys": 3;
        "storey_height": 3.0;
        "bay_width": 2.5;
        "window_width": 0.55;
        "window_height": 0.55;
        "balcony_probability": 0.25;
        "balcony_depth": 0.7;
        "roof": "flat";
        "roof_height": 1.5;
        "seed": 0;
        "rules": "[]"
    ),
    cook: building_rules,
};

#[derive(Clone, Copy)]
struct RuleValues {
    window_width: f64,
    window_height: f64,
    balcony_probability: f64,
}

fn building_rules(inputs: &Inputs, p: &Params) -> Result<Geometry> {
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

        let mut rng = super::np_random::Rng::new(&[(seed as u32) as u64, bi as u64]);

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
        super::model::write_group(&mut out, "prim", name, &mask)?;
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

pub const REGION_IDENTIFY: NodeSpec = NodeSpec {
    name: "region_identify",
    inputs: GEO,
    params: crate::params!(
        "mode": "connectivity";
        "attribute": "class";
        "seed": 0
    ),
    cook: region_identify,
};

pub const REGION_TRANSFORM: NodeSpec = NodeSpec {
    name: "region_transform",
    inputs: GEO,
    params: crate::params!(
        "translate": [0.0, 0.0, 0.0];
        "rotate": [0.0, 0.0, 0.0];
        "scale": [1.0, 1.0, 1.0];
        "random_translate": [0.0, 0.0, 0.0];
        "random_rotate": [0.0, 0.0, 0.0];
        "random_scale": 0.0;
        "extrude": 0.0;
        "random_extrude": 0.0;
        "seed": 0
    ),
    cook: region_transform,
};

fn region_identify(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = REGION_IDENTIFY.one(inputs, 0)?;
    region_identify_impl(&geo, p)
}

fn region_identify_impl(geo: &Geometry, p: &Params) -> Result<Geometry> {
    for &v in geo.p() {
        if !v.is_finite() {
            return Err(Error::new("region positions must be finite"));
        }
    }
    let mode = p.e("mode", &["connectivity", "primitive", "attribute"])?;
    let attribute = p.s("attribute")?;
    let seed = p.i("seed")?;

    let labels: Vec<i32> = match mode.as_str() {
        "connectivity" => {
            let (_, l) = super::attrib_tools::piece_labels(geo);
            l
        }
        "primitive" => (0..geo.nprims() as i32).collect(),
        "attribute" => {
            let a = geo.prim.get(&attribute).ok_or_else(|| {
                Error::new("region attribute must be a scalar integer primitive attribute")
            })?;
            if a.width != 1 {
                return Err(Error::new(
                    "region attribute must be a scalar integer primitive attribute",
                ));
            }
            let vals = match &a.data {
                AttrData::I32(v) => v.clone(),
                _ => {
                    return Err(Error::new(
                        "region attribute must be a scalar integer primitive attribute",
                    ))
                }
            };
            if vals.len() != geo.nprims() {
                return Err(Error::new(
                    "region attribute must be a scalar integer primitive attribute",
                ));
            }
            vals
        }
        _ => unreachable!(),
    };

    let mut keys: Vec<i32> = labels.clone();
    keys.sort_unstable();
    keys.dedup();

    if keys.len() > 10000 {
        return Err(Error::new("at most 10000 regions are supported"));
    }
    if keys.len() * geo.nvertices().max(1) > 50_000_000 {
        return Err(Error::new(
            "region identification work budget exceeded; process fewer faces or regions",
        ));
    }

    let nprims = geo.nprims();
    let mut random = vec![0.0f32; nprims];
    let mut centers = vec![0.0f32; nprims * 3];
    let mut colors = vec![0.0f32; nprims * 3];

    let p_slice = geo.p();
    let vp = &geo.topology.vertex_point;
    let offs = &geo.topology.prim_offsets;

    for key in keys {
        let mut used_pts = Vec::new();
        for p in 0..nprims {
            if labels[p] == key {
                let start = offs[p] as usize;
                let end = offs[p + 1] as usize;
                for &v in &vp[start..end] {
                    used_pts.push(v as usize);
                }
            }
        }
        used_pts.sort_unstable();
        used_pts.dedup();

        let mut center = [0.0f64; 3];
        if !used_pts.is_empty() {
            for &pt in &used_pts {
                center[0] += p_slice[pt * 3] as f64;
                center[1] += p_slice[pt * 3 + 1] as f64;
                center[2] += p_slice[pt * 3 + 2] as f64;
            }
            center[0] /= used_pts.len() as f64;
            center[1] /= used_pts.len() as f64;
            center[2] /= used_pts.len() as f64;
        }

        let mut rng = super::np_random::Rng::new(&[
            (seed as u64) & 0xFFFF_FFFF,
            (key as u64) & 0xFFFF_FFFF,
        ]);
        let rand_val = rng.random() as f32;
        let col_val = [
            (0.15 + 0.70 * rng.random()) as f32,
            (0.15 + 0.70 * rng.random()) as f32,
            (0.15 + 0.70 * rng.random()) as f32,
        ];

        for p in 0..nprims {
            if labels[p] == key {
                random[p] = rand_val;
                centers[p * 3] = center[0] as f32;
                centers[p * 3 + 1] = center[1] as f32;
                centers[p * 3 + 2] = center[2] as f32;
                colors[p * 3] = col_val[0];
                colors[p * 3 + 1] = col_val[1];
                colors[p * 3 + 2] = col_val[2];
            }
        }
    }

    let mut out = geo.clone();
    out.prim.insert("region_id".into(), Arc::new(Attr::i32(1, labels)));
    out.prim
        .insert("region_random".into(), Arc::new(Attr::f32(1, random)));
    out.prim
        .insert("region_center".into(), Arc::new(Attr::f32(3, centers)));
    out.prim.insert("Cd".into(), Arc::new(Attr::f32(3, colors)));
    Ok(out)
}

fn region_transform(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = REGION_TRANSFORM.one(inputs, 0)?;
    for &v in geo.p() {
        if !v.is_finite() {
            return Err(Error::new("region positions must be finite"));
        }
    }
    if geo.topology.has_open() {
        return Err(Error::new("region_transform requires polygon faces"));
    }
    for p_idx in 0..geo.nprims() {
        let size = geo.topology.prim_offsets[p_idx + 1] - geo.topology.prim_offsets[p_idx];
        if size < 3 {
            return Err(Error::new("region_transform requires polygon faces"));
        }
    }

    let seed = p.i("seed")?;
    let geo = if !geo.prim.contains_key("region_id") {
        let id_params = Params::resolve(
            REGION_IDENTIFY.params,
            serde_json::json!({
                "mode": "connectivity",
                "attribute": "class",
                "seed": seed
            })
            .as_object(),
        )?;
        region_identify_impl(&geo, &id_params)?
    } else {
        (*geo).clone()
    };

    let labels_attr = geo
        .prim
        .get("region_id")
        .ok_or_else(|| Error::new("region_id must be a scalar integer primitive attribute"))?;
    if labels_attr.width != 1 {
        return Err(Error::new(
            "region_id must be a scalar integer primitive attribute",
        ));
    }
    let labels = match &labels_attr.data {
        AttrData::I32(v) => v,
        _ => {
            return Err(Error::new(
                "region_id must be a scalar integer primitive attribute",
            ))
        }
    };
    if labels.len() != geo.nprims() {
        return Err(Error::new(
            "region_id must be a scalar integer primitive attribute",
        ));
    }

    let mut keys: Vec<i32> = labels.clone();
    keys.sort_unstable();
    keys.dedup();

    if keys.len() > 10000 || geo.nvertices() > 1_000_000 {
        return Err(Error::new(
            "region processing limit: 10000 regions and 1000000 input vertices",
        ));
    }
    if keys.len() * geo.nvertices().max(1) > 50_000_000 {
        return Err(Error::new(
            "region processing work budget exceeded; process fewer faces or regions",
        ));
    }

    let translate = p.v3("translate")?;
    let rotate = p.v3("rotate")?;
    let scale = p.v3("scale")?;
    let random_translate = p.v3("random_translate")?;
    let random_rotate = p.v3("random_rotate")?;
    let random_scale = p.f("random_scale")?;
    let extrude = p.f("extrude")?;
    let random_extrude = p.f("random_extrude")?;

    let mut parts = Vec::new();
    for &key in &keys {
        let mask: Vec<bool> = labels.iter().map(|&l| l == key).collect();
        let mut part = super::group::keep_prims(&geo, &mask, true)?;

        let mut center = [0.0f64; 3];
        if part.npoints > 0 {
            for c in part.p().chunks_exact(3) {
                center[0] += c[0] as f64;
                center[1] += c[1] as f64;
                center[2] += c[2] as f64;
            }
            center[0] /= part.npoints as f64;
            center[1] /= part.npoints as f64;
            center[2] /= part.npoints as f64;
        }

        let mut rng = super::np_random::Rng::new(&[
            (seed as u64) & 0xFFFF_FFFF,
            (key as u64) & 0xFFFF_FFFF,
        ]);
        let t = [
            translate[0] + (-1.0 + 2.0 * rng.random()) * random_translate[0],
            translate[1] + (-1.0 + 2.0 * rng.random()) * random_translate[1],
            translate[2] + (-1.0 + 2.0 * rng.random()) * random_translate[2],
        ];
        let r = [
            rotate[0] + (-1.0 + 2.0 * rng.random()) * random_rotate[0],
            rotate[1] + (-1.0 + 2.0 * rng.random()) * random_rotate[1],
            rotate[2] + (-1.0 + 2.0 * rng.random()) * random_rotate[2],
        ];
        let s_var = 1.0 + (-random_scale + 2.0 * random_scale * rng.random());
        let s = [scale[0] * s_var, scale[1] * s_var, scale[2] * s_var];
        let d = extrude + (-random_extrude + 2.0 * random_extrude * rng.random());

        if d != 0.0 {
            part = super::model::extrude(&part, "", d, 0.0, "connected", true, None)?;
            part.point.remove("N");
            part.vertex.remove("N");
            part.prim.remove("N");
        }

        let r_rad = [r[0].to_radians(), r[1].to_radians(), r[2].to_radians()];
        let rot_mat = super::pcg::euler_xyz_to_matrix(r_rad);

        let mut new_p = Vec::with_capacity(part.npoints * 3);
        for c in part.p().chunks_exact(3) {
            let p_rel = [
                (c[0] as f64 - center[0]) * s[0],
                (c[1] as f64 - center[1]) * s[1],
                (c[2] as f64 - center[2]) * s[2],
            ];
            let px = rot_mat[0][0] * p_rel[0]
                + rot_mat[0][1] * p_rel[1]
                + rot_mat[0][2] * p_rel[2]
                + center[0]
                + t[0];
            let py = rot_mat[1][0] * p_rel[0]
                + rot_mat[1][1] * p_rel[1]
                + rot_mat[1][2] * p_rel[2]
                + center[1]
                + t[1];
            let pz = rot_mat[2][0] * p_rel[0]
                + rot_mat[2][1] * p_rel[1]
                + rot_mat[2][2] * p_rel[2]
                + center[2]
                + t[2];
            if !px.is_finite() || !py.is_finite() || !pz.is_finite() {
                return Err(Error::new("transformed region positions must be finite"));
            }
            if px.abs() > f32::MAX as f64
                || py.abs() > f32::MAX as f64
                || pz.abs() > f32::MAX as f64
            {
                return Err(Error::new(
                    "transformed region positions exceed float32 range",
                ));
            }
            new_p.extend([px as f32, py as f32, pz as f32]);
        }
        part.set_p(new_p);

        let transform_normals = |attr: &Attr| -> Attr {
            if let AttrData::F32(vals) = &attr.data {
                let mut out = Vec::with_capacity(vals.len());
                for c in vals.chunks_exact(attr.width) {
                    let n_unscaled = [c[0] as f64 / s[0], c[1] as f64 / s[1], c[2] as f64 / s[2]];
                    let nx = rot_mat[0][0] * n_unscaled[0]
                        + rot_mat[0][1] * n_unscaled[1]
                        + rot_mat[0][2] * n_unscaled[2];
                    let ny = rot_mat[1][0] * n_unscaled[0]
                        + rot_mat[1][1] * n_unscaled[1]
                        + rot_mat[1][2] * n_unscaled[2];
                    let nz = rot_mat[2][0] * n_unscaled[0]
                        + rot_mat[2][1] * n_unscaled[1]
                        + rot_mat[2][2] * n_unscaled[2];
                    let len = (nx * nx + ny * ny + nz * nz).sqrt();
                    let inv = if len < 1e-20 { 1.0 } else { 1.0 / len };
                    out.extend([(nx * inv) as f32, (ny * inv) as f32, (nz * inv) as f32]);
                }
                Attr {
                    width: attr.width,
                    data: AttrData::F32(out),
                }
            } else {
                attr.clone()
            }
        };

        if let Some(n_attr) = part.point.get("N").cloned() {
            part.point.insert("N".into(), Arc::new(transform_normals(&n_attr)));
        }
        if let Some(n_attr) = part.vertex.get("N").cloned() {
            part.vertex
                .insert("N".into(), Arc::new(transform_normals(&n_attr)));
        }
        if let Some(n_attr) = part.prim.get("N").cloned() {
            part.prim.insert("N".into(), Arc::new(transform_normals(&n_attr)));
        }

        part.prim
            .insert("region_id".into(), Arc::new(Attr::i32(1, vec![key; part.nprims()])));
        let mut region_centers = Vec::with_capacity(part.nprims() * 3);
        for _ in 0..part.nprims() {
            region_centers.extend([
                (center[0] + t[0]) as f32,
                (center[1] + t[1]) as f32,
                (center[2] + t[2]) as f32,
            ]);
        }
        part.prim
            .insert("region_center".into(), Arc::new(Attr::f32(3, region_centers)));
        parts.push(part);
    }

    if parts.is_empty() {
        Ok(geo)
    } else {
        super::merge::merge(
            &[parts.into_iter().map(Arc::new).collect()],
            &Params::resolve(&[], None)?,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad() -> Geometry {
        let p = vec![0.0, 0.0, 0.0, 8.0, 0.0, 0.0, 8.0, 6.0, 0.0, 0.0, 6.0, 0.0];
        let vp = vec![0, 1, 2, 3];
        let offs = vec![0, 4];
        Geometry::from_arrays(p, vp, offs, None).unwrap()
    }

    #[test]
    fn test_building_rules_counts_and_groups() {
        let q = quad();
        let p = Params::resolve(
            BUILDING_RULES.params,
            serde_json::json!({
                "storeys": 3,
                "bay_width": 2.0,
                "balcony_probability": 0.0
            })
            .as_object(),
        )
        .unwrap();
        let out = building_rules(&[vec![Arc::new(q)]], &p).unwrap();
        let crate::geometry::AttrData::I32(comp) = &out.prim["component"].data else {
            panic!("component must be an integer attribute");
        };
        assert_eq!(comp.iter().filter(|&&c| c == 0).count(), 42);
        assert_eq!(comp.iter().filter(|&&c| c == 1).count(), 42);
        assert_eq!(comp.iter().filter(|&&c| c == 2).count(), 0);
        assert_eq!(comp.iter().filter(|&&c| c == 3).count(), 1);
        assert_eq!(comp.iter().filter(|&&c| c == 4).count(), 1);

        let max_z = out.p().chunks_exact(3).map(|c| c[2]).fold(f32::NEG_INFINITY, f32::max);
        assert!((max_z - 9.0).abs() < 1e-5);
    }

    #[test]
    fn test_region_identify_and_transform() {
        let q = quad();
        let p_id = Params::resolve(
            REGION_IDENTIFY.params,
            serde_json::json!({"mode": "primitive"}).as_object(),
        )
        .unwrap();
        let marked = region_identify(&[vec![Arc::new(q.clone())]], &p_id).unwrap();
        assert_eq!(marked.nprims(), 1);
        let AttrData::I32(region_id) = &marked.prim["region_id"].data else {
            panic!("region_id must be I32");
        };
        assert_eq!(region_id, &[0]);
        assert!(marked.prim.contains_key("region_random"));
        assert!(marked.prim.contains_key("region_center"));
        assert!(marked.prim.contains_key("Cd"));

        let p_xform = Params::resolve(
            REGION_TRANSFORM.params,
            serde_json::json!({"translate": [1.0, 2.0, 3.0]}).as_object(),
        )
        .unwrap();
        let xformed = region_transform(&[vec![Arc::new(marked)]], &p_xform).unwrap();
        assert_eq!(xformed.npoints, 4);
        assert!((xformed.p()[0] - 1.0).abs() < 1e-5);
        assert!((xformed.p()[1] - 2.0).abs() < 1e-5);
        assert!((xformed.p()[2] - 3.0).abs() < 1e-5);
    }

    #[test]
    fn test_region_transform_extrude() {
        let q = quad();
        let p_xform = Params::resolve(
            REGION_TRANSFORM.params,
            serde_json::json!({"extrude": 0.5}).as_object(),
        )
        .unwrap();
        let xformed = region_transform(&[vec![Arc::new(q)]], &p_xform).unwrap();
        // Quad extruded with back=true produces 6 faces (1 front, 1 back, 4 sides)
        assert_eq!(xformed.nprims(), 6);
    }
}
