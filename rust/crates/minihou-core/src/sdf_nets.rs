//! Dense-grid surface nets, preserving NumPy's axis, crossing and corner order.

fn flat(p: [usize; 3], dims: [usize; 3]) -> usize {
    (p[0] * dims[1] + p[1]) * dims[2] + p[2]
}

fn coords(i: usize, dims: [usize; 3]) -> [usize; 3] {
    [i / (dims[1] * dims[2]), (i / dims[2]) % dims[1], i % dims[2]]
}

fn others(ax: usize) -> [usize; 2] {
    match ax { 0 => [1, 2], 1 => [0, 2], _ => [0, 1] }
}

fn resolve(values: &[f64], dims: [usize; 3], max_rounds: usize, f32_output: bool) -> Vec<f64> {
    assert_eq!(values.len(), dims.iter().product::<usize>());
    let mut v = values.to_vec();
    for _ in 0..max_rounds {
        // The Python signs are a snapshot of the whole round, not recomputed per axis.
        let signs: Vec<bool> = v.iter().map(|&x| x < 0.0).collect();
        let mut found = false;
        for ax in 0..3 {
            let [o0, o1] = others(ax);
            if dims[o0] < 2 || dims[o1] < 2 { continue; }
            let mut faces = dims;
            faces[o0] -= 1;
            faces[o1] -= 1;
            let mut targets = Vec::new();
            for i in 0..faces.iter().product() {
                let p = coords(i, faces);
                let corners = [(0,0), (1,0), (1,1), (0,1)].map(|(d0,d1)| {
                    let mut c = p;
                    c[o0] += d0;
                    c[o1] += d1;
                    flat(c, dims)
                });
                let [a,b,c,d] = corners.map(|j| signs[j]);
                if a == c && b == d && a != b {
                    let mut pick = corners[0];
                    for &j in &corners[1..] {
                        // np.argmin selects the first minimum (including the first NaN).
                        if !v[pick].is_nan() && (v[j].is_nan() || v[j].abs() < v[pick].abs()) { pick = j; }
                    }
                    targets.push(pick);
                }
            }
            if targets.is_empty() { continue; }
            found = true;
            let largest = v.iter().fold(0.0_f64, |a, b| if a.is_nan() || b.is_nan() { f64::NAN } else { a.max(b.abs()) });
            let scale = if largest * 1e-6 > 1.0 { largest * 1e-6 } else { 1.0 };
            // Advanced-index assignment gathers all RHS values before writing, including duplicates.
            let changes: Vec<_> = targets.iter().map(|&j| {
                let value = if v[j] < 0.0 { 1e-7 } else { -1e-7 } * scale;
                if f32_output { (value as f32) as f64 } else { value }
            }).collect();
            for (j, value) in targets.into_iter().zip(changes) { v[j] = value; }
        }
        if !found { break; }
    }
    v
}

/// Resolve face saddles in a copied float64 field.
pub fn resolve_face_saddles(values: &[f64], dims: [usize; 3], max_rounds: usize) -> Vec<f64> {
    resolve(values, dims, max_rounds, false)
}

/// Float32 variant: round assigned nudges after each axis, just like NumPy assignment.
pub fn resolve_face_saddles_f32(values: &[f32], dims: [usize; 3], max_rounds: usize) -> Vec<f32> {
    resolve(&values.iter().map(|&x| x as f64).collect::<Vec<_>>(), dims, max_rounds, true)
        .into_iter().map(|x| x as f32).collect()
}

/// Return flattened float64 points and int64 quad indices in NumPy's exact order.
pub fn surface_nets(values: &[f64], dims: [usize; 3], origin: [f64; 3], h: f64) -> (Vec<f64>, Vec<i64>) {
    let v = resolve_face_saddles(values, dims, 16);
    if dims.iter().any(|&n| n < 2) { return (Vec::new(), Vec::new()); }
    let cells = dims.map(|n| n - 1);
    let nc = cells.iter().product();
    let mut acc = vec![[0.0; 3]; nc];
    let mut cnt = vec![0_u8; nc];
    let mut crossings: [Vec<[usize; 3]>; 3] = std::array::from_fn(|_| Vec::new());
    for (ax, crossed) in crossings.iter_mut().enumerate() {
        let mut edges = dims;
        edges[ax] -= 1;
        for i in 0..edges.iter().product() {
            let p = coords(i, edges);
            let mut end = p;
            end[ax] += 1;
            if (v[flat(p, dims)] < 0.0) != (v[flat(end, dims)] < 0.0) { crossed.push(p); }
        }
        let [o0, o1] = others(ax);
        // Match np.add.at's accumulation order: offsets first, then C-order grid edges.
        for d0 in [0, -1] {
            for d1 in [0, -1] {
                for &p in crossed.iter() {
                    let mut c = p.map(|x| x as isize);
                    c[o0] += d0;
                    c[o1] += d1;
                    if (0..3).any(|a| c[a] < 0 || c[a] >= cells[a] as isize) { continue; }
                    let mut end = p;
                    end[ax] += 1;
                    let a = v[flat(p, dims)];
                    let b = v[flat(end, dims)];
                    let mut position = p.map(|x| x as f64);
                    position[ax] += a / (a - b);
                    let j = flat(c.map(|x| x as usize), cells);
                    for (dst, value) in acc[j].iter_mut().zip(position) { *dst += value; }
                    cnt[j] += 1;
                }
            }
        }
    }
    let mut vid = vec![-1_i64; nc];
    let mut points = Vec::new();
    for j in 0..nc {
        if cnt[j] == 0 { continue; }
        vid[j] = (points.len() / 3) as i64;
        for (a, &base) in origin.iter().enumerate() { points.push(base + (acc[j][a] / cnt[j] as f64) * h); }
    }
    let mut quads = Vec::new();
    for (ax, crossed) in crossings.iter().enumerate() {
        let [o0, o1] = others(ax);
        for &p in crossed {
            if p[o0] < 1 || p[o1] < 1 || p[o0] >= cells[o0] || p[o1] >= cells[o1] || p[ax] >= cells[ax] { continue; }
            let mut q = [(-1, -1), (0, -1), (0, 0), (-1, 0)].map(|(d0, d1)| {
                let mut c = p;
                c[o0] = (c[o0] as isize + d0) as usize;
                c[o1] = (c[o1] as isize + d1) as usize;
                vid[flat(c, cells)]
            });
            let right_handed = (o0 + 1) % 3 == o1;
            let keep = (v[flat(p, dims)] < 0.0) ^ !right_handed;
            if !keep { q.reverse(); }
            quads.extend(q);
        }
    }
    (points, quads)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn single_cell_and_degenerate() {
        let (p, q) = surface_nets(&[-1., 1., 1., 1., 1., 1., 1., 1.], [2;3], [0.;3], 1.);
        assert_eq!(p, vec![1./6.;3]);
        assert!(q.is_empty());
        assert!(surface_nets(&[1.], [1;3], [0.;3], 1.).0.is_empty());
    }
    #[test]
    fn saddle_tie_and_input_unchanged() {
        let input = [-1., 1., 1., -1.];
        assert_eq!(resolve_face_saddles(&input, [1,2,2], 1), vec![1e-7, 1., 1., -1.]);
        assert_eq!(input, [-1., 1., 1., -1.]);
    }
}

