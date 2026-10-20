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

pub(crate) fn pack(
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
