use super::*;

pub(super) struct BoundaryLoop {
    pub(super) pts: Vec<usize>,
    pub(super) corners: Vec<usize>,
    pub(super) is_open: bool,
}

pub(super) fn get_boundary_mask(g: &Geometry, pattern: &str) -> Result<Vec<bool>> {
    let e = topo::edges(&g.topology, g.npoints);
    let mut mask = vec![false; e.len()];
    let pattern = pattern.trim();
    if pattern.is_empty() {
        for i in 0..e.len() {
            if e.count[i] == 1 {
                mask[i] = true;
            }
        }
        return Ok(mask);
    }

    if let Ok(em) = edge_tools::edge_mask(g, pattern) {
        let mut any = false;
        for i in 0..e.len() {
            if e.count[i] == 1 && em[i] {
                mask[i] = true;
                any = true;
            }
        }
        if any {
            return Ok(mask);
        }
    }

    if let Ok(pm) = groups::point_mask(g, pattern) {
        let mut any = false;
        for i in 0..e.len() {
            if e.count[i] == 1 {
                let p0 = e.points[2 * i] as usize;
                let p1 = e.points[2 * i + 1] as usize;
                if pm[p0] && pm[p1] {
                    mask[i] = true;
                    any = true;
                }
            }
        }
        if any {
            return Ok(mask);
        }
    }

    let prim_key = format!("group_{pattern}");
    if let Some(prim_attr) = g.prim.get(&prim_key) {
        let (pr, _, _) = model::corners(g);
        let mut any = false;
        for v in 0..g.nvertices() {
            let prim_idx = pr[v];
            let in_group = match &prim_attr.data {
                AttrData::I32(d) => d[prim_idx] != 0,
                AttrData::F32(d) => d[prim_idx] != 0.0,
            };
            if in_group {
                let edge_id = e.of_vertex[v];
                if edge_id >= 0 && e.count[edge_id as usize] == 1 {
                    mask[edge_id as usize] = true;
                    any = true;
                }
            }
        }
        if any {
            return Ok(mask);
        }
    }

    Err(Error::new(format!("cannot resolve boundary group {pattern:?}")))
}

pub(super) fn extract_loops(g: &Geometry, boundary_mask: &[bool], auto_complete: bool) -> Result<Vec<BoundaryLoop>> {
    let e = topo::edges(&g.topology, g.npoints);
    let (_, nx, _) = model::corners(g);
    let vp = &g.topology.vertex_point;

    let mut succ: BTreeMap<usize, (usize, usize)> = BTreeMap::new();
    let mut indeg: BTreeMap<usize, usize> = BTreeMap::new();
    let mut all_succ: BTreeMap<usize, (usize, usize)> = BTreeMap::new();
    let mut starts = Vec::new();

    for v in 0..g.nvertices() {
        let edge_id = e.of_vertex[v];
        if edge_id >= 0 && e.count[edge_id as usize] == 1 {
            let a = vp[v] as usize;
            let b = vp[nx[v]] as usize;
            // A boundary walk has no unambiguous successor at a junction. Do not
            // silently replace an earlier edge and lose a hole.
            if all_succ.insert(b, (a, v)).is_some() {
                return Err(Error::new(format!(
                    "boundary touches itself at point {b}; cannot choose a fill loop"
                )));
            }

            if boundary_mask[edge_id as usize] {
                succ.insert(b, (a, v));
                *indeg.entry(a).or_insert(0) += 1;
                indeg.entry(b).or_insert(0);
                starts.push(b);
            }
        }
    }

    let mut loops = Vec::new();
    let mut visited_edges: BTreeSet<(usize, usize)> = BTreeSet::new();

    let open_starts: Vec<usize> = succ
        .keys()
        .copied()
        .filter(|&k| indeg.get(&k).copied().unwrap_or(0) == 0)
        .collect();

    for start in open_starts {
        let mut cur = start;
        let mut pts = Vec::new();
        let mut corners = Vec::new();
        let mut chain_visited = BTreeSet::new();

        while let Some(&(next, corner)) = succ.get(&cur) {
            if !chain_visited.insert((cur, next)) {
                break;
            }
            visited_edges.insert((cur, next));
            pts.push(cur);
            corners.push(corner);
            cur = next;
        }

        if auto_complete {
            let chain_end = cur;
            let mut completed = false;
            let mut walk = chain_end;
            let mut auto_pts = Vec::new();
            let mut auto_corners = Vec::new();
            let mut auto_visited = BTreeSet::new();

            while let Some(&(next, corner)) = all_succ.get(&walk) {
                if !auto_visited.insert((walk, next)) {
                    break;
                }
                auto_pts.push(walk);
                auto_corners.push(corner);
                walk = next;
                if walk == start {
                    pts.extend(auto_pts);
                    corners.extend(auto_corners);
                    completed = true;
                    break;
                }
            }

            if completed {
                if pts.len() >= 3 {
                    loops.push(BoundaryLoop {
                        pts,
                        corners,
                        is_open: true,
                    });
                }
            } else {
                if pts.len() >= 2 {
                    pts.push(chain_end);
                    corners.push(corners.last().copied().unwrap_or(0));
                    loops.push(BoundaryLoop {
                        pts,
                        corners,
                        is_open: true,
                    });
                }
            }
        }
    }

    for &start in starts.iter().chain(succ.keys()) {
        if visited_edges.contains(&(start, succ[&start].0)) {
            continue;
        }
        let mut cur = start;
        let mut pts = Vec::new();
        let mut corners = Vec::new();
        let mut cycle = false;

        loop {
            let Some(&(next, corner)) = succ.get(&cur) else {
                break;
            };
            if visited_edges.contains(&(cur, next)) {
                break;
            }
            visited_edges.insert((cur, next));
            pts.push(cur);
            corners.push(corner);
            cur = next;
            if cur == start {
                cycle = true;
                break;
            }
        }

        if cycle && pts.len() >= 3 {
            loops.push(BoundaryLoop {
                pts,
                corners,
                is_open: false,
            });
        }
    }

    Ok(loops)
}
