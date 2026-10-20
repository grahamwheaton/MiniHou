//! UV projection with per-face seam correction (nodes/uv.py).
use std::sync::Arc;
use super::{Inputs,NodeSpec,GEO};
use crate::{error::{Error, Result},geometry::{Attr, AttrData, Geometry},math,params::Params};

mod projection;
pub use projection::UV_PROJECT;
mod islands;
pub use islands::uv_islands;
use islands::{relabel_components, uf_union};

pub const UV_TRANSFORM: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
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

    let sel = super::group::mask(&g, "prim", &group)?;
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

    let mut rng = super::np_random::Rng::new(&[(seed as u32) as u64]);
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

pub const UV_FLATTEN: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "uv_flatten",
    inputs: GEO,
    params: crate::params!(
        "seam_angle": 70.0;
        "split_attrib": "";
        "padding": 0.02;
        "island_attrib": "island"
    ),
    cook: flatten_uv,
};

fn flatten_uv(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = UV_FLATTEN.one(inputs, 0)?;
    if geo.topology.has_open() {
        let n = (0..geo.nprims()).filter(|&i| !geo.topology.is_closed(i)).count();
        return Err(Error::new(format!(
            "uv_flatten works on polygons, but the input has {n} open curve(s); remove them first, e.g. blast with group '@closed == 0'"
        )));
    }
    if geo.nprims() == 0 {
        return Ok((*geo).clone());
    }

    let seam_angle = p.f("seam_angle")?;
    let split_attrib = p.s("split_attrib")?;
    let padding = p.f("padding")?;
    let island_attrib = p.s("island_attrib")?;

    let nprims = geo.nprims();
    let nv = geo.nvertices();
    let topo = &geo.topology;
    let vp = &topo.vertex_point;
    let (pv, nx, _) = super::model::corners(&geo);
    let tw = super::modeling_tools::twins(&geo);

    let pn = crate::normals::prim_normals(geo.p(), topo);
    let un: Vec<[f64; 3]> = pn.iter().map(|n| {
        let mut len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        if len < 1e-20 { len = 1.0; }
        [n[0] / len, n[1] / len, n[2] / len]
    }).collect();

    let cos_thresh = seam_angle.to_radians().cos() - 1e-9;
    let mut seam = vec![false; nv];
    for v in 0..nv {
        match tw[v] {
            None => seam[v] = true,
            Some(tw_v) => {
                let p1 = pv[v];
                let p2 = pv[tw_v];
                let dot = un[p1][0] * un[p2][0] + un[p1][1] * un[p2][1] + un[p1][2] * un[p2][2];
                if dot < cos_thresh {
                    seam[v] = true;
                }
            }
        }
    }

    if !split_attrib.is_empty() {
        let attr = geo.prim.get(&split_attrib).ok_or_else(|| {
            Error::new(format!("no primitive attribute '{split_attrib}' to split islands by"))
        })?;
        let w = attr.width;
        match &attr.data {
            AttrData::F32(data) => {
                for v in 0..nv {
                    if let Some(tw_v) = tw[v] {
                        let p1 = pv[v];
                        let p2 = pv[tw_v];
                        if data[p1 * w..(p1 + 1) * w] != data[p2 * w..(p2 + 1) * w] {
                            seam[v] = true;
                        }
                    }
                }
            }
            AttrData::I32(data) => {
                for v in 0..nv {
                    if let Some(tw_v) = tw[v] {
                        let p1 = pv[v];
                        let p2 = pv[tw_v];
                        if data[p1 * w..(p1 + 1) * w] != data[p2 * w..(p2 + 1) * w] {
                            seam[v] = true;
                        }
                    }
                }
            }
        }
    }

    let init_label = face_islands(nprims, nv, &pv, &tw, &seam);
    let label = make_discs(&geo, &pv, &nx, &tw, &seam, &pn, init_label, 6);
    let k = if label.is_empty() { 0 } else { *label.iter().max().unwrap() + 1 };

    let npoints = geo.npoints.max(1) as u64;
    let mut keys = Vec::with_capacity(nv);
    for v in 0..nv {
        let isl = label[pv[v]] as u64;
        let pt = vp[v] as u64;
        keys.push(isl * npoints + pt);
    }
    let mut ukeys = keys.clone();
    ukeys.sort_unstable();
    ukeys.dedup();
    let nu = ukeys.len();

    let mut uid = Vec::with_capacity(nv);
    for v in 0..nv {
        let idx = ukeys.binary_search(&keys[v]).unwrap();
        uid.push(idx);
    }

    let island_of: Vec<usize> = ukeys.iter().map(|&k| (k / npoints) as usize).collect();
    let p_coords = geo.p();
    let pu: Vec<[f64; 3]> = ukeys.iter().map(|&k| {
        let pt = (k % npoints) as usize;
        [p_coords[pt * 3] as f64, p_coords[pt * 3 + 1] as f64, p_coords[pt * 3 + 2] as f64]
    }).collect();

    let mut tris_v = Vec::new();
    let mut tri_prim = Vec::new();
    for i in 0..nprims {
        let s = topo.prim_offsets[i] as usize;
        let e = topo.prim_offsets[i + 1] as usize;
        let count = (e - s).saturating_sub(2);
        for j in 0..count {
            tris_v.push([s, s + j + 1, s + j + 2]);
            tri_prim.push(i);
        }
    }
    let tris_u: Vec<[usize; 3]> = tris_v.iter().map(|tri| [
        uid[tri[0]],
        uid[tri[1]],
        uid[tri[2]],
    ]).collect();

    let (pins, pin_uv) = get_pins(&pu, &island_of, k);
    let mut uv = lscm(&pu, &tris_u, &pins, &pin_uv, &island_of, k);

    let mut a3_sum = vec![0f64; k];
    let mut a2_sum = vec![0f64; k];
    for (t, tri) in tris_u.iter().enumerate() {
        let u0 = tri[0];
        let u1 = tri[1];
        let u2 = tri[2];
        let p0 = pu[u0];
        let p1 = pu[u1];
        let p2 = pu[u2];
        let e1 = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
        let e2 = [p2[0] - p0[0], p2[1] - p0[1], p2[2] - p0[2]];
        let cross = [
            e1[1] * e2[2] - e1[2] * e2[1],
            e1[2] * e2[0] - e1[0] * e2[2],
            e1[0] * e2[1] - e1[1] * e2[0],
        ];
        let a3 = (cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2]).sqrt() / 2.0;

        let uv0 = uv[u0];
        let uv1 = uv[u1];
        let uv2 = uv[u2];
        let du1 = [uv1[0] - uv0[0], uv1[1] - uv0[1]];
        let du2 = [uv2[0] - uv0[0], uv2[1] - uv0[1]];
        let a2 = (du1[0] * du2[1] - du1[1] * du2[0]) / 2.0;

        let isl = label[tri_prim[t]];
        a3_sum[isl] += a3;
        a2_sum[isl] += a2;
    }

    let mut flip = vec![false; k];
    let mut scale = vec![1.0f64; k];
    for i in 0..k {
        flip[i] = a2_sum[i] < 0.0;
        let abs_a2 = a2_sum[i].abs();
        if abs_a2 > 1e-30 {
            scale[i] = (a3_sum[i] / abs_a2.max(1e-30)).sqrt();
        }
    }

    for u in 0..nu {
        let isl = island_of[u];
        if flip[isl] {
            uv[u][0] = -uv[u][0];
        }
    }

    let uv = pack(uv, &island_of, &scale, k, padding);

    let mut out = (*geo).clone();
    out.point.remove("uv");
    out.prim.remove("uv");

    let mut flat_uv = Vec::with_capacity(nv * 2);
    for v in 0..nv {
        let u_idx = uid[v];
        flat_uv.push(uv[u_idx][0] as f32);
        flat_uv.push(uv[u_idx][1] as f32);
    }
    out.vertex.insert("uv".into(), Arc::new(Attr::f32(2, flat_uv)));

    if !island_attrib.is_empty() {
        let label_i32: Vec<i32> = label.iter().map(|&l| l as i32).collect();
        out.prim.insert(island_attrib, Arc::new(Attr::i32(1, label_i32)));
    }

    Ok(out)
}

fn face_islands(nprims: usize, nv: usize, pv: &[usize], tw: &[Option<usize>], seam: &[bool]) -> Vec<usize> {
    let mut parent: Vec<usize> = (0..nprims).collect();
    for v in 0..nv {
        if let Some(tw_v) = tw[v] {
            if !seam[v] {
                uf_union(&mut parent, pv[v], pv[tw_v]);
            }
        }
    }
    relabel_components(&mut parent, nprims)
}

fn euler(geo: &Geometry, pv: &[usize], nx: &[usize], label: &[usize], k: usize) -> Vec<i64> {
    if k == 0 {
        return Vec::new();
    }
    let nv = geo.nvertices();
    let vp = &geo.topology.vertex_point;
    let mut v_pairs: Vec<(usize, usize)> = (0..nv).map(|v| (label[pv[v]], vp[v] as usize)).collect();
    v_pairs.sort_unstable();
    v_pairs.dedup();
    let mut v_count = vec![0i64; k];
    for (isl, _) in v_pairs {
        v_count[isl] += 1;
    }

    let mut e_pairs: Vec<(usize, usize, usize)> = (0..nv).map(|v| {
        let p0 = vp[v] as usize;
        let p1 = vp[nx[v]] as usize;
        (label[pv[v]], p0.min(p1), p0.max(p1))
    }).collect();
    e_pairs.sort_unstable();
    e_pairs.dedup();
    let mut e_count = vec![0i64; k];
    for (isl, _, _) in e_pairs {
        e_count[isl] += 1;
    }

    let mut f_count = vec![0i64; k];
    for &isl in label {
        f_count[isl] += 1;
    }

    let mut chi = vec![0i64; k];
    for i in 0..k {
        chi[i] = v_count[i] - e_count[i] + f_count[i];
    }
    chi
}

fn make_discs(
    geo: &Geometry,
    pv: &[usize],
    nx: &[usize],
    tw: &[Option<usize>],
    seam: &[bool],
    pn: &[[f64; 3]],
    mut label: Vec<usize>,
    rounds: usize,
) -> Vec<usize> {
    let nprims = geo.nprims();
    let nv = geo.nvertices();
    let vp = &geo.topology.vertex_point;
    let p_coords = geo.p();

    let area: Vec<f64> = pn.iter().map(|n| {
        (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt() / 2.0 + 1e-30
    }).collect();

    let mut cen = vec![[0f64; 3]; nprims];
    for i in 0..nprims {
        let s = geo.topology.prim_offsets[i] as usize;
        let e = geo.topology.prim_offsets[i + 1] as usize;
        let count = (e - s).max(1) as f64;
        let mut sum = [0f64; 3];
        for v in s..e {
            let pt = vp[v] as usize;
            sum[0] += p_coords[pt * 3] as f64;
            sum[1] += p_coords[pt * 3 + 1] as f64;
            sum[2] += p_coords[pt * 3 + 2] as f64;
        }
        cen[i] = [sum[0] / count, sum[1] / count, sum[2] / count];
    }

    let relabel = |lab: &[usize]| -> Vec<usize> {
        let mut parent: Vec<usize> = (0..nprims).collect();
        for v in 0..nv {
            if let Some(tw_v) = tw[v] {
                if !seam[v] && lab[pv[v]] == lab[pv[tw_v]] {
                    uf_union(&mut parent, pv[v], pv[tw_v]);
                }
            }
        }
        relabel_components(&mut parent, nprims)
    };

    for _ in 0..rounds {
        let mut k = if label.is_empty() { 0 } else { *label.iter().max().unwrap() + 1 };
        let chi = euler(geo, pv, nx, &label, k);
        let bad: Vec<usize> = (0..k).filter(|&i| chi[i] != 1).collect();
        if bad.is_empty() {
            break;
        }
        for isl in bad {
            let f: Vec<usize> = (0..nprims).filter(|&p| label[p] == isl).collect();
            if f.len() < 2 {
                continue;
            }
            let mut total_w = 0f64;
            let mut mean_c = [0f64; 3];
            for &p in &f {
                let w = area[p];
                total_w += w;
                mean_c[0] += cen[p][0] * w;
                mean_c[1] += cen[p][1] * w;
                mean_c[2] += cen[p][2] * w;
            }
            let avg_c = [mean_c[0] / total_w, mean_c[1] / total_w, mean_c[2] / total_w];

            let c: Vec<[f64; 3]> = f.iter().map(|&p| [
                cen[p][0] - avg_c[0],
                cen[p][1] - avg_c[1],
                cen[p][2] - avg_c[2],
            ]).collect();

            let mut cov = [[0f64; 3]; 3];
            for (j, &p) in f.iter().enumerate() {
                let w = area[p];
                let cj = c[j];
                for r in 0..3 {
                    for cl in 0..3 {
                        cov[r][cl] += w * cj[r] * cj[cl];
                    }
                }
            }

            let (eig_vals, eig_vecs) = super::model::eigen(cov);
            let mut eig_pairs = [
                (eig_vals[0], [eig_vecs[0][0], eig_vecs[1][0], eig_vecs[2][0]]),
                (eig_vals[1], [eig_vecs[0][1], eig_vecs[1][1], eig_vecs[2][1]]),
                (eig_vals[2], [eig_vecs[0][2], eig_vecs[1][2], eig_vecs[2][2]]),
            ];
            eig_pairs.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

            let test_axes = [
                [1., 0., 0.],
                [0., 1., 0.],
                [0., 0., 1.],
                eig_pairs[0].1,
                eig_pairs[1].1,
                eig_pairs[2].1,
            ];

            let mut best: Option<Vec<bool>> = None;
            let mut best_score: Option<(f64, i64)> = None;

            for ax in test_axes {
                let side: Vec<bool> = c.iter().map(|cj| {
                    (cj[0] * ax[0] + cj[1] * ax[1] + cj[2] * ax[2]) > 0.0
                }).collect();
                if side.iter().all(|&b| b) || !side.iter().any(|&b| b) {
                    continue;
                }
                let mut trial = label.clone();
                for (j, &s) in side.iter().enumerate() {
                    if s {
                        trial[f[j]] = k;
                    }
                }
                let trial = relabel(&trial);
                let kk = if trial.is_empty() { 0 } else { *trial.iter().max().unwrap() + 1 };
                let mut pieces: Vec<usize> = f.iter().map(|&p| trial[p]).collect();
                pieces.sort_unstable();
                pieces.dedup();

                let chi_t = euler(geo, pv, nx, &trial, kk);
                let mut num_ones = 0;
                let mut min_chi = i64::MAX;
                for &pc in &pieces {
                    let val = chi_t[pc];
                    if val == 1 {
                        num_ones += 1;
                    }
                    min_chi = min_chi.min(val);
                }
                let score = (num_ones as f64 - pieces.len() as f64 * 0.01, -(min_chi - 1).abs());
                let is_better = match best_score {
                    None => true,
                    Some(b) => {
                        if score.0 > b.0 {
                            true
                        } else if (score.0 - b.0).abs() < 1e-12 {
                            score.1 > b.1
                        } else {
                            false
                        }
                    }
                };
                if is_better {
                    best = Some(side);
                    best_score = Some(score);
                }
            }

            if let Some(best_side) = best {
                for (j, &s) in best_side.iter().enumerate() {
                    if s {
                        label[f[j]] = k;
                    }
                }
                k += 1;
            }
        }
        label = relabel(&label);
    }
    label
}

fn get_pins(pu: &[[f64; 3]], island_of: &[usize], k: usize) -> (Vec<usize>, Vec<[f64; 2]>) {
    let nu = pu.len();
    let mut starts = vec![0; k + 1];
    let mut cur_isl = 0;
    for i in 0..nu {
        let isl = island_of[i];
        while cur_isl < isl {
            cur_isl += 1;
            starts[cur_isl] = i;
        }
    }
    while cur_isl < k {
        cur_isl += 1;
        starts[cur_isl] = nu;
    }

    let mut pins = Vec::new();
    let mut puv = Vec::new();
    for i in 0..k {
        let s = starts[i];
        let e = starts[i + 1];
        if s == e {
            continue;
        }
        let idx: Vec<usize> = (s..e).collect();
        let mut min_coord = [f64::INFINITY; 3];
        let mut max_coord = [f64::NEG_INFINITY; 3];
        for &u in &idx {
            for d in 0..3 {
                min_coord[d] = min_coord[d].min(pu[u][d]);
                max_coord[d] = max_coord[d].max(pu[u][d]);
            }
        }
        let diff = [
            max_coord[0] - min_coord[0],
            max_coord[1] - min_coord[1],
            max_coord[2] - min_coord[2],
        ];
        let mut ax = 0;
        let mut best_diff = diff[0];
        if diff[1] > best_diff {
            ax = 1;
            best_diff = diff[1];
        }
        if diff[2] > best_diff {
            ax = 2;
        }

        let mut min_val = f64::INFINITY;
        let mut min_u = idx[0];
        let mut max_val = f64::NEG_INFINITY;
        let mut max_u = idx[0];
        for &u in &idx {
            let val = pu[u][ax];
            if val < min_val {
                min_val = val;
                min_u = u;
            }
            if val > max_val {
                max_val = val;
                max_u = u;
            }
        }
        let a = min_u;
        let b = max_u;
        pins.push(a);
        puv.push([0.0, 0.0]);
        if b != a {
            pins.push(b);
            let dist = ((pu[b][0] - pu[a][0]).powi(2)
                + (pu[b][1] - pu[a][1]).powi(2)
                + (pu[b][2] - pu[a][2]).powi(2))
            .sqrt();
            puv.push([dist, 0.0]);
        }
    }
    (pins, puv)
}

fn planar_guess(
    pu: &[[f64; 3]],
    tris_u: &[[usize; 3]],
    nrm: &[[f64; 3]],
    island_of: &[usize],
    pins: &[usize],
    pin_uv: &[[f64; 2]],
    k: usize,
) -> Vec<[f64; 2]> {
    let nu = pu.len();
    let mut out = vec![[0f64; 2]; nu];
    let mut mean_n = vec![[0f64; 3]; k];
    for (t, tri) in tris_u.iter().enumerate() {
        let isl = island_of[tri[0]];
        mean_n[isl][0] += nrm[t][0];
        mean_n[isl][1] += nrm[t][1];
        mean_n[isl][2] += nrm[t][2];
    }

    let mut pin_of = vec![usize::MAX; k];
    let mut pin2_of = vec![usize::MAX; k];
    for (p_idx, &pin) in pins.iter().enumerate().rev() {
        pin_of[island_of[pin]] = p_idx;
    }
    for (p_idx, &pin) in pins.iter().enumerate() {
        pin2_of[island_of[pin]] = p_idx;
    }

    let mut starts = vec![0; k + 1];
    let mut cur_isl = 0;
    for i in 0..nu {
        let isl = island_of[i];
        while cur_isl < isl {
            cur_isl += 1;
            starts[cur_isl] = i;
        }
    }
    while cur_isl < k {
        cur_isl += 1;
        starts[cur_isl] = nu;
    }

    for i in 0..k {
        let s = starts[i];
        let e = starts[i + 1];
        let idx_len = e - s;
        if idx_len < 3 || pin_of[i] == usize::MAX || pin_of[i] == pin2_of[i] {
            continue;
        }
        let pa = pin_of[i];
        let pb = pin2_of[i];

        let mut mean_pos = [0f64; 3];
        for u in s..e {
            mean_pos[0] += pu[u][0];
            mean_pos[1] += pu[u][1];
            mean_pos[2] += pu[u][2];
        }
        mean_pos[0] /= idx_len as f64;
        mean_pos[1] /= idx_len as f64;
        mean_pos[2] /= idx_len as f64;

        let mut cov = [[0f64; 3]; 3];
        for u in s..e {
            let q = [pu[u][0] - mean_pos[0], pu[u][1] - mean_pos[1], pu[u][2] - mean_pos[2]];
            for r in 0..3 {
                for cl in 0..3 {
                    cov[r][cl] += q[r] * q[cl];
                }
            }
        }

        let (eig_vals, eig_vecs) = super::model::eigen(cov);
        let mut eig_pairs = [
            (eig_vals[0], [eig_vecs[0][0], eig_vecs[1][0], eig_vecs[2][0]]),
            (eig_vals[1], [eig_vecs[0][1], eig_vecs[1][1], eig_vecs[2][1]]),
            (eig_vals[2], [eig_vecs[0][2], eig_vecs[1][2], eig_vecs[2][2]]),
        ];
        eig_pairs.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        let a = eig_pairs[0].1;
        let mut b = eig_pairs[1].1;
        let cross = [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ];
        if cross[0] * mean_n[i][0] + cross[1] * mean_n[i][1] + cross[2] * mean_n[i][2] < 0.0 {
            b[0] = -b[0];
            b[1] = -b[1];
            b[2] = -b[2];
        }

        let pin_a_pos = pu[pins[pa]];
        let pin_b_pos = pu[pins[pb]];
        let dpa = [pin_a_pos[0] - mean_pos[0], pin_a_pos[1] - mean_pos[1], pin_a_pos[2] - mean_pos[2]];
        let dpb = [pin_b_pos[0] - mean_pos[0], pin_b_pos[1] - mean_pos[1], pin_b_pos[2] - mean_pos[2]];

        let za = (dpa[0] * a[0] + dpa[1] * a[1] + dpa[2] * a[2], dpa[0] * b[0] + dpa[1] * b[1] + dpa[2] * b[2]);
        let zb = (dpb[0] * a[0] + dpb[1] * a[1] + dpb[2] * a[2], dpb[0] * b[0] + dpb[1] * b[1] + dpb[2] * b[2]);

        let wa = (pin_uv[pa][0], pin_uv[pa][1]);
        let wb = (pin_uv[pb][0], pin_uv[pb][1]);

        let dz = (zb.0 - za.0, zb.1 - za.1);
        let dz_norm_sq = dz.0 * dz.0 + dz.1 * dz.1;
        let dw = (wb.0 - wa.0, wb.1 - wa.1);
        let sim = if dz_norm_sq > 1e-60 {
            (
                (dw.0 * dz.0 + dw.1 * dz.1) / dz_norm_sq,
                (dw.1 * dz.0 - dw.0 * dz.1) / dz_norm_sq,
            )
        } else {
            (1.0, 0.0)
        };

        for u in s..e {
            let q = [pu[u][0] - mean_pos[0], pu[u][1] - mean_pos[1], pu[u][2] - mean_pos[2]];
            let z = (q[0] * a[0] + q[1] * a[1] + q[2] * a[2], q[0] * b[0] + q[1] * b[1] + q[2] * b[2]);
            let z_diff = (z.0 - za.0, z.1 - za.1);
            let sim_prod = (
                sim.0 * z_diff.0 - sim.1 * z_diff.1,
                sim.0 * z_diff.1 + sim.1 * z_diff.0,
            );
            out[u] = [wa.0 + sim_prod.0, wa.1 + sim_prod.1];
        }
    }
    out
}

fn lscm(
    pu: &[[f64; 3]],
    tris_u: &[[usize; 3]],
    pins: &[usize],
    pin_uv: &[[f64; 2]],
    island_of: &[usize],
    k: usize,
) -> Vec<[f64; 2]> {
    let n = pu.len();
    if n == 0 {
        return Vec::new();
    }

    let mut max_p = 0.0f64;
    for p in pu {
        for d in 0..3 {
            max_p = max_p.max(p[d].abs());
        }
    }
    let thresh = 1e-20 * (1.0f64).max(max_p * max_p);

    struct TriData {
        tri: [usize; 3],
        wr: [f64; 3],
        wi: [f64; 3],
        nrm: [f64; 3],
    }
    let mut valid_tris = Vec::with_capacity(tris_u.len());
    for &tri in tris_u {
        let p0 = pu[tri[0]];
        let p1 = pu[tri[1]];
        let p2 = pu[tri[2]];
        let e1 = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
        let e2 = [p2[0] - p0[0], p2[1] - p0[1], p2[2] - p0[2]];
        let nrm = [
            e1[1] * e2[2] - e1[2] * e2[1],
            e1[2] * e2[0] - e1[0] * e2[2],
            e1[0] * e2[1] - e1[1] * e2[0],
        ];
        let dt = (nrm[0] * nrm[0] + nrm[1] * nrm[1] + nrm[2] * nrm[2]).sqrt();
        if dt <= thresh {
            continue;
        }
        let len_e1 = (e1[0] * e1[0] + e1[1] * e1[1] + e1[2] * e1[2]).sqrt().max(1e-300);
        let x = [e1[0] / len_e1, e1[1] / len_e1, e1[2] / len_e1];
        let n_unit = [nrm[0] / dt, nrm[1] / dt, nrm[2] / dt];
        let y = [
            n_unit[1] * x[2] - n_unit[2] * x[1],
            n_unit[2] * x[0] - n_unit[0] * x[2],
            n_unit[0] * x[1] - n_unit[1] * x[0],
        ];
        let xs = [0.0, e1[0] * x[0] + e1[1] * x[1] + e1[2] * x[2], e2[0] * x[0] + e2[1] * x[1] + e2[2] * x[2]];
        let ys = [0.0, 0.0, e2[0] * y[0] + e2[1] * y[1] + e2[2] * y[2]];
        let s = 1.0 / dt.sqrt();
        let wr = [(xs[2] - xs[1]) * s, (xs[0] - xs[2]) * s, (xs[1] - xs[0]) * s];
        let wi = [(ys[2] - ys[1]) * s, (ys[0] - ys[2]) * s, (ys[1] - ys[0]) * s];
        valid_tris.push(TriData { tri, wr, wi, nrm });
    }

    let mut fixed = vec![false; 2 * n];
    let mut xfix = vec![0.0f64; 2 * n];
    for (i, &p) in pins.iter().enumerate() {
        fixed[p] = true;
        fixed[n + p] = true;
        xfix[p] = pin_uv[i][0];
        xfix[n + p] = pin_uv[i][1];
    }

    let mut free = Vec::new();
    let mut col_of = vec![usize::MAX; 2 * n];
    for col in 0..2 * n {
        if !fixed[col] {
            col_of[col] = free.len();
            free.push(col);
        }
    }
    let n_free = free.len();

    let mut sol = xfix.clone();
    if n_free == 0 {
        return (0..n).map(|u| [sol[u], sol[n + u]]).collect();
    }

    let t_count = valid_tris.len();
    let mut atb = vec![0.0f64; n_free];
    let mut mt_entries: Vec<(u32, u32, f64)> = Vec::with_capacity(36 * t_count);

    for tdata in &valid_tris {
        let real_entries = [
            (tdata.tri[0], tdata.wr[0]),
            (tdata.tri[1], tdata.wr[1]),
            (tdata.tri[2], tdata.wr[2]),
            (n + tdata.tri[0], -tdata.wi[0]),
            (n + tdata.tri[1], -tdata.wi[1]),
            (n + tdata.tri[2], -tdata.wi[2]),
        ];
        let mut rhs_r = 0.0f64;
        let mut free_r: [(usize, f64); 6] = [(0, 0.0); 6];
        let mut free_r_len = 0;
        for &(col, val) in &real_entries {
            if fixed[col] {
                rhs_r -= val * xfix[col];
            } else {
                free_r[free_r_len] = (col_of[col], val);
                free_r_len += 1;
            }
        }
        for i in 0..free_r_len {
            atb[free_r[i].0] += free_r[i].1 * rhs_r;
            for j in 0..free_r_len {
                mt_entries.push((free_r[i].0 as u32, free_r[j].0 as u32, free_r[i].1 * free_r[j].1));
            }
        }

        let imag_entries = [
            (tdata.tri[0], tdata.wi[0]),
            (tdata.tri[1], tdata.wi[1]),
            (tdata.tri[2], tdata.wi[2]),
            (n + tdata.tri[0], tdata.wr[0]),
            (n + tdata.tri[1], tdata.wr[1]),
            (n + tdata.tri[2], tdata.wr[2]),
        ];
        let mut rhs_i = 0.0f64;
        let mut free_i: [(usize, f64); 6] = [(0, 0.0); 6];
        let mut free_i_len = 0;
        for &(col, val) in &imag_entries {
            if fixed[col] {
                rhs_i -= val * xfix[col];
            } else {
                free_i[free_i_len] = (col_of[col], val);
                free_i_len += 1;
            }
        }
        for i in 0..free_i_len {
            atb[free_i[i].0] += free_i[i].1 * rhs_i;
            for j in 0..free_i_len {
                mt_entries.push((free_i[i].0 as u32, free_i[j].0 as u32, free_i[i].1 * free_i[j].1));
            }
        }
    }

    mt_entries.sort_by_key(|&(r, c, _)| (r, c));
    let mut csr_row_ptr = vec![0usize; n_free + 1];
    let mut csr_cols = Vec::new();
    let mut csr_vals = Vec::new();
    let mut diag = vec![0.0f64; n_free];

    let mut idx = 0;
    while idx < mt_entries.len() {
        let (r, c, mut v) = mt_entries[idx];
        idx += 1;
        while idx < mt_entries.len() && mt_entries[idx].0 == r && mt_entries[idx].1 == c {
            v += mt_entries[idx].2;
            idx += 1;
        }
        if r == c {
            diag[r as usize] = v;
        }
        csr_cols.push(c as usize);
        csr_vals.push(v);
        csr_row_ptr[r as usize + 1] += 1;
    }
    for i in 0..n_free {
        csr_row_ptr[i + 1] += csr_row_ptr[i];
    }

    let tris_for_guess: Vec<[usize; 3]> = valid_tris.iter().map(|td| td.tri).collect();
    let nrm_for_guess: Vec<[f64; 3]> = valid_tris.iter().map(|td| td.nrm).collect();
    let guess = planar_guess(pu, &tris_for_guess, &nrm_for_guess, island_of, pins, pin_uv, k);
    let mut x0 = vec![0.0f64; n_free];
    for (i, &col) in free.iter().enumerate() {
        if col < n {
            x0[i] = guess[col][0];
        } else {
            x0[i] = guess[col - n][1];
        }
    }

    let mut minv = vec![1.0f64; n_free];
    for i in 0..n_free {
        if diag[i] > 0.0 {
            minv[i] = 1.0 / diag[i];
        }
    }

    let mat_vec = |vec_in: &[f64], vec_out: &mut [f64]| {
        for i in 0..n_free {
            let start = csr_row_ptr[i];
            let end = csr_row_ptr[i + 1];
            let mut sum = 0.0;
            for j in start..end {
                sum += csr_vals[j] * vec_in[csr_cols[j]];
            }
            vec_out[i] = sum;
        }
    };

    let mut atb_minv_atb = 0.0f64;
    for i in 0..n_free {
        atb_minv_atb += atb[i] * minv[i] * atb[i];
    }
    let tol = 1e-7f64;
    let stop = tol * tol * atb_minv_atb.max(1e-300);

    let mut x = x0;
    let mut mx = vec![0.0f64; n_free];
    mat_vec(&x, &mut mx);

    let mut g = vec![0.0f64; n_free];
    let mut z = vec![0.0f64; n_free];
    let mut gz = 0.0f64;
    for i in 0..n_free {
        g[i] = atb[i] - mx[i];
        z[i] = minv[i] * g[i];
        gz += g[i] * z[i];
    }

    let mut p = z.clone();
    let mut mp = vec![0.0f64; n_free];
    let max_iter = 50000;

    for _ in 0..max_iter {
        if gz <= stop {
            break;
        }
        mat_vec(&p, &mut mp);
        let mut p_mp = 0.0f64;
        for i in 0..n_free {
            p_mp += p[i] * mp[i];
        }
        let alpha = gz / p_mp.max(1e-300);
        let mut gz_new = 0.0f64;
        for i in 0..n_free {
            x[i] += alpha * p[i];
            g[i] -= alpha * mp[i];
            z[i] = minv[i] * g[i];
            gz_new += g[i] * z[i];
        }
        let beta = gz_new / gz;
        for i in 0..n_free {
            p[i] = z[i] + beta * p[i];
        }
        gz = gz_new;
    }

    for (i, &col) in free.iter().enumerate() {
        sol[col] = x[i];
    }

    (0..n).map(|u| [sol[u], sol[n + u]]).collect()
}

fn align_island(mut q: Vec<[f64; 2]>) -> Vec<[f64; 2]> {
    let n = q.len();
    if n < 2 {
        return q;
    }

    let ptp_area = |ang: f64| -> f64 {
        let (s, c) = ang.sin_cos();
        let mut min_x = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        for p in &q {
            let x = p[0] * c - p[1] * s;
            let y = p[0] * s + p[1] * c;
            min_x = min_x.min(x);
            max_x = max_x.max(x);
            min_y = min_y.min(y);
            max_y = max_y.max(y);
        }
        (max_x - min_x) * (max_y - min_y)
    };

    let mut best_coarse_ang = 0.0;
    let mut best_coarse_area = f64::INFINITY;
    for deg in 0..90 {
        let ang = (deg as f64).to_radians();
        let area = ptp_area(ang);
        if area < best_coarse_area {
            best_coarse_area = area;
            best_coarse_ang = ang;
        }
    }

    let mut best_ang = best_coarse_ang;
    let mut best_area = f64::INFINITY;
    for step in 0..=100 {
        let offset_deg = -1.0 + (step as f64) * 0.02;
        let ang = best_coarse_ang + offset_deg.to_radians();
        let area = ptp_area(ang);
        if area < best_area {
            best_area = area;
            best_ang = ang;
        }
    }

    let (s, c) = best_ang.sin_cos();
    for p in &mut q {
        let x = p[0] * c - p[1] * s;
        let y = p[0] * s + p[1] * c;
        *p = [x, y];
    }

    let mut min_x = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for p in &q {
        min_x = min_x.min(p[0]);
        max_x = max_x.max(p[0]);
        min_y = min_y.min(p[1]);
        max_y = max_y.max(p[1]);
    }
    if (max_y - min_y) > (max_x - min_x) {
        for p in &mut q {
            *p = [p[1], -p[0]];
        }
    }
    q
}

fn pack(
    mut uv: Vec<[f64; 2]>,
    island_of: &[usize],
    scale: &[f64],
    k: usize,
    padding: f64,
) -> Vec<[f64; 2]> {
    let nu = uv.len();
    let mut starts = vec![0; k + 1];
    let mut cur_isl = 0;
    for i in 0..nu {
        let isl = island_of[i];
        while cur_isl < isl {
            cur_isl += 1;
            starts[cur_isl] = i;
        }
    }
    while cur_isl < k {
        cur_isl += 1;
        starts[cur_isl] = nu;
    }

    let mut boxes = vec![[0f64; 2]; k];
    for i in 0..k {
        let s = starts[i];
        let e = starts[i + 1];
        if s == e {
            continue;
        }
        let count = (e - s) as f64;
        let mut mean_uv = [0f64; 2];
        for u in s..e {
            mean_uv[0] += uv[u][0];
            mean_uv[1] += uv[u][1];
        }
        mean_uv[0] /= count;
        mean_uv[1] /= count;

        let q: Vec<[f64; 2]> = (s..e).map(|u| [
            uv[u][0] - mean_uv[0],
            uv[u][1] - mean_uv[1],
        ]).collect();

        let mut aligned = align_island(q);
        let sc = scale[i];
        let mut min_q = [f64::INFINITY; 2];
        let mut max_q = [f64::NEG_INFINITY; 2];
        for p in &mut aligned {
            p[0] *= sc;
            p[1] *= sc;
            min_q[0] = min_q[0].min(p[0]);
            min_q[1] = min_q[1].min(p[1]);
        }
        for p in &mut aligned {
            p[0] -= min_q[0];
            p[1] -= min_q[1];
            max_q[0] = max_q[0].max(p[0]);
            max_q[1] = max_q[1].max(p[1]);
        }
        for (idx, u) in (s..e).enumerate() {
            uv[u] = aligned[idx];
        }
        boxes[i] = [max_q[0].max(0.0), max_q[1].max(0.0)];
    }

    let total: f64 = boxes.iter().map(|b| b[0] * b[1]).sum();
    let gap = padding * total.max(1e-30).sqrt();
    let max_box_w = boxes.iter().map(|b| b[0]).fold(0.0f64, f64::max);
    let width = (total.sqrt() * 1.15).max(max_box_w);

    let max_box_h = boxes.iter().map(|b| b[1]).fold(0.0f64, f64::max).max(1e-30);
    let tall: Vec<f64> = boxes.iter().map(|b| {
        (b[1] / max_box_h * 1000.0).round() / 1000.0
    }).collect();

    let mut isl_order: Vec<usize> = (0..k).collect();
    isl_order.sort_by(|&a, &b| {
        (-tall[a]).partial_cmp(&(-tall[b])).unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut offs = vec![[0f64; 2]; k];
    let mut x = 0.0f64;
    let mut y = 0.0f64;
    let mut shelf = 0.0f64;

    for &i in &isl_order {
        if x > 0.0 && x + boxes[i][0] > width {
            x = 0.0;
            y = y + shelf + gap;
            shelf = 0.0;
        }
        offs[i] = [x, y];
        x += boxes[i][0] + gap;
        shelf = shelf.max(boxes[i][1]);
    }

    for u in 0..nu {
        let isl = island_of[u];
        uv[u][0] += offs[isl][0];
        uv[u][1] += offs[isl][1];
    }

    let mut max_uv = 0.0f64;
    for p in &uv {
        max_uv = max_uv.max(p[0]).max(p[1]);
    }
    let span = max_uv.max(1e-30);
    let margin = padding / 2.0;
    let factor = (1.0 - 2.0 * margin) / span;

    for p in &mut uv {
        p[0] = margin + p[0] * factor;
        p[1] = margin + p[1] * factor;
    }

    uv
}

#[cfg(test)]
mod tests{
    use super::*;

    fn make_test_grid() -> Geometry {
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

    fn table(v: &serde_json::Value) -> std::collections::BTreeMap<String, Arc<Attr>> {
        v.as_object()
            .unwrap()
            .iter()
            .map(|(name, attr)| {
                let width = attr["width"].as_i64().unwrap() as usize;
                let rows = attr["data"].as_array().unwrap();
                let attr = if attr["dtype"].as_str().unwrap() == "float32" {
                    Attr::f32(
                        width,
                        rows.iter().map(|x| x.as_f64().unwrap() as f32).collect(),
                    )
                } else {
                    Attr::i32(
                        width,
                        rows.iter().map(|x| x.as_i64().unwrap() as i32).collect(),
                    )
                };
                (name.clone(), Arc::new(attr))
            })
            .collect()
    }

    fn geometry_from_json(v: &serde_json::Value) -> Geometry {
        let point = table(&v["point"]);
        let closed = v["closed"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_bool().unwrap())
            .collect();
        let topo = crate::geometry::Topology {
            vertex_point: v["vp"]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_i64().unwrap() as i32)
                .collect(),
            prim_offsets: v["offsets"]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_i64().unwrap() as i32)
                .collect(),
            closed: Some(closed),
        };
        let g = Geometry { packed: Default::default(), edge: Default::default(),
            npoints: point["P"].rows(),
            topology: Arc::new(topo),
            point,
            vertex: table(&v["vertex"]),
            prim: table(&v["prim"]),
            detail: v["detail"].as_object().unwrap().clone(),
        };
        g.validate().unwrap();
        g
    }

    #[test]
    fn test_uv_flatten_matches_python_parity() {
        let cases: serde_json::Value =
            serde_json::from_str(include_str!("../test_data/uv_flatten_parity.json")).unwrap();
        for c in cases.as_array().unwrap() {
            let name = c["name"].as_str().unwrap();
            let spec = &UV_FLATTEN;
            let defaults = c["defaults"].as_object().unwrap();
            assert_eq!(spec.params.len(), defaults.len(), "{name}: default count");
            for (pname, value) in defaults {
                let p = spec.params.iter().find(|p| p.name == pname).unwrap_or_else(|| panic!("missing Python parameter {pname}"));
                assert!(crate::params::same_value(value, &(p.default)()), "{name}: default {}", p.name);
            }
            let inputs: Vec<_> = c["inputs"].as_array().unwrap().iter().map(|v| {
                if v.is_null() { vec![] } else { vec![Arc::new(geometry_from_json(v))] }
            }).collect();
            let result = Params::resolve(spec.params, c["params"].as_object()).and_then(|p| (spec.cook)(&inputs, &p));
            if c.get("error").is_some() {
                assert!(result.is_err(), "{name}: expected error");
                continue;
            }
            let out = result.unwrap_or_else(|e| panic!("{name}: {e}"));
            out.validate().unwrap();
            let expected = geometry_from_json(&c["expected"]);
            assert_eq!(out.npoints, expected.npoints, "{name}: point count");
            assert_eq!(out.topology.vertex_point, expected.topology.vertex_point, "{name}: corner points");
            assert_eq!(out.topology.prim_offsets, expected.topology.prim_offsets, "{name}: offsets");
            assert_eq!(out.topology.closed_mask(), expected.topology.closed_mask(), "{name}: closed flags");

            let out_uv = out.vertex["uv"].as_f32().unwrap();
            let exp_uv = expected.vertex["uv"].as_f32().unwrap();
            assert_eq!(out_uv.len(), exp_uv.len(), "{name}: uv length");
            for i in 0..out_uv.len() {
                assert!(
                    (out_uv[i] - exp_uv[i]).abs() <= 2e-3,
                    "{name}: uv[{i}] Rust {} vs Python {}", out_uv[i], exp_uv[i]
                );
            }

            if expected.prim.contains_key("island") {
                let out_isl = out.prim["island"].as_i32().unwrap();
                let exp_isl = expected.prim["island"].as_i32().unwrap();
                assert_eq!(out_isl, exp_isl, "{name}: island prim attrib");
            }
        }
    }

    #[test]
    fn test_uv_flatten_grid_developable() {
        let grid = make_test_grid();
        let p = Params::resolve(UV_FLATTEN.params, serde_json::json!({}).as_object()).unwrap();
        let out = flatten_uv(&[vec![Arc::new(grid)]], &p).unwrap();
        out.validate().unwrap();
        let uv = out.vertex["uv"].as_f32().unwrap();
        assert!(uv.iter().all(|&v| v >= 0.0 && v <= 1.0));
        assert_eq!(out.prim["island"].as_i32().unwrap(), &[0, 0, 0, 0]);
    }

    #[test]
    fn test_uv_flatten_errors() {
        let grid = make_test_grid();
        // Unknown split attribute
        let p_err_split = Params::resolve(
            UV_FLATTEN.params,
            serde_json::json!({"split_attrib": "nonexistent"}).as_object(),
        )
        .unwrap();
        assert!(flatten_uv(&[vec![Arc::new(grid)]], &p_err_split).is_err());

        // Open curve geometry
        let curve = Geometry::from_arrays(
            vec![0., 0., 0., 1., 1., 1.],
            vec![0, 1],
            vec![0, 2],
            Some(vec![false]),
        )
        .unwrap();
        let p_default = Params::resolve(UV_FLATTEN.params, serde_json::json!({}).as_object()).unwrap();
        let err = flatten_uv(&[vec![Arc::new(curve)]], &p_default).unwrap_err();
        assert!(err.to_string().contains("open curve"));
    }

    #[test]
    fn test_uv_flatten_edge_cases() {
        // 1. Empty geometry (0 points, 0 prims)
        let empty = Geometry::points_only(vec![]).unwrap();
        let p = Params::resolve(UV_FLATTEN.params, serde_json::json!({}).as_object()).unwrap();
        let out_empty = flatten_uv(&[vec![Arc::new(empty)]], &p).unwrap();
        assert_eq!(out_empty.npoints, 0);
        assert_eq!(out_empty.nprims(), 0);

        // 2. Single triangle
        let tri = Geometry::from_arrays(
            vec![0., 0., 0., 1., 0., 0., 0., 1., 0.],
            vec![0, 1, 2],
            vec![0, 3],
            None,
        ).unwrap();
        let out_tri = flatten_uv(&[vec![Arc::new(tri)]], &p).unwrap();
        assert_eq!(out_tri.nprims(), 1);
        let uv_tri = out_tri.vertex["uv"].as_f32().unwrap();
        assert_eq!(uv_tri.len(), 6); // 3 vertices * 2 coords
        for &coord in uv_tri {
            assert!(coord >= 0.0 && coord <= 1.0, "UV in unit square: {coord}");
        }

        // 3. Degenerate zero-area triangle: collinear vertices
        let degen = Geometry::from_arrays(
            vec![0., 0., 0., 1., 0., 0., 2., 0., 0.],
            vec![0, 1, 2],
            vec![0, 3],
            None,
        ).unwrap();
        // Degenerate triangle must either succeed safely with clamped UVs or return an error, never panic or hang
        let _ = flatten_uv(&[vec![Arc::new(degen)]], &p);

        // 4. Determinism
        let grid = make_test_grid();
        let out1 = flatten_uv(&[vec![Arc::new(grid.clone())]], &p).unwrap();
        let out2 = flatten_uv(&[vec![Arc::new(grid.clone())]], &p).unwrap();
        assert_eq!(out1.vertex["uv"].data, out2.vertex["uv"].data);
        assert_eq!(out1.prim["island"].data, out2.prim["island"].data);

        // 5. Attribute preservation
        let mut grid_attr = grid;
        grid_attr.point.insert("Cd".into(), Arc::new(Attr::f32(3, vec![1.0; grid_attr.npoints * 3])));
        grid_attr.prim.insert("material_index".into(), Arc::new(Attr::i32(1, vec![5; grid_attr.nprims()])));
        let out_attr = flatten_uv(&[vec![Arc::new(grid_attr.clone())]], &p).unwrap();
        assert_eq!(out_attr.point["Cd"].data, grid_attr.point["Cd"].data);
        assert_eq!(out_attr.prim["material_index"].data, grid_attr.prim["material_index"].data);
        assert_eq!(out_attr.topology.vertex_point, grid_attr.topology.vertex_point);
    }
}

