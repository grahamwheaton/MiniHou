//! `input_define_v1`: Blender defines, renames or removes a dynamic modifier input in the bound document.
//!
//! The app's own "Add input" button and the Blender link share [`add_entry`], so ids, name collision
//! suffixes and validation are identical on both sides. The edit is applied as a normal undoable
//! `Edit::ModifierInputs` on the UI thread (see main.rs).
use crate::{modifier_inputs, Edit, Result};
use serde_json::{json, Value};

pub const CAPABILITY: &str = "input_define_v1";
/// Upper bound for one serialized `input_define` header (a definition is tiny; anything larger is refused).
pub const MAX_MESSAGE_BYTES: usize = 64 * 1024;

/// What a user asks for; omitted fields fall back to the same defaults as the app's Add input button.
#[derive(Clone, Debug, Default)]
pub struct Draft {
    pub name: String,
    pub kind: String,
    pub default: Option<Value>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub subtype: String,
    pub options: Vec<String>,
    pub doc: String,
}

fn taken_names(doc: &Value) -> Vec<String> {
    let mut names: Vec<String> = doc["exposed"].as_array().into_iter().flatten()
        .filter_map(|e| e["name"].as_str().map(str::to_owned)).collect();
    names.extend(modifier_inputs::entries(doc).into_iter().filter_map(|e| e["name"].as_str().map(str::to_owned)));
    names
}

fn reserved(name: &str) -> bool {
    name == "Geometry" || name == "Source Object" || name.starts_with("Object: ")
}

/// `wanted`, or `wanted 2`, `wanted 3`, ... when the name is taken anywhere in the document (or reserved).
pub fn unique_name(taken: &[String], wanted: &str) -> String {
    let base = wanted.trim();
    let base = if base.is_empty() { "Input".to_owned() } else { base.to_owned() };
    // "Object: " is reserved for object bindings and no suffix frees it.
    let base = match base.strip_prefix("Object: ") { Some(rest) => format!("Object {rest}"), None => base };
    let base = base.as_str();
    let free = |n: &str| !reserved(n) && !taken.iter().any(|t| t == n);
    if free(base) { return base.to_owned(); }
    (2..).map(|i| format!("{base} {i}")).find(|n| free(n)).unwrap()
}

/// The next definition list for `node` with `draft` appended. Returns (inputs, new entry).
pub fn add_entry(doc: &Value, node: &str, draft: &Draft) -> Result<(Vec<Value>, Value)> {
    let entries = doc["nodes"][node]["modifier_inputs"].as_array().cloned().unwrap_or_default();
    if !modifier_inputs::KINDS.contains(&draft.kind.as_str()) {
        return Err(format!("unsupported Blender input type {:?}", draft.kind).into());
    }
    let id = (1..).map(|i| format!("input_{i}")).find(|id| !entries.iter().any(|e| e["id"] == *id)).unwrap();
    let name = unique_name(&taken_names(doc), &draft.name);
    let options: Vec<&str> = draft.options.iter().map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
    let default = match &draft.default {
        Some(v) if !v.is_null() => v.clone(),
        _ if draft.kind == "menu" => options.first().map_or(Value::Null, |s| json!(s)),
        _ => modifier_inputs::default_for(&draft.kind),
    };
    let mut entry = json!({"id": id, "name": name, "kind": draft.kind, "default": default});
    if let Some(min) = draft.min { entry["min"] = json!(min); }
    if let Some(max) = draft.max { entry["max"] = json!(max); }
    if draft.kind == "menu" { entry["options"] = json!(options); }
    else if !draft.subtype.trim().is_empty() { entry["subtype"] = json!(draft.subtype.trim()); }
    if !draft.doc.trim().is_empty() { entry["doc"] = json!(draft.doc.trim()); }
    let mut next = entries;
    next.push(entry.clone());
    modifier_inputs::validate(&next)?;
    Ok((next, entry))
}

/// The modifier_input node to edit: the named one, else the first in document order.
fn target_node(doc: &Value, wanted: Option<&str>) -> Result<String> {
    let nodes = doc["nodes"].as_object().ok_or("document has no nodes")?;
    if let Some(name) = wanted.filter(|s| !s.is_empty()) {
        return if nodes.get(name).is_some_and(|n| n["type"] == "modifier_input") { Ok(name.to_owned()) }
            else { Err(format!("{name:?} is not a MiniHou input node").into()) };
    }
    nodes.iter().find(|(_, n)| n["type"] == "modifier_input").map(|(k, _)| k.clone())
        .ok_or_else(|| "this network has no MiniHou input node (modifier_input); add one in MiniHou first".into())
}

fn draft(msg: &Value) -> Result<Draft> {
    let input = &msg["input"];
    let num = |k: &str| -> Result<Option<f64>> {
        match input.get(k) { None | Some(Value::Null) => Ok(None),
            Some(v) => v.as_f64().map(Some).ok_or_else(|| format!("{k} must be numeric").into()) }
    };
    Ok(Draft {
        name: input["name"].as_str().unwrap_or("").to_owned(),
        kind: input["kind"].as_str().ok_or("input_define needs input.kind")?.to_owned(),
        default: input.get("default").cloned(),
        min: num("min")?, max: num("max")?,
        subtype: input["subtype"].as_str().unwrap_or("").to_owned(),
        options: input["options"].as_array().into_iter().flatten().filter_map(|v| v.as_str().map(str::to_owned)).collect(),
        doc: input["doc"].as_str().unwrap_or("").to_owned(),
    })
}

/// Turn one `input_define` message into the document edit plus the resulting entry
/// (for `remove`, the removed entry). Pure: the caller applies the edit through undo history.
pub fn plan(doc: &Value, msg: &Value) -> Result<(Edit, Value)> {
    if serde_json::to_vec(msg).map(|b| b.len()).unwrap_or(usize::MAX) > MAX_MESSAGE_BYTES {
        return Err(format!("input_define is larger than {MAX_MESSAGE_BYTES} bytes").into());
    }
    let node = target_node(doc, msg["node"].as_str())?;
    let entries = doc["nodes"][&node]["modifier_inputs"].as_array().cloned().unwrap_or_default();
    let find = |id: &str| entries.iter().position(|e| e["id"] == id).ok_or_else(|| format!("unknown input id {id:?}"));
    let (inputs, entry) = match msg["op"].as_str().unwrap_or("add") {
        "add" => add_entry(doc, &node, &draft(msg)?)?,
        "rename" => {
            let index = find(msg["id"].as_str().ok_or("rename needs id")?)?;
            let mut next = entries.clone();
            let old = next[index]["name"].as_str().unwrap_or("").to_owned();
            let taken: Vec<String> = taken_names(doc).into_iter().filter(|n| *n != old).collect();
            next[index]["name"] = json!(unique_name(&taken, msg["name"].as_str().unwrap_or("")));
            modifier_inputs::validate(&next)?;
            let entry = next[index].clone();
            (next, entry)
        }
        "remove" => {
            let mut next = entries.clone();
            let entry = next.remove(find(msg["id"].as_str().ok_or("remove needs id")?)?);
            (next, entry)
        }
        other => return Err(format!("unknown input_define op {other:?}").into()),
    };
    let mut entry = entry;
    entry["node"] = json!(node);
    Ok((Edit::ModifierInputs { node, inputs }, entry))
}

/// UI-thread handler shared by the app and the link_selftest host. `live` is the bound live modifier path
/// (LinkServer::bound_path); without one, the open document's own path must match. Applies the edit
/// through undo history and returns (reply, changed); the caller recooks with `Edit::Replace` when changed.
pub fn handle(editor: &mut crate::EditorDocument, live: Option<&str>, msg: &Value) -> (Value, bool) {
    let path = msg["path"].as_str().unwrap_or("");
    let bound = match live {
        Some(live) => live == path,
        None => !path.is_empty() && std::path::Path::new(path) == editor.path.as_path(),
    };
    if !bound {
        return (error(msg, "not_bound", "this network is not the document open in MiniHou; edit it there first, or the input is stored offline"), false);
    }
    let applied = plan(editor.doc.value(), msg).and_then(|(edit, entry)| {
        editor.history.finish(&editor.doc);
        editor.history.edit(&mut editor.doc, &edit)?;
        editor.history.finish(&editor.doc);
        Ok(entry)
    });
    match applied {
        Ok(entry) => (ack(msg, &entry, editor.doc.value()), true),
        Err(e) => (error(msg, "invalid", &e.to_string()), false),
    }
}

pub fn ack(msg: &Value, entry: &Value, graph: &Value) -> Value {
    json!({"type":"input_define_ack","status":"ok","request_id":msg["request_id"],"instance_id":msg["instance_id"],
           "op":msg["op"].as_str().unwrap_or("add"),"input":entry,"graph":graph,
           "interface":crate::link_interface(graph)["params"]})
}

/// `code`: "not_bound" (the network is not the document open in MiniHou), "invalid" (rejected definition).
pub fn error(msg: &Value, code: &str, message: &str) -> Value {
    json!({"type":"input_define_ack","status":"error","code":code,"message":message,
           "request_id":msg["request_id"],"instance_id":msg["instance_id"]})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Document;

    fn doc() -> Document {
        Document::parse(json!({"display":"input","exposed":[{"name":"Scale","node":"input","param":"x"}],
            "nodes":{"input":{"type":"modifier_input","modifier_inputs":[{"id":"input_1","name":"Amount","kind":"float","default":0.0}]}}}).to_string()).unwrap()
    }

    #[test]
    fn add_suffixes_collisions_and_is_undoable() {
        let mut d = doc();
        let mut history = crate::History::default();
        let msg = json!({"type":"input_define","request_id":4,"instance_id":"m","op":"add",
                         "input":{"name":"Amount","kind":"object"}});
        let (edit, entry) = plan(d.value(), &msg).unwrap();
        assert_eq!((entry["id"].as_str(), entry["name"].as_str(), &entry["default"]), (Some("input_2"), Some("Amount 2"), &json!("")));
        history.edit(&mut d, &edit).unwrap(); history.finish(&d);
        let a = ack(&msg, &entry, d.value());
        assert_eq!((a["status"].as_str(), a["request_id"].as_i64()), (Some("ok"), Some(4)));
        assert!(a["interface"].as_array().unwrap().iter().any(|p| p["name"] == "Amount 2" && p["kind"] == "object"));
        // Exposed names and reserved Blender names collide too.
        for (wanted, got) in [("Scale", "Scale 2"), ("Geometry", "Geometry 2"), ("Object: x", "Object x"), ("", "Input")] {
            let m = json!({"input":{"name":wanted,"kind":"bool"}});
            assert_eq!(plan(d.value(), &m).unwrap().1["name"], json!(got));
        }
        history.undo(&mut d);
        assert_eq!(d.value()["nodes"]["input"]["modifier_inputs"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn every_kind_definable_and_bad_definitions_rejected() {
        for kind in modifier_inputs::KINDS {
            let opts = if *kind == "menu" { json!(["A","B"]) } else { json!(null) };
            let m = json!({"input":{"name":kind,"kind":kind,"options":opts}});
            assert!(plan(doc().value(), &m).is_ok(), "{kind}");
        }
        for bad in [json!({"input":{"name":"x","kind":"quaternion"}}),
                    json!({"input":{"name":"x","kind":"float","default":"s"}}),
                    json!({"input":{"name":"x","kind":"float","min":2.0,"max":1.0}}),
                    json!({"input":{"name":"x","kind":"menu"}}),
                    json!({"op":"explode"}),
                    json!({"input":{"name":"x".repeat(MAX_MESSAGE_BYTES),"kind":"float"}})] {
            assert!(plan(doc().value(), &bad).is_err(), "{bad}");
        }
        let e = error(&json!({"request_id":2,"instance_id":"m"}), "not_bound", "nope");
        assert_eq!((e["type"].as_str(), e["status"].as_str(), e["request_id"].as_i64()), (Some("input_define_ack"), Some("error"), Some(2)));
        assert!(plan(&json!({"nodes":{"b":{"type":"box"}}}), &json!({"input":{"kind":"float"}})).is_err());
    }

    #[test]
    fn handle_requires_the_bound_document_and_edits_through_undo() {
        let mut editor = crate::EditorDocument::new(doc(), "net.json".into());
        let msg = json!({"type":"input_define","request_id":1,"instance_id":"m","path":"other.json","input":{"name":"T","kind":"object"}});
        let (reply, changed) = handle(&mut editor, None, &msg);
        assert_eq!((reply["code"].as_str(), changed), (Some("not_bound"), false));
        let (reply, changed) = handle(&mut editor, Some("other.json"), &msg);
        assert_eq!((reply["status"].as_str(), changed), (Some("ok"), true));
        let mut ok = msg.clone(); ok["path"] = json!("net.json");
        assert_eq!(handle(&mut editor, None, &ok).0["input"]["name"], "T 2");
        assert_eq!(handle(&mut editor, None, &json!({"path":"net.json","input":{"kind":"float","default":"x"}})).0["code"], "invalid");
        editor.history.undo(&mut editor.doc); editor.history.undo(&mut editor.doc);
        assert_eq!(editor.doc.value()["nodes"]["input"]["modifier_inputs"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn rename_and_remove_keep_stable_ids() {
        let d = doc();
        let (edit, entry) = plan(d.value(), &json!({"op":"rename","id":"input_1","name":"Scale"})).unwrap();
        assert_eq!((entry["id"].as_str(), entry["name"].as_str()), (Some("input_1"), Some("Scale 2")));
        let Edit::ModifierInputs { inputs, .. } = edit else { panic!() };
        assert_eq!(inputs[0]["id"], "input_1");
        let (edit, _) = plan(d.value(), &json!({"op":"remove","id":"input_1"})).unwrap();
        assert!(matches!(edit, Edit::ModifierInputs { inputs, .. } if inputs.is_empty()));
        assert!(plan(d.value(), &json!({"op":"remove","id":"nope"})).is_err());
    }
}
