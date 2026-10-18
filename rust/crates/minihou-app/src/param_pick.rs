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
    pub(super) previous_selected_set: BTreeSet<String>, pub(super) ready: bool,
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
    use super::*;
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
            source:"input".into(), previous_display:"output".into(), previous_selected_set:Default::default(), ready:false };
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
            source:"input".into(), previous_display:"output".into(), previous_selected_set:Default::default(), ready:false };
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
