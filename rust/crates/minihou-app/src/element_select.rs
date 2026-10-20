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
    pub(super) core: SelectTopology,
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
    /// Encode the same point, edge-pair, or primitive tokens used by group-consuming nodes.
    pub fn group_string(&self, sel: &Selection) -> String {
        self.tokens(sel.mode, &self.ids(sel)).join(" ")
    }
    /// Inverse of `group_string`: parse point numbers, `a-b` / `p<a>-<b>` edge pairs, or polygon numbers into a selection.
    /// Tokens outside this topology (stale or foreign ids) are dropped, never indexed.
    pub fn parse_group(&self, text: &str, mode: SelectionMode) -> Selection {
        let mut s = Selection { mode, ..Default::default() };
        let tokens: Vec<_> = text.split_whitespace().collect();
        if !tokens.is_empty() && tokens.iter().all(|tok| tok.starts_with('^')) {
            s = self.parse_group("*", mode);
        }
        for tok in tokens.iter().copied().filter(|tok| !tok.starts_with('^')) {
            let tok = tok.trim_start_matches('p');
            let wildcard = tok.contains(['*', '?', '[']);
            match mode {
                SelectionMode::Points if wildcard => for i in 0..self.npoints as u32 {
                    if minihou_core::group_pattern::matches(tok, &i.to_string()) && !s.points.contains(&i) { s.points.push(i); }
                },
                SelectionMode::Points => if let Ok(i) = tok.parse::<u32>() { if (i as usize) < self.npoints && !s.points.contains(&i) { s.points.push(i); } },
                SelectionMode::Edges if wildcard => for &(a, b) in &self.edges {
                    if minihou_core::group_pattern::matches(tok, &format!("{a}-{b}")) && !s.edges.contains(&(a, b)) { s.edges.push((a, b)); }
                },
                SelectionMode::Edges => if let Some((a, b)) = tok.split_once('-') {
                    if let (Ok(a), Ok(b)) = (a.parse::<u32>(), b.parse::<u32>()) {
                        if self.edge_of(a, b).is_some() && !s.edges.contains(&(a, b)) { s.edges.push((a, b)); }
                    }
                },
                SelectionMode::Faces if wildcard => for i in 0..self.npolys {
                    if minihou_core::group_pattern::matches(tok, &i.to_string()) {
                        for &t in self.poly_tris(i) { if !s.tris.contains(&t) { s.tris.push(t); } }
                    }
                },
                SelectionMode::Faces => if let Ok(i) = tok.parse::<u32>() {
                    if i < self.npolys { for &t in self.poly_tris(i) { if !s.tris.contains(&t) { s.tris.push(t); } } }
                },
                SelectionMode::Object => {}
            }
        }
        for tok in tokens.iter().filter_map(|tok| tok.strip_prefix('^')).filter(|tok| !tok.is_empty()) {
            let excluded = self.parse_group(tok, mode);
            s.points.retain(|i| !excluded.points.contains(i));
            s.edges.retain(|e| !excluded.edges.iter().any(|x| key(x.0, x.1) == key(e.0, e.1)));
            s.tris.retain(|t| !excluded.tris.contains(t));
        }
        s
    }
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

    pub(super) fn tris_of(&self, polys: &[u32]) -> Vec<u32> { polys.iter().flat_map(|&f| self.poly_tris(f).iter().copied()).collect() }
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

mod group_nodes;
pub use group_nodes::{find_input_group_field, SELECTION_NODES};

/// The first free `<prefix><N>` name (group1, group2, boundary1, boundary2, ...) among the document's nodes, so each viewport
/// group or selection node gets its own unique name.
pub fn free_group_name_with_prefix(doc: &serde_json::Value, prefix: &str) -> String {
    let used: std::collections::BTreeSet<&str> = doc["nodes"].as_object().into_iter().flat_map(|m| m.values())
        .flat_map(|n| {
            let p = &n["params"];
            [p["name"].as_str(), p["output_group_name"].as_str(), p["group_name"].as_str()]
        })
        .flatten()
        .collect();
    (1..).map(|i| format!("{prefix}{i}")).find(|n| !used.contains(n.as_str())).unwrap_or_default()
}

pub fn free_group_name(doc: &serde_json::Value) -> String {
    free_group_name_with_prefix(doc, "group")
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
mod tests;
