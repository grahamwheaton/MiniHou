//! Throwaway TCP host for add-on interoperability tests. Never creates a window or GPU.
#[path = "../src/blender_link.rs"]
#[allow(dead_code)]
mod blender_link;
use minihou_app::{cook_batch, CookRequest, Document, Edit, EditorDocument};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
fn main() -> minihou_app::Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if !(3..=4).contains(&args.len()) {
        return Err("usage: link_selftest GRAPH.json ADDRESS SECONDS [CONTROL.json]".into());
    }
    let path = PathBuf::from(&args[0]).canonicalize()?;
    let mut editor = EditorDocument::new(Document::open(&path)?, path);
    let mut session = editor.doc.session()?;
    let mut target = session.graph().main_output()?;
    let mut server = blender_link::LinkServer::bind(&args[1], &editor.doc, &editor.path)?;
    let mut materials=minihou_app::materials::Registry::new(".");
    let mut revision = 1;
    let response = cook_batch(
        &mut session,
        &mut target,
        vec![CookRequest {
            revision,
            edit: Edit::Replace(editor.doc.value().clone()),
        }],
        |_| (),
    );
    server.cooked(&response);
    println!("LINK_READY {}", server.address());
    let mut command_id = 0;
    let mut last_geometry = response.geometry.clone();
    // graph_embed_v1: the last conflict the app would show in its modal; answered by a "resolve" command.
    let mut conflict: Option<serde_json::Value> = None;
    let mut answer: Option<serde_json::Value> = None;
    let deadline = Instant::now() + Duration::from_secs(args[2].parse()?);
    while Instant::now() < deadline {
        if server.apply_asset_updates(&mut materials) { eprintln!("IMAGE_ASSETS {}",materials.cpu_resident_bytes()); }
        let mut edits = Vec::new();
        if let Some(control) = args.get(3) {
            if let Ok(text) = std::fs::read_to_string(control) {
                if let Ok(command) = serde_json::from_str::<serde_json::Value>(&text) {
                    if command["id"].as_u64().is_some_and(|id| id > command_id) {
                        command_id = command["id"].as_u64().unwrap();
                        match command["op"].as_str().unwrap_or("") {
                            "stop" => break,
                            "texture_probe" => {
                                let suffix = command["slot_suffix"].as_str().unwrap_or("");
                                let slot = server.texture_binding_ids().into_iter()
                                    .find(|id| id.ends_with(suffix)).unwrap_or_default();
                                let binding = materials.resolve(&slot);
                                let cooked = last_geometry.as_ref().map(|geo| {
                                    let triangles=minihou_core::preview::triangulate_geometry(geo);
                                    let names=geo.detail.get("materials").and_then(|v|v.as_array());
                                    let ids=geo.detail.get("material_bindings").and_then(|v|v.get("slots")).and_then(|v|v.as_array());
                                    let slots:Vec<_>=triangles.material_slot.iter().map(|&index| {
                                        let i=index as usize;
                                        ids.and_then(|a|a.get(i)).and_then(|v|v.as_str()).or_else(||names.and_then(|a|a.get(i)).and_then(|v|v.as_str())).unwrap_or("")
                                    }).collect();
                                    serde_json::json!({"materials":geo.detail.get("materials"),"bindings":geo.detail.get("material_bindings"),
                                        "draw_slots":slots,"uv_triangles":triangles.uv_sets.get("uv").map_or(0,|uv|uv.len()/6)})
                                });
                                let report = serde_json::json!({"slot":slot,
                                    "image_id":binding.descriptor.as_ref().and_then(|d|d.textures.base_color.as_ref()).and_then(|t|t.image_id.as_ref()),
                                    "image":binding.image.as_ref().map(|i|[i.width,i.height]),
                                    "state":format!("{:?}",binding.state),
                                    "diagnostics":server.texture_status(),"cooked":cooked});
                                std::fs::write(command["output"].as_str().ok_or("texture probe needs output")?,report.to_string())?;
                            }
                            "expose" => {
                                let edit = minihou_app::toggle_exposed(
                                    editor.doc.value(),
                                    command["node"].as_str().unwrap(),
                                    command["param"].as_str().unwrap(),
                                );
                                editor.history.edit(&mut editor.doc, &edit)?;
                                editor.history.finish(&editor.doc);
                                edits.push(edit);
                            }
                            "param" => {
                                let edit =
                                    minihou_app::link_param_edit(editor.doc.value(), &command)?;
                                editor.history.edit(&mut editor.doc, &edit)?;
                                editor.history.finish(&editor.doc);
                                edits.push(edit);
                            }
                            "undo" => {
                                if let Some(edit) = editor.history.undo(&mut editor.doc) {
                                    edits.push(edit);
                                }
                            }
                            "redo" => {
                                if let Some(edit) = editor.history.redo(&mut editor.doc) {
                                    edits.push(edit);
                                }
                            }
                            "open_dirty" => server.set_open_dirty(command["value"] == true),
                            // Answered once the conflict request has arrived (it may still be queued).
                            "resolve" => answer = Some(command.clone()),
                            "replace" => {
                                let doc = Document::parse(command["graph"].to_string())?;
                                editor.history.replace(&mut editor.doc, doc);
                                edits.push(Edit::Replace(editor.doc.value().clone()));
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        for request in server.poll() {
            let result: minihou_app::Result<()> = (|| {
                match request.message["type"].as_str().unwrap_or("") {
                    "live_activate" | "live_update" | "live_remove" | "live_replace" => {
                        edits.extend(server.handle_live(&request, &mut editor)?)
                    }
                    "embed_conflict" => {
                        eprintln!("EMBED_CONFLICT {}", request.message["kind"].as_str().unwrap_or(""));
                        conflict = Some(request.message.clone());
                    }
                    "set_param" => {
                        let edit =
                            minihou_app::link_param_edit(editor.doc.value(), &request.message)?;
                        editor.history.finish(&editor.doc);
                        editor.history.edit(&mut editor.doc, &edit)?;
                        editor.history.finish(&editor.doc);
                        edits.push(edit);
                    }
                    "input_define" => {
                        let live = server.bound_path().map(str::to_owned);
                        let (reply, changed) = minihou_app::input_define::handle(&mut editor, live.as_deref(), &request.message);
                        if changed { edits.push(Edit::Replace(editor.doc.value().clone())); }
                        server.reply(request.peer, reply);
                    }
                    "set_frame" => {
                        if server.set_frame(&request.message)? {
                            edits.push(Edit::Time {
                                frame: server.frame,
                                fps: server.fps,
                            });
                        }
                    }
                    "object_geo" | "object_missing" | "image_data" | "image_missing" => {
                        match minihou_app::link_protocol::external_edit(
                            &request.message,
                            &request.arrays,
                        ) {
                            Ok(edit) => edits.push(edit),
                            Err(e) => {
                                let image = request.message["type"]
                                    .as_str()
                                    .unwrap_or("")
                                    .starts_with("image_");
                                if let Some(name) =
                                    request.message[if image { "image" } else { "object" }].as_str()
                                {
                                    edits.push(Edit::External{name:format!("{}:{name}",if image{"image"}else{"object"}),input:minihou_core::nodes::blender_input::ExternalInput::Missing(format!("could not read input: {e}"))});
                                }
                                return Err(e.into());
                            }
                        }
                    }
                    _ => {}
                }
                Ok(())
            })();
            if let Err(e) = result {
                server.reply_error(
                    request.peer,
                    request.message["type"].as_str().unwrap_or(""),
                    &e.to_string(),
                );
            }
        }
        if conflict.is_some() && answer.is_some() {
            let (c, command) = (conflict.take().unwrap(), answer.take().unwrap());
            let choice = minihou_app::graph_embed::Resolution::parse(command["choice"].as_str().unwrap_or(""))
                .ok_or("resolve needs use_embedded, keep_mine or open_as_new")?;
            if let Some(path) = command["export"].as_str() {
                editor.export_to(std::path::Path::new(path))?;
            }
            edits.extend(server.resolve_embed(&c, choice, &mut editor)?);
        }
        server.document_changed(&editor.doc, &editor.path);
        if !edits.is_empty() {
            revision += 1;
            let requests = edits
                .into_iter()
                .map(|edit| CookRequest { revision, edit })
                .collect();
            let response = cook_batch(&mut session, &mut target, requests, |_| ());
            last_geometry=response.geometry.clone();
            server.cooked(&response);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    Ok(())
}
