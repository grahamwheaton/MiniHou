//! Topology queries (`minihou/topo.py`).

use rayon::prelude::*;

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

/// [`edges`] on borrowed arrays (the Python binding passes numpy buffers straight through). The arrays must form a valid
/// topology ([`crate::geometry::validate_topology`]).
pub fn edges_raw(vp: &[i32], offs: &[i32], closed: Option<&[bool]>, npoints: usize) -> Edges {
    let nv = vp.len();
    let n = npoints.max(1);
    const RUN: usize = 1 << 14;

    // half-edges as (lower, higher); `u32::MAX` marks "no edge". In parallel over runs of primitives, which own contiguous,
    // disjoint vertex ranges.
    let mut half: Vec<(u32, u32)> = vec![(u32::MAX, 0); nv];
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
                    let w = if v + 1 < e { v + 1 } else if closed { s } else { continue };
                    let (a, b) = (vp[v] as u32, vp[w] as u32);
                    part[v - base] = (a.min(b), a.max(b));
                }
            }
        });
    }

    // counting sort by the lower point: start[lo]..start[lo + 1] are the half-edges leaving `lo`
    let mut start = vec![0usize; n + 1];
    for &(lo, _) in &half {
        if lo != u32::MAX {
            start[lo as usize + 1] += 1;
        }
    }
    for i in 0..n {
        start[i + 1] += start[i];
    }
    let total = start[n];
    let mut fill = start.clone();
    let mut sorted: Vec<(u32, u32)> = vec![(0, 0); total]; // (higher point, vertex)
    for (v, &(lo, hi)) in half.iter().enumerate() {
        if lo != u32::MAX {
            sorted[fill[lo as usize]] = (hi, v as u32);
            fill[lo as usize] += 1;
        }
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
    let ne: usize = distinct.iter().sum();

    // number the edges in (lower, higher) order: each run writes its own slice of the edge arrays
    let mut points = vec![0i64; ne * 2];
    let mut count = vec![0i64; ne];
    let mut of_vertex = vec![-1i64; nv];
    let ov = SharedMut(of_vertex.as_mut_ptr());
    let mut jobs = Vec::with_capacity(runs.len());
    let (mut prest, mut crest): (&mut [i64], &mut [i64]) = (&mut points, &mut count);
    let mut edge_base = 0usize;
    for ((first, run), k) in runs.iter().zip(&distinct) {
        let (pp, pr) = prest.split_at_mut(k * 2);
        let (cc, cr) = crest.split_at_mut(*k);
        jobs.push((*first, &**run, pp, cc, edge_base));
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

/// A raw pointer that may be shared by the threads of one parallel loop (each writes elements no other thread touches).
struct SharedMut(*mut i64);
unsafe impl Sync for SharedMut {}
