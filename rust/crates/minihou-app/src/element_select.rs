//! Element-mode selection on the displayed mesh: polygons rebuilt from the fan triangles and the polygon edge list,
//! edge loops and rings, Ctrl-click paths and selection conversion between point/edge/face modes. Ported from the
//! pre-rust browser viewport (preview.js `quadStrip`, `pathPick`, `setMode`; selection-tools.js `directPath`).
//! Behaviour spec: docs/RUST_ELEMENT_SELECT.md.
use crate::picking::{self, Hit, Selection};
use crate::ui::SelectionMode;
use std::collections::{BinaryHeap, HashMap, HashSet, VecDeque};

/// Compressed adjacency lists: the items of key `k` are `items[offs[k]..offs[k + 1]]`.
#[derive(Debug, Default)]
struct Csr { offs: Vec<u32>, items: Vec<u32> }
impl Csr {
    /// Groups `(key, item)` pairs by key, keeping their order within a key.
    fn build(n: usize, pairs: &[(u32, u32)]) -> Self {
        let mut offs = vec![0u32; n + 1];
        for &(k, _) in pairs { offs[k as usize + 1] += 1; }
        for i in 0..n { offs[i + 1] += offs[i]; }
        let mut fill = offs.clone();
        let mut items = vec![0; pairs.len()];
        for &(k, v) in pairs { items[fill[k as usize] as usize] = v; fill[k as usize] += 1; }
        Self { offs, items }
    }
    fn get(&self, k: u32) -> &[u32] {
        let k = k as usize;
        if k + 1 >= self.offs.len() { return &[]; }
        &self.items[self.offs[k] as usize..self.offs[k + 1] as usize]
    }
}

const NONE: u32 = u32::MAX;

/// Polygon/edge adjacency of one displayed mesh; build once per cooked scene.
#[derive(Debug, Default)]
pub struct Topology {
    /// Real (polygon) edges as sorted point pairs; ids index this list.
    pub edges: Vec<(u32, u32)>,
    edge_id: HashMap<(u32, u32), u32>,
    point_edges: Csr,
    edge_polys: Csr,
    /// Polygon of each triangle (`NONE` for invalid triangles).
    tri_poly: Vec<u32>,
    poly_tris: Csr,
    /// Sorted, unique points of each polygon.
    poly_verts: Csr,
    poly_edges: Csr,
    npolys: u32,
}

fn key(a: u32, b: u32) -> (u32, u32) { (a.min(b), a.max(b)) }
fn sub(a: [f32; 3], b: [f32; 3]) -> [f64; 3] { [0, 1, 2].map(|k| a[k] as f64 - b[k] as f64) }
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 { a[0] * b[0] + a[1] * b[1] + a[2] * b[2] }
fn len(a: [f64; 3]) -> f64 { dot(a, a).sqrt() }

impl Topology {
    /// `positions` xyz per point, `tris` fan triangles, `edge_list` polygon edges as point pairs (the preview layout).
    /// Triangles sharing an edge that is not a polygon edge (a fan diagonal) belong to the same polygon.
    pub fn new(positions: &[f32], tris: &[u32], edge_list: &[u32]) -> Self {
        let npoints = positions.len() / 3;
        let valid = |i: u32| (i as usize) < npoints;
        let mut edges: Vec<(u32, u32)> = edge_list.chunks_exact(2).filter(|e| e[0] != e[1] && valid(e[0]) && valid(e[1]))
            .map(|e| key(e[0], e[1])).collect();
        let tri_chunks = tris.as_chunks::<3>().0;
        if edges.is_empty() {
            // No polygon edge list: every triangle is its own face.
            for t in tri_chunks { if t.iter().all(|&i| valid(i)) { for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] { if a != b { edges.push(key(a, b)); } } } }
        }
        edges.sort_unstable(); edges.dedup();
        let edge_id: HashMap<(u32, u32), u32> = edges.iter().enumerate().map(|(i, &e)| (e, i as u32)).collect();
        // union-find over triangles joined by diagonals
        let ntri = tri_chunks.len();
        let mut parent: Vec<u32> = (0..ntri as u32).collect();
        fn find(parent: &mut [u32], mut a: u32) -> u32 {
            while parent[a as usize] != a { parent[a as usize] = parent[parent[a as usize] as usize]; a = parent[a as usize]; }
            a
        }
        let mut diagonal: HashMap<(u32, u32), u32> = HashMap::new();
        for (n, t) in tri_chunks.iter().enumerate() {
            if !t.iter().all(|&i| valid(i)) { continue; }
            for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                let k = key(a, b);
                if a == b || edge_id.contains_key(&k) { continue; }
                match diagonal.get(&k) {
                    Some(&other) => { let (x, y) = (find(&mut parent, other), find(&mut parent, n as u32)); if x != y { parent[y as usize] = x; } }
                    None => { diagonal.insert(k, n as u32); }
                }
            }
        }
        let mut tri_poly = vec![NONE; ntri];
        let mut root_poly: HashMap<u32, u32> = HashMap::new();
        let (mut poly_tri_pairs, mut vert_pairs, mut edge_pairs) = (Vec::new(), Vec::new(), Vec::new());
        for (n, t) in tri_chunks.iter().enumerate() {
            if !t.iter().all(|&i| valid(i)) { continue; }
            let root = find(&mut parent, n as u32);
            let next = root_poly.len() as u32;
            let poly = *root_poly.entry(root).or_insert(next);
            tri_poly[n] = poly;
            poly_tri_pairs.push((poly, n as u32));
            for &v in t { vert_pairs.push((poly, v)); }
            for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] { if let Some(&e) = edge_id.get(&key(a, b)) { edge_pairs.push((poly, e)); } }
        }
        let npolys = root_poly.len() as u32;
        vert_pairs.sort_unstable(); vert_pairs.dedup();
        edge_pairs.sort_unstable(); edge_pairs.dedup();
        let edge_poly_pairs: Vec<(u32, u32)> = edge_pairs.iter().map(|&(p, e)| (e, p)).collect();
        let point_pairs: Vec<(u32, u32)> = edges.iter().enumerate().flat_map(|(i, &(a, b))| [(a, i as u32), (b, i as u32)]).collect();
        Self {
            point_edges: Csr::build(npoints, &point_pairs),
            edge_polys: Csr::build(edges.len(), &edge_poly_pairs),
            poly_tris: Csr::build(npolys as usize, &poly_tri_pairs),
            poly_verts: Csr::build(npolys as usize, &vert_pairs),
            poly_edges: Csr::build(npolys as usize, &edge_pairs),
            edges, edge_id, tri_poly, npolys,
        }
    }
    pub fn edge_of(&self, a: u32, b: u32) -> Option<u32> { self.edge_id.get(&key(a, b)).copied() }
    pub fn poly_of_tri(&self, tri: u32) -> Option<u32> { self.tri_poly.get(tri as usize).copied().filter(|&p| p != NONE) }
    pub fn poly_tris(&self, poly: u32) -> &[u32] { self.poly_tris.get(poly) }
    fn poly_has(&self, poly: u32, v: u32) -> bool { self.poly_verts.get(poly).binary_search(&v).is_ok() }
    fn poly_has_edge_points(&self, poly: u32, e: u32) -> bool { let (a, b) = self.edges[e as usize]; self.poly_has(poly, a) && self.poly_has(poly, b) }
    fn other(&self, e: u32, v: u32) -> u32 { let (a, b) = self.edges[e as usize]; if a == v { b } else { a } }

    /// The polygon edge nearest `point` (world space) among the edges of the clicked triangle's polygon.
    pub fn nearest_edge(&self, p: &[f32], tri: u32, point: [f32; 3]) -> Option<u32> {
        let poly = self.poly_of_tri(tri)?;
        let distance = |e: u32| {
            let (a, b) = self.edges[e as usize];
            let (pa, pb) = (picking::at(p, a), picking::at(p, b));
            let (ab, ap) = (sub(pb, pa), sub(point, pa));
            let t = (dot(ap, ab) / dot(ab, ab).max(1e-30)).clamp(0., 1.);
            len([0, 1, 2].map(|k| ap[k] - ab[k] * t))
        };
        self.poly_edges.get(poly).iter().copied().min_by(|&x, &y| distance(x).total_cmp(&distance(y)))
    }

    /// Edge loop (`ring == false`) or ring through `start`, as edge ids in walk order (pre-rust `quadStrip`).
    pub fn walk(&self, p: &[f32], start: u32, ring: bool) -> Vec<u32> {
        if start as usize >= self.edges.len() { return Vec::new(); }
        let mut found = vec![start];
        let mut seen = HashSet::from([start]);
        let mut queue = VecDeque::from([start]);
        while let Some(e) = queue.pop_front() {
            let faces = self.edge_polys.get(e);
            if faces.len() > 2 { continue; }
            let mut next = Vec::new();
            if ring {
                for &f in faces {
                    if self.poly_verts.get(f).len() != 4 { continue; }
                    let (a, b) = self.edges[e as usize];
                    if let Some(&o) = self.poly_edges.get(f).iter().find(|&&o| { let (c, d) = self.edges[o as usize]; c != a && c != b && d != a && d != b }) { next.push(o); }
                }
            } else {
                let (a, b) = self.edges[e as usize];
                for v in [a, b] {
                    let at = self.point_edges.get(v);
                    let candidates: Vec<u32> = at.iter().copied().filter(|&o| o != e && self.edge_polys.get(o).len() <= 2).collect();
                    // Preserve the unambiguous topological continuation through regular quads.
                    if at.len() == 4 {
                        let opposite: Vec<u32> = candidates.iter().copied().filter(|&o| !faces.iter().any(|&f| self.poly_has_edge_points(f, o))).collect();
                        if opposite.len() == 1 { next.push(opposite[0]); continue; }
                    }
                    // A cap perimeter is a coherent flow even when each vertex has valence 3.
                    let perimeter: Vec<u32> = candidates.iter().copied().filter(|&o| faces.iter().any(|&f| self.poly_verts.get(f).len() > 4 && self.poly_has_edge_points(f, o))).collect();
                    if perimeter.len() == 1 { next.push(perimeter[0]); continue; }
                    // Irregular wire/boundary flow: one clearly forward direction, never fan out.
                    let incoming = sub(picking::at(p, v), picking::at(p, self.other(e, v)));
                    let il = len(incoming);
                    let mut ranked: Vec<(f64, u32)> = candidates.iter().map(|&o| {
                        let d = sub(picking::at(p, self.other(o, v)), picking::at(p, v));
                        let dl = len(d);
                        (if il > 0. && dl > 0. { dot(incoming, d) / (il * dl) } else { -1. }, o)
                    }).collect();
                    ranked.sort_by(|x, y| y.0.total_cmp(&x.0));
                    if let Some(&(best, o)) = ranked.first() {
                        if best > 0.5 && ranked.get(1).is_none_or(|second| best - second.0 > 0.15) { next.push(o); }
                    }
                }
            }
            for n in next { if seen.insert(n) { found.push(n); queue.push_back(n); } }
        }
        found
    }

    /// The loop or ring selection in `mode` through edge `edge`. Faces mode gives the face loop (polygons of the ring).
    pub fn loop_selection(&self, p: &[f32], edge: u32, ring: bool, mode: SelectionMode) -> Selection {
        let mut s = Selection { mode, ..Default::default() };
        match mode {
            SelectionMode::Object => {}
            SelectionMode::Edges => s.edges = self.walk(p, edge, ring).into_iter().map(|e| self.edges[e as usize]).collect(),
            SelectionMode::Points => {
                let mut seen = HashSet::new();
                for e in self.walk(p, edge, ring) { let (a, b) = self.edges[e as usize]; for v in [a, b] { if seen.insert(v) { s.points.push(v); } } }
            }
            SelectionMode::Faces => {
                let mut polys = Vec::new();
                for e in self.walk(p, edge, true) { for &f in self.edge_polys.get(e) { if !polys.contains(&f) { polys.push(f); } } }
                s.tris = self.tris_of(&polys);
            }
        }
        s
    }

    fn tris_of(&self, polys: &[u32]) -> Vec<u32> { polys.iter().flat_map(|&f| self.poly_tris(f).iter().copied()).collect() }
    pub(crate) fn polys_of(&self, tris: &[u32]) -> Vec<u32> {
        let mut seen = HashSet::new();
        tris.iter().filter_map(|&t| self.poly_of_tri(t)).filter(|f| seen.insert(*f)).collect()
    }

    /// What a plain click on `hit` selects: the nearest corner, the nearest polygon edge, or the whole polygon.
    pub fn pick(&self, p: &[f32], tris: &[u32], hit: Hit, point: [f32; 3], mode: SelectionMode) -> Selection {
        let mut s = picking::element_of(hit, tris, mode);
        match mode {
            SelectionMode::Edges => if let Some(e) = self.nearest_edge(p, hit.tri as u32, point) { s.edges = vec![self.edges[e as usize]]; },
            SelectionMode::Faces => if let Some(f) = self.poly_of_tri(hit.tri as u32) { s.tris = self.poly_tris(f).to_vec(); },
            _ => {}
        }
        s
    }

    /// Expands face selections to whole polygons and drops edges that are not polygon edges (fan diagonals).
    pub fn complete(&self, mut s: Selection) -> Selection {
        let polys = self.polys_of(&s.tris);
        s.tris = self.tris_of(&polys);
        s.edges.retain(|&(a, b)| self.edge_of(a, b).is_some());
        s
    }

    /// The active (last selected) element of `sel` in its mode's id space: point, edge id or polygon.
    fn active(&self, sel: &Selection) -> Option<u32> {
        match sel.mode {
            SelectionMode::Points => sel.points.last().copied(),
            SelectionMode::Edges => sel.edges.last().and_then(|&(a, b)| self.edge_of(a, b)),
            SelectionMode::Faces => sel.tris.last().and_then(|&t| self.poly_of_tri(t)),
            SelectionMode::Object => None,
        }
    }
    fn element(&self, sel: &Selection) -> Option<u32> {
        // the clicked element of a one-element pick
        match sel.mode {
            SelectionMode::Points => sel.points.first().copied(),
            SelectionMode::Edges => sel.edges.first().and_then(|&(a, b)| self.edge_of(a, b)),
            SelectionMode::Faces => sel.tris.first().and_then(|&t| self.poly_of_tri(t)),
            SelectionMode::Object => None,
        }
    }

    fn neighbours(&self, mode: SelectionMode, x: u32, out: &mut Vec<u32>) {
        out.clear();
        match mode {
            SelectionMode::Points => out.extend(self.point_edges.get(x).iter().map(|&e| self.other(e, x))),
            SelectionMode::Edges => {
                let (a, b) = self.edges[x as usize];
                for v in [a, b] { out.extend(self.point_edges.get(v).iter().copied().filter(|&e| e != x)); }
            }
            SelectionMode::Faces => for &e in self.poly_edges.get(x) { out.extend(self.edge_polys.get(e).iter().copied().filter(|&f| f != x)); },
            SelectionMode::Object => {}
        }
    }
    fn position(&self, p: &[f32], mode: SelectionMode, x: u32) -> [f32; 3] {
        let mean = |ids: &[u32]| { let n = ids.len().max(1) as f32; [0, 1, 2].map(|k| ids.iter().map(|&i| picking::at(p, i)[k]).sum::<f32>() / n) };
        match mode {
            SelectionMode::Points => picking::at(p, x),
            SelectionMode::Edges => { let (a, b) = self.edges[x as usize]; mean(&[a, b]) }
            SelectionMode::Faces => mean(self.poly_verts.get(x)),
            SelectionMode::Object => [0.; 3],
        }
    }
    fn count(&self, mode: SelectionMode) -> usize {
        match mode { SelectionMode::Points => self.point_edges.offs.len().saturating_sub(1), SelectionMode::Edges => self.edges.len(), SelectionMode::Faces => self.npolys as usize, SelectionMode::Object => 0 }
    }

    /// Route from `start` to `target` (ids in `mode`'s space), both included; empty when disconnected.
    pub fn path(&self, p: &[f32], mode: SelectionMode, start: u32, target: u32, method: Method) -> Vec<u32> {
        let n = self.count(mode);
        if start as usize >= n || target as usize >= n { return Vec::new(); }
        if start == target { return vec![start]; }
        match method { Method::Shortest => self.fewest_hops(mode, start, target), Method::Direct => self.most_direct(p, mode, start, target) }
    }
    fn fewest_hops(&self, mode: SelectionMode, start: u32, target: u32) -> Vec<u32> {
        let mut previous = HashMap::from([(start, NONE)]);
        let mut queue = VecDeque::from([start]);
        let mut nb = Vec::new();
        while let Some(x) = queue.pop_front() {
            if x == target { break; }
            self.neighbours(mode, x, &mut nb);
            for &y in &nb { if let std::collections::hash_map::Entry::Vacant(v) = previous.entry(y) { v.insert(x); queue.push_back(y); } }
        }
        if !previous.contains_key(&target) { return Vec::new(); }
        let mut route = vec![target];
        let mut x = target;
        while let Some(&prev) = previous.get(&x) { if prev == NONE { break; } route.push(prev); x = prev; }
        route.reverse();
        route
    }
    /// Pre-rust `directPath`: Dijkstra over (previous, current) so bends can be costed. Penalises length,
    /// distance from the start-target chord and turning.
    fn most_direct(&self, p: &[f32], mode: SelectionMode, start: u32, target: u32) -> Vec<u32> {
        #[derive(PartialEq)]
        struct State { cost: f64, prev: u32, cur: u32 }
        impl Eq for State {}
        impl Ord for State { fn cmp(&self, o: &Self) -> std::cmp::Ordering { o.cost.total_cmp(&self.cost) } }
        impl PartialOrd for State { fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> { Some(self.cmp(o)) } }
        let pos = |x: u32| self.position(p, mode, x);
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
            if pops > 4_000_000 { return self.fewest_hops(mode, start, target); }
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
            self.neighbours(mode, s.cur, &mut nb);
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

    /// Converts `sel` to `to` (pre-rust `setMode`): down to constituents, up only to fully contained elements.
    pub fn convert(&self, sel: &Selection, to: SelectionMode) -> Selection {
        let from = sel.mode;
        let mut out = Selection { mode: to, ..Default::default() };
        if from == to { let mut same = sel.clone(); same.mode = to; return same; }
        if to == SelectionMode::Object || from == SelectionMode::Object { return out; }
        let polys = self.polys_of(&sel.tris);
        let mut points: Vec<u32> = match from {
            SelectionMode::Points => sel.points.clone(),
            SelectionMode::Edges => sel.edges.iter().flat_map(|&(a, b)| [a, b]).collect(),
            _ => polys.iter().flat_map(|&f| self.poly_verts.get(f).iter().copied()).collect(),
        };
        points.sort_unstable(); points.dedup();
        let has = |v: u32| points.binary_search(&v).is_ok();
        match to {
            SelectionMode::Points => out.points = points.clone(),
            SelectionMode::Edges => {
                let mut ids: Vec<u32> = if from == SelectionMode::Faces {
                    polys.iter().flat_map(|&f| self.poly_verts.get(f).iter().flat_map(move |&v| self.point_edges.get(v).iter().copied()
                        .filter(move |&e| self.poly_has(f, self.other(e, v))))).collect()
                } else {
                    points.iter().flat_map(|&v| self.point_edges.get(v).iter().copied().filter(move |&e| has(self.other(e, v)))).collect()
                };
                ids.sort_unstable(); ids.dedup();
                out.edges = ids.into_iter().map(|e| self.edges[e as usize]).collect();
            }
            SelectionMode::Faces => {
                let selected: HashSet<u32> = sel.edges.iter().filter_map(|&(a, b)| self.edge_of(a, b)).collect();
                let faces: Vec<u32> = (0..self.npolys).filter(|&f| if from == SelectionMode::Points {
                    self.poly_verts.get(f).iter().all(|&v| has(v))
                } else {
                    let es = self.poly_edges.get(f);
                    !es.is_empty() && es.iter().all(|e| selected.contains(e))
                }).collect();
                out.tris = self.tris_of(&faces);
            }
            SelectionMode::Object => {}
        }
        out
    }

    /// Applies a click to `current` (see `ClickKind`). `picked` is what a plain click on the hit selects.
    pub fn apply_click(&self, p: &[f32], kind: ClickKind, picked: Option<(Selection, u32 /* hit triangle */, [f32; 3])>, mode: SelectionMode, current: &mut Selection) -> bool {
        if current.mode != mode { *current = Selection { mode, ..Default::default() }; }
        let empty = Selection { mode, ..Default::default() };
        match kind {
            ClickKind::Pick => { *current = picked.map(|x| x.0).unwrap_or(empty); true }
            ClickKind::Toggle => {
                match picked { Some((new, ..)) if !new.is_empty() => toggle(current, &new), _ => *current = empty }
                true
            }
            ClickKind::Subtract => { let Some((new, ..)) = picked else { return false; }; subtract(current, &new); true }
            ClickKind::Loop { ring, add } => {
                let Some((_, tri, point)) = picked else { return false; };
                let Some(edge) = self.nearest_edge(p, tri, point) else { return false; };
                let found = self.loop_selection(p, edge, ring, mode);
                if add { union(current, &found); } else { *current = found; }
                true
            }
            ClickKind::Path(method) => {
                let Some((new, ..)) = picked else { return false; };
                let (Some(start), Some(target)) = (self.active(current), self.element(&new)) else { *current = new; return true; };
                let route = self.path(p, mode, start, target, method);
                if route.is_empty() { return false; }
                let mut found = Selection { mode, ..Default::default() };
                match mode {
                    SelectionMode::Points => found.points = route,
                    SelectionMode::Edges => found.edges = route.into_iter().map(|e| self.edges[e as usize]).collect(),
                    SelectionMode::Faces => found.tris = self.tris_of(&route),
                    SelectionMode::Object => {}
                }
                union(current, &found);
                // the target becomes active so repeated Ctrl-clicks chain
                subtract(current, &new); union(current, &new);
                true
            }
        }
    }
}

/// The topology of `scene`, built on first use and kept in `cell` until the scene changes (the caller resets it).
pub fn cached<'a>(cell: &'a std::cell::OnceCell<Topology>, scene: &crate::viewport::Scene) -> &'a Topology {
    cell.get_or_init(|| Topology::new(scene.positions(), scene.triangle_indices(), scene.edge_indices()))
}

/// Path search variants: pre-rust "Most direct" (default) and "Shortest" (fewest steps).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Method { #[default] Direct, #[allow(dead_code)] /* no UI switch yet (pre-rust Select menu) */ Shortest }

/// What a viewport click does in an element mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClickKind { Pick, Toggle, Subtract, Loop { ring: bool, add: bool }, Path(Method) }

/// Click modifiers to action (docs/RUST_ELEMENT_SELECT.md). `alt_selects` is false under Houdini navigation, where
/// Alt+LMB orbits: an Alt click then does nothing (`None`).
pub fn classify(shift: bool, ctrl: bool, alt: bool, alt_selects: bool) -> Option<ClickKind> {
    Some(match (shift, ctrl, alt) {
        (_, _, true) if !alt_selects => return None,
        (add, ring, true) => ClickKind::Loop { ring, add },
        (true, true, false) => ClickKind::Subtract,
        (false, true, false) => ClickKind::Path(Method::Direct),
        (true, false, false) => ClickKind::Toggle,
        (false, false, false) => ClickKind::Pick,
    })
}

fn union(current: &mut Selection, new: &Selection) {
    for &x in &new.points { if !current.points.contains(&x) { current.points.push(x); } }
    for &x in &new.edges { if !current.edges.contains(&x) { current.edges.push(x); } }
    for &x in &new.tris { if !current.tris.contains(&x) { current.tris.push(x); } }
}
fn subtract(current: &mut Selection, new: &Selection) {
    current.points.retain(|x| !new.points.contains(x));
    current.edges.retain(|x| !new.edges.contains(x));
    current.tris.retain(|x| !new.tris.contains(x));
}
/// Shift-click: remove the element when it is entirely selected, otherwise add it (whole polygons in face mode).
fn toggle(current: &mut Selection, new: &Selection) {
    let all = new.points.iter().all(|x| current.points.contains(x)) && new.edges.iter().all(|x| current.edges.contains(x))
        && new.tris.iter().all(|x| current.tris.contains(x));
    if all { subtract(current, new); } else { union(current, new); }
}

#[cfg(test)]
mod tests {
    use super::*;
    use SelectionMode::{Edges, Faces, Points};

    /// An `nx` by `ny` grid of quads in z = 0, each quad split by a diagonal; point (i, j) is j * (nx + 1) + i.
    fn grid(nx: u32, ny: u32) -> (Vec<f32>, Vec<u32>, Vec<u32>) {
        let w = nx + 1;
        let p: Vec<f32> = (0..=ny).flat_map(|j| (0..=nx).flat_map(move |i| [i as f32, j as f32, 0.])).collect();
        let (mut t, mut e) = (Vec::new(), Vec::new());
        for j in 0..ny { for i in 0..nx {
            let (a, b, c, d) = (j * w + i, j * w + i + 1, (j + 1) * w + i + 1, (j + 1) * w + i);
            t.extend([a, b, c, a, c, d]);
            e.extend([a, b, b, c, c, d, d, a]);
        } }
        (p, t, e)
    }
    /// Quads given as point lists, fan-triangulated, with their perimeter edges.
    fn mesh(p: Vec<f32>, polys: &[&[u32]]) -> (Vec<f32>, Vec<u32>, Vec<u32>) {
        let (mut t, mut e) = (Vec::new(), Vec::new());
        for f in polys {
            for k in 1..f.len() - 1 { t.extend([f[0], f[k], f[k + 1]]); }
            for k in 0..f.len() { e.extend([f[k], f[(k + 1) % f.len()]]); }
        }
        (p, t, e)
    }
    fn cube() -> (Vec<f32>, Vec<u32>, Vec<u32>) {
        let p = (0..8).flat_map(|c| [(c & 1) as f32, ((c >> 1) & 1) as f32, ((c >> 2) & 1) as f32]).collect();
        mesh(p, &[&[0, 2, 3, 1], &[4, 5, 7, 6], &[0, 1, 5, 4], &[2, 6, 7, 3], &[0, 4, 6, 2], &[1, 3, 7, 5]])
    }
    fn topo((p, t, e): &(Vec<f32>, Vec<u32>, Vec<u32>)) -> Topology { Topology::new(p, t, e) }
    fn sorted(mut v: Vec<(u32, u32)>) -> Vec<(u32, u32)> { v.sort(); v }
    fn edge_walk(m: &(Vec<f32>, Vec<u32>, Vec<u32>), a: u32, b: u32, ring: bool) -> Vec<(u32, u32)> {
        let t = topo(m);
        sorted(t.walk(&m.0, t.edge_of(a, b).unwrap(), ring).into_iter().map(|e| t.edges[e as usize]).collect())
    }

    #[test]
    fn polygons_are_rebuilt_from_diagonals() {
        let m = grid(3, 2);
        let t = topo(&m);
        assert_eq!(t.npolys, 6);
        assert_eq!(t.edges.len(), 3 * 3 + 4 * 2);
        assert_eq!(t.poly_of_tri(0), t.poly_of_tri(1));
        assert_ne!(t.poly_of_tri(1), t.poly_of_tri(2));
        assert_eq!(t.poly_tris(t.poly_of_tri(4).unwrap()), &[4, 5]);
        assert!(t.edge_of(0, 5).is_none()); // diagonal of the first quad
    }

    #[test]
    fn grid_loop_runs_border_to_border_and_ring_crosses_strip() {
        let m = grid(4, 3); // w = 5
        // interior horizontal edge (1,1)-(2,1): loop is the whole row j = 1
        assert_eq!(edge_walk(&m, 6, 7, false), (0..4).map(|i| (5 + i, 6 + i)).collect::<Vec<_>>());
        // ring of the same edge: every horizontal edge in column i = 1
        assert_eq!(edge_walk(&m, 6, 7, true), (0..4).map(|j| (5 * j + 1, 5 * j + 2)).collect::<Vec<_>>());
        // a vertical interior edge loops the full column
        assert_eq!(edge_walk(&m, 7, 12, false), (0..3).map(|j| (5 * j + 2, 5 * j + 7)).collect::<Vec<_>>());
    }

    #[test]
    fn grid_border_loop_follows_border_and_stops_at_corners() {
        let m = grid(4, 3);
        assert_eq!(edge_walk(&m, 1, 2, false), (0..4).map(|i| (i, i + 1)).collect::<Vec<_>>());
        // a border edge ring runs across the grid through quads only
        assert_eq!(edge_walk(&m, 0, 5, true), (0..5).map(|i| (i, i + 5)).collect::<Vec<_>>());
    }

    #[test]
    fn cube_loop_stops_at_poles_and_ring_wraps() {
        let m = cube();
        assert_eq!(edge_walk(&m, 0, 1, false), vec![(0, 1)]);
        assert_eq!(edge_walk(&m, 0, 1, true), vec![(0, 1), (2, 3), (4, 5), (6, 7)]);
    }

    #[test]
    fn ring_stops_at_triangles_and_ngons() {
        // three quads in a row, the last one replaced by two triangles (a real edge splits it)
        let p: Vec<f32> = (0..2).flat_map(|j| (0..4).flat_map(move |i| [i as f32, j as f32, 0.])).collect();
        let m = mesh(p, &[&[0, 1, 5, 4], &[1, 2, 6, 5], &[2, 3, 7], &[2, 7, 6]]);
        assert_eq!(edge_walk(&m, 0, 4, true), vec![(0, 4), (1, 5), (2, 6)]);
        // the strip as a pentagon at the end: the ring stops at its edge as well
        let p: Vec<f32> = (0..2).flat_map(|j| (0..4).flat_map(move |i| [i as f32, j as f32, 0.])).chain([3.5, 0.5, 0.]).collect();
        let m = mesh(p, &[&[0, 1, 5, 4], &[1, 2, 6, 5], &[2, 3, 8, 7, 6]]);
        assert_eq!(edge_walk(&m, 0, 4, true), vec![(0, 4), (1, 5), (2, 6)]);
    }

    #[test]
    fn loop_stops_at_a_pole_between_quads() {
        // A valence-3 interior vertex (point 4 of a 3-quad fan) cannot be continued: no straight candidate.
        let p = vec![0., 0., 0., 1., 0., 0., 1., 1., 0., 0., 1., 0., 0.5, 0.5, 0., -1., 0.5, 0., 0.5, -1., 0.];
        // quads around centre 4: (4,0,...) shapes; edge 5-4 enters the pole
        let m = mesh(p, &[&[4, 3, 5, 0], &[4, 0, 6, 1], &[4, 1, 2, 3]]);
        let t = topo(&m);
        assert_eq!(t.point_edges.get(4).len(), 3);
        assert_eq!(edge_walk(&m, 0, 4, false), vec![(0, 4)]);
    }

    #[test]
    fn ngon_cap_perimeter_is_followed() {
        // a tube with 8 columns: bottom ring 0..8, top ring 8..16, top cap is an 8-gon
        let n = 8u32;
        let p: Vec<f32> = (0..2).flat_map(|j| (0..n).flat_map(move |i| { let a = i as f32 * std::f32::consts::TAU / n as f32; [a.cos(), a.sin(), j as f32] })).collect();
        let sides: Vec<Vec<u32>> = (0..n).map(|i| vec![i, (i + 1) % n, n + (i + 1) % n, n + i]).collect();
        let cap: Vec<u32> = (n..2 * n).collect();
        let mut polys: Vec<&[u32]> = sides.iter().map(|v| v.as_slice()).collect();
        polys.push(&cap);
        let m = mesh(p, &polys);
        assert_eq!(edge_walk(&m, 8, 9, false), sorted((0..n).map(|i| key(n + i, n + (i + 1) % n)).collect()));
        // isolated octagon
        let p: Vec<f32> = (0..8).flat_map(|i| { let a = i as f32 * std::f32::consts::FRAC_PI_4; [a.cos(), a.sin(), 0.] }).collect();
        let m = mesh(p, &[&[0, 1, 2, 3, 4, 5, 6, 7]]);
        assert_eq!(edge_walk(&m, 0, 1, false).len(), 8);
    }

    #[test]
    fn ambiguous_wire_branch_stops() {
        let p = vec![-1., 0., 0., 0., 0., 0., 1., 0.4, 0., 1., -0.4, 0.];
        let t = Topology::new(&p, &[], &[0, 1, 1, 2, 1, 3]);
        assert_eq!(t.walk(&p, t.edge_of(0, 1).unwrap(), false), vec![t.edge_of(0, 1).unwrap()]);
        // a straight wire continues
        let p = vec![0., 0., 0., 1., 0., 0., 2., 0., 0., 3., 0., 0.];
        let t = Topology::new(&p, &[], &[0, 1, 1, 2, 2, 3]);
        assert_eq!(t.walk(&p, t.edge_of(1, 2).unwrap(), false).len(), 3);
    }

    #[test]
    fn loop_selection_in_each_mode() {
        let m = grid(4, 3);
        let t = topo(&m);
        let e = t.edge_of(6, 7).unwrap();
        assert_eq!(t.loop_selection(&m.0, e, false, Points).points.len(), 5);
        // face loop: the column of quads crossed by the ring of a horizontal edge
        let faces = t.loop_selection(&m.0, e, false, Faces);
        assert_eq!(faces.tris.len(), 3 * 2);
        assert_eq!(t.polys_of(&faces.tris).len(), 3);
        assert_eq!(faces, t.loop_selection(&m.0, e, true, Faces));
    }

    #[test]
    fn nearest_edge_ignores_diagonals_and_pick_selects_whole_polygons() {
        let m = grid(1, 1);
        let t = topo(&m);
        // a point on the diagonal near corner 0 is closest to the diagonal but must snap to a real edge
        let e = t.nearest_edge(&m.0, 0, [0.3, 0.25, 0.]).unwrap();
        assert_eq!(t.edges[e as usize], (0, 1));
        let hit = Hit { tri: 0, t: 1., bary: [0.4, 0.3, 0.3] };
        assert_eq!(t.pick(&m.0, &m.1, hit, [0.5, 0.45, 0.], Faces).tris, vec![0, 1]);
        assert_eq!(t.pick(&m.0, &m.1, hit, [0.5, 0.1, 0.], Edges).edges, vec![(0, 1)]);
    }

    #[test]
    fn paths_in_every_mode() {
        let m = grid(4, 3); // w = 5
        let t = topo(&m);
        for method in [Method::Direct, Method::Shortest] {
            // straight along a row
            assert_eq!(t.path(&m.0, Points, 5, 9, method), vec![5, 6, 7, 8, 9]);
            let (a, b) = (t.edge_of(5, 6).unwrap(), t.edge_of(8, 9).unwrap());
            let route = t.path(&m.0, Edges, a, b, method);
            assert_eq!(route.len(), 4);
            assert!(route.iter().all(|&e| { let (x, y) = t.edges[e as usize]; (5..=9).contains(&x) && (5..=9).contains(&y) }));
            let (f0, f3) = (t.poly_of_tri(0).unwrap(), t.poly_of_tri(6).unwrap());
            assert_eq!(t.path(&m.0, Faces, f0, f3, method).len(), 4);
        }
        // diagonal across the grid: direct stays near the chord, shortest has the minimum hop count
        let direct = t.path(&m.0, Points, 0, 18, Method::Direct);
        assert_eq!(direct.len(), 7);
        assert_eq!(t.path(&m.0, Points, 0, 18, Method::Shortest).len(), 7);
        // not the L-shaped route along the border (3 units off the x = y chord)
        let off_chord = direct.iter().map(|&v| { let q = picking::at(&m.0, v); (q[0] - q[1]).abs() }).fold(0f32, f32::max);
        assert!(off_chord <= 2., "{direct:?}");
        // disconnected
        let p = vec![0., 0., 0., 1., 0., 0., 5., 0., 0., 6., 0., 0.];
        let w = Topology::new(&p, &[], &[0, 1, 2, 3]);
        assert!(w.path(&p, Points, 0, 3, Method::Direct).is_empty());
        assert!(w.path(&p, Points, 0, 3, Method::Shortest).is_empty());
    }

    #[test]
    fn direct_path_prefers_a_straight_detailed_chain() {
        // 0 -> 4 straight through 1, 2, 3 or a two-step detour via 5 above the line
        let p = vec![0., 0., 0., 1., 0., 0., 2., 0., 0., 3., 0., 0., 4., 0., 0., 2., 1.5, 0.];
        let t = Topology::new(&p, &[], &[0, 1, 1, 2, 2, 3, 3, 4, 0, 5, 5, 4]);
        assert_eq!(t.path(&p, Points, 0, 4, Method::Direct), vec![0, 1, 2, 3, 4]);
        assert_eq!(t.path(&p, Points, 0, 4, Method::Shortest), vec![0, 5, 4]);
    }

    fn sel(mode: SelectionMode) -> Selection { Selection { mode, ..Default::default() } }

    #[test]
    fn conversion_face_down_and_back_up() {
        let m = cube();
        let t = topo(&m);
        let face = Selection { tris: t.poly_tris(0).to_vec(), ..sel(Faces) };
        let edges = t.convert(&face, Edges);
        assert_eq!(edges.edges, sorted(vec![(0, 2), (2, 3), (1, 3), (0, 1)]));
        let points = t.convert(&face, Points);
        assert_eq!(points.points, vec![0, 1, 2, 3]);
        assert_eq!(t.convert(&edges, Faces).tris, t.poly_tris(0));
        assert_eq!(t.convert(&points, Faces).tris, t.poly_tris(0));
        assert_eq!(t.convert(&edges, Points).points, vec![0, 1, 2, 3]);
        assert_eq!(t.convert(&points, Edges).edges, edges.edges);
        assert_eq!(t.convert(&face, Faces), face);
    }

    #[test]
    fn conversion_partial_selections_do_not_grow() {
        let m = cube();
        let t = topo(&m);
        let one = Selection { points: vec![0], ..sel(Points) };
        assert!(t.convert(&one, Edges).edges.is_empty());
        assert!(t.convert(&one, Faces).tris.is_empty());
        let edge = Selection { edges: vec![(0, 1)], ..sel(Edges) };
        assert!(t.convert(&edge, Faces).tris.is_empty());
        assert_eq!(t.convert(&edge, Points).points, vec![0, 1]);
        // three of a face's four edges: no face
        let three = Selection { edges: vec![(0, 1), (1, 3), (2, 3)], ..sel(Edges) };
        assert!(t.convert(&three, Faces).tris.is_empty());
        // two adjacent faces down to edges share their common edge once
        let two = Selection { tris: [t.poly_tris(0), t.poly_tris(2)].concat(), ..sel(Faces) };
        assert_eq!(t.convert(&two, Edges).edges.len(), 7);
        assert_eq!(t.convert(&two, Points).points.len(), 6);
        // all points select every face; object mode conversions are empty
        let all = Selection { points: (0..8).collect(), ..sel(Points) };
        assert_eq!(t.polys_of(&t.convert(&all, Faces).tris).len(), 6);
        assert_eq!(t.convert(&all, Edges).edges.len(), 12);
        assert!(t.convert(&all, SelectionMode::Object).is_empty());
        assert!(t.convert(&sel(SelectionMode::Object), Points).is_empty());
    }

    #[test]
    fn conversion_quad_diagonal_points_need_the_whole_quad() {
        let m = grid(1, 1);
        let t = topo(&m);
        let diag = Selection { points: vec![0, 1, 2], ..sel(Points) };
        assert!(t.convert(&diag, Faces).tris.is_empty());
        assert_eq!(t.convert(&diag, Edges).edges, vec![(0, 1), (0, 2)]);
    }

    #[test]
    fn click_modifier_mapping() {
        assert_eq!(classify(false, false, false, true), Some(ClickKind::Pick));
        assert_eq!(classify(true, false, false, true), Some(ClickKind::Toggle));
        assert_eq!(classify(false, true, false, true), Some(ClickKind::Path(Method::Direct)));
        assert_eq!(classify(true, true, false, true), Some(ClickKind::Subtract));
        assert_eq!(classify(false, false, true, true), Some(ClickKind::Loop { ring: false, add: false }));
        assert_eq!(classify(true, false, true, true), Some(ClickKind::Loop { ring: false, add: true }));
        assert_eq!(classify(false, true, true, true), Some(ClickKind::Loop { ring: true, add: false }));
        assert_eq!(classify(true, true, true, true), Some(ClickKind::Loop { ring: true, add: true }));
        assert_eq!(classify(false, false, true, false), None);
        assert_eq!(classify(false, true, false, false), Some(ClickKind::Path(Method::Direct)));
    }

    #[test]
    fn clicks_apply_loops_paths_toggles() {
        let m = grid(4, 3);
        let t = topo(&m);
        let pick = |v: u32| Some((Selection { points: vec![v], ..sel(Points) }, 0u32, [0f32; 3]));
        let mut cur = sel(Points);
        // Ctrl-click with nothing selected selects the element
        assert!(t.apply_click(&m.0, ClickKind::Path(Method::Direct), pick(5), Points, &mut cur));
        assert_eq!(cur.points, vec![5]);
        t.apply_click(&m.0, ClickKind::Path(Method::Direct), pick(8), Points, &mut cur);
        assert_eq!(cur.points, vec![5, 6, 7, 8]);
        t.apply_click(&m.0, ClickKind::Path(Method::Direct), pick(18), Points, &mut cur);
        assert_eq!(cur.points.last(), Some(&18));
        assert_eq!(cur.points.len(), 6);
        t.apply_click(&m.0, ClickKind::Toggle, pick(18), Points, &mut cur);
        assert!(!cur.points.contains(&18));
        t.apply_click(&m.0, ClickKind::Subtract, pick(5), Points, &mut cur);
        assert!(!cur.points.contains(&5));
        // Alt-click in edge mode near the bottom edge of quad (1, 1): the row loop; Shift adds the ring
        let mut cur = sel(Edges);
        let tri = (5 * 2) as u32; // first triangle of quad (i = 1, j = 1)
        assert!(t.apply_click(&m.0, ClickKind::Loop { ring: false, add: false }, Some((sel(Edges), tri, [1.5, 1.05, 0.])), Edges, &mut cur));
        assert_eq!(sorted(cur.edges.clone()), (0..4).map(|i| (5 + i, 6 + i)).collect::<Vec<_>>());
        t.apply_click(&m.0, ClickKind::Loop { ring: true, add: true }, Some((sel(Edges), tri, [1.5, 1.05, 0.])), Edges, &mut cur);
        assert_eq!(cur.edges.len(), 4 + 3);
        // misses change nothing for loops, paths and subtraction
        let before = cur.clone();
        assert!(!t.apply_click(&m.0, ClickKind::Loop { ring: false, add: false }, None, Edges, &mut cur));
        assert!(!t.apply_click(&m.0, ClickKind::Path(Method::Direct), None, Edges, &mut cur));
        assert_eq!(cur, before);
        // Shift-click toggles whole polygons
        let mut cur = sel(Faces);
        let face = Some((Selection { tris: vec![0, 1], ..sel(Faces) }, 0, [0.5, 0.5, 0.]));
        t.apply_click(&m.0, ClickKind::Toggle, face.clone(), Faces, &mut cur);
        assert_eq!(cur.tris, vec![0, 1]);
        t.apply_click(&m.0, ClickKind::Toggle, face, Faces, &mut cur);
        assert!(cur.tris.is_empty());
    }

    #[test]
    fn complete_expands_polygons_and_drops_diagonals() {
        let m = grid(2, 1);
        let t = topo(&m);
        let s = t.complete(Selection { tris: vec![1], edges: vec![(0, 4), (0, 1)], ..sel(Faces) });
        assert_eq!(s.tris, vec![0, 1]);
        assert_eq!(s.edges, vec![(0, 1)]);
    }
}
