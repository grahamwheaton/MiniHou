use eframe::egui::{pos2, Pos2};
use minihou_core::Graph;
use std::collections::{HashMap, VecDeque};

pub(super) fn layout(graph: &Graph, pos_lookup: impl Fn(&str) -> Option<Pos2>) -> Vec<(String, Pos2)> {
    let ids: HashMap<&str, usize> = graph.nodes.iter().enumerate().map(|(i, (id, _))| (id.as_str(), i)).collect();
    let mut children = vec![Vec::new(); graph.nodes.len()];
    let mut incoming = vec![0usize; graph.nodes.len()];
    for (target, (_, def)) in graph.nodes.iter().enumerate() {
        for (_, refs) in &def.inputs {
            for source in refs {
                if let Some(&source) = ids.get(source.as_str()) {
                    children[source].push(target);
                    incoming[target] += 1;
                }
            }
        }
    }
    let mut queue: VecDeque<_> = incoming.iter().enumerate().filter(|(_, count)| **count == 0).map(|(i, _)| i).collect();
    let mut depth = vec![0usize; graph.nodes.len()];
    while let Some(source) = queue.pop_front() {
        for &target in &children[source] {
            depth[target] = depth[target].max(depth[source] + 1);
            incoming[target] -= 1;
            if incoming[target] == 0 { queue.push_back(target); }
        }
    }
    // Malformed cycles stay at their last finite depth; the queue cannot loop forever.
    let mut layers: HashMap<usize, Vec<usize>> = HashMap::new();
    for (i, &d) in depth.iter().enumerate() { layers.entry(d).or_default().push(i); }
    graph.nodes.iter().enumerate().map(|(i, (id, _))| {
        let siblings = &layers[&depth[i]];
        let index = siblings.iter().position(|&s| s == i).unwrap_or(0);
        let x = 90.0 + (index as f32 - (siblings.len() - 1) as f32 / 2.0) * 250.0;
        let automatic = pos2(x, 34.0 + depth[i] as f32 * 100.0);
        (id.clone(), pos_lookup(id).unwrap_or(automatic))
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph(nodes: serde_json::Value) -> Graph { Graph::from_value(&serde_json::json!({"nodes": nodes})).unwrap() }

    #[test]
    fn chain_and_explicit_position() {
        let g = graph(serde_json::json!({"a":{"type":"box"},"b":{"type":"normal","inputs":{"geo":"a"}},"c":{"type":"output","inputs":{"geo":"b"}}}));
        let p = layout(&g, |id| (id == "b").then_some(pos2(42.0, 53.0)));
        assert_eq!(p, vec![("a".into(), pos2(90.0, 34.0)), ("b".into(), pos2(42.0, 53.0)), ("c".into(), pos2(90.0, 234.0))]);
    }

    #[test]
    fn branch_and_merge_uses_deepest_input() {
        let g = graph(serde_json::json!({"a":{"type":"box"},"b":{"type":"box"},"c":{"type":"normal","inputs":{"geo":"a"}},"d":{"type":"merge","inputs":{"geos":["b","c"]}}}));
        let p = layout(&g, |_| None);
        let at = |id| p.iter().find(|(n, _)| n == id).unwrap().1;
        assert_ne!(at("a").x, at("b").x);
        assert_eq!(at("c").y, 134.0);
        assert_eq!(at("d").y, 234.0);
        assert_eq!(p, layout(&g, |_| None));
    }

    #[test]
    fn malformed_cycle_terminates() {
        let g = graph(serde_json::json!({"a":{"type":"normal","inputs":{"geo":"b"}},"b":{"type":"normal","inputs":{"geo":"a"}}}));
        assert_eq!(layout(&g, |_| None).len(), 2);
    }
}
