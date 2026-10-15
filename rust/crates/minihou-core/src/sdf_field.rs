//! Dense field operations, with the arithmetic and visitation order of `minihou.sdf`.

pub struct Field {
    pub origin: [f64; 3],
    pub dims: [usize; 3],
    pub values: Vec<f32>,
}

// NumPy's contiguous float64 reduction uses eight accumulators in blocks of 128.
fn numpy_sum(a: &[f64]) -> f64 {
    if a.len() < 8 {
        return a.iter().fold(-0.0, |s, x| s + x);
    }
    if a.len() <= 128 {
        let mut r = [0.0; 8];
        r.copy_from_slice(&a[..8]);
        let end = a.len() - a.len() % 8;
        for chunk in a[8..end].as_chunks::<8>().0.iter() {
            for j in 0..8 {
                r[j] += chunk[j];
            }
        }
        let mut s = ((r[0] + r[1]) + (r[2] + r[3])) + ((r[4] + r[5]) + (r[6] + r[7]));
        for x in &a[end..] {
            s += x;
        }
        return s;
    }
    let mid = (a.len() / 2) & !7;
    numpy_sum(&a[..mid]) + numpy_sum(&a[mid..])
}

/// Metaball field. Points are xyz triples; radii have already been broadcast to point count.
pub fn from_points(p: &[f64], radius: &[f64], h: f64, blend: f64) -> Result<Field, String> {
    if p.is_empty() {
        return Err("zero-size array to reduction operation minimum which has no identity".into());
    }
    if !p.len().is_multiple_of(3) || radius.len() != p.len() / 3 {
        return Err("points and radius shapes disagree".into());
    }
    if !h.is_finite() || h <= 0.0 {
        return Err("voxel size must be finite and positive".into());
    }
    let blend = blend.max(0.0);
    let support: Vec<f64> = radius.iter().map(|r| r * (1.0 + blend)).collect();
    let reach_world = support.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if !reach_world.is_finite() || reach_world < 0.0 || p.iter().any(|x| !x.is_finite()) {
        return Err("points and kernel support must be finite; support must be nonnegative".into());
    }
    let mut origin = [0.0; 3];
    let mut dims = [0; 3];
    for ax in 0..3 {
        let lo = p.as_chunks::<3>().0.iter().map(|p| p[ax]).fold(f64::INFINITY, f64::min);
        let hi = p.as_chunks::<3>().0.iter().map(|p| p[ax]).fold(f64::NEG_INFINITY, f64::max);
        origin[ax] = (lo - reach_world) - 2.0 * h;
        let upper = (hi + reach_world) + 2.0 * h;
        let d = ((upper - origin[ax]) / h).floor() + 2.0;
        if !(0.0..=24_000_000.0).contains(&d) {
            return Err("too many voxels; use a larger voxel size".into());
        }
        dims[ax] = d as usize;
    }
    let total = dims.iter().try_fold(1usize, |n, d| n.checked_mul(*d))
        .filter(|n| *n <= 24_000_000)
        .ok_or("too many voxels; use a larger voxel size")?;
    let m = (reach_world / h).ceil() as usize + 1;
    let width = m.checked_mul(2).and_then(|n| n.checked_add(2)).ok_or("too much point work")?;
    let offsets = width.checked_pow(3).ok_or("too much point work")?;
    if offsets.checked_mul(radius.len()).is_none_or(|n| n > 150_000_000) {
        return Err("too much point work; use a larger voxel_size or a smaller radius".into());
    }
    let mut field = vec![0.0f64; total];
    let bases: Vec<[i64; 3]> = p.as_chunks::<3>().0.iter().map(|p| {
        std::array::from_fn(|ax| ((p[ax] - origin[ax]) / h).floor() as i64)
    }).collect();
    // np.array_split uses this many chunks, placing the longer chunks first.
    // add.at traverses each chunk in point-major order, which matters where kernels overlap.
    let chunks = (offsets / 64).max(1);
    let short = offsets / chunks;
    let extra = offsets % chunks;
    let mut start = 0;
    for chunk in 0..chunks {
        let end = start + short + usize::from(chunk < extra);
        for ((p, base), support) in p.as_chunks::<3>().0.iter().zip(&bases).zip(&support) {
            let support2 = support * support;
            for index in start..end {
                let off = [index / (width * width), (index / width) % width, index % width];
                let ijk: [i64; 3] = std::array::from_fn(|ax| base[ax] + off[ax] as i64 - m as i64);
                if (0..3).any(|ax| ijk[ax] < 0 || ijk[ax] >= dims[ax] as i64) {
                    continue;
                }
                let d: [f64; 3] = std::array::from_fn(|ax| (origin[ax] + ijk[ax] as f64 * h) - p[ax]);
                let q2 = ((d[0] * d[0] + d[1] * d[1]) + d[2] * d[2]) / support2;
                if q2 < 1.0 {
                    let w = (1.0 - q2).powf(3.0);
                    let flat = (ijk[0] as usize * dims[1] + ijk[1] as usize) * dims[2] + ijk[2] as usize;
                    field[flat] += w;
                }
            }
        }
        start = end;
    }
    let thr = (1.0 - (1.0 / (1.0 + blend)).powf(2.0)).powf(3.0);
    let scale = (numpy_sum(radius) / radius.len() as f64) / thr.max(1e-6);
    let values = field.into_iter().map(|f| ((thr - f) * scale) as f32).collect();
    Ok(Field { origin, dims, values })
}

/// Repeated separable edge-padded three-tap means, rounding after every float32 operation.
pub fn blur(values: &[f32], dims: [usize; 3], iterations: usize) -> Vec<f32> {
    let mut v = values.to_vec();
    let mut out = vec![0.0; v.len()];
    let strides = [dims[1] * dims[2], dims[2], 1];
    for _ in 0..iterations {
        for ax in 0..3 {
            let stride = strides[ax];
            for i in 0..v.len() {
                let coordinate = (i / stride) % dims[ax];
                let left = if coordinate == 0 { i } else { i - stride };
                let right = if coordinate + 1 == dims[ax] { i } else { i + stride };
                out[i] = ((v[left] + v[i]) + v[right]) / 3.0;
            }
            std::mem::swap(&mut v, &mut out);
        }
    }
    v
}

/// Copy the overlapping integer-aligned region; the wrapper computes NumPy's round-to-even offset.
pub fn resample_to(values: &[f32], src_dims: [usize; 3], offset: [i64; 3], dims: [usize; 3], fill: f32) -> Vec<f32> {
    let mut out = vec![fill; dims.iter().product()];
    let start: [i64; 3] = std::array::from_fn(|ax| offset[ax].max(0));
    let end: [i64; 3] = std::array::from_fn(|ax| (offset[ax] + dims[ax] as i64).min(src_dims[ax] as i64));
    if (0..3).any(|ax| end[ax] <= start[ax]) {
        return out;
    }
    for x in start[0]..end[0] {
        for y in start[1]..end[1] {
            let src = (x as usize * src_dims[1] + y as usize) * src_dims[2] + start[2] as usize;
            let dst = ((x - offset[0]) as usize * dims[1] + (y - offset[1]) as usize) * dims[2] + (start[2] - offset[2]) as usize;
            let count = (end[2] - start[2]) as usize;
            out[dst..dst + count].copy_from_slice(&values[src..src + count]);
        }
    }
    out
}

macro_rules! smooth_min_impl {
    ($name:ident, $float:ty) => {
        /// Polynomial smooth minimum on already-broadcast inputs.
        pub fn $name(a: &[$float], b: &[$float], k: $float) -> Vec<$float> {
            a.iter().zip(b).map(|(&a, &b)| {
                if k <= 0.0 {
                    // np.minimum propagates NaN and selects the second operand on ties (including signed zero).
                    return if a.is_nan() || a < b { a } else { b };
                }
                let h = (0.5 + (0.5 * (b - a)) / k).clamp(0.0, 1.0);
                (b * (1.0 - h) + a * h) - ((k * h) * (1.0 - h))
            }).collect()
        }
    };
}
smooth_min_impl!(smooth_min_f32, f32);
smooth_min_impl!(smooth_min_f64, f64);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn singleton_blur_and_padding() {
        assert_eq!(blur(&[3.0], [1, 1, 1], 3), vec![3.0]);
        let out = resample_to(&[3.0], [1, 1, 1], [-1, -1, -1], [3, 3, 3], 9.0);
        assert_eq!(out[13], 3.0);
        assert_eq!(out.iter().filter(|&&x| x == 9.0).count(), 26);
    }

    #[test]
    fn empty_points_rejected() {
        assert!(from_points(&[], &[], 1.0, 1.0).is_err());
    }

    #[test]
    fn polynomial_minimum() {
        assert_eq!(smooth_min_f32(&[0.0, 2.0], &[0.0, 1.0], 1.0), vec![-0.25, 1.0]);
        assert_eq!(smooth_min_f64(&[0.0], &[-0.0], 0.0)[0].to_bits(), (-0.0f64).to_bits());
    }
}

