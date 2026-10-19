//! Element-selection topology shared by the viewport (minihou-app `element_select`) and the selection nodes
//! (`grow`, `select_path`, `select_loop`, `select_ring`): edge loops and rings, paths, and grow / shrink.
//! Pure logic over polygon/edge adjacency built once (compressed lists, O(n) per grow step).
//! Behaviour spec: docs/RUST_ELEMENT_SELECT.md (viewport) and docs/RUST_SELECT_NODES.md (nodes).
use std::collections::{BinaryHeap, HashMap, HashSet, VecDeque};

use crate::geometry::Geometry;

/// Compressed adjacency lists: the items of key `k` are `items[offs[k]..offs[k + 1]]`.
#[derive(Debug, Default)]
pub struct Csr { offs: Vec<u32>, items: Vec<u32> }
impl Csr {
    /// Groups `(key, item)` pairs by key, keeping their order within a key.
    pub fn build(n: usize, pairs: &[(u32, u32)]) -> Self {
        let mut offs = vec![0u32; n + 1];
        for &(k, _) in pairs { offs[k as usize + 1] += 1; }
        for i in 0..n { offs[i + 1] += offs[i]; }
        let mut fill = offs.clone();
        let mut items = vec![0; pairs.len()];
        for &(k, v) in pairs { items[fill[k as usize] as usize] = v; fill[k as usize] += 1; }
        Self { offs, items }
    }
    pub fn get(&self, k: u32) -> &[u32] {
        let k = k as usize;
        if k + 1 >= self.offs.len() { return &[]; }
        &self.items[self.offs[k] as usize..self.offs[k + 1] as usize]
    }
    /// Number of keys.
    pub fn len(&self) -> usize { self.offs.len().saturating_sub(1) }
    pub fn is_empty(&self) -> bool { self.len() == 0 }
}

const NONE: u32 = u32::MAX;

/// Which elements a selection holds (Blender's vertex / edge / face select modes).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Domain { Points, Edges, Faces }

/// Path search variants.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PathMethod {
    /// The viewport's pre-rust "most direct" heuristic: length, distance from the start-target chord and turning.
    #[default]
    MostDirect,
    /// Fewest steps (breadth-first hops; Blender's topological distance).
    FewestSteps,
    /// Geometrically shortest: Dijkstra by the distance between element centres (Blender's default Select Shortest Path).
    ShortestDistance,
}

/// Polygon/edge adjacency of one mesh.
#[derive(Debug, Default)]
pub struct SelectTopology {
    /// Edges as sorted point pairs; ids index this list.
    pub edges: Vec<(u32, u32)>,
    pub edge_id: HashMap<(u32, u32), u32>,
    pub point_edges: Csr,
    pub edge_polys: Csr,
    /// Sorted, unique points of each polygon.
    pub poly_verts: Csr,
    pub poly_edges: Csr,
    pub point_polys: Csr,
    pub npoints: usize,
    pub npolys: u32,
}

pub fn key(a: u32, b: u32) -> (u32, u32) { (a.min(b), a.max(b)) }
fn sub(a: [f32; 3], b: [f32; 3]) -> [f64; 3] { [0, 1, 2].map(|k| a[k] as f64 - b[k] as f64) }
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 { a[0] * b[0] + a[1] * b[1] + a[2] * b[2] }
fn len(a: [f64; 3]) -> f64 { dot(a, a).sqrt() }
/// Point `i` of flat xyz positions (zero when out of range).
pub fn at(p: &[f32], i: u32) -> [f32; 3] {
    let k = i as usize * 3;
    p.get(k..k + 3).map_or([0.; 3], |q| [q[0], q[1], q[2]])
}

impl SelectTopology {
    /// `edges` sorted unique point pairs (their order defines edge ids), `poly_verts` / `poly_edges` as `(polygon, point)` and
    /// `(polygon, edge id)` pairs in any order.
    pub fn from_parts(npoints: usize, edges: Vec<(u32, u32)>, mut vert_pairs: Vec<(u32, u32)>, mut edge_pairs: Vec<(u32, u32)>, npolys: u32) -> Self {
        let edge_id: HashMap<(u32, u32), u32> = edges.iter().enumerate().map(|(i, &e)| (e, i as u32)).collect();
        vert_pairs.sort_unstable(); vert_pairs.dedup();
        edge_pairs.sort_unstable(); edge_pairs.dedup();
        let edge_poly_pairs: Vec<(u32, u32)> = edge_pairs.iter().map(|&(p, e)| (e, p)).collect();
        let point_pairs: Vec<(u32, u32)> = edges.iter().enumerate().filter(|(_, &(a, b))| a != b && (b as usize) < npoints)
            .flat_map(|(i, &(a, b))| [(a, i as u32), (b, i as u32)]).collect();
        let point_poly_pairs: Vec<(u32, u32)> = vert_pairs.iter().map(|&(p, v)| (v, p)).filter(|&(v, _)| (v as usize) < npoints).collect();
        Self {
            point_edges: Csr::build(npoints, &point_pairs),
            edge_polys: Csr::build(edges.len(), &edge_poly_pairs),
            poly_verts: Csr::build(npolys as usize, &vert_pairs),
            poly_edges: Csr::build(npolys as usize, &edge_pairs),
            point_polys: Csr::build(npoints, &point_poly_pairs),
            edges, edge_id, npoints, npolys,
        }
    }

    /// Geometry topology: edges numbered as `topo::edges` (and edge group patterns) number them, polygons are the closed
    /// primitives (polygon ids are primitive numbers; open curves contribute edges but are not faces).
    pub fn from_geometry(g: &Geometry) -> Self {
        let e = crate::topo::edges(&g.topology, g.npoints);
        let edges: Vec<(u32, u32)> = e.points.chunks_exact(2).map(|q| (q[0] as u32, q[1] as u32)).collect();
        let (mut vert_pairs, mut edge_pairs) = (Vec::new(), Vec::new());
        let offs = &g.topology.prim_offsets;
        for prim in 0..g.nprims() {
            let r = offs[prim] as usize..offs[prim + 1] as usize;
            if !g.topology.is_closed(prim) || r.len() < 3 { continue; }
            for v in r {
                vert_pairs.push((prim as u32, g.topology.vertex_point[v] as u32));
                let id = e.of_vertex[v];
                if id >= 0 && e.points[2 * id as usize] != e.points[2 * id as usize + 1] { edge_pairs.push((prim as u32, id as u32)); }
            }
        }
        Self::from_parts(g.npoints, edges, vert_pairs, edge_pairs, g.nprims() as u32)
    }

    pub fn edge_of(&self, a: u32, b: u32) -> Option<u32> { self.edge_id.get(&key(a, b)).copied() }
    pub fn poly_has(&self, poly: u32, v: u32) -> bool { self.poly_verts.get(poly).binary_search(&v).is_ok() }
    pub fn poly_has_edge_points(&self, poly: u32, e: u32) -> bool { let (a, b) = self.edges[e as usize]; self.poly_has(poly, a) && self.poly_has(poly, b) }
    pub fn other(&self, e: u32, v: u32) -> u32 { let (a, b) = self.edges[e as usize]; if a == v { b } else { a } }
    /// Edges used by exactly one polygon.
    pub fn is_boundary(&self, e: u32) -> bool { self.edge_polys.get(e).len() == 1 }

    /// Number of elements in `domain`.
    pub fn count(&self, domain: Domain) -> usize {
        match domain { Domain::Points => self.npoints, Domain::Edges => self.edges.len(), Domain::Faces => self.npolys as usize }
    }

    /// Edge loop (`ring == false`) or ring through `start`, as edge ids in walk order (pre-rust `quadStrip`).
    pub fn walk(&self, p: &[f32], start: u32, ring: bool) -> Vec<u32> { self.walk_side(p, start, ring, None) }

    /// Like [`walk`](Self::walk), but `side` restricts the first step to one side of the seed: for a loop the seed's lower (0)
    /// or higher (1) point, for a ring its first (0) or second (1) adjacent polygon. The walk can still come back round a
    /// closed loop.
    pub fn walk_side(&self, p: &[f32], start: u32, ring: bool, side: Option<usize>) -> Vec<u32> {
        if start as usize >= self.edges.len() { return Vec::new(); }
        let mut found = vec![start];
        let mut seen = HashSet::from([start]);
        let mut queue = VecDeque::from([start]);
        while let Some(e) = queue.pop_front() {
            let faces = self.edge_polys.get(e);
            if faces.len() > 2 { continue; }
            let only = if e == start { side } else { None };
            let mut next = Vec::new();
            if ring {
                for (i, &f) in faces.iter().enumerate() {
                    if only.is_some_and(|s| s != i) { continue; }
                    if self.poly_verts.get(f).len() != 4 { continue; }
                    let (a, b) = self.edges[e as usize];
                    if let Some(&o) = self.poly_edges.get(f).iter().find(|&&o| { let (c, d) = self.edges[o as usize]; c != a && c != b && d != a && d != b }) { next.push(o); }
                }
            } else {
                let (a, b) = self.edges[e as usize];
                for (i, v) in [a, b].into_iter().enumerate() {
                    if only.is_some_and(|s| s != i) { continue; }
                    if let Some(o) = self.loop_step(p, e, v, faces) { next.push(o); }
                }
            }
            for n in next { if seen.insert(n) { found.push(n); queue.push_back(n); } }
        }
        found
    }

    /// The loop continuation of edge `e` through its point `v`, if unambiguous.
    fn loop_step(&self, p: &[f32], e: u32, v: u32, faces: &[u32]) -> Option<u32> {
        let at_v = self.point_edges.get(v);
        let candidates: Vec<u32> = at_v.iter().copied().filter(|&o| o != e && self.edge_polys.get(o).len() <= 2).collect();
        // Preserve the unambiguous topological continuation through regular quads.
        if at_v.len() == 4 {
            let opposite: Vec<u32> = candidates.iter().copied().filter(|&o| !faces.iter().any(|&f| self.poly_has_edge_points(f, o))).collect();
            if opposite.len() == 1 { return Some(opposite[0]); }
        }
        // A cap perimeter is a coherent flow even when each vertex has valence 3.
        let perimeter: Vec<u32> = candidates.iter().copied().filter(|&o| faces.iter().any(|&f| self.poly_verts.get(f).len() > 4 && self.poly_has_edge_points(f, o))).collect();
        if perimeter.len() == 1 { return Some(perimeter[0]); }
        // Irregular wire/boundary flow: one clearly forward direction, never fan out.
        let incoming = sub(at(p, v), at(p, self.other(e, v)));
        let il = len(incoming);
        let mut ranked: Vec<(f64, u32)> = candidates.iter().map(|&o| {
            let d = sub(at(p, self.other(o, v)), at(p, v));
            let dl = len(d);
            (if il > 0. && dl > 0. { dot(incoming, d) / (il * dl) } else { -1. }, o)
        }).collect();
        ranked.sort_by(|x, y| y.0.total_cmp(&x.0));
        let &(best, o) = ranked.first()?;
        (best > 0.5 && ranked.get(1).is_none_or(|second| best - second.0 > 0.15)).then_some(o)
    }

    /// The open border through boundary edge `start` (edges used by one polygon), in walk order. Stops where the border
    /// branches (a vertex with more than two boundary edges); a non-boundary seed gives nothing.
    pub fn boundary_walk(&self, start: u32, side: Option<usize>) -> Vec<u32> {
        if start as usize >= self.edges.len() || !self.is_boundary(start) { return Vec::new(); }
        let mut found = vec![start];
        let mut seen = HashSet::from([start]);
        let mut queue = VecDeque::from([start]);
        while let Some(e) = queue.pop_front() {
            let (a, b) = self.edges[e as usize];
            for (i, v) in [a, b].into_iter().enumerate() {
                if e == start && side.is_some_and(|s| s != i) { continue; }
                let next: Vec<u32> = self.point_edges.get(v).iter().copied().filter(|&o| o != e && self.is_boundary(o)).collect();
                if next.len() == 1 && seen.insert(next[0]) { found.push(next[0]); queue.push_back(next[0]); }
            }
        }
        found
    }

    /// The polygons on both sides of each edge of `edges` (a face loop from a ring), first-seen order.
    pub fn faces_of_edges(&self, edges: &[u32]) -> Vec<u32> {
        let mut seen = HashSet::new();
        edges.iter().flat_map(|&e| self.edge_polys.get(e).iter().copied()).filter(|f| seen.insert(*f)).collect()
    }
    /// The points of `edges`, first-seen order.
    pub fn points_of_edges(&self, edges: &[u32]) -> Vec<u32> {
        let mut seen = HashSet::new();
        edges.iter().flat_map(|&e| { let (a, b) = self.edges[e as usize]; [a, b] }).filter(|v| seen.insert(*v)).collect()
    }

    /// Path neighbours: points sharing an edge, edges sharing a point, polygons sharing an edge.
    pub fn neighbours(&self, domain: Domain, x: u32, out: &mut Vec<u32>) {
        out.clear();
        match domain {
            Domain::Points => out.extend(self.point_edges.get(x).iter().map(|&e| self.other(e, x))),
            Domain::Edges => {
                let (a, b) = self.edges[x as usize];
                for v in [a, b] { out.extend(self.point_edges.get(v).iter().copied().filter(|&e| e != x)); }
            }
            Domain::Faces => for &e in self.poly_edges.get(x) { out.extend(self.edge_polys.get(e).iter().copied().filter(|&f| f != x)); },
        }
    }
    /// Element centre: the point, the edge midpoint or the polygon's point average.
    pub fn position(&self, p: &[f32], domain: Domain, x: u32) -> [f32; 3] {
        let mean = |ids: &[u32]| { let n = ids.len().max(1) as f32; [0, 1, 2].map(|k| ids.iter().map(|&i| at(p, i)[k]).sum::<f32>() / n) };
        match domain {
            Domain::Points => at(p, x),
            Domain::Edges => { let (a, b) = self.edges[x as usize]; mean(&[a, b]) }
            Domain::Faces => mean(self.poly_verts.get(x)),
        }
    }

    /// Route from `start` to `target` (ids in `domain`), both included; empty when disconnected or out of range.
    pub fn path(&self, p: &[f32], domain: Domain, start: u32, target: u32, method: PathMethod) -> Vec<u32> {
        let n = self.count(domain);
        if start as usize >= n || target as usize >= n { return Vec::new(); }
        if start == target { return vec![start]; }
        let route = match method {
            PathMethod::FewestSteps => self.fewest_hops(domain, start, target),
            PathMethod::MostDirect => self.most_direct(p, domain, start, target),
            PathMethod::ShortestDistance => self.shortest_distance(p, domain, start, target),
        };
        if route.is_empty() { self.welded_path(p, domain, start, target, method) } else { route }
    }
    /// Split edges leave coincident points unconnected; route over the topology with coincident points merged and
    /// return every original element that lies on the merged route.
    fn welded_path(&self, p: &[f32], domain: Domain, start: u32, target: u32, method: PathMethod) -> Vec<u32> {
        let mut first: HashMap<[i64; 3], u32> = HashMap::new();
        let canon: Vec<u32> = (0..self.npoints as u32).map(|i| {
            let q = at(p, i);
            *first.entry(q.map(|x| (x as f64 * 1e5).round() as i64)).or_insert(i)
        }).collect();
        if canon.iter().enumerate().all(|(i, &c)| c == i as u32) { return Vec::new(); }
        let mut edges: Vec<(u32, u32)> = self.edges.iter().map(|&(a, b)| key(canon[a as usize], canon[b as usize])).filter(|e| e.0 != e.1).collect();
        edges.sort_unstable(); edges.dedup();
        let wid: HashMap<(u32, u32), u32> = edges.iter().enumerate().map(|(i, &e)| (e, i as u32)).collect();
        let (mut vp, mut ep) = (Vec::new(), Vec::new());
        for poly in 0..self.npolys {
            for &v in self.poly_verts.get(poly) { vp.push((poly, canon[v as usize])); }
            for &e in self.poly_edges.get(poly) {
                let (a, b) = self.edges[e as usize];
                if let Some(&w) = wid.get(&key(canon[a as usize], canon[b as usize])) { ep.push((poly, w)); }
            }
        }
        let welded = Self::from_parts(self.npoints, edges, vp, ep, self.npolys);
        let edge_to_w = |e: u32| { let (a, b) = self.edges[e as usize]; wid.get(&key(canon[a as usize], canon[b as usize])).copied() };
        let (s, t) = match domain {
            Domain::Points => (Some(canon[start as usize]), Some(canon[target as usize])),
            Domain::Edges => (edge_to_w(start), edge_to_w(target)),
            Domain::Faces => (Some(start), Some(target)),
        };
        let (Some(s), Some(t)) = (s, t) else { return Vec::new(); };
        let route = welded.path(p, domain, s, t, method);
        match domain {
            Domain::Faces => route,
            Domain::Points => {
                let on: HashSet<u32> = route.into_iter().collect();
                (0..self.npoints as u32).filter(|&i| on.contains(&canon[i as usize])).collect()
            }
            Domain::Edges => {
                let on: HashSet<u32> = route.into_iter().collect();
                (0..self.edges.len() as u32).filter(|&e| edge_to_w(e).is_some_and(|w| on.contains(&w))).collect()
            }
        }
    }
    fn fewest_hops(&self, domain: Domain, start: u32, target: u32) -> Vec<u32> {
        let mut previous = HashMap::from([(start, NONE)]);
        let mut queue = VecDeque::from([start]);
        let mut nb = Vec::new();
        while let Some(x) = queue.pop_front() {
            if x == target { break; }
            self.neighbours(domain, x, &mut nb);
            for &y in &nb { if let std::collections::hash_map::Entry::Vacant(v) = previous.entry(y) { v.insert(x); queue.push_back(y); } }
        }
        trace(&previous, target)
    }
    fn shortest_distance(&self, p: &[f32], domain: Domain, start: u32, target: u32) -> Vec<u32> {
        #[derive(PartialEq)]
        struct State { cost: f64, cur: u32 }
        impl Eq for State {}
        impl Ord for State { fn cmp(&self, o: &Self) -> std::cmp::Ordering { o.cost.total_cmp(&self.cost).then(o.cur.cmp(&self.cur)) } }
        impl PartialOrd for State { fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> { Some(self.cmp(o)) } }
        let mut best: HashMap<u32, f64> = HashMap::from([(start, 0.)]);
        let mut previous = HashMap::from([(start, NONE)]);
        let mut open = BinaryHeap::from([State { cost: 0., cur: start }]);
        let mut nb = Vec::new();
        while let Some(s) = open.pop() {
            if s.cur == target { return trace(&previous, target); }
            if s.cost > best.get(&s.cur).copied().unwrap_or(f64::INFINITY) { continue; }
            let pc = self.position(p, domain, s.cur);
            self.neighbours(domain, s.cur, &mut nb);
            for &v in &nb {
                // a tiny per-step cost breaks ties between equally long routes towards fewer steps
                let cost = s.cost + len(sub(self.position(p, domain, v), pc)) + 1e-9;
                if cost < best.get(&v).copied().unwrap_or(f64::INFINITY) {
                    best.insert(v, cost);
                    previous.insert(v, s.cur);
                    open.push(State { cost, cur: v });
                }
            }
        }
        Vec::new()
    }
    /// Pre-rust `directPath`: Dijkstra over (previous, current) so bends can be costed. Penalises length,
    /// distance from the start-target chord and turning.
    fn most_direct(&self, p: &[f32], domain: Domain, start: u32, target: u32) -> Vec<u32> {
        #[derive(PartialEq)]
        struct State { cost: f64, prev: u32, cur: u32 }
        impl Eq for State {}
        impl Ord for State { fn cmp(&self, o: &Self) -> std::cmp::Ordering { o.cost.total_cmp(&self.cost) } }
        impl PartialOrd for State { fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> { Some(self.cmp(o)) } }
        let pos = |x: u32| self.position(p, domain, x);
        let (a, b) = (pos(start), pos(target));
        let chord = sub(b, a);
        let scale = { let l = len(chord); if l > 0. { l } else { 1. } };
        let distance = |q: [f32; 3]| {
            let v = sub(q, a);
            let t = (dot(v, chord) / (scale * scale)).clamp(0., 1.);
            len([0, 1, 2].map(|k| v[k] - chord[k] * t)) / scale
        };
        let mut best: HashMap<(u32, u32), f64> = HashMap::from([((NONE, start), 0.)]);
        let mut parent: HashMap<(u32, u32), (u32, u32)> = HashMap::new();
        let mut open = BinaryHeap::from([State { cost: 0., prev: NONE, cur: start }]);
        let mut nb = Vec::new();
        let mut pops = 0usize;
        while let Some(s) = open.pop() {
            pops += 1;
            if pops > 4_000_000 { return self.fewest_hops(domain, start, target); }
            if s.cost > best.get(&(s.prev, s.cur)).copied().unwrap_or(f64::INFINITY) { continue; }
            if s.cur == target {
                let mut route = vec![s.cur];
                let mut k = (s.prev, s.cur);
                while let Some(&up) = parent.get(&k) { route.push(up.1); k = up; }
                route.reverse();
                return route;
            }
            let pc = pos(s.cur);
            let incoming = if s.prev == NONE { chord } else { sub(pc, pos(s.prev)) };
            let il = len(incoming);
            self.neighbours(domain, s.cur, &mut nb);
            for &v in &nb {
                if v == s.prev { continue; }
                let q = pos(v);
                let d = sub(q, pc);
                let l = len(d);
                let turn = if il > 0. && l > 0. { 1. - (dot(incoming, d) / (il * l)).clamp(-1., 1.) } else { 0. };
                let cost = s.cost + (l / scale).max(1e-9) * (1. + 8. * (distance(pc) + distance(q)) / 2.) + 0.75 * turn;
                let k = (s.cur, v);
                if cost < best.get(&k).copied().unwrap_or(f64::INFINITY) - 1e-12 {
                    best.insert(k, cost);
                    parent.insert(k, (s.prev, s.cur));
                    open.push(State { cost, prev: s.cur, cur: v });
                }
            }
        }
        Vec::new()
    }

    /// One Select More step (Blender `select_more`) in place, O(elements + adjacency). Without `face_step` neighbours
    /// share an edge (points, faces) or a point (edges); with it, they also share a polygon through a point (points of
    /// touching polygons, edges of polygons at either end, polygons sharing a point).
    pub fn grow_step(&self, domain: Domain, sel: &mut [bool], face_step: bool) {
        let old = sel.to_vec();
        match domain {
            Domain::Points => {
                let mut via_poly = vec![false; self.npolys as usize];
                for (v, _) in old.iter().enumerate().filter(|x| *x.1) {
                    for &e in self.point_edges.get(v as u32) { sel[self.other(e, v as u32) as usize] = true; }
                    if face_step { for &f in self.point_polys.get(v as u32) { via_poly[f as usize] = true; } }
                }
                for (f, _) in via_poly.iter().enumerate().filter(|x| *x.1) { for &v in self.poly_verts.get(f as u32) { sel[v as usize] = true; } }
            }
            Domain::Edges => {
                let mut points = vec![false; self.npoints];
                for (e, _) in old.iter().enumerate().filter(|x| *x.1) { let (a, b) = self.edges[e]; points[a as usize] = true; points[b as usize] = true; }
                let mut via_poly = vec![false; self.npolys as usize];
                for (v, _) in points.iter().enumerate().filter(|x| *x.1) {
                    for &e in self.point_edges.get(v as u32) { sel[e as usize] = true; }
                    if face_step { for &f in self.point_polys.get(v as u32) { via_poly[f as usize] = true; } }
                }
                for (f, _) in via_poly.iter().enumerate().filter(|x| *x.1) { for &e in self.poly_edges.get(f as u32) { sel[e as usize] = true; } }
            }
            Domain::Faces => {
                if face_step {
                    let mut points = vec![false; self.npoints];
                    for (f, _) in old.iter().enumerate().filter(|x| *x.1) { for &v in self.poly_verts.get(f as u32) { points[v as usize] = true; } }
                    for (v, _) in points.iter().enumerate().filter(|x| *x.1) { for &f in self.point_polys.get(v as u32) { sel[f as usize] = true; } }
                } else {
                    for (f, _) in old.iter().enumerate().filter(|x| *x.1) {
                        for &e in self.poly_edges.get(f as u32) { for &g in self.edge_polys.get(e) { sel[g as usize] = true; } }
                    }
                }
            }
        }
    }
    /// One Select Less step: an element stays selected only when every neighbour (as in [`grow_step`](Self::grow_step))
    /// is selected. Elements without neighbours keep their state; open borders do not erode by themselves.
    pub fn shrink_step(&self, domain: Domain, sel: &mut [bool], face_step: bool) {
        // the neighbour relation is symmetric, so Select Less is the complement of growing the complement
        for x in sel.iter_mut() { *x = !*x; }
        self.grow_step(domain, sel, face_step);
        for x in sel.iter_mut() { *x = !*x; }
    }
    /// `steps` grow (or shrink) steps; stops early once nothing changes.
    pub fn grow(&self, domain: Domain, sel: &mut [bool], steps: usize, face_step: bool, shrink: bool) {
        for _ in 0..steps {
            let before = sel.iter().filter(|x| **x).count();
            if shrink { self.shrink_step(domain, sel, face_step) } else { self.grow_step(domain, sel, face_step) }
            if sel.iter().filter(|x| **x).count() == before { break; }
        }
    }
}

fn trace(previous: &HashMap<u32, u32>, target: u32) -> Vec<u32> {
    if !previous.contains_key(&target) { return Vec::new(); }
    let mut route = vec![target];
    let mut x = target;
    while let Some(&prev) = previous.get(&x) { if prev == NONE { break; } route.push(prev); x = prev; }
    route.reverse();
    route
}

#[cfg(test)]
mod tests {
    use super::*;
    use Domain::{Edges, Faces, Points};

    /// `nx` by `ny` quads in z = 0; point (i, j) is j * (nx + 1) + i, quad (i, j) is j * nx + i.
    pub(crate) fn grid(nx: u32, ny: u32) -> (Vec<f32>, Vec<Vec<u32>>) {
        let w = nx + 1;
        let p = (0..=ny).flat_map(|j| (0..=nx).flat_map(move |i| [i as f32, j as f32, 0.])).collect();
        let polys = (0..ny).flat_map(|j| (0..nx).map(move |i| vec![j * w + i, j * w + i + 1, (j + 1) * w + i + 1, (j + 1) * w + i])).collect();
        (p, polys)
    }
    pub(crate) fn topo(p: &[f32], polys: &[Vec<u32>]) -> SelectTopology {
        let mut edges = Vec::new();
        for f in polys { for k in 0..f.len() { edges.push(key(f[k], f[(k + 1) % f.len()])); } }
        edges.sort_unstable(); edges.dedup();
        let ids: HashMap<_, _> = edges.iter().enumerate().map(|(i, &e)| (e, i as u32)).collect();
        let (mut vp, mut ep) = (Vec::new(), Vec::new());
        for (i, f) in polys.iter().enumerate() {
            for k in 0..f.len() { vp.push((i as u32, f[k])); ep.push((i as u32, ids[&key(f[k], f[(k + 1) % f.len()])])); }
        }
        SelectTopology::from_parts(p.len() / 3, edges, vp, ep, polys.len() as u32)
    }
    #[test] fn paths_cross_split_edges_by_merging_coincident_points() {
        let p = [0.,0.,0., 1.,0.,0., 1.,1.,0., 0.,1.,0.,  1.,0.,0., 2.,0.,0., 2.,1.,0., 1.,1.,0.];
        let polys = vec![vec![0, 1, 2, 3], vec![4, 5, 6, 7]];
        let t = topo(&p, &polys);
        let route = t.path(&p, Points, 0, 5, PathMethod::FewestSteps);
        assert!(route.contains(&0) && route.contains(&5) && (route.contains(&1) || route.contains(&4)), "{route:?}");
        let faces = t.path(&p, Faces, 0, 1, PathMethod::FewestSteps);
        assert_eq!(faces, Vec::<u32>::new().into_iter().chain([0, 1]).collect::<Vec<_>>());
        let e0 = t.edge_of(0, 1).unwrap(); let e1 = t.edge_of(5, 6).unwrap();
        assert!(!t.path(&p, Edges, e0, e1, PathMethod::FewestSteps).is_empty());
    }
    fn mask(n: usize, on: &[u32]) -> Vec<bool> { let mut m = vec![false; n]; for &i in on { m[i as usize] = true; } m }
    fn ids(m: &[bool]) -> Vec<u32> { m.iter().enumerate().filter(|x| *x.1).map(|x| x.0 as u32).collect() }

    #[test]
    fn grow_faces_by_edge_and_by_vertex() {
        let (p, polys) = grid(5, 5);
        let t = topo(&p, &polys);
        let centre = 2 * 5 + 2;
        let mut by_edge = mask(25, &[centre]);
        t.grow_step(Faces, &mut by_edge, false);
        assert_eq!(ids(&by_edge), vec![7, 11, 12, 13, 17]);
        let mut by_vertex = mask(25, &[centre]);
        t.grow_step(Faces, &mut by_vertex, true);
        assert_eq!(ids(&by_vertex), vec![6, 7, 8, 11, 12, 13, 16, 17, 18]);
    }

    #[test]
    fn shrink_needs_every_neighbour() {
        let (p, polys) = grid(5, 5);
        let t = topo(&p, &polys);
        let mut s = mask(25, &[6, 7, 8, 11, 12, 13, 16, 17, 18]);
        t.shrink_step(Faces, &mut s, true);
        assert_eq!(ids(&s), vec![12]);
        let mut s = mask(25, &[6, 7, 8, 11, 12, 13, 16, 17, 18]);
        t.shrink_step(Faces, &mut s, false);
        assert_eq!(ids(&s), vec![12]);
        // a full open grid does not erode at its border
        let mut all = vec![true; 25];
        t.shrink_step(Faces, &mut all, true);
        assert!(all.iter().all(|x| *x));
    }

    #[test]
    fn points_and_edges_grow() {
        let (p, polys) = grid(2, 2); // w = 3, centre point 4
        let t = topo(&p, &polys);
        let mut s = mask(9, &[4]);
        t.grow_step(Points, &mut s, false);
        assert_eq!(ids(&s), vec![1, 3, 4, 5, 7]);
        let mut s = mask(9, &[4]);
        t.grow_step(Points, &mut s, true);
        assert_eq!(ids(&s), (0..9).collect::<Vec<_>>());
        let e = t.edge_of(0, 1).unwrap();
        let mut s = mask(t.edges.len(), &[e]);
        t.grow_step(Edges, &mut s, false);
        let got: Vec<_> = ids(&s).into_iter().map(|e| t.edges[e as usize]).collect();
        assert_eq!(got, vec![(0, 1), (0, 3), (1, 2), (1, 4)]);
        let mut s = mask(t.edges.len(), &[e]);
        t.grow_step(Edges, &mut s, true);
        // plus the edges of the two quads at points 0 and 1
        assert!(s[t.edge_of(3, 4).unwrap() as usize] && s[t.edge_of(4, 5).unwrap() as usize] && s[t.edge_of(2, 5).unwrap() as usize]);
    }

    #[test]
    fn shortest_distance_and_fewest_steps_differ() {
        // straight chain 0-1-2-3-4 or a two-edge detour through 5
        let p = vec![0., 0., 0., 1., 0., 0., 2., 0., 0., 3., 0., 0., 4., 0., 0., 2., 1.5, 0.];
        let edges = vec![(0, 1), (0, 5), (1, 2), (2, 3), (3, 4), (4, 5)];
        let t = SelectTopology::from_parts(6, edges, vec![], vec![], 0);
        assert_eq!(t.path(&p, Points, 0, 4, PathMethod::ShortestDistance), vec![0, 1, 2, 3, 4]);
        assert_eq!(t.path(&p, Points, 0, 4, PathMethod::FewestSteps), vec![0, 5, 4]);
    }
}
