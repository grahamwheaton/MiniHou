use super::*;
use crate::noise;

pub(super) fn noise_node(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = NOISE.one(inputs, 0)?;
    let attrib = p.s("attrib")?;
    if attrib.is_empty() {
        return Err(Error::new("'attrib' must not be empty"));
    }
    let mode = p.e("mode", &["scalar", "vector"])?;
    let amp = p.v3("amplitude")?.map(|x| x as f32);
    let freq = p.f("frequency")? as f32;
    let offset = p.v3("offset")?.map(|x| x as f32);
    let octaves = p.i("octaves")?;
    if !(1..=12).contains(&octaves) {
        return Err(Error::new("octaves must be between 1 and 12"));
    }
    let (lac, gain, seed) = (p.f("lacunarity")?, p.f("gain")?, p.i("seed")?);
    let group = p.s("group").unwrap_or_default();
    let blend = (p.f("blend").unwrap_or(1.0) as f32).clamp(0.0, 1.0);
    let class = p.e("class", &["point", "primitive", "vertex", "detail"]).unwrap_or_else(|_| "point".into());
    let channel_x = p.b("channel_x").unwrap_or(true);
    let channel_y = p.b("channel_y").unwrap_or(true);
    let channel_z = p.b("channel_z").unwrap_or(true);
    let noise_along_vec = p.b("noise_along_vector").unwrap_or(false);
    let vector_attr = p.s("vector_attribute").unwrap_or_else(|_| "N".into());
    let op = p.e("operation", &["default", "add", "set", "multiply", "subtract"]).unwrap_or_else(|_| "default".into());
    let range_vals = p.e("range_values", &["zero_centered", "positive"]).unwrap_or_else(|_| "zero_centered".into());
    let output_raw = p.b("output_raw").unwrap_or(false);
    let enable_remap = p.b("enable_remap").unwrap_or(false);
    let remap_min = p.f("remap_min").unwrap_or(0.0) as f32;
    let remap_max = p.f("remap_max").unwrap_or(1.0) as f32;
    let noise_type = p.e("noise_type", &["perlin", "simplex", "periodic", "worley"]).unwrap_or_else(|_| "perlin".into());
    let location_attrib = p.s("location_attribute").unwrap_or_else(|_| "P".into());
    let element_size = p.f("element_size").unwrap_or(1.0) as f32;
    let freq = if element_size != 1.0 && freq == 1.0 { 1.0 / element_size.max(1e-6) } else { freq };
    let animate = p.b("animate").unwrap_or(false);
    let anim_speed = p.f("animation_speed").unwrap_or(1.0) as f32;
    let anim_time = p.f("animation_time").unwrap_or(0.0) as f32;
    let fractal_type = p.e("fractal_type", &["standard", "terrain", "hybrid_terrain"]).unwrap_or_else(|_| "standard".into());
    let warping = p.b("warping").unwrap_or(false);
    let warp_amp = p.f("warp_amplitude").unwrap_or(1.0) as f32;
    let warp_freq = p.f("warp_frequency").unwrap_or(1.0) as f32;
    let clamp_min = p.b("clamp_min").unwrap_or(false);
    let clamp_max = p.b("clamp_max").unwrap_or(false);
    let min_val = p.f("min_value").unwrap_or(0.0) as f32;
    let max_val = p.f("max_value").unwrap_or(1.0) as f32;

    let is_simple_fast_path = class == "point"
        && !noise_along_vec
        && op == "default"
        && range_vals == "zero_centered"
        && !output_raw
        && !enable_remap
        && noise_type == "perlin"
        && location_attrib == "P"
        && !animate
        && fractal_type == "standard"
        && !warping
        && !clamp_min
        && !clamp_max
        && blend == 1.0
        && group.is_empty()
        && channel_x
        && channel_y
        && channel_z
        && element_size == 1.0;

    let src = geo.p();
    let mut out = (*geo).clone();

    if is_simple_fast_path {
        if mode == "scalar" && attrib != "P" {
            let pos: Vec<f32> = src.par_chunks_exact(3).flat_map_iter(|q| [0, 1, 2].map(|k| q[k] * freq + offset[k])).collect();
            let v = noise::fbm3(&pos, seed, octaves as u32, lac, gain);
            out.point.insert(attrib, Arc::new(Attr::f32(1, v.iter().map(|x| x * amp[0]).collect())));
            return Ok(out);
        }
        let add = attrib == "P";
        let v = noise::noise_vec(src, freq, offset, amp, seed, octaves as u32, lac, gain, mode == "vector", add);
        if add {
            out.set_p(v);
        } else {
            out.point.insert(attrib, Arc::new(Attr::f32(3, v)));
        }
        return Ok(out);
    }

    let n = match class.as_str() {
        "primitive" => geo.nprims(),
        "vertex" => geo.topology.nvertices(),
        "detail" => 1,
        _ => geo.npoints,
    };
    if n == 0 {
        return Ok(out);
    }

    let positions: Vec<[f32; 3]> = if location_attrib == "P" {
        match class.as_str() {
            "primitive" => {
                let p = geo.p();
                (0..geo.nprims()).map(|i| {
                    let r = geo.topology.prim_offsets[i] as usize..geo.topology.prim_offsets[i + 1] as usize;
                    let count = r.len();
                    if count == 0 { return [0.0, 0.0, 0.0]; }
                    let mut sum = [0.0f32; 3];
                    for v in r {
                        let pt = geo.topology.vertex_point[v] as usize;
                        for k in 0..3 { sum[k] += p[pt * 3 + k]; }
                    }
                    [sum[0] / count as f32, sum[1] / count as f32, sum[2] / count as f32]
                }).collect()
            }
            "vertex" => {
                let p = geo.p();
                (0..geo.topology.nvertices()).map(|v| {
                    let pt = geo.topology.vertex_point[v] as usize;
                    [p[pt * 3], p[pt * 3 + 1], p[pt * 3 + 2]]
                }).collect()
            }
            "detail" => {
                let p = geo.p();
                if geo.npoints > 0 {
                    let mut sum = [0.0f32; 3];
                    for pt in 0..geo.npoints {
                        for k in 0..3 { sum[k] += p[pt * 3 + k]; }
                    }
                    vec![[sum[0] / geo.npoints as f32, sum[1] / geo.npoints as f32, sum[2] / geo.npoints as f32]]
                } else {
                    vec![[0.0, 0.0, 0.0]]
                }
            }
            _ => {
                geo.p().chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect()
            }
        }
    } else {
        let attr_opt = geo.point.get(&location_attrib)
            .or_else(|| geo.prim.get(&location_attrib))
            .or_else(|| geo.vertex.get(&location_attrib));
        if let Some(attr) = attr_opt {
            if let Some(data) = attr.as_f32() {
                let w = attr.width;
                (0..n).map(|i| {
                    if i < attr.rows() {
                        let idx = i * w;
                        [
                            data[idx],
                            if w > 1 { data[idx + 1] } else { 0.0 },
                            if w > 2 { data[idx + 2] } else { 0.0 },
                        ]
                    } else {
                        [0.0, 0.0, 0.0]
                    }
                }).collect()
            } else {
                vec![[0.0, 0.0, 0.0]; n]
            }
        } else {
            geo.p().chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect()
        }
    };

    let mut eff_offset = offset;
    if animate {
        let t = anim_speed * anim_time;
        eff_offset[0] += t;
        eff_offset[1] += t * 0.7;
        eff_offset[2] += t * 0.5;
    }

    let normals_vec: Option<Vec<[f32; 3]>> = if noise_along_vec {
        let attr_opt = match class.as_str() {
            "primitive" => geo.prim.get(&vector_attr),
            "vertex" => geo.vertex.get(&vector_attr),
            _ => geo.point.get(&vector_attr),
        };
        if let Some(attr) = attr_opt {
            if let Some(data) = attr.as_f32() {
                let w = attr.width;
                Some((0..n).map(|i| {
                    if i < attr.rows() && w >= 3 {
                        let idx = i * w;
                        [data[idx], data[idx + 1], data[idx + 2]]
                    } else {
                        [0.0, 0.0, 1.0]
                    }
                }).collect())
            } else {
                None
            }
        } else if vector_attr == "N" {
            if class == "primitive" {
                Some(normals::prim_normals(geo.p(), &geo.topology).iter().map(|n| [n[0] as f32, n[1] as f32, n[2] as f32]).collect())
            } else {
                let pn = normals::point_normals(geo.p(), &geo.topology, geo.npoints);
                Some(pn.chunks_exact(3).map(|n| [n[0], n[1], n[2]]).collect())
            }
        } else {
            None
        }
    } else {
        None
    };

    let mask: Vec<bool> = if group.is_empty() {
        vec![true; n]
    } else {
        match class.as_str() {
            "primitive" => crate::nodes::group::mask(&geo, "prim", &group)?,
            "point" => crate::nodes::group::mask(&geo, "point", &group)?,
            _ => vec![true; n],
        }
    };

    let evaluate_element = |idx: usize| -> [f32; 3] {
        if !mask[idx] {
            return [0.0, 0.0, 0.0];
        }
        let mut p_coord = positions[idx];
        if warping {
            let wp = [p_coord[0] * warp_freq, p_coord[1] * warp_freq, p_coord[2] * warp_freq];
            p_coord[0] += noise::perlin3(wp, seed + 101) * warp_amp;
            p_coord[1] += noise::perlin3(wp, seed + 202) * warp_amp;
            p_coord[2] += noise::perlin3(wp, seed + 303) * warp_amp;
        }
        let sample_pos = [
            p_coord[0] * freq + eff_offset[0],
            p_coord[1] * freq + eff_offset[1],
            p_coord[2] * freq + eff_offset[2],
        ];
        let mut raw_val = if mode == "vector" && !noise_along_vec {
            [
                noise::sample_fractal(&noise_type, &fractal_type, sample_pos, seed, octaves as u32, lac, gain),
                noise::sample_fractal(&noise_type, &fractal_type, sample_pos, seed + 1013, octaves as u32, lac, gain),
                noise::sample_fractal(&noise_type, &fractal_type, sample_pos, seed + 2026, octaves as u32, lac, gain),
            ]
        } else {
            let s = noise::sample_fractal(&noise_type, &fractal_type, sample_pos, seed, octaves as u32, lac, gain);
            [s, s, s]
        };

        if range_vals == "positive" {
            for c in 0..3 { raw_val[c] = (raw_val[c] + 1.0) * 0.5; }
        }
        if enable_remap {
            let span = (remap_max - remap_min).abs().max(1e-6);
            for c in 0..3 { raw_val[c] = ((raw_val[c] - remap_min) / span).clamp(0.0, 1.0); }
        }

        let mut val = if output_raw {
            raw_val
        } else if noise_along_vec {
            let norm = normals_vec.as_ref().map(|ns| ns[idx]).unwrap_or([0.0, 0.0, 1.0]);
            let s = raw_val[0];
            [norm[0] * s * amp[0], norm[1] * s * amp[1], norm[2] * s * amp[2]]
        } else {
            [raw_val[0] * amp[0], raw_val[1] * amp[1], raw_val[2] * amp[2]]
        };

        if clamp_min {
            for c in 0..3 { val[c] = val[c].max(min_val); }
        }
        if clamp_max {
            for c in 0..3 { val[c] = val[c].min(max_val); }
        }
        val
    };

    let computed: Vec<[f32; 3]> = (0..n).into_par_iter().map(evaluate_element).collect();

    let is_p = attrib == "P" && class == "point";
    let is_vector = mode == "vector" || is_p || noise_along_vec;
    let width = if is_vector { 3 } else { 1 };

    let orig_vals: Vec<f32> = if is_p {
        geo.p().to_vec()
    } else {
        let table = match class.as_str() {
            "primitive" => &geo.prim,
            "vertex" => &geo.vertex,
            _ => &geo.point,
        };
        if let Some(attr) = table.get(&attrib).and_then(|a| a.as_f32()) {
            attr.to_vec()
        } else {
            vec![0.0f32; n * width]
        }
    };

    let mut final_vals = orig_vals.clone();
    let actual_op = if op == "default" {
        if is_p { "add" } else { "set" }
    } else {
        op.as_str()
    };

    for i in 0..n {
        if !mask[i] { continue; }
        let c = computed[i];
        for k in 0..width {
            let ch_enabled = match k {
                0 => channel_x,
                1 => channel_y,
                2 => channel_z,
                _ => true,
            };
            if !ch_enabled { continue; }
            let orig = orig_vals[i * width + k];
            let nv = c[k];
            let combined = match actual_op {
                "add" => orig + nv,
                "set" | "replace" => nv,
                "multiply" => orig * nv,
                "subtract" => orig - nv,
                _ => orig + nv,
            };
            final_vals[i * width + k] = orig + (combined - orig) * blend;
        }
    }

    if is_p {
        out.set_p(final_vals);
    } else {
        match class.as_str() {
            "primitive" => { out.prim.insert(attrib, Arc::new(Attr::f32(width, final_vals))); }
            "vertex" => { out.vertex.insert(attrib, Arc::new(Attr::f32(width, final_vals))); }
            "detail" => {
                if width == 1 {
                    out.detail.insert(attrib, serde_json::json!(final_vals[0]));
                } else {
                    out.detail.insert(attrib, serde_json::json!(final_vals));
                }
            }
            _ => { out.point.insert(attrib, Arc::new(Attr::f32(width, final_vals))); }
        }
    }

    Ok(out)
}

