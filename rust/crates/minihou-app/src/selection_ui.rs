use super::*;
use crate::element_pick_glue::element_visible_at;

/// Picking records displayed triangle numbers; transform groups use the triangle's point numbers.
pub(crate) fn selected_point_ids(selection: &picking::Selection, mode: ui::SelectionMode, triangles: &[u32]) -> Vec<u32> {
    let mut ids = match mode {
        ui::SelectionMode::Object => Vec::new(),
        ui::SelectionMode::Points => selection.points.clone(),
        ui::SelectionMode::Edges => selection.edges.iter().flat_map(|&(a, b)| [a, b]).collect(),
        ui::SelectionMode::Faces => selection.tris.iter().flat_map(|&t| {
            triangles.get(t as usize * 3..t as usize * 3 + 3).unwrap_or(&[]).iter().copied()
        }).collect(),
    };
    ids.sort_unstable(); ids.dedup(); ids
}

pub(crate) fn select_nodes(set: &mut std::collections::BTreeSet<String>, ids: impl IntoIterator<Item = String>, extend: bool, toggle: bool) {
    if !extend && !toggle { set.clear(); }
    for id in ids {
        if toggle { if !set.remove(&id) { set.insert(id); } }
        else { set.insert(id); }
    }
}
/// Keep the last picked node active; removing it falls back to a remaining selection.
pub(crate) fn select_active_nodes(active: &mut String, set: &mut std::collections::BTreeSet<String>, ids: Vec<String>, extend: bool, toggle: bool) {
    let last = ids.last().cloned();
    select_nodes(set, ids, extend, toggle);
    if let Some(id) = last.filter(|id| set.contains(id)) { *active = id; }
    else if !set.contains(active) { *active = set.iter().next().cloned().unwrap_or_default(); }
}

impl App {
    pub(crate) fn region_select(&mut self, shape: picking::RegionShape, op: picking::SelectOp) {
        use picking::SelectOp;
        if self.object_hidden { return; }
        let Some(scene) = &self.scene else { return; };
        let camera = self.camera_info(); let points = scene.positions(); let tris = scene.triangle_indices();
        let active=self.active_object().and_then(|id|active_range(id,&self.display_ranges));
        let position = |i: u32| -> Option<[f32; 3]> { if self.editor_state.selection==ui::SelectionMode::Object {
                if !point_pickable(self.editor_state.object_visibility,&self.display_ranges,&self.selected_set,i){return None;}
            } else if !active.is_some_and(|range|range.contains(&i)){return None;}
            let k = i as usize * 3; let a = points.get(k..k + 3)?; Some([a[0], a[1], a[2]]) };
        let project = |p| ui::transform_gizmo::project(camera, self.last_viewport, p);
        let inside = |p| project(p).is_some_and(|q| shape.contains(q));
        let mode = self.editor_state.selection;
        let ignore_occlusion=picking::ignores_surface_occlusion(self.mode,self.editor_state.overlays.enabled,
            self.editor_state.xray);
        let active_pick_set=self.active_pick_set();
        let element_visible=|p| element_visible_at(scene,camera,self.last_viewport,p,
            viewport::ObjectVisibility::HideOthers,&self.display_ranges,&active_pick_set,ignore_occlusion);
        if mode == ui::SelectionMode::Object {
            if let Some((path, selected)) = &self.subnet_scope {
                let prefix = format!("{}/", path.join("/"));
                let mut candidates = Vec::new();
                let mut seen = std::collections::BTreeSet::new();
                for (id, range) in self.display_ranges.iter().rev().filter(|(id, _)| id.starts_with(&prefix)) {
                    if !seen.insert((range.start, range.end)) || self.display_ranges.iter().any(|(other, smaller)|
                        other.starts_with(&prefix) && smaller.start >= range.start && smaller.end <= range.end
                            && smaller.len() < range.len()) { continue; }
                    let Some(object_points) = points.get(range.start as usize * 3..range.end as usize * 3) else { continue; };
                    if object_points.is_empty() { continue; }
                    let (lo, hi) = minihou_core::preview::bounds(object_points);
                    if inside([0, 1, 2].map(|axis| (lo[axis] + hi[axis]) * 0.5)) {
                        candidates.push(id[prefix.len()..].to_owned());
                    }
                }
                let mut chosen = if op == SelectOp::Replace { std::collections::BTreeSet::new() }
                    else { std::collections::BTreeSet::from([selected.clone()]) };
                for id in candidates { if op == SelectOp::Subtract { chosen.remove(&id); } else { chosen.insert(id); } }
                let ids: Vec<_> = chosen.into_iter().collect();
                self.subnet_scope = Some((path.clone(), ids.first().cloned().unwrap_or_default()));
                self.subnet_pending_selection = Some(ids);
                self.selection_dirty = true;
                return;
            }
        }
        let mut found = picking::Selection { mode, ..Default::default() };
        match mode {
            ui::SelectionMode::Points => { for i in 0..points.len()/3 { if position(i as u32).is_some_and(|p|inside(p)&&element_visible(p)) { found.points.push(i as u32); } } }
            ui::SelectionMode::Edges => {
                let mut edges = std::collections::BTreeSet::new();
                for t in tris.as_chunks::<3>().0 { for (a,b) in [(t[0],t[1]),(t[1],t[2]),(t[2],t[0])] { edges.insert((a.min(b), a.max(b))); } }
                for pair in scene.line_primitive_indices().chunks_exact(2) { edges.insert((pair[0].min(pair[1]),pair[0].max(pair[1]))); }
                for (a,b) in edges {
                    if let (Some(a_pos), Some(b_pos)) = (position(a), position(b)) {
                        if let (Some(pa), Some(pb)) = (project(a_pos),project(b_pos)) {
                            if shape.contains_edge(pa, pb) && element_visible(a_pos) && element_visible(b_pos) { found.edges.push((a,b)); }
                        }
                    }
                }
            }
            ui::SelectionMode::Faces => { for (i,t) in tris.as_chunks::<3>().0.iter().enumerate() { if let (Some(a), Some(b), Some(c)) = (position(t[0]), position(t[1]), position(t[2])) { let center=[0,1,2].map(|k| (a[k]+b[k]+c[k])/3.); if inside(center) && element_visible(center) { found.tris.push(i as u32); } } } }
            ui::SelectionMode::Object => {
                let hits: Vec<String> = self.display_ranges.iter().filter_map(|(id, range)| {
                    if self.editor_state.object_visibility != viewport::ObjectVisibility::ShowAll && !self.selected_set.contains(id) { return None; }
                    let object_points = points.get(range.start as usize * 3..range.end as usize * 3)?;
                    if object_points.is_empty() { return None; }
                    let (lo, hi) = minihou_core::preview::bounds(object_points);
                    inside([0,1,2].map(|i| (lo[i] + hi[i]) * 0.5)).then(|| id.clone())
                }).collect();
                if op == SelectOp::Subtract { for id in hits { self.selected_set.remove(&id); } }
                else { select_active_nodes(&mut self.selected, &mut self.selected_set, hits, op == SelectOp::Add, false); }
                if !self.selected_set.contains(&self.selected) { self.selected = self.selected_set.iter().next().cloned().unwrap_or_default(); }
                self.selection_dirty = true;
                return;
            }
        }
        // whole polygons, polygon edges only (no fan diagonals)
        let found = element_select::cached(&self.elem_topo, scene).complete(found);
        self.selection_dirty |= element_select::apply_region(&mut self.selection, found, op);
    }

    pub(crate) fn selected_points(&self) -> Vec<u32> {
        selected_point_ids(&self.selection, self.editor_state.selection,
            self.scene.as_ref().map_or(&[], Scene::triangle_indices)).into_iter().filter(|id| self.active_object().and_then(|active|active_range(active,&self.display_ranges)).is_some_and(|range|range.contains(id))).collect()
    }
    pub(crate) fn active_object(&self) -> Option<&str> {
        // while a field is being picked the displayed pick source is the active object (the selected node is not displayed)
        if let Some(pick) = &self.param_pick { return active_range(&pick.source, &self.display_ranges).map(|_| pick.source.as_str()); }
        if let Some((path, selected)) = &self.subnet_scope {
            if self.object_hidden { return None; }
            return picking::scoped_display_owner(self.editor.doc.value(), path, selected, &self.display_ranges);
        }
        if self.object_hidden || !self.selected_set.contains(&self.selected) { return None; }
        resolve_active_displayed_node(self.editor.doc.value(), &self.selected, &self.display_ranges,
            self.output_range_count, self.selected_preview)
    }
    pub(crate) fn active_selection(&self) -> Option<picking::Selection> {
        Some(selection_in_range(&self.selection,self.scene.as_ref()?,active_range(self.active_object()?,&self.display_ranges)?))
    }
    /// Objects whose points can be picked/highlighted: the node selection plus the pick source while a field pick is active.
    pub(crate) fn pick_set(&self) -> std::collections::BTreeSet<String> {
        let mut set = self.selected_set.clone();
        if let Some((path, selected)) = &self.subnet_scope {
            if let Some(owner) = picking::scoped_display_owner(self.editor.doc.value(), path, selected, &self.display_ranges) {
                set.insert(owner.to_owned());
            }
        }
        if let Some(pick) = &self.param_pick { set.insert(pick.source.clone()); }
        set
    }
    pub(crate) fn active_pick_set(&self) -> std::collections::BTreeSet<String> {
        self.active_object().map(|id|std::collections::BTreeSet::from([id.to_owned()])).unwrap_or_default()
    }
    pub(crate) fn selection_center(&self) -> Option<[f32; 3]> {
        if self.editor_state.selection == ui::SelectionMode::Object {
            if self.subnet_scope.is_some() {
                let range = active_range(self.active_object()?, &self.display_ranges)?;
                let points = self.scene.as_ref()?.positions().get(range.start as usize * 3..range.end as usize * 3)?;
                if points.is_empty() { return None; }
                let (lo, hi) = minihou_core::preview::bounds(points);
                return Some([0, 1, 2].map(|axis| (lo[axis] + hi[axis]) * 0.5));
            }
            return self.active_object().map(|id| object_origin(self.editor.doc.value(), id));
        }
        self.element_metrics().center
    }
    pub(crate) fn element_metrics(&self) -> selection_metrics::Metrics {
        let Some(scene)=self.scene.as_ref() else { return Default::default(); };
        self.selection_metrics.get(self.cooking.completed,&self.active_selection().unwrap_or_default(),self.editor_state.selection,scene,
            viewport::ObjectVisibility::HideOthers,&self.display_ranges,&self.active_pick_set())
    }
}
