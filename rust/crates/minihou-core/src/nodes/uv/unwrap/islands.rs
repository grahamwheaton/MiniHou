use crate::{geometry::Geometry, nodes::model};

use super::super::islands::{relabel_components, uf_union};

pub(super) fn face_islands(nprims: usize, nv: usize, pv: &[usize], tw: &[Option<usize>], seam: &[bool]) -> Vec<usize> {
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

pub(super) fn euler(geo: &Geometry, pv: &[usize], nx: &[usize], label: &[usize], k: usize) -> Vec<i64> {
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

pub(super) fn make_discs(
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

            let (eig_vals, eig_vecs) = model::eigen(cov);
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
