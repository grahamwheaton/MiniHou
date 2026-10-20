use super::*;

// ---------------------------------------------------------------------------
// REVOLVE
// ---------------------------------------------------------------------------

fn compact_points(geo: Geometry) -> Result<Geometry> {
    let mut used = vec![false; geo.npoints];
    for &v in &geo.topology.vertex_point {
        if v >= 0 && (v as usize) < geo.npoints {
            used[v as usize] = true;
        }
    }
    let mut remap = vec![-1i32; geo.npoints];
    let mut new_pts = Vec::new();
    let mut new_count = 0usize;
    let old_p = geo.p();
    for i in 0..geo.npoints {
        if used[i] {
            remap[i] = new_count as i32;
            new_count += 1;
            new_pts.extend_from_slice(&old_p[i * 3..i * 3 + 3]);
        }
    }
    let new_vp: Vec<i32> = geo.topology.vertex_point.iter().map(|&v| remap[v as usize]).collect();
    let mut out = Geometry::from_arrays(new_pts, new_vp, geo.topology.prim_offsets.clone(), geo.topology.closed.clone())?;
    for (name, attr) in &geo.point {
        if name == "P" {
            continue;
        }
        let mut new_attr = attr.zeros_like(new_count);
        match (&attr.data, &mut new_attr.data) {
            (AttrData::F32(src), AttrData::F32(dst)) => {
                let w = attr.width;
                for i in 0..geo.npoints {
                    if remap[i] >= 0 {
                        let target = remap[i] as usize;
                        dst[target * w..(target + 1) * w].copy_from_slice(&src[i * w..(i + 1) * w]);
                    }
                }
            }
            (AttrData::I32(src), AttrData::I32(dst)) => {
                let w = attr.width;
                for i in 0..geo.npoints {
                    if remap[i] >= 0 {
                        let target = remap[i] as usize;
                        dst[target * w..(target + 1) * w].copy_from_slice(&src[i * w..(i + 1) * w]);
                    }
                }
            }
            _ => unreachable!(),
        }
        out.point.insert(name.clone(), Arc::new(new_attr));
    }
    out.vertex = geo.vertex.clone();
    out.prim = geo.prim.clone();
    out.detail = geo.detail.clone();
    Ok(out)
}

pub(super) fn revolve(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = REVOLVE.one(inputs, 0)?;
    let origin = p.v3("origin")?;
    let axis = p.v3("axis")?;
    let ax_len = len3(axis);
    if ax_len <= 1e-12 {
        return Err(Error::new("revolve needs a non-zero axis"));
    }
    let ax = [axis[0] / ax_len, axis[1] / ax_len, axis[2] / ax_len];

    let angle = p.f("angle")?;
    let divisions = p.i("divisions")?;
    if divisions < 1 {
        return Err(Error::new("divisions must be at least 1"));
    }
    let start_angle = p.f("start_angle")?;
    let end_caps = p.b("end_caps")?;

    let t = &geo.topology;
    let mut usable = Vec::new();
    for prim_idx in 0..geo.nprims() {
        let size = t.prim_offsets[prim_idx + 1] - t.prim_offsets[prim_idx];
        if size >= 2 {
            usable.push(prim_idx);
        }
    }
    if usable.is_empty() {
        return Geometry::from_arrays(vec![], vec![], vec![0], None);
    }

    let full = angle.abs() >= 360.0 - 1e-9;
    let steps = divisions as usize;
    let copies = if full { steps } else { steps + 1 };

    let n = geo.npoints;
    let p_slice = geo.p();
    let mut on_axis = vec![false; n];
    let mut rel = Vec::with_capacity(n);
    let mut along = Vec::with_capacity(n);
    let mut radial = Vec::with_capacity(n);

    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for i in 0..n {
        for k in 0..3 {
            lo[k] = lo[k].min(p_slice[i * 3 + k] as f64);
            hi[k] = hi[k].max(p_slice[i * 3 + k] as f64);
        }
    }
    let ptp_max = if n > 0 {
        (hi[0] - lo[0]).max(hi[1] - lo[1]).max(hi[2] - lo[2]).max(1e-9)
    } else {
        1.0
    };
    let axis_tol = 1e-7 * ptp_max;

    for i in 0..n {
        let p_pt = [p_slice[i * 3] as f64, p_slice[i * 3 + 1] as f64, p_slice[i * 3 + 2] as f64];
        let r_pt = sub3(p_pt, origin);
        let al = dot(r_pt, ax);
        let rad = sub3(r_pt, [al * ax[0], al * ax[1], al * ax[2]]);
        if len3(rad) <= axis_tol {
            on_axis[i] = true;
        }
        rel.push(r_pt);
        along.push(al);
        radial.push(rad);
    }

    let mut pos = Vec::with_capacity(copies * n * 3);
    let mut pid = vec![vec![0usize; n]; copies];

    for c in 0..copies {
        let th = (start_angle + angle * (c as f64 / steps as f64)).to_radians();
        let (sn, cs) = th.sin_cos();
        for i in 0..n {
            if on_axis[i] {
                pid[c][i] = pid[0][i];
            } else {
                pid[c][i] = c * n + i;
            }
            let rad = radial[i];
            let kx = cross(ax, rad);
            let rot = [
                rad[0] * cs + kx[0] * sn,
                rad[1] * cs + kx[1] * sn,
                rad[2] * cs + kx[2] * sn,
            ];
            let pt = [
                origin[0] + along[i] * ax[0] + rot[0],
                origin[1] + along[i] * ax[1] + rot[1],
                origin[2] + along[i] * ax[2] + rot[2],
            ];
            pos.push(pt[0] as f32);
            pos.push(pt[1] as f32);
            pos.push(pt[2] as f32);
        }
    }

    // Profile segments
    struct Segment {
        a: usize,
        b: usize,
        prim: usize,
        ta: f64,
        tb: f64,
    }
    let mut segs = Vec::new();
    for &p_idx in &usable {
        let start = t.prim_offsets[p_idx] as usize;
        let end = t.prim_offsets[p_idx + 1] as usize;
        let is_cl = t.is_closed(p_idx);
        let mut v: Vec<usize> = t.vertex_point[start..end].iter().map(|&x| x as usize).collect();
        if is_cl {
            v.push(v[0]);
        }
        let mut s_cum = vec![0.0];
        let mut total_len = 0.0;
        for i in 0..v.len() - 1 {
            let p0 = [p_slice[v[i] * 3] as f64, p_slice[v[i] * 3 + 1] as f64, p_slice[v[i] * 3 + 2] as f64];
            let p1 = [p_slice[v[i + 1] * 3] as f64, p_slice[v[i + 1] * 3 + 1] as f64, p_slice[v[i + 1] * 3 + 2] as f64];
            total_len += len3(sub3(p1, p0));
            s_cum.push(total_len);
        }
        let norm_s: Vec<f64> = if total_len > 0.0 {
            s_cum.iter().map(|&x| x / total_len).collect()
        } else {
            linspace(0.0, 1.0, v.len())
        };
        for i in 0..v.len() - 1 {
            segs.push(Segment {
                a: v[i],
                b: v[i + 1],
                prim: p_idx,
                ta: norm_s[i],
                tb: norm_s[i + 1],
            });
        }
    }

    let mut flat_v = Vec::new();
    let mut flat_uv = Vec::new();
    let mut sizes_out = Vec::new();
    let mut prim_src = Vec::new();

    for seg in &segs {
        for j in 0..steps {
            let j1 = if full { (j + 1) % copies } else { j + 1 };
            let a0 = pid[j][seg.a];
            let a1 = pid[j1][seg.a];
            let b1 = pid[j1][seg.b];
            let b0 = pid[j][seg.b];

            let eq01 = a0 == a1;
            let eq23 = b1 == b0;
            if eq01 && eq23 {
                continue;
            }

            let u0 = j as f32 / steps as f32;
            let u1 = (j + 1) as f32 / steps as f32;
            let va = seg.ta as f32;
            let vb = seg.tb as f32;

            let (corners, uvs) = if eq01 {
                (vec![a0 as i32, b1 as i32, b0 as i32], vec![[u0, va], [u1, vb], [u0, vb]])
            } else if eq23 {
                (vec![a0 as i32, a1 as i32, b1 as i32], vec![[u0, va], [u1, va], [u1, vb]])
            } else {
                (vec![a0 as i32, a1 as i32, b1 as i32, b0 as i32], vec![[u0, va], [u1, va], [u1, vb], [u0, vb]])
            };

            let mut corners = corners;
            let mut uvs = uvs;
            if angle < 0.0 {
                corners.reverse();
                uvs.reverse();
            }

            sizes_out.push(corners.len() as i32);
            flat_v.extend(corners);
            flat_uv.extend(uvs);
            prim_src.push(seg.prim);
        }
    }

    let nbody = sizes_out.len();
    if end_caps && !full {
        for &p_idx in &usable {
            let start = t.prim_offsets[p_idx] as usize;
            let end = t.prim_offsets[p_idx + 1] as usize;
            let v = &t.vertex_point[start..end];
            if v.len() < 3 {
                continue;
            }
            for (copy, forwards) in [(0, angle >= 0.0), (steps, angle < 0.0)] {
                let ids: Vec<i32> = if forwards {
                    v.iter().map(|&pt| pid[copy][pt as usize] as i32).collect()
                } else {
                    v.iter().rev().map(|&pt| pid[copy][pt as usize] as i32).collect()
                };
                let count = ids.len();
                flat_v.extend(ids);
                flat_uv.extend(vec![[0.0f32, 0.0f32]; count]);
                sizes_out.push(count as i32);
                prim_src.push(p_idx);
            }
        }
    }

    let mut offsets = vec![0i32];
    for &sz in &sizes_out {
        offsets.push(offsets.last().unwrap() + sz);
    }

    let mut out = Geometry::from_arrays(pos, flat_v, offsets, None)?;

    // Point attributes replicated copies times
    for (name, attr) in &geo.point {
        if name == "P" {
            continue;
        }
        let w = attr.width;
        let mut rep = attr.zeros_like(copies * n);
        match (&attr.data, &mut rep.data) {
            (AttrData::F32(src), AttrData::F32(dst)) => {
                for c in 0..copies {
                    dst[c * n * w..(c + 1) * n * w].copy_from_slice(src);
                }
            }
            (AttrData::I32(src), AttrData::I32(dst)) => {
                for c in 0..copies {
                    dst[c * n * w..(c + 1) * n * w].copy_from_slice(src);
                }
            }
            _ => unreachable!(),
        }
        out.point.insert(name.clone(), Arc::new(rep));
    }

    // Primitive attributes gathered
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

    // Vertex uv attribute
    let mut uv_flat = Vec::with_capacity(flat_uv.len() * 2);
    for uv in flat_uv {
        uv_flat.push(uv[0]);
        uv_flat.push(uv[1]);
    }
    out.vertex.insert("uv".into(), Arc::new(Attr::f32(2, uv_flat)));

    if p.b("outgrp_endcaps")? {
        let grp_name = p.s("outgrp_endcaps_name")?;
        if !grp_name.is_empty() {
            let mut vals = vec![0i32; out.nprims()];
            for v in &mut vals[nbody..] {
                *v = 1;
            }
            out.prim.insert(grp_name, Arc::new(Attr::i32(1, vals)));
        }
    }

    compact_points(out)
}
