use super::cloud::PcgData;
use super::random::euler_xyz_to_matrix;

// -- Oriented bounding boxes & SAT ----------------------------------------------------------------------------

pub(crate) struct OrientedBoxes {
    pub(crate) c: Vec<[f64; 3]>,
    pub(crate) a: Vec<[[f64; 3]; 3]>, // a[k] is axis k column vector
    pub(crate) h: Vec<[f64; 3]>,
}

pub(crate) fn compute_boxes(d: &PcgData) -> OrientedBoxes {
    let n = d.p.len();
    let mut c = Vec::with_capacity(n);
    let mut a = Vec::with_capacity(n);
    let mut h = Vec::with_capacity(n);

    for i in 0..n {
        let r = euler_xyz_to_matrix(d.rot[i]);
        let s = d.scale[i];
        let lo = [
            d.bounds_min[i][0] * s[0],
            d.bounds_min[i][1] * s[1],
            d.bounds_min[i][2] * s[2],
        ];
        let hi = [
            d.bounds_max[i][0] * s[0],
            d.bounds_max[i][1] * s[1],
            d.bounds_max[i][2] * s[2],
        ];
        let half = [
            (hi[0] - lo[0]).abs() / 2.0,
            (hi[1] - lo[1]).abs() / 2.0,
            (hi[2] - lo[2]).abs() / 2.0,
        ];
        let mid = [
            (lo[0] + hi[0]) / 2.0,
            (lo[1] + hi[1]) / 2.0,
            (lo[2] + hi[2]) / 2.0,
        ];
        let centre = [
            d.p[i][0] + r[0][0] * mid[0] + r[0][1] * mid[1] + r[0][2] * mid[2],
            d.p[i][1] + r[1][0] * mid[0] + r[1][1] * mid[1] + r[1][2] * mid[2],
            d.p[i][2] + r[2][0] * mid[0] + r[2][1] * mid[1] + r[2][2] * mid[2],
        ];
        let flip = [
            if s[0] == 0.0 { 1.0 } else { s[0].signum() },
            if s[1] == 0.0 { 1.0 } else { s[1].signum() },
            if s[2] == 0.0 { 1.0 } else { s[2].signum() },
        ];
        let mut axes = [[0.0; 3]; 3];
        for row in 0..3 {
            for col in 0..3 {
                axes[row][col] = r[row][col] * flip[col];
            }
        }
        c.push(centre);
        a.push(axes);
        h.push(half);
    }
    OrientedBoxes { c, a, h }
}

pub(crate) fn compute_aabbs(b: &OrientedBoxes) -> (Vec<[f64; 3]>, Vec<[f64; 3]>) {
    let n = b.c.len();
    let mut lo = Vec::with_capacity(n);
    let mut hi = Vec::with_capacity(n);
    for i in 0..n {
        let mut ext = [0.0; 3];
        for k in 0..3 {
            ext[k] = b.a[i][k][0].abs() * b.h[i][0]
                + b.a[i][k][1].abs() * b.h[i][1]
                + b.a[i][k][2].abs() * b.h[i][2];
        }
        lo.push([b.c[i][0] - ext[0], b.c[i][1] - ext[1], b.c[i][2] - ext[2]]);
        hi.push([b.c[i][0] + ext[0], b.c[i][1] + ext[1], b.c[i][2] + ext[2]]);
    }
    (lo, hi)
}

pub(crate) fn obb_overlap_one(
    ca: [f64; 3],
    aa: [[f64; 3]; 3],
    ha: [f64; 3],
    cb: [f64; 3],
    ab: [[f64; 3]; 3],
    hb: [f64; 3],
    tol: f64,
) -> bool {
    let mut r = [[0.0; 3]; 3];
    let mut ar = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            let dot = aa[0][i] * ab[0][j] + aa[1][i] * ab[1][j] + aa[2][i] * ab[2][j];
            r[i][j] = dot;
            ar[i][j] = dot.abs() + 1e-12;
        }
    }
    let d = [cb[0] - ca[0], cb[1] - ca[1], cb[2] - ca[2]];
    let mut t = [0.0; 3];
    for i in 0..3 {
        t[i] = aa[0][i] * d[0] + aa[1][i] * d[1] + aa[2][i] * d[2];
    }
    let max_ha = ha[0].max(ha[1]).max(ha[2]);
    let max_hb = hb[0].max(hb[1]).max(hb[2]);
    let scale = max_ha.max(max_hb).max(1e-12) * tol;

    for i in 0..3 {
        let rb = hb[0] * ar[i][0] + hb[1] * ar[i][1] + hb[2] * ar[i][2];
        if t[i].abs() >= ha[i] + rb - scale {
            return false;
        }
    }

    for j in 0..3 {
        let ra = ha[0] * ar[0][j] + ha[1] * ar[1][j] + ha[2] * ar[2][j];
        let t_proj = (t[0] * r[0][j] + t[1] * r[1][j] + t[2] * r[2][j]).abs();
        if t_proj >= ra + hb[j] - scale {
            return false;
        }
    }

    for i in 0..3 {
        let i1 = (i + 1) % 3;
        let i2 = (i + 2) % 3;
        for j in 0..3 {
            let j1 = (j + 1) % 3;
            let j2 = (j + 2) % 3;
            let ra = ha[i1] * ar[i2][j] + ha[i2] * ar[i1][j];
            let rb = hb[j1] * ar[i][j2] + hb[j2] * ar[i][j1];
            let dist = (t[i2] * r[i1][j] - t[i1] * r[i2][j]).abs();
            if dist > ra + rb {
                return false;
            }
        }
    }
    true
}

pub(crate) fn overlapping_pairs(
    ba: &OrientedBoxes,
    bb: Option<&OrientedBoxes>,
) -> (Vec<usize>, Vec<usize>) {
    let is_same = bb.is_none();
    let bb = bb.unwrap_or(ba);
    let (la, ha) = compute_aabbs(ba);
    let (lb, hb) = if is_same {
        (la.clone(), ha.clone())
    } else {
        compute_aabbs(bb)
    };

    let mut out_i = Vec::new();
    let mut out_j = Vec::new();

    for i in 0..ba.c.len() {
        let start_j = if is_same { i + 1 } else { 0 };
        for j in start_j..bb.c.len() {
            if la[i][0] <= hb[j][0]
                && lb[j][0] <= ha[i][0]
                && la[i][1] <= hb[j][1]
                && lb[j][1] <= ha[i][1]
                && la[i][2] <= hb[j][2]
                && lb[j][2] <= ha[i][2]
            {
                if obb_overlap_one(ba.c[i], ba.a[i], ba.h[i], bb.c[j], bb.a[j], bb.h[j], 1e-9) {
                    out_i.push(i);
                    out_j.push(j);
                }
            }
        }
    }
    (out_i, out_j)
}

pub(crate) fn covered_fraction(
    ba: &OrientedBoxes,
    bb: &OrientedBoxes,
    pairs_i: &[usize],
    pairs_j: &[usize],
    samples: usize,
) -> Vec<f64> {
    let n = ba.c.len();
    if pairs_i.is_empty() {
        return vec![0.0; n];
    }
    let mut grid = Vec::new();
    for ix in 0..samples {
        let gx = (ix as f64 + 0.5) / samples as f64 * 2.0 - 1.0;
        for iy in 0..samples {
            let gy = (iy as f64 + 0.5) / samples as f64 * 2.0 - 1.0;
            for iz in 0..samples {
                let gz = (iz as f64 + 0.5) / samples as f64 * 2.0 - 1.0;
                grid.push([gx, gy, gz]);
            }
        }
    }
    let total_samples = grid.len();
    let mut covered = vec![vec![false; total_samples]; n];

    for (&i, &j) in pairs_i.iter().zip(pairs_j.iter()) {
        for (k, g) in grid.iter().enumerate() {
            if covered[i][k] {
                continue;
            }
            let pt = [
                ba.c[i][0]
                    + ba.a[i][0][0] * (g[0] * ba.h[i][0])
                    + ba.a[i][0][1] * (g[1] * ba.h[i][1])
                    + ba.a[i][0][2] * (g[2] * ba.h[i][2]),
                ba.c[i][1]
                    + ba.a[i][1][0] * (g[0] * ba.h[i][0])
                    + ba.a[i][1][1] * (g[1] * ba.h[i][1])
                    + ba.a[i][1][2] * (g[2] * ba.h[i][2]),
                ba.c[i][2]
                    + ba.a[i][2][0] * (g[0] * ba.h[i][0])
                    + ba.a[i][2][1] * (g[1] * ba.h[i][1])
                    + ba.a[i][2][2] * (g[2] * ba.h[i][2]),
            ];
            let diff = [pt[0] - bb.c[j][0], pt[1] - bb.c[j][1], pt[2] - bb.c[j][2]];
            let mut inside = true;
            for c in 0..3 {
                let loc =
                    bb.a[j][0][c] * diff[0] + bb.a[j][1][c] * diff[1] + bb.a[j][2][c] * diff[2];
                if loc.abs() > bb.h[j][c] + 1e-12 {
                    inside = false;
                    break;
                }
            }
            if inside {
                covered[i][k] = true;
            }
        }
    }

    covered
        .into_iter()
        .map(|row| row.iter().filter(|&&b| b).count() as f64 / total_samples as f64)
        .collect()
}

pub fn greedy_independent(
    n: usize,
    edges_i: &[usize],
    edges_j: &[usize],
    priority: &[f64],
) -> Vec<bool> {
    let mut adj = vec![Vec::new(); n];
    for (&i, &j) in edges_i.iter().zip(edges_j.iter()) {
        adj[i].push(j);
        adj[j].push(i);
    }
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| {
        priority[b]
            .partial_cmp(&priority[a])
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.cmp(&b))
    });

    let mut kept = vec![false; n];
    let mut dropped = vec![false; n];
    for v in order {
        if !dropped[v] {
            kept[v] = true;
            for &u in &adj[v] {
                dropped[u] = true;
            }
        }
    }
    kept
}
