use super::*;

pub fn set_display(value: &Value, id: &str, additive: bool) -> Option<Value> {
    nodes(value)?.get(id)?;
    let mut next = value.clone();
    let mut displayed = minihou_app::display_nodes(value);
    if additive {
        if displayed.iter().any(|node| node == id) {
            if displayed.len() > 1 { displayed.retain(|node| node != id); }
        } else { displayed.push(id.to_owned()); }
    } else { displayed = vec![id.to_owned()]; }
    if displayed.is_empty() { next.as_object_mut()?.remove("display"); }
    else { next["display"] = if displayed.len() == 1 { json!(displayed[0]) } else { json!(displayed) }; }
    Some(next)
}

/// Rename a node and preserve every channel's resolved target in one graph edit.
pub fn rename(value: &Value, old: &str, new: &str) -> Option<Value> {
    if new.is_empty() || new.contains('"') || new.contains('\'') || new.contains(' ') { return None; }
    let mut value=value.clone();
    let all=nodes_mut(&mut value)?;
    if all.contains_key(new) { return None; }
    let mut renamed=all.remove(old)?;
    // A Blender Object node's group picker keeps its socket (value, links) across renames.
    if renamed["type"]=="blender_object" && renamed.get(minihou_app::modifier_inputs::PICKER_ID).is_none() { renamed[minihou_app::modifier_inputs::PICKER_ID]=json!(old); }
    all.insert(new.into(),renamed);
    for (owner,node) in all.iter_mut() {
        if let Some(inputs)=node.get_mut("inputs").and_then(Value::as_object_mut) {
            for source in inputs.values_mut() {
                match source { Value::String(s) if s==old => *s=new.into(), Value::Array(xs) => for x in xs {if x==old {*x=json!(new)}}, _=>{} }
            }
        }
        if let Some(channels)=node.get_mut("channels").and_then(Value::as_object_mut) {
            for expr in channels.values_mut() {
                if let Some(source)=expr.as_str() {
                    let before=if owner==new {old} else {owner};
                    *expr=json!(crate::value_link_ui::rewrite_expression(source,before,owner,|node|if node==old {new.into()} else {node.into()}));
                }
            }
        }
    }
    if value["display"]==old {value["display"]=json!(new);}
    if let Some(xs)=value["display"].as_array_mut() {for x in xs {if x==old {*x=json!(new)}}}
    Some(value)
}

