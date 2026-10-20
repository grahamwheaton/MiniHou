use super::*;

pub(super) fn ear_clip_triangles(pts: &[usize], pos: &[[f64; 3]], n: [f64; 3]) -> Vec<Vec<usize>> {
    let count = pts.len();
    if count < 3 {
        return Vec::new();
    }
    if count == 3 {
        return vec![pts.to_vec()];
    }

    let mut axis = 2;
    if n[0].abs() > n[1].abs() && n[0].abs() > n[2].abs() {
        axis = 0;
    } else if n[1].abs() > n[2].abs() {
        axis = 1;
    }
    let (ax, ay) = match axis {
        0 => (1, 2),
        1 => (0, 2),
        _ => (0, 1),
    };

    let p2d: Vec<[f64; 2]> = pts.iter().map(|&i| [pos[i][ax], pos[i][ay]]).collect();
    let mut area2d = 0.0;
    for i in 0..count {
        let (p0, p1) = (p2d[i], p2d[(i + 1) % count]);
        area2d += p0[0] * p1[1] - p1[0] * p0[1];
    }
    let sign = if area2d >= 0.0 { 1.0 } else { -1.0 };

    let turn = |a: [f64; 2], b: [f64; 2], c: [f64; 2]| -> f64 {
        sign * ((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]))
    };

    let mut remaining: Vec<usize> = (0..count).collect();
    let mut tris = Vec::new();

    while remaining.len() > 3 {
        let r_len = remaining.len();
        let mut clipped = false;

        for k in 0..r_len {
            let prev = remaining[(k + r_len - 1) % r_len];
            let curr = remaining[k];
            let next = remaining[(k + 1) % r_len];

            if turn(p2d[prev], p2d[curr], p2d[next]) <= 1e-12 {
                continue;
            }

            let mut inside = false;
            for &j in &remaining {
                if j != prev && j != curr && j != next {
                    let p = p2d[j];
                    if turn(p2d[prev], p2d[curr], p) >= -1e-12
                        && turn(p2d[curr], p2d[next], p) >= -1e-12
                        && turn(p2d[next], p2d[prev], p) >= -1e-12
                    {
                        inside = true;
                        break;
                    }
                }
            }
            if !inside {
                tris.push(vec![pts[prev], pts[curr], pts[next]]);
                remaining.remove(k);
                clipped = true;
                break;
            }
        }

        if !clipped {
            let mut best_k = 0;
            let mut best_turn = -1e30;
            for k in 0..r_len {
                let prev = remaining[(k + r_len - 1) % r_len];
                let curr = remaining[k];
                let next = remaining[(k + 1) % r_len];
                let t = turn(p2d[prev], p2d[curr], p2d[next]);
                if t > best_turn {
                    best_turn = t;
                    best_k = k;
                }
            }
            let prev = remaining[(best_k + r_len - 1) % r_len];
            let curr = remaining[best_k];
            let next = remaining[(best_k + 1) % r_len];
            tris.push(vec![pts[prev], pts[curr], pts[next]]);
            remaining.remove(best_k);
        }
    }

    if remaining.len() == 3 {
        tris.push(vec![pts[remaining[0]], pts[remaining[1]], pts[remaining[2]]]);
    }
    tris
}

pub(super) fn auto_layout(pos: &[[f64; 3]]) -> (usize, usize) {
    let n = pos.len();
    if n < 4 {
        return (1, 0);
    }
    let ang: Vec<f64> = (0..n)
        .map(|i| angle(pos[(i + n - 1) % n], pos[i], pos[(i + 1) % n]))
        .collect();
    let arc = |from: usize, len: usize| -> f64 {
        (0..len)
            .map(|k| dist(pos[(from + k) % n], pos[(from + k + 1) % n]))
            .sum::<f64>()
            .max(1e-12)
    };
    let mut best = (f64::INFINITY, 1, 0);
    for s in 1..n / 2 {
        let m = n / 2 - s;
        for o in 0..n {
            let corners = [o, (o + s) % n, (o + s + m) % n, (o + 2 * s + m) % n];
            let mut score = 0.;
            for (i, a) in ang.iter().enumerate() {
                score += (a
                    - if corners.contains(&i) {
                        std::f64::consts::FRAC_PI_2
                    } else {
                        std::f64::consts::PI
                    })
                .abs();
            }
            score += 0.5
                * ((arc(o, s) / arc((o + s) % n, m)).ln().abs()
                    + (arc((o + s + m) % n, s) / arc((o + 2 * s + m) % n, m))
                        .ln()
                        .abs());
            if score < best.0 - 1e-9 {
                best = (score, s, o);
            }
        }
    }
    (best.1, best.2)
}

pub(super) fn params_along(c: &[[f64; 3]]) -> Vec<f64> {
    let mut a = vec![0.];
    for k in 1..c.len() {
        a.push(a[k - 1] + dist(c[k - 1], c[k]));
    }
    let tot = a.last().copied().unwrap_or(1.0).max(1e-12);
    a.iter().map(|x| x / tot).collect()
}

