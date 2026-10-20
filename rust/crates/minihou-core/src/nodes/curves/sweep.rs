use std::sync::Arc;
use super::{SWEEP, Inputs};
use crate::{error::{Error, Result}, geometry::{Attr, AttrData, Geometry}, params::Params};

pub(super) fn normalize3(v: [f64; 3]) -> [f64; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if l > 1e-30 { [v[0] / l, v[1] / l, v[2] / l] } else { [0.0, 0.0, 0.0] }
}

pub(super) fn cross3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

pub(super) fn dot3(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub(super) fn rmf_frames(p: &[[f64; 3]], sizes: &[usize], closed: &[bool]) -> (Vec<[f64; 3]>, Vec<[f64; 3]>) {
    let n = p.len();
    let mut t_vec = vec![[0.0; 3]; n];
    let mut n_vec = vec![[0.0; 3]; n];

    let mut first = 0;
    for (&sz, &cl) in sizes.iter().zip(closed) {
        if sz == 0 {
            continue;
        }
        for k in 0..sz {
            let cur = first + k;
            let prev = if k == 0 { if cl { first + sz - 1 } else { first } } else { cur - 1 };
            let nxt = if k == sz - 1 { if cl { first } else { first + sz - 1 } } else { cur + 1 };
            let d = [p[nxt][0] - p[prev][0], p[nxt][1] - p[prev][1], p[nxt][2] - p[prev][2]];
            t_vec[cur] = normalize3(d);
        }

        let t0 = t_vec[first];
        let helper = if t0[2].abs() < 0.9 { [0.0, 0.0, 1.0] } else { [1.0, 0.0, 0.0] };
        n_vec[first] = normalize3(cross3(cross3(t0, helper), t0));

        for k in 1..sz {
            let cur = first + k;
            let p0 = p[cur - 1];
            let p1 = p[cur];
            let v1 = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
            let c1 = dot3(v1, v1).max(1e-30);
            let n_prev = n_vec[cur - 1];
            let t_prev = t_vec[cur - 1];
            let dot_v1_n = dot3(v1, n_prev);
            let dot_v1_t = dot3(v1, t_prev);
            let r_l = [
                n_prev[0] - (2.0 / c1 * dot_v1_n) * v1[0],
                n_prev[1] - (2.0 / c1 * dot_v1_n) * v1[1],
                n_prev[2] - (2.0 / c1 * dot_v1_n) * v1[2],
            ];
            let t_l = [
                t_prev[0] - (2.0 / c1 * dot_v1_t) * v1[0],
                t_prev[1] - (2.0 / c1 * dot_v1_t) * v1[1],
                t_prev[2] - (2.0 / c1 * dot_v1_t) * v1[2],
            ];
            let v2 = [t_vec[cur][0] - t_l[0], t_vec[cur][1] - t_l[1], t_vec[cur][2] - t_l[2]];
            let c2 = dot3(v2, v2).max(1e-30);
            let dot_v2_rl = dot3(v2, r_l);
            n_vec[cur] = [
                r_l[0] - (2.0 / c2 * dot_v2_rl) * v2[0],
                r_l[1] - (2.0 / c2 * dot_v2_rl) * v2[1],
                r_l[2] - (2.0 / c2 * dot_v2_rl) * v2[2],
            ];
        }

        for k in 0..sz {
            let cur = first + k;
            let d = dot3(n_vec[cur], t_vec[cur]);
            n_vec[cur] = normalize3([
                n_vec[cur][0] - d * t_vec[cur][0],
                n_vec[cur][1] - d * t_vec[cur][1],
                n_vec[cur][2] - d * t_vec[cur][2],
            ]);
        }
        first += sz;
    }
    (t_vec, n_vec)
}

pub(super) fn sweep(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = SWEEP.one(inputs, 0)?;
    let section = inputs.get(1).and_then(|v| v.first());

    let radius = p.f("radius")?;
    let section_scale = p.f("section_scale")?;
    let divisions = p.i("divisions")?;
    if divisions < 3 {
        return Err(Error::new("divisions must be at least 3"));
    }
    let caps = p.b("caps")?;
    let twist = p.f("twist")?;

    let t = &geo.topology;
    let mut usable = Vec::new();
    for prim_idx in 0..geo.nprims() {
        if t.prim_offsets[prim_idx + 1] - t.prim_offsets[prim_idx] >= 2 {
            usable.push(prim_idx);
        }
    }
    if usable.is_empty() {
        return Err(Error::new("sweep needs curves; make them with line, curve, helix, convert_line or resample"));
    }

    let p_slice = geo.p();
    let mut curve_pts: Vec<[f64; 3]> = Vec::new();
    let mut sizes = Vec::new();
    let mut closed = Vec::new();
    let mut vp_indices = Vec::new();
    let mut u_coords = Vec::new();

    for &prim_idx in &usable {
        let start = t.prim_offsets[prim_idx] as usize;
        let end = t.prim_offsets[prim_idx + 1] as usize;
        let is_cl = t.is_closed(prim_idx);
        let count = end - start;
        sizes.push(count);
        closed.push(is_cl);

        let mut pts = Vec::with_capacity(count);
        let mut vps = Vec::with_capacity(count);
        for v in start..end {
            let pt = t.vertex_point[v] as usize;
            vps.push(pt);
            pts.push([p_slice[pt * 3] as f64, p_slice[pt * 3 + 1] as f64, p_slice[pt * 3 + 2] as f64]);
        }

        let mut s_cum = vec![0.0];
        let mut total = 0.0;
        for i in 0..count - 1 {
            let d = (0..3).map(|k| (pts[i + 1][k] - pts[i][k]).powi(2)).sum::<f64>().sqrt();
            total += d;
            s_cum.push(total);
        }
        if is_cl {
            let d = (0..3).map(|k| (pts[0][k] - pts[count - 1][k]).powi(2)).sum::<f64>().sqrt();
            total += d;
        }
        let total = if total > 0.0 { total } else { 1.0 };
        for &s in &s_cum {
            u_coords.push((s / total) as f32);
        }

        curve_pts.extend(pts);
        vp_indices.extend(vps);
    }

    let (t_vec, n_vec) = rmf_frames(&curve_pts, &sizes, &closed);

    // Cross section
    let (sx, sy, sec_closed, vcoord): (Vec<f64>, Vec<f64>, bool, Vec<f64>) = if let Some(sec) = section {
        let st = &sec.topology;
        let prim_sec = (0..sec.nprims()).find(|&pr| st.prim_offsets[pr + 1] - st.prim_offsets[pr] >= 2)
            .ok_or_else(|| Error::new("sweep: the cross-section input has no polygon or curve"))?;
        let start = st.prim_offsets[prim_sec] as usize;
        let end = st.prim_offsets[prim_sec + 1] as usize;
        let sp_slice = sec.p();
        let pts: Vec<[f64; 3]> = (start..end).map(|v| {
            let pt = st.vertex_point[v] as usize;
            [sp_slice[pt * 3] as f64, sp_slice[pt * 3 + 1] as f64, sp_slice[pt * 3 + 2] as f64]
        }).collect();
        let is_cl = st.is_closed(prim_sec);
        let n_sec = pts.len();
        if n_sec < 2 {
            return Err(Error::new("sweep: the cross-section needs at least two points"));
        }
        let c = [
            pts.iter().map(|p| p[0]).sum::<f64>() / n_sec as f64,
            pts.iter().map(|p| p[1]).sum::<f64>() / n_sec as f64,
            pts.iter().map(|p| p[2]).sum::<f64>() / n_sec as f64,
        ];
        let sx: Vec<f64> = pts.iter().map(|p| p[0] - c[0]).collect();
        let sy: Vec<f64> = pts.iter().map(|p| p[1] - c[1]).collect();

        let mut s_cum = vec![0.0];
        let mut total = 0.0;
        for i in 0..n_sec - 1 {
            let d = ((sx[i + 1] - sx[i]).powi(2) + (sy[i + 1] - sy[i]).powi(2)).sqrt();
            total += d;
            s_cum.push(total);
        }
        if is_cl {
            let d = ((sx[0] - sx[n_sec - 1]).powi(2) + (sy[0] - sy[n_sec - 1]).powi(2)).sqrt();
            total += d;
        }
        let total = if total > 0.0 { total } else { 1.0 };
        let mut vc: Vec<f64> = s_cum.iter().map(|&s| s / total).collect();
        if is_cl {
            vc.push(1.0);
        }
        (sx, sy, is_cl && n_sec >= 3, vc)
    } else {
        let d = divisions as usize;
        let mut sx = Vec::with_capacity(d);
        let mut sy = Vec::with_capacity(d);
        let mut vc = Vec::with_capacity(d + 1);
        for i in 0..d {
            let a = i as f64 * 2.0 * std::f64::consts::PI / d as f64;
            sx.push(a.cos());
            sy.push(a.sin());
            vc.push(i as f64 / d as f64);
        }
        vc.push(1.0);
        (sx, sy, true, vc)
    };

    let d_count = sx.len();
    let num_ring_pts = curve_pts.len();
    let mut ring_pts = Vec::with_capacity(num_ring_pts * d_count * 3);

    let width_attr = geo.point.get("width").or_else(|| geo.point.get("pscale"));
    let width_data = width_attr.and_then(|a| a.as_f32());

    for (i, p) in curve_pts.iter().enumerate() {
        let scale = if section.is_none() { radius } else { section_scale };
        let scale = if let Some(wd) = width_data {
            scale * wd[vp_indices[i] * width_attr.unwrap().width] as f64
        } else {
            scale
        };

        let u = u_coords[i] as f64;
        let tw = twist.to_radians() * u;
        let (sn, cs) = tw.sin_cos();

        let n = n_vec[i];
        let bn = cross3(t_vec[i], n);

        for j in 0..d_count {
            let cx = cs * sx[j] - sn * sy[j];
            let cy = sn * sx[j] + cs * sy[j];
            let pt = [
                p[0] + scale * (cx * n[0] + cy * bn[0]),
                p[1] + scale * (cx * n[1] + cy * bn[1]),
                p[2] + scale * (cx * n[2] + cy * bn[2]),
            ];
            ring_pts.push(pt[0] as f32);
            ring_pts.push(pt[1] as f32);
            ring_pts.push(pt[2] as f32);
        }
    }

    let nseg = if sec_closed { d_count } else { d_count - 1 };
    let mut quads = Vec::new();
    let mut uv_quads = Vec::new();
    let mut prim_src = Vec::new();

    let mut curve_start = 0usize;
    for (prim_num, (&sz, &cl)) in sizes.iter().zip(&closed).enumerate() {
        for local_i in 0..sz {
            let has_next = local_i < sz - 1 || cl;
            if !has_next {
                continue;
            }
            let i0 = curve_start + local_i;
            let i1 = if local_i == sz - 1 { curve_start } else { i0 + 1 };

            let u0 = u_coords[i0];
            let u1 = if i1 == curve_start { 1.0f32 } else { u_coords[i1] };

            for js in 0..nseg {
                let j_next = (js + 1) % d_count;
                let a = (i0 * d_count + js) as i32;
                let b = (i0 * d_count + j_next) as i32;
                let c = (i1 * d_count + j_next) as i32;
                let d = (i1 * d_count + js) as i32;

                quads.push([a, b, c, d]);
                let v0 = vcoord[js] as f32;
                let v1 = vcoord[js + 1] as f32;
                uv_quads.push([[u0, v0], [u0, v1], [u1, v1], [u1, v0]]);
                prim_src.push(usable[prim_num]);
            }
        }
        curve_start += sz;
    }

    let mut flat_vp = Vec::new();
    let mut flat_uv = Vec::new();
    let mut sizes_out = Vec::new();

    for (q, uvs) in quads.iter().zip(&uv_quads) {
        flat_vp.extend_from_slice(q);
        for uv in uvs {
            flat_uv.extend_from_slice(uv);
        }
        sizes_out.push(4i32);
    }
    let nq = quads.len();
    let mut ncaps = 0usize;

    if caps && sec_closed {
        let mut c_start = 0usize;
        for (prim_num, (&sz, &cl)) in sizes.iter().zip(&closed).enumerate() {
            if !cl {
                let s_ring = c_start;
                let e_ring = c_start + sz - 1;

                // Start cap (reversed)
                for j in (0..d_count).rev() {
                    flat_vp.push((s_ring * d_count + j) as i32);
                    flat_uv.push(0.5 + 0.5 * (sx[j] / 1.0) as f32);
                    flat_uv.push(0.5 + 0.5 * (sy[j] / 1.0) as f32);
                }
                sizes_out.push(d_count as i32);
                prim_src.push(usable[prim_num]);

                // End cap
                for j in 0..d_count {
                    flat_vp.push((e_ring * d_count + j) as i32);
                    flat_uv.push(0.5 + 0.5 * (sx[j] / 1.0) as f32);
                    flat_uv.push(0.5 + 0.5 * (sy[j] / 1.0) as f32);
                }
                sizes_out.push(d_count as i32);
                prim_src.push(usable[prim_num]);

                ncaps += 1;
            }
            c_start += sz;
        }
    }

    let mut offsets = vec![0i32];
    for &sz in &sizes_out {
        offsets.push(offsets.last().unwrap() + sz);
    }

    let mut out = Geometry::from_arrays(ring_pts, flat_vp, offsets, None)?;

    // Point attributes replicated d_count times
    for (name, attr) in &geo.point {
        if name == "P" || name == "N" {
            continue;
        }
        let w = attr.width;
        let mut rep = attr.zeros_like(num_ring_pts * d_count);
        match (&attr.data, &mut rep.data) {
            (AttrData::F32(src), AttrData::F32(dst)) => {
                for i in 0..num_ring_pts {
                    let orig_pt = vp_indices[i];
                    for j in 0..d_count {
                        let dst_idx = i * d_count + j;
                        dst[dst_idx * w..(dst_idx + 1) * w].copy_from_slice(&src[orig_pt * w..(orig_pt + 1) * w]);
                    }
                }
            }
            (AttrData::I32(src), AttrData::I32(dst)) => {
                for i in 0..num_ring_pts {
                    let orig_pt = vp_indices[i];
                    for j in 0..d_count {
                        let dst_idx = i * d_count + j;
                        dst[dst_idx * w..(dst_idx + 1) * w].copy_from_slice(&src[orig_pt * w..(orig_pt + 1) * w]);
                    }
                }
            }
            _ => unreachable!(),
        }
        out.point.insert(name.clone(), Arc::new(rep));
    }

    let mut curveu = Vec::with_capacity(num_ring_pts * d_count);
    for &u in &u_coords {
        for _ in 0..d_count {
            curveu.push(u);
        }
    }
    out.point.insert("curveu".into(), Arc::new(Attr::f32(1, curveu)));
    out.vertex.insert("uv".into(), Arc::new(Attr::f32(2, flat_uv)));

    // Prim attributes gathered
    for (name, attr) in &geo.prim {
        let w = attr.width;
        let mut gathered = attr.zeros_like(prim_src.len());
        match (&attr.data, &mut gathered.data) {
            (AttrData::F32(src), AttrData::F32(dst)) => {
                for (out_i, &src_i) in prim_src.iter().enumerate() {
                    dst[out_i * w..(out_i + 1) * w].copy_from_slice(&src[src_i * w..(src_i + 1) * w]);
                }
            }
            (AttrData::I32(src), AttrData::I32(dst)) => {
                for (out_i, &src_i) in prim_src.iter().enumerate() {
                    dst[out_i * w..(out_i + 1) * w].copy_from_slice(&src[src_i * w..(src_i + 1) * w]);
                }
            }
            _ => unreachable!(),
        }
        out.prim.insert(name.clone(), Arc::new(gathered));
    }

    if p.b("outgrp_sides")? {
        let grp_name = p.s("outgrp_sides_name")?;
        if !grp_name.is_empty() {
            let mut vals = vec![0i32; out.nprims()];
            for v in &mut vals[..nq] { *v = 1; }
            out.prim.insert(grp_name, Arc::new(Attr::i32(1, vals)));
        }
    }

    if p.b("outgrp_startcap")? {
        let grp_name = p.s("outgrp_startcap_name")?;
        if !grp_name.is_empty() {
            let mut vals = vec![0i32; out.nprims()];
            for i in 0..ncaps {
                vals[nq + i * 2] = 1;
            }
            out.prim.insert(grp_name, Arc::new(Attr::i32(1, vals)));
        }
    }

    if p.b("outgrp_endcap")? {
        let grp_name = p.s("outgrp_endcap_name")?;
        if !grp_name.is_empty() {
            let mut vals = vec![0i32; out.nprims()];
            for i in 0..ncaps {
                vals[nq + i * 2 + 1] = 1;
            }
            out.prim.insert(grp_name, Arc::new(Attr::i32(1, vals)));
        }
    }

    Ok(out)
}
