use super::*;

/// Slots include unused declared inputs; multi-input wires share their named port.
pub(super) fn input_slots(node: &NodeDef) -> Vec<String> {
    minihou_core::nodes::get(&node.node_type).map(|spec| spec.inputs.iter().flat_map(|port| {
        let count = node.inputs.iter().find(|(name, _)| name == port.name).map_or(0, |(_, refs)| refs.len()).max(1);
        std::iter::repeat_n(port.name.to_owned(), count)
    }).collect()).unwrap_or_default()
}

pub(super) fn input_offset(index: usize, count: usize) -> Vec2 {
    vec2(CARD.x * 0.5 + (index as f32 - (count.saturating_sub(1)) as f32 * 0.5) * 16.0, 0.0)
}

pub(super) fn canvas_ports(v: &View, positions: &[(String, Pos2)]) -> Vec<(AddConnection, Pos2)> {
    let mut ports = Vec::new();
    for (id, at) in positions {
        let Some((_, node)) = v.graph.nodes.iter().find(|(n, _)| n == id) else { continue };
        let inputs = input_slots(node);
        for (index, port) in inputs.iter().enumerate() {
            ports.push((AddConnection::Input { node: id.clone(), port: port.clone() }, *at + input_offset(index, inputs.len())));
        }
        if node.node_type != "output" { ports.push((AddConnection::Output(id.clone()), *at + vec2(CARD.x * 0.5, CARD.y))); }
    }
    ports
}

pub(super) fn wire_connection(a: &AddConnection, b: &AddConnection) -> Option<UiAction> {
    let (from, to, port) = match (a, b) {
        (AddConnection::Output(from), AddConnection::Input { node, port }) |
        (AddConnection::Input { node, port }, AddConnection::Output(from)) => (from, node, port),
        _ => return None,
    };
    (from != to).then(|| UiAction::ConnectNodes { from: from.clone(), to: to.clone(), port: Some(port.clone()) })
}

pub(super) fn noodle(p: &egui::Painter, a: Pos2, b: Pos2, color: Color32) {
    p.add(egui::Shape::line(network_wires::curve(a,b), Stroke::new(1.5_f32, color)));
}

pub(super) fn link_curve(a: Pos2, b: Pos2) -> Vec<Pos2> {
    let bend = ((b.x - a.x).abs() * 0.45).max(40.);
    let c = a + vec2(bend, 0.);
    let d = b - vec2(bend, 0.);
    (0..=32).map(|i| {
        let t = i as f32 / 32.; let u = 1. - t;
        pos2(u*u*u*a.x + 3.*u*u*t*c.x + 3.*u*t*t*d.x + t*t*t*b.x,
             u*u*u*a.y + 3.*u*u*t*c.y + 3.*u*t*t*d.y + t*t*t*b.y)
    }).collect()
}
pub(super) fn link_hit(points: &[Pos2], at: Pos2) -> bool {
    points.windows(2).any(|w| network_wires::distance(at,w[0],w[1]) <= 6.)
}
pub(super) fn dependency_links(p: &egui::Painter, canvas: Rect, state: &State, v: &View, positions: &[(String, Pos2)]) {
    let lookup: HashMap<&str,Pos2> = positions.iter().map(|(id,at)|(id.as_str(),*at)).collect();
    let pointer = p.ctx().input(|i| i.pointer.hover_pos());
    for link in minihou_app::value_link_ui::links(v.doc) {
        let Some(&target) = lookup.get(link.target.as_str()) else { continue; };
        let b = screen(canvas,state,target + vec2(0.,CARD.y*0.5));
        let a = lookup.get(link.source.as_str()).map(|at|screen(canvas,state,*at + vec2(CARD.x,CARD.y*0.5)))
            .unwrap_or(b - vec2(80. * state.zoom, 35. * state.zoom));
        let points = link_curve(a,b);
        let hot = v.selected == link.source || v.selected == link.target || pointer.is_some_and(|at| link_hit(&points,at));
        let color = if link.broken { Color32::from_rgb(239,68,68) } else { Color32::from_rgb(56,189,248) }.gamma_multiply(if hot { 0.95 } else { 0.25 });
        for (i,segment) in points.windows(2).enumerate() {
            if i % 4 < 2 { p.line_segment([segment[0],segment[1]],Stroke::new(if hot {2.} else {1.2},color)); }
        }
        if hot {
            let mid=points[16];
            p.text(mid,Align2::CENTER_BOTTOM,format!("{} → {}",link.source_param,link.target_param),FontId::proportional(10.),color);
        }
    }
}

#[cfg(test)] mod link_geometry_tests {
    use super::*;
    #[test] fn curved_link_hit_is_separate_from_card_and_noodle_geometry() {
        let points=link_curve(pos2(10.,20.),pos2(210.,70.));
        assert_eq!(points.first().copied(),Some(pos2(10.,20.)));
        assert_eq!(points.last().copied(),Some(pos2(210.,70.)));
        assert!(link_hit(&points,points[16]));
        assert!(!link_hit(&points,pos2(100.,300.)));
    }
}

pub(super) fn wire_curves(canvas: Rect, state: &State, v: &View, positions: &[(String, Pos2)]) -> Vec<(WireRef, Vec<Pos2>)> {
    curves_for(canvas, state, v.graph, positions)
}
pub(super) fn curves_for(canvas: Rect, state: &State, graph: &minihou_core::Graph, positions: &[(String, Pos2)]) -> Vec<(WireRef, Vec<Pos2>)> {
    let mut wires = Vec::new();
    let mut seen = BTreeSet::new();
    let lookup: HashMap<&str, Pos2> = positions.iter().map(|(id, at)| (id.as_str(), *at)).collect();
    for (target, def) in &graph.nodes {
        let Some(&end) = lookup.get(target.as_str()) else { continue };
        let slots = input_slots(def);
        let mut used = HashMap::<&str, usize>::new();
        for (port, source) in def.inputs.iter().flat_map(|(port, refs)| refs.iter().map(move |source| (port, source))) {
            let offset = used.entry(port).or_default();
            let Some(index) = slots.iter().position(|name| name == port).map(|i| i + *offset) else { continue };
            *offset += 1;
            let Some(&start) = lookup.get(source.as_str()) else { continue };
            let a = screen(canvas, state, start + vec2(CARD.x * 0.5, CARD.y));
            let b = screen(canvas, state, end + input_offset(index, slots.len()));
            let wire = WireRef { from: source.clone(), to: target.clone(), port: port.clone() };
            if seen.insert(wire.clone()) { wires.push((wire, network_wires::curve(a,b))); }
        }
    }
    wires
}

pub(super) fn wires(p: &egui::Painter, canvas: Rect, state: &State, v: &View, positions: &[(String, Pos2)]) {
    for (wire, points) in wire_curves(canvas, state, v, positions) {
        if state.pickup.as_ref().is_some_and(|p| p.original == wire) { continue; }
        let cutting = state.cut.as_ref().is_some_and(|cut| cut.hits.contains(&wire));
        let selected = state.selected_wires.contains(&wire);
        let color = if cutting { Color32::from_rgb(255,100,100) }
            else if selected || v.selected == wire.from || v.selected == wire.to { v.theme.accent }
            else { v.theme.wire };
        p.add(egui::Shape::line(points, Stroke::new(if selected || cutting { 2.5_f32 } else { 1.5_f32 }, color)));
    }
}

