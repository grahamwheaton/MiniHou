//! The viewport's drawing layout for big meshes (`minihou/preview.py`): fan triangles, sorted along a Morton curve of their
//! centres into runs with bounding boxes, plus the unique edges, all as the final `u32` arrays the editor receives.

use rayon::prelude::*;

use crate::topo;

/// What the viewer draws for one topology.
pub struct Layout {
    /// 3 point numbers per triangle, in drawing order.
    pub tri: Vec<u32>,
    /// The primitive each triangle comes from.
    pub tri_prim: Vec<u32>,
    /// 2 point numbers per edge, numbered like `topo.edges`.
    pub edge: Vec<u32>,
    /// One row per run of `chunk` triangles (first, count, min xyz, max xyz), when the triangles were sorted.
    pub table: Option<Vec<f32>>,
    /// Triangles or edges were cut at their cap.
    pub truncated: bool,
}

/// Fan triangles of the closed polygons with 3 or more corners, as point triples with their primitive: the same triangles in
/// the same order as `proxy.fan_triangles` / the numba `fan_point_triangles`.
pub fn fan_triangles(vp: &[i32], offs: &[i32], closed: Option<&[bool]>) -> (Vec<u32>, Vec<u32>) {
    let np = offs.len() - 1;
    let count = |f: usize| -> usize {
        let n = (offs[f + 1] - offs[f]) as usize;
        if closed.is_none_or(|c| c[f]) && n >= 3 { n - 2 } else { 0 }
    };
    let mut first = vec![0usize; np + 1];
    for f in 0..np {
        first[f + 1] = first[f] + count(f);
    }
    let total = first[np];
    let mut tri = vec![0u32; total * 3];
    let mut owner = vec![0u32; total];
    // runs of polygons own contiguous, disjoint ranges of the output
    const RUN: usize = 1 << 14;
    let mut jobs = Vec::with_capacity(np / RUN + 1);
    let (mut trest, mut orest): (&mut [u32], &mut [u32]) = (&mut tri, &mut owner);
    for f0 in (0..np).step_by(RUN) {
        let f1 = (f0 + RUN).min(np);
        let k = first[f1] - first[f0];
        let (t, tr) = trest.split_at_mut(k * 3);
        let (o, or) = orest.split_at_mut(k);
        jobs.push((f0, f1, t, o));
        trest = tr;
        orest = or;
    }
    jobs.into_par_iter().for_each(|(f0, f1, t, o)| {
        let mut j = 0;
        for (f, &a) in offs.iter().enumerate().take(f1).skip(f0) {
            let n = count(f);
            let a = a as usize;
            let p0 = vp[a] as u32;
            for k in 0..n {
                t[j * 3] = p0;
                t[j * 3 + 1] = vp[a + k + 1] as u32;
                t[j * 3 + 2] = vp[a + k + 2] as u32;
                o[j] = f as u32;
                j += 1;
            }
        }
    });
    (tri, owner)
}

/// numpy's `minimum` / `maximum`: a NaN wins.
#[inline]
fn nan_min(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() { f32::NAN } else { a.min(b) }
}

#[inline]
fn nan_max(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() { f32::NAN } else { a.max(b) }
}

/// Per-axis minimum and maximum of points (x, y, z per point), as numpy's `P.min(0)` / `P.max(0)` (NaN propagates);
/// infinities when there are none.
pub fn bounds(p: &[f32]) -> ([f32; 3], [f32; 3]) {
    p.par_chunks(3 * 4096)
        .map(|c| {
            let (mut lo, mut hi) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
            for q in c.as_chunks::<3>().0 {
                for k in 0..3 {
                    lo[k] = nan_min(lo[k], q[k]);
                    hi[k] = nan_max(hi[k], q[k]);
                }
            }
            (lo, hi)
        })
        .reduce(|| ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]), |a, b| {
            ([0, 1, 2].map(|k| nan_min(a.0[k], b.0[k])), [0, 1, 2].map(|k| nan_max(a.1[k], b.1[k])))
        })
}

#[inline]
fn spread10(v: u64) -> u64 {
    let mut v = v & 1023;
    v = (v | (v << 16)) & 0x0300_00FF;
    v = (v | (v << 8)) & 0x0300_F00F;
    v = (v | (v << 4)) & 0x030C_30C3;
    v = (v | (v << 2)) & 0x0924_9249;
    v
}

/// Sort triangles along a 30-bit Morton curve of their centres (stable, so equal codes keep their order) and build the run
/// table: exactly `preview._chunk_numba`.
pub fn chunk_sort(p: &[f32], tri: &mut Vec<u32>, tri_prim: &mut Vec<u32>, chunk: usize) -> Vec<f32> {
    let n = tri_prim.len();
    if n == 0 {
        return Vec::new();
    }
    // bounds of the points up to the highest one a triangle uses (as `P[: tri.max() + 1]` in Python)
    let top = tri.par_iter().copied().max().unwrap_or(0) as usize + 1;
    let (lo, hi) = bounds(&p[..top * 3]);
    let lo64 = lo.map(|x| x as f64);
    let scale = [0, 1, 2].map(|k| 1023.0 / (hi[k] as f64 - lo64[k]).max(1e-12));
    // code in the high 32 bits, triangle number in the low: sorting these distinct keys (unstable is fine) is the stable sort by
    // code that the numba radix sort does
    let mut keyed: Vec<u64> = tri
        .par_chunks_exact(3)
        .enumerate()
        .map(|(t, c)| {
            let mut m = 0u64;
            for ax in 0..3 {
                let x = (p[c[0] as usize * 3 + ax] as f64 + p[c[1] as usize * 3 + ax] as f64 + p[c[2] as usize * 3 + ax] as f64) / 3.0;
                let q = (((x - lo64[ax]) * scale[ax]) as i64).clamp(0, 1023) as u64;
                m |= spread10(q) << ax;
            }
            (m << 32) | t as u64
        })
        .collect();
    keyed.par_sort_unstable();
    let old_tri = std::mem::take(tri);
    let old_prim = std::mem::take(tri_prim);
    let mut new_tri = vec![0u32; old_tri.len()];
    let mut new_prim = vec![0u32; n];
    new_tri.par_chunks_exact_mut(3).zip(new_prim.par_iter_mut()).zip(keyed.par_iter()).for_each(|((t, pr), &k)| {
        let src = (k as u32) as usize;
        t.copy_from_slice(&old_tri[src * 3..src * 3 + 3]);
        *pr = old_prim[src];
    });
    *tri = new_tri;
    *tri_prim = new_prim;
    chunk_bounds(p, tri, chunk)
}

/// One row per run of `chunk` triangles: first, count, min xyz, max xyz of their corners (float32, like `chunk_bounds`).
pub fn chunk_bounds(p: &[f32], tri: &[u32], chunk: usize) -> Vec<f32> {
    let n = tri.len() / 3;
    let rows = n.div_ceil(chunk);
    (0..rows)
        .into_par_iter()
        .flat_map_iter(|k| {
            let (a, b) = (k * chunk, ((k + 1) * chunk).min(n));
            let mut mn = [f32::INFINITY; 3];
            let mut mx = [f32::NEG_INFINITY; 3];
            for &q in &tri[a * 3..b * 3] {
                let q = q as usize * 3;
                for ax in 0..3 {
                    mn[ax] = mn[ax].min(p[q + ax]);
                    mx[ax] = mx[ax].max(p[q + ax]);
                }
            }
            [a as f32, (b - a) as f32, mn[0], mn[1], mn[2], mx[0], mx[1], mx[2]]
        })
        .collect()
}

/// The whole layout of a big mesh: fan triangles (cut at `max_tris`), sorted into runs when there are `chunk_min` or more,
/// and the edges (cut at `max_edges`).
#[allow(clippy::too_many_arguments)]
pub fn layout(p: &[f32], vp: &[i32], offs: &[i32], closed: Option<&[bool]>, npoints: usize, max_tris: usize, max_edges: usize,
              chunk_min: usize, chunk: usize) -> Layout {
    let ((mut tri, mut tri_prim), mut edge) = rayon::join(|| fan_triangles(vp, offs, closed), || topo::edge_points_u32(vp, offs, closed, npoints));
    let mut truncated = false;
    if tri_prim.len() > max_tris {
        tri.truncate(max_tris * 3);
        tri_prim.truncate(max_tris);
        truncated = true;
    }
    if edge.len() / 2 > max_edges {
        edge.truncate(max_edges * 2);
        truncated = true;
    }
    let table = (tri_prim.len() >= chunk_min).then(|| chunk_sort(p, &mut tri, &mut tri_prim, chunk));
    Layout { tri, tri_prim, edge, table, truncated }
}
