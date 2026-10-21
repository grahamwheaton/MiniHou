use super::*;

pub(super) fn node_positions(state: &State, v: &View) -> Vec<(String, Pos2)> {
    positions_for(state, v.graph, v.doc)
}

pub(super) fn positions_for(state: &State, graph: &minihou_core::Graph, doc: &serde_json::Value) -> Vec<(String, Pos2)> {
    let authored = |id: &str| json_pos(doc, id);
    network_layout::layout_ordered(graph, |id| state.moved.get(id).copied()
        .or_else(|| (!state.tidied).then(|| authored(id)).flatten()),
        |id| state.tidy_order.get(id).copied().or_else(|| authored(id)))
}

pub(super) fn insertion_candidate(doc: &serde_json::Value, node: &str, rect: Rect, wires: &[(WireRef, Vec<Pos2>)]) -> Option<WireRef> {
    wires.iter().filter_map(|(wire, points)| {
        let distance = network_wires::node_hit(points, rect)?;
        crate::graph_edit::insert_existing(doc, node, wire)?;
        Some((wire, distance))
    }).min_by(|(a, da), (b, db)| da.total_cmp(db).then_with(|| a.cmp(b))).map(|(wire, _)| wire.clone())
}

pub(super) fn insertion_candidate_at_point(doc: &serde_json::Value, node: &str, at: Pos2, wires: &[(WireRef, Vec<Pos2>)]) -> Option<WireRef> {
    crate::noodle_hit::closest_at_point(wires, at, 8.0, |wire| crate::graph_edit::insert_existing(doc, node, wire).is_some())
}

pub fn insert_added(_ctx: &egui::Context, _path: &Path, old: &serde_json::Value, next: &mut serde_json::Value, node: &str, _pos: [f32; 2], drop_wire: Option<&WireRef>) -> bool {
    let candidate = drop_wire.cloned();
    if let Some(wire) = candidate {
        if let Some(mut inserted) = crate::graph_edit::insert_existing(next, node, &wire) {
            // A stream insertion keeps the existing display/output flags.
            if let Some(display) = old.get("display") { inserted["display"] = display.clone(); }
            else { inserted.as_object_mut().unwrap().remove("display"); }
            *next = inserted;
            return true;
        }
    }
    false
}

pub(super) fn shelf_drop_position(at: Pos2, canvas: Rect, pan: Vec2, zoom: f32, snap_enabled: bool, invert: bool) -> Pos2 {
    let center = ((at - canvas.min - pan) / zoom).to_pos2();
    snap(center - CARD * 0.5, 32.0, snap_enabled, invert)
}

pub(super) fn ghost_card(p: &egui::Painter, rect: Rect, node_type: &str, v: &View, zoom: f32) {
    let theme = v.theme;
    p.rect_filled(rect, 6.0 * zoom, theme.card.gamma_multiply(0.7));
    p.rect_stroke(rect, 6.0 * zoom, Stroke::new(1.5, theme.accent.gamma_multiply(0.8)), StrokeKind::Inside);
    let dot = pos2(rect.left() + 35.0 * zoom, rect.center().y);
    p.circle_filled(dot, 5.0 * zoom, theme.category_color(node_type).gamma_multiply(0.8));
    p.text(dot + vec2(13.0 * zoom, 0.0), Align2::LEFT_CENTER, node_type, FontId::proportional(13.0 * zoom), theme.text.gamma_multiply(0.8));
}

/// Persist insertion layout in the add's document edit, using the actual canvas
/// positions (which may have been dragged or tidied since the file was saved).
pub fn position_add(ctx: &egui::Context, path: &Path, old: &serde_json::Value,
    next: &mut serde_json::Value, after: Option<&str>) -> Option<minihou_app::Document> {
    position_add_impl(ctx, path, old, next, after, &[])
}

pub fn position_add_merge(ctx: &egui::Context, path: &Path, old: &serde_json::Value,
    next: &mut serde_json::Value, selected: &[String]) -> Option<minihou_app::Document> {
    position_add_impl(ctx, path, old, next, None, selected)
}

fn position_add_impl(ctx: &egui::Context, path: &Path, old: &serde_json::Value,
    next: &mut serde_json::Value, after: Option<&str>, merge_sources: &[String]) -> Option<minihou_app::Document> {
    let (Ok(graph), Ok(next_graph)) = (minihou_core::Graph::from_value(old), minihou_core::Graph::from_value(next)) else { return None };
    let (state, add_pos) = ctx.data_mut(|d| {
        let mut s = d.get_persisted::<State>(state_id(path)).unwrap_or_default();
        let pos = s.add_pos.take();
        d.insert_persisted(state_id(path), s.clone());
        (s, pos)
    });
    let mut positions = positions_for(&state, &graph, old);
    let live_override = after.is_some_and(|anchor| state.moved.get(anchor).is_some_and(|at| json_pos(old, anchor) != Some(*at)));
    // History must remember the pre-add canvas too, including unsaved drags.
    let mut before = old.clone();
    for (id, at) in &positions { before["nodes"][id]["pos"] = serde_json::json!([at.x, at.y]); }
    if let Some(anchor) = after.filter(|id| old["nodes"].get(*id).is_some()) {
        let before = old["nodes"][anchor]["type"] == "output";
        let inserted = next_graph.nodes.iter().find(|(id, node)| old["nodes"].get(id).is_none() &&
            if before { next_graph.nodes.iter().any(|(target, def)| target == anchor &&
                def.inputs.iter().any(|(_, refs)| refs.contains(id))) }
            else { node.inputs.iter().any(|(_, refs)| refs.iter().any(|source| source == anchor)) });
        if let Some((id, _)) = inserted {
            positions = network_layout::insert_position(&graph, &positions, anchor, id, before);
        }
    }
    let mut lookup: HashMap<_, _> = positions.into_iter().collect();
    if !merge_sources.is_empty() {
        let sources: Vec<_> = merge_sources.iter().filter_map(|id| lookup.get(id).copied()).collect();
        if sources.len() == merge_sources.len() {
            if let Some((id, _)) = next_graph.nodes.iter().find(|(id, _)| old["nodes"].get(id).is_none()) {
                let left = sources.iter().map(|at| at.x).fold(f32::INFINITY, f32::min);
                let right = sources.iter().map(|at| at.x).fold(f32::NEG_INFINITY, f32::max);
                let bottom = sources.iter().map(|at| at.y).fold(f32::NEG_INFINITY, f32::max);
                let preferred = pos2((left + right) * 0.5, bottom + 128.0);
                lookup.insert(id.clone(), network_layout::free_position(preferred, &lookup.values().copied().collect::<Vec<_>>()));
            }
        }
    }
    if after.is_none() && merge_sources.is_empty() {
        if let Some(pos) = add_pos {
            if let Some((id, _)) = next_graph.nodes.iter().find(|(id, _)| old["nodes"].get(id).is_none()) {
                lookup.insert(id.clone(), pos);
            }
        }
    }
    for (id, _) in &next_graph.nodes {
        if !live_override && (old["nodes"].get(id).is_none() || json_pos(old, id) != json_pos(next, id)) && json_pos(next, id).is_some() {
            lookup.insert(id.clone(), json_pos(next, id).unwrap());
        }
    }
    for (id, at) in network_layout::layout(&next_graph, |id| lookup.get(id).copied()) {
        next["nodes"][&id]["pos"] = serde_json::json!([at.x, at.y]);
    }
    crate::graph_edit::validated(before).ok()
}

pub fn layout_changed(old: &serde_json::Value, next: &serde_json::Value) -> bool {
    let positions = |doc: &serde_json::Value| -> HashMap<String, Pos2> {
        doc["nodes"].as_object().into_iter().flat_map(|nodes| nodes.keys())
            .filter_map(|id| json_pos(doc, id).map(|at| (id.clone(), at))).collect()
    };
    positions(old) != positions(next)
}

/// Discard stale canvas overrides after a document layout change or history edit.
pub fn restore_positions(ctx: &egui::Context, path: &Path, doc: &serde_json::Value) {
    ctx.data_mut(|d| {
        let id = state_id(path);
        let mut state = d.get_persisted::<State>(id).unwrap_or_default();
        state.moved.clear();
        state.springs = None;
        state.tidied = false;
        if let Ok(graph) = minihou_core::Graph::from_value(doc) {
            state.moved = positions_for(&state, &graph, doc).into_iter().collect();
        }
        d.insert_persisted(id, state);
    });
}

pub(super) fn snap(pos: Pos2, grid: f32, enabled: bool, invert: bool) -> Pos2 {
    if !(enabled ^ invert) || grid <= 0.0 { return pos; }
    pos2((pos.x / grid).round() * grid, (pos.y / grid).round() * grid)
}

#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub(super) struct Guides { pub(super) x: Option<f32>, pub(super) y: Option<f32> }
pub(super) fn align_to_cards(pos: Pos2, peers: &[Pos2], size: Vec2, tolerance: f32) -> (Pos2, Guides) {
    let mut best_x: Option<(f32, f32, f32)> = None;
    let mut best_y: Option<(f32, f32)> = None;
    for peer in peers {
        for peer_offset in [0.0, size.x * 0.5, size.x] {
            for own_offset in [0.0, size.x * 0.5, size.x] {
                let guide = peer.x + peer_offset;
                let distance = (pos.x + own_offset - guide).abs();
                if distance <= tolerance && best_x.is_none_or(|(d, _, _)| distance < d) {
                    best_x = Some((distance, guide - own_offset, guide));
                }
            }
        }
        let distance = (pos.y - peer.y).abs();
        if distance <= tolerance && best_y.is_none_or(|(d, _)| distance < d) { best_y = Some((distance, peer.y)); }
    }
    let next = pos2(best_x.map_or(pos.x, |(_, x, _)| x), best_y.map_or(pos.y, |(_, y)| y));
    (next, Guides { x: best_x.map(|(_, _, guide)| guide), y: best_y.map(|(_, y)| y) })
}

pub(super) fn json_pos(doc: &serde_json::Value, id: &str) -> Option<Pos2> {
    let a = doc.get("nodes")?.get(id)?.get("pos")?.as_array()?;
    let x = a.first()?.as_f64()? as f32;
    let y = a.get(1)?.as_f64()? as f32;
    (x.is_finite() && y.is_finite()).then_some(pos2(x, y))
}
pub(super) fn content_bounds(positions: &[(String, Pos2)]) -> Rect {
    positions.iter().fold(Rect::NOTHING, |r, (_, at)| r.union(Rect::from_min_size(*at, CARD)))
}
pub(super) fn frame_all(state: &mut State, canvas: Rect, bounds: Rect) {
    if bounds == Rect::NOTHING { state.zoom = 1.0; state.pan = Vec2::ZERO; return; }
    let size = bounds.size() + vec2(48.0, 48.0);
    state.zoom = (canvas.width() / size.x).min(canvas.height() / size.y).clamp(0.25, 2.0);
    state.pan = canvas.size() * 0.5 - bounds.center().to_vec2() * state.zoom;
}
pub(super) fn screen(canvas: Rect, state: &State, at: Pos2) -> Pos2 { canvas.min + state.pan + at.to_vec2() * state.zoom }

