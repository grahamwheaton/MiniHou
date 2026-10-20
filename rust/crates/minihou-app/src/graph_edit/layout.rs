use super::*;

/// Persist stream spacing in the same document edit as the new node and its wires.
/// The canvas may refine this using its live positions when an add originates there.
pub(super) fn position_in_stream(old: &Value, next: &mut Value, anchor: &str, inserted: &str, before: bool) {
    let Some(all) = nodes(old) else { return };
    fn automatic(all: &Map<String, Value>, id: &str, seen: &mut HashSet<String>) -> [f32; 2] {
        if !seen.insert(id.to_owned()) { return [96.0, 32.0] }
        let Some(node) = all.get(id) else { return [96.0, 32.0] };
        let parents: Vec<_> = node.get("inputs").and_then(Value::as_object).into_iter().flat_map(|m| m.values())
            .flat_map(|v| if let Some(s) = v.as_str() { vec![s] } else { v.as_array().map_or_else(Vec::new, |a| a.iter().filter_map(Value::as_str).collect()) })
            .filter(|source| all.contains_key(*source)).collect();
        if parents.is_empty() {
            let roots: Vec<_> = all.iter().filter(|(_, n)| n.get("inputs").and_then(Value::as_object).is_none_or(|m| m.is_empty())).map(|(id, _)| id).collect();
            return [96.0 + roots.iter().position(|root| root.as_str() == id).unwrap_or(0) as f32 * 704.0, 32.0];
        }
        let positions: Vec<_> = parents.iter().map(|p| automatic(all, p, &mut seen.clone())).collect();
        let x = positions.iter().map(|p| p[0]).sum::<f32>() / positions.len() as f32;
        [x, positions.iter().map(|p| p[1]).fold(32.0_f32, f32::max) + 128.0]
    }
    let position = |id: &str| -> Option<[f32; 2]> {
        let node = all.get(id)?;
        if let Some(at) = node.get("pos").and_then(Value::as_array) {
            return Some([at.first()?.as_f64()? as f32, at.get(1)?.as_f64()? as f32]);
        }
        Some(automatic(all, id, &mut HashSet::new()))
    };
    let Some(origin) = position(anchor) else { return };
    let consumes = |node: &Value, source: &str| -> bool {
        node.get("inputs").and_then(Value::as_object).is_some_and(|inputs| inputs.values().any(|v|
            v.as_str() == Some(source) || v.as_array().is_some_and(|a| a.iter().any(|v| v.as_str() == Some(source)))))
    };
    let mut pitch = all.iter().filter_map(|(id, node)| {
        let at = position(id)?;
        let adjacent = if before { consumes(&all[anchor], id) } else { consumes(node, anchor) };
        let dy = if before { origin[1] - at[1] } else { at[1] - origin[1] };
        (adjacent && (at[0] - origin[0]).abs() < 16.0 && dy >= 96.0).then_some(dy)
    }).min_by(f32::total_cmp).unwrap_or(128.0);
    pitch = (pitch / 32.0).round() * 32.0;
    let slot = [origin[0], origin[1] + if before { 0.0 } else { pitch }];
    let mut downstream = HashSet::from([anchor.to_owned()]);
    loop {
        let mut changed = false;
        for (id, node) in all {
            if downstream.iter().any(|source| consumes(node, source)) { changed |= downstream.insert(id.clone()); }
        }
        if !changed { break; }
    }
    for id in downstream {
        if (!before && id == anchor) || position(&id).is_none_or(|at| at[1] < slot[1]) { continue; }
        let at = position(&id).unwrap();
        next["nodes"][&id]["pos"] = json!([at[0], at[1] + pitch]);
    }
    next["nodes"][inserted]["pos"] = json!(slot);
}

