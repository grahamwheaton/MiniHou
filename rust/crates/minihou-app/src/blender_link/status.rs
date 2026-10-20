use std::{collections::BTreeMap, io};
use serde_json::{json, Value};
use super::LinkServer;

pub fn texture_binding_ids(server: &LinkServer) -> Vec<String> {
    server.shared.image_assets.lock().unwrap().manifests.values().flat_map(|m|
        m["bindings"].as_array().into_iter().flatten().filter_map(|v| v.as_str().map(str::to_owned))
    ).collect()
}

pub fn texture_status(server: &LinkServer) -> Vec<String> {
    let service = server.shared.image_assets.lock().unwrap();
    let mut result = Vec::new();
    for (scope, manifest) in &service.manifests {
        if let Some(diagnostics) = manifest["diagnostics"].as_object() {
            for (slot, reason) in diagnostics {
                result.push(format!("{scope} / {slot}: unsupported: {}", reason.as_str().unwrap_or("unknown")));
            }
        }
        if let Some(images) = manifest["images"].as_array() {
            for hash in images.iter().filter_map(Value::as_str) {
                if !service.has(hash) {
                    result.push(format!("{scope}: pending image {hash}"));
                }
            }
        }
    }
    result
}

pub fn apply_asset_updates(server: &LinkServer, registry: &mut minihou_app::materials::Registry) -> bool {
    let updates = std::mem::take(&mut *server.shared.asset_updates.lock().unwrap());
    if updates.is_empty() { return false; }
    let service = server.shared.image_assets.lock().unwrap();
    for scope in updates { let _ = service.bind(&scope, registry); }
    true
}

pub fn reply(server: &LinkServer, peer: u64, message: Value) {
    if let Some(p) = server.shared.peers.lock().unwrap().get(&peer) {
        let _ = p.replies.send(message);
    }
}

pub fn reply_error(server: &LinkServer, peer: u64, where_: &str, message: &str) {
    if let Some(p) = server.shared.peers.lock().unwrap().get(&peer) {
        let _ = p.replies.send(json!({"type":"error","where":where_,"message":message}));
    }
}

pub fn cooked<T>(server: &LinkServer, response: &minihou_app::CookResponse<T>) {
    let mut snapshot = server.shared.snapshot.lock().unwrap();
    // Sequence numbers belong to this listener, not a document's worker. File Open
    // starts a new worker whose revision counter can be lower than the previous one.
    snapshot.version += 1;
    snapshot.seq += 1;
    snapshot.error = response.error.clone();
    if response.geometry.is_none() {
        return;
    } // retain the last good result for new subscribers/resync
    snapshot.good_seq = snapshot.seq;
    snapshot.extras = response.extras.clone();
    snapshot.scene = response.scene.clone();
    snapshot.geometry = response.geometry.clone();
    snapshot.cook = json!({"ms":response.milliseconds,"cooked":response.stats.cooked,"cached":response.stats.cached,
        "node_ms":response.stats.node_ms.iter().cloned().collect::<BTreeMap<_,_>>(),"frame":server.frame,"fps":server.fps});
}

pub fn set_frame(server: &mut LinkServer, message: &Value) -> io::Result<bool> {
    let frame = match message.get("frame") {
        Some(v) => v.as_f64().ok_or_else(|| io::Error::other("bad frame"))?,
        None => 1.0,
    };
    let fps = match message.get("fps").filter(|v| !v.is_null()) {
        Some(v) => v.as_f64().ok_or_else(|| io::Error::other("bad fps"))?,
        None => server.fps,
    };
    if !frame.is_finite() || !fps.is_finite() || fps <= 0.0 {
        return Err(io::Error::other("frame must be finite and fps positive"));
    }
    let changed = frame != server.frame || fps != server.fps;
    server.frame = frame;
    server.fps = fps;
    Ok(changed)
}
