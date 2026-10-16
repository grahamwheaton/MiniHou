use eframe::egui::{pos2, Pos2, Rect};
use minihou_core::Graph;
use std::collections::{HashMap, HashSet, VecDeque};

pub(super) fn layout(graph: &Graph, pos_lookup: impl Fn(&str) -> Option<Pos2>) -> Vec<(String, Pos2)> {
    layout_ordered(graph, &pos_lookup, &pos_lookup)
}

/// Source columns survive changes in stream length. Ordering positions are hints only;
/// authored positions are reserved separately so ordinary additions never move existing cards.
pub(super) fn layout_ordered(
    graph: &Graph,
    pos_lookup: impl Fn(&str) -> Option<Pos2>,
    source_lookup: impl Fn(&str) -> Option<Pos2>,
) -> Vec<(String, Pos2)> {
    let ids: HashMap<&str, usize> = graph.nodes.iter().enumerate().map(|(i, (id, _))| (id.as_str(), i)).collect();
    let mut children = vec![Vec::new(); graph.nodes.len()];
    let mut incoming = vec![0usize; graph.nodes.len()];
    let mut parents = vec![Vec::new(); graph.nodes.len()];
    for (target, (_, def)) in graph.nodes.iter().enumerate() {
        for (_, refs) in &def.inputs {
            for source in refs {
                if let Some(&source) = ids.get(source.as_str()) {
                    children[source].push(target);
                    incoming[target] += 1;
                    parents[target].push(source);
                }
            }
        }
    }
    let mut roots: Vec<_> = incoming.iter().enumerate().filter(|(_, count)| **count == 0).map(|(i, _)| i).collect();
    roots.sort_by(|&a, &b| {
        let x = |i: usize| source_lookup(&graph.nodes[i].0).map_or(0.0, |at| at.x);
        x(a).total_cmp(&x(b)).then(a.cmp(&b))
    });
    let mut queue: VecDeque<_> = roots.iter().copied().collect();
    let mut depth = vec![0usize; graph.nodes.len()];
    let mut x = vec![96.0; graph.nodes.len()];
    // Three card widths rounded up to an even number of grid cells: >= two
    // widths of clear gap, and the midpoint of two adjacent columns is on-grid.
    let pitch = (super::CARD.x * 3.0 / 64.0).ceil() * 64.0;
    for (column, &root) in roots.iter().enumerate() { x[root] += column as f32 * pitch; }
    while let Some(source) = queue.pop_front() {
        for &target in &children[source] {
            depth[target] = depth[target].max(depth[source] + 1);
            incoming[target] -= 1;
            if incoming[target] == 0 {
                let inputs = &parents[target];
                let left = inputs.iter().map(|&i| x[i]).fold(f32::INFINITY, f32::min);
                let right = inputs.iter().map(|&i| x[i]).fold(f32::NEG_INFINITY, f32::max);
                x[target] = ((left + right) * 0.5 / 32.0).round() * 32.0;
                queue.push_back(target);
            }
        }
    }
    // Malformed cycles get separate fallback columns; traversal cannot loop forever.
    let mut fallback = roots.len();
    for i in 0..incoming.len() {
        if incoming[i] != 0 { x[i] = 96.0 + fallback as f32 * pitch; fallback += 1; }
    }
    let explicit: HashMap<_, _> = graph.nodes.iter().filter_map(|(id, _)| pos_lookup(id).map(|at| (id.as_str(), at))).collect();
    let mut occupied: Vec<_> = explicit.values().copied().collect();
    graph.nodes.iter().enumerate().map(|(i, (id, _))| {
        let automatic = pos2(x[i], 32.0 + depth[i] as f32 * 128.0);
        let at = explicit.get(id.as_str()).copied().unwrap_or_else(|| {
            let at = free_position(automatic, &occupied);
            occupied.push(at);
            at
        });
        (id.clone(), at)
    }).collect()
}

/// Reserve authored/manual cards first; new cards search grid rows without moving their peers.
pub(super) fn free_position(preferred: Pos2, occupied: &[Pos2]) -> Pos2 {
    let mut at = super::snap(preferred, 32.0, true, false);
    while occupied.iter().any(|peer| Rect::from_min_size(*peer, super::CARD).expand(8.0)
        .intersects(Rect::from_min_size(at, super::CARD).expand(8.0))) {
        at.y += 96.0;
    }
    at
}

/// Open one row in a stream. Shared-input joins stay put unless a shifted card
/// collides with them; collisions propagate downwards, including across columns.
pub(super) fn insert_position(
    graph: &Graph, positions: &[(String, Pos2)], anchor: &str, new_id: &str, before: bool,
) -> Vec<(String, Pos2)> {
    let Some(&(_, origin)) = positions.iter().find(|(id, _)| id == anchor) else { return positions.to_vec() };
    let parents: HashMap<_, Vec<&str>> = graph.nodes.iter().map(|(id, def)|
        (id.as_str(), def.inputs.iter().flat_map(|(_, refs)| refs.iter().map(String::as_str)).collect())).collect();
    let lookup: HashMap<_, _> = positions.iter().map(|(id, at)| (id.as_str(), *at)).collect();
    // Use the local stream spacing (including manually spaced streams), rounded
    // to grid cells. The automatic layout's row pitch is the fallback.
    let pitch = graph.nodes.iter().filter_map(|(id, _)| {
        let at = *lookup.get(id.as_str())?;
        let adjacent = if before { parents[anchor].contains(&id.as_str()) }
            else { parents[id.as_str()].contains(&anchor) };
        let dy = if before { origin.y - at.y } else { at.y - origin.y };
        (adjacent && (at.x - origin.x).abs() < 16.0 && dy >= 96.0).then_some(dy)
    }).min_by(f32::total_cmp).map_or(128.0, |dy| (dy / 32.0).round() * 32.0);
    let slot = super::snap(if before { origin } else { origin + eframe::egui::vec2(0.0, pitch) }, 32.0, true, false);
    let mut stream = HashSet::from([anchor]);
    loop {
        let mut changed = false;
        for (id, inputs) in &parents {
            if !inputs.is_empty() && inputs.iter().all(|source| stream.contains(source)) {
                changed |= stream.insert(*id);
            }
        }
        if !changed { break; }
    }
    let mut result = positions.to_vec();
    let mut shifted = HashSet::new();
    for (i, (id, at)) in result.iter_mut().enumerate() {
        if stream.contains(id.as_str()) && (before || id != anchor) && at.y >= slot.y {
            *at = super::snap(*at + eframe::egui::vec2(0.0, pitch), 32.0, true, false);
            shifted.insert(i);
        }
    }
    result.push((new_id.to_owned(), slot));
    shifted.insert(result.len() - 1);
    // Visit from top to bottom. A blocker is moved below the card which pushed
    // it, so propagation terminates even for malformed graph cycles.
    let mut order: Vec<_> = (0..result.len()).collect();
    order.sort_by(|&a, &b| result[a].1.y.total_cmp(&result[b].1.y)
        .then(shifted.contains(&b).cmp(&shifted.contains(&a))).then(b.cmp(&a)));
    let mut settled: Vec<usize> = Vec::new();
    for i in order {
        loop {
            let blockers: Vec<_> = settled.iter().copied().filter(|&j|
                (shifted.contains(&i) || shifted.contains(&j)) && overlaps(result[i].1, result[j].1)).collect();
            if blockers.is_empty() { break; }
            result[i].1.y = blockers.iter().map(|&j| result[j].1.y + pitch).fold(result[i].1.y, f32::max);
            result[i].1 = super::snap(result[i].1, 32.0, true, false);
            shifted.insert(i);
        }
        settled.push(i);
    }
    result
}

fn overlaps(a: Pos2, b: Pos2) -> bool {
    Rect::from_min_size(a, super::CARD).expand(8.0).intersects(Rect::from_min_size(b, super::CARD).expand(8.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn noodle_drop_snaps_and_skips_overlapping_cards() {
        let occupied = [pos2(32., 64.), pos2(32., 160.)];
        let at = free_position(pos2(33., 65.), &occupied);
        assert_eq!(at, pos2(32., 256.));
        assert_eq!(free_position(pos2(-65., -33.), &[]), pos2(-64., -32.));
        for peer in occupied {
            assert!(!Rect::from_min_size(peer, super::super::CARD).expand(8.).intersects(Rect::from_min_size(at, super::super::CARD).expand(8.)));
        }
    }

    fn graph(nodes: serde_json::Value) -> Graph { Graph::from_value(&serde_json::json!({"nodes": nodes})).unwrap() }

    fn checked_insert(g: &Graph, p: &[(String, Pos2)], anchor: &str, before: bool) -> HashMap<String, Pos2> {
        let result = insert_position(g, p, anchor, "new", before);
        for (i, (_, at)) in result.iter().enumerate() {
            assert_eq!(*at, super::super::snap(*at, 32., true, false));
            for (_, peer) in result.iter().skip(i + 1) { assert!(!overlaps(*at, *peer)); }
        }
        result.into_iter().collect()
    }

    #[test]
    fn insert_before_output_takes_its_slot_and_uses_stream_pitch() {
        let g = graph(serde_json::json!({"a":{"type":"normal"},"out":{"type":"output","inputs":{"geo":"a"}}}));
        for pitch in [96., 128., 160., 192.] {
            let p = vec![("a".into(), pos2(96., 32.)), ("out".into(), pos2(96., 32. + pitch))];
            let result = checked_insert(&g, &p, "out", true);
            assert_eq!(result["a"], p[0].1);
            assert_eq!(result["new"], p[1].1);
            assert_eq!(result["out"], pos2(96., 32. + 2. * pitch));
        }
    }

    #[test]
    fn insert_after_middle_pushes_exclusive_branches_but_not_shared_join() {
        let g = graph(serde_json::json!({
            "a":{"type":"normal"}, "b":{"type":"normal","inputs":{"geo":"a"}},
            "c":{"type":"normal","inputs":{"geo":"b"}},
            "branch":{"type":"normal","inputs":{"geo":"a"}}, "other":{"type":"box"},
            "join":{"type":"merge","inputs":{"geos":["b","other"]}}
        }));
        let p = vec![("a".into(),pos2(96.,32.)), ("b".into(),pos2(96.,160.)),
            ("c".into(),pos2(96.,288.)), ("branch".into(),pos2(-320.,160.)),
            ("other".into(),pos2(800.,32.)), ("join".into(),pos2(448.,544.))];
        let result = checked_insert(&g, &p, "a", false);
        assert_eq!(result["new"], pos2(96.,160.));
        assert_eq!(result["b"], pos2(96.,288.));
        assert_eq!(result["c"], pos2(96.,416.));
        assert_eq!(result["branch"], pos2(-320.,288.));
        assert_eq!(result["other"], p[4].1);
        assert_eq!(result["join"], p[5].1);
    }

    #[test]
    fn insertion_pushes_colliding_neighbor_column_and_cascades() {
        let g = graph(serde_json::json!({"a":{"type":"normal"},"out":{"type":"output","inputs":{"geo":"a"}},
            "block":{"type":"box"},"lower":{"type":"box"}}));
        let p = vec![("a".into(),pos2(96.,32.)), ("out".into(),pos2(96.,160.)),
            ("block".into(),pos2(288.,288.)), ("lower".into(),pos2(288.,416.))];
        let result = checked_insert(&g, &p, "out", true);
        assert_eq!(result["new"], pos2(96.,160.));
        assert_eq!(result["out"], pos2(96.,288.));
        assert_eq!(result["block"], pos2(288.,416.));
        assert_eq!(result["lower"], pos2(288.,544.));
        assert_eq!(insert_position(&g, &p, "out", "new", true), insert_position(&g, &p, "out", "new", true));
    }

    #[test]
    fn insertion_handles_empty_output_and_cycles() {
        let g = graph(serde_json::json!({"out":{"type":"output"}}));
        let result = checked_insert(&g, &[("out".into(), pos2(96.,32.))], "out", true);
        assert_eq!(result["out"],pos2(96.,160.));
        let g = graph(serde_json::json!({"a":{"type":"normal","inputs":{"geo":"b"}},"b":{"type":"normal","inputs":{"geo":"a"}}}));
        checked_insert(&g, &[("a".into(),pos2(96.,32.)),("b".into(),pos2(96.,160.))], "a", false);
    }

    #[test]
    fn chain_and_explicit_position() {
        let g = graph(serde_json::json!({"a":{"type":"box"},"b":{"type":"normal","inputs":{"geo":"a"}},"c":{"type":"output","inputs":{"geo":"b"}}}));
        let p = layout(&g, |id| (id == "b").then_some(pos2(420.0, 53.0)));
        assert_eq!(p, vec![("a".into(), pos2(96.0, 32.0)), ("b".into(), pos2(420.0, 53.0)), ("c".into(), pos2(96.0, 288.0))]);
    }

    #[test]
    fn branch_and_merge_uses_deepest_input() {
        let g = graph(serde_json::json!({"a":{"type":"box"},"b":{"type":"box"},"c":{"type":"normal","inputs":{"geo":"a"}},"d":{"type":"merge","inputs":{"geos":["b","c"]}}}));
        let p = layout(&g, |_| None);
        let at = |id| p.iter().find(|(n, _)| n == id).unwrap().1;
        assert_ne!(at("a").x, at("b").x);
        assert_eq!(at("c").y, 160.0);
        assert_eq!(at("d").y, 288.0);
        assert_eq!(at("c").x, at("a").x);
        assert_eq!(at("d").x, (at("b").x + at("c").x) * 0.5);
        assert_eq!(p, layout(&g, |_| None));
    }

    #[test]
    fn new_nodes_avoid_reserved_cards_and_each_other() {
        let g = graph(serde_json::json!({"a":{"type":"box"},"b":{"type":"box"},"c":{"type":"box"}}));
        let p = layout(&g, |id| (id == "a").then_some(pos2(96.0, 32.0)));
        assert_eq!(p[0].1, pos2(96.0, 32.0));
        for (i, (_, at)) in p.iter().enumerate() {
            assert_eq!(*at, super::super::snap(*at, 32.0, true, false));
            for (_, peer) in p.iter().skip(i + 1) {
                assert!(!Rect::from_min_size(*at, super::super::CARD).intersects(Rect::from_min_size(*peer, super::super::CARD)));
            }
        }
    }

    #[test]
    fn malformed_cycle_terminates() {
        let g = graph(serde_json::json!({"a":{"type":"normal","inputs":{"geo":"b"}},"b":{"type":"normal","inputs":{"geo":"a"}}}));
        assert_eq!(layout(&g, |_| None).len(), 2);
    }

    #[test]
    fn two_streams_keep_source_columns_spacing_and_grid() {
        let g = graph(serde_json::json!({
            "grid":{"type":"grid"},
            "noise":{"type":"noise","inputs":{"geo":"grid"}},
            "transform":{"type":"transform","inputs":{"geo":"noise"}},
            "normal":{"type":"normal","inputs":{"geo":"transform"}},
            "output":{"type":"output","inputs":{"geo":"normal"}},
            "torus":{"type":"torus"},
            "torus_transform":{"type":"transform","inputs":{"geo":"torus"}}
        }));
        // Reverse the authored/source order relative to document order.
        let p = layout_ordered(&g, |_| None, |id| match id {
            "grid" => Some(pos2(700., 330.)), "torus" => Some(pos2(-100., 400.)), _ => None,
        });
        let at = |id| p.iter().find(|(n, _)| n == id).unwrap().1;
        assert_eq!(at("grid").y, at("torus").y);
        assert!(at("grid").x - at("torus").x - super::super::CARD.x >= 2.0 * super::super::CARD.x);
        for chain in [&["grid", "noise", "transform", "normal", "output"][..], &["torus", "torus_transform"][..]] {
            for (row, id) in chain.iter().enumerate() {
                assert_eq!(at(id).x, at(chain[0]).x);
                assert_eq!(at(id).y, 32.0 + row as f32 * 128.0);
            }
        }
        for (_, position) in &p { assert_eq!(*position, super::super::snap(*position, 32., true, false)); }
        for (i, (_, a)) in p.iter().enumerate() {
            for (_, b) in p.iter().skip(i + 1) {
                assert!(!Rect::from_min_size(*a, super::super::CARD).intersects(Rect::from_min_size(*b, super::super::CARD)));
            }
        }
    }
}
