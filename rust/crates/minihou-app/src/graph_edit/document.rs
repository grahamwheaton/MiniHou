use super::*;

pub fn validated(value: Value) -> Result<Document, String> {
    let text = serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?;
    Document::parse(text).map_err(|e| e.to_string())
}

pub(super) fn nodes(value: &Value) -> Option<&Map<String, Value>> { value.get("nodes")?.as_object() }
pub(super) fn nodes_mut(value: &mut Value) -> Option<&mut Map<String, Value>> { value.get_mut("nodes")?.as_object_mut() }

pub(super) fn fresh_id(nodes: &Map<String, Value>, ty: &str) -> String {
    (1..).map(|n| format!("{ty}{n}")).find(|id| !nodes.contains_key(id)).unwrap()
}
