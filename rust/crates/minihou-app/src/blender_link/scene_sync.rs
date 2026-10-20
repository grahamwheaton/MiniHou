use std::{
    io,
    path::{Path, PathBuf},
    sync::{atomic::Ordering, Arc},
};
use minihou_app::{
    link_protocol::{self, Arrays},
    Document,
};
use serde_json::{json, Value};
use super::{LinkRequest, LinkServer};

pub fn restore_editor(
    server: &mut LinkServer,
    editor: &mut minihou_app::EditorDocument,
) -> Vec<minihou_app::Edit> {
    let Some(path) = server.active_live.clone() else {
        return vec![];
    };
    let request = LinkRequest {
        peer: 0,
        message: json!({"type":"live_remove","path":path}),
        arrays: Arrays::new(),
        completion: None,
    };
    server.handle_live(&request, editor).unwrap_or_default()
}

/// Apply the user's answer to an `embed_conflict` request (graph_embed_v1). The add-on
/// learns the outcome through `graph_pull`. Unsaved work is never overwritten here:
/// `KeepMine` changes nothing, the others replace the bound document through its
/// undo history (the caller exports the user's version first for `OpenAsNew`).
pub fn resolve_embed(
    server: &mut LinkServer,
    conflict: &Value,
    resolution: minihou_app::graph_embed::Resolution,
    editor: &mut minihou_app::EditorDocument,
) -> io::Result<Vec<minihou_app::Edit>> {
    let id = conflict["instance_id"].as_str().unwrap_or("").to_string();
    let mut edits = Vec::new();
    if resolution.adopts_embedded() && conflict["kind"] == "live" {
        let request = LinkRequest {
            peer: 0,
            message: json!({"type":"live_replace","path":conflict["path"],"graph":conflict["graph"]}),
            arrays: Arrays::new(),
            completion: None,
        };
        edits = server.handle_live(&request, editor)?;
    }
    server.shared.embed_resolutions.lock().unwrap().insert(id, resolution.as_str().into());
    Ok(edits)
}

pub fn document_changed(server: &LinkServer, doc: &Document, path: &Path) {
    let interface = minihou_app::link_interface(doc.value());
    let watch = link_protocol::watch_objects(doc.value());
    let path = path.to_string_lossy();
    if let Some(key) = &server.active_live {
        server.shared
            .live_documents
            .lock()
            .unwrap()
            .insert(key.clone(), doc.value().clone());
    }
    let mut snapshot = server.shared.snapshot.lock().unwrap();
    if snapshot.document != *doc.value()
        || snapshot.interface != interface
        || snapshot.graph_path.as_str() != path.as_ref()
        || snapshot.watch != watch
    {
        if snapshot.graph_path.as_str() != path.as_ref() {
            snapshot.geometry = None;
            snapshot.error = None;
        }
        snapshot.document = doc.value().clone();
        snapshot.watch = watch;
        snapshot.interface = interface;
        snapshot.graph_path = path.into_owned();
        snapshot.version += 1;
    }
}

/// UI-thread boundary for live modifier documents. Original document/history stay in memory.
/// All feeds are transient worker edits, never saved into the graph.
pub fn handle_live(
    server: &mut LinkServer,
    request: &LinkRequest,
    editor: &mut minihou_app::EditorDocument,
) -> io::Result<Vec<minihou_app::Edit>> {
    let result = (|| -> minihou_app::Result<Vec<minihou_app::Edit>> {
        use minihou_app::Edit;
        use minihou_core::nodes::blender_input::ExternalInput;
        let h = &request.message;
        let key = h["path"].as_str().ok_or("missing live path")?.to_owned();
        let current = server.active_live.as_deref() == Some(&key);
        if h["type"] == "live_remove" {
            server.shared.live_documents.lock().unwrap().remove(&key);
            server.live_editors.remove(&key);
            if current {
                server.active_live = None;
                server.shared.live_active.store(false, Ordering::Relaxed);
                if let Some(original) = server.saved_editor.take() {
                    *editor = original;
                    server.document_changed(&editor.doc, &editor.path);
                    let (frame, fps) = server.saved_time.take().unwrap_or((1., 24.));
                    server.frame = frame;
                    server.fps = fps;
                    return Ok(vec![
                        Edit::Replace(editor.doc.value().clone()),
                        Edit::Time { frame, fps },
                    ]);
                }
            }
            return Ok(vec![]);
        }
        if h["type"] == "live_replace" {
            // graph_embed_v1: Blender's embedded graph replaces the bound document (undoable).
            let doc = Document::parse(h["graph"].to_string())?;
            server.shared.live_documents.lock().unwrap().insert(key.clone(), doc.value().clone());
            if current {
                editor.history.replace(&mut editor.doc, doc);
                server.document_changed(&editor.doc, &editor.path);
                return Ok(vec![Edit::Replace(editor.doc.value().clone())]);
            }
            if let Some(cached) = server.live_editors.get_mut(&key) {
                cached.history.replace(&mut cached.doc, doc);
            }
            return Ok(vec![]);
        }
        if h["type"] != "live_activate" && !current {
            if let Some(overrides)=h["overrides"].as_object() {
                let graph=server.shared.live_documents.lock().unwrap().get(&key).cloned().unwrap_or(h["graph"].clone());
                let mut doc=Document::parse(graph.to_string())?;
                for(name,value)in overrides { let edit=minihou_app::link_param_edit(doc.value(),&json!({"name":name,"value":value}))?;doc.edit(&edit)?; }
                server.shared.live_documents.lock().unwrap().insert(key.clone(),doc.value().clone());
                if let Some(cached)=server.live_editors.get_mut(&key){cached.history.replace(&mut cached.doc,doc);}
            }
            return Ok(vec![]);
        }
        // Input updates never replace a graph edited in the native app. Merge only
        // explicitly changed Blender controls into the current document on this thread.
        let mut doc=if current {editor.doc.clone()}else{Document::parse(h["graph"].to_string())?};
        if let Some(overrides)=h["overrides"].as_object(){for(name,value)in overrides{
            let edit=minihou_app::link_param_edit(doc.value(),&json!({"name":name,"value":value}))?;doc.edit(&edit)?;
        }}
        // Validate every transient feed before changing the selected document.
        let input = request
            .arrays
            .iter()
            .filter(|(k, _)| !k.starts_with("source:"))
            .map(|(k, a)| (k.clone(), a.clone()))
            .collect();
        let geo = link_protocol::decode_object(h, &input)?;
        let mut edits = vec![Edit::External {
            name: "modifier_input".into(),
            input: ExternalInput::Object {
                geo: Arc::new(geo),
                matrix: None,
                info: Value::Null,
            },
        }];
        for (nid, n) in doc.value()["nodes"].as_object().into_iter().flatten() {
            if n["type"] == "object_input" {
                let binding = n["params"]["binding"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .unwrap_or(nid);
                edits.push(Edit::External {
                    name: format!("object_input:{binding}"),
                    input: ExternalInput::Missing(
                        "Choose an object in this modifier's Object input picker in Blender"
                            .into(),
                    ),
                });
            }
        }
        // Blender Object nodes follow their group picker while the modifier is live (empty: waiting).
        let pickers = minihou_app::modifier_inputs::pickers(doc.value());
        for (node, _) in &pickers {
            edits.push(Edit::External { name: minihou_core::nodes::blender_input::picker_external(node), input: ExternalInput::Missing(String::new()) });
        }
        if let Some(sources) = h["sources"].as_array() {
            for (i, name) in sources.iter().enumerate() {
                let prefix = format!("source:{i}/");
                let arrays = request
                    .arrays
                    .iter()
                    .filter_map(|(k, a)| {
                        k.strip_prefix(&prefix).map(|k| (k.to_string(), a.clone()))
                    })
                    .collect();
                if let Some((node, _)) = pickers.iter().find(|(_, uid)| minihou_app::modifier_inputs::picker_key(uid) == name.as_str().unwrap_or("")) {
                    edits.push(Edit::External {
                        name: minihou_core::nodes::blender_input::picker_external(node),
                        input: ExternalInput::Object { geo: Arc::new(link_protocol::decode_object(&Value::Null, &arrays)?), matrix: minihou_app::modifier_inputs::reference_matrix(&h["references"][name.as_str().unwrap_or("")]), info: h["references"][name.as_str().unwrap_or("")].clone() },
                    });
                }
                edits.push(Edit::External {
                    name: format!("object_input:{}", name.as_str().ok_or("bad source name")?),
                    input: ExternalInput::Object {
                        geo: Arc::new(link_protocol::decode_object(h, &arrays)?),
                        matrix: None,
                        info: Value::Null,
                    },
                });
            }
        }
        let graph_changed = !current || editor.doc.value() != doc.value();
        if !current {
            let mut new = server.live_editors.remove(&key).unwrap_or_else(|| {
                minihou_app::EditorDocument::new(doc.clone(), PathBuf::from(&key))
            });
            new.history.replace(&mut new.doc, doc);
            if let Some(label) = h["label"].as_str().filter(|s| !s.is_empty()) {
                new.blender_label = Some(label.chars().take(200).collect());
            }
            let old = std::mem::replace(editor, new);
            if let Some(old_key) = server.active_live.take() {
                server.live_editors.insert(old_key, old);
            } else if server.saved_editor.is_none() {
                server.saved_editor = Some(old);
                server.saved_time = Some((server.frame, server.fps));
            }
        } else {
            editor.history.replace(&mut editor.doc, doc);
        }
        server.active_live = Some(key);
        server.shared.live_active.store(true, Ordering::Relaxed);
        server.document_changed(&editor.doc, &editor.path);
        if graph_changed {
            edits.insert(0, Edit::Replace(editor.doc.value().clone()));
        }
        let (frame, fps) = (
            h["frame"].as_f64().unwrap_or(1.),
            h["fps"].as_f64().unwrap_or(24.),
        );
        server.frame = frame;
        server.fps = fps;
        edits.push(Edit::Time { frame, fps });
        Ok(edits)
    })();
    if let Some(completion) = &request.completion {
        let _ = completion.send(result.as_ref().map(|_| ()).map_err(|e| e.to_string()));
    }
    result.map_err(|e| io::Error::other(e.to_string()))
}
