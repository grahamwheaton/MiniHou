//! `transform` (`minihou/nodes/transform.py`), `normal` (`normal.py`) and `noise` (`noise.py`).

use std::sync::Arc;
use std::cmp::{Ordering, Reverse};
use std::collections::BinaryHeap;

use rayon::prelude::*;

use super::{InputSpec, Inputs, NodeSpec, GEO};
use crate::error::{Error, Result};
use crate::geometry::{Attr, AttrData, AttrTable, Geometry, Topology};
use crate::params::Params;
use crate::{math, noise, normals, topo, xform};
use super::groups;

mod transform;
use transform::transform;

pub const TRANSFORM: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "transform",
    inputs: GEO,
    params: crate::params!(
        "translate": [0.0, 0.0, 0.0]; "rotate": [0.0, 0.0, 0.0]; "scale": [1.0, 1.0, 1.0]; "uniform_scale": 1.0;
        "pivot": [0.0, 0.0, 0.0]; "axis_rotation": [0.0, 0.0, 0.0];
        "group": "";
        "proportional_enabled": false;
        "proportional_radius": 1.0;
        "proportional_falloff": "smooth";
        "proportional_connected": false;
        "proportional_projected": false;
        "proportional_view_direction": [0.0, 0.0, 1.0];
    ),
    cook: transform,
};

pub const NOISE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "noise",
    inputs: GEO,
    params: crate::params!(
        "attrib": "P"; "mode": "scalar"; "amplitude": [0.0, 0.0, 1.0]; "frequency": 1.0; "offset": [0.0, 0.0, 0.0];
        "octaves": 1; "lacunarity": 2.0; "gain": 0.5; "seed": 0;
        "group": "";
        "blend": 1.0;
        "class": "point";
        "channel_x": true; "channel_y": true; "channel_z": true;
        "noise_along_vector": false;
        "vector_attribute": "N";
        "operation": "default";
        "range_values": "zero_centered";
        "output_raw": false;
        "enable_remap": false;
        "remap_min": 0.0;
        "remap_max": 1.0;
        "noise_type": "perlin";
        "location_attribute": "P";
        "element_size": 1.0;
        "use_vexpression": false;
        "vexpression": "";
        "animate": false;
        "animation_speed": 1.0;
        "animation_time": 0.0;
        "fractal_type": "standard";
        "warping": false;
        "warp_amplitude": 1.0;
        "warp_frequency": 1.0;
        "clamp_min": false;
        "clamp_max": false;
        "min_value": 0.0;
        "max_value": 1.0;
    ),
    cook: noise_node,
};

pub const MOUNTAIN: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "mountain",
    inputs: GEO,
    params: crate::params!(
        "attrib": "P"; "mode": "scalar"; "amplitude": [0.0, 0.0, 1.0]; "frequency": 1.0; "offset": [0.0, 0.0, 0.0];
        "octaves": 3; "lacunarity": 2.0; "gain": 0.5; "seed": 0;
        "group": "";
        "blend": 1.0;
        "class": "point";
        "channel_x": true; "channel_y": true; "channel_z": true;
        "noise_along_vector": true;
        "vector_attribute": "N";
        "operation": "add";
        "range_values": "zero_centered";
        "output_raw": false;
        "enable_remap": false;
        "remap_min": 0.0;
        "remap_max": 1.0;
        "noise_type": "perlin";
        "location_attribute": "P";
        "element_size": 1.0;
        "use_vexpression": false;
        "vexpression": "";
        "animate": false;
        "animation_speed": 1.0;
        "animation_time": 0.0;
        "fractal_type": "standard";
        "warping": false;
        "warp_amplitude": 1.0;
        "warp_frequency": 1.0;
        "clamp_min": false;
        "clamp_max": false;
        "min_value": 0.0;
        "max_value": 1.0;
    ),
    cook: noise_node,
};

fn noise_node(inputs: &Inputs, p: &Params) -> Result<Geometry> {
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

pub const MATCH_SIZE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    // Retained for the Python fast path; the package owns the public match_size ID.
    name: "match_size_legacy",
    inputs: &[
        InputSpec { name: "geo", multi: false, optional: false },
        InputSpec { name: "target", multi: false, optional: true },
    ],
    params: crate::params!(
        "translate": true; "center": [0.0, 0.0, 0.0]; "justify_z": "center";
        "scale": false; "size": [1.0, 1.0, 1.0]; "uniform": true;
    ),
    cook: match_size,
};

fn match_size(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = MATCH_SIZE.one(inputs, 0)?;
    if geo.npoints == 0 {
        return Ok((*geo).clone());
    }
    let target = inputs[1].first().filter(|t| t.npoints > 0);
    let normals = geo.point.get("N").and_then(|a| (a.width == 3).then(|| a.as_f32()).flatten());
    let out = match_size_points(
        geo.p(),
        target.map(|t| t.p()),
        p.b("translate")?,
        p.v3("center")?,
        &p.e("justify_z", &["center", "min", "max"])?,
        p.b("scale")?,
        p.v3("size")?,
        p.b("uniform")?,
        normals,
    );
    let mut g = (*geo).clone();
    g.set_p(out.p);
    if let Some(n) = out.n {
        g.point.insert("N".into(), Arc::new(Attr::f32(3, n)));
    }
    Ok(g)
}

/// New positions and, when the scale is not uniform, rescaled and renormalised normals
/// (`minihou/nodes/attrib_tools.py`), all in float64 like the numpy body, back to float32.
pub struct MatchSizeOut {
    pub p: Vec<f32>,
    pub n: Option<Vec<f32>>,
}

/// `match_size` on raw arrays, shared by the node cook above and the Python fast path. `target` is the second input's
/// nonempty points (its bounding box replaces `center`/`size`); `normals` the point `N` when it is xyz, else `None`.
#[allow(clippy::too_many_arguments)]
pub fn match_size_points(
    points: &[f32],
    target: Option<&[f32]>,
    translate: bool,
    center: [f64; 3],
    justify_z: &str,
    scale: bool,
    size: [f64; 3],
    uniform: bool,
    normals: Option<&[f32]>,
) -> MatchSizeOut {
    let (mut center, mut size) = (center, size);
    if let Some(t) = target {
        let (tlo, thi) = (axis_extremes(t, false), axis_extremes(t, true));
        size = [thi[0] - tlo[0], thi[1] - tlo[1], thi[2] - tlo[2]];
        center = [(tlo[0] + thi[0]) / 2.0, (tlo[1] + thi[1]) / 2.0,
                  if justify_z == "center" { (tlo[2] + thi[2]) / 2.0 } else if justify_z == "min" { tlo[2] } else { thi[2] }];
    }
    let (lo, hi) = (axis_extremes(points, false), axis_extremes(points, true));
    let mid = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0, (lo[2] + hi[2]) / 2.0];
    let mut s = [1.0f64; 3];
    if scale {
        let ext = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
        let ok = [ext[0] > 1e-12, ext[1] > 1e-12, ext[2] > 1e-12];
        for k in 0..3 {
            s[k] = if ok[k] { size[k] / ext[k] } else { 1.0 };
        }
        if uniform {
            let vals: Vec<f64> = (0..3).filter(|&k| ok[k]).map(|k| s[k]).collect();
            let u = if vals.is_empty() { 1.0 } else { numpy_min(&vals) };
            s = [u, u, u];
        }
    }
    let anchor = [mid[0], mid[1], match justify_z { "min" => lo[2], "max" => hi[2], _ => mid[2] }];
    let tgt = if translate { center } else { anchor };
    let mut p = vec![0.0f32; points.len()];
    for (i, q) in points.as_chunks::<3>().0.iter().enumerate() {
        for k in 0..3 {
            p[3 * i + k] = ((q[k] as f64 - anchor[k]) * s[k] + tgt[k]) as f32;
        }
    }
    // numpy's `np.allclose(s, s[0])`: only a real per-axis scale change rescales N
    let s0 = s[0];
    let close = (0..3).all(|k| (s[k] - s0).abs() <= 1e-8 + 1e-5 * s0.abs());
    let n = normals.and_then(|nrm| {
        if close {
            return None;
        }
        let mut result = vec![0.0f32; nrm.len()];
        for (i, row) in nrm.as_chunks::<3>().0.iter().enumerate() {
            let v = [row[0] as f64 / s[0], row[1] as f64 / s[1], row[2] as f64 / s[2]];
            let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
            let len = if len < 1e-20 { 1.0 } else { len };
            for k in 0..3 {
                result[3 * i + k] = (v[k] / len) as f32;
            }
        }
        Some(result)
    });
    MatchSizeOut { p, n }
}

/// Per-axis minimum or maximum of flat xyz points, NaN-propagating like `np.ndarray.min(0)` / `.max(0)`.
fn axis_extremes(points: &[f32], want_max: bool) -> [f64; 3] {
    let mut out = [if want_max { f64::NEG_INFINITY } else { f64::INFINITY }; 3];
    let mut nan = [false; 3];
    for q in points.as_chunks::<3>().0 {
        for k in 0..3 {
            let v = q[k] as f64;
            if v.is_nan() {
                nan[k] = true;
            } else if (want_max && v > out[k]) || (!want_max && v < out[k]) {
                out[k] = v;
            }
        }
    }
    for k in 0..3 {
        if nan[k] {
            out[k] = f64::NAN;
        }
    }
    out
}

/// `np.min` over a list: NaN if any element is NaN, else the smallest.
fn numpy_min(v: &[f64]) -> f64 {
    let mut m = f64::INFINITY;
    for &x in v {
        if x.is_nan() {
            return f64::NAN;
        }
        if x < m {
            m = x;
        }
    }
    m
}

pub const DELETE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "delete",
    inputs: GEO,
    params: crate::params!("expr": "@P.z < 0"; "invert": false;),
    cook: delete_node,
};

fn delete_node(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = DELETE.one(inputs, 0)?;
    let expr = p.s("expr")?;
    let invert = p.b("invert")?;

    if geo.npoints == 0 {
        return Ok((*geo).clone());
    }

    let mut mask = delete_mask(&geo, &expr)?;
    if invert {
        for m in &mut mask {
            *m = !*m;
        }
    }

    if !mask.iter().any(|&m| m) {
        return Ok((*geo).clone());
    }

    let keep: Vec<bool> = mask.iter().map(|&m| !m).collect();
    keep_points(&geo, &keep)
}

fn delete_mask(geo: &Geometry, expr: &str) -> Result<Vec<bool>> {
    if expr.contains(';') || expr.contains('\n') {
        return Err(Error::new("the delete expression must be a single expression, e.g. @P.z < 0"));
    }
    let text = expr.trim();
    if text.is_empty() || text == "0" {
        return Ok(vec![false; geo.npoints]);
    }
    if text == "1" {
        return Ok(vec![true; geo.npoints]);
    }
    let trimmed = text.trim_start_matches('(').trim_end_matches(')').trim();
    if let Some(axis) = trimmed.strip_prefix("@P.") {
        let (k, rest) = match axis.chars().next() {
            Some('x') => (0, &axis[1..]),
            Some('y') => (1, &axis[1..]),
            Some('z') => (2, &axis[1..]),
            _ => return Err(Error::new("delete expression is not supported by the Rust backend")),
        };
        let rest = rest.trim();
        let (op, rhs) = [">=", "<=", "==", "!=", ">", "<"].into_iter()
            .find_map(|op| rest.strip_prefix(op).map(|rhs| (op, rhs.trim())))
            .ok_or_else(|| Error::new("delete expression is not supported by the Rust backend"))?;
        let rhs = rhs.trim_matches(|c| c == '(' || c == ')').trim();
        let value: f32 = rhs.parse().map_err(|_| Error::new("delete expression is not supported by the Rust backend"))?;
        return Ok(geo.p().as_chunks::<3>().0.iter().map(|q| match op {
            ">=" => q[k] >= value,
            "<=" => q[k] <= value,
            "==" => q[k] == value,
            "!=" => q[k] != value,
            ">" => q[k] > value,
            _ => q[k] < value,
        }).collect());
    }
    Err(Error::new("delete expression is not supported by the Rust backend"))
}

pub(super) fn keep_points(geo: &Geometry, keep: &[bool]) -> Result<Geometry> {
    let npoints = geo.npoints;
    let topo = &geo.topology;
    let nverts = topo.nvertices();
    let nprims = topo.nprims();
    let closed = topo.closed_mask();

    let mut remap = vec![-1i32; npoints];
    let mut new_point_count = 0;
    for (i, &k) in keep.iter().enumerate() {
        if k {
            remap[i] = new_point_count as i32;
            new_point_count += 1;
        }
    }


    let vkeep_pt: Vec<bool> = topo.vertex_point.iter().map(|&v| keep[v as usize]).collect();

    let mut dropped_per_prim = vec![0usize; nprims];
    let mut vert_prim = Vec::with_capacity(nverts);
    for (p, drop_count) in dropped_per_prim.iter_mut().enumerate() {
        let start = topo.prim_offsets[p] as usize;
        let end = topo.prim_offsets[p + 1] as usize;
        vert_prim.extend(std::iter::repeat_n(p, end - start));
        for &vk in &vkeep_pt[start..end] {
            if !vk {
                *drop_count += 1;
            }
        }
    }

    let mut vkeep = vec![false; nverts];
    for i in 0..nverts {
        let p = vert_prim[i];
        vkeep[i] = if closed[p] {
            dropped_per_prim[p] == 0
        } else {
            vkeep_pt[i]
        };
    }

    let mut first = vec![false; nverts];
    for p in 0..nprims {
        let start = topo.prim_offsets[p] as usize;
        let end = topo.prim_offsets[p + 1] as usize;
        if end > start {
            first[start] = true;
        }
    }

    let mut starts_run = vec![false; nverts];
    for i in 0..nverts {
        if vkeep[i] {
            let is_first = first[i] || i == 0 || !vkeep[i - 1];
            starts_run[i] = is_first;
        }
    }

    let mut run = vec![-1i32; nverts];
    let mut run_count = 0;
    for i in 0..nverts {
        if starts_run[i] {
            run_count += 1;
        }
        if vkeep[i] {
            run[i] = (run_count - 1) as i32;
        }
    }

    let mut run_size = vec![0i32; run_count];
    for i in 0..nverts {
        if vkeep[i] {
            run_size[run[i] as usize] += 1;
        }
    }

    let mut run_prim = Vec::with_capacity(run_count);
    for i in 0..nverts {
        if starts_run[i] {
            run_prim.push(vert_prim[i]);
        }
    }

    let mut good = vec![false; run_count];
    for r in 0..run_count {
        let p = run_prim[r];
        good[r] = if closed[p] {
            run_size[r] > 0
        } else {
            run_size[r] >= 2
        };
    }

    for i in 0..nverts {
        if vkeep[i] {
            let r = run[i] as usize;
            vkeep[i] = good[r];
        }
    }

    let new_vp: Vec<i32> = vkeep.iter()
        .zip(topo.vertex_point.iter())
        .filter_map(|(&k, &v)| k.then_some(remap[v as usize]))
        .collect();

    let mut new_offs = Vec::with_capacity(run_count + 1);
    new_offs.push(0);
    let mut acc = 0;
    for r in 0..run_count {
        if good[r] {
            acc += run_size[r];
            new_offs.push(acc);
        }
    }

    let src_prim: Vec<usize> = good.iter().enumerate().filter_map(|(r, &g)| g.then_some(run_prim[r])).collect();

    let new_closed = if topo.closed.is_none() {
        None
    } else {
        Some(src_prim.iter().map(|&p| closed[p]).collect())
    };

    Ok(Geometry { packed: Default::default(), edge: Default::default(),
        npoints: new_point_count,
        topology: Arc::new(Topology {
            vertex_point: new_vp,
            prim_offsets: new_offs,
            closed: new_closed,
        }),
        point: slice_table_mask(&geo.point, keep),
        vertex: slice_table_mask(&geo.vertex, &vkeep),
        prim: slice_table_indices(&geo.prim, &src_prim),
        detail: geo.detail.clone(),
    })
}

fn slice_table_mask(table: &AttrTable, mask: &[bool]) -> AttrTable {
    let mut out = AttrTable::new();
    for (name, attr) in table {
        let w = attr.width;
        let data = match &attr.data {
            AttrData::F32(v) => {
                let mut d = Vec::new();
                for (i, &m) in mask.iter().enumerate() {
                    if m {
                        d.extend_from_slice(&v[i * w..(i + 1) * w]);
                    }
                }
                AttrData::F32(d)
            }
            AttrData::I32(v) => {
                let mut d = Vec::new();
                for (i, &m) in mask.iter().enumerate() {
                    if m {
                        d.extend_from_slice(&v[i * w..(i + 1) * w]);
                    }
                }
                AttrData::I32(d)
            }
        };
        out.insert(name.clone(), Arc::new(Attr { width: w, data }));
    }
    out
}

fn slice_table_indices(table: &AttrTable, indices: &[usize]) -> AttrTable {
    let mut out = AttrTable::new();
    for (name, attr) in table {
        let w = attr.width;
        let data = match &attr.data {
            AttrData::F32(v) => {
                let mut d = Vec::with_capacity(indices.len() * w);
                for &idx in indices {
                    d.extend_from_slice(&v[idx * w..(idx + 1) * w]);
                }
                AttrData::F32(d)
            }
            AttrData::I32(v) => {
                let mut d = Vec::with_capacity(indices.len() * w);
                for &idx in indices {
                    d.extend_from_slice(&v[idx * w..(idx + 1) * w]);
                }
                AttrData::I32(d)
            }
        };
        out.insert(name.clone(), Arc::new(Attr { width: w, data }));
    }
    out
}

