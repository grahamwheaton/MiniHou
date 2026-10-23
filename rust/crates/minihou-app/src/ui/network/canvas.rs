use super::*;

pub(super) fn canvas_ui(ui: &egui::Ui, r: Rect, v: &View, out: &mut Actions, state: &mut State, id: Id) {
    let ctx = ui.ctx();
    if v.cooking.cooking() { ctx.request_repaint_after(std::time::Duration::from_millis(16)); }
    let pointer = ctx.input(|i| i.pointer.hover_pos());
    let over = pointer.is_some_and(|at| r.contains(at));
    state.canvas = Some(r);
    let mut positions = node_positions(state, v);
    if let Some(focus) = state.focus.take() {
        if let Some((_,at)) = positions.iter().find(|(node,_)| *node == focus) {
            state.pan = r.center().to_vec2() - r.min.to_vec2() - (*at + CARD * 0.5).to_vec2() * state.zoom;
        }
    }
    // Cache automatic positions too: adding a root must not shuffle the existing layer.
    state.moved.retain(|node, _| v.graph.nodes.iter().any(|(id, _)| id == node));
    for (node, at) in &positions { state.moved.insert(node.clone(), *at); }

    let layout_frame = state.layout.is_some();
    if let Some(layout) = &state.layout {
        let cancel = ctx.input(|i| i.key_pressed(egui::Key::Escape) || i.pointer.button_pressed(PointerButton::Secondary));
        let confirm = ctx.input(|i| i.key_pressed(egui::Key::Enter) || i.pointer.button_pressed(PointerButton::Primary));
        if cancel {
            for (node, at) in &layout.selected { state.moved.insert(node.clone(), *at); }
            state.layout = None;
        } else {
            let pointer = pointer.unwrap_or(layout.anchor);
            let changed = position::layout_positions(layout.kind, &layout.selected,
                (pointer - layout.anchor) / state.zoom.max(0.25));
            for (node, at) in &changed { state.moved.insert(node.clone(), *at); }
            for (node, at) in &mut positions {
                if let Some((_, moved)) = changed.iter().find(|(id, _)| id == node) { *at = *moved; }
            }
            if confirm {
                let mut next = layout.before.clone();
                for (node, at) in changed { next["nodes"][&node]["pos"] = serde_json::json!([at.x, at.y]); }
                if next != layout.before { out.push(UiAction::NetworkGesture {
                    expected: v.doc.clone(), before: layout.before.clone(), next }); }
                state.layout = None;
            } else { ctx.request_repaint(); }
        }
    }

    if !state.physics { state.springs = None; }
    if let Some(springs) = &mut state.springs {
        let snappiness = ctx.data(|d| d.get_temp::<f32>(Id::new("node_physics_snappiness"))).unwrap_or(0.7);
        if springs.step_with_snappiness(&mut positions, ctx.input(|i| i.stable_dt), snappiness) {
            ctx.request_repaint();
        } else if state.drag.is_none() { state.springs = None; }
        for (node, at) in &positions { state.moved.insert(node.clone(), *at); }
    }
    let bounds = content_bounds(&positions);
    if state.zoom < 0.0 || (over && ctx.input(|i| i.key_pressed(egui::Key::Home))) { frame_all(state, r, bounds); }
    if !ctx.input(|i| i.pointer.primary_down()) { state.alt_pan = false; }
    if over && ctx.input(|i| i.pointer.button_pressed(PointerButton::Primary) && i.modifiers.alt) {
        let on_card = pointer.is_some_and(|at| positions.iter().any(|(_, pos)| Rect::from_min_size(screen(r, state, *pos), CARD * state.zoom).contains(at)));
        if !on_card { state.alt_pan = true; }
    }
    if over {
        let (zoom_delta, scroll, delta, dragging, ctrl) = ctx.input(|i| (i.zoom_delta(), i.smooth_scroll_delta, i.pointer.delta(),
            i.pointer.middle_down() || state.alt_pan || (i.key_down(egui::Key::Space) && i.pointer.primary_down()), i.modifiers.ctrl));
        let trackpad_pan = !ctrl && scroll.x != 0.0 && scroll.y.abs() < 50.0;
        let factor = if zoom_delta != 1.0 { zoom_delta } else if !trackpad_pan && scroll.y != 0.0 { (scroll.y * if ctrl { 0.01 } else { 0.0015 }).exp() } else { 1.0 };
        if factor != 1.0 {
            let at = pointer.unwrap_or(r.center());
            let old = state.zoom;
            state.zoom = (old * factor).clamp(0.25, 2.0);
            state.pan = (at - r.min) - (at - r.min - state.pan) * (state.zoom / old);
        } else if trackpad_pan { state.pan += scroll; }
        if dragging { state.pan += delta; }
    }
    let mut tool_dropped = false;
    if over && ctx.input(|i| i.pointer.any_released()) {
        if let Some(tool) = egui::DragAndDrop::take_payload::<crate::ui::ToolDrag>(ctx) {
            let at = shelf_drop_position(pointer.unwrap(), r, state.pan, state.zoom, state.snap, ctx.input(|i| i.modifiers.ctrl));
            let mut action = crate::ui::shelf::dropped_tool(&tool, at);
            if let Some((added, node)) = crate::graph_edit::add(v.doc, &tool.0, None) {
                if let UiAction::AddNetworkNode { wire, .. } = &mut action {
                    *wire = insertion_candidate_at_point(&added, &node, pointer.unwrap(), &wire_curves(r, state, v, &positions));
                }
            }
            out.push(action);
            state.menu = None;
            tool_dropped = true;
        }
    }
    let secondary_pressed = over && ctx.input(|i| i.pointer.button_pressed(PointerButton::Secondary));
    // Ctrl-RMB drag cuts noodles; Shift-RMB stays free for selection.
    let cut_mod = ctx.input(|i| i.modifiers.ctrl);
    if secondary_pressed && cut_mod && state.menu.is_none() && state.wire.is_none() {
        state.cut = pointer.map(Cut::new);
        state.box_start = None;
    }
    if pointer.is_none() || ctx.input(|i| i.key_pressed(egui::Key::Escape) || !i.focused) {
        state.cut = None;
        state.box_start = None;
    }
    // Only Ctrl-RMB is deferred until release so an ordinary RMB still opens
    // its context menu immediately. A sub-threshold Ctrl-RMB is a normal click.
    let mut shifted_click = None;
    let drawn_wires = wire_curves(r, state, v, &positions);
    if let Some(cut) = &mut state.cut {
        if let Some(at) = pointer {
            if cut_mod || cut.active { cut.update(at, &drawn_wires); }
        }
        if ctx.input(|i| i.pointer.button_released(PointerButton::Secondary)) {
            let cut = state.cut.take().unwrap();
            if cut.active {
                if !cut.hits.is_empty() { out.push(UiAction::DisconnectWires(cut.hits)); }
            } else { shifted_click = cut.points.first().copied(); }
        } else if !ctx.input(|i| i.pointer.secondary_down()) {
            state.cut = None;
        }
    }
    let right_click = ((secondary_pressed && !cut_mod) || shifted_click.is_some()) && state.wire.is_none();
    let tab = over && state.menu.is_none() && ctx.input(|i| i.key_pressed(egui::Key::Tab));
    if !layout_frame && (right_click || tab) {
        let at = shifted_click.or(pointer).unwrap_or(r.center());
        let target = if right_click { positions.iter().rev().find(|(_, pos)| Rect::from_min_size(screen(r, state, *pos), CARD * state.zoom).contains(at))
            .map(|(id, _)| id.clone()) } else { None };
        let zoom = if state.zoom <= 0.0 { 1.0 } else { state.zoom };
        let graph_pos = ((at - r.min - state.pan) / zoom).to_pos2();
        if tab || target.is_none() { state.add_pos = Some(graph_pos); }
        let menu = if tab { network_menu::MenuState::tab(at) }
            else { network_menu::MenuState::new(at, target, false) };
        state.menu = Some(menu.with_canvas_pos(graph_pos));
        if tab { ctx.input_mut(|i| i.consume_key(egui::Modifiers::default(), egui::Key::Tab)); }
    }
    let p = ui.painter().with_clip_rect(r);
    p.rect_filled(r, 0.0, v.theme.base);
    grid(&p, r, state, v.theme.grid);
    let ports = canvas_ports(v, &positions);
    let hit_port = pointer.filter(|at| r.contains(*at)).and_then(|at| ports.iter().rev()
        .find(|(_, world)| screen(r, state, *world).distance(at) <= (9.0 * state.zoom).max(6.0)));
    if !layout_frame && state.menu.is_none() && ctx.input(|i| i.pointer.button_pressed(PointerButton::Primary)) {
        if let Some((port, world)) = hit_port {
            state.wire_output_index = if matches!(port, AddConnection::Output(_)) {
                ports.iter().filter(|(candidate, _)| candidate == port)
                    .position(|(_, position)| position == world)
                    .and_then(|position| match port { AddConnection::Output(node) => output_index(&v.doc["nodes"][node],position), _ => None })
                    .unwrap_or(0)
            } else { 0 };
            state.pickup = match port {
                AddConnection::Input { node, port } => crate::graph_edit::WirePickup::begin(v.doc, node, port),
                _ => None,
            };
            if let Some(pickup) = &state.pickup {
                state.wire_output_index = pickup.source_output(v.doc).unwrap_or(0);
            }
            state.wire = Some(state.pickup.as_ref().map_or_else(|| port.clone(), |p| AddConnection::Output(p.original.from.clone())));
            state.box_start = None;
            state.drag = None;
            state.springs = None;
        }
    }
    let blocked = layout_frame || tool_dropped || state.menu.is_some() || state.wire.is_some() || state.cut.is_some();
    if ctx.input(|i| i.key_pressed(egui::Key::Escape) || !i.focused || i.pointer.button_pressed(PointerButton::Secondary)) {
        state.wire = None;
        state.pickup = None;
    }
    if state.wire.is_some() && ctx.input(|i| i.pointer.button_released(PointerButton::Primary)) {
        let source = state.wire.take().unwrap();
        let source_output = std::mem::take(&mut state.wire_output_index);
        let subnet_target = if hit_port.is_none() { pointer.filter(|at| r.contains(*at)).and_then(|at| positions.iter().rev()
            .find(|(node, pos)| v.doc["nodes"][node]["type"] == "subnet"
                && Rect::from_min_size(screen(r, state, *pos), CARD * state.zoom).contains(at))
            .map(|(node, _)| node.clone())) } else { None };
        if let Some(subnet) = subnet_target {
            let next = if let Some(pickup) = state.pickup.take() {
                pickup.release(v.doc, None).and_then(|base|
                    crate::subnet_edit::add_output(&base, &subnet, &pickup.original.to, &pickup.original.port))
            } else {
                match source {
                    AddConnection::Output(from) => crate::subnet_edit::add_input_from(v.doc, &subnet, &from, source_output),
                    AddConnection::Input { node, port } => crate::subnet_edit::add_output(v.doc, &subnet, &node, &port),
                }
            };
            if let Some(next) = next { out.push(UiAction::NetworkGesture { expected: v.doc.clone(), before: v.doc.clone(), next }); }
        } else if let Some(pickup) = state.pickup.take() {
            let target = hit_port.map(|(target, _)| target.clone()).or_else(|| {
                let at = pointer.filter(|at| r.contains(*at))?;
                let (node, _) = positions.iter().rev().find(|(_, pos)| Rect::from_min_size(screen(r, state, *pos), CARD * state.zoom).contains(at))?;
                let (_, def) = v.graph.nodes.iter().find(|(id, _)| id == node)?;
                let port = minihou_core::nodes::get(&def.node_type)?.inputs.first()?;
                Some(AddConnection::Input { node: node.clone(), port: port.name.into() })
            });
            let next = match target {
                Some(AddConnection::Input { node, port }) if v.doc["nodes"][&node]["type"] == "subnet" && port.starts_with("in") =>
                    pickup.release(v.doc, None).and_then(|base| crate::subnet_edit::connect_input_from(&base, &node, &port, &pickup.original.from, source_output)),
                Some(AddConnection::Input { node, port }) => pickup.release(v.doc, Some((&node, &port))),
                None if over => pickup.release(v.doc, None),
                _ => None,
            };
            if let Some(next) = next { out.push(UiAction::NetworkGesture { expected: v.doc.clone(), before: v.doc.clone(), next }); }
        } else if let Some((target, target_world)) = hit_port {
            let target_output = if matches!(target, AddConnection::Output(_)) {
                ports.iter().filter(|(candidate, _)| candidate == target)
                    .position(|(_, position)| position == target_world)
                    .and_then(|position| match target { AddConnection::Output(node) => output_index(&v.doc["nodes"][node],position), _ => None })
                    .unwrap_or(0)
            } else { 0 };
            let subnet_connection = match (&source, target) {
                (AddConnection::Output(from), AddConnection::Input { node, port })
                    if v.doc["nodes"][node]["type"] == "subnet" && port.starts_with("in") =>
                    crate::subnet_edit::connect_input_from(v.doc, node, port, from, source_output),
                (AddConnection::Input { node, port }, AddConnection::Output(from))
                    if v.doc["nodes"][node]["type"] == "subnet" && port.starts_with("in") =>
                    crate::subnet_edit::connect_input_from(v.doc, node, port, from, target_output),
                _ => None,
            };
            if let Some(next) = subnet_connection { out.push(UiAction::NetworkGesture { expected: v.doc.clone(), before: v.doc.clone(), next }); }
            else if let (AddConnection::Output(from), AddConnection::Input { node, port }) = (&source, target) {
                if source_output > 0 {
                    if let Some(next) = crate::graph_edit::connect_output(v.doc, from, source_output, node, Some(port)) {
                        out.push(UiAction::NetworkGesture { expected: v.doc.clone(), before: v.doc.clone(), next });
                    }
                } else if let Some(action) = wire_connection(&source, target) { out.push(action); }
            }
            else if let (AddConnection::Input { node, port }, AddConnection::Output(from)) = (&source, target) {
                if target_output > 0 {
                    if let Some(next) = crate::graph_edit::connect_output(v.doc, from, target_output, node, Some(port)) {
                        out.push(UiAction::NetworkGesture { expected: v.doc.clone(), before: v.doc.clone(), next });
                    }
                } else if let Some(action) = wire_connection(&source, target) { out.push(action); }
            }
            else if let Some(action) = wire_connection(&source, target) { out.push(action); }
        } else if let Some(at) = pointer.filter(|at| r.contains(*at)) {
            let on_card = positions.iter().any(|(_, pos)| Rect::from_min_size(screen(r, state, *pos), CARD * state.zoom).contains(at));
            let on_map = Rect::from_min_size(pos2(r.right() - 106.0, r.bottom() - 130.0), vec2(96.0, 120.0)).contains(at);
            if !on_card && !on_map {
                let world = ((at - r.min - state.pan) / state.zoom).to_pos2();
                let world = snap(world - CARD * 0.5, 32.0, state.snap, ctx.input(|i| i.modifiers.ctrl));
                state.menu = Some(network_menu::MenuState::connected(at, source, world));
            }
        }
    }
    // Pointer disappearance/cancellation must not leave a stale noodle.
    if !ctx.input(|i| i.pointer.primary_down()) || pointer.is_none() { state.wire = None; state.pickup = None; }
    let sense = if blocked { Sense::hover() } else { Sense::click_and_drag() };
    let background = ui.interact(r, id.with("box_selection"), sense);
    let background_hit_card = pointer.is_some_and(|at| positions.iter().any(|(_, pos)| Rect::from_min_size(screen(r, state, *pos), CARD * state.zoom).contains(at)));
    if background.double_clicked() && !background_hit_card {
        let at = pointer.unwrap_or(r.center());
        let zoom = if state.zoom <= 0.0 { 1.0 } else { state.zoom };
        let graph_pos = ((at - r.min - state.pan) / zoom).to_pos2();
        state.add_pos = Some(graph_pos);
        state.menu = Some(network_menu::MenuState::new(at, None, true).with_canvas_pos(graph_pos));
    }
    if background.drag_started_by(PointerButton::Primary) && !state.alt_pan && !background_hit_card {
        state.box_start = ctx.input(|i| i.pointer.press_origin());
        state.box_op = ctx.input(|i| SelectionOp::modifiers(i.modifiers.shift, i.modifiers.ctrl));
    }
    if background.clicked() && !background_hit_card && !blocked && hit_port.is_none() {
        let hit = pointer.and_then(|at| drawn_wires.iter().rev().find(|(_, points)|
            points.windows(2).any(|w| network_wires::distance(at,w[0],w[1]) <= 6.0)));
        let op = ctx.input(|i| SelectionOp::modifiers(i.modifiers.shift, i.modifiers.ctrl));
        network_wires::select(&mut state.selected_wires, hit.map(|(wire,_)| wire.clone()), op);
        if matches!(op, SelectionOp::Replace) { out.push(UiAction::SelectSet { ids: vec![], extend: false }); }
    }
    if positions.is_empty() {
        p.text(pos2(r.center().x, r.center().y - 12.0), Align2::CENTER_CENTER, "The network is empty", FontId::proportional(15.0), v.theme.text);
        let hint = p.layout("Open a graph file, press Tab, or pick a tool from the shelf above.".into(), FontId::proportional(12.0), v.theme.dim, (r.width() - 80.0).max(80.0));
        p.galley(pos2(r.center().x - hint.size().x * 0.5, r.center().y + 5.0), hint, v.theme.dim);
    } else {
        if state.show_links { dependency_links(&p,r,state,v,&positions); }
        wires(&p, r, state, v, &positions);
        let mut guide = None;
        let ref_links = minihou_app::value_link_ui::links(v.doc);
        for (node_id, at) in &positions {
            let Some((_, node)) = v.graph.nodes.iter().find(|(n, _)| n == node_id) else { continue };
            let card = Rect::from_min_size(screen(r, state, *at), CARD * state.zoom);
            if !card.intersects(r) { continue; }
            let face = Rect::from_min_max(pos2(card.left() + 20.0 * state.zoom, card.top()), pos2(card.right() - 20.0 * state.zoom, card.bottom()));
            let response = ui.interact(face, id.with(("node", node_id)), sense)
                .on_hover_text(format!("{} · {} · {} params", node_id, node.node_type, node.params.as_ref().map_or(0, |p| p.len())));
            node_context_menu(&response, node_id, v, out);
            if response.double_clicked() {
                if node.node_type == "subnet" { state.subnet_path.push(node_id.clone()); reset_scope(state); }
                else { out.push(UiAction::SetDisplay(node_id.clone())); }
            }
            else if response.clicked() {
                state.selected_wires.clear();
                out.push(if ctx.input(|i| i.modifiers.shift || i.modifiers.ctrl) { UiAction::ToggleSelect(node_id.clone()) } else { UiAction::Select(node_id.clone()) });
            }
            if response.drag_started_by(PointerButton::Primary) {
                state.selected_wires.clear();
                if !v.selected_set.contains(node_id) { out.push(UiAction::Select(node_id.clone())); }
                state.drag = Some((node_id.clone(), *at)); state.springs = None; state.box_start = None;
                let mut before = v.doc.clone();
                for (id, at) in &positions { before["nodes"][id]["pos"] = serde_json::json!([at.x, at.y]); }
                state.drag_before = Some(before);
                state.drag_physics = false;
                state.wiggle = Wiggle::default();
                if let Some(origin) = ctx.input(|i| i.pointer.press_origin()) {
                    state.wiggle.update(origin.x, ctx.input(|i| i.time));
                }
            }
            if response.dragged_by(PointerButton::Primary) && !ctx.input(|i| i.key_down(egui::Key::Space)) {
                if let Some(pointer) = response.interact_pointer_pos() {
                    if state.wiggle.update(pointer.x, ctx.input(|i| i.time)) {
                        out.push(UiAction::DetachNode(node_id.clone()));
                        state.springs = None;
                    }
                }
                if !state.wiggle.fired && state.physics && ctx.input(|i| i.modifiers.shift) {
                    state.drag_physics = true;
                    if state.springs.is_none() {
                        let mut pinned = if v.selected_set.contains(node_id) { v.selected_set.clone() } else { Default::default() };
                        pinned.insert(node_id.clone());
                        state.springs = Some(network_physics::Springs::new(v.graph, &positions, pinned));
                    }
                    // The solver runs at the start of the next canvas frame. Schedule it
                    // even if this was the final pointer event before release.
                    ctx.request_repaint();
                } else { state.springs = None; }
                let delta = ctx.input(|i| i.pointer.delta()) / state.zoom;
                let raw = if let Some((active, raw)) = &mut state.drag {
                    if active == node_id { *raw += delta; *raw } else { *at + delta }
                } else { *at + delta };
                let invert = ctx.input(|i| i.modifiers.ctrl);
                let enabled = state.snap ^ invert;
                let base = snap(raw, 32.0, state.snap, invert);
                let peers: Vec<_> = positions.iter().filter(|(id, _)| id != node_id).map(|(_, at)| *at).collect();
                let (next, lines) = if enabled { align_to_cards(base, &peers, CARD, 6.0 / state.zoom) } else { (base, Guides::default()) };
                guide = Some(lines);
                state.moved.insert(node_id.clone(), next);
                if v.selected_set.contains(node_id) {
                    let offset = next - *at;
                    for (peer, peer_at) in &positions {
                        if peer != node_id && v.selected_set.contains(peer) { state.moved.insert(peer.clone(), *peer_at + offset); }
                    }
                }
            }

            for flag in 0..4 {
                if ui.interact(flag_rect(card, flag), id.with(("flag", node_id, flag)), if blocked { Sense::hover() } else { Sense::click() })
                    .on_hover_text(["Mute", "Locked", "Template", "Visible"][flag]).clicked() {
                    if v.selected_set.contains(node_id) && v.selected_set.len() > 1 {
                        match flag {
                            0 => out.push(UiAction::Command("mute_flag".into())),
                            1 => for peer in v.selected_set { out.push(UiAction::ToggleLocked(peer.clone())); },
                            2 => for peer in v.selected_set { out.push(UiAction::ToggleTemplate(peer.clone())); },
                            _ => out.push(UiAction::Command("display_flag".into())),
                        }
                    } else {
                        out.push(if flag == 3 && ctx.input(|i| i.modifiers.shift) { UiAction::ToggleDisplay(node_id.clone()) } else { flag_action(flag, node_id, node.bypass) });
                    }
                }
            }
            let count = ref_links.iter().filter(|l| &l.source == node_id || &l.target == node_id).count();
            let external_count = v.doc["exposed"].as_array().into_iter().flatten().filter(|entry| entry["node"] == node_id.as_str()).count();
            let broken = ref_links.iter().any(|l| l.broken && &l.target == node_id);
            card_ui(&p, card, node_id, node, v, state.zoom, count + external_count, exposed_to_blender(v.doc, node_id), broken);
        }
        if let Some(lines) = guide {
            if let Some(x) = lines.x { let x = screen(r, state, pos2(x, 0.0)).x; p.line_segment([pos2(x, r.top()), pos2(x, r.bottom())], Stroke::new(1.0_f32, v.theme.accent)); }
            if let Some(y) = lines.y { let y = screen(r, state, pos2(0.0, y)).y; p.line_segment([pos2(r.left(), y), pos2(r.right(), y)], Stroke::new(1.0_f32, v.theme.accent)); }
        }
        if !blocked { minimap(ui, r, state, v, &positions, id); }
    }
    let cancelled = pointer.is_none() || ctx.input(|i| i.key_pressed(egui::Key::Escape) || !i.focused || i.pointer.button_pressed(PointerButton::Secondary));
    if cancelled && state.drag.is_some() {
        if let Some(before) = state.drag_before.take() {
            for (id, _) in &positions { if let Some(at) = json_pos(&before, id) { state.moved.insert(id.clone(), at); } }
        }
        state.drag = None;
        state.springs = None;
    }
    let current_positions: Vec<_> = positions.iter().map(|(id, at)| (id.clone(), state.moved.get(id).copied().unwrap_or(*at))).collect();
    let current_wires = wire_curves(r, state, v, &current_positions);
    let candidate = state.drag.as_ref().filter(|(node, _)| !state.drag_physics && !state.wiggle.fired &&
        !ctx.input(|i| i.modifiers.shift || i.modifiers.alt) && (!v.selected_set.contains(node) || v.selected_set.len() <= 1))
        .and_then(|(node, _)| {
            let at = current_positions.iter().find(|(id, _)| id == node)?.1;
            pointer.and_then(|at| insertion_candidate_at_point(v.doc, node, at, &current_wires))
                .or_else(|| insertion_candidate(v.doc, node, Rect::from_min_size(screen(r, state, at), CARD * state.zoom), &current_wires))
        });
    if let Some(wire) = &candidate {
        if let Some((_, points)) = current_wires.iter().find(|(w, _)| w == wire) {
            p.add(egui::Shape::line(points.clone(), Stroke::new(3.5_f32, v.theme.accent)));
        }
    }
    if state.drag.is_some() && ctx.input(|i| i.pointer.button_released(PointerButton::Primary)) {
        if let (Some(wire), Some(before), Some((node, _))) = (candidate, state.drag_before.take(), state.drag.as_ref()) {
            let mut moved = v.doc.clone();
            for (id, at) in &current_positions { moved["nodes"][id]["pos"] = serde_json::json!([at.x, at.y]); }
            if let Some(next) = crate::graph_edit::insert_existing(&moved, node, &wire) {
                out.push(UiAction::NetworkGesture { expected: v.doc.clone(), before, next });
            }
        }
        state.drag = None;
        state.drag_before = None;
    }
    // Shelf and Tab-menu drag payloads share the same preview and drop edit.
    if let (Some(tool), Some(at)) = (egui::DragAndDrop::payload::<crate::ui::ToolDrag>(ctx), pointer) {
        if over {
            if let Some((value, node)) = crate::graph_edit::add(v.doc, &tool.0, None) {
                if let Some(wire) = insertion_candidate_at_point(&value, &node, at, &drawn_wires) {
                    if let Some((_, points)) = drawn_wires.iter().find(|(w, _)| w == &wire) {
                        p.add(egui::Shape::line(points.clone(), Stroke::new(3.5_f32, v.theme.accent)));
                    }
                }
            }
        }
        let rect = if over {
            let world = shelf_drop_position(at, r, state.pan, state.zoom, state.snap, ctx.input(|i| i.modifiers.ctrl));
            Rect::from_min_size(screen(r, state, world), CARD * state.zoom)
        } else { Rect::from_center_size(at, CARD) };
        let ghost = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, id.with("tool_ghost")));
        ghost_card(&ghost, rect, &tool.0, v, if over { state.zoom } else { 1.0 });
    }
    if let (Some(start), Some(end)) = (state.box_start, pointer) {
        if background.dragged_by(PointerButton::Primary) {
            let box_rect = Rect::from_two_pos(start, end);
            p.rect_stroke(box_rect, 0.0, Stroke::new(1.0_f32, v.theme.accent), StrokeKind::Inside);
        }
        if background.drag_stopped_by(PointerButton::Primary) {
            let box_rect = Rect::from_two_pos(start, end);
            let hits = positions.iter().filter(|(_, at)| Rect::from_min_size(screen(r, state, *at), CARD * state.zoom).intersects(box_rect)).map(|(id, _)| id.clone());
            let mut nodes = v.selected_set.clone();
            network_wires::select(&mut nodes, hits, state.box_op);
            out.push(UiAction::SelectSet { ids: nodes.into_iter().collect(), extend: false });
            let hits = drawn_wires.iter().filter(|(_, points)| network_wires::in_box(points, box_rect)).map(|(wire,_)| wire.clone());
            network_wires::select(&mut state.selected_wires, hits, state.box_op);
            state.box_start = None;
        }
    }
    if let Some(cut) = state.cut.as_ref().filter(|cut| cut.active) {
        // Thin white underlay and red freehand trace, with red cut candidates.
        p.add(egui::Shape::line(cut.points.clone(), Stroke::new(2.0_f32, Color32::WHITE)));
        p.add(egui::Shape::line(cut.points.clone(), Stroke::new(1.0_f32, Color32::from_rgb(255,100,100))));
    }
    let live = state.wire.as_ref().map(|port| (port, pointer))
        .or_else(|| { let menu = state.menu.as_ref()?; menu.connect_to.as_ref().map(|(port, _)| (port, Some(menu.at()))) });
    if let Some((port, Some(end))) = live {
        let output_index = if state.wire.is_some() && matches!(port, AddConnection::Output(_)) { state.wire_output_index } else { 0 };
        if let Some((_, start)) = ports.iter().filter(|(candidate, _)| candidate == port).nth(output_index) {
            let start = screen(r, state, *start);
            let (a, b) = if matches!(port, AddConnection::Output(_)) { (start, end) } else { (end, start) };
            noodle(&p, a, b, v.theme.accent);
        }
    }
    canvas_menu(ctx, id, state, v, out, r, &positions);
    p.text(pos2(r.left() + 10.0, r.bottom() - 8.0), Align2::LEFT_BOTTOM, "Tab add node · Home frame all · Del delete", FontId::proportional(11.0), v.theme.faint);
}

fn grid(p: &egui::Painter, r: Rect, state: &State, color: Color32) {
    let step = 32.0 * state.zoom;
    let origin = r.min + state.pan;
    let mut x = r.left() + (origin.x - r.left()).rem_euclid(step);
    while x < r.right() { p.line_segment([pos2(x.round(), r.top()), pos2(x.round(), r.bottom())], Stroke::new(1.0_f32, color)); x += step; }
    let mut y = r.top() + (origin.y - r.top()).rem_euclid(step);
    while y < r.bottom() { p.line_segment([pos2(r.left(), y.round()), pos2(r.right(), y.round())], Stroke::new(1.0_f32, color)); y += step; }
}

