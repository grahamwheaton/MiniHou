//! Spatial acceleration over the displayed scene: welded edge list (with soft/hidden flags),
//! a triangle BVH for surface hits and an edge BVH for cone (screen-tolerance) queries.
//! Built lazily on the first query of a new geometry revision and cached until it changes.
use super::math::*;

#[derive(Clone, Copy, Debug)]
pub struct Edge { pub a: u32, pub b: u32, pub tri: u32, pub hidden: bool, pub degenerate: bool }

#[derive(Clone, Copy, Debug)]
struct Node { min: V3, max: V3, start: u32, count: u32, right: u32 }

#[derive(Clone, Debug, Default)]
pub struct Bvh { nodes: Vec<Node>, items: Vec<u32> }

const LEAF: usize = 8;

impl Bvh {
    pub fn build(boxes: &[(V3, V3)]) -> Self {
        let mut items: Vec<u32> = (0..boxes.len() as u32).collect();
        let mut nodes = Vec::with_capacity(boxes.len() / LEAF * 2 + 1);
        if !boxes.is_empty() { Self::build_range(boxes, &mut items, 0, boxes.len(), &mut nodes); }
        Self { nodes, items }
    }
    fn build_range(boxes: &[(V3, V3)], items: &mut [u32], start: usize, end: usize, nodes: &mut Vec<Node>) -> usize {
        let (mut min, mut max) = ([f32::MAX; 3], [f32::MIN; 3]);
        for &i in &items[start..end] { let (a, b) = boxes[i as usize]; for k in 0..3 { min[k] = min[k].min(a[k]); max[k] = max[k].max(b[k]); } }
        let index = nodes.len();
        nodes.push(Node { min, max, start: start as u32, count: (end - start) as u32, right: 0 });
        if end - start <= LEAF { return index; }
        let ext = sub(max, min);
        let axis = if ext[0] >= ext[1] && ext[0] >= ext[2] { 0 } else if ext[1] >= ext[2] { 1 } else { 2 };
        let mid = (start + end) / 2;
        let centre = |i: &u32| { let (a, b) = boxes[*i as usize]; a[axis] + b[axis] };
        items[start..end].select_nth_unstable_by(mid - start, |x, y| centre(x).total_cmp(&centre(y)));
        nodes[index].count = 0;
        Self::build_range(boxes, items, start, mid, nodes);
        let right = Self::build_range(boxes, items, mid, end, nodes);
        nodes[index].right = right as u32;
        index
    }
    /// Visit items of nodes that pass `keep(min,max)`.
    pub fn visit(&self, mut keep: impl FnMut(V3, V3) -> bool, mut item: impl FnMut(u32)) {
        if self.nodes.is_empty() { return; }
        let mut stack = vec![0usize];
        while let Some(i) = stack.pop() {
            let n = self.nodes[i];
            if !keep(n.min, n.max) { continue; }
            if n.count > 0 { for &it in &self.items[n.start as usize..(n.start + n.count) as usize] { item(it); } }
            else { stack.push(n.right as usize); stack.push(i + 1); }
        }
    }
}

/// Slab test; returns entry distance.
pub fn ray_box(ray: Ray, min: V3, max: V3) -> Option<f32> {
    let (mut t0, mut t1) = (0f32, f32::MAX);
    for k in 0..3 {
        let inv = 1. / ray.1[k];
        let (mut a, mut b) = ((min[k] - ray.0[k]) * inv, (max[k] - ray.0[k]) * inv);
        if a > b { std::mem::swap(&mut a, &mut b); }
        if a.is_nan() || b.is_nan() { if ray.0[k] < min[k] || ray.0[k] > max[k] { return None; } continue; }
        t0 = t0.max(a); t1 = t1.min(b);
        if t0 > t1 { return None; }
    }
    Some(t0)
}

/// Identifies one evaluated geometry. Pointer/length catch reallocation, the sampled hash catches
/// in-place edits, the explicit revision (when the host has one) catches everything else.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct GeometryKey { pub revision: u64, ptr: usize, len: usize, tri_ptr: usize, tri_len: usize, sample: u64 }
impl GeometryKey {
    pub fn of(revision: u64, positions: &[f32], triangles: &[u32]) -> Self {
        let mut h = 0xcbf29ce484222325u64;
        let step = (positions.len() / 61).max(1);
        for x in positions.iter().step_by(step).chain(positions.last()) { h = (h ^ x.to_bits() as u64).wrapping_mul(0x100000001b3); }
        let step = (triangles.len() / 31).max(1);
        for x in triangles.iter().step_by(step) { h = (h ^ *x as u64).wrapping_mul(0x100000001b3); }
        Self { revision, ptr: positions.as_ptr() as usize, len: positions.len(), tri_ptr: triangles.as_ptr() as usize, tri_len: triangles.len(), sample: h }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Accel {
    pub key: GeometryKey,
    pub points: Vec<V3>,
    pub tris: Vec<[u32; 3]>,
    pub edges: Vec<Edge>,
    /// Cached derived candidates.
    pub midpoints: Vec<V3>,
    pub bounds: (V3, V3),
    pub scale: f32,
    tri_bvh: Bvh,
    edge_bvh: Bvh,
    pub build_count: u32,
}

impl Accel {
    pub fn build(key: GeometryKey, positions: &[f32], triangles: &[u32]) -> Self {
        let points: Vec<V3> = positions.chunks_exact(3).map(|p| [p[0], p[1], p[2]]).collect();
        let n = points.len() as u32;
        let tris: Vec<[u32; 3]> = triangles.chunks_exact(3).filter(|t| t.iter().all(|&i| i < n)).map(|t| [t[0], t[1], t[2]]).collect();
        let normal = |t: [u32; 3]| norm(cross(sub(points[t[1] as usize], points[t[0] as usize]), sub(points[t[2] as usize], points[t[0] as usize])));
        // (lo, hi, tri) sorted: equal neighbours share the edge.
        let mut half: Vec<(u32, u32, u32)> = Vec::with_capacity(tris.len() * 3);
        for (ti, t) in tris.iter().enumerate() { for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] { half.push((a.min(b), a.max(b), ti as u32)); } }
        half.sort_unstable();
        let mut edges = Vec::with_capacity(half.len() / 2 + 1);
        let mut i = 0;
        while i < half.len() {
            let (a, b, t) = half[i];
            let mut j = i + 1;
            while j < half.len() && half[j].0 == a && half[j].1 == b { j += 1; }
            // An edge shared by exactly two coplanar triangles is a triangulation diagonal: hidden (soft).
            let hidden = j - i == 2 && match (normal(tris[t as usize]), normal(tris[half[i + 1].2 as usize])) { (Some(x), Some(y)) => dot(x, y) > 0.99999, _ => false };
            let degenerate = a == b || dist(points[a as usize], points[b as usize]) < 1e-9;
            edges.push(Edge { a, b, tri: t, hidden, degenerate });
            i = j;
        }
        let midpoints = edges.iter().map(|e| lerp(points[e.a as usize], points[e.b as usize], 0.5)).collect();
        let bbox = |ps: &[u32]| { let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]); for &p in ps { let q = points[p as usize]; for k in 0..3 { lo[k] = lo[k].min(q[k]); hi[k] = hi[k].max(q[k]); } } (lo, hi) };
        let tri_boxes: Vec<_> = tris.iter().map(|t| bbox(t)).collect();
        let edge_boxes: Vec<_> = edges.iter().map(|e| bbox(&[e.a, e.b])).collect();
        let bounds = if points.is_empty() { ([0.; 3], [0.; 3]) } else { bbox(&(0..n).collect::<Vec<_>>()) };
        let scale = len(sub(bounds.1, bounds.0)).max(1e-3);
        Self { key, tri_bvh: Bvh::build(&tri_boxes), edge_bvh: Bvh::build(&edge_boxes), points, tris, edges, midpoints, bounds, scale, build_count: 1 }
    }
    pub fn p(&self, i: u32) -> V3 { self.points[i as usize] }
    /// Nearest triangle hit: `(triangle, t, barycentrics)`.
    pub fn ray_hit(&self, ray: Ray) -> Option<(u32, f32, [f32; 3])> {
        let best_t = std::cell::Cell::new(f32::MAX);
        let mut best: Option<(u32, f32, [f32; 3])> = None;
        self.tri_bvh.visit(|lo, hi| ray_box(ray, lo, hi).is_some_and(|t| t <= best_t.get()), |i| {
            let t = self.tris[i as usize];
            if let Some((d, u, v)) = ray_triangle(ray, self.p(t[0]), self.p(t[1]), self.p(t[2])) {
                if d < best_t.get() { best_t.set(d); best = Some((i, d, [1. - u - v, u, v])); }
            }
        });
        best
    }
    /// Edges whose bounds may lie within the cone `radius(t) = a + b*t` around the ray, up to `max_t`.
    pub fn edges_near(&self, ray: Ray, a: f32, b: f32, max_t: f32, mut f: impl FnMut(u32)) {
        self.edge_bvh.visit(|lo, hi| {
            let c = mul(add(lo, hi), 0.5); let h = dist(hi, lo) * 0.5;
            let tc = dot(sub(c, ray.0), ray.1);
            if tc + h < 0. || tc - h > max_t { return false; }
            let foot = add(ray.0, mul(ray.1, tc.max(0.)));
            dist(c, foot) <= a + b * (tc + h).max(0.) + h
        }, &mut f);
    }
}
