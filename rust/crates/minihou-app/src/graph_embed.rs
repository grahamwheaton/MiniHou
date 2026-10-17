//! Embedded graphs (`graph_embed_v1`): Blender keeps a MiniHou network inside the .blend and
//! pushes it to the app; the app sends the edited graph back. Pure logic only (no UI, no I/O)
//! so framing, limits, migration and conflict decisions are unit tested. See docs/EMBEDDED_GRAPHS.md.
use serde_json::{json, Value};

/// Capability advertised in the link welcome. Older peers never see the graph_* messages.
pub const CAPABILITY: &str = "graph_embed_v1";
/// Version of the embed envelope (the graph_push / graph_state fields), not of the graph.
pub const EMBED_SCHEMA: u64 = 1;
/// Upper bound for one embedded graph text. Large attrib_paint / sketch payloads fit; nothing is truncated.
pub const MAX_GRAPH_BYTES: usize = 32 * 1024 * 1024;
/// Graph schema this build writes (`version` in the graph JSON, see minihou_core::graph::GRAPH_VERSION).
pub const GRAPH_SCHEMA: i64 = minihou_core::graph::GRAPH_VERSION;

/// Compact JSON preserving key order (node order is meaningful: the first output is the main output).
pub fn canonical_text(v: &Value) -> String {
    serde_json::to_string(v).unwrap_or_default()
}
pub fn graph_hash(v: &Value) -> String {
    sha256_hex(canonical_text(v).as_bytes())
}

/// Bring an older graph up to [`GRAPH_SCHEMA`]. A newer graph is refused with a clear message and
/// is never modified (the caller keeps the original bytes).
pub fn migrate(graph: &Value) -> Result<Value, String> {
    let obj = graph.as_object().ok_or("embedded graph must be a JSON object")?;
    if !obj.get("nodes").is_some_and(Value::is_object) {
        return Err("embedded graph has no 'nodes' object".into());
    }
    match obj.get("version") {
        None => {
            // Schema 0: graphs saved before graphs carried a version. Same layout; stamp it.
            let mut out = graph.clone();
            out["version"] = json!(GRAPH_SCHEMA);
            Ok(out)
        }
        Some(v) => match v.as_i64() {
            Some(n) if n == GRAPH_SCHEMA => Ok(graph.clone()),
            Some(n) if n > GRAPH_SCHEMA => Err(format!(
                "this network was saved by a newer MiniHou (graph schema {n}; this app reads up to {GRAPH_SCHEMA}). \
                 Update MiniHou; the embedded data was left untouched")),
            _ => Err(format!("unsupported graph schema {v}")),
        },
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Push {
    pub network_id: String,
    pub name: String,
    pub label: String,
    pub graph: Value,
    pub hash: String,
    pub size: usize,
}

/// Validate a `graph_push` header: bounded size, integrity hash over the exact bytes sent,
/// known envelope schema, parseable and migratable graph.
pub fn parse_push(h: &Value) -> Result<Push, String> {
    let network_id = h["instance_id"].as_str().filter(|s| !s.is_empty() && s.len() <= 256)
        .ok_or("graph_push needs the network id in instance_id")?.to_string();
    let schema = h["schema"].as_u64().ok_or("graph_push needs a numeric schema")?;
    if schema > EMBED_SCHEMA {
        return Err(format!("graph_push schema {schema} is newer than this app ({EMBED_SCHEMA}); update MiniHou"));
    }
    let text = h["json"].as_str().ok_or("graph_push needs the graph text in 'json'")?;
    if text.len() > MAX_GRAPH_BYTES {
        return Err(format!("embedded graph is {} bytes; the limit is {MAX_GRAPH_BYTES}", text.len()));
    }
    if h["size"].as_u64().is_some_and(|s| s as usize != text.len()) {
        return Err("graph_push size does not match the graph text".into());
    }
    let hash = h["hash"].as_str().ok_or("graph_push needs a sha256 'hash'")?;
    if !hash.eq_ignore_ascii_case(&sha256_hex(text.as_bytes())) {
        return Err("graph_push hash mismatch: the embedded graph was damaged in transit".into());
    }
    let value: Value = serde_json::from_str(text).map_err(|e| format!("embedded graph is not valid JSON: {e}"))?;
    let graph = migrate(&value)?;
    let clip = |k: &str| h[k].as_str().unwrap_or("").chars().take(256).collect::<String>();
    Ok(Push { network_id, name: clip("name"), label: clip("label"), graph, hash: hash.to_ascii_lowercase(), size: text.len() })
}

/// What to do with a pushed graph, given the app's live copy and the last agreed graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Identical: nothing to do.
    Unchanged,
    /// The app copy is absent or unedited since the last agreement: take Blender's.
    AdoptEmbedded,
    /// Blender's copy is the last agreed one; the app has newer edits: send them back.
    KeepApp,
    /// Both changed since the last agreement: the user decides (modal in the app).
    Conflict,
    /// No live copy, but the app has an unrelated modified document open: ask before switching.
    OpenDocumentDirty,
}

pub fn decide(pushed: &Value, app: Option<&Value>, base: Option<&Value>, open_dirty: bool) -> Decision {
    match app {
        None if open_dirty => Decision::OpenDocumentDirty,
        None => Decision::AdoptEmbedded,
        Some(app) if app == pushed => Decision::Unchanged,
        Some(app) if base == Some(app) => Decision::AdoptEmbedded,
        Some(_) if base == Some(pushed) => Decision::KeepApp,
        Some(_) => Decision::Conflict,
    }
}

/// The user's answer in the conflict modal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    UseEmbedded,
    KeepMine,
    OpenAsNew,
}
impl Resolution {
    pub fn as_str(self) -> &'static str {
        match self { Self::UseEmbedded => "use_embedded", Self::KeepMine => "keep_mine", Self::OpenAsNew => "open_as_new" }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s { "use_embedded" => Self::UseEmbedded, "keep_mine" => Self::KeepMine, "open_as_new" => Self::OpenAsNew, _ => return None })
    }
    /// Whether the bound document ends up holding Blender's embedded graph.
    pub fn adopts_embedded(self) -> bool { self != Self::KeepMine }
}

/// Text and buttons of the app's conflict modal. `kind` is `live` (the bound document was edited
/// in the app while Blender's copy also changed) or `open_document` (an unrelated document with
/// unsaved edits is open).
pub fn conflict_prompt(kind: &str, label: &str) -> (String, [(Resolution, &'static str); 3]) {
    let who = binding_title(label);
    let text = if kind == "live" {
        format!("{who}: the network embedded in the .blend changed while you edited it here. \
                 Use embedded replaces your edits (undo brings them back); Keep my version sends yours \
                 to Blender; Open embedded as new document first saves your version to a .minihou file.")
    } else {
        format!("{who} wants to open its embedded network, but your open document has unsaved changes. \
                 Use embedded or Open embedded as new document keep your document in memory and restore it \
                 when the Blender edit ends; Keep my open file leaves the embedded network closed.")
    };
    (text, [(Resolution::UseEmbedded, "Use embedded graph"),
            (Resolution::KeepMine, if kind == "live" { "Keep my version" } else { "Keep my open file" }),
            (Resolution::OpenAsNew, "Open embedded as new document")])
}

/// Window title for a document bound to a Blender network.
pub fn binding_title(label: &str) -> String {
    let label = label.replace(" / ", "/");
    format!("Blender: {}", if label.is_empty() { "network" } else { &label })
}

/// `graph_state` / `graph_ack` payload describing the app's copy.
pub fn state(kind: &str, status: &str, graph: Option<&Value>) -> Value {
    let mut out = json!({"type": kind, "status": status, "schema": EMBED_SCHEMA, "graph_schema": GRAPH_SCHEMA});
    if let Some(graph) = graph {
        let text = canonical_text(graph);
        out["hash"] = json!(sha256_hex(text.as_bytes()));
        out["size"] = json!(text.len());
        out["json"] = json!(text);
    }
    out
}

/// SHA-256 (FIPS 180-4). Small and dependency free; used only for integrity ids.
pub fn sha256_hex(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
        0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
        0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
        0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2];
    let mut h: [u32; 8] = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 { msg.push(0); }
    msg.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for chunk in msg.chunks_exact(64) {
        let mut w = [0u32; 64];
        for i in 0..16 { w[i] = u32::from_be_bytes(chunk[i * 4..i * 4 + 4].try_into().unwrap()); }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g; g = f; f = e; e = d.wrapping_add(t1); d = c; c = b; b = a; a = t1.wrapping_add(t2);
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) { *x = x.wrapping_add(y); }
    }
    h.iter().map(|x| format!("{x:08x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn push(graph: &Value) -> Value {
        let text = canonical_text(graph);
        json!({"type":"graph_push","instance_id":"net","schema":1,"name":"N","label":"Cube / MiniHou",
               "json":text,"hash":sha256_hex(text.as_bytes()),"size":text.len()})
    }

    #[test]
    fn sha256_matches_known_vectors() {
        assert_eq!(sha256_hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        let long = vec![b'a'; 1000];
        assert_eq!(sha256_hex(&long), "41edece42d63e8d9bf515a9ba6932e1c20cbc9f5a5d134645adb5db1b9737ea3");
    }

    #[test]
    fn canonical_text_preserves_node_order() {
        let g: Value = serde_json::from_str(r#"{"nodes":{"z":{"type":"box"},"a":{"type":"output","inputs":{"geo":"z"}}}}"#).unwrap();
        assert!(canonical_text(&g).find("\"z\"").unwrap() < canonical_text(&g).find("\"a\"").unwrap());
    }

    #[test]
    fn push_validates_hash_size_schema_and_migrates() {
        let g = json!({"nodes":{"b":{"type":"box"}},"display":"b"});
        let p = parse_push(&push(&g)).unwrap();
        assert_eq!(p.graph["version"], 1, "unversioned graphs are migrated to schema 1");
        assert_eq!(p.label, "Cube / MiniHou");
        let mut bad = push(&g);
        bad["hash"] = json!("00");
        assert!(parse_push(&bad).unwrap_err().contains("hash mismatch"));
        let mut bad = push(&g);
        bad["size"] = json!(3);
        assert!(parse_push(&bad).is_err());
        let mut bad = push(&g);
        bad["schema"] = json!(2);
        assert!(parse_push(&bad).unwrap_err().contains("newer"));
        let big = "x".repeat(MAX_GRAPH_BYTES + 1);
        let bad = json!({"instance_id":"net","schema":1,"json":big,"hash":sha256_hex(big.as_bytes())});
        assert!(parse_push(&bad).unwrap_err().contains("limit"));
    }

    #[test]
    fn newer_graph_schema_is_refused_untouched() {
        let g = json!({"version":99,"nodes":{}});
        let e = migrate(&g).unwrap_err();
        assert!(e.contains("newer MiniHou") && e.contains("untouched"), "{e}");
        assert_eq!(migrate(&json!({"version":1,"nodes":{}})).unwrap()["version"], 1);
    }

    #[test]
    fn conflict_decisions() {
        let (a, b, c) = (json!({"n":1}), json!({"n":2}), json!({"n":3}));
        assert_eq!(decide(&a, None, None, false), Decision::AdoptEmbedded);
        assert_eq!(decide(&a, None, None, true), Decision::OpenDocumentDirty);
        assert_eq!(decide(&a, Some(&a), None, true), Decision::Unchanged);
        assert_eq!(decide(&b, Some(&a), Some(&a), false), Decision::AdoptEmbedded, "app unedited");
        assert_eq!(decide(&a, Some(&b), Some(&a), false), Decision::KeepApp, "only the app edited");
        assert_eq!(decide(&b, Some(&c), Some(&a), false), Decision::Conflict, "both edited");
        assert_eq!(decide(&b, Some(&c), None, false), Decision::Conflict, "no agreement yet");
        assert!(Resolution::OpenAsNew.adopts_embedded() && !Resolution::KeepMine.adopts_embedded());
        for r in [Resolution::UseEmbedded, Resolution::KeepMine, Resolution::OpenAsNew] {
            assert_eq!(Resolution::parse(r.as_str()), Some(r));
        }
    }

    #[test]
    fn state_carries_verifiable_hash_and_title() {
        let g = json!({"version":1,"nodes":{"b":{"type":"box"}}});
        let s = state("graph_state", "ok", Some(&g));
        assert_eq!(s["hash"], sha256_hex(s["json"].as_str().unwrap().as_bytes()));
        assert_eq!(s["size"], s["json"].as_str().unwrap().len());
        assert_eq!(binding_title("Cube / MiniHou"), "Blender: Cube/MiniHou");
    }
}
