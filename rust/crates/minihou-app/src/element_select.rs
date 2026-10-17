//! Element-mode selection on the displayed mesh: polygons rebuilt from the fan triangles and the polygon edge list,
//! edge loops and rings, Ctrl-click paths and selection conversion between point/edge/face modes. Ported from the
//! pre-rust browser viewport (preview.js `quadStrip`, `pathPick`, `setMode`; selection-tools.js `directPath`).
//! Behaviour spec: docs/RUST_ELEMENT_SELECT.md.
use crate::picking::{self, Hit, Selection};
use crate::ui::SelectionMode;
use minihou_core::select_topo::{key, Csr, Domain, PathMethod, SelectTopology};
use std::collections::{HashMap, HashSet};

const NONE: u32 = u32::MAX;

/// Polygon/edge adjacency of one displayed mesh; build once per cooked scene. The loop, ring, path and grow logic is the
/// shared `minihou_core::select_topo::SelectTopology` (also used by the selection nodes), reached through `Deref`.
#[derive(Debug, Default)]
pub struct Topology {
    core: SelectTopology,
    /// Polygon of each triangle (`NONE` for invalid triangles).
    tri_poly: Vec<u32>,
    poly_tris: Csr,
}
impl std::ops::Deref for Topology {
    type Target = SelectTopology;
    fn deref(&self) -> &SelectTopology { &self.core }
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f64; 3] { [0, 1, 2].map(|k| a[k] as f64 - b[k] as f64) }
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 { a[0] * b[0] + a[1] * b[1] + a[2] * b[2] }
fn len(a: [f64; 3]) -> f64 { dot(a, a).sqrt() }

/// The shared topology's domain for an element selection mode (`None` in object mode).
pub fn domain(mode: SelectionMode) -> Option<Domain> {
    match mode { SelectionMode::Points => Some(Domain::Points), SelectionMode::Edges => Some(Domain::Edges), SelectionMode::Faces => Some(Domain::Faces), SelectionMode::Object => None }
}

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
        Self {
            core: SelectTopology::from_parts(npoints, edges, vert_pairs, edge_pairs, npolys),
            poly_tris: Csr::build(npolys as usize, &poly_tri_pairs),
            tri_poly,
        }
    }
    pub fn poly_of_tri(&self, tri: u32) -> Option<u32> { self.tri_poly.get(tri as usize).copied().filter(|&p| p != NONE) }
    pub fn poly_tris(&self, poly: u32) -> &[u32] { self.poly_tris.get(poly) }
    pub fn polygons(&self) -> std::ops::Range<u32> { 0..self.npolys }
    pub fn polygon_vertices(&self, poly: u32) -> &[u32] { self.poly_verts.get(poly) }
    pub fn polygon_edges(&self, poly: u32) -> impl Iterator<Item = (u32, u32)> + '_ {
        self.poly_edges.get(poly).iter().map(|&id| self.edges[id as usize])
    }

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

    /// The loop or ring selection in `mode` through edge `edge`. Faces mode gives the face loop (polygons of the ring).
    pub fn loop_selection(&self, p: &[f32], edge: u32, ring: bool, mode: SelectionMode) -> Selection {
        let mut s = Selection { mode, ..Default::default() };
        match mode {
            SelectionMode::Object => {}
            SelectionMode::Edges => s.edges = self.walk(p, edge, ring).into_iter().map(|e| self.edges[e as usize]).collect(),
            SelectionMode::Points => s.points = self.points_of_edges(&self.walk(p, edge, ring)),
            SelectionMode::Faces => s.tris = self.tris_of(&self.faces_of_edges(&self.walk(p, edge, true))),
        }
        s
    }

    fn tris_of(&self, polys: &[u32]) -> Vec<u32> { polys.iter().flat_map(|&f| self.poly_tris(f).iter().copied()).collect() }
    pub(crate) fn polys_of(&self, tris: &[u32]) -> Vec<u32> {
        let mut seen = HashSet::new();
        tris.iter().filter_map(|&t| self.poly_of_tri(t)).filter(|f| seen.insert(*f)).collect()
    }

    /// Constant-time face-marker lookup after one pass over selected triangles.
    pub(crate) fn polygon_selection_mask(&self, tris: &[u32]) -> Vec<bool> {
        let mut selected = vec![false; self.npolys as usize];
        for &tri in tris {
            if let Some(poly) = self.poly_of_tri(tri) { selected[poly as usize] = true; }
        }
        selected
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

    /// Route from `start` to `target` (ids in `mode`'s space), both included; empty when disconnected.
    pub fn path(&self, p: &[f32], mode: SelectionMode, start: u32, target: u32, method: Method) -> Vec<u32> {
        let Some(domain) = domain(mode) else { return Vec::new(); };
        self.core.path(p, domain, start, target, match method { Method::Direct => PathMethod::MostDirect, Method::Shortest => PathMethod::FewestSteps })
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

    /// Apply a point or wire-edge click without requiring a surface triangle.
    pub fn apply_wire_click(&self, p: &[f32], kind: ClickKind, picked: Option<Selection>, mode: SelectionMode, current: &mut Selection) -> bool {
        if let ClickKind::Loop { ring, add } = kind {
            let Some(edge) = picked.as_ref().and_then(|s| s.edges.first()).and_then(|&(a,b)| self.edge_of(a,b)) else { return false; };
            let found = self.loop_selection(p,edge,ring,mode);
            if add { union(current,&found); } else { *current=found; }
            return true;
        }
        // Pick, toggle, subtract and path use only the element; the triangle payload is unused.
        self.apply_click(p,kind,picked.map(|s|(s,0,[0.;3])),mode,current)
    }
}

#[cfg(test)] mod wire_tests {
    use super::*;
    #[test] fn wire_path_and_loop_are_selection_only() {
        let p=[0.,0.,0.,1.,0.,0.,2.,0.,0.,3.,0.,0.];
        let topo=Topology::new(&p,&[],&[0,1,1,2,2,3]);
        let mut current=Selection {mode:SelectionMode::Points,points:vec![0],..Default::default()};
        assert!(topo.apply_wire_click(&p,ClickKind::Path(Method::Shortest),Some(Selection {mode:SelectionMode::Points,points:vec![3],..Default::default()}),SelectionMode::Points,&mut current));
        assert_eq!(current.points,[0,1,2,3]);
        let mut edges=Selection {mode:SelectionMode::Edges,..Default::default()};
        assert!(topo.apply_wire_click(&p,ClickKind::Loop {ring:false,add:false},Some(Selection {mode:SelectionMode::Edges,edges:vec![(1,2)],..Default::default()}),SelectionMode::Edges,&mut edges));
        assert!(!edges.edges.is_empty());
    }
}

/// Selection nodes that take their seeds from the viewport selection when added from the Tab menu (docs/RUST_SELECT_NODES.md).
pub const SELECTION_NODES: [&str; 4] = ["grow", "select_path", "select_loop", "select_ring"];

impl Topology {
    /// The ids of `sel` in its mode's id space (points, edge ids, polygons), in selection order.
    fn ids(&self, sel: &Selection) -> Vec<u32> {
        match sel.mode {
            SelectionMode::Points => sel.points.clone(),
            SelectionMode::Edges => sel.edges.iter().filter_map(|&(a, b)| self.edge_of(a, b)).collect(),
            SelectionMode::Faces => self.polys_of(&sel.tris),
            SelectionMode::Object => Vec::new(),
        }
    }
    /// Element tokens of the selection-node patterns: point / primitive numbers, `p<a>-<b>` edges.
    fn tokens(&self, mode: SelectionMode, ids: &[u32]) -> Vec<String> {
        ids.iter().map(|&i| match mode {
            SelectionMode::Edges => { let (a, b) = self.edges[i as usize]; format!("p{a}-{b}") }
            _ => i.to_string(),
        }).collect()
    }

    /// Expand connected components or invert, within the currently pickable domain.
    pub fn linked_or_inverse(&self, sel:&mut Selection, mode:SelectionMode, inverse:bool, visible:impl Fn(u32)->bool) -> bool {
        let Some(domain)=domain(mode) else {return false;};
        let eligible=|id:u32| match mode {
            SelectionMode::Points=>visible(id),
            SelectionMode::Edges=>{let (a,b)=self.edges[id as usize];visible(a)&&visible(b)},
            SelectionMode::Faces=>self.polygon_vertices(id).iter().all(|&v|visible(v)),
            _=>false,
        };
        let ids=if sel.mode==mode {self.ids(sel)}else{Vec::new()};
        let mut mask=vec![false;self.count(domain)];
        for &id in &ids {if let Some(value)=mask.get_mut(id as usize) {if eligible(id) {*value=true;}}}
        if inverse {for (id,value) in mask.iter_mut().enumerate() {*value=!*value&&eligible(id as u32);}}
        else {
            let mut pending:Vec<_>=ids.iter().copied().filter(|&id|mask.get(id as usize)==Some(&true)).collect();
            let mut neighbours=Vec::new();
            while let Some(id)=pending.pop() {
                self.neighbours(domain,id,&mut neighbours);
                for &next in &neighbours {if !mask[next as usize]&&eligible(next) {mask[next as usize]=true;pending.push(next);}}
            }
        }
        let mut next=Selection {mode,..Default::default()};
        let selected:Vec<_>=mask.iter().enumerate().filter(|(_,value)|**value).map(|(id,_)|id as u32).collect();
        match mode {
            SelectionMode::Points=>next.points=selected,
            SelectionMode::Edges=>next.edges=selected.into_iter().map(|id|self.edges[id as usize]).collect(),
            SelectionMode::Faces=>next.tris=self.tris_of(&selected),
            _=>{},
        }
        let changed=*sel!=next;*sel=next;changed
    }

    /// Select More / Less (Blender Ctrl+Numpad +/-, Face Step on) on the viewport selection. Selection state only: the
    /// `grow` node is never created by this. Kept elements keep their order; added ones follow in id order. Returns
    /// whether the selection changed.
    pub fn grow_selection(&self, sel: &mut Selection, shrink: bool) -> bool {
        let Some(d) = domain(sel.mode) else { return false; };
        let ids = self.ids(sel);
        if ids.is_empty() { return false; }
        let mut mask = vec![false; self.count(d)];
        for &i in &ids { if let Some(m) = mask.get_mut(i as usize) { *m = true; } }
        self.core.grow(d, &mut mask, 1, true, shrink);
        let mut next: Vec<u32> = ids.iter().copied().filter(|&i| mask.get(i as usize) == Some(&true)).collect();
        let kept: HashSet<u32> = next.iter().copied().collect();
        next.extend(mask.iter().enumerate().filter(|(i, m)| **m && !kept.contains(&(*i as u32))).map(|(i, _)| i as u32));
        let before = sel.count();
        match sel.mode {
            SelectionMode::Points => sel.points = next,
            SelectionMode::Edges => sel.edges = next.into_iter().map(|e| self.edges[e as usize]).collect(),
            SelectionMode::Faces => sel.tris = self.tris_of(&next),
            SelectionMode::Object => {}
        }
        sel.count() != before
    }

    /// Parameters for a selection node added from the Tab menu while elements are selected: `grow` gets the selection
    /// as its group, `select_loop` / `select_ring` the selected edges as seeds (other modes converted to edges),
    /// `select_path` the first two selected elements as waypoints. `None` for other nodes or an empty selection.
    pub fn node_params(&self, node_type: &str, sel: &Selection) -> Option<serde_json::Value> {
        if !SELECTION_NODES.contains(&node_type) || sel.is_empty() { return None; }
        let name = match sel.mode { SelectionMode::Points => "points", SelectionMode::Edges => "edges", SelectionMode::Faces => "primitives", SelectionMode::Object => return None };
        let ids = self.ids(sel);
        if ids.is_empty() { return None; }
        let join = |mode: SelectionMode, ids: &[u32]| self.tokens(mode, ids).join(" ");
        Some(match node_type {
            "grow" => serde_json::json!({"group": join(sel.mode, &ids), "domain": name}),
            "select_path" => serde_json::json!({"waypoints": join(sel.mode, &ids[..ids.len().min(2)]), "domain": name}),
            _ => {
                let edges = if sel.mode == SelectionMode::Edges { ids } else { self.ids(&self.convert(sel, SelectionMode::Edges)) };
                if edges.is_empty() { return None; }
                serde_json::json!({"seeds": join(SelectionMode::Edges, &edges), "domain": name})
            }
        })
    }
}

/// The topology of `scene`, built on first use and kept in `cell` until the scene changes (the caller resets it).
pub fn cached<'a>(cell: &'a std::cell::OnceCell<Topology>, scene: &crate::viewport::Scene) -> &'a Topology {
    cell.get_or_init(|| Topology::new(scene.positions(), scene.triangle_indices(), scene.edge_indices()))
}

/// Fingerprint of the element numbering a selection refers to: point count, fan triangles and polygon edges.
pub fn topology_key(npoints: usize, tris: &[u32], edges: &[u32]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325_u64 ^ npoints as u64;
    let mut mix = |x: u64| { h = (h ^ x).wrapping_mul(0x0000_0100_0000_01b3).rotate_left(5); };
    mix(tris.len() as u64);
    for &x in tris { mix(x as u64); }
    mix(edges.len() as u64);
    for &x in edges { mix(x as u64); }
    h
}

/// Element selections (point ids, edge point pairs, triangle ids) only mean something on the topology they were made
/// on. A cook that changes the topology (Ctrl+B bevel, divisions, a new node) would otherwise keep the old numbers and
/// the highlight would draw old point pairs as orange lines across the new faces and half-quad triangles. Each
/// topology keeps its own selection: a new topology starts empty, returning to a known one (cancel, undo) restores it.
#[derive(Debug, Default)]
pub struct SelectionMemory<T> { key: Option<u64>, saved: Vec<(u64, T)> }
const SELECTION_MEMORY: usize = 8;
impl<T: Default> SelectionMemory<T> {
    /// Called for each newly cooked scene with the current selection state; returns the state to use from now on.
    pub fn retarget(&mut self, key: u64, current: T) -> T {
        match self.key.replace(key) {
            None => current,
            Some(old) if old == key => current,
            Some(old) => {
                self.saved.retain(|(k, _)| *k != old);
                self.saved.push((old, current));
                if self.saved.len() > SELECTION_MEMORY { self.saved.remove(0); }
                self.saved.iter().position(|(k, _)| *k == key).map(|i| self.saved.remove(i).1).unwrap_or_default()
            }
        }
    }
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

fn add_unique<T: Copy + Eq + std::hash::Hash>(current: &mut Vec<T>, new: &[T]) {
    if new.is_empty() { return; }
    let mut seen: HashSet<T> = current.iter().copied().collect();
    current.extend(new.iter().copied().filter(|x| seen.insert(*x)));
}
fn remove_items<T: Copy + Eq + std::hash::Hash>(current: &mut Vec<T>, new: &[T]) {
    if new.is_empty() { return; }
    let removed: HashSet<T> = new.iter().copied().collect();
    current.retain(|x| !removed.contains(x));
}
// Point/triangle IDs are dense on cooked meshes; avoid hashing the full growing
// selection for every brush frame. Sparse or malformed IDs keep the bounded fallback.
fn merge_ids(current: &mut Vec<u32>, new: &[u32], subtract: bool) {
    if new.is_empty() { return; }
    let size=current.iter().chain(new).copied().max().unwrap_or(0) as usize+1;
    if size>(current.len()+new.len()).saturating_mul(16).saturating_add(1024) {
        if subtract {remove_items(current,new);} else {add_unique(current,new);}
        return;
    }
    let mut mask=vec![false;size];
    if subtract {
        for &id in new {mask[id as usize]=true;}
        current.retain(|&id|!mask[id as usize]);
    } else {
        for &id in current.iter() {mask[id as usize]=true;}
        current.extend(new.iter().copied().filter(|&id| {let seen=&mut mask[id as usize];let fresh=!*seen;*seen=true;fresh}));
    }
}
fn union(current: &mut Selection, new: &Selection) {
    merge_ids(&mut current.points,&new.points,false);
    add_unique(&mut current.edges,&new.edges);
    merge_ids(&mut current.tris,&new.tris,false);
}
fn subtract(current: &mut Selection, new: &Selection) {
    merge_ids(&mut current.points,&new.points,true);
    remove_items(&mut current.edges,&new.edges);
    merge_ids(&mut current.tris,&new.tris,true);
}
/// Merge a region once, preserving active-element order and avoiding no-op redraw work.
pub fn apply_region(current: &mut Selection, found: Selection, op: picking::SelectOp) -> bool {
    let mode_changed=current.mode != found.mode;
    if mode_changed { *current=Selection {mode:found.mode,..Default::default()}; }
    if op == picking::SelectOp::Replace {
        let changed=*current!=found; *current=found; return mode_changed || changed;
    }
    let before=current.count();
    match op {picking::SelectOp::Add=>union(current,&found),picking::SelectOp::Subtract=>subtract(current,&found),_=>unreachable!()}
    mode_changed || current.count()!=before
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

    #[test] fn linked_and_inverse_respect_components_and_visibility_in_all_modes() {
        let m=mesh(vec![0.;24],&[&[0,1,2,3],&[4,5,6,7]]);
        let t=topo(&m);
        for mode in [Points,Edges,Faces] {
            let mut sel=Selection {mode,..Default::default()};
            match mode {Points=>sel.points=vec![0],Edges=>sel.edges=vec![(0,1)],Faces=>sel.tris=vec![0],_=>{}}
            assert!(t.linked_or_inverse(&mut sel,mode,false,|_|true));
            match mode {Points=>assert_eq!(sel.points,vec![0,1,2,3]),Edges=>assert_eq!(sel.edges.len(),4),Faces=>assert_eq!(sel.tris,vec![0,1]),_=>{}}
            assert!(t.linked_or_inverse(&mut sel,mode,true,|_|true));
            match mode {Points=>assert_eq!(sel.points,vec![4,5,6,7]),Edges=>assert!(sel.edges.iter().all(|(a,b)|*a>=4&&*b>=4)),Faces=>assert_eq!(sel.tris,vec![2,3]),_=>{}}
            assert!(t.linked_or_inverse(&mut sel,mode,true,|id|id>=4));
            assert!(sel.is_empty(),"inverse cannot select hidden components");
            assert!(!t.linked_or_inverse(&mut sel,mode,false,|_|true));
        }
    }
    #[test]
    fn brush_merges_preserve_order_detect_noops_and_subtract() {
        let mut selection=Selection {mode:Faces,tris:(0..50_562).collect(),..Default::default()};
        let dab=Selection {mode:Faces,tris:vec![7,2,7,50_562,50_563],..Default::default()};
        let start=std::time::Instant::now();
        assert!(apply_region(&mut selection,dab.clone(),picking::SelectOp::Add));
        assert_eq!(&selection.tris[50_562..],&[50_562,50_563]);
        assert!(!apply_region(&mut selection,dab.clone(),picking::SelectOp::Add));
        assert!(apply_region(&mut selection,dab.clone(),picking::SelectOp::Subtract));
        assert!(!apply_region(&mut selection,dab,picking::SelectOp::Subtract));
        assert!(!selection.tris.contains(&7));
        assert_eq!(&selection.tris[..3],&[0,1,3]);
        eprintln!("Four large brush merges: {:?}",start.elapsed());
        let sparse=Selection {mode:Faces,tris:vec![u32::MAX],..Default::default()};
        assert!(apply_region(&mut selection,sparse.clone(),picking::SelectOp::Add));
        assert!(!apply_region(&mut selection,sparse.clone(),picking::SelectOp::Add));
        assert!(apply_region(&mut selection,sparse,picking::SelectOp::Subtract));
        let subtract=Selection {mode:Points,points:vec![3],..Default::default()};
        assert!(apply_region(&mut selection,subtract,picking::SelectOp::Subtract));
        assert!(selection.is_empty()); assert_eq!(selection.mode,Points);
    }
    #[test]
    fn face_marker_mask_matches_polygon_selection_on_startup_mesh() {
        let graph = minihou_core::Graph::from_json(include_str!("default_graph.json")).unwrap();
        let geo = graph.cook(Some("output1")).unwrap();
        let scene = crate::viewport::Scene::new(&geo);
        let topo = Topology::new(scene.positions(), scene.triangle_indices(), scene.edge_indices());
        let all: Vec<_> = (0..scene.triangles() as u32).collect();
        for selected in [vec![], vec![0, 0, 3, u32::MAX], all] {
            let expected: HashSet<_> = topo.polys_of(&selected).into_iter().collect();
            let start = std::time::Instant::now();
            let mask = topo.polygon_selection_mask(&selected);
            let count = topo.polygons().filter(|&poly| mask[poly as usize]).count();
            eprintln!("Face markers: {} selected triangles, {count} selected polygons, {:?}", selected.len(), start.elapsed());
            assert_eq!(mask.len(), topo.polygons().len());
            for poly in topo.polygons() { assert_eq!(mask[poly as usize], expected.contains(&poly)); }
        }
    }

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
    fn face_overlay_uses_one_quad_and_only_its_perimeter() {
        let m = grid(1, 1);
        let topology = topo(&m);
        assert_eq!(topology.polygons().len(), 1);
        assert_eq!(topology.polygon_vertices(0), &[0, 1, 2, 3]);
        assert_eq!(topology.polygon_edges(0).count(), 4);
        assert!(!topology.polygon_edges(0).any(|(a, b)| (a, b) == (0, 3)));
        assert_eq!(topology.polys_of(&[0, 1]), vec![0]);
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
    fn select_more_and_less_change_only_the_selection() {
        let m = grid(4, 4); // w = 5
        let t = topo(&m);
        let mut s = Selection { points: vec![12], ..sel(Points) };
        assert!(t.grow_selection(&mut s, false));
        assert_eq!(s.points[0], 12, "kept elements keep their order");
        assert_eq!(s.points.len(), 9, "face step: the four quads round the point");
        assert!(t.grow_selection(&mut s, true));
        assert_eq!(s.points, vec![12]);
        // faces: whole polygons, one ring by shared points
        let centre = t.poly_of_tri(2 * (4 + 1)).unwrap();
        let mut s = Selection { tris: t.poly_tris(centre).to_vec(), ..sel(Faces) };
        t.grow_selection(&mut s, false);
        assert_eq!(t.polys_of(&s.tris).len(), 9);
        assert_eq!(s.tris.len(), 18);
        let mut s = Selection { edges: vec![(6, 7)], ..sel(Edges) };
        t.grow_selection(&mut s, false);
        assert!(s.edges.contains(&(6, 7)) && s.edges.len() > 4);
        // empty and object selections are left alone
        let mut empty = sel(Points);
        assert!(!t.grow_selection(&mut empty, false));
        assert!(!t.grow_selection(&mut sel(SelectionMode::Object), false));
    }

    #[test]
    fn selection_nodes_take_their_seeds_from_the_selection() {
        let m = grid(4, 3);
        let t = topo(&m);
        let points = Selection { points: vec![7, 3, 9], ..sel(Points) };
        assert_eq!(t.node_params("grow", &points).unwrap(), serde_json::json!({"group": "7 3 9", "domain": "points"}));
        assert_eq!(t.node_params("select_path", &points).unwrap(), serde_json::json!({"waypoints": "7 3", "domain": "points"}));
        let edges = Selection { edges: vec![(6, 7), (7, 8)], ..sel(Edges) };
        assert_eq!(t.node_params("select_loop", &edges).unwrap(), serde_json::json!({"seeds": "p6-7 p7-8", "domain": "edges"}));
        assert_eq!(t.node_params("select_ring", &edges).unwrap()["seeds"], "p6-7 p7-8");
        let face = Selection { tris: vec![0, 1], ..sel(Faces) };
        assert_eq!(t.node_params("grow", &face).unwrap(), serde_json::json!({"group": "0", "domain": "primitives"}));
        // faces convert to their edges as loop seeds
        assert_eq!(t.node_params("select_ring", &face).unwrap()["seeds"].as_str().unwrap().split(' ').count(), 4);
        assert!(t.node_params("blast", &points).is_none());
        assert!(t.node_params("grow", &sel(Points)).is_none());
    }

    #[test]
    fn complete_expands_polygons_and_drops_diagonals() {
        let m = grid(2, 1);
        let t = topo(&m);
        let s = t.complete(Selection { tris: vec![1], edges: vec![(0, 4), (0, 1)], ..sel(Faces) });
        assert_eq!(s.tris, vec![0, 1]);
        assert_eq!(s.edges, vec![(0, 1)]);
    }

    /// Polygon edges of ngons and a beveled box corner never include fan/triangulation diagonals, and the
    /// rebuilt polygons own exactly their sides.
    #[test]
    fn ngon_and_bevel_corner_edges_have_no_interior_diagonals() {
        use minihou_core::Geometry;
        let check = |g: &Geometry, what: &str| {
            let scene = crate::viewport::Scene::new(g);
            let t = &g.topology;
            let mut sides = HashSet::new();
            for f in 0..g.nprims() {
                let vs: Vec<u32> = (t.prim_offsets[f] as usize..t.prim_offsets[f + 1] as usize).map(|v| t.vertex_point[v] as u32).collect();
                for i in 0..vs.len() { sides.insert(key(vs[i], vs[(i + 1) % vs.len()])); }
            }
            let edges: HashSet<(u32, u32)> = scene.edge_indices().chunks_exact(2).map(|e| key(e[0], e[1])).collect();
            assert_eq!(edges, sides, "{what}: edge list must be exactly the polygon sides");
            let topo = Topology::new(scene.positions(), scene.triangle_indices(), scene.edge_indices());
            assert_eq!(topo.polygons().len(), g.nprims(), "{what}: one polygon per primitive");
            for poly in topo.polygons() {
                let n = topo.polygon_vertices(poly).len();
                assert_eq!(topo.polygon_edges(poly).count(), n, "{what}: polygon {poly} outline must be its {n} sides");
                assert!(topo.polygon_edges(poly).all(|(a, b)| sides.contains(&key(a, b))));
            }
        };
        // pentagon, hexagon and a concave (arrow) hexagon side by side
        let mut p = Vec::new(); let mut vp = Vec::new(); let mut offs = vec![0];
        for (k, n) in [5usize, 6].into_iter().enumerate() {
            let base = p.len() as i32 / 3;
            for i in 0..n { let a = i as f32 / n as f32 * std::f32::consts::TAU; p.extend([a.cos() + k as f32 * 3., a.sin(), 0.]); vp.push(base + i as i32); }
            offs.push(vp.len() as i32);
        }
        let base = p.len() as i32 / 3;
        for q in [[0., 0.], [2., 0.], [2., 2.], [1., 0.8], [0., 2.], [-0.5, 1.]] { p.extend([q[0] + 9., q[1], 0.]); }
        vp.extend((0..6).map(|i| base + i)); offs.push(vp.len() as i32);
        check(&Geometry::from_arrays(p, vp, offs, None).unwrap(), "ngons");
        // the beveled box corner (three beveled edges meeting, divisions 2-4: hexagon to octagon caps and corner patches)
        for div in [2, 3, 4] {
            let doc = minihou_app::Document::parse(serde_json::json!({"nodes":{"box":{"type":"box"},
                "bev":{"type":"polybevel","inputs":{"geo":"box"},"params":{"distance":0.1,"divisions":div,"edges":"0-1 0-2 0-4 0-3 0-5 0-6 0-7"}}},
                "display":"bev"}).to_string()).unwrap();
            let g = doc.session().unwrap().cook(Some("bev")).unwrap();
            check(&g, &format!("bevel divisions {div}"));
        }
    }

    #[test]
    fn selections_follow_their_topology() {
        let sel = |a: u32, b: u32| Selection { mode: SelectionMode::Edges, edges: vec![(a, b)], ..Default::default() };
        let mut memory = SelectionMemory::default();
        let boxed = topology_key(8, &[0, 1, 2], &[0, 1, 1, 2]);
        let beveled = topology_key(12, &[0, 1, 2, 2, 3, 0], &[0, 1, 1, 2, 2, 3]);
        assert_ne!(boxed, beveled);
        assert_eq!(memory.retarget(boxed, sel(0, 1)), sel(0, 1));
        // the same topology (a param drag) keeps the selection
        assert_eq!(memory.retarget(boxed, sel(0, 1)), sel(0, 1));
        // bevel changes the numbering: the old point pairs must not be highlighted on the new mesh
        assert!(memory.retarget(beveled, sel(0, 1)).is_empty());
        // cancel / undo back to the box restores its selection; the beveled one is kept too
        assert_eq!(memory.retarget(boxed, sel(5, 6)), sel(0, 1));
        assert_eq!(memory.retarget(beveled, Selection::default()), sel(5, 6));
    }
}
