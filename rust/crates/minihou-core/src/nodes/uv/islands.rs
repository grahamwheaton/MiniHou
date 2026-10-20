use crate::geometry::Geometry;

fn uf_find(parent: &mut [usize], mut i: usize) -> usize {
    let mut root = i;
    while parent[root] != root {
        root = parent[root];
    }
    while parent[i] != root {
        let next = parent[i];
        parent[i] = root;
        i = next;
    }
    root
}

pub(crate) fn uf_union(parent: &mut [usize], i: usize, j: usize) {
    let ri = uf_find(parent, i);
    let rj = uf_find(parent, j);
    if ri != rj {
        if ri < rj {
            parent[rj] = ri;
        } else {
            parent[ri] = rj;
        }
    }
}

pub(crate) fn relabel_components(parent: &mut [usize], n: usize) -> Vec<usize> {
    let mut root_to_label = vec![usize::MAX; n];
    let mut num_labels = 0;
    for i in 0..n {
        let r = uf_find(parent, i);
        if root_to_label[r] == usize::MAX {
            root_to_label[r] = num_labels;
            num_labels += 1;
        }
    }
    (0..n).map(|i| root_to_label[uf_find(parent, i)]).collect()
}

pub fn uv_islands(geo: &Geometry, uv: &[[f64; 2]]) -> Vec<usize> {
    let nprims = geo.nprims();
    if nprims == 0 {
        return Vec::new();
    }
    let topo = &geo.topology;
    let nv = geo.nvertices();
    let mut pv = vec![0usize; nv];
    let mut nxt = vec![0usize; nv];
    let mut a = vec![0usize; nv];
    let mut b = vec![0usize; nv];

    for p in 0..nprims {
        let start = topo.prim_offsets[p] as usize;
        let end = topo.prim_offsets[p + 1] as usize;
        for v in start..end {
            pv[v] = p;
            let nx = if v + 1 < end { v + 1 } else { start };
            nxt[v] = nx;
            a[v] = topo.vertex_point[v] as usize;
            b[v] = topo.vertex_point[nx] as usize;
        }
    }

    let mut order: Vec<usize> = (0..nv).collect();
    order.sort_by_key(|&v| (a[v].min(b[v]), a[v].max(b[v])));

    let mut parent: Vec<usize> = (0..nprims).collect();

    for i in 0..order.len().saturating_sub(1) {
        let h1 = order[i];
        let h2 = order[i + 1];
        if (a[h1].min(b[h1]), a[h1].max(b[h1])) == (a[h2].min(b[h2]), a[h2].max(b[h2])) {
            let first = if a[h2] == a[h1] { h2 } else { nxt[h2] };
            let second = if a[h2] == a[h1] { nxt[h2] } else { h2 };
            let ua1 = uv[h1];
            let ub1 = uv[nxt[h1]];
            let u_first = uv[first];
            let u_second = uv[second];
            let close = (ua1[0] - u_first[0]).abs() < 1e-6
                && (ua1[1] - u_first[1]).abs() < 1e-6
                && (ub1[0] - u_second[0]).abs() < 1e-6
                && (ub1[1] - u_second[1]).abs() < 1e-6;
            if close {
                uf_union(&mut parent, pv[h1], pv[h2]);
            }
        }
    }

    relabel_components(&mut parent, nprims)
}
