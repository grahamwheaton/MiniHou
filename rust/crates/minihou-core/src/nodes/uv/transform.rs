use std::sync::Arc;

use crate::{
    error::{Error, Result},
    geometry::{Attr, Geometry},
    nodes::{group, np_random, Inputs, NodeSpec, GEO},
    params::Params,
};

use super::islands::uv_islands;

pub const UV_TRANSFORM: NodeSpec = NodeSpec {
    outputs: &["geometry"],
    cook_multi: None,
    cook_sidecar: None,
    name: "uv_transform",
    inputs: GEO,
    params: crate::params!(
        "uv": "uv";
        "scope": "whole";
        "translate": [0., 0., 0.];
        "rotate": 0.;
        "scale": [1., 1., 1.];
        "flip_u": false;
        "flip_v": false;
        "pivot": "center";
        "pivot_uv": [0.5, 0.5, 0.];
        "align": false;
        "direction": [0., 0., 1.];
        "align_axis": "v";
        "random_translate": [0., 0., 0.];
        "random_rotate": 0.;
        "random_scale": 0.;
        "seed": 0;
        "group": ""
    ),
    cook: transform_uv,
};

fn uv_corners(geo: &Geometry, name: &str) -> Result<Vec<[f64; 2]>> {
    let nv = geo.nvertices();
    if let Some(attr) = geo.vertex.get(name) {
        if attr.width >= 2 {
            if let Some(data) = attr.as_f32() {
                let w = attr.width;
                let mut uvc = Vec::with_capacity(nv);
                for i in 0..nv {
                    uvc.push([data[i * w] as f64, data[i * w + 1] as f64]);
                }
                return Ok(uvc);
            }
        }
    }
    if let Some(attr) = geo.point.get(name) {
        if attr.width >= 2 {
            if let Some(data) = attr.as_f32() {
                let w = attr.width;
                let mut uvc = Vec::with_capacity(nv);
                for &vp in &geo.topology.vertex_point {
                    let pt = vp as usize;
                    uvc.push([data[pt * w] as f64, data[pt * w + 1] as f64]);
                }
                return Ok(uvc);
            }
        }
    }
    Err(Error::new(format!(
        "uv_transform: no uv attribute {:?} (project or unwrap first, e.g. uv_project or uv_flatten)",
        name
    )))
}

fn align_angles(geo: &Geometry, uv: &[[f64; 2]], label: &[usize], count: usize, direction: [f64; 3]) -> Vec<f64> {
    let t = &geo.topology;
    let nprims = geo.nprims();
    let p = geo.p();
    let d = direction;
    let mut acc = vec![[0f64; 2]; count];

    for i in 0..nprims {
        let s = t.prim_offsets[i] as usize;
        let e = t.prim_offsets[i + 1] as usize;
        if e - s < 3 {
            continue;
        }
        let (c0, c1, c2) = (s, s + 1, s + 2);
        let pt0 = t.vertex_point[c0] as usize;
        let pt1 = t.vertex_point[c1] as usize;
        let pt2 = t.vertex_point[c2] as usize;

        let p0 = [p[pt0 * 3] as f64, p[pt0 * 3 + 1] as f64, p[pt0 * 3 + 2] as f64];
        let p1 = [p[pt1 * 3] as f64, p[pt1 * 3 + 1] as f64, p[pt1 * 3 + 2] as f64];
        let p2 = [p[pt2 * 3] as f64, p[pt2 * 3 + 1] as f64, p[pt2 * 3 + 2] as f64];

        let e1 = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
        let e2 = [p2[0] - p0[0], p2[1] - p0[1], p2[2] - p0[2]];

        let u1 = [uv[c1][0] - uv[c0][0], uv[c1][1] - uv[c0][1]];
        let u2 = [uv[c2][0] - uv[c0][0], uv[c2][1] - uv[c0][1]];

        let nrm = [
            e1[1] * e2[2] - e1[2] * e2[1],
            e1[2] * e2[0] - e1[0] * e2[2],
            e1[0] * e2[1] - e1[1] * e2[0],
        ];
        let area = (nrm[0] * nrm[0] + nrm[1] * nrm[1] + nrm[2] * nrm[2]).sqrt();
        let nn = if area > 0.0 {
            [nrm[0] / area, nrm[1] / area, nrm[2] / area]
        } else {
            nrm
        };

        let nn_dot_d = nn[0] * d[0] + nn[1] * d[1] + nn[2] * d[2];
        let dp = [
            d[0] - nn_dot_d * nn[0],
            d[1] - nn_dot_d * nn[1],
            d[2] - nn_dot_d * nn[2],
        ];

        let g11 = e1[0] * e1[0] + e1[1] * e1[1] + e1[2] * e1[2];
        let g12 = e1[0] * e2[0] + e1[1] * e2[1] + e1[2] * e2[2];
        let g22 = e2[0] * e2[0] + e2[1] * e2[1] + e2[2] * e2[2];

        let r1 = dp[0] * e1[0] + dp[1] * e1[1] + dp[2] * e1[2];
        let r2 = dp[0] * e2[0] + dp[1] * e2[1] + dp[2] * e2[2];

        let det = g11 * g22 - g12 * g12;
        let dp_norm = (dp[0] * dp[0] + dp[1] * dp[1] + dp[2] * dp[2]).sqrt();

        let good = det.abs() > 1e-18 && dp_norm > 1e-9;
        if good {
            let a = (r1 * g22 - r2 * g12) / det;
            let b = (g11 * r2 - g12 * r1) / det;
            let w = [a * u1[0] + b * u2[0], a * u1[1] + b * u2[1]];
            let ln = (w[0] * w[0] + w[1] * w[1]).sqrt();
            if ln > 1e-12 {
                let factor = (area / 2.0) / ln;
                let lbl = label[i];
                acc[lbl][0] += w[0] * factor;
                acc[lbl][1] += w[1] * factor;
            }
        }
    }

    acc.iter()
        .map(|a| {
            let ln = (a[0] * a[0] + a[1] * a[1]).sqrt();
            if ln > 1e-12 {
                a[1].atan2(a[0])
            } else {
                f64::NAN
            }
        })
        .collect()
}

fn transform_uv(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = UV_TRANSFORM.one(inputs, 0)?;
    let uv_name = p.s("uv")?;
    let scope = p.e("scope", &["whole", "island", "face"])?;
    let translate = p.v3("translate")?;
    let rotate = p.f("rotate")?;
    let scale = p.v3("scale")?;
    let flip_u = p.b("flip_u")?;
    let flip_v = p.b("flip_v")?;
    let pivot = p.e("pivot", &["center", "custom"])?;
    let pivot_uv = p.v3("pivot_uv")?;
    let align = p.b("align")?;
    let direction = p.v3("direction")?;
    let align_axis = p.e("align_axis", &["v", "u"])?;
    let random_translate = p.v3("random_translate")?;
    let random_rotate = p.f("random_rotate")?;
    let random_scale = p.f("random_scale")?;
    let seed = p.i("seed")?;
    let group = p.s("group")?;

    let uvc = uv_corners(&g, &uv_name)?;
    let nv = g.nvertices();
    let nprims = g.nprims();

    if align {
        let d_norm = (direction[0] * direction[0] + direction[1] * direction[1] + direction[2] * direction[2]).sqrt();
        if d_norm < 1e-12 {
            return Err(Error::new("uv_transform: the direction is zero"));
        }
    }

    let sel = group::mask(&g, "prim", &group)?;
    let mut pv = vec![0usize; nv];
    for prim_idx in 0..nprims {
        let s = g.topology.prim_offsets[prim_idx] as usize;
        let e = g.topology.prim_offsets[prim_idx + 1] as usize;
        for v in s..e {
            pv[v] = prim_idx;
        }
    }

    let on: Vec<bool> = (0..nv).map(|v| sel.get(pv[v]).copied().unwrap_or(false)).collect();
    if !on.iter().any(|&b| b) {
        let mut out = (*g).clone();
        out.point.remove(&uv_name);
        out.prim.remove(&uv_name);
        let mut flat = Vec::with_capacity(nv * 2);
        for v in 0..nv {
            flat.push(uvc[v][0] as f32);
            flat.push(uvc[v][1] as f32);
        }
        out.vertex.insert(uv_name, Arc::new(Attr::f32(2, flat)));
        return Ok(out);
    }

    let (label, count) = match scope.as_str() {
        "whole" => (vec![0usize; nprims], 1usize),
        "face" => ((0..nprims).collect(), nprims),
        _ => {
            let isl = uv_islands(&g, &uvc);
            let c = if nprims == 0 { 0 } else { isl.iter().copied().max().unwrap_or(0) + 1 };
            (isl, c)
        }
    };

    let corner_label: Vec<usize> = (0..nv).map(|v| label[pv[v]]).collect();

    let piv = if pivot == "custom" {
        vec![[pivot_uv[0], pivot_uv[1]]; count]
    } else {
        let mut lo = vec![[f64::INFINITY; 2]; count];
        let mut hi = vec![[f64::NEG_INFINITY; 2]; count];
        for v in 0..nv {
            if on[v] {
                let l = corner_label[v];
                lo[l][0] = lo[l][0].min(uvc[v][0]);
                lo[l][1] = lo[l][1].min(uvc[v][1]);
                hi[l][0] = hi[l][0].max(uvc[v][0]);
                hi[l][1] = hi[l][1].max(uvc[v][1]);
            }
        }
        let mut p = vec![[0f64; 2]; count];
        for l in 0..count {
            if lo[l][0].is_finite() && hi[l][0].is_finite() {
                p[l] = [(lo[l][0] + hi[l][0]) / 2.0, (lo[l][1] + hi[l][1]) / 2.0];
            }
        }
        p
    };

    let mut rot = vec![rotate.to_radians(); count];
    if align {
        let ang = align_angles(&g, &uvc, &label, count, direction);
        let target = if align_axis == "v" { std::f64::consts::FRAC_PI_2 } else { 0.0 };
        for l in 0..count {
            if !ang[l].is_nan() {
                rot[l] += target - ang[l];
            }
        }
    }

    let mut rng = np_random::Rng::new(&[(seed as u32) as u64]);
    let mut rnd = vec![[0f64; 4]; count];
    for item in rnd.iter_mut() {
        for k in 0..4 {
            item[k] = rng.random() * 2.0 - 1.0;
        }
    }

    let mut sc = vec![[scale[0], scale[1]]; count];
    let mut tr = vec![[translate[0], translate[1]]; count];
    for l in 0..count {
        rot[l] += random_rotate.to_radians() * rnd[l][0];
        let sc_rand = 1.0 + random_scale * rnd[l][1];
        sc[l][0] = scale[0] * sc_rand * (if flip_u { -1.0 } else { 1.0 });
        sc[l][1] = scale[1] * sc_rand * (if flip_v { -1.0 } else { 1.0 });
        tr[l][0] = translate[0] + random_translate[0] * rnd[l][2];
        tr[l][1] = translate[1] + random_translate[1] * rnd[l][3];
    }

    let mut new_uv = uvc.clone();
    for v in 0..nv {
        if on[v] {
            let l = corner_label[v];
            let p = piv[l];
            let d0 = (uvc[v][0] - p[0]) * sc[l][0];
            let d1 = (uvc[v][1] - p[1]) * sc[l][1];
            let (sn, cs) = rot[l].sin_cos();
            new_uv[v][0] = d0 * cs - d1 * sn + p[0] + tr[l][0];
            new_uv[v][1] = d0 * sn + d1 * cs + p[1] + tr[l][1];
        }
    }

    let mut out = (*g).clone();
    out.point.remove(&uv_name);
    out.prim.remove(&uv_name);
    let mut flat = Vec::with_capacity(nv * 2);
    for v in 0..nv {
        flat.push(new_uv[v][0] as f32);
        flat.push(new_uv[v][1] as f32);
    }
    out.vertex.insert(uv_name, Arc::new(Attr::f32(2, flat)));
    Ok(out)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn make_test_grid() -> Geometry {
        // 3x3 vertices = 2x2 quads (4 quads), points from x in [-1, 1], y in [-1, 1]
        // point UVs from 0 to 1
        let mut p = Vec::new();
        let mut uv = Vec::new();
        for row in 0..3 {
            let y = -1.0 + (row as f32);
            let v = (row as f32) / 2.0;
            for col in 0..3 {
                let x = -1.0 + (col as f32);
                let u = (col as f32) / 2.0;
                p.extend_from_slice(&[x, y, 0.0]);
                uv.extend_from_slice(&[u, v]);
            }
        }
        let mut vp = Vec::new();
        let mut offs = vec![0i32];
        for row in 0..2 {
            for col in 0..2 {
                let i0 = (row * 3 + col) as i32;
                let i1 = (row * 3 + col + 1) as i32;
                let i2 = ((row + 1) * 3 + col + 1) as i32;
                let i3 = ((row + 1) * 3 + col) as i32;
                vp.extend_from_slice(&[i0, i1, i2, i3]);
                offs.push(vp.len() as i32);
            }
        }
        let mut g = Geometry::from_arrays(p, vp, offs, None).unwrap();
        g.point.insert("uv".into(), Arc::new(Attr::f32(2, uv)));
        g
    }

    #[test]
    fn test_uv_transform_scale_translate_rotate_flip() {
        let grid = make_test_grid();
        let uvc = uv_corners(&grid, "uv").unwrap();

        // 1. scale and translate with custom pivot
        let p = Params::resolve(
            UV_TRANSFORM.params,
            serde_json::json!({
                "scale": [2., 3., 1.],
                "translate": [0.1, 0.2, 0.],
                "pivot": "custom",
                "pivot_uv": [0., 0., 0.]
            })
            .as_object(),
        )
        .unwrap();
        let out = transform_uv(&[vec![Arc::new(grid.clone())]], &p).unwrap();
        let out_uv = out.vertex["uv"].as_f32().unwrap();
        for (i, uv) in uvc.iter().enumerate() {
            let expected_u = uv[0] * 2.0 + 0.1;
            let expected_v = uv[1] * 3.0 + 0.2;
            assert!((out_uv[i * 2] as f64 - expected_u).abs() < 1e-6);
            assert!((out_uv[i * 2 + 1] as f64 - expected_v).abs() < 1e-6);
        }

        // 2. rotate 90 degrees about center
        let p_rot = Params::resolve(
            UV_TRANSFORM.params,
            serde_json::json!({"rotate": 90.0}).as_object(),
        )
        .unwrap();
        let out_rot = transform_uv(&[vec![Arc::new(grid.clone())]], &p_rot).unwrap();
        let rot_uv = out_rot.vertex["uv"].as_f32().unwrap();
        for (i, uv) in uvc.iter().enumerate() {
            let d0 = uv[0] - 0.5;
            let d1 = uv[1] - 0.5;
            let expected_u = -d1 + 0.5;
            let expected_v = d0 + 0.5;
            assert!((rot_uv[i * 2] as f64 - expected_u).abs() < 1e-6);
            assert!((rot_uv[i * 2 + 1] as f64 - expected_v).abs() < 1e-6);
        }

        // 3. flip_u
        let p_flip_u = Params::resolve(
            UV_TRANSFORM.params,
            serde_json::json!({"flip_u": true}).as_object(),
        )
        .unwrap();
        let out_flip_u = transform_uv(&[vec![Arc::new(grid.clone())]], &p_flip_u).unwrap();
        let flip_u_uv = out_flip_u.vertex["uv"].as_f32().unwrap();
        for (i, uv) in uvc.iter().enumerate() {
            let expected_u = 1.0 - uv[0];
            let expected_v = uv[1];
            assert!((flip_u_uv[i * 2] as f64 - expected_u).abs() < 1e-6);
            assert!((flip_u_uv[i * 2 + 1] as f64 - expected_v).abs() < 1e-6);
        }
        assert!(!out_flip_u.point.contains_key("uv"));

        // 4. flip_v
        let p_flip_v = Params::resolve(
            UV_TRANSFORM.params,
            serde_json::json!({"flip_v": true}).as_object(),
        )
        .unwrap();
        let out_flip_v = transform_uv(&[vec![Arc::new(grid.clone())]], &p_flip_v).unwrap();
        let flip_v_uv = out_flip_v.vertex["uv"].as_f32().unwrap();
        for (i, uv) in uvc.iter().enumerate() {
            let expected_u = uv[0];
            let expected_v = 1.0 - uv[1];
            assert!((flip_v_uv[i * 2] as f64 - expected_u).abs() < 1e-6);
            assert!((flip_v_uv[i * 2 + 1] as f64 - expected_v).abs() < 1e-6);
        }
    }

    #[test]
    fn test_uv_transform_islands_and_random() {
        let grid = make_test_grid();
        let uvc = uv_corners(&grid, "uv").unwrap();
        assert_eq!(uv_islands(&grid, &uvc), vec![0, 0, 0, 0]);

        // Random face transform is deterministic with same seed
        let p_rnd1 = Params::resolve(
            UV_TRANSFORM.params,
            serde_json::json!({
                "scope": "face",
                "random_translate": [0.3, 0.3, 0.],
                "random_rotate": 45.0,
                "random_scale": 0.3,
                "seed": 5
            })
            .as_object(),
        )
        .unwrap();
        let out_rnd1 = transform_uv(&[vec![Arc::new(grid.clone())]], &p_rnd1).unwrap();

        let p_rnd2 = Params::resolve(
            UV_TRANSFORM.params,
            serde_json::json!({
                "scope": "face",
                "random_translate": [0.3, 0.3, 0.],
                "random_rotate": 45.0,
                "random_scale": 0.3,
                "seed": 5
            })
            .as_object(),
        )
        .unwrap();
        let out_rnd2 = transform_uv(&[vec![Arc::new(grid.clone())]], &p_rnd2).unwrap();
        assert_eq!(
            out_rnd1.vertex["uv"].as_f32().unwrap(),
            out_rnd2.vertex["uv"].as_f32().unwrap()
        );

        // Different seed produces different results
        let p_rnd3 = Params::resolve(
            UV_TRANSFORM.params,
            serde_json::json!({
                "scope": "face",
                "random_translate": [0.3, 0.3, 0.],
                "random_rotate": 45.0,
                "random_scale": 0.3,
                "seed": 6
            })
            .as_object(),
        )
        .unwrap();
        let out_rnd3 = transform_uv(&[vec![Arc::new(grid.clone())]], &p_rnd3).unwrap();
        let uv1 = out_rnd1.vertex["uv"].as_f32().unwrap();
        let uv3 = out_rnd3.vertex["uv"].as_f32().unwrap();
        let max_diff = uv1
            .iter()
            .zip(uv3.iter())
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(max_diff > 0.01);

        // Group limitation
        let p_grp = Params::resolve(
            UV_TRANSFORM.params,
            serde_json::json!({
                "translate": [1., 0., 0.],
                "scope": "face",
                "group": "0"
            })
            .as_object(),
        )
        .unwrap();
        let out_grp = transform_uv(&[vec![Arc::new(grid.clone())]], &p_grp).unwrap();
        let grp_uv = out_grp.vertex["uv"].as_f32().unwrap();
        // Face 0 moved, faces 1, 2, 3 unmoved
        for v in 0..4 {
            assert!((grp_uv[v * 2] as f64 - (uvc[v][0] + 1.0)).abs() < 1e-6);
        }
        for v in 4..16 {
            assert!((grp_uv[v * 2] as f64 - uvc[v][0]).abs() < 1e-6);
        }
    }

    #[test]
    fn test_uv_transform_align_and_errors() {
        let grid = make_test_grid();
        let p_align_v = Params::resolve(
            UV_TRANSFORM.params,
            serde_json::json!({
                "align": true,
                "direction": [1., 0., 0.],
                "align_axis": "v"
            })
            .as_object(),
        )
        .unwrap();
        let out_align_v = transform_uv(&[vec![Arc::new(grid.clone())]], &p_align_v).unwrap();
        let uv_v = out_align_v.vertex["uv"].as_f32().unwrap();
        // Direction [1, 0, 0] aligned with v means v grows with x (90 degree ccw turn about center 0.5, 0.5)
        let uvc = uv_corners(&grid, "uv").unwrap();
        for (i, uv) in uvc.iter().enumerate() {
            let d0 = uv[0] - 0.5;
            let d1 = uv[1] - 0.5;
            let expected_u = -d1 + 0.5;
            let expected_v = d0 + 0.5;
            assert!((uv_v[i * 2] as f64 - expected_u).abs() < 1e-5);
            assert!((uv_v[i * 2 + 1] as f64 - expected_v).abs() < 1e-5);
        }

        // Missing UV attribute error
        let p_err_uv = Params::resolve(
            UV_TRANSFORM.params,
            serde_json::json!({"uv": "nonexistent"}).as_object(),
        )
        .unwrap();
        assert!(transform_uv(&[vec![Arc::new(grid.clone())]], &p_err_uv).is_err());

        // Zero direction error
        let p_err_dir = Params::resolve(
            UV_TRANSFORM.params,
            serde_json::json!({"align": true, "direction": [0., 0., 0.]}).as_object(),
        )
        .unwrap();
        assert!(transform_uv(&[vec![Arc::new(grid.clone())]], &p_err_dir).is_err());
    }
}
