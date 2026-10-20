use super::guess::planar_guess;

pub(super) fn lscm(
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

