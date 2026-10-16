//! Shift-drag layout springs. Only card positions change; graph connections are read-only.
use eframe::egui::{Pos2, Vec2};
use minihou_core::Graph;
use std::collections::{BTreeSet, HashMap};

const SETTLE_THRESHOLD: f32 = 0.05;

#[derive(Clone)]
pub(super) struct Springs {
    edges: Vec<(String, String, Vec2)>,
    velocity: HashMap<String, Vec2>,
    pinned: BTreeSet<String>,
    nodes: BTreeSet<String>,
}
impl Springs {
    pub(super) fn new(graph: &Graph, positions: &[(String, Pos2)], pinned: BTreeSet<String>) -> Self {
        let lookup: HashMap<_, _> = positions.iter().map(|(id, at)| (id.as_str(), *at)).collect();
        let mut edges = Vec::new();
        let mut seen = BTreeSet::new();
        for (target, node) in &graph.nodes {
            for (_, sources) in &node.inputs {
                for source in sources {
                    if source == target || !seen.insert((source.clone(), target.clone())) { continue; }
                    if let (Some(a), Some(b)) = (lookup.get(source.as_str()), lookup.get(target.as_str())) {
                        edges.push((source.clone(), target.clone(), *b - *a));
                    }
                }
            }
        }
        Self { edges, velocity: HashMap::new(), pinned, nodes: lookup.keys().map(|id| (*id).to_owned()).collect() }
    }

    /// Damped springs preserve each wire's original displacement. Selected cards are anchors.
    pub(super) fn step(&mut self, positions: &mut [(String, Pos2)], dt: f32) -> bool {
        self.step_with_snappiness(positions, dt, 0.7)
    }

    pub(super) fn step_with_snappiness(&mut self, positions: &mut [(String, Pos2)], dt: f32, snappiness: f32) -> bool {
        // Small substeps keep the stiff end of the slider stable even on slow frames.
        let dt = dt.clamp(0.001, 0.032);
        let steps = (dt / 0.008).ceil() as usize;
        let mut moving = false;
        for _ in 0..steps { moving = self.integrate(positions, dt / steps as f32, snappiness.clamp(0.0, 1.0)); }
        moving
    }

    fn integrate(&mut self, positions: &mut [(String, Pos2)], dt: f32, snappiness: f32) -> bool {
        if positions.len() != self.nodes.len()
            || positions.iter().any(|(id, _)| !self.nodes.contains(id)) { return false; }
        let lookup: HashMap<_, _> = positions.iter().map(|(id, at)| (id.as_str(), *at)).collect();
        let mut forces: HashMap<&str, (Vec2, f32)> = HashMap::new();
        for (source, target, rest) in &self.edges {
            let (Some(a), Some(b)) = (lookup.get(source.as_str()), lookup.get(target.as_str())) else { continue };
            let error = *b - *a - *rest;
            for (id, force) in [(source.as_str(), error), (target.as_str(), -error)] {
                let entry = forces.entry(id).or_insert((Vec2::ZERO, 0.0));
                entry.0 += force;
                entry.1 += 1.0;
            }
        }
        let stiffness = 100.0 + 700.0 * snappiness;
        let damping = (-(20.0 - 4.0 * snappiness) * dt).exp();
        let mut moving = false;
        for (id, at) in positions.iter_mut() {
            if self.pinned.contains(id) { self.velocity.remove(id); continue; }
            let (force, degree) = forces.get(id.as_str()).copied().unwrap_or((Vec2::ZERO, 1.0));
            let force = force / degree;
            let velocity = self.velocity.entry(id.clone()).or_insert(Vec2::ZERO);
            *velocity = (*velocity + force * (stiffness * dt)) * damping;
            *at += *velocity * dt;
            moving |= force.length() >= SETTLE_THRESHOLD || velocity.length() >= SETTLE_THRESHOLD;
        }
        moving
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::pos2;

    #[test]
    fn snappiness_follows_faster_and_settles_sooner_at_bounded_frame_rates() {
        let graph = Graph::from_value(&serde_json::json!({"nodes": {
            "a":{"type":"box"}, "b":{"type":"normal","inputs":{"geo":"a"}}
        }})).unwrap();
        for dt in [1. / 120., 1. / 60., 0.032] {
            let mut settling = Vec::new();
            let mut early = Vec::new();
            for snappiness in [0.0, 0.7, 1.0] {
                let mut positions = vec![("a".into(),pos2(0.,0.)), ("b".into(),pos2(0.,96.))];
                let mut springs = Springs::new(&graph, &positions, BTreeSet::from(["a".into()]));
                positions[0].1.x = 160.;
                let mut settled = None;
                for frame in 0..1200 {
                    let moving = springs.step_with_snappiness(&mut positions, dt, snappiness);
                    assert!(positions[1].1.x.is_finite());
                    if frame == 3 { early.push(positions[1].1.x); }
                    if !moving { settled = Some(frame); break; }
                }
                settling.push(settled.expect("springs must settle"));
                assert!((positions[1].1.x - 160.).abs() < 0.1);
            }
            assert!(early[1] > early[0] * 2.0);
            assert!(settling[1] < settling[0]);
            assert!(settling[2] < settling[0]);
        }
    }

    #[test]
    fn shift_drag_pulls_chain_with_damping_and_keeps_unconnected_nodes_fixed() {
        let doc = serde_json::json!({"nodes": {
            "a":{"type":"box"}, "b":{"type":"normal","inputs":{"geo":"a"}},
            "c":{"type":"normal","inputs":{"geo":"b"}}, "alone":{"type":"box"}
        }});
        let graph = Graph::from_value(&doc).unwrap();
        let mut positions = vec![("a".into(),pos2(0.,0.)), ("b".into(),pos2(0.,96.)),
            ("c".into(),pos2(0.,192.)), ("alone".into(),pos2(512.,0.))];
        let mut springs = Springs::new(&graph, &positions, BTreeSet::from(["a".into()]));
        positions[0].1.x = 160.;
        assert!(springs.step(&mut positions, 1. / 60.));
        assert!(positions[1].1.x > 0. && positions[1].1.x < 160.);
        for _ in 0..2400 { springs.step(&mut positions, 1. / 60.); }
        assert_eq!(positions[0].1, pos2(160.,0.));
        assert!((positions[1].1.x - 160.).abs() < 0.1);
        assert!((positions[2].1.x - 160.).abs() < 0.1);
        assert_eq!(positions[3].1, pos2(512.,0.));
        assert_eq!(graph.nodes.iter().find(|(id, _)| id == "b").unwrap().1.inputs[0].1, vec!["a"]);
    }

    #[test]
    fn selected_nodes_are_pinned_and_topology_changes_stop_simulation() {
        let graph = Graph::from_value(&serde_json::json!({"nodes": {
            "a":{"type":"box"},"b":{"type":"normal","inputs":{"geo":"a"}}
        }})).unwrap();
        let mut positions = vec![("a".into(),pos2(0.,0.)),("b".into(),pos2(0.,96.))];
        let mut springs = Springs::new(&graph, &positions, BTreeSet::from(["a".into(),"b".into()]));
        positions[0].1.x = 128.;
        assert!(!springs.step(&mut positions, 1. / 60.));
        assert_eq!(positions[1].1, pos2(0.,96.));
        positions.pop();
        assert!(!springs.step(&mut positions, 1.));
    }

    #[test]
    fn dragging_middle_card_pulls_both_directions_then_settles() {
        let graph = Graph::from_value(&serde_json::json!({"nodes": {
            "a":{"type":"grid"}, "b":{"type":"transform","inputs":{"geo":"a"}},
            "c":{"type":"normal","inputs":{"geo":"b"}}
        }})).unwrap();
        let mut positions = vec![("a".into(), pos2(0.,0.)), ("b".into(), pos2(0.,128.)), ("c".into(), pos2(0.,256.))];
        let mut springs = Springs::new(&graph, &positions, BTreeSet::from(["b".into()]));
        positions[1].1.x = 200.;
        assert!(springs.step(&mut positions, 1. / 60.));
        assert!(positions[0].1.x > 0. && positions[2].1.x > 0.);
        // Release keeps the dropped card anchored while its neighbours finish following.
        let mut settled = false;
        for _ in 0..2400 {
            if !springs.step(&mut positions, 1. / 60.) { settled = true; break; }
        }
        assert!(settled, "settled springs must stop scheduling repaints");
        assert_eq!(positions[1].1, pos2(200.,128.));
        assert!((positions[0].1.x - 200.).abs() < 0.1);
        assert!((positions[2].1.x - 200.).abs() < 0.1);
    }
}
