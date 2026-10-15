//! Topology queries (`minihou/topo.py`).

use std::sync::atomic::{AtomicU32, Ordering};

use rayon::prelude::*;

/// Points (or primitives) handled together by one parallel task.
const RUN: usize = 1 << 14;

use crate::geometry::Topology;

/// Unique undirected edges, numbered exactly as `topo.edges` numbers them: sorted by (lower point, higher point).
#[derive(Clone, Debug, PartialEq)]
pub struct Edges {
    /// `(E, 2)` point numbers, lower first, flattened.
    pub points: Vec<i64>,
    /// Edge of each half-edge (vertex -> next vertex of its primitive); -1 for the last vertex of an open curve.
    pub of_vertex: Vec<i64>,
    /// How many half-edges use each edge (1 = boundary, 2 = interior of a manifold).
    pub count: Vec<i64>,
}

impl Edges {
    pub fn len(&self) -> usize {
        self.count.len()
    }

    pub fn is_empty(&self) -> bool {
        self.count.is_empty()
    }
}

/// Every half-edge of the primitives: each vertex and the next one in its primitive (the last wraps to the first), except the
/// last vertex of an open curve. A closed one-vertex primitive gives a degenerate edge from its point to itself, as in Python.
pub fn edges(topo: &Topology, npoints: usize) -> Edges {
    edges_raw(&topo.vertex_point, &topo.prim_offsets, topo.closed.as_deref(), npoints)
}

/// Half-edges bucketed by their lower point and sorted inside each bucket: `(start, sorted (higher point, vertex), RUN-sized runs of
/// buckets with their number of distinct edges)`.
fn bucketed(vp: &[i32], offs: &[i32], closed: Option<&[bool]>, npoints: usize) -> (Vec<usize>, Vec<(u32, u32)>, Vec<usize>) {
    let nv = vp.len();
    let n = npoints.max(1);

    // half-edges as (lower, higher); `u32::MAX` marks "no edge". In parallel over runs of primitives, which own contiguous,
    // disjoint vertex ranges. Every big array here starts zeroed: the OS hands those pages out lazily, so touching them happens
    // in the parallel loops instead of in a serial fill (which was half the time on many cores).
    let mut half: Vec<(u32, u32)> = vec![(0, 0); nv];
    {
        let np = offs.len() - 1;
        let mut parts: Vec<(usize, &mut [(u32, u32)])> = Vec::with_capacity(np / RUN + 1);
        let mut rest: &mut [(u32, u32)] = &mut half;
        for first in (0..np).step_by(RUN) {
            let last = (first + RUN).min(np);
            let (part, r) = rest.split_at_mut((offs[last] - offs[first]) as usize);
            parts.push((first, part));
            rest = r;
        }
        parts.par_iter_mut().for_each(|(first, part)| {
            let base = offs[*first] as usize;
            for p in *first..(*first + RUN).min(np) {
                let (s, e) = (offs[p] as usize, offs[p + 1] as usize);
                let closed = closed.is_none_or(|c| c[p]);
                for v in s..e {
                    let w = if v + 1 < e {
                        v + 1
                    } else if closed {
                        s
                    } else {
                        part[v - base] = (u32::MAX, 0);
                        continue;
                    };
                    let (a, b) = (vp[v] as u32, vp[w] as u32);
                    part[v - base] = (a.min(b), a.max(b));
                }
            }
        });
    }

    // counting sort by the lower point: start[lo]..start[lo + 1] are the half-edges leaving `lo`. Counting and scattering run in
    // parallel with atomic counters: the order inside a bucket is then arbitrary, but each bucket is sorted below, so the result
    // is the same every time.
    let fill: Vec<AtomicU32> = (0..n).into_par_iter().map(|_| AtomicU32::new(0)).collect();
    half.par_iter().for_each(|&(lo, _)| {
        if lo != u32::MAX {
            fill[lo as usize].fetch_add(1, Ordering::Relaxed);
        }
    });
    // prefix sum in two parallel passes over runs of points (a serial scan over the run totals in between); the counters become
    // each bucket's next free slot
    let run_total: Vec<usize> = (0..n.div_ceil(RUN))
        .into_par_iter()
        .map(|r| fill[r * RUN..((r + 1) * RUN).min(n)].iter().map(|c| c.load(Ordering::Relaxed) as usize).sum())
        .collect();
    let mut run_start = Vec::with_capacity(run_total.len());
    let mut total = 0usize;
    for t in &run_total {
        run_start.push(total);
        total += t;
    }
    let mut start = vec![0usize; n + 1];
    start[..n].par_chunks_mut(RUN).zip(fill.par_chunks(RUN)).zip(run_start.par_iter()).for_each(|((st, c), &s0)| {
        let mut s = s0;
        for (st, c) in st.iter_mut().zip(c) {
            *st = s;
            s += c.load(Ordering::Relaxed) as usize;
            c.store(*st as u32, Ordering::Relaxed);
        }
    });
    start[n] = total;
    let mut sorted: Vec<(u32, u32)> = vec![(0, 0); total]; // (higher point, vertex)
    {
        let out = SharedMut(sorted.as_mut_ptr());
        half.par_iter().enumerate().for_each(|(v, &(lo, hi))| {
            let out = &out;
            if lo != u32::MAX {
                let at = fill[lo as usize].fetch_add(1, Ordering::Relaxed) as usize;
                // SAFETY: `fetch_add` hands every half-edge its own slot inside its bucket, and buckets do not overlap, so no
                // two threads write the same element; `sorted` outlives the loop.
                unsafe { *out.0.add(at) = (hi, v as u32) };
            }
        });
    }
    drop(fill);
    drop(half);

    // runs of lower points own contiguous, disjoint parts of `sorted`; sort each bucket by the higher point (buckets are tiny:
    // a point's valence) and count the distinct edges of each run
    let mut runs: Vec<(usize, &mut [(u32, u32)])> = Vec::with_capacity(n / RUN + 1);
    let mut rest: &mut [(u32, u32)] = &mut sorted;
    for first in (0..n).step_by(RUN) {
        let last = (first + RUN).min(n);
        let (run, r) = rest.split_at_mut(start[last] - start[first]);
        runs.push((first, run));
        rest = r;
    }
    let distinct: Vec<usize> = runs
        .par_iter_mut()
        .map(|(first, run)| {
            let base = start[*first];
            let mut edges = 0;
            for lo in *first..(*first + RUN).min(n) {
                let b = &mut run[start[lo] - base..start[lo + 1] - base];
                b.sort_unstable();
                edges += b.iter().enumerate().filter(|(i, x)| *i == 0 || b[i - 1].0 != x.0).count();
            }
            edges
        })
        .collect();
    (start, sorted, distinct)
}

/// [`edges`] on borrowed arrays (the Python binding passes numpy buffers straight through). The arrays must form a valid
/// topology ([`crate::geometry::validate_topology`]).
pub fn edges_raw(vp: &[i32], offs: &[i32], closed: Option<&[bool]>, npoints: usize) -> Edges {
    let n = npoints.max(1);
    let nv = vp.len();
    let (start, sorted, distinct) = bucketed(vp, offs, closed, npoints);
    let ne: usize = distinct.iter().sum();
    let runs: Vec<(usize, &[(u32, u32)])> = (0..n).step_by(RUN).map(|first| {
        let last = (first + RUN).min(n);
        (first, &sorted[start[first]..start[last]])
    }).collect();

    // number the edges in (lower, higher) order: each run writes its own slice of the edge arrays
    let mut points = vec![0i64; ne * 2];
    let mut count = vec![0i64; ne];
    // every vertex gets its edge below, except the last vertex of an open curve: only then is the -1 fill needed
    let mut of_vertex = vec![0i64; nv];
    if closed.is_some() {
        of_vertex.par_iter_mut().for_each(|x| *x = -1);
    }
    let ov = SharedMut(of_vertex.as_mut_ptr());
    let mut jobs = Vec::with_capacity(runs.len());
    let (mut prest, mut crest): (&mut [i64], &mut [i64]) = (&mut points, &mut count);
    let mut edge_base = 0usize;
    for (&(first, run), k) in runs.iter().zip(&distinct) {
        let (pp, pr) = prest.split_at_mut(k * 2);
        let (cc, cr) = crest.split_at_mut(*k);
        jobs.push((first, run, pp, cc, edge_base));
        prest = pr;
        crest = cr;
        edge_base += k;
    }
    jobs.into_par_iter().for_each(|(first, run, pp, cc, edge_base)| {
        let ov = &ov;
        let base = start[first];
        let mut e = 0usize;
        for lo in first..(first + RUN).min(n) {
            let mut prev: Option<u32> = None;
            for &(hi, v) in &run[start[lo] - base..start[lo + 1] - base] {
                if prev != Some(hi) {
                    pp[e * 2] = lo as i64;
                    pp[e * 2 + 1] = hi as i64;
                    e += 1;
                    prev = Some(hi);
                }
                cc[e - 1] += 1;
                // SAFETY: every vertex appears exactly once in `sorted`, so no two writes target the same element, and
                // `of_vertex` outlives this parallel loop.
                unsafe { *ov.0.add(v as usize) = (edge_base + e - 1) as i64 };
            }
        }
    });
    Edges { points, of_vertex, count }
}

/// Only the `(E * 2)` point numbers of [`edges_raw`], as `u32` (what the viewport draws), without the per-vertex and count arrays.
pub fn edge_points_u32(vp: &[i32], offs: &[i32], closed: Option<&[bool]>, npoints: usize) -> Vec<u32> {
    let n = npoints.max(1);
    let (start, sorted, distinct) = bucketed(vp, offs, closed, npoints);
    let ne: usize = distinct.iter().sum();
    let mut points = vec![0u32; ne * 2];
    let mut jobs = Vec::with_capacity(distinct.len());
    let mut rest: &mut [u32] = &mut points;
    for (i, k) in distinct.iter().enumerate() {
        let (pp, r) = rest.split_at_mut(k * 2);
        jobs.push((i * RUN, pp));
        rest = r;
    }
    jobs.into_par_iter().for_each(|(first, pp)| {
        let mut e = 0;
        for lo in first..(first + RUN).min(n) {
            let mut prev: Option<u32> = None;
            for &(hi, _) in &sorted[start[lo]..start[lo + 1]] {
                if prev != Some(hi) {
                    pp[e * 2] = lo as u32;
                    pp[e * 2 + 1] = hi;
                    e += 1;
                    prev = Some(hi);
                }
            }
        }
    });
    points
}

/// A raw pointer that may be shared by the threads of one parallel loop (each writes elements no other thread touches).
pub(crate) struct SharedMut<T>(pub(crate) *mut T);
unsafe impl<T> Sync for SharedMut<T> {}
