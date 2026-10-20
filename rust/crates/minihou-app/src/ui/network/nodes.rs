use super::*;

pub(super) fn flag_rect(card: Rect, flag: usize) -> Rect {
    let w = card.width() * 20.0 / CARD.x;
    let x = if flag < 2 { card.left() } else { card.right() - w };
    let y = if flag.is_multiple_of(2) { card.top() } else { card.center().y };
    Rect::from_min_size(pos2(x, y), vec2(w, card.height() * 0.5))
}

pub(super) fn flag_action(flag: usize, id: &str, bypass: bool) -> UiAction {
    match flag {
        0 => UiAction::Edit { edit: Edit::Bypass { node: id.into(), flag: !bypass }, continuous: false },
        1 => UiAction::ToggleLocked(id.into()),
        2 => UiAction::ToggleTemplate(id.into()),
        _ => UiAction::SetDisplay(id.into()),
    }
}

pub(super) fn exposed_to_blender(doc: &serde_json::Value, id: &str) -> bool {
    minihou_app::exposed_nodes(doc).contains(id)
}

pub(super) fn card_ui(p: &egui::Painter, r: Rect, id: &str, node: &NodeDef, v: &View, zoom: f32, link_count: usize, external: bool, broken_link: bool) {
    let t = v.theme;
    let flags = v.flags_of(id);
    let unsupported = !v.runs_on_rust(&node.node_type);
    let error = v.cooking.error.as_deref().filter(|e| e.contains(&format!("node '{id}'")));
    let visible = minihou_app::display_nodes(v.doc).iter().any(|node| node == id);
    let waiting = if error.is_none() && !node.bypass { crate::ui::waiting::waiting_of(&v.cooking.stats, id) } else { None };
    let fill = if node.bypass { t.panel } else if error.is_some() { t.err_bg } else if visible { t.accent_bg } else { t.card };
    let border = if node.bypass { t.stroke } else if error.is_some() { t.err } else if waiting.is_some() { crate::ui::waiting::outline(t.warn) } else if unsupported { t.warn } else if flags.template { t.tpl } else if visible || v.cooking.cooking() { t.accent } else { t.stroke2 };
    p.rect_filled(r, 6.0 * zoom, fill);
    if node.bypass || unsupported || flags.template { dashed_rect(p, r, border); }
    else { p.rect_stroke(r, 6.0 * zoom, Stroke::new(1.0_f32, border), StrokeKind::Inside); }
    if v.selected_set.contains(id) { p.rect_stroke(r.expand(2.0), 8.0 * zoom, Stroke::new(2.0_f32, t.sel), StrokeKind::Outside); }
    let left = r.left() + 20.0 * zoom;
    let right = r.right() - 20.0 * zoom;
    for x in [left, right] { p.line_segment([pos2(x, r.top()), pos2(x, r.bottom())], Stroke::new(1.0_f32, t.stroke)); }
    for flag in 0..4 {
        let fr = flag_rect(r, flag);
        if !flag.is_multiple_of(2) { p.line_segment([pos2(fr.left(), fr.top()), pos2(fr.right(), fr.top())], Stroke::new(1.0_f32, t.stroke)); }
        let on = match flag { 0 => node.bypass, 1 => flags.locked, 2 => flags.template, _ => visible };
        if on { p.rect_filled(fr.shrink(1.0), 2.0 * zoom, [t.warn, t.dim, t.tpl, t.accent][flag]); }
        let ink = if on { if flag == 1 && !t.dark { t.card } else { t.sel } }
            else { t.dim.gamma_multiply(if v.selected == id { 0.85 } else { 0.5 }) };
        flag_icon(p, fr.center(), flag, ink, zoom);
    }
    let face = Rect::from_min_max(pos2(left + 10.0 * zoom, r.top() + 7.0 * zoom), pos2(right - 10.0 * zoom, r.bottom() - 7.0 * zoom));
    let text = if node.bypass { t.faint } else { t.text };
    let dim = if node.bypass { t.faint } else { t.dim };
    let cat = if node.bypass || unsupported { t.faint } else { t.category_color(&node.node_type) };
    let link_width = if link_count > 0 { 29.0 * zoom } else { 0.0 };
    let link_color = if external { Color32::from_rgb(234, 118, 0) }
        else if broken_link { Color32::from_rgb(235, 80, 80) } else { Color32::from_rgb(62, 190, 110) };
    if zoom >= 0.7 {
        p.rect_filled(Rect::from_min_size(pos2(face.left(), face.top() + 2.0 * zoom), vec2(10.0 * zoom, 10.0 * zoom)), 2.0 * zoom, cat);
        let lod_width = if node.node_type == "lod_bake" { 24.0 * zoom } else { 0.0 };
        let tag_width = lod_width + link_width;
        if link_count > 0 {
            p.text(pos2(face.right(), face.top()), Align2::RIGHT_TOP,
                if broken_link && !external { format!("⚠ {link_count}") } else { format!("↗ {link_count}") },
                FontId::proportional(10.0 * zoom), link_color);
        }
        if lod_width > 0.0 {
            let tag = Rect::from_min_size(pos2(face.right() - link_width - lod_width, face.top()), vec2(lod_width, 15.0 * zoom));
            p.rect_filled(tag, 3.0 * zoom, t.ok_bg);
            p.rect_stroke(tag, 3.0 * zoom, Stroke::new(1.0_f32, t.ok), StrokeKind::Inside);
            p.text(tag.center(), Align2::CENTER_CENTER, "LOD", FontId::monospace(10.0 * zoom), t.ok);
        }
        p.text(pos2(face.left() + 17.0 * zoom, face.top()), Align2::LEFT_TOP, ellipsis(id, face.width() - 17.0 * zoom - tag_width, 12.5 * zoom), FontId::monospace(12.5 * zoom), text);
        let (status, status_color) = node_status(v, id, node.bypass, unsupported, error.is_some());
        p.text(pos2(face.left(), face.bottom()), Align2::LEFT_BOTTOM, ellipsis(if node.node_type == "lod_bake" { "Nanite proxy" } else { &node.node_type }, 92.0 * zoom, 11.0 * zoom), FontId::proportional(11.0 * zoom), dim);
        p.text(pos2(face.right(), face.bottom()), Align2::RIGHT_BOTTOM, status, FontId::monospace(11.0 * zoom), status_color);
        if v.cooking.cooking() && !node.bypass {
            let time = p.ctx().input(|i| i.time as f32);
            let center = pos2(face.right() - 58.0 * zoom, face.bottom() - 5.0 * zoom);
            let arc: Vec<_> = (0..=12).map(|i| {
                let angle = time * std::f32::consts::TAU + i as f32 * std::f32::consts::PI * 1.5 / 12.0;
                center + vec2(angle.cos(), angle.sin()) * 4.5 * zoom
            }).collect();
            p.add(egui::Shape::line(arc, Stroke::new(1.8_f32 * zoom, t.accent)));
        }
    } else if zoom >= 0.4 {
        p.rect_filled(Rect::from_min_size(pos2(face.left(), r.center().y - 5.0 * zoom), vec2(10.0 * zoom, 10.0 * zoom)), 2.0, cat);
        p.text(pos2(face.left() + 16.0 * zoom, r.center().y), Align2::LEFT_CENTER, ellipsis(id, face.width() - 16.0 * zoom - link_width, 12.0), FontId::monospace(12.0), text);
        if link_count > 0 { p.text(pos2(face.right(), r.center().y), Align2::RIGHT_CENTER, format!("↗ {link_count}"), FontId::proportional(10.0 * zoom), link_color); }
    } else { p.rect_filled(r.shrink(2.0), 3.0, cat); }
    let inputs = input_slots(node);
    for index in 0..inputs.len() {
        p.circle(r.min + input_offset(index, inputs.len()) * zoom, 5.0 * zoom, t.base, Stroke::new(1.5_f32, t.stroke2));
    }
    if node.node_type != "output" { p.circle(pos2(r.center().x, r.bottom()), 5.0 * zoom, t.base, Stroke::new(1.5_f32, t.stroke2)); }
    if v.cooking.cooking() && !node.bypass { p.rect_filled(Rect::from_min_size(pos2(r.left() + 2.0, r.bottom() - 3.0), vec2(r.width() * 0.38, 2.0)), 0.0, t.accent); }
    if let Some(message) = error {
        let galley = p.layout(message.to_owned(), FontId::monospace(11.0 * zoom), t.text, r.width() - 16.0);
        let msg = Rect::from_min_size(pos2(r.left(), r.bottom() + 6.0 * zoom), vec2(r.width(), galley.size().y + 6.0));
        p.rect_filled(msg, 4.0, t.err_bg);
        p.rect_stroke(msg, 4.0, Stroke::new(1.0_f32, t.err), StrokeKind::Inside);
        p.galley(msg.left_top() + vec2(8.0, 3.0), galley, t.text);
    } else if let Some(message) = waiting {
        // Waiting for external data: a quiet note, never the red error box.
        let galley = p.layout(crate::ui::waiting::card_note(message), FontId::proportional(11.0 * zoom), t.dim, r.width() - 22.0);
        let msg = Rect::from_min_size(pos2(r.left(), r.bottom() + 6.0 * zoom), vec2(r.width(), galley.size().y + 6.0));
        p.rect_filled(msg, 4.0, t.panel);
        p.circle_filled(pos2(msg.left() + 8.0, msg.top() + 3.0 + galley.size().y / 2.0), 3.0, t.warn);
        p.galley(msg.left_top() + vec2(15.0, 3.0), galley, t.dim);
    }
}

pub(super) fn node_status(v: &View, id: &str, bypass: bool, unsupported: bool, error: bool) -> (String, Color32) {
    let t = v.theme;
    if bypass { return ("muted".into(), t.faint); }
    if unsupported { return ("unsupported".into(), t.warn); }
    if error { return ("failed".into(), t.err); }
    if crate::ui::waiting::waiting_of(&v.cooking.stats, id).is_some() { return (crate::ui::waiting::NODE_STATUS.into(), t.warn); }
    if v.cooking.cooking() { return ("cooking".into(), t.accent); }
    if v.cooking.stats.cached.iter().any(|n| n == id) { return ("cached".into(), t.dim); }
    if let Some((_, ms)) = v.cooking.stats.node_ms.iter().find(|(n, _)| n == id) {
        return (format!("{ms:.1} ms"), if *ms > 100.0 { t.warn } else { t.dim });
    }
    ("not cooked".into(), t.dim)
}
pub(super) fn ellipsis(text: &str, width: f32, font_size: f32) -> String {
    let count = (width / (font_size * 0.61)).floor().max(1.0) as usize;
    if text.chars().count() <= count { text.into() } else { format!("{}…", text.chars().take(count.saturating_sub(1)).collect::<String>()) }
}
pub(super) fn dashed_rect(p: &egui::Painter, r: Rect, color: Color32) {
    for (a, b) in [(r.left_top(), r.right_top()), (r.right_top(), r.right_bottom()), (r.right_bottom(), r.left_bottom()), (r.left_bottom(), r.left_top())] {
        let steps = (a.distance(b) / 6.0).ceil().max(1.0) as usize;
        for i in (0..steps).step_by(2) { p.line_segment([a.lerp(b, i as f32 / steps as f32), a.lerp(b, (i + 1).min(steps) as f32 / steps as f32)], Stroke::new(1.0_f32, color)); }
    }
}
pub(super) fn fit_icon(p: &egui::Painter, center: Pos2, color: Color32) {
    let s = Stroke::new(1.4_f32, color);
    for x in [-1.0, 1.0] { for y in [-1.0, 1.0] {
        let corner = center + vec2(6.0 * x, 6.0 * y);
        p.line_segment([corner, corner - vec2(3.0 * x, 0.0)], s);
        p.line_segment([corner, corner - vec2(0.0, 3.0 * y)], s);
    } }
}
pub(super) fn flag_icon(p: &egui::Painter, center: Pos2, flag: usize, color: Color32, zoom: f32) {
    let s = Stroke::new(1.3_f32, color);
    let at = |x: f32, y: f32| center + vec2(x * zoom, y * zoom);
    match flag {
        0 => { p.line_segment([at(-4.0, 0.0), at(3.0, 0.0)], s); p.line_segment([at(0.0, -3.0), at(3.0, 0.0)], s); p.line_segment([at(0.0, 3.0), at(3.0, 0.0)], s); }
        1 => { p.rect_stroke(Rect::from_min_max(at(-3.0, 0.0), at(3.0, 4.0)), 1.0, s, StrokeKind::Inside); p.line_segment([at(-2.0, 0.0), at(-2.0, -2.0)], s); p.line_segment([at(2.0, 0.0), at(2.0, -2.0)], s); p.line_segment([at(-2.0, -2.0), at(2.0, -2.0)], s); }
        2 => dashed_rect(p, Rect::from_min_max(at(-4.0, -4.0), at(4.0, 4.0)), color),
        _ => { p.add(egui::Shape::convex_polygon(vec![at(-4.0, 0.0), at(-2.0, -2.5), at(0.0, -3.0), at(2.0, -2.5), at(4.0, 0.0), at(2.0, 2.5), at(0.0, 3.0), at(-2.0, 2.5)], Color32::TRANSPARENT, s)); p.circle_filled(center, 1.2 * zoom, color); }
    }
}

pub(super) fn minimap(ui: &egui::Ui, canvas: Rect, state: &mut State, v: &View, positions: &[(String, Pos2)], id: Id) {
    let p = ui.painter().with_clip_rect(canvas);
    let bounds = content_bounds(positions);
    let map = Rect::from_min_size(pos2(canvas.right() - 106.0, canvas.bottom() - 130.0), vec2(96.0, 120.0));
    p.rect_filled(map, 4.0, v.theme.panel);
    p.rect_stroke(map, 4.0, Stroke::new(1.0_f32, v.theme.stroke), StrokeKind::Inside);
    let extent = bounds.expand(20.0);
    let scale = (92.0 / extent.width()).min(116.0 / extent.height());
    let origin = map.min + vec2(2.0, 2.0);
    let project = |point: Pos2| origin + (point - extent.min) * scale;
    for (node_id, at) in positions {
        if let Some((_, def)) = v.graph.nodes.iter().find(|(id, _)| id == node_id) {
            p.rect_filled(Rect::from_min_size(project(*at), CARD * scale), 2.0, v.theme.category_color(&def.node_type));
        }
    }
    let viewport = Rect::from_min_size(pos2(-state.pan.x / state.zoom, -state.pan.y / state.zoom), canvas.size() / state.zoom);
    let visible = Rect::from_min_max(project(viewport.min), project(viewport.max)).intersect(map.shrink(1.0));
    if visible.width() > 0.0 && visible.height() > 0.0 {
        p.rect_stroke(visible, 2.0, Stroke::new(1.0_f32, v.theme.accent), StrokeKind::Inside);
    }
    let response = ui.interact(map, id.with("minimap"), Sense::click_and_drag());
    if response.clicked() || response.dragged() {
        if let Some(at) = response.interact_pointer_pos() {
            let world = extent.min + (at - origin) / scale;
            state.pan = canvas.size() * 0.5 - world.to_vec2() * state.zoom;
        }
    }
}

