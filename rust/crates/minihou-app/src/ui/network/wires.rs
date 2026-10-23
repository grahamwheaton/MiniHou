use super::*;

/// One entry per declared input: (port name, is multi-input). A multi port is one elongated bar.
pub(super) fn input_slots(node: &NodeDef, raw: &serde_json::Value) -> Vec<(String, bool)> {
    if node.node_type == "subnet" {
        if let Some(sockets) = raw["input_sockets"].as_array() {
            return sockets.iter().filter_map(|socket| socket["index"].as_u64())
                .map(|index| (format!("in{index}"), false)).collect();
        }
    }
    minihou_core::nodes::get(&node.node_type).map(|spec| spec.inputs.iter().map(|p| (p.name.to_owned(), p.multi)).collect()).unwrap_or_default()
}

/// Inputs are spread evenly across the card width, so a lone input stays centred.
pub(super) fn input_offset(index: usize, count: usize) -> Vec2 {
    vec2(CARD.x * (index as f32 + 1.0) / (count.max(1) as f32 + 1.0), 0.0)
}

/// Local x of wire `wire` of `wires` on port `group` of `groups`; a multi port spreads its wires along its bar.
pub(super) fn port_x(groups: usize, group: usize, multi: bool, wire: usize, wires: usize) -> f32 {
    if !multi { return input_offset(group, groups).x; }
    let (left, width) = bar_span(groups, group);
    left + width * (wire as f32 + 1.0) / (wires.max(1) as f32 + 1.0)
}

/// Local (left, width) of a multi-input bar: 80% of the card for a lone port, else just under the spacing between ports.
pub(super) fn bar_span(groups: usize, group: usize) -> (f32, f32) {
    let width = if groups <= 1 { CARD.x * 0.8 } else { CARD.x / (groups as f32 + 1.0) * 0.9 };
    (input_offset(group, groups).x - width * 0.5, width)
}

pub(super) fn wire_count(node: &NodeDef, port: &str) -> usize {
    node.inputs.iter().find(|(name, _)| name == port).map_or(0, |(_, refs)| refs.len())
}

pub(super) fn output_count(raw: &serde_json::Value) -> usize {
    raw["output_sockets"].as_array().map_or_else(||
        raw["type"].as_str().and_then(minihou_core::nodes::get).map_or(1, |spec| spec.outputs.len().max(1)),
        |sockets| sockets.len().max(1))
}

pub(super) fn output_position(raw: &serde_json::Value, index: usize) -> Option<usize> {
    raw["output_sockets"].as_array().map_or(Some(index), |sockets|
        sockets.iter().position(|socket| socket["index"].as_u64() == Some(index as u64)))
}

pub(super) fn output_index(raw: &serde_json::Value, position: usize) -> Option<usize> {
    raw["output_sockets"].as_array().map_or(Some(position), |sockets|
        sockets.get(position)?.get("index")?.as_u64().and_then(|index| usize::try_from(index).ok()))
}

pub(super) fn output_offset(index: usize, count: usize) -> Vec2 {
    vec2(CARD.x * 0.5 + (index as f32 - (count.saturating_sub(1)) as f32 * 0.5) * 16.0, CARD.y)
}

#[cfg(test)]
mod subnet_port_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn declared_subnet_sockets_have_distinct_canvas_positions() {
        let mut doc = json!({"nodes": {"box1": {"type": "box"}, "sub": {"type": "subnet", "inputs": {"geo": "box1"}}}});
        crate::subnet_edit::initialize(&mut doc, "sub");
        let doc = crate::subnet_edit::add_input(&doc, "sub", "box1").unwrap();
        let graph = minihou_core::Graph::from_value(&doc).unwrap();
        let node = &graph.nodes.iter().find(|(id, _)| id == "sub").unwrap().1;
        let names: Vec<String> = input_slots(node, &doc["nodes"]["sub"]).into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, vec!["in0", "in1"]);
        assert_ne!(input_offset(0, 2), input_offset(1, 2));
        assert_ne!(output_offset(0, 2), output_offset(1, 2));
        let sparse=json!({"output_sockets":[{"index":0,"name":"First"},{"index":3,"name":"Fourth"}]});
        assert_eq!(output_index(&sparse,1),Some(3));
        assert_eq!(output_position(&sparse,3),Some(1));
        assert_eq!(output_count(&json!({"type":"flood_fill"})),2);
    }
}

pub(super) fn canvas_ports(v: &View, positions: &[(String, Pos2)]) -> Vec<(AddConnection, Pos2)> {
    let mut ports = Vec::new();
    for (id, at) in positions {
        let Some((_, node)) = v.graph.nodes.iter().find(|(n, _)| n == id) else { continue };
        let inputs = input_slots(node, &v.doc["nodes"][id]);
        for (index, (port, multi)) in inputs.iter().enumerate() {
            let target = AddConnection::Input { node: id.clone(), port: port.clone() };
            if *multi {
                // A drop anywhere on the bar targets the port: sample it densely.
                let (left, width) = bar_span(inputs.len(), index);
                let steps = (width / 8.0).ceil() as usize;
                for s in 0..=steps { ports.push((target.clone(), *at + vec2(left + width * s as f32 / steps as f32, 0.0))); }
            } else {
                ports.push((target, *at + input_offset(index, inputs.len())));
            }
        }
        if node.node_type != "output" {
            let count = output_count(&v.doc["nodes"][id]);
            for index in 0..count {
                ports.push((AddConnection::Output(id.clone()), *at + output_offset(index, count)));
            }
        }
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
    curves_for(canvas, state, v.graph, v.doc, positions)
}
pub(super) fn curves_for(canvas: Rect, state: &State, graph: &minihou_core::Graph, doc: &serde_json::Value, positions: &[(String, Pos2)]) -> Vec<(WireRef, Vec<Pos2>)> {
    let mut wires = Vec::new();
    let mut seen = BTreeSet::new();
    let lookup: HashMap<&str, Pos2> = positions.iter().map(|(id, at)| (id.as_str(), *at)).collect();
    for (target, def) in &graph.nodes {
        let Some(&end) = lookup.get(target.as_str()) else { continue };
        let slots = input_slots(def, &doc["nodes"][target]);
        let mut used = HashMap::<&str, usize>::new();
        for (port, source, source_index) in def.inputs.iter().flat_map(|(port, refs)| refs.iter().enumerate().map(move |(index, source)|
            (port, source, index))) {
            let offset = used.entry(port).or_default();
            let Some(index) = slots.iter().position(|(name, _)| name == port) else { continue };
            let x = port_x(slots.len(), index, slots[index].1, *offset, wire_count(def, port));
            *offset += 1;
            let Some(&start) = lookup.get(source.as_str()) else { continue };
            let endpoint = def.output_for(port, source_index, source);
            let raw=&doc["nodes"][source];
            let Some(position)=output_position(raw,endpoint.output) else { continue };
            let a = screen(canvas, state, start + output_offset(position, output_count(raw)));
            let b = screen(canvas, state, end + vec2(x, 0.0));
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


#[cfg(test)] mod port_spacing_tests {
    use super::*;
    #[test] fn single_port_is_centred_and_multiple_ports_are_even() {
        assert_eq!(input_offset(0, 1).x, CARD.x * 0.5);
        let xs: Vec<f32> = (0..3).map(|i| input_offset(i, 3).x).collect();
        assert!((xs[1] - xs[0] - (xs[2] - xs[1])).abs() < 1e-4);
        assert!((xs[0] - (CARD.x - xs[2])).abs() < 1e-4);
        assert!(xs[0] > 0.0 && xs[2] < CARD.x);
    }
}

#[cfg(test)] mod merged_socket_tests {
    use super::*;
    #[test] fn multi_wires_spread_along_one_bar_and_singles_stay_put() {
        let (left, width) = bar_span(1, 0);
        assert!((left + width * 0.5 - CARD.x * 0.5).abs() < 1e-4);
        assert!(left >= 0.0 && left + width <= CARD.x);
        let xs: Vec<f32> = (0..3).map(|k| port_x(1, 0, true, k, 3)).collect();
        assert!((xs[1] - xs[0] - (xs[2] - xs[1])).abs() < 1e-4);
        assert!(xs[0] > left && xs[2] < left + width);
        assert!((port_x(1, 0, true, 0, 1) - CARD.x * 0.5).abs() < 1e-3);
        assert_eq!(port_x(2, 1, false, 0, 1), input_offset(1, 2).x);
        let (l0, w0) = bar_span(2, 0);
        let (l1, _) = bar_span(2, 1);
        assert!(l0 + w0 <= l1);
    }
}
