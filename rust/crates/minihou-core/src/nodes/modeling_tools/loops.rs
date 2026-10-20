use super::*;

pub(super) type Pair = (usize, usize);
pub(super) fn key(a: usize, b: usize) -> Pair {
    (a.min(b), a.max(b))
}
fn walk(
    owners: &BTreeMap<Pair, Vec<usize>>,
    quads: &BTreeMap<usize, Vec<usize>>,
    mut p: usize,
    mut f: usize,
    mut t: usize,
    start: Pair,
) -> Vec<(usize, Pair, Pair)> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    loop {
        if !seen.insert(p) {
            break;
        }
        let Some(q) = quads.get(&p) else {
            break;
        };
        let Some(k) = q.iter().position(|&v| v == f) else {
            break;
        };
        let (a, b) = if q[(k + 1) % 4] == t {
            (q[(k + 3) % 4], q[(k + 2) % 4])
        } else if q[(k + 3) % 4] == t {
            (q[(k + 1) % 4], q[(k + 2) % 4])
        } else {
            break;
        };
        out.push((p, (f, t), (a, b)));
        let edge = key(a, b);
        if edge == start {
            break;
        }
        let nb: Vec<_> = owners[&edge].iter().copied().filter(|&i| i != p).collect();
        if nb.len() != 1 {
            break;
        }
        p = nb[0];
        f = a;
        t = b;
    }
    out
}
/// Fewest/most loops one polysplit (and the viewport Loop Cut count) allows, as in Python's `Int("loops", min=1, max=64)`.
pub const LOOP_CUT_MAX: usize = 64;
/// Where the new points sit along each ring edge, measured from its "from" end: `position` for a single loop, else even.
pub fn loop_cut_fractions(loops: usize, position: f64) -> Vec<f64> {
    if loops <= 1 {
        vec![position]
    } else {
        (0..loops).map(|j| (j + 1) as f64 / (loops + 1) as f64).collect()
    }
}
/// Signed displacement of all cuts by one interval. Zero preserves polysplit's Python semantics.
pub fn loop_slide_fractions(loops: usize, position: f64, factor: f64, clamp: bool) -> Vec<f64> {
    if factor == 0. { return loop_cut_fractions(loops, position); }
    let f = if clamp { factor.clamp(-1., 1.) } else { factor };
    let base = if loops == 1 { 0.5 } else { 0. };
    (0..loops).map(|j| if loops == 1 { base + 0.5 * f } else { (j + 1) as f64 / (loops + 1) as f64 + f / (loops + 1) as f64 }).collect()
}
/// Edge/quad adjacency polysplit walks. Shared with the viewport Loop Cut preview so hover and cook agree exactly.
#[derive(Clone, Debug, Default)]
pub struct LoopTopology {
    pub(super) owners: BTreeMap<Pair, Vec<usize>>,
    pub(super) quads: BTreeMap<usize, Vec<usize>>,
}
/// The ring edges a set of seeds cuts (undirected key -> from/to; fractions count from `from`)
/// and the quads that become strips (quad -> entry edge, opposite edge).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LoopRing {
    pub dirs: BTreeMap<Pair, Pair>,
    pub quads: BTreeMap<usize, (Pair, Pair)>,
}
impl LoopTopology {
    pub fn new(g: &Geometry) -> Self {
        Self::with_edges(g, &topo::edges(&g.topology, g.npoints))
    }
    pub(super) fn with_edges(g: &Geometry, e: &topo::Edges) -> Self {
        let (pv, _, _) = corners(g);
        let vp = &g.topology.vertex_point;
        let mut owners = BTreeMap::<Pair, Vec<usize>>::new();
        for v in 0..g.nvertices() {
            if e.of_vertex[v] < 0 {
                continue;
            }
            let i = e.of_vertex[v] as usize;
            owners
                .entry((e.points[2 * i] as usize, e.points[2 * i + 1] as usize))
                .or_default()
                .push(pv[v]);
        }
        let quads = (0..g.nprims())
            .filter(|&i| range(g, i).len() == 4)
            .map(|i| (i, range(g, i).map(|v| vp[v] as usize).collect::<Vec<_>>()))
            .collect();
        Self { owners, quads }
    }
    /// Every edge as a sorted point pair.
    pub fn edges(&self) -> impl Iterator<Item = Pair> + '_ {
        self.owners.keys().copied()
    }
    /// Walk each seed's quad strip on both sides until a non-quad, a boundary, a non-manifold edge or the start.
    /// Seeds are sorted point pairs; an edge with no faces or more than two is skipped (like Python's `_walk`).
    pub fn ring(&self, seeds: impl IntoIterator<Item = Pair>) -> LoopRing {
        let mut out = LoopRing::default();
        for (a, b) in seeds {
            let k = key(a, b);
            let Some(owners) = self.owners.get(&k) else {
                continue;
            };
            if out.dirs.contains_key(&k) || !(1..=2).contains(&owners.len()) {
                continue;
            }
            out.dirs.insert(k, k);
            for &face in owners {
                for (q, entry, opp) in walk(&self.owners, &self.quads, face, k.0, k.1, k) {
                    if out.quads.contains_key(&q) {
                        continue;
                    }
                    out.quads.insert(q, (entry, opp));
                    for (f, t) in [entry, opp] {
                        out.dirs.entry(key(f, t)).or_insert((f, t));
                    }
                }
            }
        }
        out
    }
}
impl LoopRing {
    /// The new points on edge a-b listed from a (indices 0..n into that edge's points) and their fractions from a.
    pub fn along(&self, fractions: &[f64], a: usize, b: usize) -> (Vec<usize>, Vec<f64>) {
        let mut ids: Vec<_> = (0..fractions.len()).collect();
        let mut fs = fractions.to_vec();
        if self.dirs.get(&key(a, b)).is_some_and(|d| d.0 != a) {
            ids.reverse();
            fs.reverse();
            for x in &mut fs {
                *x = 1. - *x;
            }
        }
        (ids, fs)
    }
    /// World-space segments of the new edges across each split quad: exactly where polysplit puts them.
    pub fn cut_lines(&self, p: &[f32], fractions: &[f64]) -> Vec<([f32; 3], [f32; 3])> {
        let at = |a: usize, b: usize, f: f64| {
            [0, 1, 2].map(|k| (p[3 * a + k] as f64 * (1. - f) + p[3 * b + k] as f64 * f) as f32)
        };
        let mut out = Vec::new();
        for &(entry, opp) in self.quads.values() {
            let (_, fa) = self.along(fractions, entry.0, entry.1);
            let (_, fd) = self.along(fractions, opp.0, opp.1);
            for (x, y) in fa.iter().zip(&fd) {
                out.push((at(entry.0, entry.1, *x), at(opp.0, opp.1, *y)));
            }
        }
        out
    }
}
/// Slide selected edge chains along the unselected face rails. Each face supplies a rail
/// on its side of an oriented chain, so open borders have one available direction.
pub(super) fn slide_selected_edges(g: &Geometry, selected: &BTreeSet<Pair>, factor: f64, even: bool,
    flipped: bool, clamp: bool, correct_uvs: bool) -> Geometry {
    let mut out = g.clone();
    if selected.is_empty() || factor == 0. { return out; }
    let mut adjacency = BTreeMap::<usize, Vec<usize>>::new();
    for &(a,b) in selected { adjacency.entry(a).or_default().push(b); adjacency.entry(b).or_default().push(a); }
    for n in adjacency.values_mut() { n.sort_unstable(); }
    let mut oriented = Vec::new();
    let mut visited = BTreeSet::new();
    while visited.len() < selected.len() {
        let start = adjacency.iter().find(|(v, n)| n.len() == 1 && n.iter().any(|&u| !visited.contains(&key(**v,u))))
            .map(|(v,_)| *v).or_else(|| adjacency.iter().find(|(v,n)| n.iter().any(|&u| !visited.contains(&key(**v,u)))).map(|(v,_)| *v));
        let Some(mut current) = start else { break; };
        loop {
            let Some(next) = adjacency[&current].iter().copied().find(|&u| !visited.contains(&key(current,u))) else { break; };
            visited.insert(key(current,next)); oriented.push((current,next)); current = next;
        }
    }
    let vp = &g.topology.vertex_point;
    let mut rails = BTreeMap::<usize,[Option<usize>;2]>::new();
    for &(a,b) in &oriented {
        for face in 0..g.nprims() {
            if !g.topology.is_closed(face) { continue; }
            let r = range(g,face);
            if r.len() < 3 { continue; }
            let corners: Vec<_> = r.collect();
            for i in 0..corners.len() {
                let x = vp[corners[i]] as usize;
                let y = vp[corners[(i+1)%corners.len()]] as usize;
                let (side, u, v, before, after) = if (x,y)==(a,b) {
                    (0,a,b,vp[corners[(i+corners.len()-1)%corners.len()]] as usize,vp[corners[(i+2)%corners.len()]] as usize)
                } else if (x,y)==(b,a) {
                    (1,a,b,vp[corners[(i+2)%corners.len()]] as usize,vp[corners[(i+corners.len()-1)%corners.len()]] as usize)
                } else { continue; };
                if !selected.contains(&key(u,before)) { rails.entry(u).or_default()[side].get_or_insert(before); }
                if !selected.contains(&key(v,after)) { rails.entry(v).or_default()[side].get_or_insert(after); }
            }
        }
    }
    let side = (factor.is_sign_negative() as usize) ^ (flipped as usize);
    let reference = rails.iter().find_map(|(&v,r)| r[side].map(|n| (v,n)))
        .map(|(v,n)| (0..3).map(|k| (g.p()[3*v+k]-g.p()[3*n+k]).powi(2)).sum::<f32>().sqrt() as f64).unwrap_or(0.);
    let mut targets = BTreeMap::<usize,(usize,f64)>::new();
    for (&v,r) in &rails {
        let Some(n) = r[side] else { continue; };
        let distance=(0..3).map(|k| (g.p()[3*v+k]-g.p()[3*n+k]).powi(2)).sum::<f32>().sqrt() as f64;
        if distance <= 1e-12 { continue; }
        let amount=factor.abs() * if even && reference > 0. { reference/distance } else { 1. };
        targets.insert(v,(n,if clamp { amount.clamp(0.,1.) } else { amount }));
    }
    let mut positions=g.p().to_vec();
    for (&v,&(n,t)) in &targets { for k in 0..3 { positions[3*v+k]=(g.p()[3*v+k] as f64*(1.-t)+g.p()[3*n+k] as f64*t) as f32; } }
    out.set_p(positions);
    for (name,attr) in &g.point {
        if name=="P" || name.starts_with("group_") { continue; }
        if let AttrData::F32(data)=&attr.data {
            let mut data=data.clone();
            for (&v,&(n,t)) in &targets { for k in 0..attr.width { data[v*attr.width+k]=(attr.as_f32().unwrap()[v*attr.width+k] as f64*(1.-t)+attr.as_f32().unwrap()[n*attr.width+k] as f64*t) as f32; } }
            out.point.insert(name.clone(),Arc::new(Attr::f32(attr.width,data)));
        }
    }
    if correct_uvs {
        let (_,next,prev)=corners(g);
        for (name,attr) in &g.vertex {
            if name.starts_with("edge_group_") { continue; }
            if let AttrData::F32(data)=&attr.data {
                let mut changed=data.clone();
                for c in 0..g.nvertices() {
                    let v=vp[c] as usize;
                    let Some(&(n,t))=targets.get(&v) else { continue; };
                    let rail=if vp[next[c]] as usize==n { Some(next[c]) } else if vp[prev[c]] as usize==n { Some(prev[c]) } else { None };
                    if let Some(q)=rail { for k in 0..attr.width { changed[c*attr.width+k]=(data[c*attr.width+k] as f64*(1.-t)+data[q*attr.width+k] as f64*t) as f32; } }
                }
                out.vertex.insert(name.clone(),Arc::new(Attr::f32(attr.width,changed)));
            }
        }
    }
    out
}
/// Geometry used by the viewport preview. The node calls the same kernel after parsing its group.
pub fn edge_slide_preview(g: &Geometry, selected: &[(usize,usize)], factor: f64, even: bool,
    flipped: bool, clamp: bool, correct_uvs: bool) -> Geometry {
    slide_selected_edges(g,&selected.iter().map(|&(a,b)|key(a,b)).collect(),factor,even,flipped,clamp,correct_uvs)
}
/// Cook-backed preview for the modal, including Even and Flip on irregular rings.
pub fn loop_cut_preview(g: &Geometry, seed: Pair, count: usize, factor: f64, even: bool,
    flipped: bool, clamp: bool) -> Result<Vec<([f32;3],[f32;3])>> {
    let value=serde_json::json!({"split_edges":format!("p{}-{}",seed.0,seed.1),"loops":count,
        "factor":factor,"even":even,"flipped":flipped,"clamp":clamp,"outgrp_edges":true});
    let params=Params::resolve(POLYSPLIT.params,value.as_object())?;
    let result=polysplit(&[vec![Arc::new(g.clone())]],&params)?;
    let edges=topo::edges(&result.topology,result.npoints);
    let mask=edges::edge_mask(&result,"splitedges")?;
    let positions=result.p();
    Ok(edges.points.chunks_exact(2).enumerate().filter(|(i,_)|mask[*i]).map(|(_,pair)| {
        let point=|id:i64| [0,1,2].map(|k|positions[3*id as usize+k]);
        (point(pair[0]),point(pair[1]))
    }).collect())
}
pub(super) fn edge_slide(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g=EDGE_SLIDE.one(inputs,0)?;
    polygons(&g,"edge_slide")?;
    let group=p.s("group")?;
    if group.trim().is_empty() { return Ok((*g).clone()); }
    let edges=topo::edges(&g.topology,g.npoints);
    let mask=edges::edge_mask(&g,&group)?;
    let selected: BTreeSet<_>=edges.points.chunks_exact(2).enumerate().filter(|(i,_)|mask[*i])
        .map(|(_,v)|key(v[0] as usize,v[1] as usize)).collect();
    let factor=p.f("factor")?;
    if !(-1. ..=1.).contains(&factor) { return Err(Error::new("factor must be -1..1")); }
    let mut out=slide_selected_edges(&g,&selected,factor,p.b("even")?,p.b("flipped")?,p.b("clamp")?,p.b("correct_uvs")?);
    // Mirror editing is reserved for the editor's symmetry context; without one, geometry is unchanged.
    let _=p.b("mirror_editing")?;
    if p.b("output_group")? { let name=p.s("output_group_name")?; group_name(&name)?;
        let mask=pair_mask(&out,&selected.iter().copied().collect::<Vec<_>>());
        write_group(&mut out,"edge",&name,&mask)?;
    }
    Ok(out)
}
