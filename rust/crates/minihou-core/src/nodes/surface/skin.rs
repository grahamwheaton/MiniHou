use super::*;

// ---------------------------------------------------------------------------
// SKIN
// ---------------------------------------------------------------------------

pub(super) fn skin(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = SKIN.one(inputs, 0)?;
    let t = &geo.topology;
    if geo.nprims() < 2 {
        return Err(Error::new("skin needs at least two curves (primitives)"));
    }
    let closed_mask = t.closed_mask();
    let has_closed = closed_mask.iter().any(|&c| c);
    let all_closed = closed_mask.iter().all(|&c| c);
    if has_closed && !all_closed {
        return Err(Error::new("skin: mix of open and closed curves; all curves must be open or all closed"));
    }
    for prim_idx in 0..geo.nprims() {
        let sz = t.prim_offsets[prim_idx + 1] - t.prim_offsets[prim_idx];
        if sz < 2 {
            return Err(Error::new("skin: every curve needs at least two points"));
        }
    }

    let isclosed = all_closed;
    let samples = p.i("samples")?;
    let mut max_sz = 0;
    for prim_idx in 0..geo.nprims() {
        let sz = (t.prim_offsets[prim_idx + 1] - t.prim_offsets[prim_idx]) as usize;
        max_sz = max_sz.max(sz);
    }
    let mut n = if samples > 0 { samples as usize } else { max_sz };
    n = n.max(if isclosed { 3 } else { 2 });

    let loop_param = p.b("loop")?;
    let cap_param = p.b("cap")?;

    let p_slice = geo.p();
    let mut rows: Vec<Vec<[f64; 3]>> = Vec::new();

    for prim_idx in 0..geo.nprims() {
        let start = t.prim_offsets[prim_idx] as usize;
        let end = t.prim_offsets[prim_idx + 1] as usize;
        let mut pts: Vec<[f64; 3]> = (start..end)
            .map(|v| {
                let pt = t.vertex_point[v] as usize;
                [p_slice[pt * 3] as f64, p_slice[pt * 3 + 1] as f64, p_slice[pt * 3 + 2] as f64]
            })
            .collect();
        if isclosed {
            pts.push(pts[0]);
        }
        let mut s_cum = vec![0.0];
        let mut total = 0.0;
        for i in 0..pts.len() - 1 {
            total += len3(sub3(pts[i + 1], pts[i]));
            s_cum.push(total);
        }
        let total = if total > 0.0 { total } else { 1.0 };

        let u_vals: Vec<f64> = if isclosed {
            (0..n).map(|i| i as f64 / n as f64).collect()
        } else {
            linspace(0.0, 1.0, n)
        };

        let mut row = Vec::with_capacity(n);
        for &u in &u_vals {
            let target = u * total;
            let idx = s_cum[..pts.len() - 1]
                .partition_point(|&val| val <= target)
                .saturating_sub(1)
                .min(pts.len() - 2);
            let seg_len = s_cum[idx + 1] - s_cum[idx];
            let frac = if seg_len > 0.0 { ((target - s_cum[idx]) / seg_len).clamp(0.0, 1.0) } else { 0.0 };
            row.push([
                pts[idx][0] * (1.0 - frac) + pts[idx + 1][0] * frac,
                pts[idx][1] * (1.0 - frac) + pts[idx + 1][1] * frac,
                pts[idx][2] * (1.0 - frac) + pts[idx + 1][2] * frac,
            ]);
        }
        rows.push(row);
    }

    let m_curves = rows.len();
    let mut pos = Vec::with_capacity(m_curves * n * 3);
    for row in &rows {
        for pt in row {
            pos.push(pt[0] as f32);
            pos.push(pt[1] as f32);
            pos.push(pt[2] as f32);
        }
    }

    let nu = if isclosed { n } else { n - 1 };
    let nv = if loop_param { m_curves } else { m_curves - 1 };

    let mut quads_vp = Vec::with_capacity(nv * nu * 4);
    for i in 0..nv {
        let i1 = (i + 1) % m_curves;
        for j in 0..nu {
            let j1 = (j + 1) % n;
            quads_vp.push((i * n + j) as i32);
            quads_vp.push((i * n + j1) as i32);
            quads_vp.push((i1 * n + j1) as i32);
            quads_vp.push((i1 * n + j) as i32);
        }
    }
    let nquads = quads_vp.len() / 4;

    let mut verts = quads_vp;
    let mut sizes_out = vec![4i32; nquads];

    if cap_param && isclosed && !loop_param {
        // First cap (reversed)
        for j in (0..n).rev() {
            verts.push(j as i32);
        }
        sizes_out.push(n as i32);

        // Last cap
        for j in 0..n {
            verts.push(((m_curves - 1) * n + j) as i32);
        }
        sizes_out.push(n as i32);
    }

    let mut offsets = vec![0i32];
    for &sz in &sizes_out {
        offsets.push(offsets.last().unwrap() + sz);
    }

    let mut uv = Vec::with_capacity(m_curves * n * 2);
    let u_denom = if isclosed { n as f32 } else { (n - 1) as f32 };
    let v_denom = if loop_param { m_curves as f32 } else { (m_curves - 1) as f32 };
    for i in 0..m_curves {
        let v_val = i as f32 / v_denom;
        for j in 0..n {
            let u_val = j as f32 / u_denom;
            uv.push(u_val);
            uv.push(v_val);
        }
    }

    let mut out = Geometry::from_arrays(pos, verts, offsets, None)?;
    out.point.insert("uv".into(), Arc::new(Attr::f32(2, uv)));

    if p.b("outgrp_surface")? {
        let grp_name = p.s("outgrp_surface_name")?;
        if !grp_name.is_empty() {
            let mut vals = vec![0i32; out.nprims()];
            for v in &mut vals[..nquads] {
                *v = 1;
            }
            out.prim.insert(grp_name, Arc::new(Attr::i32(1, vals)));
        }
    }

    if p.b("outgrp_cap")? {
        let grp_name = p.s("outgrp_cap_name")?;
        if !grp_name.is_empty() {
            let mut vals = vec![0i32; out.nprims()];
            for v in &mut vals[nquads..] {
                *v = 1;
            }
            out.prim.insert(grp_name, Arc::new(Attr::i32(1, vals)));
        }
    }

    Ok(out)
}
