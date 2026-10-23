use super::*;

/// Branch the selected part of a stream, then join its movable copy with the original.
/// The viewport fills `split_params` using the same topology conversion as other group nodes.
pub fn duplicate_selection_nodes(value: &Value, source: &str, split_params: Value)
    -> Option<(Value, [String; 3])>
{
    let group = split_params.get("group")?.as_str()?;
    let domain = split_params.get("group_type")?.as_str()?;
    if group.trim().is_empty() || !matches!(domain, "points" | "edges" | "primitives") { return None; }
    let all = nodes(value)?;
    let source_node = all.get(source)?;
    let source = if source_node["type"] == "output" {
        source_node["inputs"]["geo"].as_str()?
    } else { source };
    all.get(source)?;

    let mut next = value.clone();
    let all = nodes_mut(&mut next)?;
    let split = fresh_id(all, "split");
    all.insert(split.clone(), json!({"type":"split", "inputs":{"geo":source}, "params":split_params}));
    let transform = fresh_id(all, "transform");
    all.insert(transform.clone(), json!({"type":"transform", "inputs":{"geo":split},
        "params":{"translate":[0.0, 0.0, 0.0]}}));
    let merge = fresh_id(all, "merge");
    // Rewire existing consumers before inserting Merge so its original input stays intact.
    for (id, node) in all.iter_mut() {
        if id == &split || id == &transform { continue; }
        if let Some(inputs) = node.get_mut("inputs").and_then(Value::as_object_mut) {
            for input in inputs.values_mut() {
                if input.as_str() == Some(source) { *input = json!(merge); }
                else if let Some(items) = input.as_array_mut() {
                    for item in items { if item.as_str() == Some(source) { *item = json!(merge); } }
                }
            }
        }
    }
    all.insert(merge.clone(), json!({"type":"merge", "inputs":{"geos":[source, transform]}}));
    next["display"] = json!(merge);
    if let Some(pos) = value["nodes"][source]["pos"].as_array() {
        if let (Some(x), Some(y)) = (pos.first().and_then(Value::as_f64), pos.get(1).and_then(Value::as_f64)) {
            next["nodes"][&split]["pos"] = json!([x + 192.0, y + 96.0]);
            next["nodes"][&transform]["pos"] = json!([x + 192.0, y + 224.0]);
            next["nodes"][&merge]["pos"] = json!([x, y + 352.0]);
        }
    }
    Some((next, [split, transform, merge]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_faces_branch_through_split_transform_and_merge() {
        let value = json!({"nodes":{"box1":{"type":"box"},
            "out":{"type":"output","inputs":{"geo":"box1"}}},"display":"out"});
        let (next, [split, transform, merge]) = duplicate_selection_nodes(
            &value, "out", json!({"group":"1 3", "group_type":"primitives"})).unwrap();
        assert_eq!(next["nodes"][&split]["inputs"]["geo"], "box1");
        assert_eq!(next["nodes"][&split]["params"], json!({"group":"1 3","group_type":"primitives"}));
        assert_eq!(next["nodes"][&transform]["inputs"]["geo"], split);
        assert_eq!(next["nodes"][&transform]["params"]["translate"], json!([0.0,0.0,0.0]));
        assert_eq!(next["nodes"][&merge]["inputs"]["geos"], json!(["box1",transform]));
        assert_eq!(next["nodes"]["out"]["inputs"]["geo"], merge);
        validated(next).unwrap();
    }

    #[test]
    fn empty_selection_does_nothing() {
        let value = json!({"nodes":{"box1":{"type":"box"}}});
        assert!(duplicate_selection_nodes(&value, "box1", json!({"group":"", "group_type":"points"})).is_none());
        assert_eq!(value["nodes"].as_object().unwrap().len(), 1);
    }
}
