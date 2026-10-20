use crate::nodes::model;

pub(super) fn get_pins(pu: &[[f64; 3]], island_of: &[usize], k: usize) -> (Vec<usize>, Vec<[f64; 2]>) {
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

pub(super) fn planar_guess(
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

        let (eig_vals, eig_vecs) = model::eigen(cov);
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
