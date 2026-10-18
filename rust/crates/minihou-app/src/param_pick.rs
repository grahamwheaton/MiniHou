//! Viewport parameter selection state and event routing.
use crate::{element_select, picking, ui};
use eframe::egui;
use minihou_app::Edit;
use serde_json::json;
use std::collections::BTreeSet;

pub(super) fn input_source(doc: &serde_json::Value, node: &str, display: &str) -> String {
    let inputs = &doc["nodes"][node]["inputs"];
    inputs["geo"].as_str()
        .or_else(|| inputs.as_object().and_then(|m| m.values().find_map(|v| v.as_str().filter(|s| !s.is_empty()))))
        .unwrap_or(display).to_owned()
}
pub(super) fn source_ready(ranges: &[(String, std::ops::Range<u32>)], source: &str) -> bool {
    ranges.len() == 1 && ranges[0].0 == source
}

pub(super) struct PickedField {
    pub(super) node: String,
    pub(super) selection: picking::Selection,
    pub(super) faces: Vec<f32>,
    pub(super) lines: Vec<f32>,
}

pub(super) struct ParamPick {
    pub(super) node: String, pub(super) name: String, pub(super) original: String, pub(super) changed: bool,
    pub(super) previous_selection: picking::Selection, pub(super) previous_mode: ui::SelectionMode, pub(super) pairs: bool,
    pub(super) source: String, pub(super) previous_display: String,
    pub(super) previous_selected_set: BTreeSet<String>, pub(super) ready: bool, pub(super) load: bool,
}
impl ParamPick {
    pub(super) fn text(&self, topo: &element_select::Topology, selection: &picking::Selection) -> String {
        let text = topo.group_string(selection);
        if self.pairs && selection.mode == ui::SelectionMode::Edges {
            text.split_whitespace().map(|token| token.trim_start_matches('p')).collect::<Vec<_>>().join(" ")
        } else { text }
    }
    pub(super) fn clicked(&mut self, topo: &element_select::Topology, selection: &picking::Selection) -> Edit {
        self.changed = true;
        Edit::Param { node:self.node.clone(), name:self.name.clone(), value:json!(self.text(topo, selection)) }
    }
    pub(super) fn cancel_edit(&self) -> Edit {
        Edit::Param { node:self.node.clone(), name:self.name.clone(), value:json!(self.original) }
    }
    pub(super) fn highlight(&self, selection: &picking::Selection, positions: &[f32], triangles: &[u32], size: f32) -> PickedField {
        let (faces, lines) = picking::highlight(positions, triangles, selection, size);
        PickedField { node:self.node.clone(), selection:selection.clone(), faces, lines }
    }
}
/// Flood Fill's `domain` menu as a selection mode.
pub(super) fn flood_domain(params: &serde_json::Value) -> ui::SelectionMode {
    ui::params::pick_domain(Some(&json!({"pick_domain":params["domain"].as_str().unwrap_or("points")})), params).unwrap_or(ui::SelectionMode::Points)
}
/// Orange overlay geometry (faces, lines) for the barrier group text on the node's input topology.
pub(super) fn barrier_overlay(topo: &element_select::Topology, text: &str, mode: ui::SelectionMode,
        positions: &[f32], triangles: &[u32], size: f32) -> (Vec<f32>, Vec<f32>) {
    if text.trim().is_empty() { return Default::default(); }
    picking::highlight(positions, triangles, &topo.parse_group(text, mode), size)
}
/// The element type of a field's text: edge pairs (`0-1`, `p0-1`) mean edges whatever the node's default domain says
/// (faces keep their default, where `a-b` may be a range).
pub(super) fn infer_mode(text: &str, default: ui::SelectionMode) -> ui::SelectionMode {
    let pair = |t: &str| t.trim_start_matches('p').split_once('-').is_some_and(|(a, b)| a.parse::<u32>().is_ok() && b.parse::<u32>().is_ok());
    if default != ui::SelectionMode::Faces && text.split_whitespace().any(pair) { ui::SelectionMode::Edges } else { default }
}
/// Parse a group field into a selection on `topo` plus its purple highlight (shared by the arrow hover and the pick start).
pub(super) fn field_selection(topo: &element_select::Topology, text: &str, mode: ui::SelectionMode, node: &str,
        positions: &[f32], triangles: &[u32], size: f32) -> PickedField {
    let selection = topo.parse_group(text, mode);
    let (faces, lines) = picking::highlight(positions, triangles, &selection, size);
    PickedField { node:node.to_owned(), selection, faces, lines }
}
pub(super) fn param_pick_end(ctx: &egui::Context, viewport: egui::Rect) -> Option<bool> {
    ctx.input(|i| {
        if i.key_pressed(egui::Key::Escape) { Some(true) }
        else if i.key_pressed(egui::Key::Enter) ||
            (i.pointer.primary_pressed() && i.pointer.latest_pos().is_some_and(|p| !viewport.contains(p))) { Some(false) }
        else { None }
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn barrier_overlay_colour_differs_from_seed_overlay() {
        let shader = include_str!("shader.wgsl");
        let colour = |name: &str| { let l = shader.lines().find(|l| l.contains(name)).unwrap(); l.split("vec3(").nth(1).unwrap().split(')').next().unwrap().to_owned() };
        assert_ne!(colour("fn fs_barrier_line"), colour("fn fs_group_line"));
        assert_ne!(colour("fn fs_barrier_fill"), colour("fn fs_group_fill"));
    }
    #[test]
    fn flood_domain_follows_menu() {
        assert_eq!(flood_domain(&json!({})), ui::SelectionMode::Points);
        assert_eq!(flood_domain(&json!({"domain":"edges"})), ui::SelectionMode::Edges);
        assert_eq!(flood_domain(&json!({"domain":"faces"})), ui::SelectionMode::Faces);
    }

    use super::*;
    fn quad() -> (Vec<f32>, Vec<u32>, Vec<u32>) {
        (vec![0.,0.,0., 1.,0.,0., 1.,1.,0., 0.,1.,0.], vec![0,1,2,0,2,3], vec![0,1,1,2,2,3,3,0])
    }
    #[test]
    fn group_text_round_trips_through_selection() {
        let (p, t, e) = quad();
        let topo = element_select::Topology::new(&p, &t, &e);
        for (mode, text, pairs) in [(ui::SelectionMode::Points, "0 2 3", false), (ui::SelectionMode::Edges, "0-1 2-3", true), (ui::SelectionMode::Faces, "0", false)] {
            let sel = topo.parse_group(text, mode);
            assert!(!sel.is_empty(), "{text}");
            let pick = ParamPick { node:"n".into(), name:"group".into(), original:text.into(), changed:false, previous_selection:Default::default(),
                previous_mode:ui::SelectionMode::Object, pairs, source:"s".into(), previous_display:"d".into(), previous_selected_set:Default::default(), ready:true, load:false };
            assert_eq!(pick.text(&topo, &sel), text);
        }
        assert!(topo.parse_group("9 0", ui::SelectionMode::Points).points == vec![0]); // out-of-range ids dropped
        assert!(topo.parse_group("0-2", ui::SelectionMode::Edges).is_empty()); // not a polygon edge
    }
    #[test]
    fn edge_text_previews_as_edges_even_when_the_domain_says_points() {
        let (p, t, e) = quad();
        let topo = element_select::Topology::new(&p, &t, &e);
        for text in ["0-1", "p0-1 p1-2"] {
            let mode = infer_mode(text, ui::SelectionMode::Points);
            assert_eq!(mode, ui::SelectionMode::Edges);
            let hover = field_selection(&topo, text, mode, "n", &p, &t, 0.01);
            assert!(!hover.selection.edges.is_empty() && hover.selection.points.is_empty());
            // the arrow click loads the same parse as the live selection, in the field's element type, with a matching count
            assert_eq!(hover.selection.count(), text.split_whitespace().count());
            assert_eq!(hover.selection.mode, ui::SelectionMode::Edges);
        }
        assert_eq!(infer_mode("0 1", ui::SelectionMode::Points), ui::SelectionMode::Points);
        assert_eq!(infer_mode("0-3", ui::SelectionMode::Faces), ui::SelectionMode::Faces);
    }
    #[test]
    fn hover_and_arrow_click_keep_the_field_selection() {
        let (p, t, e) = quad();
        let topo = element_select::Topology::new(&p, &t, &e);
        let hover = field_selection(&topo, "0 1", ui::SelectionMode::Points, "n", &p, &t, 0.01);
        assert_eq!(hover.selection.points, vec![0, 1]);
        assert!(!hover.lines.is_empty());
        // Arrow click loads the same parse into the live selection; a toggle then edits it and Enter would write the text.
        let mut live = field_selection(&topo, "0 1", ui::SelectionMode::Points, "n", &p, &t, 0.01).selection;
        assert_eq!(live.points, vec![0, 1]);
        assert!(topo.apply_click(&p, element_select::ClickKind::Toggle, Some((picking::Selection { mode:ui::SelectionMode::Points, points:vec![2], ..Default::default() }, 0, [0.;3])), ui::SelectionMode::Points, &mut live));
        let pick = ParamPick { node:"n".into(), name:"group".into(), original:"0 1".into(), changed:false, previous_selection:Default::default(),
            previous_mode:ui::SelectionMode::Object, pairs:false, source:"s".into(), previous_display:"d".into(), previous_selected_set:Default::default(), ready:true, load:false };
        assert_eq!(pick.text(&topo, &live), "0 1 2");
        assert!(matches!(pick.cancel_edit(), Edit::Param { value, .. } if value == "0 1"));
    }
    #[test]
    fn subdivide_picks_input_topology() {
        let doc = json!({"nodes":{"box1":{"type":"box"},"subdivide1":{"type":"subdivide","inputs":{"geo":"box1"}}}});
        assert_eq!(input_source(&doc,"subdivide1","subdivide1"),"box1");
        let doc2 = json!({"nodes":{"b":{"type":"box"},"n":{"type":"blast","inputs":{"input":"b"}}}});
        assert_eq!(input_source(&doc2,"n","n"),"b");
        assert_eq!(input_source(&json!({"nodes":{"n":{"type":"blast"}}}),"n","disp"),"disp");
        assert!(!source_ready(&[("subdivide1".into(), 0..8)], "box1"));
        assert!(source_ready(&[("box1".into(), 0..8)], "box1"));
    }
    #[test]
    fn escape_cancels_parameter_pick_without_committing_its_original_text() {
        let ctx = egui::Context::default();
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.,300.));
        let mut end = None;
        let _ = ctx.run(egui::RawInput { events:vec![egui::Event::Key { key:egui::Key::Escape,
            physical_key:None, pressed:true, repeat:false, modifiers:egui::Modifiers::NONE }],
            ..Default::default() }, |ctx| end = param_pick_end(ctx, viewport));
        assert_eq!(end, Some(true));
        let original = ParamPick { node:"n".into(), name:"group".into(), original:"3 5".into(),
            changed:true, previous_selection:Default::default(), previous_mode:ui::SelectionMode::Object, pairs:false,
            source:"input".into(), previous_display:"output".into(), previous_selected_set:Default::default(), ready:false, load:false };
        assert!(matches!(original.cancel_edit(), Edit::Param {value,..} if value=="3 5"));
    }

    #[test]
    fn parameter_pick_routes_selection_to_face_field_and_highlight() {
        let positions = [0.,0.,0., 1.,0.,0., 1.,1.,0., 0.,1.,0.];
        let tris = [0,1,2,0,2,3];
        let edges = [0,1,1,2,2,3,3,0];
        let topo = element_select::Topology::new(&positions, &tris, &edges);
        let mut selection = picking::Selection { mode:ui::SelectionMode::Faces, ..Default::default() };
        let hit = picking::Hit { tri:0, t:1., bary:[0.3,0.3,0.4] };
        let picked = topo.pick(&positions, &tris, hit, [0.7,0.4,0.], ui::SelectionMode::Faces);
        assert!(topo.apply_click(&positions, element_select::ClickKind::Pick,
            Some((picked, 0, [0.7,0.4,0.])), ui::SelectionMode::Faces, &mut selection));
        let mut field = ParamPick { node:"extrude1".into(), name:"group".into(), original:"old".into(),
            changed:false, previous_selection:Default::default(), previous_mode:ui::SelectionMode::Object, pairs:false,
            source:"input".into(), previous_display:"output".into(), previous_selected_set:Default::default(), ready:false, load:false };
        assert!(matches!(field.clicked(&topo, &selection), Edit::Param { value, .. } if value == "0"));
        assert_eq!(field.original, "old"); // Esc restores this snapshot.
        let (faces, lines) = picking::highlight(&positions, &tris, &selection, 0.01);
        assert!(!faces.is_empty());
        assert_eq!(faces.len(), selection.tris.len() * 9);
        assert!(lines.is_empty());
        let highlight = field.highlight(&selection, &positions, &tris, 0.01);
        assert_eq!(highlight.selection.tris, selection.tris);
        assert_eq!(highlight.faces, faces);
        let edge_selection = picking::Selection { mode:ui::SelectionMode::Edges, edges:vec![(0,1)], ..Default::default() };
        field.pairs = true;
        assert!(matches!(field.clicked(&topo, &edge_selection), Edit::Param { value, .. } if value == "0-1"));
    }

}
