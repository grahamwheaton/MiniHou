use std::{
    collections::BTreeMap,
    io::{self, Write},
    net::{Shutdown, TcpStream},
    sync::{
        atomic::Ordering,
        mpsc, Arc,
    },
    thread,
    time::{Duration, Instant},
};
use minihou_app::link_protocol::{self, Arrays, DiffState};
use serde_json::{json, Value};
use super::{
    link_instances, LinkRequest, LinkShared, Peer,
};

/// Join only finished workers, with one shared deadline for a whole shutdown.
/// Dropping an unfinished JoinHandle detaches it; it cannot retain the listener
/// or a usable peer socket, and must never hold up application exit.
pub(crate) fn join_until(worker: thread::JoinHandle<()>, deadline: Instant) {
    while !worker.is_finished() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(2));
    }
    if worker.is_finished() {
        let _ = worker.join();
    }
}

pub(crate) fn send_frame(socket: &mut TcpStream, header: &Value, arrays: &Arrays) -> io::Result<()> {
    socket.write_all(&link_protocol::encode_message(header, arrays)?)
}

pub(crate) fn serve_client(
    mut socket: TcpStream,
    state: Arc<LinkShared>,
    requests: mpsc::SyncSender<LinkRequest>,
) -> io::Result<()> {
    // Windows accepted sockets inherit the listener's nonblocking mode. The framing
    // reader uses read_exact, so both handshake and reader thread require blocking I/O.
    socket.set_nonblocking(false)?;
    socket.set_nodelay(true)?;
    socket.set_write_timeout(Some(Duration::from_secs(2)))?;
    // A stalled/partial hello cannot leave a reader behind forever.
    socket.set_read_timeout(Some(Duration::from_secs(10)))?;
    let (hello, _) = link_protocol::read_message(&mut socket, 1024 * 1024)?;
    if hello["type"] != "hello" {
        return Err(io::Error::other("expected hello"));
    }
    if hello["protocol"].as_u64() != Some(link_protocol::PROTOCOL_VERSION as u64) {
        send_frame(
            &mut socket,
            &json!({"type":"error","message":"unsupported protocol version"}),
            &Arrays::new(),
        )?;
        return Ok(());
    }
    let snapshot = state.snapshot.lock().unwrap().clone();
    send_frame(
        &mut socket,
        &json!({"type":"welcome","protocol":link_protocol::PROTOCOL_VERSION,"server":"minihou",
        "capabilities":["instances_v1","live_modifier_v1","object_inputs_v1","scene_objects_v1","image_exchange_v1","viewport_materials_v1","image_assets_v1",minihou_app::graph_embed::CAPABILITY,minihou_app::input_define::CAPABILITY],"editor_url":null,"editor":"native","graph_path":snapshot.graph_path}),
        &Arrays::new(),
    )?;
    let instance_mode = hello["mode"] == "instances";
    if !instance_mode {
        send_frame(&mut socket, &snapshot.watch, &Arrays::new())?;
    }
    socket.set_read_timeout(None)?;
    let mut reader = socket.try_clone()?;
    let (incoming, received) = mpsc::sync_channel(32);
    let reader_state = state.clone();
    let reader_thread = thread::Builder::new()
        .name("minihou-link-reader".into())
        .spawn(move || loop {
            let mut message = link_protocol::read_message(&mut reader, 512 * 1024 * 1024);
            let failed = message.is_err();
            loop {
                if reader_state.stop.load(Ordering::Relaxed) {
                    return;
                }
                match incoming.try_send(message) {
                    Ok(()) => break,
                    Err(mpsc::TrySendError::Disconnected(_)) => return,
                    Err(mpsc::TrySendError::Full(value)) => {
                        message = value;
                        thread::sleep(Duration::from_millis(2));
                    }
                }
            }
            if failed { break; }
        })?;
    let id = state.next_peer.fetch_add(1, Ordering::Relaxed);
    let (replies, reply_rx) = mpsc::channel();
    state.peers.lock().unwrap().insert(
        id,
        Peer {
            name: hello["client"].as_str().unwrap_or("?").into(),
            protocol: hello["protocol"].as_u64().unwrap_or(1),
            replies,
        },
    );
    let mut instances = link_instances::InstanceManager::new(id, state.clone(), requests.clone());
    // object_names_v1: the session connection of a capable add-on answers object-name searches.
    let names_capable = !instance_mode && hello["capabilities"].as_array().is_some_and(|a| a.iter().any(|v| v == minihou_app::object_names::CAPABILITY));
    if names_capable { if let Ok(mut hub) = minihou_app::object_names::HUB.lock() { hub.capable_peers += 1; } }
    let result = (|| -> io::Result<()> {
        let mut diff = DiffState::default();
        let mut version = 0;
        let mut seq = None;
        let mut good_seq = None;
        let mut interface = Value::Null;
        let mut watch = snapshot.watch.clone();
        let mut extra_diffs: BTreeMap<String, DiffState> = BTreeMap::new();
        let mut scene_keys: BTreeMap<String, Value> = BTreeMap::new();
        loop {
            if state.stop.load(Ordering::Relaxed) {
                break;
            }
            match received.recv_timeout(Duration::from_millis(16)) {
                Ok(Ok((h, arrays))) => match h["type"].as_str().unwrap_or("") {
                    "object_names" if names_capable => {
                        if let Ok(mut hub) = minihou_app::object_names::HUB.lock() { if hub.incoming.len() < 64 { hub.incoming.push(h); } }
                    }
                    "material_manifest" if hello["capabilities"].as_array().is_some_and(|a|a.iter().any(|v|v=="viewport_materials_v1")) => {
                        let scope=h["scope"].as_str().unwrap_or("");
                        let scoped=format!("peer{id}:{scope}");
                        let revision=h["revision"].as_u64().unwrap_or(0);
                        let hashes:Vec<String>=h["images"].as_array().into_iter().flatten().filter_map(|v|v.as_str().map(str::to_owned)).collect();
                        if !scope.is_empty() && revision>0 {
                            if h.to_string().len()>1024*1024 || h["descriptors"].as_object().is_none_or(|v|v.len()>4096)
                                || h["bindings"].as_array().is_none_or(|v|v.len()>4096)
                                || hashes.len()>4096 {
                                send_frame(&mut socket,&json!({"type":"error","where":"material_manifest","message":"invalid or oversized material manifest"}),&Arrays::new())?;
                                continue;
                            }
                            let mut service=state.image_assets.lock().unwrap();
                            let mut manifest=h.clone();
                            if let Some(desc)=manifest["descriptors"].as_object_mut() {
                                let renamed=std::mem::take(desc).into_iter().map(|(key,value)|(format!("peer{id}:{key}"),value)).collect();
                                *desc=renamed;
                            }
                            if let Some(bindings)=manifest["bindings"].as_array_mut() {for binding in bindings {if let Some(key)=binding.as_str(){*binding=json!(format!("peer{id}:{key}"));}}}
                            if service.manifest(&scoped,revision,manifest) {
                                let missing=service.missing(&hashes);
                                drop(service);
                                state.asset_updates.lock().unwrap().push(scoped);
                                send_frame(&mut socket,&json!({"type":"image_request","scope":scope,"revision":revision,"hashes":missing}),&Arrays::new())?;
                                if let Some(instance)=scope.strip_prefix("instance:") {
                                    let bindings={state.image_assets.lock().unwrap().manifests
                                        .get(&format!("peer{id}:{scope}")).and_then(|m|m.get("bindings")).cloned()};
                                    if let Some(bindings)=bindings {
                                        let _=instances.rebind_manifest(instance,&bindings);
                                    }
                                }
                            }
                        }
                    }
                    "image_begin" | "image_chunk" | "image_end" | "image_error" if hello["capabilities"].as_array().is_some_and(|a|a.iter().any(|v|v=="image_assets_v1")) => {
                        let id=h["transfer_id"].as_str().unwrap_or("");
                        let scoped_id=format!("peer{id}:{id}");
                        let mut service=state.image_assets.lock().unwrap();
                        let result=match h["type"].as_str().unwrap_or("") {
                            "image_begin"=>service.begin(&scoped_id,h["hash"].as_str().unwrap_or(""),h["total"].as_u64().and_then(|n|usize::try_from(n).ok()).unwrap_or(0)).map(|_|None),
                            "image_chunk"=>arrays.get("data").ok_or_else(||"missing data array".to_string()).and_then(|a|if a.dtype=="|u1" && a.bytes.len()<=minihou_app::image_assets::CHUNK {service.chunk(&scoped_id,h["offset"].as_u64().and_then(|n|usize::try_from(n).ok()).unwrap_or(usize::MAX),&a.bytes)}else{Err("invalid chunk array".into())}).map(|_|None),
                            "image_end"=>service.end(&scoped_id).map(Some),
                            _=>{service.cancel(&scoped_id);Ok(None)},
                        };
                        let affected=if result.as_ref().is_ok_and(Option::is_some) {service.manifests.keys().cloned().collect::<Vec<_>>()} else {Vec::new()};
                        drop(service);
                        match result {
                            Ok(Some(hash))=>{
                                state.asset_updates.lock().unwrap().extend(affected);
                                send_frame(&mut socket,&json!({"type":"image_ack","transfer_id":id,"hash":hash}),&Arrays::new())?;
                            },
                            Ok(None)=>{},
                            Err(message)=>send_frame(&mut socket,&json!({"type":"image_error","transfer_id":id,"message":message}),&Arrays::new())?,
                        }
                    }
                    "ping" => send_frame(
                        &mut socket,
                        &json!({"type":"pong","t":h["t"]}),
                        &Arrays::new(),
                    )?,
                    "resync" => {
                        diff.reset();
                        extra_diffs.clear();
                        scene_keys.clear();
                        version = 0;
                        seq = None;
                        good_seq = None;
                        interface = Value::Null;
                        watch = Value::Null;
                    }
                    typ if typ.starts_with("instance_") || typ.starts_with("graph_") => {
                        let typ=typ.to_owned();
                        let mut h=h;
                        if matches!(typ.as_str(),"instance_cook"|"instance_edit"|"instance_sources") && h.get("manifest_revision").is_some() {
                            let scope=format!("peer{id}:instance:{}",h["instance_id"].as_str().unwrap_or(""));
                            let service=state.image_assets.lock().unwrap();
                            if service.revisions.get(&scope).copied()!=h["manifest_revision"].as_u64() {
                                drop(service);
                                send_frame(&mut socket,&json!({"type":"instance_error","request_id":h["request_id"],"instance_id":h["instance_id"],"message":"geometry manifest revision is unavailable"}),&Arrays::new())?;
                                continue;
                            }
                            if let Some(bindings)=service.manifests.get(&scope).and_then(|v|v.get("bindings")).filter(|b|b.as_array().is_some_and(|a|!a.is_empty())) {
                                h["material_bindings"]=json!({"version":1,"slots":bindings});
                            }
                        }
                        let (header, output) = if instance_mode {
                            instances.handle(&h, &arrays)
                        } else {
                            (
                                json!({"type":"instance_error","request_id":h["request_id"],"instance_id":h["instance_id"],"message":"instance requests require hello mode instances"}),
                                Arrays::new(),
                            )
                        };
                        send_frame(&mut socket, &header, &output)?;
                    }
                    "set_param" | "set_frame" | "object_geo" | "object_missing" | "image_data"
                    | "image_missing" => {
                        let mut h=h;
                        if h["type"]=="object_geo" && h.get("manifest_revision").is_some() {
                            let scope=format!("peer{id}:object:{}",h["object"].as_str().unwrap_or(""));
                            let service=state.image_assets.lock().unwrap();
                            if service.revisions.get(&scope).copied()!=h["manifest_revision"].as_u64() {
                                drop(service);
                                send_frame(&mut socket,&json!({"type":"error","where":"material_manifest","message":"geometry manifest revision is unavailable"}),&Arrays::new())?;
                                continue;
                            }
                            if let Some(bindings)=service.manifests.get(&scope).and_then(|v|v.get("bindings")).filter(|b|b.as_array().is_some_and(|a|!a.is_empty())) {
                                h["material_bindings"]=json!({"version":1,"slots":bindings});
                            }
                        }
                        if state.live_active.load(Ordering::Relaxed) && h["type"] == "set_param" {
                            send_frame(
                                &mut socket,
                                &json!({"type":"error","where":"set_param","message":"Use the selected modifier controls or live editor"}),
                                &Arrays::new(),
                            )?;
                            continue;
                        }
                        if requests
                            .try_send(LinkRequest {
                                peer: id,
                                message: h,
                                arrays,
                                completion: None,
                            })
                            .is_err()
                        {
                            send_frame(
                                &mut socket,
                                &json!({"type":"error","message":"editor request queue is full"}),
                                &Arrays::new(),
                            )?;
                        }
                    }
                    // input_define_v1: applied on the UI thread as an undoable edit; it always answers
                    // with input_define_ack (status ok|error) carrying this request_id/instance_id.
                    "input_define" => {
                        if requests.try_send(LinkRequest { peer: id, message: h.clone(), arrays, completion: None }).is_err() {
                            send_frame(&mut socket, &minihou_app::input_define::error(&h, "busy", "editor request queue is full"), &Arrays::new())?;
                        }
                    }
                    "geo_ack" => {} // TCP needs no acknowledgement; some addon versions send one anyway.
                    _ => send_frame(
                        &mut socket,
                        &json!({"type":"error","message":"unsupported message type"}),
                        &Arrays::new(),
                    )?,
                },
                Ok(Err(e)) => return Err(e),
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            for reply in reply_rx.try_iter() {
                send_frame(&mut socket, &reply, &Arrays::new())?;
            }
            if names_capable {
                let queries = minihou_app::object_names::HUB.lock().map(|mut hub| std::mem::take(&mut hub.outgoing)).unwrap_or_default();
                for query in queries { send_frame(&mut socket, &query, &Arrays::new())?; }
            }
            if instance_mode || state.live_active.load(Ordering::Relaxed) {
                continue;
            }
            let snapshot = state.snapshot.lock().unwrap().clone();
            if version == snapshot.version {
                continue;
            }
            version = snapshot.version;
            if watch != snapshot.watch {
                send_frame(&mut socket, &snapshot.watch, &Arrays::new())?;
                watch = snapshot.watch.clone();
            }
            if interface != snapshot.interface || seq != Some(snapshot.seq) {
                send_frame(&mut socket, &snapshot.interface, &Arrays::new())?;
                interface = snapshot.interface;
            }
            if seq != Some(snapshot.seq) {
                if let Some(geo) = snapshot
                    .geometry
                    .filter(|_| good_seq != Some(snapshot.good_seq))
                {
                    let shown = link_protocol::output_geometry(&geo).map_err(io::Error::other)?;
                    let arrays = link_protocol::geometry_arrays(&shown);
                    let (header, changed) =
                        diff.build(&arrays, &shown, snapshot.good_seq, &snapshot.cook);
                    send_frame(&mut socket, &header, &changed)?;
                    let names: Vec<_> = snapshot.extras.keys().cloned().collect();
                    send_frame(
                        &mut socket,
                        &json!({"type":"outputs","names":names}),
                        &Arrays::new(),
                    )?;
                    extra_diffs.retain(|name, _| snapshot.extras.contains_key(name));
                    for (name, g) in &snapshot.extras {
                        let shown = link_protocol::output_geometry(g).map_err(io::Error::other)?;
                        let (mut h, a) = extra_diffs.entry(name.clone()).or_default().build(
                            &link_protocol::geometry_arrays(&shown),
                            &shown,
                            snapshot.good_seq,
                            &snapshot.cook,
                        );
                        h["output"] = json!(name);
                        send_frame(&mut socket, &h, &a)?;
                    }
                    good_seq = Some(snapshot.good_seq);
                    let nodes: Vec<_> = snapshot
                        .scene
                        .iter()
                        .filter_map(|(h, _)| h["node"].as_str())
                        .collect();
                    send_frame(
                        &mut socket,
                        &json!({"type":"scene_nodes","nodes":nodes}),
                        &Arrays::new(),
                    )?;
                    scene_keys.retain(|node, _| nodes.contains(&node.as_str()));
                    for (h, a) in &snapshot.scene {
                        if let Some(node) = h["node"].as_str() {
                            if scene_keys.get(node) != Some(&h["key"]) {
                                send_frame(&mut socket, h, a)?;
                                scene_keys.insert(node.into(), h["key"].clone());
                            }
                        }
                    }
                }
                if let Some(error) = snapshot.error {
                    send_frame(
                        &mut socket,
                        &json!({"type":"error","where":"cook","seq":snapshot.seq,"message":error}),
                        &Arrays::new(),
                    )?;
                }
                seq = Some(snapshot.seq);
            }
        }
        Ok(())
    })();
    // Wake read_exact and release a full incoming queue before any cleanup.
    let _ = socket.shutdown(Shutdown::Both);
    drop(received);
    join_until(reader_thread, Instant::now() + Duration::from_millis(50));
    instances.clear();
    state.image_assets.lock().unwrap().remove_peer(&format!("peer{id}:"));
    state.peers.lock().unwrap().remove(&id);
    if names_capable { if let Ok(mut hub) = minihou_app::object_names::HUB.lock() { hub.capable_peers = hub.capable_peers.saturating_sub(1); } }
    result
}
