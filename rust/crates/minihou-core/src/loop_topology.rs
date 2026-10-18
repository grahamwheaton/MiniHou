//! Shared topology helpers for extracting ordered vertex paths and closed loops from edge/point selections.

use std::collections::{BTreeMap, BTreeSet};
use crate::{
    error::{Error, Result},
    geometry::{AttrData, Geometry},
};

/// An ordered sequence of point indices representing an open path or closed loop.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoopPath {
    /// Ordered point indices along the loop or path.
    pub points: Vec<usize>,
    /// True if the loop is closed (first connects to last), false if open.
    pub is_closed: bool,
}

/// Extract ordered paths and closed loops from an explicit list of undirected edge index pairs.
/// Errors if any vertex has valence > 2 (branching).
pub fn extract_loops_from_edge_pairs(pairs: &[(usize, usize)]) -> Result<Vec<LoopPath>> {
    let mut edges = BTreeSet::new();
    for &(u, v) in pairs {
        if u == v {
            continue; // Skip zero-length degenerate edges
        }
        let (lo, hi) = if u < v { (u, v) } else { (v, u) };
        edges.insert((lo, hi));
    }

    if edges.is_empty() {
        return Ok(Vec::new());
    }

    let mut adj: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for &(u, v) in &edges {
        adj.entry(u).or_default().push(v);
        adj.entry(v).or_default().push(u);
    }

    // Error on branching (valence over 2)
    for (&v, neighbors) in &adj {
        if neighbors.len() > 2 {
            return Err(Error::new(format!(
                "branching vertex {v} has valence {} (valence over 2 is not supported in loop tools)",
                neighbors.len()
            )));
        }
    }

    let mut visited = BTreeSet::new();
    let mut result = Vec::new();

    // 1. Extract open paths (endpoints have degree 1)
    let endpoints: Vec<usize> = adj
        .iter()
        .filter_map(|(&v, neighbors)| if neighbors.len() == 1 { Some(v) } else { None })
        .collect();

    for start in endpoints {
        if visited.contains(&start) {
            continue;
        }

        let mut path = vec![start];
        visited.insert(start);
        let mut curr = start;
        let mut prev = None;

        loop {
            let next_candidates: Vec<usize> = adj[&curr]
                .iter()
                .copied()
                .filter(|&n| Some(n) != prev)
                .collect();

            if next_candidates.is_empty() {
                break;
            }

            let next = next_candidates[0];
            path.push(next);
            visited.insert(next);
            prev = Some(curr);
            curr = next;

            if adj[&curr].len() == 1 {
                // Reached other endpoint
                break;
            }
        }

        result.push(LoopPath {
            points: path,
            is_closed: false,
        });
    }

    // 2. Extract closed cycles (all remaining unvisited vertices have degree 2)
    for &v in adj.keys() {
        if visited.contains(&v) {
            continue;
        }

        // Find all vertices in this component to pick the minimal index as canonical start
        let mut component = Vec::new();
        let mut queue = vec![v];
        let mut comp_seen = BTreeSet::new();
        comp_seen.insert(v);

        while let Some(node) = queue.pop() {
            component.push(node);
            for &n in &adj[&node] {
                if comp_seen.insert(n) {
                    queue.push(n);
                }
            }
        }

        let start = *component.iter().min().unwrap();
        let neighbors = &adj[&start];
        let next_start = neighbors[0].min(neighbors[1]);

        let mut path = vec![start];
        visited.insert(start);
        let mut curr = next_start;
        let mut prev = start;

        while curr != start {
            path.push(curr);
            visited.insert(curr);
            let next = if adj[&curr][0] == prev {
                adj[&curr][1]
            } else {
                adj[&curr][0]
            };
            prev = curr;
            curr = next;
        }

        result.push(LoopPath {
            points: path,
            is_closed: true,
        });
    }

    Ok(result)
}

/// Extract loops from an edge selection mask (one bool per edge in topo::edges).
pub fn extract_loops_from_edge_mask(geo: &Geometry, edge_mask: &[bool]) -> Result<Vec<LoopPath>> {
    let e = crate::topo::edges(&geo.topology, geo.npoints);
    let mut pairs = Vec::new();
    for (i, &selected) in edge_mask.iter().enumerate() {
        if selected && i < e.len() {
            let u = e.points[2 * i] as usize;
            let v = e.points[2 * i + 1] as usize;
            pairs.push((u, v));
        }
    }
    extract_loops_from_edge_pairs(&pairs)
}

/// Extract loops from a point selection mask (one bool per point).
/// An edge is selected if both of its endpoints are in the point selection.
pub fn extract_loops_from_point_mask(geo: &Geometry, point_mask: &[bool]) -> Result<Vec<LoopPath>> {
    let e = crate::topo::edges(&geo.topology, geo.npoints);
    let mut pairs = Vec::new();
    for i in 0..e.len() {
        let u = e.points[2 * i] as usize;
        let v = e.points[2 * i + 1] as usize;
        if u < point_mask.len() && v < point_mask.len() && point_mask[u] && point_mask[v] {
            pairs.push((u, v));
        }
    }
    extract_loops_from_edge_pairs(&pairs)
}

fn is_attr_nonzero(_geo: &Geometry, attr_name: &str, table: &crate::geometry::AttrTable, index: usize) -> bool {
    let Some(attr) = table.get(attr_name) else {
        return false;
    };
    match &attr.data {
        AttrData::F32(v) => {
            let start = index * attr.width;
            start < v.len() && v[start] != 0.0
        }
        AttrData::I32(v) => {
            let start = index * attr.width;
            start < v.len() && v[start] != 0
        }
    }
}

/// Extract ordered loops and paths from geometry based on a group pattern or name.
/// Empty group selects all edges of the geometry.
pub fn extract_loops(geo: &Geometry, group: &str) -> Result<Vec<LoopPath>> {
    let trimmed = group.trim();
    let e = crate::topo::edges(&geo.topology, geo.npoints);

    if trimmed.is_empty() {
        let mask = vec![true; e.len()];
        return extract_loops_from_edge_mask(geo, &mask);
    }

    // 1. Check if it matches an edge group in geo.vertex
    let edge_group_name = if geo.vertex.contains_key(&format!("edge_group_{trimmed}")) {
        Some(format!("edge_group_{trimmed}"))
    } else if geo.vertex.contains_key(trimmed) {
        Some(trimmed.to_string())
    } else {
        None
    };

    if let Some(ref name) = edge_group_name {
        let mut mask = vec![false; e.len()];
        for v in 0..geo.nvertices() {
            if is_attr_nonzero(geo, name, &geo.vertex, v) {
                let edge_id = e.of_vertex[v];
                if edge_id >= 0 && (edge_id as usize) < e.len() {
                    mask[edge_id as usize] = true;
                }
            }
        }
        return extract_loops_from_edge_mask(geo, &mask);
    }

    // 2. Check if it matches a point group in geo.point
    let point_group_name = if geo.point.contains_key(&format!("group_{trimmed}")) {
        Some(format!("group_{trimmed}"))
    } else if geo.point.contains_key(trimmed) {
        Some(trimmed.to_string())
    } else {
        None
    };

    if let Some(ref name) = point_group_name {
        let mask: Vec<bool> = (0..geo.npoints)
            .map(|i| is_attr_nonzero(geo, name, &geo.point, i))
            .collect();
        return extract_loops_from_point_mask(geo, &mask);
    }

    // 3. Check if it matches a primitive group in geo.prim
    let prim_group_name = if geo.prim.contains_key(&format!("group_{trimmed}")) {
        Some(format!("group_{trimmed}"))
    } else if geo.prim.contains_key(trimmed) {
        Some(trimmed.to_string())
    } else {
        None
    };

    if let Some(ref name) = prim_group_name {
        let mut pt_mask = vec![false; geo.npoints];
        for prim in 0..geo.nprims() {
            if is_attr_nonzero(geo, name, &geo.prim, prim) {
                let start = geo.topology.prim_offsets[prim] as usize;
                let end = geo.topology.prim_offsets[prim + 1] as usize;
                for &pt in &geo.topology.vertex_point[start..end] {
                    if (pt as usize) < geo.npoints {
                        pt_mask[pt as usize] = true;
                    }
                }
            }
        }
        return extract_loops_from_point_mask(geo, &pt_mask);
    }

    // 4. Check for edge pair pattern (e.g. "p0-1" or "0-1")
    if let Some(pair) = trimmed
        .strip_prefix('p')
        .unwrap_or(trimmed)
        .split_once('-')
        .filter(|(a, b)| a.chars().all(|c| c.is_ascii_digit()) && b.chars().all(|c| c.is_ascii_digit()))
    {
        if let (Ok(u), Ok(v)) = (pair.0.parse::<usize>(), pair.1.parse::<usize>()) {
            return extract_loops_from_edge_pairs(&[(u, v)]);
        }
    }

    // 5. Check for numeric point indices/range, e.g. "0 1 2 3" or "0-3"
    let tokens: Vec<&str> = trimmed.split(|c: char| c == ',' || c.is_whitespace()).filter(|s| !s.is_empty()).collect();
    let mut pt_mask = vec![false; geo.npoints];
    let mut any_num = false;
    for token in tokens {
        if let Some((start_s, end_s)) = token.split_once('-') {
            if let (Ok(lo), Ok(hi)) = (start_s.parse::<usize>(), end_s.parse::<usize>()) {
                any_num = true;
                for i in lo..=hi.min(geo.npoints.saturating_sub(1)) {
                    pt_mask[i] = true;
                }
            }
        } else if let Ok(idx) = token.parse::<usize>() {
            any_num = true;
            if idx < geo.npoints {
                pt_mask[idx] = true;
            }
        }
    }
    if any_num {
        return extract_loops_from_point_mask(geo, &pt_mask);
    }

    Err(Error::new(format!("no group or matching selection {trimmed:?}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use crate::geometry::Attr;

    #[test]
    fn test_extract_open_path() {
        let pairs = vec![(0, 1), (1, 2), (2, 3)];
        let loops = extract_loops_from_edge_pairs(&pairs).unwrap();
        assert_eq!(loops.len(), 1);
        assert!(!loops[0].is_closed);
        assert_eq!(loops[0].points, vec![0, 1, 2, 3]);
    }

    #[test]
    fn test_extract_closed_loop() {
        let pairs = vec![(0, 1), (1, 2), (2, 3), (3, 0)];
        let loops = extract_loops_from_edge_pairs(&pairs).unwrap();
        assert_eq!(loops.len(), 1);
        assert!(loops[0].is_closed);
        assert_eq!(loops[0].points, vec![0, 1, 2, 3]);
    }

    #[test]
    fn test_branching_errors() {
        // T-junction at vertex 1
        let pairs = vec![(0, 1), (1, 2), (1, 3)];
        let err = extract_loops_from_edge_pairs(&pairs).unwrap_err();
        assert!(err.to_string().contains("branching vertex 1 has valence 3"));
    }

    #[test]
    fn test_extract_from_geometry_point_group() {
        let pts = vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 2.0, 0.0, 0.0];
        let vp = vec![0, 1, 2];
        let offs = vec![0, 3];
        let mut geo = Geometry::from_arrays(pts, vp, offs, Some(vec![false])).unwrap();
        geo.point.insert("group_path".into(), Arc::new(Attr::i32(1, vec![1, 1, 1])));

        let loops = extract_loops(&geo, "path").unwrap();
        assert_eq!(loops.len(), 1);
        assert!(!loops[0].is_closed);
        assert_eq!(loops[0].points, vec![0, 1, 2]);
    }

    #[test]
    fn test_extract_from_geometry_edge_group() {
        let pts = vec![
            0.0, 0.0, 0.0,
            1.0, 0.0, 0.0,
            1.0, 1.0, 0.0,
            0.0, 1.0, 0.0,
        ];
        let vp = vec![0, 1, 2, 3];
        let offs = vec![0, 4];
        let mut geo = Geometry::from_arrays(pts, vp, offs, None).unwrap();
        // Set edge attribute on vertex for edges 0 and 1
        // In quad 0,1,2,3: vertex 0 is edge (0,1), vertex 1 is edge (1,2)
        geo.vertex.insert("edge_group_my_edges".into(), Arc::new(Attr::i32(1, vec![1, 1, 0, 0])));

        let loops = extract_loops(&geo, "my_edges").unwrap();
        assert_eq!(loops.len(), 1);
        assert_eq!(loops[0].points.len(), 3);
        assert!(!loops[0].is_closed);
    }
}
