use super::*;

// ---------------------------------------------------------------------------
// BRIDGE
// ---------------------------------------------------------------------------

struct DisjointSet {
    parent: Vec<usize>,
}

impl DisjointSet {
    fn new(n: usize) -> Self {
        Self { parent: (0..n).collect() }
    }
    fn find(&mut self, mut x: usize) -> usize {
        while self.parent[x] != x {
            self.parent[x] = self.parent[self.parent[x]];
            x = self.parent[x];
        }
        x
    }
    fn union(&mut self, a: usize, b: usize) {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra != rb {
            self.parent[rb] = ra;
        }
    }
}

fn boundary_loops(geo: &Geometry) -> Result<Vec<Vec<usize>>> {
    let e = topo::edges(&geo.topology, geo.npoints);
    let vp = &geo.topology.vertex_point;
    let mut succ = BTreeMap::new();
    let mut starts = Vec::new();

    for p in 0..geo.nprims() {
        let start = geo.topology.prim_offsets[p] as usize;
        let end = geo.topology.prim_offsets[p + 1] as usize;
        let n = end - start;
        for i in 0..n {
            let v = start + i;
            let nx = start + (i + 1) % n;
            let edge_idx = e.of_vertex[v];
            if edge_idx >= 0 && e.count[edge_idx as usize] == 1 {
                let a = vp[v] as usize;
                let b = vp[nx] as usize;
                if succ.insert(b, a).is_some() {
                    return Err(Error::new("the boundary touches itself (a point with several open edges); cannot fill it"));
                }
                starts.push(b);
            }
        }
    }

    let mut seen = BTreeSet::new();
    let mut loops = Vec::new();
    for start in starts {
        if seen.contains(&start) {
            continue;
        }
        let mut loop_pts = Vec::new();
        let mut at = start;
        while !seen.contains(&at) {
            seen.insert(at);
            loop_pts.push(at);
            if let Some(&nxt) = succ.get(&at) {
                at = nxt;
            } else {
                break;
            }
        }
        if at == start && loop_pts.len() >= 3 {
            loops.push(loop_pts);
        }
    }
    Ok(loops)
}

fn bridge_pair(la: &[usize], lb: &[usize], p_slice: &[f32], align: bool, twist: i64) -> Vec<Vec<i32>> {
    let mut lb_rev = lb.to_vec();
    lb_rev.reverse();
    let na = la.len();
    let nb = lb_rev.len();
    let mut s = 0usize;

    if align && na == nb {
        let mut best: Option<(f64, usize)> = None;
        for k in 0..nb {
            let mut d = 0.0;
            for i in 0..na {
                let pa = &p_slice[la[i] * 3..la[i] * 3 + 3];
                let pb = &p_slice[lb_rev[(i + k) % nb] * 3..lb_rev[(i + k) % nb] * 3 + 3];
                for c in 0..3 {
                    d += (pa[c] as f64 - pb[c] as f64).powi(2);
                }
            }
            if best.is_none() || d < best.unwrap().0 - 1e-12 {
                best = Some((d, k));
            }
        }
        s = best.unwrap().1;
    }

    s = (s as i64 + twist).rem_euclid(nb as i64) as usize;
    lb_rev.rotate_left(s);

    let mut faces = Vec::new();
    if na == nb {
        for i in 0..na {
            let j = (i + 1) % na;
            faces.push(vec![la[i] as i32, la[j] as i32, lb_rev[j] as i32, lb_rev[i] as i32]);
        }
        return faces;
    }

    let mut i = 0usize;
    let mut j = 0usize;
    while i < na || j < nb {
        let a0 = la[i % na];
        let a1 = la[(i + 1) % na];
        let b0 = lb_rev[j % nb];
        let b1 = lb_rev[(j + 1) % nb];

        let p_a1 = [p_slice[a1 * 3] as f64, p_slice[a1 * 3 + 1] as f64, p_slice[a1 * 3 + 2] as f64];
        let p_b0 = [p_slice[b0 * 3] as f64, p_slice[b0 * 3 + 1] as f64, p_slice[b0 * 3 + 2] as f64];
        let p_a0 = [p_slice[a0 * 3] as f64, p_slice[a0 * 3 + 1] as f64, p_slice[a0 * 3 + 2] as f64];
        let p_b1 = [p_slice[b1 * 3] as f64, p_slice[b1 * 3 + 1] as f64, p_slice[b1 * 3 + 2] as f64];

        let adv_a = i < na && (j >= nb || len3(sub3(p_a1, p_b0)) <= len3(sub3(p_a0, p_b1)));
        if adv_a {
            faces.push(vec![a0 as i32, a1 as i32, b0 as i32]);
            i += 1;
        } else {
            faces.push(vec![a0 as i32, b1 as i32, b0 as i32]);
            j += 1;
        }
    }
    faces
}

pub(super) fn bridge(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = BRIDGE.one(inputs, 0)?;
    if geo.nprims() == 0 {
        return Err(Error::new("bridge needs polygons"));
    }
    let loops = boundary_loops(&geo)?;
    if loops.len() < 2 {
        return Err(Error::new(format!("bridge needs at least two open loops, the input has {}", loops.len())));
    }

    let align = p.b("align")?;
    let twist = p.i("twist")? as i64;
    let pairing = p.e("pairing", &["mutual_nearest", "in_order"])?;

    let p_slice = geo.p();
    let mut cent = Vec::with_capacity(loops.len());
    for l in &loops {
        let mut c = [0.0; 3];
        for &idx in l {
            for k in 0..3 {
                c[k] += p_slice[idx * 3 + k] as f64;
            }
        }
        for k in 0..3 {
            c[k] /= l.len() as f64;
        }
        cent.push(c);
    }

    let mut pairs = Vec::new();
    if pairing == "in_order" {
        for i in (0..loops.len().saturating_sub(1)).step_by(2) {
            pairs.push((i, i + 1));
        }
    } else {
        let e = topo::edges(&geo.topology, geo.npoints);
        let mut uf = DisjointSet::new(geo.npoints);
        for edge_idx in 0..e.len() {
            uf.union(e.points[edge_idx * 2] as usize, e.points[edge_idx * 2 + 1] as usize);
        }
        let piece: Vec<usize> = loops.iter().map(|l| uf.find(l[0])).collect();
        let num_pieces = piece.iter().copied().collect::<BTreeSet<_>>().len();

        let mut dist = vec![vec![f64::INFINITY; loops.len()]; loops.len()];
        for i in 0..loops.len() {
            for j in 0..loops.len() {
                if i == j {
                    continue;
                }
                if num_pieces > 1 && piece[i] == piece[j] {
                    continue;
                }
                dist[i][j] = len3(sub3(cent[i], cent[j]));
            }
        }

        let mut nearest = vec![0usize; loops.len()];
        for i in 0..loops.len() {
            let mut min_d = f64::INFINITY;
            let mut min_j = 0usize;
            for j in 0..loops.len() {
                if dist[i][j] < min_d {
                    min_d = dist[i][j];
                    min_j = j;
                }
            }
            nearest[i] = min_j;
        }

        for i in 0..loops.len() {
            let j = nearest[i];
            if i < j && dist[i][j].is_finite() && nearest[j] == i {
                pairs.push((i, j));
            }
        }
        if pairs.is_empty() {
            return Err(Error::new("bridge found no two loops that are each other's nearest; wire pieces with matching ends"));
        }
    }

    let mut new_faces = Vec::new();
    for (a, b) in pairs {
        new_faces.extend(bridge_pair(&loops[a], &loops[b], p_slice, align, twist));
    }

    let mut vp = geo.topology.vertex_point.clone();
    let mut offsets = geo.topology.prim_offsets.clone();
    let old_nprims = geo.nprims();
    let old_nverts = geo.nvertices();

    for face in &new_faces {
        vp.extend(face);
        offsets.push(vp.len() as i32);
    }

    let mut out = Geometry::from_arrays(geo.p().to_vec(), vp, offsets, None)?;
    out.point = geo.point.clone();
    out.detail = geo.detail.clone();

    // Pad vertex attributes with zero
    let new_verts_count = out.nvertices() - old_nverts;
    for (name, attr) in &geo.vertex {
        let mut padded = (**attr).clone();
        match &mut padded.data {
            AttrData::F32(v) => v.resize(v.len() + new_verts_count * attr.width, 0.0),
            AttrData::I32(v) => v.resize(v.len() + new_verts_count * attr.width, 0),
        }
        out.vertex.insert(name.clone(), Arc::new(padded));
    }

    // Pad prim attributes with zero
    let new_prims_count = out.nprims() - old_nprims;
    for (name, attr) in &geo.prim {
        let mut padded = (**attr).clone();
        match &mut padded.data {
            AttrData::F32(v) => v.resize(v.len() + new_prims_count * attr.width, 0.0),
            AttrData::I32(v) => v.resize(v.len() + new_prims_count * attr.width, 0),
        }
        out.prim.insert(name.clone(), Arc::new(padded));
    }

    if p.b("outgrp_bridge")? {
        let grp_name = p.s("outgrp_bridge_name")?;
        if !grp_name.is_empty() {
            let mut vals = vec![0i32; out.nprims()];
            for v in &mut vals[old_nprims..] {
                *v = 1;
            }
            out.prim.insert(grp_name, Arc::new(Attr::i32(1, vals)));
        }
    }

    Ok(out)
}

