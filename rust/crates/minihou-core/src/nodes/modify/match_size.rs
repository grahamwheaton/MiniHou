use super::*;

pub(super) fn match_size(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = MATCH_SIZE.one(inputs, 0)?;
    if geo.npoints == 0 {
        return Ok((*geo).clone());
    }
    let target = inputs[1].first().filter(|t| t.npoints > 0);
    let normals = geo.point.get("N").and_then(|a| (a.width == 3).then(|| a.as_f32()).flatten());
    let out = match_size_points(
        geo.p(),
        target.map(|t| t.p()),
        p.b("translate")?,
        p.v3("center")?,
        &p.e("justify_z", &["center", "min", "max"])?,
        p.b("scale")?,
        p.v3("size")?,
        p.b("uniform")?,
        normals,
    );
    let mut g = (*geo).clone();
    g.set_p(out.p);
    if let Some(n) = out.n {
        g.point.insert("N".into(), Arc::new(Attr::f32(3, n)));
    }
    Ok(g)
}

/// New positions and, when the scale is not uniform, rescaled and renormalised normals
/// (`minihou/nodes/attrib_tools.py`), all in float64 like the numpy body, back to float32.
pub struct MatchSizeOut {
    pub p: Vec<f32>,
    pub n: Option<Vec<f32>>,
}

/// `match_size` on raw arrays, shared by the node cook above and the Python fast path. `target` is the second input's
/// nonempty points (its bounding box replaces `center`/`size`); `normals` the point `N` when it is xyz, else `None`.
#[allow(clippy::too_many_arguments)]
pub fn match_size_points(
    points: &[f32],
    target: Option<&[f32]>,
    translate: bool,
    center: [f64; 3],
    justify_z: &str,
    scale: bool,
    size: [f64; 3],
    uniform: bool,
    normals: Option<&[f32]>,
) -> MatchSizeOut {
    let (mut center, mut size) = (center, size);
    if let Some(t) = target {
        let (tlo, thi) = (axis_extremes(t, false), axis_extremes(t, true));
        size = [thi[0] - tlo[0], thi[1] - tlo[1], thi[2] - tlo[2]];
        center = [(tlo[0] + thi[0]) / 2.0, (tlo[1] + thi[1]) / 2.0,
                  if justify_z == "center" { (tlo[2] + thi[2]) / 2.0 } else if justify_z == "min" { tlo[2] } else { thi[2] }];
    }
    let (lo, hi) = (axis_extremes(points, false), axis_extremes(points, true));
    let mid = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0, (lo[2] + hi[2]) / 2.0];
    let mut s = [1.0f64; 3];
    if scale {
        let ext = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
        let ok = [ext[0] > 1e-12, ext[1] > 1e-12, ext[2] > 1e-12];
        for k in 0..3 {
            s[k] = if ok[k] { size[k] / ext[k] } else { 1.0 };
        }
        if uniform {
            let vals: Vec<f64> = (0..3).filter(|&k| ok[k]).map(|k| s[k]).collect();
            let u = if vals.is_empty() { 1.0 } else { numpy_min(&vals) };
            s = [u, u, u];
        }
    }
    let anchor = [mid[0], mid[1], match justify_z { "min" => lo[2], "max" => hi[2], _ => mid[2] }];
    let tgt = if translate { center } else { anchor };
    let mut p = vec![0.0f32; points.len()];
    for (i, q) in points.as_chunks::<3>().0.iter().enumerate() {
        for k in 0..3 {
            p[3 * i + k] = ((q[k] as f64 - anchor[k]) * s[k] + tgt[k]) as f32;
        }
    }
    // numpy's `np.allclose(s, s[0])`: only a real per-axis scale change rescales N
    let s0 = s[0];
    let close = (0..3).all(|k| (s[k] - s0).abs() <= 1e-8 + 1e-5 * s0.abs());
    let n = normals.and_then(|nrm| {
        if close {
            return None;
        }
        let mut result = vec![0.0f32; nrm.len()];
        for (i, row) in nrm.as_chunks::<3>().0.iter().enumerate() {
            let v = [row[0] as f64 / s[0], row[1] as f64 / s[1], row[2] as f64 / s[2]];
            let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
            let len = if len < 1e-20 { 1.0 } else { len };
            for k in 0..3 {
                result[3 * i + k] = (v[k] / len) as f32;
            }
        }
        Some(result)
    });
    MatchSizeOut { p, n }
}

/// Per-axis minimum or maximum of flat xyz points, NaN-propagating like `np.ndarray.min(0)` / `.max(0)`.
fn axis_extremes(points: &[f32], want_max: bool) -> [f64; 3] {
    let mut out = [if want_max { f64::NEG_INFINITY } else { f64::INFINITY }; 3];
    let mut nan = [false; 3];
    for q in points.as_chunks::<3>().0 {
        for k in 0..3 {
            let v = q[k] as f64;
            if v.is_nan() {
                nan[k] = true;
            } else if (want_max && v > out[k]) || (!want_max && v < out[k]) {
                out[k] = v;
            }
        }
    }
    for k in 0..3 {
        if nan[k] {
            out[k] = f64::NAN;
        }
    }
    out
}

/// `np.min` over a list: NaN if any element is NaN, else the smallest.
fn numpy_min(v: &[f64]) -> f64 {
    let mut m = f64::INFINITY;
    for &x in v {
        if x.is_nan() {
            return f64::NAN;
        }
        if x < m {
            m = x;
        }
    }
    m
}

