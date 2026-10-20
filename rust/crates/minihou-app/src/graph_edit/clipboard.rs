use super::*;

/// The clipboard contains node definitions. References outside the copied set are dropped.
pub fn copy(value: &Value, ids: &[String]) -> Option<Value> {
    let all = nodes(value)?;
    let selected: HashSet<_> = ids.iter().map(String::as_str).collect();
    let mut copied = Map::new();
    for id in ids {
        let mut node = all.get(id)?.clone();
        retain_wires(&mut node, &selected);
        copied.insert(id.clone(), node);
    }
    (!copied.is_empty()).then(|| json!({"nodes": copied}))
}

fn retain_wires(node: &mut Value, ids: &HashSet<&str>) {
    if let Some(inputs) = node.get_mut("inputs").and_then(Value::as_object_mut) {
        inputs.retain(|_, source| match source {
            Value::String(s) => ids.contains(s.as_str()),
            Value::Array(a) => { a.retain(|x| x.as_str().is_some_and(|s| ids.contains(s))); !a.is_empty() }
            _ => false,
        });
    }
}

pub fn paste(value: &Value, clipboard: &Value) -> Option<(Value, Vec<String>)> {
    let copied = nodes(clipboard)?;
    if copied.is_empty() { return None; }
    let mut value = value.clone();
    let all = nodes_mut(&mut value)?;
    let mut ids = HashMap::new();
    for (old, node) in copied {
        let ty = node.get("type")?.as_str()?;
        let id = fresh_id(all, ty);
        all.insert(id.clone(), json!({"type": ty}));
        ids.insert(old.as_str(), id);
    }
    for (old, src) in copied {
        let mut node = src.clone();
        node.as_object_mut()?.remove("linked_params_group");
        if let Some(channels) = node.get_mut("channels").and_then(Value::as_object_mut) {
            for expr in channels.values_mut() {
                if let Some(source) = expr.as_str() {
                    *expr = json!(crate::value_link_ui::rewrite_expression(source, old, &ids[old.as_str()], |referenced| ids.get(referenced).cloned().unwrap_or_else(||referenced.into())));
                }
            }
        }
        if let Some(inputs) = node.get_mut("inputs").and_then(Value::as_object_mut) {
            for source in inputs.values_mut() {
                match source {
                    Value::String(s) => if let Some(id) = ids.get(s.as_str()) { *s = id.clone(); },
                    Value::Array(a) => for item in a { if let Some(id) = item.as_str().and_then(|s| ids.get(s)) { *item = json!(id); } },
                    _ => {},
                }
            }
        }
        all.insert(ids[old.as_str()].clone(), node);
    }
    let new_ids = copied.keys().map(|old| ids[old.as_str()].clone()).collect();
    Some((value, new_ids))
}

/// Copy displayed objects with their upstream graphs so later source edits do not alter the copies.
pub fn duplicate_objects(value: &Value, ids: &[String]) -> Option<(Value, Vec<String>)> {
    let all = nodes(value)?;
    let mut closure = HashSet::new();
    let mut pending = ids.to_vec();
    while let Some(id) = pending.pop() {
        if !closure.insert(id.clone()) { continue; }
        let inputs = all.get(&id)?.get("inputs").and_then(Value::as_object);
        if let Some(inputs) = inputs {
            for source in inputs.values() {
                if let Some(id) = source.as_str() { pending.push(id.to_owned()); }
                if let Some(list) = source.as_array() {
                    pending.extend(list.iter().filter_map(Value::as_str).map(str::to_owned));
                }
            }
        }
    }
    let mut old_ids: Vec<_> = closure.into_iter().collect();
    old_ids.sort();
    let clip = copy(value, &old_ids)?;
    let (mut next, new_ids) = paste(value, &clip)?;
    let mapping: HashMap<_, _> = old_ids.iter().cloned().zip(new_ids).collect();
    let roots: Vec<_> = ids.iter().map(|id| mapping.get(id).cloned()).collect::<Option<_>>()?;
    let all = nodes_mut(&mut next)?;
    let mut copies = Vec::new();
    for root in roots {
        let wrapper = fresh_id(all, "transform");
        all.insert(wrapper.clone(), json!({"type":"transform","inputs":{"geo":root}}));
        copies.push(wrapper);
    }
    let mut shown = minihou_app::display_nodes(value);
    shown.extend(copies.iter().cloned());
    next["display"] = json!(shown);
    Some((next, copies))
}

/// Copies nodes and joins each pair's parameter edits, including non-numeric parameters.
pub fn duplicate_linked(value: &Value, ids: &[String]) -> Option<(Value, Vec<String>)> {
    let clip = copy(value, ids)?;
    let (mut next, copies) = paste(value, &clip)?;
    for (source, copy) in ids.iter().zip(&copies) {
        let group = next["nodes"][source]["linked_params_group"].as_str()
            .map(str::to_owned).unwrap_or_else(|| source.clone());
        next["nodes"][source]["linked_params_group"] = json!(group);
        next["nodes"][copy]["linked_params_group"] = json!(group);
    }
    Some((next, copies))
}

#[cfg(test)]
mod duplicate_tests {
    use super::*;
    #[test]
    fn network_linked_duplicates_share_groups_but_regular_copies_do_not() {
        let doc = json!({"nodes":{"box1":{"type":"box","params":{"size":[1,2,3]}}}});
        let (linked, copies) = duplicate_linked(&doc, &["box1".into()]).unwrap();
        assert_eq!(linked["nodes"]["box1"]["linked_params_group"], "box1");
        assert_eq!(linked["nodes"][&copies[0]]["linked_params_group"], "box1");
        let (regular, regular_ids) = paste(&linked, &copy(&linked, &["box1".into()]).unwrap()).unwrap();
        assert!(regular["nodes"][&regular_ids[0]].get("linked_params_group").is_none());
    }
    #[test]
    fn pick_viewport_duplicate_copies_upstream_nodes_and_displays_both() {
        let doc = json!({"display":"transform1","nodes":{
            "box1":{"type":"box"},
            "transform1":{"type":"transform","inputs":{"geo":"box1"}}
        }});
        let (copy, ids) = duplicate_objects(&doc, &["transform1".into()]).unwrap();
        assert!(crate::Document::parse(copy.to_string()).is_ok());
        let root = copy["nodes"][&ids[0]]["inputs"]["geo"].as_str().unwrap();
        let upstream = copy["nodes"][root]["inputs"]["geo"].as_str().unwrap();
        assert_ne!(upstream, "box1");
        assert!(copy["nodes"].get(upstream).is_some());
        assert_eq!(copy["display"], json!(["transform1", ids[0]]));
    }
    #[test]
    fn pick_viewport_duplicate_of_output_is_valid() {
        let doc = json!({"display":"out","nodes":{
            "box1":{"type":"box"},
            "out":{"type":"output","inputs":{"geo":"box1"}}
        }});
        let (copy, ids) = duplicate_objects(&doc, &["out".into()]).unwrap();
        assert!(crate::Document::parse(copy.to_string()).is_ok());
        assert_eq!(copy["display"], json!(["out", ids[0]]));
    }
}

