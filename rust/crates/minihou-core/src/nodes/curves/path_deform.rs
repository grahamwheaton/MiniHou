use std::sync::Arc;
use super::{cross3, dot3, norm, normalize3, rmf_frames, sub, Inputs, PATH_DEFORM};
use crate::{error::{Error, Result}, geometry::{Attr, Geometry}, params::Params};

pub(super) fn path_deform(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = PATH_DEFORM.one(inputs, 0)?;
    let curve = PATH_DEFORM.one(inputs, 1)?;

    if geo.npoints == 0 {
        return Ok((*geo).clone());
    }
    if curve.nprims() == 0 {
        return Err(Error::new("the curve input is empty"));
    }

    let ct = &curve.topology;
    let start = ct.prim_offsets[0] as usize;
    let end = ct.prim_offsets[1] as usize;
    if end - start < 2 {
        return Err(Error::new("the curve needs at least two points"));
    }

    let cp_slice = curve.p();
    let mut c_pts: Vec<[f64; 3]> = (start..end).map(|v| {
        let pt = ct.vertex_point[v] as usize;
        [cp_slice[pt * 3] as f64, cp_slice[pt * 3 + 1] as f64, cp_slice[pt * 3 + 2] as f64]
    }).collect();

    let is_cl = ct.is_closed(0);
    if is_cl {
        c_pts.push(c_pts[0]);
    }

    let (mut t_vec, mut n_vec) = rmf_frames(&c_pts, &[c_pts.len()], &[false]);

    let mut s_cum = vec![0.0];
    let mut total_len = 0.0;
    for i in 0..c_pts.len() - 1 {
        let d = norm(sub(c_pts[i + 1], c_pts[i]));
        total_len += d;
        s_cum.push(total_len);
    }
    if total_len <= 0.0 {
        return Err(Error::new("the curve has no length"));
    }

    let up = p.v3("up")?;
    let frame = p.e("frame", &["minimal_rotation", "up_vector"])?;
    if frame == "up_vector" {
        for i in 0..t_vec.len() {
            let t = t_vec[i];
            let t_dot_u = dot3(t, up);
            let upp = [up[0] - t_dot_u * t[0], up[1] - t_dot_u * t[1], up[2] - t_dot_u * t[2]];
            if norm(upp) > 1e-6 {
                n_vec[i] = normalize3(upp);
            }
        }
    }

    let t0 = t_vec[0];
    let t0_dot_u = dot3(up, t0);
    let mut upp = [up[0] - t0_dot_u * t0[0], up[1] - t0_dot_u * t0[1], up[2] - t0_dot_u * t0[2]];
    let upp_len = norm(upp);
    if frame == "minimal_rotation" && upp_len > 1e-9 {
        upp = [upp[0] / upp_len, upp[1] / upp_len, upp[2] / upp_len];
        let n0_cross_upp = cross3(n_vec[0], upp);
        let ang = dot3(n0_cross_upp, t0).atan2(dot3(n_vec[0], upp));
        let (sn, cs) = ang.sin_cos();
        for i in 0..n_vec.len() {
            let bv = cross3(t_vec[i], n_vec[i]);
            n_vec[i] = [
                n_vec[i][0] * cs + bv[0] * sn,
                n_vec[i][1] * cs + bv[1] * sn,
                n_vec[i][2] * cs + bv[2] * sn,
            ];
        }
    }

    let axis = p.e("axis", &["x", "y", "z"])?;
    let k = match axis.as_str() { "x" => 0, "y" => 1, "z" => 2, _ => 0 };

    let stretch = p.b("stretch")?;
    let offset = p.f("offset")?;
    let twist = p.f("twist")?;
    let strength = p.f("strength")?;
    if !strength.is_finite() { return Err(Error::new("path_deform strength must be finite")); }
    let selected = super::super::group::mask(&geo, "point", &p.s("group")?)?;

    let p_slice = geo.p();
    let mut min_k = f64::INFINITY;
    let mut max_k = f64::NEG_INFINITY;
    for i in 0..geo.npoints {
        let val = p_slice[i * 3 + k] as f64;
        min_k = min_k.min(val);
        max_k = max_k.max(val);
    }
    let ptp_k = (max_k - min_k).max(1e-12);

    let mut new_p = Vec::with_capacity(geo.npoints * 3);
    let mut new_normals = geo.point.get("N").filter(|a| a.width == 3).map(|_| Vec::with_capacity(geo.npoints * 3));
    let n_slice = geo.point.get("N").and_then(|a| a.as_f32());

    for pt_idx in 0..geo.npoints {
        let orig = [
            p_slice[pt_idx * 3] as f64,
            p_slice[pt_idx * 3 + 1] as f64,
            p_slice[pt_idx * 3 + 2] as f64,
        ];
        if !selected[pt_idx] || strength == 0.0 {
            new_p.extend(orig.map(|v| v as f32));
            if let (Some(n_src), Some(n_dst)) = (n_slice, &mut new_normals) {
                n_dst.extend_from_slice(&n_src[pt_idx * 3..pt_idx * 3 + 3]);
            }
            continue;
        }
        let mut along = orig[k] - min_k;
        if stretch {
            along *= total_len / ptp_k;
        }
        along += offset;

        let i = s_cum[..s_cum.len() - 1]
            .partition_point(|&val| val <= along)
            .saturating_sub(1)
            .min(s_cum.len() - 2);
        let seg_len = (s_cum[i + 1] - s_cum[i]).max(1e-30);
        let f = ((along - s_cum[i]) / seg_len).clamp(0.0, 1.0);

        let mut base = [
            c_pts[i][0] * (1.0 - f) + c_pts[i + 1][0] * f,
            c_pts[i][1] * (1.0 - f) + c_pts[i + 1][1] * f,
            c_pts[i][2] * (1.0 - f) + c_pts[i + 1][2] * f,
        ];
        let tp = normalize3([
            t_vec[i][0] * (1.0 - f) + t_vec[i + 1][0] * f,
            t_vec[i][1] * (1.0 - f) + t_vec[i + 1][1] * f,
            t_vec[i][2] * (1.0 - f) + t_vec[i + 1][2] * f,
        ]);
        let mut np = [
            n_vec[i][0] * (1.0 - f) + n_vec[i + 1][0] * f,
            n_vec[i][1] * (1.0 - f) + n_vec[i + 1][1] * f,
            n_vec[i][2] * (1.0 - f) + n_vec[i + 1][2] * f,
        ];
        let dot_np_tp = dot3(np, tp);
        np = normalize3([
            np[0] - dot_np_tp * tp[0],
            np[1] - dot_np_tp * tp[1],
            np[2] - dot_np_tp * tp[2],
        ]);

        let extra = if along < 0.0 {
            along
        } else if along > total_len {
            along - total_len
        } else {
            0.0
        };
        base = [
            base[0] + tp[0] * extra,
            base[1] + tp[1] * extra,
            base[2] + tp[2] * extra,
        ];

        if twist != 0.0 {
            let a = twist.to_radians() * along.clamp(0.0, total_len) / total_len;
            let (sn, cs) = a.sin_cos();
            let bp = cross3(tp, np);
            np = [
                np[0] * cs + bp[0] * sn,
                np[1] * cs + bp[1] * sn,
                np[2] * cs + bp[2] * sn,
            ];
        }
        let bp = cross3(tp, np);

        let (col0, col1, col2) = match axis.as_str() {
            "x" => (tp, [-bp[0], -bp[1], -bp[2]], np),
            "y" => (bp, tp, np),
            "z" => ([-bp[0], -bp[1], -bp[2]], np, tp),
            _ => unreachable!(),
        };

        let mut local = orig;
        local[k] = 0.0;

        let res_pt = [
            base[0] + col0[0] * local[0] + col1[0] * local[1] + col2[0] * local[2],
            base[1] + col0[1] * local[0] + col1[1] * local[1] + col2[1] * local[2],
            base[2] + col0[2] * local[0] + col1[2] * local[1] + col2[2] * local[2],
        ];
        for k in 0..3 { new_p.push((orig[k] + strength * (res_pt[k] - orig[k])) as f32); }

        if let (Some(n_src), Some(n_dst)) = (n_slice, &mut new_normals) {
            let norm_orig = [
                n_src[pt_idx * 3] as f64,
                n_src[pt_idx * 3 + 1] as f64,
                n_src[pt_idx * 3 + 2] as f64,
            ];
            let res_norm = [
                col0[0] * norm_orig[0] + col1[0] * norm_orig[1] + col2[0] * norm_orig[2],
                col0[1] * norm_orig[0] + col1[1] * norm_orig[1] + col2[1] * norm_orig[2],
                col0[2] * norm_orig[0] + col1[2] * norm_orig[1] + col2[2] * norm_orig[2],
            ];
            for k in 0..3 { n_dst.push((norm_orig[k] + strength * (res_norm[k] - norm_orig[k])) as f32); }
        }
    }

    let mut out = (*geo).clone();
    out.set_p(new_p);
    if let Some(n_dst) = new_normals {
        out.point.insert("N".into(), Arc::new(Attr::f32(3, n_dst)));
    }
    Ok(out)
}
